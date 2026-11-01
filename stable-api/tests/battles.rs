//! Whole battles through the stable API alone (DESIGN.md §47.0 items 10-11, M10.4b): a policy
//! that reads nothing but the requests and `choice_options` plays real formats' battles — a
//! singles and a doubles one — to their end, and a rollout forked by clone replays the same.

mod common;

use common::{Policy, started};
use pokemon_battle_engine::TraceMode;

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
