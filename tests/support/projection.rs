use std::collections::BTreeMap;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::ResolvedDex;
use pokemon_battle_engine::engine::{BattleRequest, EngineStep};
use pokemon_battle_engine::ids::{MoveId, SideId};
use pokemon_battle_engine::rules::ResolvedRules;
use pokemon_battle_engine::spec::Generation;
use pokemon_battle_engine::state::{
    BattleState, FieldCondition, FieldConditionKind, SideCondition, SideConditionKind, StatusState,
    VolatileCondition, WeatherKind,
};
use serde::{Deserialize, Serialize};

/// SemanticFrame v1 (tests/fixtures/SCHEMA.md). Bumping it means regenerating every pinned
/// Showdown output and changing `normalize.mjs` in the same commit.
pub const SEMANTIC_FRAME_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticFrame {
    pub schema: u32,
    pub checkpoint: String,
    pub turn: u32,
    pub request: Option<String>,
    pub outcome: Option<String>,
    pub sides: [SideProjection; 2],
    pub field: FieldProjection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SideProjection {
    pub active: Vec<Option<usize>>,
    pub party: Vec<PokemonProjection>,
    pub conditions: BTreeMap<String, Option<u16>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PokemonProjection {
    pub hp: u16,
    pub max_hp: u16,
    pub status: Option<StatusProjection>,
    pub boosts: BoostProjection,
    pub volatiles: BTreeMap<String, Option<u16>>,
    /// The types as the battle reads them (Showdown `getTypes`, `engine::types_of`), Showdown
    /// ids. Recorded from 2026-10-01 (DESIGN.md §41.8); an older recording has none and the
    /// replay does not compare them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    /// The ability the Pokémon has (Showdown `pokemon.ability`, `engine::raw_ability`), Gen 3+.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability: Option<String>,
    /// The item it holds (Showdown `pokemon.item`; empty: none), Gen 2+.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// The species it battles as (Showdown `pokemon.species.id`, `engine::species_of`): a
    /// forme it changed into, or the Pokémon it transformed into. Recorded from 2026-10-04
    /// (DESIGN.md §43.5); an older recording has none and the replay does not compare it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub species: Option<String>,
    /// Each move slot's PP, in move order (Showdown `moveSlots[].pp`). Recorded from
    /// 2026-10-02 (DESIGN.md R240); an older recording has none and the replay does not
    /// compare them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pp: Option<Vec<u8>>,
    /// Showdown `pokemon.terastallized` (`PokemonState::terastallized`), Gen 9: the Tera type,
    /// written only while the Pokémon has Terastallized (DESIGN.md §44.8, recorded from
    /// 2026-10-04). Both sides follow that rule, so a recording made before the field existed
    /// still compares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terastallized: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusProjection {
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoostProjection {
    pub attack: i8,
    pub defense: i8,
    pub special_attack: i8,
    pub special_defense: i8,
    pub speed: i8,
    pub accuracy: i8,
    pub evasion: i8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FieldProjection {
    pub conditions: BTreeMap<String, Option<u16>>,
    pub weather: Option<TimedProjection>,
    pub terrain: Option<TimedProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimedProjection {
    pub kind: String,
    /// `None`: no duration at all (Showdown's strong weathers have none; a Gen 3-5 weather
    /// ability's is 0).
    pub turns_remaining: Option<u8>,
}

/// Showdown `toID`: lower-case alphanumerics only.
fn showdown_id(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

pub fn capture(
    engine: &BattleEngine,
    checkpoint: impl Into<String>,
    step: Option<&EngineStep>,
) -> SemanticFrame {
    let state = engine.state();
    let request = match step {
        Some(EngineStep::Awaiting(BattleRequest::TurnChoices { .. })) => {
            Some("turn_choices".into())
        }
        // Showdown asks a side that switches and owes a Revival Blessing at once in one forced
        // switch with a `reviving` slot, which the oracle reports as `revive` (R-M8-171).
        Some(EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements })) => {
            let reviving = replacements.iter().any(|requirement| {
                requirement.required_count > 0 && state.side(requirement.side).reviving.is_some()
            });
            Some(if reviving { "revive" } else { "forced_switch" }.into())
        }
        Some(EngineStep::Awaiting(BattleRequest::Revive { .. })) => Some("revive".into()),
        Some(EngineStep::Finished(_)) => Some("finished".into()),
        _ => None,
    };
    capture_state(state, engine.rules(), engine.dex(), checkpoint, request)
}

/// A battle state's frame, with the request it stands at: the engine's (`capture`), or the
/// team preview's, before any engine (`"team_preview"`, M9.4).
pub fn capture_state(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    checkpoint: impl Into<String>,
    request: Option<String>,
) -> SemanticFrame {
    let topology = rules.topology();

    let sides = [SideId::P1, SideId::P2].map(|side| {
        let side_state = state.side(side);
        let active = side_state
            .active
            .iter()
            .map(|active| {
                (*active).and_then(|id| {
                    side_state
                        .team
                        .iter()
                        .position(|candidate| *candidate == id)
                })
            })
            .collect();

        let party = side_state
            .team
            .iter()
            .map(|id| {
                let pokemon = state
                    .pokemon(*id)
                    .expect("team member missing from battle state");
                let mut volatiles = BTreeMap::new();
                for volatile in pokemon.volatiles.entries() {
                    match volatile {
                        // Showdown's single-turn volatiles (`protect`, `followme`,
                        // `ragepowder`, `helpinghand`, `flinch`) have duration 1: a frame sees
                        // them only when the battle ends before the residual expires them.
                        VolatileCondition::Protect { kind, .. } => {
                            use pokemon_battle_engine::state::ProtectKind;
                            let id = match kind {
                                ProtectKind::Protect => "protect",
                                ProtectKind::KingsShield => "kingsshield",
                                ProtectKind::SpikyShield => "spikyshield",
                                ProtectKind::BanefulBunker => "banefulbunker",
                                ProtectKind::Obstruct => "obstruct",
                                ProtectKind::SilkTrap => "silktrap",
                                ProtectKind::BurningBulwark => "burningbulwark",
                                ProtectKind::MaxGuard => "maxguard",
                            };
                            volatiles.insert(id.into(), Some(1));
                        }
                        // Dynamax: Showdown's `effectState` keeps `turns`, which its frame
                        // projection does not report (no duration, no time).
                        VolatileCondition::Dynamax { .. } => {
                            volatiles.insert("dynamax".into(), None);
                        }
                        VolatileCondition::Endure { .. } => {
                            volatiles.insert("endure".into(), Some(1));
                        }
                        VolatileCondition::Substitute { hp, .. } => {
                            volatiles.insert("substitute".into(), Some(*hp));
                        }
                        VolatileCondition::LeechSeed { .. } => {
                            volatiles.insert("leechseed".into(), None);
                        }
                        VolatileCondition::Taunt { turns } => {
                            volatiles.insert("taunt".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::Encore { turns, .. } => {
                            volatiles.insert("encore".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::Disable { turns, .. } => {
                            volatiles.insert("disable".into(), Some(u16::from(*turns)));
                        }
                        // Showdown gives mustrecharge a duration of 2 that its residual has
                        // already ticked to 1 by the time a frame is taken.
                        VolatileCondition::MustRecharge { turns } => {
                            volatiles.insert("mustrecharge".into(), turns.map(u16::from));
                        }
                        VolatileCondition::ChoiceLock { .. } => {
                            volatiles.insert("choicelock".into(), None);
                        }
                        // Showdown's single-turn coat (duration 1): a frame sees it only when
                        // the battle ends before the residual expires it.
                        VolatileCondition::MagicCoat { .. } => {
                            volatiles.insert("magiccoat".into(), Some(1));
                        }
                        VolatileCondition::Snatch { .. } => {
                            volatiles.insert("snatch".into(), Some(1));
                        }
                        VolatileCondition::MeFirst { .. } => {
                            volatiles.insert("mefirst".into(), Some(1));
                        }
                        // Showdown gives `allyswitch` a duration of 2; the frame shows the
                        // counter's turns as it stands.
                        VolatileCondition::AllySwitch { turns, .. } => {
                            volatiles.insert("allyswitch".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::FollowMe => {
                            volatiles.insert("followme".into(), Some(1));
                        }
                        VolatileCondition::RagePowder => {
                            volatiles.insert("ragepowder".into(), Some(1));
                        }
                        VolatileCondition::SubstituteBroken { .. } => {
                            volatiles.insert("substitutebroken".into(), None);
                        }
                        VolatileCondition::ResidualDamage { .. } => {
                            volatiles.insert("residualdmg".into(), None);
                        }
                        // Showdown reports flinch with duration 1 and confusion with its time.
                        VolatileCondition::Flinch { .. } => {
                            volatiles.insert("flinch".into(), Some(1));
                        }
                        VolatileCondition::Confusion { time, .. } => {
                            volatiles.insert("confusion".into(), Some(*time));
                        }
                        // Showdown's `twoturnmove` (duration 2) plus, for the charging turn,
                        // the move itself as a volatile: with the same duration when the move
                        // has a condition (`fly`, `dig`: semi-invulnerable), without one
                        // otherwise (`solarbeam`).
                        VolatileCondition::PartiallyTrapped { turns, .. } => {
                            volatiles.insert("partiallytrapped".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::Ingrain => {
                            volatiles.insert("ingrain".into(), None);
                        }
                        // Zen Mode's has no duration either.
                        VolatileCondition::ZenMode => {
                            volatiles.insert("zenmode".into(), None);
                        }
                        // Commander's two have none either.
                        VolatileCondition::Commanding => {
                            volatiles.insert("commanding".into(), None);
                        }
                        VolatileCondition::Commanded => {
                            volatiles.insert("commanded".into(), None);
                        }
                        // Showdown's `helpinghand` has duration 1 and is gone by any frame.
                        VolatileCondition::HelpingHand { .. } => {
                            volatiles.insert("helpinghand".into(), Some(1));
                        }
                        VolatileCondition::Stall { turns, .. } => {
                            volatiles.insert("stall".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::SureHit { turns } => {
                            volatiles.insert("toxic".into(), Some(u16::from(*turns)));
                        }
                        // Gen 1's screens, Gen 1-2 Mist: volatiles without a duration.
                        VolatileCondition::Reflect => {
                            volatiles.insert("reflect".into(), None);
                        }
                        VolatileCondition::LightScreen => {
                            volatiles.insert("lightscreen".into(), None);
                        }
                        VolatileCondition::Mist => {
                            volatiles.insert("mist".into(), None);
                        }
                        VolatileCondition::MudSport => {
                            volatiles.insert("mudsport".into(), None);
                        }
                        VolatileCondition::PerishSong { turns } => {
                            volatiles.insert("perishsong".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::ChillyReception { turns } => {
                            volatiles.insert("chillyreception".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::Trapped { .. } => {
                            volatiles.insert("trapped".into(), None);
                        }
                        VolatileCondition::Trapper => {
                            volatiles.insert("trapper".into(), None);
                        }
                        VolatileCondition::Octolock { .. } => {
                            volatiles.insert("octolock".into(), None);
                        }
                        VolatileCondition::FocusPunch { turns, .. } => {
                            volatiles.insert("focuspunch".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::BeakBlast { turns } => {
                            volatiles.insert("beakblast".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::ShellTrap { turns, .. } => {
                            volatiles.insert("shelltrap".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::Counter { turns } => {
                            volatiles.insert("counter".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::MirrorCoat { turns } => {
                            volatiles.insert("mirrorcoat".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::DestinyBond => {
                            volatiles.insert("destinybond".into(), None);
                        }
                        VolatileCondition::Grudge => {
                            volatiles.insert("grudge".into(), None);
                        }
                        VolatileCondition::Spotlight => {
                            volatiles.insert("spotlight".into(), Some(1));
                        }
                        VolatileCondition::Rolling { move_id, turns, .. } => {
                            let key = if *move_id == MoveId::ICE_BALL {
                                "iceball"
                            } else {
                                "rollout"
                            };
                            volatiles.insert(key.into(), Some(u16::from(*turns)));
                        }
                        // The oracle's `rolloutstorage` lasts until it is used up (issues.md
                        // I212): no duration.
                        VolatileCondition::RolloutStorage { .. } => {
                            volatiles.insert("rolloutstorage".into(), None);
                        }
                        VolatileCondition::Pursuit { .. } => {
                            volatiles.insert("pursuit".into(), Some(1));
                        }
                        // Showdown `lockedmove`: its `duration` (Gen 3-4 the cartridge's count
                        // under the oracle patch), Gen 1's `time`.
                        VolatileCondition::Rampage(rampage) => {
                            volatiles.insert("lockedmove".into(), Some(u16::from(rampage.turns)));
                        }
                        // Showdown `uproar`: its `duration` (Gen 3-4 the cartridge's count
                        // under the oracle patch, kept as `time`).
                        VolatileCondition::Uproar(uproar) => {
                            volatiles.insert("uproar".into(), Some(u16::from(uproar.turns)));
                        }
                        VolatileCondition::Torment => {
                            volatiles.insert("torment".into(), None);
                        }
                        VolatileCondition::Imprison => {
                            volatiles.insert("imprison".into(), None);
                        }
                        // Showdown `lockon` on its user (Gen 3+); Gen 2's flag on the target is
                        // the oracle patch's `lockedon` (issues.md I69).
                        VolatileCondition::LockOn { turns, .. } => {
                            volatiles.insert("lockon".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::LockedOn => {
                            volatiles.insert("lockedon".into(), None);
                        }
                        VolatileCondition::Curse => {
                            volatiles.insert("curse".into(), None);
                        }
                        // Showdown `furycutter` (Gen 5+); the cartridges' Gen 2-4 count is the
                        // oracle patch's `pokemon.m`, not a volatile (issues.md I75).
                        VolatileCondition::FuryCutter {
                            turns: Some(turns), ..
                        } => {
                            volatiles.insert("furycutter".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::FuryCutter { turns: None, .. } => {}
                        // Showdown `rage` (the oracle patch's flag in Gen 2-4; Gen 2's count is
                        // its `counter`, which no frame shows).
                        VolatileCondition::Rage { .. } => {
                            volatiles.insert("rage".into(), None);
                        }
                        VolatileCondition::GlaiveRush => {
                            volatiles.insert("glaiverush".into(), None);
                        }
                        VolatileCondition::Truant => {
                            volatiles.insert("truant".into(), None);
                        }
                        VolatileCondition::Invulnerability { .. } => {
                            volatiles.insert("invulnerability".into(), None);
                        }
                        VolatileCondition::SkyDrop { turns, .. } => {
                            volatiles.insert("skydrop".into(), Some(u16::from(*turns)));
                        }
                        // No Showdown condition on the carried Pokémon.
                        VolatileCondition::SkyDropped { .. } => {}
                        VolatileCondition::Bide(bide) => {
                            // The oracle shows `duration`, or Gen 1's `time`.
                            volatiles.insert(
                                "bide".into(),
                                Some(u16::from(bide.turns.unwrap_or(bide.time))),
                            );
                        }
                        VolatileCondition::PartialTrappingLock { turns, .. } => {
                            volatiles.insert("partialtrappinglock".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::FakePartiallyTrapped { turns, .. } => {
                            volatiles
                                .insert("fakepartiallytrapped".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::WaterSport => {
                            volatiles.insert("watersport".into(), None);
                        }
                        VolatileCondition::Attract { .. } => {
                            volatiles.insert("attract".into(), None);
                        }
                        VolatileCondition::QuarkDrive { .. } => {
                            volatiles.insert("quarkdrive".into(), None);
                        }
                        VolatileCondition::Protosynthesis { .. } => {
                            volatiles.insert("protosynthesis".into(), None);
                        }
                        VolatileCondition::Yawn { turns, .. } => {
                            volatiles.insert("yawn".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::HealBlock { turns } => {
                            volatiles.insert("healblock".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::Embargo { turns } => {
                            volatiles.insert("embargo".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::Nightmare => {
                            volatiles.insert("nightmare".into(), None);
                        }
                        VolatileCondition::SaltCure => {
                            volatiles.insert("saltcure".into(), None);
                        }
                        VolatileCondition::SyrupBomb { turns, .. } => {
                            volatiles.insert("syrupbomb".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::ThroatChop { turns } => {
                            volatiles.insert("throatchop".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::TarShot => {
                            volatiles.insert("tarshot".into(), None);
                        }
                        VolatileCondition::Unburden => {
                            volatiles.insert("unburden".into(), None);
                        }
                        VolatileCondition::DefenseCurl => {
                            volatiles.insert("defensecurl".into(), None);
                        }
                        VolatileCondition::AquaRing => {
                            volatiles.insert("aquaring".into(), None);
                        }
                        VolatileCondition::PowerTrick => {
                            volatiles.insert("powertrick".into(), None);
                        }
                        VolatileCondition::GastroAcid => {
                            volatiles.insert("gastroacid".into(), None);
                        }
                        VolatileCondition::FocusEnergy => {
                            volatiles.insert("focusenergy".into(), None);
                        }
                        VolatileCondition::Charge => {
                            volatiles.insert("charge".into(), None);
                        }
                        VolatileCondition::NoRetreat => {
                            volatiles.insert("noretreat".into(), None);
                        }
                        VolatileCondition::Minimize => {
                            volatiles.insert("minimize".into(), None);
                        }
                        // `duration: 1` until the end of the turn.
                        VolatileCondition::Roost => {
                            volatiles.insert("roost".into(), Some(1));
                        }
                        VolatileCondition::Electrify => {
                            volatiles.insert("electrify".into(), Some(1));
                        }
                        VolatileCondition::Powder => {
                            volatiles.insert("powder".into(), Some(1));
                        }
                        VolatileCondition::Foresight => {
                            volatiles.insert("foresight".into(), None);
                        }
                        VolatileCondition::MiracleEye => {
                            volatiles.insert("miracleeye".into(), None);
                        }
                        VolatileCondition::SmackDown => {
                            volatiles.insert("smackdown".into(), None);
                        }
                        VolatileCondition::MagnetRise { turns } => {
                            volatiles.insert("magnetrise".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::Telekinesis { turns } => {
                            volatiles.insert("telekinesis".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::FlashFire => {
                            volatiles.insert("flashfire".into(), None);
                        }
                        // Showdown keeps Autotomize as a volatile in Gen 5 (the gen5 mod) and
                        // lowers `weighthg` itself later.
                        VolatileCondition::Autotomize { .. } => {
                            if rules.generation() == Generation::Gen5 {
                                volatiles.insert("autotomize".into(), None);
                            }
                        }
                        VolatileCondition::DragonCheer { .. } => {
                            volatiles.insert("dragoncheer".into(), None);
                        }
                        VolatileCondition::LaserFocus { turns } => {
                            volatiles.insert("laserfocus".into(), Some(u16::from(*turns)));
                        }
                        VolatileCondition::MicleBerry { turns } => {
                            volatiles.insert("micleberry".into(), turns.map(u16::from));
                        }
                        // Gone at the `Update` after the hit, before any frame.
                        VolatileCondition::Fling { .. } => {
                            volatiles.insert("fling".into(), None);
                        }
                        VolatileCondition::Stockpile { layers, .. } => {
                            volatiles.insert("stockpile".into(), Some(u16::from(*layers)));
                        }
                        // Removed as the move ends, before any frame.
                        VolatileCondition::SparklingAria => {
                            volatiles.insert("sparklingaria".into(), None);
                        }
                        VolatileCondition::Metronome { .. } => {
                            volatiles.insert("metronome".into(), None);
                        }
                        VolatileCondition::TwoTurnMove {
                            move_id,
                            turns,
                            charging,
                            hiding,
                            ..
                        } => {
                            // Gen 1's has no duration, and the move has no condition
                            // (`invulnerability` is a volatile of its own).
                            if rules.two_turn().gen1() {
                                volatiles.insert("twoturnmove".into(), None);
                                continue;
                            }
                            volatiles.insert("twoturnmove".into(), Some(u16::from(*turns)));
                            if *charging && let Some(def) = dex.move_by_id(*move_id) {
                                // The move's own condition: a hiding one with its own count.
                                volatiles.insert(
                                    showdown_id(&def.name),
                                    hiding.map(|(_, left)| u16::from(left)),
                                );
                            }
                        }
                    }
                }
                PokemonProjection {
                    hp: pokemon.hp,
                    max_hp: pokemon.max_hp,
                    status: pokemon.status.map(project_status),
                    boosts: BoostProjection {
                        attack: pokemon.stages.attack,
                        defense: pokemon.stages.defense,
                        special_attack: pokemon.stages.special_attack,
                        special_defense: pokemon.stages.special_defense,
                        speed: pokemon.stages.speed,
                        accuracy: pokemon.stages.accuracy,
                        evasion: pokemon.stages.evasion,
                    },
                    volatiles,
                    // Showdown's ids: `???` keeps no letter.
                    types: Some(
                        pokemon_battle_engine::engine::types_of(state, rules, pokemon)
                            .iter()
                            .map(|t| match t {
                                pokemon_battle_engine::dex::PokemonType::Typeless => String::new(),
                                t => format!("{t:?}").to_ascii_lowercase(),
                            })
                            .collect(),
                    ),
                    ability: (rules.generation() >= Generation::Gen3).then(|| {
                        dex.ability_by_id(pokemon_battle_engine::engine::raw_ability(pokemon))
                            .map_or_else(String::new, |a| showdown_id(&a.name))
                    }),
                    item: (rules.generation() >= Generation::Gen2).then(|| {
                        pokemon
                            .held_item
                            .and_then(|item| dex.item_by_id(item))
                            .map_or_else(String::new, |i| showdown_id(&i.name))
                    }),
                    species: Some(
                        pokemon_battle_engine::engine::species_of(pokemon)
                            .and_then(|species| dex.species_by_id(species))
                            .map_or_else(String::new, |species| species.key.clone()),
                    ),
                    pp: Some(pokemon.move_slots().iter().map(|slot| slot.pp).collect()),
                    // Only while it has Terastallized (DESIGN.md §44.8), so a recording made
                    // before the field existed still compares.
                    terastallized: (rules.generation() == Generation::Gen9)
                        .then_some(pokemon.terastallized)
                        .flatten()
                        .map(|t| format!("{t:?}").to_ascii_lowercase()),
                }
            })
            .collect();

        let mut conditions = BTreeMap::new();
        for condition in side_state.conditions.entries() {
            match condition {
                SideCondition::StealthRock { .. } => {
                    conditions.insert("stealthrock".into(), None);
                }
                SideCondition::Spikes { layers, .. } => {
                    conditions.insert("spikes".into(), Some(u16::from(*layers)));
                }
                SideCondition::ToxicSpikes { layers, .. } => {
                    conditions.insert("toxicspikes".into(), Some(u16::from(*layers)));
                }
                SideCondition::StickyWeb { .. } => {
                    conditions.insert("stickyweb".into(), None);
                }
                SideCondition::Tailwind {
                    turns_remaining, ..
                } => {
                    conditions.insert("tailwind".into(), Some(u16::from(*turns_remaining)));
                }
                // Showdown duration 1; gone by any frame.
                SideCondition::WideGuard { .. } => {
                    conditions.insert("wideguard".into(), Some(1));
                }
                SideCondition::CraftyShield { .. } => {
                    conditions.insert("craftyshield".into(), Some(1));
                }
                SideCondition::MatBlock { .. } => {
                    conditions.insert("matblock".into(), Some(1));
                }
                SideCondition::QuickGuard { .. } => {
                    conditions.insert("quickguard".into(), Some(1));
                }
                SideCondition::Timed {
                    kind,
                    turns_remaining,
                } => {
                    let id = match kind {
                        SideConditionKind::Reflect => "reflect",
                        SideConditionKind::LightScreen => "lightscreen",
                        SideConditionKind::AuroraVeil => "auroraveil",
                        SideConditionKind::Mist => "mist",
                        SideConditionKind::Safeguard => "safeguard",
                        SideConditionKind::LuckyChant => "luckychant",
                        SideConditionKind::FirePledge => "firepledge",
                        SideConditionKind::GrassPledge => "grasspledge",
                        SideConditionKind::WaterPledge => "waterpledge",
                        other => panic!("{other:?} is not a timed side condition"),
                    };
                    conditions.insert(id.into(), Some(u16::from(*turns_remaining)));
                }
            }
        }
        SideProjection {
            active,
            party,
            conditions,
        }
    });

    let mut field_conditions = BTreeMap::new();
    for condition in state.field.conditions() {
        match condition {
            FieldCondition::TrickRoom {
                turns_remaining, ..
            } => {
                field_conditions.insert("trickroom".into(), Some(u16::from(*turns_remaining)));
            }
            FieldCondition::EchoedVoice {
                turns_remaining, ..
            } => {
                field_conditions.insert("echoedvoice".into(), Some(u16::from(*turns_remaining)));
            }
            FieldCondition::Timed {
                kind,
                turns_remaining,
            } => {
                let id = match kind {
                    FieldConditionKind::Gravity => "gravity",
                    FieldConditionKind::WonderRoom => "wonderroom",
                    FieldConditionKind::IonDeluge => "iondeluge",
                    FieldConditionKind::MudSport => "mudsport",
                    FieldConditionKind::WaterSport => "watersport",
                    FieldConditionKind::FairyLock => "fairylock",
                    FieldConditionKind::MagicRoom => "magicroom",
                    FieldConditionKind::TrickRoom | FieldConditionKind::EchoedVoice => continue,
                };
                field_conditions.insert(id.into(), Some(u16::from(*turns_remaining)));
            }
        }
    }

    let outcome = state
        .outcome
        .map(|outcome| format!("{outcome:?}").to_lowercase());

    // topology is intentionally touched here so a projection test also validates that the active
    // layout is resolvable by the configured mode, without serializing engine-specific PositionId.
    let _ = topology.positions();

    SemanticFrame {
        schema: SEMANTIC_FRAME_SCHEMA,
        checkpoint: checkpoint.into(),
        turn: state.turn,
        request,
        outcome,
        sides,
        field: FieldProjection {
            conditions: field_conditions,
            weather: state.field.weather.map(|weather| TimedProjection {
                // Showdown's condition ids; the Gen 9 snow is `snowscape`, earlier hail.
                kind: match weather.kind {
                    WeatherKind::Sun if weather.strong => "desolateland",
                    WeatherKind::Rain if weather.strong => "primordialsea",
                    WeatherKind::StrongWinds => "deltastream",
                    WeatherKind::Sun => "sunnyday",
                    WeatherKind::Rain => "raindance",
                    WeatherKind::Sandstorm => "sandstorm",
                    WeatherKind::Snow if rules.generation() >= Generation::Gen9 => "snowscape",
                    WeatherKind::Snow => "hail",
                }
                .into(),
                turns_remaining: (!weather.strong).then_some(weather.turns_remaining),
            }),
            terrain: state.field.terrain.map(|terrain| TimedProjection {
                // Showdown's condition ids.
                kind: format!("{}terrain", format!("{:?}", terrain.kind).to_lowercase()),
                turns_remaining: Some(terrain.turns_remaining),
            }),
        },
    }
}

/// Showdown's status abbreviations, so frames from both engines compare directly.
fn project_status(status: StatusState) -> StatusProjection {
    use pokemon_battle_engine::state::NonVolatileStatus::*;
    StatusProjection {
        kind: match status.kind {
            Burn => "brn",
            Paralysis => "par",
            Poison => "psn",
            Toxic => "tox",
            Sleep => "slp",
            Freeze => "frz",
        }
        .into(),
    }
}
