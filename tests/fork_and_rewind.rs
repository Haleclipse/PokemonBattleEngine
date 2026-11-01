//! Forking a battle and rewinding one (DESIGN.md §47.5 item 8, M10.4a): a clone is a battle of
//! its own from the same moment, and `rewind` puts an engine back at a snapshot exactly as
//! `restore` builds one from it — so a rollout never has to take a snapshot.

#[path = "support/policy.rs"]
mod policy;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::engine::{BattleInitError, TraceMode};
use pokemon_battle_engine::state::BattleState;
use policy::{Policy, answer, play_out, start_battle};

/// The states at every request of `engine`'s battle, played to the end by `policy`.
fn states(engine: &mut BattleEngine, policy: &mut Policy) -> Vec<BattleState> {
    let mut states = vec![engine.state().clone()];
    while answer(engine, policy).is_some() {
        states.push(engine.state().clone());
    }
    states
}

/// A gen9ou battle at its tenth turn's request, with the policy that got it there.
fn mid_battle() -> (BattleEngine, Policy) {
    let (rules, dex, start) = start_battle("gen9ou");
    let mut engine = BattleEngine::restore(rules, dex, start).expect("the battle's start");
    let mut policy = Policy(1);
    while engine.state().turn < 10 {
        answer(&mut engine, &mut policy).expect("a battle longer than ten turns");
    }
    (engine, policy)
}

#[test]
fn a_clone_plays_on_as_the_battle_it_was_cloned_from() {
    let (mut engine, policy) = mid_battle();
    let mut fork = engine.clone();
    let played = states(&mut engine, &mut Policy(policy.0));
    let forked = states(&mut fork, &mut Policy(policy.0));
    assert!(played.len() > 1, "the battle went on");
    assert!(played == forked, "the clone played another battle");
}

#[test]
fn a_clone_leaves_the_battle_it_was_cloned_from_alone() {
    let (engine, policy) = mid_battle();
    let before = engine.state().clone();
    let mut fork = engine.clone();
    fork.set_trace_mode(TraceMode::Off);
    play_out(&mut fork, &mut Policy(policy.0));
    assert!(engine.state() == &before, "the fork moved the original");
    assert_eq!(engine.trace_mode(), TraceMode::Full);
}

#[test]
fn a_rewind_is_the_restore_of_the_snapshot() {
    let (rules, dex, _) = start_battle("gen9ou");
    let (mut engine, policy) = mid_battle();
    let snapshot = engine.snapshot();
    engine.set_trace_mode(TraceMode::Off);
    play_out(&mut engine, &mut Policy(policy.0));
    engine.rewind(&snapshot).expect("a snapshot of this battle");
    let restored =
        BattleEngine::restore_with_trace_mode(rules, dex, snapshot, TraceMode::Off).expect("it");
    assert!(engine.state() == restored.state(), "another state");
    assert_eq!(engine.trace_mode(), TraceMode::Off);
    assert!(engine.trace().events().is_empty(), "the trace kept events");
    // And the battle from there is the restored one's.
    let mut restored = restored;
    assert!(
        states(&mut engine, &mut Policy(policy.0)) == states(&mut restored, &mut Policy(policy.0)),
        "the rewound battle played another battle"
    );
}

#[test]
fn a_rewind_refuses_a_snapshot_of_other_rules() {
    let (mut engine, _) = mid_battle();
    let (rules, dex, start) = start_battle("gen9vgc2025regi");
    let other = BattleEngine::restore(rules, dex, start).expect("the other battle");
    assert_eq!(
        engine.rewind(&other.snapshot()),
        Err(BattleInitError::SnapshotRulesMismatch)
    );
}
