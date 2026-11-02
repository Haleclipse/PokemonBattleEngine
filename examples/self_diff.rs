//! The structural refactor's old-against-new check (DESIGN.md §48.3): two builds behave alike on
//! these battles when this prints the same bytes for both.
//!
//! - Every format's battles between pairs of its four teams in `built.json` (each pair in turn,
//!   the session seeded with the battle's seed), played by a seeded policy that picks among `choice_options` and takes the gimmicks (Mega Evolution, Ultra
//!   Burst, Z-Moves, Dynamax, Terastallization) when offered, with the trace in full. At every
//!   request boundary: the request, the trace's events since the last one and the battle's
//!   random state; at the end, the whole state. One line per battle: format, seed, turns,
//!   digest.
//! - Every format's resolved rules and every generation's `GenerationRules`, in full.
//!
//! At every boundary the state must also be one a battle can be in: a copy restores its
//! snapshot, which checks the battle's invariants.
//!
//! The replays against Showdown compare what Showdown's log shows; this sees what it does not —
//! the engine's own event order, the draws no log line shows.
//!
//! Run: cargo run --release --example self_diff [-- <battles per format, default 30>] > out.txt
//! (`scripts/self-diff.sh` runs it, with the replays' digests, in a checkout of the base commit
//! and in this one, and compares). To look into one battle: `SELF_DIFF_ONLY="<format> <seed>"`
//! plays it alone, `SELF_DIFF_TRACE=1` prints its boundaries, choices and events, and
//! `SELF_DIFF_DEBUG=1` the choices `validate_choice` refuses.

#[path = "../tests/support/policy.rs"]
mod policy;

use std::fmt::Write as _;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::digest::sha256_hex;
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, EngineStep, ForcedReplacement, ForcedSwitchChoice, MechanicChoice,
    ReviveChoice, SideChoice, SlotChoice, TraceMode,
};
use pokemon_battle_engine::ids::SideId;
use pokemon_battle_engine::rules::GenerationRules;
use pokemon_battle_engine::session::{BattleSession, SessionFormat, SessionRequest, TeamChoice};
use pokemon_battle_engine::spec::Generation;
use policy::{MAX_TURNS, Policy, built, library, team_set};

const GENERATIONS: [Generation; 9] = [
    Generation::Gen1,
    Generation::Gen2,
    Generation::Gen3,
    Generation::Gen4,
    Generation::Gen5,
    Generation::Gen6,
    Generation::Gen7,
    Generation::Gen8,
    Generation::Gen9,
];

/// The six pairs of a format's four teams.
const PAIRS: [(usize, usize); 6] = [(0, 1), (2, 3), (0, 2), (1, 3), (0, 3), (1, 2)];

/// `format_id`'s battle `seed`: its session seeded with `seed`, between the pair of teams it
/// picks, in their own order, the trace in full.
fn start(format_id: &str, seed: u64) -> BattleEngine {
    let format = SessionFormat::new(library(), format_id).expect("a target format");
    let teams = built()["formats"][format_id]["teams"]
        .as_array()
        .expect("the format's teams");
    let (a, b) = PAIRS[(seed as usize) % PAIRS.len()];
    let rules = std::sync::Arc::new(format.rules().expect("the format's rules"));
    let dex = format.dex().clone();
    let mut session = BattleSession::new(format, seed);
    for (side, team) in [(SideId::P1, &teams[a]), (SideId::P2, &teams[b])] {
        let sets = team["sets"]
            .as_array()
            .expect("a team's sets")
            .iter()
            .map(team_set)
            .collect();
        session
            .register(side, sets)
            .expect("a team Showdown admits");
    }
    if let SessionRequest::Choose { sides, .. } = session.request() {
        for side in sides {
            session
                .choose(side, TeamChoice::default())
                .expect("the team's own order");
        }
    }
    let snapshot = session.engine().expect("the battle").snapshot();
    BattleEngine::restore_with_trace_mode(rules, dex, snapshot, TraceMode::Full)
        .expect("the start restores")
}

