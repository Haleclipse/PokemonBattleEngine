//! A seeded random policy that plays a real format's battle to the end, every choice one the
//! engine's `validate_choice` accepts (DESIGN.md §47.2): the M10 benches' workload and the
//! trace-mode tests' battles. Included by path (`#[path = "…/support/policy.rs"] mod policy;`),
//! apart from the replay harness, so neither compiles the other.
#![allow(dead_code)]

use std::sync::{Arc, OnceLock};

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::ResolvedDex;
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, BattleSnapshot, EngineStep, ForcedReplacement, ForcedSwitchChoice,
    ReviveChoice, SideChoice, SlotChoice,
};
use pokemon_battle_engine::format::{FormatLibrary, Stats, TeamSet};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules};
use pokemon_battle_engine::session::{BattleSession, SessionFormat, SessionRequest, TeamChoice};
use serde_json::Value;

/// A battle the policy has not finished by then is cut (Endless Battle Clause ends one at 1000).
pub const MAX_TURNS: u32 = 1000;

pub fn library() -> &'static FormatLibrary {
    static LIBRARY: OnceLock<FormatLibrary> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        FormatLibrary::open(concat!(env!("CARGO_MANIFEST_DIR"), "/data"))
            .unwrap_or_else(|error| panic!("{error}"))
    })
}

pub fn built() -> &'static Value {
    static BUILT: OnceLock<Value> = OnceLock::new();
    BUILT.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/formats/built.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("built.json"))
            .expect("built.json is JSON")
    })
}

