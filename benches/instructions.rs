//! The workloads whose instruction counts CI holds to a baseline (DESIGN.md §48.0 item 5, the
//! production criteria's benchmark threshold): `tools/bench_instructions.py` runs each under
//! Valgrind's cachegrind, once with the workload and once with its setup alone, and the difference
//! is the workload's count — instructions, unlike time, do not move with the machine's load.
//!
//! Each of the M10 benches' formats (`benches/engine.rs`) has four workloads, the trace off:
//!
//! - `battle`: whole battles from its first turn, policies of seeds 0..`BATTLES`;
//! - `rollout`: policies of seeds 0..`ROLLOUTS` played out from its middle turn;
//! - `snapshot`: `SNAPSHOTS` snapshots of the middle turn, each restored;
//! - `json`: `JSONS` times the middle turn's snapshot written as JSON and read back (DESIGN.md
//!   §48.2).
//!
//! The counts make each workload outweigh its setup (loading the format data, validating the
//! teams), so the difference of the two runs is mostly the workload's.
//!
//! `instructions <format> <workload>` runs one; `instructions <format> setup` runs the setup
//! alone; with no workload named (`cargo test --benches`) it runs every one at a single
//! iteration, a smoke test.

#[path = "../tests/support/policy.rs"]
mod policy;

use std::hint::black_box;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::engine::{BattleSnapshot, TraceMode};
use pokemon_battle_engine::ids::SideId;
use policy::{Policy, advance, play_out, start_battle};

const FORMATS: [&str; 4] = ["gen9ou", "gen9vgc2025regi", "gen4ou", "gen1ou"];
const WORKLOADS: [&str; 4] = ["battle", "rollout", "snapshot", "json"];
const MID_TURN: u32 = 10;
const SEED: u64 = 1;
const BATTLES: u64 = 400;
const ROLLOUTS: u64 = 400;
const SNAPSHOTS: u64 = 32768;
const JSONS: u64 = 512;

/// The benched battle at its start and at its middle turn's request (as `benches/engine.rs`
/// finds it), with the rules and dex to restore it.
struct Setup {
    restore: Box<dyn Fn(&BattleSnapshot) -> BattleEngine>,
    start: BattleSnapshot,
    mid: BattleSnapshot,
}

fn setup(format_id: &str) -> Setup {
    let (rules, dex, start) = start_battle(format_id);
    let restore = move |snapshot: &BattleSnapshot| {
        let mut engine = BattleEngine::restore(rules.clone(), dex.clone(), snapshot.clone())
            .expect("a snapshot of this battle");
        engine.set_trace_mode(TraceMode::Off);
        engine
    };
    let turns = play_out(&mut restore(&start), &mut Policy(SEED));
    let mid_turn = MID_TURN.min(turns / 2).max(1);
    let mut engine = restore(&start);
    let mut policy = Policy(SEED);
    loop {
        assert!(
            advance(&mut engine, &mut policy).is_some(),
            "{format_id}: ended early"
        );
        if engine.state().turn >= mid_turn {
            break;
        }
        let choices = [
            policy.turn_choice(&engine, SideId::P1),
            policy.turn_choice(&engine, SideId::P2),
        ];
        engine
            .submit_turn_choices(choices)
            .expect("valid turn choices");
    }
    let mid = engine.snapshot();
    Setup {
        restore: Box::new(restore),
        start,
        mid,
    }
}

fn run(setup: &Setup, workload: &str, full: bool) {
    let times = |count: u64| if full { count } else { 1 };
    match workload {
        "battle" => {
            for seed in 0..times(BATTLES) {
                black_box(play_out(
                    &mut (setup.restore)(&setup.start),
                    &mut Policy(seed),
                ));
            }
        }
        "rollout" => {
            for seed in 0..times(ROLLOUTS) {
                black_box(play_out(
                    &mut (setup.restore)(&setup.mid),
                    &mut Policy(seed),
                ));
            }
        }
        "snapshot" => {
            let engine = (setup.restore)(&setup.mid);
            for _ in 0..times(SNAPSHOTS) {
                black_box((setup.restore)(&black_box(&engine).snapshot()));
            }
        }
        "json" => {
            for _ in 0..times(JSONS) {
                let json = black_box(&setup.mid).to_json();
                black_box(BattleSnapshot::from_json(&json).expect("its own JSON"));
            }
        }
        other => panic!("no workload {other}: {WORKLOADS:?} or setup"),
    }
}

fn main() {
    // `cargo test --benches` passes its own flags; only plain arguments name a workload.
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    match args.as_slice() {
        [format_id, workload] => {
            let setup = setup(format_id);
            if workload != "setup" {
                run(&setup, workload, true);
            }
        }
        [] => {
            for format_id in FORMATS {
                let setup = setup(format_id);
                for workload in WORKLOADS {
                    run(&setup, workload, false);
                }
            }
        }
        _ => panic!("usage: instructions [<format> <workload | setup>]"),
    }
}
