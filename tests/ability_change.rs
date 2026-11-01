//! L1 rule tests for ability changes the frames cannot tell (DESIGN.md R205, R225): the
//! abilities stay the same whether the move works or fails, so only the trace shows it.
//! - Skill Swap and Role Play between two of the same ability (§41.10 A6): Gen 3-4 cartridges let
//!   both work; Showdown fails Skill Swap through Gen 5 and Role Play from Gen 5 on.
//! - Entrainment onto the same ability, Simple Beam onto Simple, and either onto Truant (whose
//!   own behavior is not implemented, so no scenario holds it) fail; Doodle passes over a user
//!   that has the ability already.
//! - Mummy gives nothing to an attacker with Mummy already, nor to one the hit's Rocky Helmet
//!   knocked out (`setAbility` needs HP; the attacker gets its ability back as it faints anyway).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, PokemonState, SideState};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

fn set(species: &str, ability: &str, moves: [&str; 1]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves);
    set.ability = Some(ability.into());
    set.level = 50;
    set
}

/// The trace of one turn: the user uses its one move on the target, which hardens.
fn one_turn(generation: Generation, user: PokemonSet, target: PokemonSet) -> Vec<TraceEvent> {
    one_turn_from(generation, user, target, |_| {})
}

/// `one_turn`, the two Pokémon set up by `prepare` first (`[user, target]`).
fn one_turn_from(
    generation: Generation,
    user: PokemonSet,
    target: PokemonSet,
    prepare: impl FnOnce(&mut [PokemonState]),
) -> Vec<TraceEvent> {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
    let mut pokemon = vec![
        build(USER, SideId::P1, user),
        build(TARGET, SideId::P2, target),
    ];
    prepare(&mut pokemon);
    let side = |id| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(USER), side(TARGET)], 1);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine.run_lead_switch_ins();
    let start = engine.trace().events().len();
    let first_move = |actor| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![first_move(USER)],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![first_move(TARGET)],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    engine.trace().events()[start..].to_vec()
}

/// One turn: Gyarados uses `user_move` on Arcanine, both with Intimidate.
fn same_ability_turn(generation: Generation, user_move: &str) -> Vec<TraceEvent> {
    one_turn(
        generation,
        set("Gyarados", "Intimidate", [user_move]),
        set("Arcanine", "Intimidate", ["Harden"]),
    )
}

/// Whether the target's ability changed in the turn.
fn target_changed(events: &[TraceEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            TraceEvent::AbilityChanged {
                pokemon: TARGET,
                ..
            }
        )
    })
}

#[test]
fn skill_swap_on_the_same_ability_fails_in_gen_5_only() {
    for generation in [
        Generation::Gen3,
        Generation::Gen4,
        Generation::Gen5,
        Generation::Gen6,
        Generation::Gen9,
    ] {
        let swapped = same_ability_turn(generation, "Skill Swap")
            .iter()
            .any(|event| matches!(event, TraceEvent::AbilitiesSwapped { .. }));
        assert_eq!(swapped, generation != Generation::Gen5, "{generation:?}");
    }
}

#[test]
fn role_play_on_the_same_ability_works_through_gen_4() {
    for generation in [
        Generation::Gen3,
        Generation::Gen4,
        Generation::Gen5,
        Generation::Gen9,
    ] {
        let copied = same_ability_turn(generation, "Role Play")
            .iter()
            .any(|event| matches!(event, TraceEvent::AbilityChanged { pokemon: USER, .. }));
        assert_eq!(copied, generation <= Generation::Gen4, "{generation:?}");
    }
}

#[test]
fn entrainment_fails_on_the_same_ability_and_on_truant() {
    for generation in [Generation::Gen5, Generation::Gen9] {
        assert!(!target_changed(&same_ability_turn(
            generation,
            "Entrainment"
        )));
        assert!(!target_changed(&one_turn(
            generation,
            set("Gyarados", "Intimidate", ["Entrainment"]),
            set("Slaking", "Truant", ["Harden"]),
        )));
        assert!(target_changed(&one_turn(
            generation,
            set("Gyarados", "Intimidate", ["Entrainment"]),
            set("Snorlax", "Thick Fat", ["Harden"]),
        )));
    }
}

#[test]
fn simple_beam_fails_on_simple_and_on_truant() {
    for generation in [Generation::Gen5, Generation::Gen9] {
        for (species, ability, changes) in [
            ("Blissey", "Simple", false),
            ("Slaking", "Truant", false),
            ("Snorlax", "Thick Fat", true),
        ] {
            let events = one_turn(
                generation,
                set("Chansey", "Natural Cure", ["Simple Beam"]),
                set(species, ability, ["Harden"]),
            );
            assert_eq!(target_changed(&events), changes, "{generation:?} {ability}");
        }
    }
}

#[test]
fn doodle_passes_over_a_user_with_the_ability_already() {
    let user_changed = |events: &[TraceEvent]| {
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::AbilityChanged { pokemon: USER, .. }))
    };
    assert!(!user_changed(&same_ability_turn(
        Generation::Gen9,
        "Doodle"
    )));
    assert!(user_changed(&one_turn(
        Generation::Gen9,
        set("Gyarados", "Intimidate", ["Doodle"]),
        set("Snorlax", "Thick Fat", ["Harden"]),
    )));
}

#[test]
fn mummy_passes_over_a_mummy_and_a_knocked_out_attacker() {
    let user_changed = |events: &[TraceEvent]| {
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::AbilityChanged { pokemon: USER, .. }))
    };
    let holder = || {
        let mut holder = set("Snorlax", "Mummy", ["Harden"]);
        holder.item = Some("Rocky Helmet".into());
        holder
    };
    assert!(!user_changed(&one_turn(
        Generation::Gen9,
        set("Gyarados", "Mummy", ["Waterfall"]),
        holder(),
    )));
    assert!(!user_changed(&one_turn_from(
        Generation::Gen9,
        set("Gyarados", "Intimidate", ["Waterfall"]),
        holder(),
        |pokemon| pokemon[0].hp = 1,
    )));
    assert!(user_changed(&one_turn(
        Generation::Gen9,
        set("Gyarados", "Intimidate", ["Waterfall"]),
        holder(),
    )));
}
