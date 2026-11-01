//! What the examples share: a team for each format they play, the format data, a battle started
//! through the pre-battle session, and a seeded policy that reads nothing but `choice_options`.
#![allow(dead_code)]

use pokemon_battle_engine::{
    ActionChoice, BattleEngine, BattleOutcome, BattleRequest, BattleSession, EngineStep,
    ForcedReplacement, ForcedSwitchChoice, FormatLibrary, ReviveChoice, SessionFormat,
    SessionRequest, SideChoice, SideId, SlotChoice, Stats, TeamChoice, TeamSet,
};

/// The engine's format data (the repository's `data/`).
pub fn library() -> FormatLibrary {
    FormatLibrary::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../data"))
        .unwrap_or_else(|error| panic!("{error}"))
}

/// A set: species, item, ability, moves, nature, EVs (HP, Atk, Def, SpA, SpD, Spe), Tera type.
pub fn set(
    species: &str,
    item: &str,
    ability: &str,
    moves: [&str; 4],
    nature: &str,
    evs: [i32; 6],
    tera_type: &str,
) -> TeamSet {
    let [hp, atk, def, spa, spd, spe] = evs;
    TeamSet {
        species: species.into(),
        item: item.into(),
        ability: ability.into(),
        moves: moves.iter().map(|m| m.to_string()).collect(),
        nature: nature.into(),
        evs: Some(Stats {
            hp,
            atk,
            def,
            spa,
            spd,
            spe,
        }),
        tera_type: tera_type.into(),
        ..TeamSet::default()
    }
}

/// A Gen 9 OU team.
pub fn ou_team() -> Vec<TeamSet> {
    vec![
        set(
            "Great Tusk",
            "Booster Energy",
            "Protosynthesis",
            ["Headlong Rush", "Ice Spinner", "Knock Off", "Rapid Spin"],
            "Jolly",
            [0, 252, 4, 0, 0, 252],
            "Ground",
        ),
        set(
            "Gholdengo",
            "Choice Scarf",
            "Good as Gold",
            ["Make It Rain", "Shadow Ball", "Trick", "Focus Blast"],
            "Timid",
            [0, 0, 0, 252, 4, 252],
            "Steel",
        ),
        set(
            "Dragapult",
            "Choice Specs",
            "Infiltrator",
            ["Draco Meteor", "Shadow Ball", "Flamethrower", "U-turn"],
            "Timid",
            [0, 0, 0, 252, 4, 252],
            "Ghost",
        ),
        set(
            "Garganacl",
            "Leftovers",
            "Purifying Salt",
            ["Salt Cure", "Recover", "Protect", "Stealth Rock"],
            "Careful",
            [252, 0, 4, 0, 252, 0],
            "Water",
        ),
        set(
            "Corviknight",
            "Leftovers",
            "Pressure",
            ["Brave Bird", "Roost", "U-turn", "Defog"],
            "Impish",
            [252, 0, 168, 0, 88, 0],
            "Dragon",
        ),
        set(
            "Clefable",
            "Leftovers",
            "Magic Guard",
            ["Moonblast", "Calm Mind", "Moonlight", "Flamethrower"],
            "Bold",
            [252, 0, 252, 0, 4, 0],
            "Steel",
        ),
    ]
}

