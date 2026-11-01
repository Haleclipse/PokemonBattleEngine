//! L1 rule tests for M7 E/F batch 6 (DESIGN.md §23.4): what the replay cannot see of the
//! pseudo-weathers — Gravity's accuracy (a hit decides nothing about the rate) and the choices
//! Gravity and Fairy Lock refuse (a script only holds legal choices).

use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, ChoiceError, SideChoice, SlotChoice, must_struggle, validate_side_choice,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, FieldConditionKind, SideState};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const BENCH: PokemonId = PokemonId(3);

fn set(species: &str, moves: &[&str], item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set.item = item.map(Into::into);
    set
}

/// A on P1 (BENCH behind it), B on P2, with `field` up.
fn battle(
    generation: Generation,
    user: PokemonSet,
    target: PokemonSet,
    field: &[FieldConditionKind],
) -> (
    BattleState,
    pokemon_battle_engine::rules::ResolvedRules,
    pokemon_battle_engine::dex::ResolvedDex,
) {
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &user).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &target).unwrap(),
        build_pokemon(&dex, BENCH, SideId::P1, &set("Mew", &["Harden"], None)).unwrap(),
    ];
    let mut p1 = SideState::new(vec![A, BENCH], 1);
    p1.active[0] = Some(A);
    let mut p2 = SideState::new(vec![B], 1);
    p2.active[0] = Some(B);
    let mut state = BattleState::new(pokemon, [p1, p2], 1);
    state.pending_movers = vec![B];
    for kind in field {
        state.field.start_timed(*kind, 5);
    }
    (state, rules, dex)
}

/// The accuracy check of A's first move on B (`engine::accuracy_roll`).
fn accuracy(
    generation: Generation,
    user: PokemonSet,
    target: PokemonSet,
    gravity: bool,
) -> Option<(u16, u16)> {
    let field: &[FieldConditionKind] = if gravity {
        &[FieldConditionKind::Gravity]
    } else {
        &[]
    };
    let (state, rules, dex) = battle(generation, user, target, field);
    let move_id = state.pokemon(A).unwrap().moves[0].move_id;
    let move_def = dex.move_by_id(move_id).unwrap().clone();
    pokemon_battle_engine::engine::accuracy_roll(&state, &rules, A, B, &move_def)
}

#[test]
fn gravity_raises_accuracy_as_the_generation_does() {
    let zap = || set("Mew", &["Zap Cannon"], None);
    let powder = || set("Snorlax", &["Harden"], Some("Bright Powder"));
    let plain = || set("Snorlax", &["Harden"], None);
    // Gen 9: Showdown's chain, Zap Cannon 50 × 6840 / 4096 = 83; with Bright Powder (the chain
    // 6840 · 3686) 75.
    assert_eq!(
        accuracy(Generation::Gen9, zap(), plain(), false),
        Some((50, 100))
    );
    assert_eq!(
        accuracy(Generation::Gen9, zap(), plain(), true),
        Some((83, 100))
    );
    assert_eq!(
        accuracy(Generation::Gen9, zap(), powder(), true),
        Some((75, 100))
    );
    // Gen 4: the cartridge's last step, 50 × 90 / 100 = 45, × 10 / 6 = 75 (Gravity first
    // would give 74); Hypnosis 60 × 10 / 6 = 100 never misses.
    assert_eq!(
        accuracy(Generation::Gen4, zap(), plain(), true),
        Some((83, 100))
    );
    assert_eq!(
        accuracy(Generation::Gen4, zap(), powder(), true),
        Some((75, 100))
    );
    let hypnosis = set("Mew", &["Hypnosis"], None);
    assert_eq!(accuracy(Generation::Gen4, hypnosis, plain(), true), None);
}

fn move_choice(rules: &pokemon_battle_engine::rules::ResolvedRules, slot: u8) -> SideChoice {
    SideChoice {
        side: SideId::P1,
        actions: vec![SlotChoice {
            actor: A,
            action: ActionChoice::Move {
                slot: MoveSlot::new(slot).unwrap(),
                target: Some(DeclaredTarget(
                    rules.topology().position(SideId::P2, 0).unwrap(),
                )),
                mechanic: None,
            },
        }],
    }
}

fn switch_choice() -> SideChoice {
    SideChoice {
        side: SideId::P1,
        actions: vec![SlotChoice {
            actor: A,
            action: ActionChoice::Switch { pokemon: BENCH },
        }],
    }
}

#[test]
fn gravity_keeps_the_airborne_moves_off_the_request() {
    let flyer = set("Corviknight", &["Fly", "Tackle"], None);
    let (state, rules, dex) = battle(
        Generation::Gen9,
        flyer,
        set("Snorlax", &["Harden"], None),
        &[FieldConditionKind::Gravity],
    );
    assert!(matches!(
        validate_side_choice(&state, &rules, &dex, SideId::P1, &move_choice(&rules, 0)),
        Err(ChoiceError::MoveDisabledByField {
            by: FieldConditionKind::Gravity,
            ..
        })
    ));
    assert!(!must_struggle(&state, &rules, &dex, A));
    // Nothing but airborne moves: Struggle.
    let (state, rules, dex) = battle(
        Generation::Gen9,
        set("Corviknight", &["Fly", "High Jump Kick"], None),
        set("Snorlax", &["Harden"], None),
        &[FieldConditionKind::Gravity],
    );
    assert!(must_struggle(&state, &rules, &dex, A));
    let (state, rules, dex) = battle(
        Generation::Gen9,
        set("Corviknight", &["Fly", "High Jump Kick"], None),
        set("Snorlax", &["Harden"], None),
        &[],
    );
    assert!(!must_struggle(&state, &rules, &dex, A));
}

#[test]
fn fairy_lock_keeps_everyone_but_ghosts_and_shed_shell_in() {
    let locked = |species: &str, item: Option<&str>| {
        let (state, rules, dex) = battle(
            Generation::Gen9,
            set(species, &["Harden"], item),
            set("Snorlax", &["Harden"], None),
            &[FieldConditionKind::FairyLock],
        );
        validate_side_choice(&state, &rules, &dex, SideId::P1, &switch_choice())
    };
    assert!(matches!(
        locked("Mew", None),
        Err(ChoiceError::TrappedByField {
            by: FieldConditionKind::FairyLock,
            ..
        })
    ));
    assert_eq!(locked("Gengar", None), Ok(()));
    assert_eq!(locked("Mew", Some("Shed Shell")), Ok(()));
}
