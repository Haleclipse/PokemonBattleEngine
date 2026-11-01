//! L1 tests for Foresight, Odor Sleuth and Miracle Eye (DESIGN.md §41.5): what the replay cannot
//! see. An accuracy check's rate decides nothing a log shows (the replay forces each hit or
//! miss), so how an identified target's evasion stage counts — Gen 2 neither stage once its
//! evasion reaches the user's accuracy (issues.md I87), Gen 3 none of it (I88), Gen 4+ only a
//! lowered one — is pinned here; and whether identifying again succeeds (Gen 3-4, I89) leaves
//! no trace in a frame.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, VolatileConditionKind};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// Jolteon (A, `moves`) against Snorlax (B, Harden) at level 50; B carries `marks`.
fn battle(
    generation: Generation,
    moves: &[&str],
    marks: &[VolatileConditionKind],
) -> (
    BattleState,
    Arc<pokemon_battle_engine::rules::ResolvedRules>,
    Arc<pokemon_battle_engine::dex::ResolvedDex>,
) {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let set = |species: &str, moves: &[&str]| {
        let mut set = PokemonSet::new(species, moves.iter().copied());
        set.level = 50;
        set
    };
    let mut b = build_pokemon(&dex, B, SideId::P2, &set("Snorlax", &["Harden"])).unwrap();
    for mark in marks {
        b.volatiles.set_marker(*mark);
    }
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &set("Jolteon", moves)).unwrap(),
        b,
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    (BattleState::new(pokemon, [side(A), side(B)], 1), rules, dex)
}

/// The accuracy check of A's `move_name` on B at the given stages.
fn accuracy(
    generation: Generation,
    move_name: &str,
    (accuracy_stage, evasion_stage): (i8, i8),
    marks: &[VolatileConditionKind],
) -> Option<(u16, u16)> {
    let (mut state, rules, dex) = battle(generation, &[move_name], marks);
    state.pokemon_mut(A).unwrap().stages.accuracy = accuracy_stage;
    state.pokemon_mut(B).unwrap().stages.evasion = evasion_stage;
    let move_id = state.pokemon(A).unwrap().moves[0].move_id;
    let move_def = dex.move_by_id(move_id).unwrap().clone();
    pokemon_battle_engine::engine::accuracy_roll(&state, &rules, A, B, &move_def)
}

const FORESIGHT: &[VolatileConditionKind] = &[VolatileConditionKind::Foresight];
const MIRACLE_EYE: &[VolatileConditionKind] = &[VolatileConditionKind::MiracleEye];

#[test]
fn gen4_and_later_ignore_a_raised_evasion_of_an_identified_target_only() {
    for generation in [Generation::Gen4, Generation::Gen7] {
        for marks in [FORESIGHT, MIRACLE_EYE] {
            // Dynamic Punch (50 %): +2 evasion sets aside, -1 still counts.
            assert_eq!(
                accuracy(generation, "Dynamic Punch", (0, 2), marks),
                Some((50, 100)),
                "{generation:?} {marks:?}"
            );
            assert_eq!(
                accuracy(generation, "Dynamic Punch", (0, -1), marks),
                Some((66, 100)),
                "{generation:?} {marks:?}"
            );
        }
        assert_eq!(
            accuracy(generation, "Dynamic Punch", (0, 2), &[]),
            Some((30, 100))
        );
    }
}

#[test]
fn gen3_foresight_sets_the_whole_evasion_stage_aside() {
    assert_eq!(
        accuracy(Generation::Gen3, "Dynamic Punch", (0, 2), FORESIGHT),
        Some((50, 100))
    );
    assert_eq!(
        accuracy(Generation::Gen3, "Dynamic Punch", (0, -1), FORESIGHT),
        Some((50, 100))
    );
    assert_eq!(
        accuracy(Generation::Gen3, "Dynamic Punch", (0, -1), &[]),
        Some((66, 100))
    );
    // The user's own accuracy stage still counts.
    assert_eq!(
        accuracy(Generation::Gen3, "Dynamic Punch", (1, 2), FORESIGHT),
        Some((66, 100))
    );
}

#[test]
fn gen2_foresight_drops_both_stages_once_evasion_reaches_accuracy() {
    let base = accuracy(Generation::Gen2, "Dynamic Punch", (0, 0), &[]);
    // Evasion at or above accuracy: neither stage.
    assert_eq!(
        accuracy(Generation::Gen2, "Dynamic Punch", (0, 1), FORESIGHT),
        base
    );
    assert_eq!(
        accuracy(Generation::Gen2, "Dynamic Punch", (2, 2), FORESIGHT),
        base
    );
    assert_ne!(
        accuracy(Generation::Gen2, "Dynamic Punch", (0, 1), &[]),
        base
    );
    // Below the user's accuracy: both count, as without Foresight.
    assert_eq!(
        accuracy(Generation::Gen2, "Dynamic Punch", (2, 1), FORESIGHT),
        accuracy(Generation::Gen2, "Dynamic Punch", (2, 1), &[])
    );
    assert_ne!(
        accuracy(Generation::Gen2, "Dynamic Punch", (2, 1), FORESIGHT),
        base
    );
    // A move of 100 % then stays at 255 and draws nothing.
    assert_eq!(
        accuracy(Generation::Gen2, "Body Slam", (0, 3), FORESIGHT),
        None
    );
    assert!(accuracy(Generation::Gen2, "Body Slam", (0, 3), &[]).is_some());
}

/// A (Jolteon) uses `move_name` on B (Snorlax, Harden), which carries `marks`: whether the move
/// failed, and B's marks afterwards.
fn identify_again(
    generation: Generation,
    move_name: &str,
    marks: &[VolatileConditionKind],
) -> (bool, bool, bool) {
    let (state, rules, dex) = battle(generation, &[move_name], marks);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let choice = |side: SideId, actor: PokemonId, target: SideId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: Some(DeclaredTarget(
                    rules.topology().position(target, 0).unwrap(),
                )),
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, A, SideId::P2),
            choice(SideId::P2, B, SideId::P1),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert!(engine.state().rng.semantic_tape_exhausted());
    let failed = engine
        .trace()
        .events()
        .iter()
        .any(|event| matches!(event, TraceEvent::MoveFailed { actor: A, .. }));
    let b = engine.state().pokemon(B).unwrap();
    (
        failed,
        b.volatiles.contains(VolatileConditionKind::Foresight),
        b.volatiles.contains(VolatileConditionKind::MiracleEye),
    )
}

#[test]
fn gen3_4_identify_again_and_across_where_gen2_and_gen5_fail() {
    // Again on a target it identified: Gen 3-4 succeed (issues.md I89), Gen 2 and Gen 7 fail.
    assert!(!identify_again(Generation::Gen3, "Foresight", FORESIGHT).0);
    assert!(!identify_again(Generation::Gen4, "Odor Sleuth", FORESIGHT).0);
    assert!(identify_again(Generation::Gen2, "Foresight", FORESIGHT).0);
    assert!(identify_again(Generation::Gen7, "Foresight", FORESIGHT).0);
    // Across: Gen 4 Foresight on a Miracle-Eyed target adds its own mark; Gen 7 fails.
    assert_eq!(
        identify_again(Generation::Gen4, "Foresight", MIRACLE_EYE),
        (false, true, true)
    );
    assert_eq!(
        identify_again(Generation::Gen7, "Foresight", MIRACLE_EYE),
        (true, false, true)
    );
    assert_eq!(
        identify_again(Generation::Gen7, "Miracle Eye", FORESIGHT),
        (true, true, false)
    );
}