/// One actor's candidate actions from its options: every choosable move at each target it
/// accepts, with each gimmick it may take on that move, its switches; or the action it is held
/// to.
fn candidates(actor: &pokemon_battle_engine::engine::ActorOptions) -> Vec<(ActionChoice, bool)> {
    if let Some(locked) = &actor.locked {
        return vec![(locked.clone(), false)];
    }
    let mechanics = &actor.mechanics;
    let mut out = Vec::new();
    for option in actor.moves.iter().filter(|option| option.refused.is_none()) {
        let mut gimmicks: Vec<MechanicChoice> = Vec::new();
        if mechanics.mega.is_some() {
            gimmicks.push(MechanicChoice::MegaEvolve);
        }
        if mechanics.ultra_burst.is_some() {
            gimmicks.push(MechanicChoice::UltraBurst);
        }
        if mechanics
            .z_moves
            .get(option.slot.index())
            .is_some_and(Option::is_some)
        {
            gimmicks.push(MechanicChoice::ZMove);
        }
        if mechanics.dynamax {
            gimmicks.push(MechanicChoice::Dynamax);
        }
        if mechanics.tera.is_some() {
            gimmicks.push(MechanicChoice::Terastallize);
        }
        for target in &option.targets {
            out.push((
                ActionChoice::Move {
                    slot: option.slot,
                    target: *target,
                    mechanic: None,
                },
                false,
            ));
            for gimmick in &gimmicks {
                out.push((
                    ActionChoice::Move {
                        slot: option.slot,
                        target: *target,
                        mechanic: Some(*gimmick),
                    },
                    true,
                ));
            }
        }
    }
    out.extend(
        actor
            .switches
            .iter()
            .map(|pokemon| (ActionChoice::Switch { pokemon: *pokemon }, false)),
    );
    out
}

/// A side's turn choice: per actor, a gimmick three times in ten where one is offered, else a
/// move nine times in ten, else a switch; a combination `validate_choice` refuses (two Mega
/// Evolutions in one turn, a trapped switch) is drawn again, and after 64 draws the benches'
/// policy decides.
fn turn_choice(engine: &BattleEngine, side: SideId, policy: &mut Policy) -> SideChoice {
    let options = engine.choice_options(side).expect("a turn's request");
    let per_actor: Vec<_> = options
        .actors
        .iter()
        .map(|actor| (actor.actor, candidates(actor)))
        .collect();
    for _ in 0..64 {
        let actions = per_actor
            .iter()
            .map(|(actor, list)| {
                let gimmick: Vec<_> = list.iter().filter(|(_, g)| *g).collect();
                let moves: Vec<_> = list
                    .iter()
                    .filter(|(action, g)| !g && matches!(action, ActionChoice::Move { .. }))
                    .collect();
                let others: Vec<_> = list
                    .iter()
                    .filter(|(action, _)| !matches!(action, ActionChoice::Move { .. }))
                    .collect();
                let pool = if !gimmick.is_empty() && policy.below(10) < 3 {
                    gimmick
                } else if !moves.is_empty() && (others.is_empty() || policy.below(10) < 9) {
                    moves
                } else if !others.is_empty() {
                    others
                } else {
                    list.iter().collect()
                };
                SlotChoice {
                    actor: *actor,
                    action: pool[policy.below(pool.len())].0.clone(),
                }
            })
            .collect();
        let choice = SideChoice { side, actions };
        match engine.validate_choice(side, &choice) {
            Ok(()) => return choice,
            Err(error) if std::env::var_os("SELF_DIFF_DEBUG").is_some() => {
                eprintln!(
                    "turn {} {side:?}: {choice:?} -> {error:?}",
                    engine.state().turn
                )
            }
            Err(_) => {}
        }
    }
    if std::env::var_os("SELF_DIFF_DEBUG").is_some() {
        eprintln!("{options:#?}");
    }
    policy.turn_choice(engine, side)
}

