//! M10 benches (DESIGN.md §47.2): the engine's cost on battles of real formats. Each format's
//! first two teams of `tests/fixtures/formats/built.json` (teams Showdown admits) meet through the
//! pre-battle session; a seeded random policy (`tests/support/policy.rs`) then plays them — every
//! choice it makes is one `validate_choice` accepts, moves nine times in ten — so a bench replays
//! the very same battle on every iteration.
//!
//! - `battle`: a whole battle from the first turn.
//! - `turn`: one turn from the middle (the tenth, or half the battle), its choices fixed.
//! - `snapshot`, `restore`: a snapshot of that turn (its state, not its trace), and a battle back
//!   from it.
//! - `fork`, `rewind`: the stable API's two ways back to that turn (DESIGN.md §47.5 item 8) — a
//!   clone of the battle, and an engine put back at the snapshot in place.
//! - `view`, `state`: both teams' HP and Speed stages at that turn, read through the omniscient
//!   `BattleView` and straight off the state — what the view's borrowing costs (§47.5 item 2).
//! - `rollout`: sixteen policies (seeds 0..16) played from that turn to the end, as a search
//!   would.
//! - `choose`: the policy's own choice of both sides at that turn — its share of `battle` and
//!   `rollout`, which are not the engine's.
//! - `battle-off`, `rollout-off`: the same with the trace off (`TraceMode::Off`, §47.0 item 5).
//!
//! `cargo bench --bench engine` measures; `cargo test --benches` runs each once.

#[path = "../tests/support/policy.rs"]
mod policy;

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::ResolvedDex;
use pokemon_battle_engine::engine::{BattleSnapshot, SideChoice, TraceMode};
use pokemon_battle_engine::ids::SideId;
use pokemon_battle_engine::rules::ResolvedRules;
use pokemon_battle_engine::state::Stat;
use pokemon_battle_engine::view::Hp;
use policy::{Policy, advance, answer, play_out, start_battle};

/// The formats benched: the newest singles and doubles, and two older generations' rules.
const FORMATS: [&str; 4] = ["gen9ou", "gen9vgc2025regi", "gen4ou", "gen1ou"];
/// The latest turn `turn`, `snapshot`, `restore` and `rollout` start from: the tenth, or the
/// battle's middle when it is shorter.
const MID_TURN: u32 = 10;
/// The policy's seed for the benched battle.
const SEED: u64 = 1;
const ROLLOUTS: u64 = 16;

/// One format's benched battle: its rules and dex, the snapshot at the first turn, and at the
/// middle turn's request with the choices the policy makes there.
struct Bench {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    start: BattleSnapshot,
    mid: BattleSnapshot,
    mid_choices: [SideChoice; 2],
    mid_turn: u32,
    /// The events the battle's trace held by then (snapshots no longer carry it, §47.0 item 6).
    mid_trace: usize,
    turns: u32,
}