/// A Gen 9 VGC (Regulation I) team.
pub fn vgc_team() -> Vec<TeamSet> {
    vec![
        set(
            "Incineroar",
            "Safety Goggles",
            "Intimidate",
            ["Fake Out", "Flare Blitz", "Knock Off", "Parting Shot"],
            "Careful",
            [252, 4, 0, 0, 252, 0],
            "Ghost",
        ),
        set(
            "Rillaboom",
            "Assault Vest",
            "Grassy Surge",
            ["Fake Out", "Grassy Glide", "Wood Hammer", "U-turn"],
            "Adamant",
            [252, 252, 0, 0, 4, 0],
            "Fire",
        ),
        set(
            "Urshifu-Rapid-Strike",
            "Choice Scarf",
            "Unseen Fist",
            ["Surging Strikes", "Close Combat", "Aqua Jet", "U-turn"],
            "Jolly",
            [0, 252, 4, 0, 0, 252],
            "Water",
        ),
        set(
            "Amoonguss",
            "Rocky Helmet",
            "Regenerator",
            ["Spore", "Rage Powder", "Pollen Puff", "Protect"],
            "Relaxed",
            [252, 0, 252, 0, 4, 0],
            "Water",
        ),
        set(
            "Flutter Mane",
            "Booster Energy",
            "Protosynthesis",
            ["Moonblast", "Shadow Ball", "Icy Wind", "Protect"],
            "Timid",
            [4, 0, 0, 252, 0, 252],
            "Fairy",
        ),
        set(
            "Farigiraf",
            "Electric Seed",
            "Armor Tail",
            ["Trick Room", "Psychic", "Hyper Voice", "Protect"],
            "Quiet",
            [252, 0, 4, 252, 0, 0],
            "Water",
        ),
    ]
}

/// A battle of `format_id` between `team` on both sides, started through the pre-battle session:
/// registered, each side bringing its team in its own order.
pub fn started(
    library: &FormatLibrary,
    format_id: &str,
    team: Vec<TeamSet>,
    seed: u64,
) -> BattleEngine {
    let mut session = BattleSession::new(SessionFormat::new(library, format_id).unwrap(), seed);
    for side in [SideId::P1, SideId::P2] {
        session
            .register(side, team.clone())
            .unwrap_or_else(|error| panic!("{format_id}: {error:?}"));
    }
    if let SessionRequest::Choose { sides, .. } = session.request() {
        for side in sides {
            session.choose(side, TeamChoice::default()).unwrap();
        }
    }
    session.engine().expect("the battle").clone()
}

/// A seeded policy (SplitMix64) that reads nothing but `choice_options`: each Pokémon its locked
/// action, else a random move with a target it accepts, else a switch no ally has taken.
pub struct Policy(pub u64);

impl Policy {
    pub fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n.max(1) as u64) as usize
    }

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
}

/// Steps the battle until it asks for something, or ends.
pub fn next_request(engine: &mut BattleEngine) -> EngineStep {
    loop {
        match engine.step().expect("a step") {
            EngineStep::Progressed { .. } => continue,
            other => return other,
        }
    }
}

/// Answers a mid-turn request (a forced switch, a revival) with `policy`'s random pick; false for
/// a request of turn choices.
pub fn answer_mid_turn(
    engine: &mut BattleEngine,
    request: &BattleRequest,
    policy: &mut Policy,
) -> bool {
    match request {
        BattleRequest::ForcedSwitch { replacements } => {
            let choices = replacements.clone().map(|requirement| {
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
            true
        }
        BattleRequest::Revive { side, candidates } => {
            let pokemon = candidates[policy.below(candidates.len())];
            engine
                .resolve_revive(ReviveChoice {
                    side: *side,
                    pokemon,
                })
                .expect("a valid revival");
            true
        }
        _ => false,
    }
}

/// Plays the battle on with `policy` for both sides, to its end or `turns` more turns.
pub fn play_out(
    engine: &mut BattleEngine,
    policy: &mut Policy,
    turns: u32,
) -> Option<BattleOutcome> {
    let mut played = 0;
    loop {
        match next_request(engine) {
            EngineStep::Finished(outcome) => return Some(outcome),
            EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) => {
                if played == turns {
                    return None;
                }
                let choices = [
                    policy.turn(engine, SideId::P1),
                    policy.turn(engine, SideId::P2),
                ];
                engine.submit_turn_choices(choices).expect("valid choices");
                played += 1;
            }
            EngineStep::Awaiting(request) => {
                assert!(answer_mid_turn(engine, &request, policy), "{request:?}");
            }
            EngineStep::Progressed { .. } => unreachable!("next_request steps past these"),
        }
    }
}
