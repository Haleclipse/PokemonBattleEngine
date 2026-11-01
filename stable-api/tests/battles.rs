//! Whole battles through the stable API alone (DESIGN.md §47.0 items 10-11, M10.4b): a policy
//! that reads nothing but the requests and `choice_options` plays real formats' battles — a
//! singles and a doubles one — to their end, and a rollout forked by clone replays the same. The
//! same seed and inputs give the same battle; another seed another one (the crate documentation's
//! "Randomness and reproducibility").

mod common;

use common::{Policy, next_request, started, started_seeded};
use pokemon_battle_engine::{BattleRequest, EngineStep, SideId, TraceMode};

#[test]
fn a_battle_plays_to_its_end_on_its_options() {
    for format_id in ["gen9ou", "gen9vgc2025regi"] {
        let mut engine = started(format_id);
        engine.set_trace_mode(TraceMode::Off);
        let (outcome, turns) = Policy(11).play_out(&mut engine, 1000);
        assert!(outcome.is_some(), "{format_id}: no end in {turns} turns");
        assert!(turns > 1, "{format_id}: {turns} turns");
    }
}

#[test]
fn a_rollout_forked_by_clone_plays_the_same_twice() {
    let mut engine = started("gen9ou");
    // Five turns in, the rollout's root.
    let (outcome, _) = Policy(5).play_out(&mut engine, 5);
    assert!(outcome.is_none(), "over before the rollout");
    let mut first = engine.clone();
    let mut second = engine.clone();
    first.set_trace_mode(TraceMode::Off);
    second.set_trace_mode(TraceMode::Off);
    let a = Policy(99).play_out(&mut first, 1000);
    let b = Policy(99).play_out(&mut second, 1000);
    assert_eq!(a, b);
    assert!(a.0.is_some(), "the rollout ends");
}

/// A whole battle of `format_id` from a session seeded `seed`, the policy's seed `policy`: every
/// request it makes and what each side may choose at every turn, then its outcome.
fn transcript(format_id: &str, seed: u64, policy: u64) -> Vec<String> {
    let mut engine = started_seeded(format_id, seed);
    engine.set_trace_mode(TraceMode::Off);
    let mut policy = Policy(policy);
    let mut lines = Vec::new();
    for _ in 0..1000 {
        let step = next_request(&mut engine);
        lines.push(format!("{step:?}"));
        match step {
            EngineStep::Finished(_) => return lines,
            EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) => {
                for side in [SideId::P1, SideId::P2] {
                    lines.push(format!("{:?}", engine.choice_options(side)));
                }
            }
            _ => {}
        }
        let (outcome, _) = policy.play_out(&mut engine, 1);
        if let Some(outcome) = outcome {
            lines.push(format!("{outcome:?}"));
            return lines;
        }
    }
    panic!("{format_id}: no end in 1000 turns");
}

#[test]
fn the_same_seed_and_inputs_give_the_same_battle() {
    for format_id in ["gen9ou", "gen9vgc2025regi"] {
        let first = transcript(format_id, 2026, 3);
        assert_eq!(first, transcript(format_id, 2026, 3), "{format_id}");
        // Not a vacuous equality: another session seed, the same inputs, another battle.
        assert_ne!(first, transcript(format_id, 2027, 3), "{format_id}");
    }
}