impl Bench {
    fn new(format_id: &str) -> Self {
        let (rules, dex, start) = start_battle(format_id);
        let restore = |snapshot: &BattleSnapshot| {
            BattleEngine::restore(rules.clone(), dex.clone(), snapshot.clone())
                .expect("a snapshot of this battle")
        };
        let turns = play_out(&mut restore(&start), &mut Policy(SEED));
        let mid_turn = MID_TURN.min(turns / 2).max(1);
        let mut engine = restore(&start);
        let mut policy = Policy(SEED);
        // Up to the middle turn's request, unanswered.
        loop {
            assert!(
                advance(&mut engine, &mut policy).is_some(),
                "{format_id}: the battle ends before turn {mid_turn}"
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
        let mid_choices = [
            policy.turn_choice(&engine, SideId::P1),
            policy.turn_choice(&engine, SideId::P2),
        ];
        let mid = engine.snapshot();
        let mid_trace = engine.trace().events().len();
        Self {
            rules,
            dex,
            start,
            mid,
            mid_choices,
            mid_turn,
            mid_trace,
            turns,
        }
    }

    fn restore(&self, snapshot: &BattleSnapshot) -> BattleEngine {
        BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot.clone())
            .expect("a snapshot of this battle")
    }

    /// `restore` with the trace off.
    fn restore_quiet(&self, snapshot: &BattleSnapshot) -> BattleEngine {
        let mut engine = self.restore(snapshot);
        engine.set_trace_mode(TraceMode::Off);
        engine
    }
}

fn benches(c: &mut Criterion) {
    for format_id in FORMATS {
        let bench = Bench::new(format_id);
        eprintln!(
            "{format_id}: {} turns; trace {} events by turn {}",
            bench.turns, bench.mid_trace, bench.mid_turn,
        );
        let mut group = c.benchmark_group(format_id);
        group.sample_size(20);
        group.bench_function("battle", |b| {
            b.iter_batched(
                || bench.restore(&bench.start),
                |mut engine| play_out(&mut engine, &mut Policy(SEED)),
                BatchSize::LargeInput,
            )
        });
        group.bench_function("battle-off", |b| {
            b.iter_batched(
                || bench.restore_quiet(&bench.start),
                |mut engine| play_out(&mut engine, &mut Policy(SEED)),
                BatchSize::LargeInput,
            )
        });
        group.bench_function("turn", |b| {
            b.iter_batched(
                || (bench.restore(&bench.mid), bench.mid_choices.clone()),
                |(mut engine, choices)| {
                    engine.submit_turn_choices(choices).expect("valid choices");
                    answer(&mut engine, &mut Policy(SEED));
                    engine
                },
                BatchSize::LargeInput,
            )
        });
        let engine = bench.restore(&bench.mid);
        group.bench_function("snapshot", |b| b.iter(|| black_box(&engine).snapshot()));
        group.bench_function("choose", |b| {
            b.iter(|| {
                let mut policy = Policy(SEED);
                [
                    policy.turn_choice(black_box(&engine), SideId::P1),
                    policy.turn_choice(black_box(&engine), SideId::P2),
                ]
            })
        });
        group.bench_function("fork", |b| b.iter(|| black_box(&engine).clone()));
        group.bench_function("view", |b| {
            b.iter(|| {
                let view = black_box(&engine).omniscient_view();
                [SideId::P1, SideId::P2]
                    .iter()
                    .map(|side| {
                        view.side(*side)
                            .team()
                            .map(|pokemon| {
                                let hp = match pokemon.hp() {
                                    Hp::Exact { hp, .. } => u32::from(hp),
                                    Hp::Percent(percent) => u32::from(percent),
                                    Hp::Pixels(pixels) => u32::from(pixels),
                                };
                                hp + u32::from(pokemon.boost(Stat::Speed).unsigned_abs())
                            })
                            .sum::<u32>()
                    })
                    .sum::<u32>()
            })
        });
        group.bench_function("state", |b| {
            b.iter(|| {
                let state = black_box(&engine).state();
                [SideId::P1, SideId::P2]
                    .iter()
                    .map(|side| {
                        state
                            .side(*side)
                            .team
                            .iter()
                            .filter_map(|id| state.pokemon(*id))
                            .map(|pokemon| {
                                u32::from(pokemon.hp)
                                    + u32::from(pokemon.stages.get(Stat::Speed).unsigned_abs())
                            })
                            .sum::<u32>()
                    })
                    .sum::<u32>()
            })
        });
        let mut rewound = bench.restore(&bench.mid);
        group.bench_function("rewind", |b| {
            b.iter(|| {
                rewound
                    .rewind(black_box(&bench.mid))
                    .expect("a snapshot of this battle")
            })
        });
        group.bench_function("restore", |b| {
            b.iter_batched(
                || bench.mid.clone(),
                |snapshot| {
                    BattleEngine::restore(bench.rules.clone(), bench.dex.clone(), snapshot)
                        .expect("a snapshot of this battle")
                },
                BatchSize::LargeInput,
            )
        });
        group.throughput(Throughput::Elements(ROLLOUTS));
        group.bench_function("rollout", |b| {
            b.iter(|| {
                (0..ROLLOUTS)
                    .map(|seed| play_out(&mut bench.restore(&bench.mid), &mut Policy(seed)))
                    .sum::<u32>()
            })
        });
        group.bench_function("rollout-off", |b| {
            b.iter(|| {
                (0..ROLLOUTS)
                    .map(|seed| play_out(&mut bench.restore_quiet(&bench.mid), &mut Policy(seed)))
                    .sum::<u32>()
            })
        });
        group.finish();
    }
}

criterion_group!(engine, benches);
criterion_main!(engine);
