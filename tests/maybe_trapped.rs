//! L1 tests for what a player knows of its Pokémon being trapped (DESIGN.md §47.0 item 14,
//! R-M10-28): a trap for all to see is `Trapped::Yes` with no switch listed; one a foe's ability
//! sets, or may set — its own, or one its species may have where the format keeps abilities legal
//! — is `Trapped::Maybe`, the switches listed and a switch that fails naming neither the foe nor
//! its ability (Showdown's request `trapped` / `maybeTrapped`); else `Trapped::No`.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, ChoiceError, EngineStep, SideChoice, SlotChoice, Trapped,
};
use pokemon_battle_engine::ids::{AbilityId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{FormatRules, MaybeTrappedBy, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);

fn set(species: &str, moves: &[&str], ability: &str) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.level = 50;
    set
}

/// P1 = [A, C (bench)]; P2 = [B], under `maybe_trapped_by`.
fn battle(
    generation: Generation,
    maybe_trapped_by: MaybeTrappedBy,
    p1: PokemonSet,
    p2: PokemonSet,
) -> BattleEngine {
    let spec = BattleSpec::new(generation, BattleMode::Singles, "custom");
    let format = FormatRules {
        maybe_trapped_by,
        ..FormatRules::custom()
    };
    let rules = Arc::new(RulesResolver::resolve_format(&spec, &format).unwrap());
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
        build_pokemon(
            &dex,
            C,
            SideId::P1,
            &set("Snorlax", &["Splash"], "Thick Fat"),
        )
        .unwrap(),
    ];
    let mut p1_side = SideState::new(vec![A, C], 1);
    p1_side.active[0] = Some(A);
    let mut p2_side = SideState::new(vec![B], 1);
    p2_side.active[0] = Some(B);
    let mut engine =
        BattleEngine::try_new(rules, dex, BattleState::new(pokemon, [p1_side, p2_side], 1))
            .unwrap();
    engine.run_lead_switch_ins();
    engine
}

/// What P1's request says of A: trapped, and the switches listed.
fn options(engine: &BattleEngine) -> (Trapped, Vec<PokemonId>) {
    let options = engine.choice_options(SideId::P1).expect("turn choices");
    let actor = &options.actors[0];
    (actor.trapped, actor.switches.clone())
}

/// A turn in which P1 switches A out to C and P2 uses its first move.
fn switch_out(engine: &mut BattleEngine) -> Result<(), BattleError> {
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![SlotChoice {
                actor: A,
                action: ActionChoice::Switch { pokemon: C },
            }],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![SlotChoice {
                actor: B,
                action: ActionChoice::Move {
                    slot: MoveSlot::FIRST,
                    target: None,
                    mechanic: None,
                },
            }],
        },
    ];
    engine.submit_turn_choices(choices)?;
    while let EngineStep::Progressed { .. } = engine.step()? {}
    Ok(())
}

fn species_abilities() -> MaybeTrappedBy {
    MaybeTrappedBy::SpeciesAbilities { banned: Vec::new() }
}

#[test]
fn a_trapping_ability_is_maybe_and_the_failed_switch_names_nothing() {
    // Dugtrio's Arena Trap holds the grounded Snorlax: maybe, as for any foe that may have it;
    // the switch is listed, and refused as it is tried with neither Dugtrio nor its ability named.
    let mut engine = battle(
        Generation::Gen9,
        MaybeTrappedBy::ItsAbility,
        set("Snorlax", &["Splash"], "Thick Fat"),
        set("Dugtrio", &["Splash"], "Arena Trap"),
    );
    assert_eq!(options(&engine), (Trapped::Maybe, vec![C]));
    assert!(matches!(
        switch_out(&mut engine),
        Err(BattleError::Choice(ChoiceError::TrappedByAbility {
            actor: A
        }))
    ));
}

#[test]
fn a_species_that_may_trap_is_maybe_where_the_format_keeps_abilities_legal() {
    // Dugtrio with Sand Veil traps nothing, but may have Arena Trap: maybe where the format keeps
    // abilities legal, the switch going through; not in a custom game, nor where Arena Trap is
    // banned, nor for a Flying type.
    let dugtrio = || set("Dugtrio", &["Splash"], "Sand Veil");
    let mut engine = battle(
        Generation::Gen9,
        species_abilities(),
        set("Snorlax", &["Splash"], "Thick Fat"),
        dugtrio(),
    );
    assert_eq!(options(&engine), (Trapped::Maybe, vec![C]));
    assert!(switch_out(&mut engine).is_ok());
    let custom = battle(
        Generation::Gen9,
        MaybeTrappedBy::ItsAbility,
        set("Snorlax", &["Splash"], "Thick Fat"),
        dugtrio(),
    );
    assert_eq!(options(&custom), (Trapped::No, vec![C]));
    let banned = battle(
        Generation::Gen9,
        MaybeTrappedBy::SpeciesAbilities {
            banned: vec![AbilityId::ARENA_TRAP],
        },
        set("Snorlax", &["Splash"], "Thick Fat"),
        dugtrio(),
    );
    assert_eq!(options(&banned), (Trapped::No, vec![C]));
    let flying = battle(
        Generation::Gen9,
        species_abilities(),
        set("Corviknight", &["Splash"], "Pressure"),
        dugtrio(),
    );
    assert_eq!(options(&flying), (Trapped::No, vec![C]));
}

#[test]
fn a_trap_for_all_to_see_lists_no_switch_and_a_gen_6_ghost_is_never_trapped() {
    // Mean Look (`-activate|p|trapped`): yes, nothing to switch to.
    let mut engine = battle(
        Generation::Gen9,
        species_abilities(),
        set("Snorlax", &["Splash"], "Thick Fat"),
        set("Umbreon", &["Mean Look"], "Synchronize"),
    );
    // Turn 1: Umbreon's Mean Look (P1's Snorlax uses Splash).
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![SlotChoice {
                actor: A,
                action: ActionChoice::Move {
                    slot: MoveSlot::FIRST,
                    target: None,
                    mechanic: None,
                },
            }],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![SlotChoice {
                actor: B,
                action: ActionChoice::Move {
                    slot: MoveSlot::FIRST,
                    target: None,
                    mechanic: None,
                },
            }],
        },
    ];
    engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert_eq!(options(&engine), (Trapped::Yes, Vec::new()));
    // Gengar escapes Dugtrio's Arena Trap (Gen 6+ Ghost types are never trapped).
    let ghost = battle(
        Generation::Gen9,
        species_abilities(),
        set("Gengar", &["Splash"], "Cursed Body"),
        set("Dugtrio", &["Splash"], "Arena Trap"),
    );
    assert_eq!(options(&ghost), (Trapped::No, vec![C]));
}