/// The battle's digest: its boundaries', then its end's. `None` past `MAX_TURNS`.
fn play(engine: &mut BattleEngine, policy: &mut Policy) -> (u32, String) {
    let mut boundaries = String::new();
    loop {
        let step = engine.step().expect("the engine steps");
        if matches!(step, EngineStep::Progressed { .. }) {
            continue;
        }
        let events = engine.take_trace_events();
        if std::env::var_os("SELF_DIFF_TRACE").is_some() {
            eprintln!("-- turn {} {step:?}", engine.state().turn);
            for event in &events {
                eprintln!("   {event:?}");
            }
        }
        // A state a battle can be in: restoring checks the battle's invariants (two actives
        // the same Pokémon, a fainted one on the field …), on a copy the battle goes on without.
        let mut probe = engine.clone();
        if let Err(error) = probe.rewind(&engine.snapshot()) {
            panic!("turn {}: {error:?}", engine.state().turn);
        }
        let boundary = format!("{step:?}\n{events:?}\n{:?}", engine.state().rng);
        writeln!(boundaries, "{}", sha256_hex(boundary.as_bytes())).expect("a String");
        match step {
            EngineStep::Progressed { .. } => unreachable!(),
            EngineStep::Finished(_) => break,
            EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) => {
                if engine.state().turn > MAX_TURNS {
                    break;
                }
                let choices = [
                    turn_choice(engine, SideId::P1, policy),
                    turn_choice(engine, SideId::P2, policy),
                ];
                if std::env::var_os("SELF_DIFF_TRACE").is_some() {
                    eprintln!("   choices {choices:?}");
                }
                engine
                    .submit_turn_choices(choices)
                    .expect("valid turn choices");
            }
            EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) => {
                let choices = replacements.map(|requirement| {
                    let mut available = requirement.available_pokemon.clone();
                    let replacements = requirement
                        .positions
                        .iter()
                        .take(requirement.required_count)
                        .map(|slot| ForcedReplacement {
                            position: slot.position,
                            pokemon: available.remove(policy.below(available.len())),
                        })
                        .collect();
                    ForcedSwitchChoice {
                        side: requirement.side,
                        replacements,
                    }
                });
                engine
                    .resolve_forced_switches(choices)
                    .expect("valid replacements");
            }
            EngineStep::Awaiting(BattleRequest::Revive { side, candidates }) => {
                let pokemon = candidates[policy.below(candidates.len())];
                engine
                    .resolve_revive(ReviveChoice { side, pokemon })
                    .expect("valid revival");
            }
        }
    }
    let end = format!("{boundaries}{:?}", engine.state());
    (engine.state().turn, sha256_hex(end.as_bytes()))
}

fn main() {
    let battles: u64 = std::env::args()
        .nth(1)
        .map_or(30, |n| n.parse().expect("a number of battles"));
    let formats: Vec<String> = built()["formats"]
        .as_object()
        .expect("built.json's formats")
        .keys()
        .cloned()
        .collect();
    for generation in GENERATIONS {
        let rules = format!("{:?}", GenerationRules::for_generation(generation));
        println!(
            "generation-rules gen{} {}",
            generation.number(),
            sha256_hex(rules.as_bytes())
        );
    }
    for format_id in &formats {
        let format = SessionFormat::new(library(), format_id).expect("a target format");
        let rules = format!("{:?}", format.rules().expect("the format's rules"));
        println!("format-rules {format_id} {}", sha256_hex(rules.as_bytes()));
        for seed in 1..=battles {
            if let Ok(only) = std::env::var("SELF_DIFF_ONLY")
                && only != format!("{format_id} {seed}")
            {
                continue;
            }
            if std::env::var_os("SELF_DIFF_DEBUG").is_some() {
                eprintln!("== {format_id} {seed}");
            }
            let mut engine = start(format_id, seed);
            let mut policy = Policy(seed);
            let (turns, digest) = play(&mut engine, &mut policy);
            println!("battle {format_id} {seed} {turns} {digest}");
        }
    }
}
