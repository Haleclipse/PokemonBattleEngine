//! The trace's modes (DESIGN.md §47.0 item 5, R-M10-4): what a battle records, never how it runs.
//! A real format's battle, played to the end by the seeded random policy of the M10 benches
//! (`tests/support/policy.rs`), in every mode: the battle is the same request after request, a
//! filtered trace is the full one's events of the kept categories, events taken step by step are
//! the full trace, and a trace that is off holds nothing.

#[path = "support/policy.rs"]
mod policy;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::engine::{TraceCategory, TraceEvent, TraceFilter, TraceMode};
use pokemon_battle_engine::ids::SideId;
use pokemon_battle_engine::state::BattleState;
use policy::{Policy, advance, start_battle};

/// A singles and a doubles format of the newest generation, and two older generations' rules.
const FORMATS: [&str; 4] = ["gen9ou", "gen9vgc2025regi", "gen4ou", "gen1ou"];

/// A battle played in `mode`: the state at every request, and the events — the trace's at the
/// end, or (`take`) the ones taken after each request, in order.
fn play(format_id: &str, mode: TraceMode, take: bool) -> (Vec<BattleState>, Vec<TraceEvent>) {
    let (rules, dex, start) = start_battle(format_id);
    let mut engine = BattleEngine::restore(rules, dex, start).expect("the battle's start");
    engine.set_trace_mode(mode);
    // The session's own events before the first request are not this battle's play.
    engine.take_trace_events();
    let mut policy = Policy(1);
    let mut states = Vec::new();
    let mut events = Vec::new();
    while advance(&mut engine, &mut policy).is_some() {
        states.push(engine.state().clone());
        if take {
            events.extend(engine.take_trace_events());
        }
        let choices = [
            policy.turn_choice(&engine, SideId::P1),
            policy.turn_choice(&engine, SideId::P2),
        ];
        engine
            .submit_turn_choices(choices)
            .expect("valid turn choices");
    }
    states.push(engine.state().clone());
    events.extend(engine.take_trace_events());
    (states, events)
}

#[test]
fn the_trace_mode_changes_what_is_recorded_never_the_battle() {
    let outcomes = TraceFilter::new()
        .keep(TraceCategory::Outcome)
        .keep(TraceCategory::Check);
    for format_id in FORMATS {
        let (full_states, full_events) = play(format_id, TraceMode::Full, false);
        assert!(
            full_states.len() > 2 && !full_events.is_empty(),
            "{format_id}: a battle"
        );

        let (states, events) = play(format_id, TraceMode::Filtered(outcomes), false);
        assert!(
            states == full_states,
            "{format_id}: filtered, another battle"
        );
        let kept: Vec<TraceEvent> = full_events
            .iter()
            .filter(|event| outcomes.keeps(event.category()))
            .cloned()
            .collect();
        assert!(events == kept, "{format_id}: the filter kept other events");
        assert!(
            events.len() < full_events.len(),
            "{format_id}: the filter kept everything"
        );

        let (states, events) = play(format_id, TraceMode::Off, false);
        assert!(states == full_states, "{format_id}: off, another battle");
        assert!(events.is_empty(), "{format_id}: an off trace recorded");

        let (states, events) = play(format_id, TraceMode::Full, true);
        assert!(states == full_states, "{format_id}: taken, another battle");
        assert!(
            events == full_events,
            "{format_id}: the events taken step by step are not the trace"
        );
    }
}

/// Every category is something a battle records: the exhaustive `category` files each event
/// somewhere, and a whole battle meets all four.
#[test]
fn a_battle_records_every_category() {
    let (_, events) = play("gen9ou", TraceMode::Full, false);
    for category in [
        TraceCategory::Flow,
        TraceCategory::Outcome,
        TraceCategory::Check,
        TraceCategory::Calculation,
    ] {
        assert!(
            events.iter().any(|event| event.category() == category),
            "no {category:?} event in a whole battle"
        );
    }
}
