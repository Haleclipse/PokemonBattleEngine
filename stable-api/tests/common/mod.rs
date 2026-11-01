//! What the stable API's tests share: the engine's format data, the teams of its `built.json`,
//! a battle started through the pre-battle session, and a seeded policy that chooses from
//! `choice_options` alone.
#![allow(dead_code)]

use std::sync::OnceLock;

use pokemon_battle_engine::{
    ActionChoice, BattleEngine, BattleOutcome, BattleRequest, BattleSession, EngineStep,
    ForcedReplacement, ForcedSwitchChoice, FormatLibrary, ReviveChoice, SessionFormat,
    SessionRequest, SideChoice, SideId, SlotChoice, Stats, TeamChoice, TeamSet,
};
use serde_json::Value;

pub fn library() -> &'static FormatLibrary {
    static LIBRARY: OnceLock<FormatLibrary> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        FormatLibrary::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../data"))
            .unwrap_or_else(|error| panic!("{error}"))
    })
}

fn built() -> &'static Value {
    static BUILT: OnceLock<Value> = OnceLock::new();
    BUILT.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/fixtures/formats/built.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("built.json"))
            .expect("built.json is JSON")
    })
}

/// The first two teams of `format_id` in the engine's `built.json` (teams Showdown admits).
pub fn teams(format_id: &str) -> Vec<Vec<TeamSet>> {
    built()["formats"][format_id]["teams"]
        .as_array()
        .expect("the format's teams")
        .iter()
        .take(2)
        .map(|team| {
            team["sets"]
                .as_array()
                .expect("a team's sets")
                .iter()
                .map(team_set)
                .collect()
        })
        .collect()
}

/// A set as the export writes it (Showdown's `PokemonSet` JSON).
fn team_set(input: &Value) -> TeamSet {
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

/// The battle a session of `format_id` starts once both teams are registered and chosen.
pub fn started(format_id: &str) -> BattleEngine {
    started_seeded(format_id, 7)
}

/// `started`, the session seeded with `seed`.
pub fn started_seeded(format_id: &str, seed: u64) -> BattleEngine {
    let format = SessionFormat::new(library(), format_id).expect("a target format");
    let mut session = BattleSession::new(format, seed);
    for (side, sets) in [SideId::P1, SideId::P2].into_iter().zip(teams(format_id)) {
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
    assert_eq!(session.request(), SessionRequest::Battle);
    session.engine().expect("the battle").clone()
}

/// Steps until the battle asks for something.
pub fn next_request(engine: &mut BattleEngine) -> EngineStep {
    loop {
        match engine.step().expect("a step") {
            EngineStep::Progressed { .. } => continue,
            other => return other,
        }
    }
}

/// A seeded policy (SplitMix64) that reads nothing but the requests and `choice_options`.
pub struct Policy(pub u64);

impl Policy {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n.max(1) as u64) as usize
    }

    /// A side's turn: each Pokémon its locked action, else a random move with a target it
    /// accepts, else a switch no ally has taken, else a pass.
    pub fn turn(&mut self, engine: &BattleEngine, side: SideId) -> SideChoice {
        let options = engine.choice_options(side).expect("a turn's request");
        let mut taken = Vec::new();
        let actions = options
            .actors
            .iter()
            .map(|actor| {
                let moves: Vec<ActionChoice> = actor
                    .moves
                    .iter()
                    .flat_map(|option| {
                        option.targets.iter().map(|target| ActionChoice::Move {
                            slot: option.slot,
                            target: *target,
                            mechanic: None,
                        })
                    })
                    .collect();
                let switches: Vec<_> = actor
                    .switches
                    .iter()
                    .filter(|member| !taken.contains(*member))
                    .copied()
                    .collect();
                let action = if let Some(locked) = &actor.locked {
                    locked.clone()
                } else if !moves.is_empty() {
                    moves[self.below(moves.len())].clone()
                } else if !switches.is_empty() {
                    let pokemon = switches[self.below(switches.len())];
                    taken.push(pokemon);
                    ActionChoice::Switch { pokemon }
                } else {
                    ActionChoice::Pass
                };
                SlotChoice {
                    actor: actor.actor,
                    action,
                }
            })
            .collect();
        SideChoice { side, actions }
    }

    /// Plays the battle to its end (or `turns` turns): its outcome, and the turns played.
    pub fn play_out(
        &mut self,
        engine: &mut BattleEngine,
        turns: u32,
    ) -> (Option<BattleOutcome>, u32) {
        let mut played = 0;
        while played < turns {
            match next_request(engine) {
                EngineStep::Finished(outcome) => return (Some(outcome), played),
                EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) => {
                    let choices = [self.turn(engine, SideId::P1), self.turn(engine, SideId::P2)];
                    engine.submit_turn_choices(choices).expect("valid choices");
                    played += 1;
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
                                pokemon: available.remove(self.below(available.len())),
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
                    let pokemon = candidates[self.below(candidates.len())];
                    engine
                        .resolve_revive(ReviveChoice { side, pokemon })
                        .expect("a valid revival");
                }
                EngineStep::Awaiting(other) => {
                    panic!("a request this policy does not know: {other:?}")
                }
                EngineStep::Progressed { .. } => unreachable!("next_request steps past these"),
            }
        }
        (None, played)
    }
}