/// A set as the export writes it (Showdown's `PokemonSet` JSON), as `tests/session.rs` reads it.
pub fn team_set(input: &Value) -> TeamSet {
    let text = |field: &str| input[field].as_str().unwrap_or_default().to_string();
    let stats = |value: &Value| {
        value.as_object().map(|stats| {
            let stat = |name: &str| stats[name].as_i64().unwrap_or(0) as i32;
            Stats {
                hp: stat("hp"),
                atk: stat("atk"),
                def: stat("def"),
                spa: stat("spa"),
                spd: stat("spd"),
                spe: stat("spe"),
            }
        })
    };
    TeamSet {
        name: text("name"),
        species: text("species"),
        item: text("item"),
        ability: text("ability"),
        moves: input["moves"]
            .as_array()
            .map(|moves| {
                moves
                    .iter()
                    .filter_map(|m| m.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        nature: text("nature"),
        gender: text("gender"),
        evs: stats(&input["evs"]),
        ivs: stats(&input["ivs"]),
        level: input["level"].as_u64().unwrap_or(0) as u32,
        shiny: input["shiny"].as_bool().unwrap_or(false),
        happiness: input["happiness"].as_f64(),
        hp_type: text("hpType"),
        gigantamax: input["gigantamax"].as_bool().unwrap_or(false),
        tera_type: text("teraType"),
    }
}

/// SplitMix64: the policy's own draws, apart from the battle's.
pub struct Policy(pub u64);

impl Policy {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// A side's turn choice `validate_choice` accepts: random picks first (a move nine times in
    /// ten), then the first valid combination in order.
    pub fn turn_choice(&mut self, engine: &BattleEngine, side: SideId) -> SideChoice {
        let state = engine.state();
        let actors: Vec<PokemonId> = state
            .side(side)
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|id| state.pokemon(*id).is_some_and(|p| !p.is_fainted()))
            .collect();
        let targets: Vec<Option<DeclaredTarget>> = std::iter::once(None)
            .chain(
                engine
                    .rules()
                    .topology()
                    .positions()
                    .iter()
                    .map(|position| Some(DeclaredTarget(position.id))),
            )
            .collect();
        let bench = state.healthy_bench(side);
        let options: Vec<(Vec<ActionChoice>, Vec<ActionChoice>)> = actors
            .iter()
            .map(|actor| {
                let slots = state.pokemon(*actor).map_or(0, |p| p.move_slots().len());
                let moves: Vec<ActionChoice> = (0..slots)
                    .filter_map(|index| MoveSlot::new(index as u8))
                    .flat_map(|slot| {
                        targets.iter().map(move |target| ActionChoice::Move {
                            slot,
                            target: *target,
                            mechanic: None,
                        })
                    })
                    .collect();
                let others: Vec<ActionChoice> = bench
                    .iter()
                    .map(|pokemon| ActionChoice::Switch { pokemon: *pokemon })
                    .chain(std::iter::once(ActionChoice::Pass))
                    .collect();
                (moves, others)
            })
            .collect();
        let choice = |picks: Vec<ActionChoice>| SideChoice {
            side,
            actions: actors
                .iter()
                .zip(picks)
                .map(|(actor, action)| SlotChoice {
                    actor: *actor,
                    action,
                })
                .collect(),
        };
        for _ in 0..256 {
            let picks = options
                .iter()
                .map(|(moves, others)| {
                    if !moves.is_empty() && self.below(10) < 9 {
                        moves[self.below(moves.len())].clone()
                    } else {
                        others[self.below(others.len())].clone()
                    }
                })
                .collect();
            let candidate = choice(picks);
            if engine.validate_choice(side, &candidate).is_ok() {
                return candidate;
            }
        }
        // Exhaustively, in order: every slot's moves, then its switches and Pass.
        let all: Vec<Vec<ActionChoice>> = options
            .into_iter()
            .map(|(moves, others)| moves.into_iter().chain(others).collect())
            .collect();
        let mut index = vec![0usize; all.len()];
        loop {
            let candidate = choice(
                index
                    .iter()
                    .zip(&all)
                    .map(|(at, list)| list[*at].clone())
                    .collect(),
            );
            if engine.validate_choice(side, &candidate).is_ok() {
                return candidate;
            }
            let mut slot = 0;
            loop {
                assert!(slot < all.len(), "no valid choice for {side:?}");
                index[slot] += 1;
                if index[slot] < all[slot].len() {
                    break;
                }
                index[slot] = 0;
                slot += 1;
            }
        }
    }
}

/// Steps to the next turn's choices, answering replacements and revivals on the way; `None`
/// once the battle is over (or past `MAX_TURNS`).
pub fn advance(engine: &mut BattleEngine, policy: &mut Policy) -> Option<()> {
    loop {
        match engine.step().expect("the engine steps") {
            EngineStep::Progressed { .. } => continue,
            EngineStep::Finished(_) => return None,
            EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) => {
                return (engine.state().turn <= MAX_TURNS).then_some(());
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
}

/// Plays one turn's choices; `None` once the battle is over.
pub fn answer(engine: &mut BattleEngine, policy: &mut Policy) -> Option<()> {
    advance(engine, policy)?;
    let choices = [
        policy.turn_choice(engine, SideId::P1),
        policy.turn_choice(engine, SideId::P2),
    ];
    engine
        .submit_turn_choices(choices)
        .expect("valid turn choices");
    Some(())
}

/// Plays to the end; the number of turns.
pub fn play_out(engine: &mut BattleEngine, policy: &mut Policy) -> u32 {
    while answer(engine, policy).is_some() {}
    engine.state().turn
}

/// A format's battle between the first two of its teams in `built.json` (teams Showdown
/// admits), started through the pre-battle session (seed 7, the teams' own order): its rules,
/// its dex, and the snapshot of its start.
pub fn start_battle(format_id: &str) -> (Arc<ResolvedRules>, Arc<ResolvedDex>, BattleSnapshot) {
    let format = SessionFormat::new(library(), format_id).expect("a target format");
    let rules = Arc::new(format.rules().expect("the format's rules"));
    let dex = format.dex().clone();
    let teams = built()["formats"][format_id]["teams"]
        .as_array()
        .expect("the format's teams");
    let mut session = BattleSession::new(format, 7);
    for (side, team) in [SideId::P1, SideId::P2].into_iter().zip(teams) {
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
    let start = session.engine().expect("the battle").snapshot();
    (rules, dex, start)
}
