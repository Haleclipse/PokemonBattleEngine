//! The scenario and its compiling into a battle; what the oracle's log tells of each action
//! (`ActionOracle`, `CalledMove` …).

use std::collections::BTreeMap;
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, FollowUp, MoveCategory, MultiHit, Nature, PokemonSet, PrimaryEffect, ResolvedDex,
    ResolvedMove, SecondaryEffect, SecondaryTarget, SecondaryTrigger, StatSpread, StatusMoveEffect,
    build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, BattleSnapshot, EngineStep, ForcedReplacement, ForcedSwitchChoice,
    MechanicChoice, PriorityChance, ReplacementRequirement, ReviveChoice, SideChoice, SlotChoice,
    TraceEvent, two_turn,
};
use pokemon_battle_engine::format::{FormatLibrary, Stats, TeamSet};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, HitStepOrder, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::session::{OpenTeamSheets, SessionFormat, TeamChoice, TeamSlot};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    ActiveMove, BattleRng, BattleState, Gender, NonVolatileStatus, RandomOverride,
    RandomRequestKind, SideState, VolatileCondition, VolatileConditionKind,
};
use serde::Deserialize;
use serde_json::Value;

use crate::support::{capture, compare_json};

use super::*;

// ---------------------------------------------------------------------------------------------
// Scenario format
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Scenario {
    pub schema: u32,
    pub id: String,
    pub battle: ScenarioBattle,
    pub teams: ScenarioTeams,
    /// A real format's Team Preview answers (`team 3142`; none: the declared order), M9.4.
    #[serde(default)]
    pub preview: Option<PreviewAnswers>,
    pub script: Vec<ScriptTurn>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioBattle {
    pub generation: u8,
    pub mode: String,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioTeams {
    pub p1: Vec<ScenarioSet>,
    pub p2: Vec<ScenarioSet>,
}

pub(super) fn default_level() -> u8 {
    100
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioSet {
    #[serde(default)]
    pub label: Option<String>,
    pub species: String,
    #[serde(default)]
    pub ability: Option<String>,
    #[serde(default)]
    pub item: Option<String>,
    #[serde(default = "default_level")]
    pub level: u8,
    #[serde(default)]
    pub nature: Option<String>,
    #[serde(default)]
    pub evs: Option<BTreeMap<String, u8>>,
    #[serde(default)]
    pub ivs: Option<BTreeMap<String, u8>>,
    pub moves: Vec<String>,
    #[serde(default)]
    pub happiness: Option<u8>,
    /// Showdown `M` / `F` / `N`; when the set names none, the replay takes the one Showdown drew
    /// from its log (`with_oracle_genders`).
    #[serde(default)]
    pub gender: Option<String>,
    /// Showdown `teraType`: the type Terastallization gives (Gen 9); `None` = the species' first
    /// type.
    #[serde(default, rename = "teraType")]
    pub tera_type: Option<String>,
    /// Showdown `dynamaxLevel` (Gen 8, DESIGN.md §44.5): 0-10; `None` = 10.
    #[serde(default, rename = "dynamaxLevel")]
    pub dynamax_level: Option<u8>,
    /// Showdown `hpType` (a real format's set, M9.4).
    #[serde(default, rename = "hpType")]
    pub hp_type: Option<String>,
    #[serde(default)]
    pub shiny: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptTurn {
    #[serde(default)]
    pub p1: Option<String>,
    #[serde(default)]
    pub p2: Option<String>,
}

/// Team Preview's answers: each side's choice, and whether both accepted the open team sheets
/// (`Open Team Sheets`; DESIGN.md §47.0 item 14).
#[derive(Debug, Clone, Deserialize)]
pub struct PreviewAnswers {
    #[serde(default)]
    pub p1: Option<String>,
    #[serde(default)]
    pub p2: Option<String>,
    #[serde(default)]
    pub open_team_sheets: bool,
}

pub(super) fn spread(map: &Option<BTreeMap<String, u8>>, default: u8) -> StatSpread {
    let get = |key: &str| {
        map.as_ref()
            .and_then(|m| m.get(key).copied())
            .unwrap_or(default)
    };
    StatSpread {
        hp: get("hp"),
        atk: get("atk"),
        def: get("def"),
        spa: get("spa"),
        spd: get("spd"),
        spe: get("spe"),
    }
}

impl ScenarioSet {
    pub fn to_set(&self) -> PokemonSet {
        PokemonSet {
            species: self.species.clone(),
            level: self.level,
            nature: self
                .nature
                .as_deref()
                .and_then(Nature::from_name)
                .unwrap_or(Nature::Hardy),
            evs: spread(&self.evs, 0),
            ivs: spread(&self.ivs, 31),
            ability: self.ability.clone(),
            item: self.item.clone(),
            moves: self.moves.clone(),
            happiness: self.happiness.unwrap_or(255),
            gender: self.gender.as_deref().and_then(|gender| match gender {
                "M" => Some(Gender::Male),
                "F" => Some(Gender::Female),
                "N" => Some(Gender::Genderless),
                _ => None,
            }),
            tera_type: self
                .tera_type
                .as_deref()
                .and_then(pokemon_battle_engine::dex::PokemonType::from_name),
            dynamax_level: self.dynamax_level.unwrap_or(10),
            hp_type: None,
        }
    }
}

pub fn generation(number: u8) -> Generation {
    match number {
        1 => Generation::Gen1,
        2 => Generation::Gen2,
        3 => Generation::Gen3,
        4 => Generation::Gen4,
        5 => Generation::Gen5,
        6 => Generation::Gen6,
        7 => Generation::Gen7,
        8 => Generation::Gen8,
        9 => Generation::Gen9,
        other => panic!("unsupported generation {other}"),
    }
}

/// Compiled battle: rules, dex, engine at the turn-1 TurnChoices boundary, and the team ids.
pub struct Compiled {
    pub rules: Arc<ResolvedRules>,
    pub dex: Arc<ResolvedDex>,
    pub engine: BattleEngine,
    pub teams: [Vec<PokemonId>; 2],
    /// Showdown's current party order per side: `switch N` counts in this order, and every
    /// switch swaps the newcomer with the Pokémon it replaces (the active one stays at index 0
    /// in singles). Kept in step with the engine as the script is replayed.
    pub party: [Vec<PokemonId>; 2],
    /// Scenario labels per side, parallel to `teams`: what a script's `switch <label>` names.
    pub labels: [Vec<String>; 2],
    /// The names Showdown gives them, parallel to `teams` (`shown_name`): what its protocol and
    /// its client call them.
    pub names: [Vec<String>; 2],
    /// Party swaps made for this boundary's declared switches, kept only if the switch really
    /// happens (Gen 1: a faint cancels the rest of the turn, the other side's switch included).
    pub pending_swaps: Vec<(usize, usize, usize, PokemonId)>,
    /// A real format's Team Preview (M9.4): every registered Pokémon built, none on the field,
    /// turn 0 — the `team_preview` frame's state.
    pub preview: Option<BattleState>,
    /// The replayed battle's events before the current engine's own trace (`flush_trace`): a
    /// snapshot holds no trace (DESIGN.md §47.0 item 6), so an engine restored from one starts
    /// an empty trace, and the log keeps what the engines before it traced. A rollback to an
    /// earlier snapshot cuts the log back to its length then (R-M10-6).
    pub log: Vec<TraceEvent>,
}

/// Moves the current engine's trace into the battle's log (`Compiled::log`). Done as each step
/// starts: the current engine's trace, and that of every probe restored from the step's
/// snapshot, then hold that step's events from index 0 (R-M10-6).
pub(super) fn flush_trace(compiled: &mut Compiled) {
    let events = compiled.engine.take_trace_events();
    compiled.log.extend(events);
}

/// The engine id of the Pokémon Showdown calls `who` (by side and the name it gives it).
pub(super) fn label_to_id(compiled: &Compiled, who: &Who) -> Option<PokemonId> {
    let side = who.side.min(1);
    compiled.names[side]
        .iter()
        .position(|label| *label == who.name)
        .map(|index| compiled.teams[side][index])
}

/// A `move` choice's move (an index or a name) and its target: Showdown reads a trailing
/// `[-+]1..3` as the target location (except in Conversion 2's name).
pub(super) fn split_move_choice(text: &str) -> (String, Option<i32>) {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if pokemon_battle_engine::data::to_id(text) == "conversion2" {
        return (text.to_owned(), None);
    }
    let target = match words.as_slice() {
        [_, .., last] => last.parse::<i32>().ok().filter(|t| {
            (1..=3).contains(&t.abs()) && last.trim_start_matches(['-', '+']).len() == 1
        }),
        _ => None,
    };
    if target.is_some() {
        words.pop();
    }
    (words.join(" "), target)
}

/// The 1-based party index a `switch` names: a number in Showdown's current party order, or a
/// Pokémon's label (Showdown `chooseSwitch` matches the name, case-insensitive), which stays
/// valid whatever the party order — the form `tools/minimize.py` rewrites scripts into.
pub(super) fn switch_index(compiled: &Compiled, side: SideId, text: &str) -> usize {
    if let Ok(n) = text.parse::<usize>() {
        return n;
    }
    let index = side.index();
    let id = compiled.labels[index]
        .iter()
        .position(|label| label.eq_ignore_ascii_case(text))
        .map(|at| compiled.teams[index][at])
        .unwrap_or_else(|| panic!("switch to an unknown Pokémon: {text}"));
    compiled.party[index]
        .iter()
        .position(|p| *p == id)
        .expect("labelled Pokémon in the party")
        + 1
}

/// Resolves `switch N` the way Showdown does and records the swap it will cause: the newcomer
/// takes the party index of the Pokémon in `slot`. Showdown's party starts with the Pokémon of
/// each slot, a fainted one too until it is replaced (`switchIn` swaps them), so index `slot`
/// is the one it leaves — the engine's slot is already empty after a faint.
pub(super) fn take_switch_target(
    compiled: &mut Compiled,
    side: SideId,
    slot: usize,
    n: usize,
) -> PokemonId {
    let party = &mut compiled.party[side.index()];
    let target = party[n - 1];
    party.swap(slot, n - 1);
    compiled
        .pending_swaps
        .push((side.index(), slot, n - 1, target));
    target
}

/// Ally Switch traded the two active Pokémon's places (`TraceEvent::PositionsSwapped`): Showdown's
/// `swapPosition` swaps them in `side.pokemon` too, whose order `switch N` counts in (DESIGN.md
/// R-M8-56).
pub(super) fn follow_position_swaps(compiled: &mut Compiled, from: usize) {
    let sides: Vec<SideId> = compiled.engine.trace().events()[from..]
        .iter()
        .filter_map(|event| match event {
            TraceEvent::PositionsSwapped { side } => Some(*side),
            _ => None,
        })
        .collect();
    for side in sides {
        compiled.party[side.index()].swap(0, 1);
    }
}

/// A drag outside an action (the residual's, R-M8-141) swaps the Showdown party order like any
/// switch — an action's drag swaps as its decision is planned.
pub(super) fn follow_drags(compiled: &mut Compiled, from: usize) {
    let drags: Vec<(PokemonId, PokemonId)> = compiled.engine.trace().events()[from..]
        .iter()
        .filter_map(|event| match event {
            TraceEvent::DraggedOut { target, incoming } => Some((*target, *incoming)),
            _ => None,
        })
        .collect();
    for (outgoing, incoming) in drags {
        let Some(side) = compiled.engine.state().pokemon(incoming).map(|p| p.side) else {
            continue;
        };
        let party = &mut compiled.party[side.index()];
        if let (Some(from), Some(to)) = (
            party.iter().position(|p| *p == outgoing),
            party.iter().position(|p| *p == incoming),
        ) {
            party.swap(from, to);
        }
    }
}

/// The residual's drags (a Red Card a foreseen attack's hit set off, R-M8-141): after the
/// residual the engine drags in with a RandomTarget decision among the healthy bench in team
/// order (none when the bench has one Pokémon); the frame's `drag` after the residual names who
/// came in.
pub(super) fn residual_drag_tape(
    compiled: &Compiled,
    state: &BattleState,
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    oracle_frame["events"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .skip_while(|event| event["kind"] != "residual")
        .filter(|event| event["kind"] == "drag")
        .filter_map(|event| {
            let who = serde_json::from_value::<Who>(event["pokemon"].clone()).ok()?;
            let entrant = label_to_id(compiled, &who)?;
            let side = if who.side == 0 {
                SideId::P1
            } else {
                SideId::P2
            };
            let bench = state.healthy_bench(side);
            let index = bench.iter().position(|id| *id == entrant)?;
            (bench.len() > 1)
                .then(|| RandomOverride::range(RandomRequestKind::RandomTarget, index as u16))
        })
        .collect()
}

/// Undoes the party swaps of switches that did not happen by the boundary (the engine's trace
/// since `from` has no `Switched` bringing that Pokémon in) — but for one still queued, the turn
/// paused for a mid-turn request (an Eject Pack's replacement): it stays pending (R-M8-124).
pub(super) fn settle_party_swaps(compiled: &mut Compiled, from: usize) {
    // `from` counts in the battle's log (`Compiled::log`): the boundary's events, every engine's.
    flush_trace(compiled);
    let swaps = std::mem::take(&mut compiled.pending_swaps);
    let queued = compiled.engine.pending_switch_ins();
    for (side, a, b, incoming) in swaps.into_iter().rev() {
        if queued.contains(&incoming) {
            compiled.pending_swaps.insert(0, (side, a, b, incoming));
            continue;
        }
        let switched_in = compiled.log[from.min(compiled.log.len())..]
            .iter()
            .any(|event| {
                matches!(event, TraceEvent::Switched { incoming: pokemon, .. } if *pokemon == incoming)
            });
        if !switched_in {
            compiled.party[side].swap(a, b);
        }
    }
}

/// Showdown's target number for `move N T`: positive counts the foe's positions from the
/// left, negative the user's own side; absent means the move needs none (or picks by default).
pub(super) fn declared_target(
    compiled: &Compiled,
    side: SideId,
    target: Option<i32>,
    pattern: TargetPattern,
) -> Option<DeclaredTarget> {
    let topology = compiled.rules.topology();
    match target {
        Some(t) if t > 0 => topology
            .position(side.other(), (t - 1) as u8)
            .map(DeclaredTarget),
        Some(t) if t < 0 => topology.position(side, (-t - 1) as u8).map(DeclaredTarget),
        // No target in the script: a foe-directed move aims at the first foe; a move that needs
        // (or may take) an ally is left without one, as Showdown's singles choice leaves it,
        // and the engine resolves it (`TargetingRules::resolve`: the user, or no target).
        _ => match pattern {
            TargetPattern::SelectedAny
            | TargetPattern::SelectedAdjacent
            | TargetPattern::SelectedFoe
            | TargetPattern::SelectedAdjacentFoe => {
                topology.position(side.other(), 0).map(DeclaredTarget)
            }
            _ => None,
        },
    }
}

/// The repository's data directory (DESIGN.md §46).
pub fn format_library() -> &'static FormatLibrary {
    static LIBRARY: std::sync::OnceLock<FormatLibrary> = std::sync::OnceLock::new();
    LIBRARY.get_or_init(|| {
        FormatLibrary::open(concat!(env!("CARGO_MANIFEST_DIR"), "/data"))
            .unwrap_or_else(|error| panic!("{error}"))
    })
}

impl ScenarioSet {
    /// The set as a player registers it under a real format (Showdown's `name` is the label).
    pub fn to_team_set(&self) -> TeamSet {
        let stats = |map: &Option<BTreeMap<String, u8>>, default: i32| {
            map.as_ref().map(|map| {
                let get = |key: &str| map.get(key).map_or(default, |v| i32::from(*v));
                Stats {
                    hp: get("hp"),
                    atk: get("atk"),
                    def: get("def"),
                    spa: get("spa"),
                    spd: get("spd"),
                    spe: get("spe"),
                }
            })
        };
        TeamSet {
            name: self.label.clone().unwrap_or_else(|| self.species.clone()),
            species: self.species.clone(),
            item: self.item.clone().unwrap_or_default(),
            ability: self.ability.clone().unwrap_or_default(),
            moves: self.moves.clone(),
            nature: self.nature.clone().unwrap_or_default(),
            gender: self.gender.clone().unwrap_or_default(),
            evs: stats(&self.evs, 0),
            ivs: stats(&self.ivs, 31),
            level: u32::from(self.level),
            shiny: self.shiny,
            happiness: self.happiness.map(f64::from),
            hp_type: self.hp_type.clone().unwrap_or_default(),
            gigantamax: false,
            tera_type: self.tera_type.clone().unwrap_or_default(),
        }
    }
}

/// A Team Preview answer (`team 3142`, 1-based; `default` or none: the declared order).
pub(super) fn team_choice(text: Option<&str>) -> TeamChoice {
    let slots = text
        .and_then(|text| text.strip_prefix("team "))
        .unwrap_or_default();
    TeamChoice {
        order: slots
            .chars()
            .filter_map(|c| c.to_digit(10))
            .map(|d| TeamSlot(d as u8 - 1))
            .collect(),
    }
}

/// A real format's battle (M9.4): both teams registered under it (validated, normalised), Team
/// Preview's state, the scenario's choices taken as Showdown takes them, and the battle started
/// from the Pokémon brought — `session::SessionFormat`'s path, the lead switch-in left to the
/// replay (its tape first).
/// The name Showdown gives a set's Pokémon (its `Pokemon` constructor: `if (set.name ===
/// set.species || !set.name) set.name = this.baseSpecies.baseSpecies`): the label, unless it is
/// the species as written — or there is none — when it is the species' base species as the dex
/// names it: `Slowbro` for a `Slowbro-Galar`, `Farfetch’d` for a `Farfetch'd` (R-M11-36).
pub(super) fn shown_name(dex: &ResolvedDex, set: &ScenarioSet) -> String {
    match &set.label {
        Some(label) if *label != set.species => label.clone(),
        _ => dex
            .species_by_name(&set.species)
            .map(|species| {
                species
                    .base_species
                    .clone()
                    .unwrap_or_else(|| species.name.clone())
            })
            .unwrap_or_else(|| set.species.clone()),
    }
}

pub(super) fn compile_format(scenario: &Scenario, format_id: &str) -> Compiled {
    let fail = |error: &dyn std::fmt::Debug| -> ! { panic!("{}: {error:?}", scenario.id) };
    let format = SessionFormat::new(format_library(), format_id).unwrap_or_else(|e| fail(&e));
    let mut rng = BattleRng::new(0);
    let sides = [&scenario.teams.p1, &scenario.teams.p2];
    let registered: Vec<_> = [SideId::P1, SideId::P2]
        .into_iter()
        .zip(sides)
        .map(|(side, sets)| {
            format
                .register(
                    side,
                    sets.iter().map(ScenarioSet::to_team_set).collect(),
                    &mut rng,
                )
                .unwrap_or_else(|e| fail(&e))
        })
        .collect();
    let policy = format.policy();
    let previewed = policy.preview || policy.picked_team_size.is_some();
    let preview = previewed.then(|| {
        let mut state = format
            .state([&registered[0].sets, &registered[1].sets], rng.clone())
            .unwrap_or_else(|e| fail(&e));
        for side in [SideId::P1, SideId::P2] {
            let slots = state.side(side).active.len();
            state.side_mut(side).active = vec![None; slots];
        }
        state.turn = 0;
        state
    });
    let answers = [
        scenario.preview.as_ref().and_then(|p| p.p1.as_deref()),
        scenario.preview.as_ref().and_then(|p| p.p2.as_deref()),
    ];
    let orders: Vec<Vec<usize>> = (0..2)
        .map(|i| {
            let size = registered[i].sets.len();
            if !previewed {
                return (0..size).collect();
            }
            let picked = policy
                .picked_team_size
                .map_or(size, |picked| size.min(usize::from(picked)));
            team_choice(answers[i])
                .resolve(size, picked)
                .unwrap_or_else(|e| fail(&e))
                .iter()
                .map(|slot| usize::from(slot.0))
                .collect()
        })
        .collect();
    let chosen: Vec<Vec<TeamSet>> = (0..2)
        .map(|i| {
            orders[i]
                .iter()
                .map(|&j| registered[i].sets[j].clone())
                .collect()
        })
        .collect();
    let mut engine = format
        .start([&chosen[0], &chosen[1]], rng)
        .unwrap_or_else(|e| fail(&e));
    // What the preview showed, the open team sheets with it when both sides accepted them or the
    // format shows them anyway (DESIGN.md §47.0 item 14).
    let accepted = scenario
        .preview
        .as_ref()
        .is_some_and(|preview| preview.open_team_sheets);
    let sheets = match format.visibility().open_team_sheets {
        OpenTeamSheets::Forced => true,
        OpenTeamSheets::Offered => accepted,
        _ => {
            assert!(
                !accepted,
                "{}: the format shows no team sheets",
                scenario.id
            );
            false
        }
    };
    if let Some(preview) = format
        .team_preview(
            [&registered[0].sets, &registered[1].sets],
            [&orders[0], &orders[1]],
            sheets,
        )
        .unwrap_or_else(|e| fail(&e))
    {
        engine.show_team_preview(preview);
    }
    let mut teams: [Vec<PokemonId>; 2] = [Vec::new(), Vec::new()];
    let mut next = 1u8;
    for (i, team) in teams.iter_mut().enumerate() {
        for _ in &orders[i] {
            team.push(PokemonId(next));
            next += 1;
        }
    }
    let labels = [0, 1].map(|i| {
        orders[i]
            .iter()
            .map(|&j| {
                let set = &sides[i][j];
                set.label.clone().unwrap_or_else(|| set.species.clone())
            })
            .collect()
    });
    let names = [0, 1].map(|i| {
        orders[i]
            .iter()
            .map(|&j| shown_name(format.dex(), &sides[i][j]))
            .collect()
    });
    Compiled {
        rules: Arc::new(format.rules().unwrap_or_else(|e| fail(&e))),
        dex: format.dex().clone(),
        engine,
        pending_swaps: Vec::new(),
        party: teams.clone(),
        labels,
        names,
        teams,
        preview,
        log: Vec::new(),
    }
}

pub fn compile(scenario: &Scenario) -> Compiled {
    // The replay names a Pokémon by its label, as Showdown's protocol names it by its nickname:
    // two of a side's Pokémon sharing one cannot be told apart (DESIGN.md R-M9-22).
    for (side, sets) in [("p1", &scenario.teams.p1), ("p2", &scenario.teams.p2)] {
        let labels: Vec<&str> = sets
            .iter()
            .map(|set| set.label.as_deref().unwrap_or(&set.species))
            .collect();
        if let Some(shared) = labels
            .iter()
            .enumerate()
            .find_map(|(i, label)| labels[..i].contains(label).then_some(label))
        {
            panic!(
                "{}: two of {side}'s Pokémon share the label {shared:?}; labels must be unique on a side",
                scenario.id
            );
        }
    }
    if let Some(format) = scenario.battle.format.as_deref()
        && !format.ends_with("customgame")
    {
        return compile_format(scenario, format);
    }
    let generation = generation(scenario.battle.generation);
    let mode = match scenario.battle.mode.as_str() {
        "doubles" => BattleMode::Doubles,
        _ => BattleMode::Singles,
    };
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, mode, "custom"))
            .unwrap_or_else(|error| panic!("{}: {error:?}", scenario.id)),
    );
    let dex = Arc::new(DexResolver::resolve(generation));

    let mut pokemon = Vec::new();
    let mut teams: [Vec<PokemonId>; 2] = [Vec::new(), Vec::new()];
    let mut next = 1u8;
    for (index, sets) in [&scenario.teams.p1, &scenario.teams.p2]
        .into_iter()
        .enumerate()
    {
        let side = if index == 0 { SideId::P1 } else { SideId::P2 };
        for set in sets {
            let id = PokemonId(next);
            next += 1;
            pokemon.push(
                build_pokemon(&dex, id, side, &set.to_set())
                    .unwrap_or_else(|error| panic!("{}: {}: {error:?}", scenario.id, set.species)),
            );
            teams[index].push(id);
        }
    }
    let slots = usize::from(rules.topology().active_slots_per_side());
    let side_state = |team: &Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), slots);
        for (slot, id) in team.iter().take(slots).enumerate() {
            side.active[slot] = Some(*id);
        }
        side
    };
    let state = BattleState::new(pokemon, [side_state(&teams[0]), side_state(&teams[1])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state)
        .unwrap_or_else(|error| panic!("{}: {error:?}", scenario.id));
    let names = [&scenario.teams.p1, &scenario.teams.p2]
        .map(|sets| sets.iter().map(|set| shown_name(&dex, set)).collect());
    Compiled {
        rules,
        dex,
        engine,
        pending_swaps: Vec::new(),
        preview: None,
        log: Vec::new(),
        party: teams.clone(),
        labels: [
            scenario
                .teams
                .p1
                .iter()
                .map(|set| set.label.clone().unwrap_or_else(|| set.species.clone()))
                .collect(),
            scenario
                .teams
                .p2
                .iter()
                .map(|set| set.label.clone().unwrap_or_else(|| set.species.clone()))
                .collect(),
        ],
        names,
        teams,
    }
}

/// The gimmick suffix of a move choice (`move 1 terastallize`), as `MechanicChoice`
/// (DESIGN.md §44.8): Showdown's `chooseMove` event suffix, stripped off the slot text.
pub(super) fn split_mechanic(choice: &str) -> (String, Option<MechanicChoice>) {
    let mut words: Vec<&str> = choice.split_whitespace().collect();
    let mechanic = match words.last().copied() {
        Some("mega" | "megax" | "megay") => Some(MechanicChoice::MegaEvolve),
        // Ultra Burst is a mechanic of its own (Q1, DESIGN.md §44.3).
        Some("ultra") => Some(MechanicChoice::UltraBurst),
        Some("zmove") => Some(MechanicChoice::ZMove),
        Some("dynamax" | "max" | "gigantamax") => Some(MechanicChoice::Dynamax),
        Some("terastallize" | "terastal") => Some(MechanicChoice::Terastallize),
        _ => None,
    };
    if mechanic.is_some() {
        words.pop();
    }
    (words.join(" "), mechanic)
}

/// "move N" / "switch N" / "pass" in Showdown's choice syntax, for one active slot.
/// One side's turn choice in Showdown syntax: one comma-separated action per active slot
/// (`move N [T]`, `switch N`, `pass`); a missing entry is a pass.
pub(super) fn parse_choice(
    compiled: &mut Compiled,
    side: SideId,
    text: Option<&str>,
) -> SideChoice {
    let actives: Vec<Option<PokemonId>> = compiled.engine.state().side(side).active.clone();
    let parts: Vec<String> = text
        .map(|t| t.split(',').map(|p| p.trim().to_string()).collect())
        .unwrap_or_default();
    let mut actions = Vec::new();
    for (slot, actor) in actives.iter().enumerate() {
        let Some(actor) = *actor else {
            continue;
        };
        let part = parts.get(slot).map(String::as_str).unwrap_or("pass");
        let words: Vec<&str> = part.split_whitespace().collect();
        let action = match words.as_slice() {
            [] | ["pass"] => ActionChoice::Pass,
            ["move", ..] => {
                // The gimmick a choice asks for, as Showdown's `chooseMove` event suffixes
                // (DESIGN.md §44.8): `move 1 mega`, `move 1 zmove`, `move 2 dynamax`,
                // `move 1 terastallize`. Showdown strips a target and a suffix in either order
                // (`move 1 1 terastallize`, `move 1 terastallize 1`).
                let text = part["move".len()..].trim();
                let (choice, target, mechanic) = match split_mechanic(text) {
                    (rest, Some(mechanic)) => {
                        let (choice, target) = split_move_choice(&rest);
                        (choice, target, Some(mechanic))
                    }
                    (_, None) => {
                        let (choice, target) = split_move_choice(text);
                        let (choice, mechanic) = split_mechanic(&choice);
                        (choice, target, mechanic)
                    }
                };
                let pokemon = compiled.engine.state().pokemon(actor).unwrap();
                // `move N` counts in the request's move list; `move <name>` names the move
                // (Showdown matches it by id), which `tools/minimize.py` rewrites scripts into.
                let n = choice.parse::<usize>().unwrap_or_else(|_| {
                    let wanted = pokemon_battle_engine::data::to_id(&choice);
                    pokemon
                        .move_slots()
                        .iter()
                        .position(|slot| {
                            compiled.dex.move_by_id(slot.move_id).is_some_and(|m| {
                                pokemon_battle_engine::data::to_id(&m.name) == wanted
                            })
                        })
                        .unwrap_or_else(|| panic!("{actor:?} has no move named {choice}"))
                        + 1
                });
                // A hard-locked Pokémon (a two-turn move, a Rollout chain) gets a request with that
                // move alone (`move 1` names it whatever its slot); it aims where the lock does.
                // So does a semi-locked one (Gen 1-2 Bide: `getSemiLockedMove`), which can still
                // switch.
                let (slot_id, move_id) =
                    match pokemon.locked_move().or_else(|| pokemon.semi_locked_move()) {
                        // A lock onto a move the slots do not hold (a charged move a Metronome
                        // called): the request's only entry, which the engine reads as the lock.
                        Some(locked) => {
                            let index = pokemon
                                .move_slots()
                                .iter()
                                .position(|m| m.move_id == locked)
                                .unwrap_or(0);
                            (MoveSlot::new(index as u8).expect("move slot"), locked)
                        }
                        None => {
                            let slot_id =
                                MoveSlot::new(u8::try_from(n - 1).unwrap()).expect("move slot");
                            (slot_id, pokemon.move_slot(slot_id).unwrap().move_id)
                        }
                    };
                // Showdown `moveSlot.target`: a copied slot keeps where the copied use went (a
                // Sketch of a non-Ghost's Curse needs no target, `MoveSlotState::target`).
                let pattern = pokemon
                    .move_slot(slot_id)
                    .filter(|slot| slot.move_id == move_id)
                    .and_then(|slot| slot.target)
                    .unwrap_or_else(|| compiled.dex.move_by_id(move_id).unwrap().target);
                // Showdown `chooseMove`: a non-Ghost's Curse takes its `nonGhostTarget` (itself),
                // so a script names no target; `resolveAction` then draws a foe for it, where a
                // Ghost by the time it moves (a Ghost Terastallization) aims (R-M9-37).
                let pattern = match compiled.dex.move_by_id(move_id).unwrap().non_ghost_target {
                    Some(non_ghost)
                        if !pokemon_battle_engine::engine::has_type(
                            compiled.engine.state(),
                            &compiled.rules,
                            pokemon,
                            pokemon_battle_engine::dex::PokemonType::Ghost,
                        ) =>
                    {
                        non_ghost
                    }
                    _ => pattern,
                };
                // A locked action aims as its lock does (Showdown `chooseMove`'s
                // `lockedMoveTargetLoc`; its choice names none): no target for it by default.
                let target = match pokemon.hard_lock() {
                    Some(lock) => lock.target.map(DeclaredTarget),
                    None => declared_target(compiled, side, target, pattern),
                };
                ActionChoice::Move {
                    slot: slot_id,
                    target,
                    mechanic,
                }
            }
            ["switch", ..] => {
                let n = switch_index(compiled, side, part["switch".len()..].trim());
                ActionChoice::Switch {
                    pokemon: take_switch_target(compiled, side, slot, n),
                }
            }
            other => panic!("unsupported choice syntax: {other:?}"),
        };
        actions.push(SlotChoice { actor, action });
    }
    SideChoice { side, actions }
}

// ---------------------------------------------------------------------------------------------
// Oracle decisions from the Showdown event stream
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Who {
    pub side: usize,
    pub slot: Option<usize>,
    pub name: String,
}

/// Whether a `[from] <source>` names an effect Showdown runs as an action of its own
/// (`runMove`: a combined Pledge's promotion, Round's, Dancer) rather than a called move inside
/// the same action (`useMove`: Metronome, Magic Coat, Snatch …). A dance is read as the
/// dancer's own action (`ActionOracle::danced`), which the replay plans after the one it
/// follows.
pub(super) fn acts_on_its_own(from: &str) -> bool {
    from.contains("Fire Pledge")
        || from.contains("Grass Pledge")
        || from.contains("Water Pledge")
        || from.contains("move: Round")
        || from.contains("ability: Dancer")
}

/// A move an action called (`[from] …`): who used it, and its name.
#[derive(Debug, Clone, PartialEq)]
pub struct CalledMove {
    pub user: Who,
    pub name: String,
    /// Where its `move` line says it went.
    pub target: Option<Who>,
    /// What called it (`[from] move: Mirror Move`, `[from] Magic Coat`, `[from] ability: Magic
    /// Bounce`), as its `move` line says.
    pub from: Option<String>,
    /// Where its stretch of the action starts in the action's `missed`, `immune` and `applied`
    /// (their lengths as its `move` line came): what it did is reported from there to the next
    /// called move's start. Two foes sending one spread move back each strike the same
    /// Pokémon, each with its own checks (R-M8-176).
    pub outcomes_from: (usize, usize, usize),
    /// The Lightning Rod / Storm Drain holder that drew it (`ActionOracle::redirected_by`).
    pub redirected_by: Option<Who>,
}

impl CalledMove {
    pub(super) fn new(
        user: Who,
        name: String,
        target: Option<Who>,
        from: Option<String>,
        action: &ActionOracle,
    ) -> Self {
        CalledMove {
            user,
            name,
            target,
            from,
            outcomes_from: (
                action.missed.len(),
                action.immune.len(),
                action.applied.len(),
            ),
            redirected_by: None,
        }
    }
}

/// What the called move `called[index]` of an action did: its stretch of the action's misses,
/// immunities and effects.
pub(super) struct CalledOutcomes<'a> {
    pub(super) missed: &'a [Who],
    pub(super) immune: &'a [Who],
    pub(super) applied: &'a [(Who, String)],
}

pub(super) fn called_outcomes(action: &ActionOracle, index: usize) -> CalledOutcomes<'_> {
    let (missed, immune, applied) = action.called[index].outcomes_from;
    let end = action.called.get(index + 1).map_or(
        (
            action.missed.len(),
            action.immune.len(),
            action.applied.len(),
        ),
        |next| next.outcomes_from,
    );
    CalledOutcomes {
        missed: &action.missed[missed..end.0],
        immune: &action.immune[immune..end.1],
        applied: &action.applied[applied..end.2],
    }
}

/// One action as Showdown reported it. An action starts at a `move`, a `cant`, a confusion
/// `activate`, or a sleep / freeze `curestatus`; everything that follows for the same user
/// belongs to it until the next action starts.
#[derive(Debug, Clone)]
pub struct ActionOracle {
    pub user: Who,
    pub move_name: Option<String>,
    /// The user took its move's crash damage (High Jump Kick hitting no target).
    pub crashed: bool,
    /// The `move` event's target: where a single-target move actually went after execution-time
    /// retargeting (Follow Me, Lightning Rod, a fainted target).
    pub move_target: Option<Who>,
    /// The target the action's own `move` line named — not a called move's, which `move_target`
    /// takes (a Magic Bounce's reflection names the reflected move's user): where a random target
    /// drawn before BeforeMove went (an Encore's override, R-M10-73).
    pub own_target: Option<Who>,
    /// Gen 1 Mimic's draw among the target's move slots (`this.sample(target.moves)`, DESIGN.md
    /// §42.5): the move it learned, from the log's `-start|…|Mimic|<move>` line. `None` while
    /// Mimic failed (nothing to learn).
    pub mimicked: Option<String>,
    /// A `-fail` on the user named its move (`-fail|p|move: Ally Switch`): the move's own
    /// handler refused after `PrepareHit` went through — a bare `-fail|p` is `PrepareHit`'s.
    pub fail_named_the_move: bool,
    /// Showdown `cant` reason (par / slp / frz / flinch), or `confusion` for a self-hit.
    pub cant: Option<String>,
    pub hit: bool,
    /// One entry per hit that reached the target (HP or Substitute): was it a critical hit?
    pub crits: Vec<bool>,
    /// Aligned with `crits`: did that hit land on a Substitute? (Showdown rolls no
    /// secondary for the target behind it.)
    pub substitute_hits: Vec<bool>,
    /// Aligned with `crits`: the Pokémon each hit reached. A spread move reports a Substitute
    /// hit on one target before the other targets' damage (Showdown settles Substitutes first),
    /// so the plan reads each target's hits by target.
    pub hit_targets: Vec<Option<Who>>,
    /// Aligned with `crits`: the HP a hit left its target at (none for a Substitute's hit).
    pub hit_hp: Vec<Option<u16>>,
    /// `-activate|p|item: Focus Band`: the Pokémon it held, and how many hits had landed before
    /// (the band's hit is the next one; a confusion self-hit or a crash has none).
    pub focus_band_held: Vec<(Who, usize)>,
    /// `-enditem|p|Focus Sash`: the Pokémon whose sash held (a roll that leaves a holder at full
    /// HP on 1 by itself shows the same HP and keeps the sash).
    pub focus_sash_held: Vec<Who>,
    /// `-ability|p|Sturdy` (no `[from]`: an OHKO move it stops is `-immune|p|[from] ability:
    /// Sturdy`): the Pokémon Sturdy left on 1 from full HP.
    pub sturdy_held: Vec<Who>,
    /// Whose Substitute each such hit struck, and whether it broke. The protocol does not
    /// show a Substitute's HP; the boundary frame does.
    pub substitute_struck: Vec<(Who, bool)>,
    /// A damaging move failed before hitting anything (`-fail|user`: Fake Out after the first
    /// turn, Sucker Punch …): it draws no hit decision.
    pub failed: bool,
    /// The move had nobody to aim at (`[notarget]`): no decision at all, status move or not.
    pub no_target: bool,
    /// `-hitcount` of a multi-hit move: the hits that actually landed.
    pub hit_count: Option<u16>,
    /// Targets the move missed (`-miss|user|target`) and targets it could not affect
    /// (`-immune|target`); a spread move reports these per target.
    pub missed: Vec<Who>,
    pub immune: Vec<Who>,
    /// The targets among `immune` stopped at TryHit (a guard, an absorbing ability, Lightning
    /// Rod) rather than by a type immunity: in Gen 3 these passed the accuracy roll.
    pub try_hit_blocked: Vec<Who>,
    /// The targets among `try_hit_blocked` a guard stopped (`-activate|p|move: Protect`, Max
    /// Guard …): nothing of the target's own showed.
    pub guarded: Vec<Who>,
    /// Pokémon dragged in by this action (`drag` events: Roar, Dragon Tail, Red Card), in
    /// order; each names the slot it entered.
    pub dragged_in: Vec<Who>,
    /// Every HP value this action committed, in order: move damage, confusion self-hit, recoil,
    /// drain / self heal. Compared one-to-one with the engine's HP commits for the action.
    pub damages: Vec<(Who, u16)>,
    pub secondary_applied: bool,
    /// The damaged targets a secondary landed on, with what it did (`status:brn`, `boost`,
    /// `unboost`, `start:confusion`, `flinch` …): a spread move rolls per target (Discharge may
    /// paralyse one foe and not the other), and Fire Fang's flinch is not its burn. The count is
    /// how many hits had landed when it showed (a multi-hit move rolls after every hit);
    /// `usize::MAX` for a flinch, which only the victim's later `cant` shows.
    pub secondary_on: Vec<(Who, String, usize)>,
    /// The Pokémon the actor's Magician took an item from (`-item|actor|…|[from] ability:
    /// Magician|[of] target`).
    pub magician_from: Option<Who>,
    /// A contact reaction on the actor (`-status … [from] ability: Static|[of] …`).
    pub contact_status: Option<String>,
    /// How many hits had landed when that contact status came (a multi-hit move rolls the
    /// reaction after every hit; only the one after this hit succeeded).
    pub contact_status_after_hits: usize,
    /// Whose ability it was (`[of] p2a: Pinsir`): a spread move reports every hit before the
    /// reactions, so the reaction belongs to the target named here.
    pub contact_status_of: Option<Who>,
    /// Every hit count a contact status came after: a berry that cured the first lets a later
    /// hit's reaction take again (Lum Berry between two Flame Body burns, R-M8-138).
    pub contact_status_hits: Vec<usize>,
    /// Sweet Veil stopped a sleep on the user after its hits (`-block|user|ability: Sweet
    /// Veil`, which it announces whatever put it to sleep): a contact ability's draw that came
    /// to sleep (Effect Spore), with the hits by then (R-M11-15).
    pub contact_sleep_blocked: Vec<usize>,
    /// The move a Disable sealed (`-start|target|Disable|Move`): Gen 1 picks it at random.
    pub disabled_move: Option<String>,
    /// Gen 1: the slot it drew (`debug|Disable: disabling move X in slot N`, the oracle's
    /// `disableslot` event) — two slots may hold the move (a Mimic's copy). Recordings older than
    /// the event leave it to the move's name.
    pub disabled_slot: Option<usize>,
    /// The types a `-start|p|typechange|T` line of this action names (Conversion's draw), by
    /// Pokémon; `[silent]` and ability-caused ones left out.
    pub typechanges: Vec<(Who, String)>,
    /// Cursed Body disabled the actor's move (`-start|user|Disable|…|[from] ability: Cursed
    /// Body|[of] holder`): how many hits had landed, and whose ability it was.
    pub cursed_body: Option<(usize, Option<Who>)>,
    /// The holder's Mummy, Lingering Aroma or Wandering Spirit took the actor's ability
    /// (`-activate|holder|ability: Mummy|user|[ability] …`; Wandering Spirit's
    /// `-activate|user|Skill Swap|…|[of] holder`): how many hits had landed. Its own hit
    /// reactions roll no more after (R-M8-86, R-M8-133).
    pub ability_taken_after: Option<usize>,
    /// The ability went by Wandering Spirit's trade (`-activate|user|Skill Swap|…`): the holder
    /// now has the user's, which may react to the later hits itself (Static, R-M9-35).
    pub ability_traded: bool,
    /// Poison Touch / Toxic Chain statused a target (`-status|target|…|[from] ability: Poison
    /// Touch|[of] user`): the target, and how many hits had landed.
    pub dealt_status: Vec<(Who, usize)>,
    /// Cute Charm infatuated the actor (`-start|user|Attract|[from] ability: Cute Charm|[of]
    /// holder`): how many hits had landed, and whose ability it was.
    pub cute_charm: Option<(usize, Option<Who>)>,
    /// Every hit count a Cute Charm infatuation came after (Mental Herb between two, R-M8-138).
    pub cute_charm_hits: Vec<usize>,
    /// Protect / Detect succeeded (`-singleturn`).
    pub protected: bool,
    /// Asleep, the actor used a `sleepUsable` move (Snore, Sleep Talk): its `cant|p|slp` is
    /// part of this action, which goes ahead (DESIGN.md §42.1).
    pub slept_through: bool,
    /// The chain of moves this action called (Showdown's further `move` lines with `[from]
    /// move: …`, DESIGN.md §42.1), outermost first: the engine runs them after the caller's own
    /// effect, so the plan and the fates are the innermost move's. A reflection's called move
    /// has another user than the action's (§42.8).
    pub called: Vec<CalledMove>,
    /// Status the actor shook off at the start of the action (slp / frz).
    pub cured: Option<String>,
    /// The actor's `defrost` move thawed it (`-curestatus … frz|[from] move: …`).
    pub defrosted: bool,
    /// Targets the move's hit thawed (a Fire move; Gen 2 Tri Attack drawing a burn).
    pub thawed: Vec<Who>,
    /// The power a move drew as it started (`-activate|user|move: Magnitude|N`).
    pub magnitude: Option<u16>,
    /// The PP the user's Spite took (`-activate|target|move: Spite|move|N`; Gen 2-3: the draw).
    pub spite_drop: Option<u16>,
    /// A move's own power chance took (`-activate|user|move: Fickle Beam`).
    pub power_chance: bool,
    /// The category a move chose as it started, when it says (`[anim] Shell Side Arm Physical`
    /// on its line, `-hint|Physical Shell Side Arm` once it hits): true for physical.
    pub physical_hint: Option<bool>,
    /// Effects the action applied, as (target, kind): `status:par`, `start:confusion`,
    /// `boost`, `heal`. Status moves report through these, not through damage.
    pub applied: Vec<(Who, String)>,
    /// The user's own effects among `applied`, each with the hits shown before it: a multi-hit
    /// move's self-boost (Parental Bond Fiery Dance) belongs to the hit it follows.
    pub user_applied_hits: Vec<(String, usize)>,
    /// A berry the Pokémon ate drew a decision at the `Update` that fed it: a confusion berry
    /// confused it (`-enditem|p|Figy Berry|[eat]`, its heal, `-start|p|confusion`: the duration),
    /// Starf Berry raised a stat (`-boost|p|spd|2|[from] item: Starf Berry`: the stat). With how
    /// many hits had landed. Not a secondary.
    pub berry_draws: Vec<(Who, usize, BerryDraw)>,
    /// The user that took the target's berry and ate it (`-enditem|target|X Berry|[from]
    /// stealeat|[move] Pluck|[of] user`): Pluck's and Bug Bite's `onHit`, so its draw comes before
    /// the hit's `DamagingHit` reactions, not at the `Update` after them (R-M9-48).
    pub stolen_berry: Option<Who>,
    /// The Pokémon that ate the berry this Fling threw at it (`flung`): eaten in the move's
    /// `onHit`, so its draws come before the hit's reactions, as a stolen one's (R-M10-3).
    pub flung_berry: Option<Who>,
    /// A Pursuit striking a Pokémon as it switches out (`-activate|target|move: Pursuit` right
    /// before its move line): Gen 3-4 run it without BeforeMove.
    pub intercepted: bool,
    /// The user's lock (Thrash and its kind) ended in this action and confused it
    /// (`-start|user|confusion|[fatigue]`; Gen 1's `[silent]`, as the last use starts).
    pub fatigue: bool,
    /// A Dancer dancing along after the action before it (`-activate|dancer|ability: Dancer`,
    /// §42.9): Showdown's `runMove` inside that action, which the engine runs there too — the
    /// replay plans the dance after that action's own move (DESIGN.md R-M8-78).
    pub danced: bool,
    /// An instructed Pokémon's extra use of its move (`-singleturn|p|move: Instruct` right
    /// before it): the instructor's action injects it, so it is no turn action of its user's —
    /// the speed-tie ranking reads that user's own action.
    pub instructed: bool,
    /// The priority chances that went off as Instruct built that use (`resolveAction`'s
    /// `FractionalPriority`: `-activate|p|ability: Quick Draw` or `item: Quick Claw` between the
    /// `-singleturn` and the move, DESIGN.md R-M9-32).
    pub instructed_chances: Vec<PriorityChance>,
    /// A Rage user's rage as the boundary frame shows it (`attribute_frame_rage`): Gen 3's flag
    /// is Rage's accuracy check, which Showdown's `-immune` hides against a Ghost (§15.19).
    pub raging_after: Option<bool>,
    /// Forewarn warning as an ability the action's move handed over starts (Skill Swap, Role
    /// Play, Entrainment, Doodle, Transform — R-M10-46; Wandering Spirit's trade): each `-activate|holder|ability:
    /// Forewarn|<move>|[of] <foe>` line's arguments. Its draw is the action's; an entrant's is
    /// `entrant_ability_tape`'s.
    pub forewarned: Vec<Vec<String>>,
    /// The Lightning Rod / Storm Drain holder that drew the action's own move
    /// (`-activate|holder|ability: Storm Drain` right after its `move` line, which names the
    /// holder): the target the move was aimed at first is another (R-M11-7).
    pub redirected_by: Option<Who>,
}

impl ActionOracle {
    pub(super) fn new(user: Who) -> Self {
        Self {
            user,
            move_name: None,
            move_target: None,
            own_target: None,
            mimicked: None,
            fail_named_the_move: false,
            cant: None,
            hit: true,
            crits: Vec::new(),
            substitute_hits: Vec::new(),
            hit_targets: Vec::new(),
            hit_hp: Vec::new(),
            focus_band_held: Vec::new(),
            focus_sash_held: Vec::new(),
            sturdy_held: Vec::new(),
            substitute_struck: Vec::new(),
            failed: false,
            no_target: false,
            hit_count: None,
            missed: Vec::new(),
            immune: Vec::new(),
            crashed: false,
            try_hit_blocked: Vec::new(),
            guarded: Vec::new(),
            dragged_in: Vec::new(),
            damages: Vec::new(),
            secondary_applied: false,
            secondary_on: Vec::new(),
            magician_from: None,
            contact_status: None,
            contact_status_after_hits: 0,
            contact_status_of: None,
            contact_status_hits: Vec::new(),
            contact_sleep_blocked: Vec::new(),
            disabled_move: None,
            disabled_slot: None,
            typechanges: Vec::new(),
            cursed_body: None,
            ability_taken_after: None,
            ability_traded: false,
            dealt_status: Vec::new(),
            cute_charm: None,
            cute_charm_hits: Vec::new(),
            protected: false,
            slept_through: false,
            called: Vec::new(),
            cured: None,
            defrosted: false,
            thawed: Vec::new(),
            magnitude: None,
            spite_drop: None,
            power_chance: false,
            physical_hint: None,
            applied: Vec::new(),
            user_applied_hits: Vec::new(),
            berry_draws: Vec::new(),
            stolen_berry: None,
            flung_berry: None,
            intercepted: false,
            fatigue: false,
            raging_after: None,
            forewarned: Vec::new(),
            redirected_by: None,
            danced: false,
            instructed: false,
            instructed_chances: Vec::new(),
        }
    }

    /// The move the action used: the innermost it called, if it called one (Metronome's Jump
    /// Kick crashes as its own, R-M8-61; Metronome's Spite shows its own drop, R-M8-67; its Rest
    /// heals silently, R-M8-70).
    pub(super) fn used_move(&self) -> Option<&String> {
        self.called
            .last()
            .map(|called| &called.name)
            .or(self.move_name.as_ref())
    }
}

/// The abilities that stop another Pokémon's move once it started (`onAnyTryMove` /
/// `onFoeTryMove`: `cant|holder|ability: …|move|[of] user`): the move's user does not go ahead.
pub(super) const TRY_MOVE_STOPPERS: [&str; 4] = [
    "ability: Damp",
    "ability: Dazzling",
    "ability: Queenly Majesty",
    "ability: Armor Tail",
];

/// The berries that confuse a holder whose nature dislikes them.
pub(super) const CONFUSION_BERRIES: [&str; 5] = [
    "Figy Berry",
    "Wiki Berry",
    "Mago Berry",
    "Aguav Berry",
    "Iapapa Berry",
];

/// What eating a berry drew (DESIGN.md R145, R185).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BerryDraw {
    /// A confusion berry's confusion: its duration, read from the frames.
    Confusion,
    /// Starf Berry's stat: its place in `engine::STAGED_STATS`, which the draw names.
    Stat(u16),
}

impl BerryDraw {
    /// The stage change `event` right after `eaten` (a Pokémon and the berry it just ate, from
    /// `berry_eaten`): the berry's own (a pinch berry's `this.boost`) — and for Starf Berry the
    /// stat it drew.
    pub(super) fn berry_boost(
        eaten: Option<&(Who, String)>,
        event: &Value,
    ) -> Option<(Who, Option<Self>)> {
        let (eater, berry) = eaten?;
        if !matches!(event["kind"].as_str(), Some("boost" | "unboost"))
            || serde_json::from_value::<Who>(event["pokemon"].clone())
                .ok()
                .as_ref()
                != Some(eater)
        {
            return None;
        }
        let draw = (berry == "Starf Berry")
            .then(|| labelled_stat(&format!("boost:{}", event["stat"].as_str()?)))
            .flatten()
            .and_then(|stat| {
                pokemon_battle_engine::engine::STAGED_STATS
                    .iter()
                    .position(|staged| *staged == stat)
            })
            .map(|code| Self::Stat(code as u16));
        Some((eater.clone(), draw))
    }
}

/// A Fling's thrown berry (`events` after its `move`, up to the next action) and the Pokémon
/// it struck (`target`).
pub(super) fn flung(events: &[Value], target: &Value) -> Option<(Who, String)> {
    let target = serde_json::from_value::<Who>(target.clone()).ok()?;
    events
        .iter()
        .take_while(|event| !matches!(event["kind"].as_str(), Some("move" | "cant" | "turn")))
        .find_map(|event| {
            (event["kind"] == "enditem" && event["args"][2] == "[from] move: Fling")
                .then(|| event["args"][1].as_str())
                .flatten()
                .filter(|item| item.ends_with(" Berry"))
                .map(|berry| (target.clone(), berry.to_string()))
        })
}

/// `-enditem|p|X Berry|[eat]`, or a berry taken and eaten by another
/// (`-enditem|target|X Berry|[from] stealeat|[move] Bug Bite|[of] user`): the eater and the
/// berry.
pub(super) fn berry_eaten(event: &Value) -> Option<(Who, String)> {
    if event["kind"] != "enditem" {
        return None;
    }
    let eater = if event["args"][2] == "[eat]" {
        who_from_text(&event["args"][0])?
    } else if event["args"][2] == "[from] stealeat" {
        let user = event["args"][4].as_str()?.strip_prefix("[of] ")?;
        who_from_text(&Value::from(user))?
    } else {
        return None;
    };
    let berry = event["args"][1].as_str()?;
    Some((eater, berry.to_string()))
}

/// Where a berry drew a decision for a Pokémon outside any action (DESIGN.md R145).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BerryConfusionAt {
    /// The switch-in of a Pokémon a switch brought in (hazards took its HP down).
    Entry,
    /// The `Update` right after the weather's residual (Gen 7+).
    AfterWeather,
    /// The residual's closing `Update`.
    ResidualEnd,
}

/// Every berry outside an action's that drew a decision: the event position of its
/// `-start|p|confusion` (a confusion berry) or `-boost` (Starf Berry), the eater, where, and
/// what it drew.
pub(super) fn outside_berry_confusions(
    events: &[Value],
    after_weather: bool,
) -> Vec<(usize, Who, BerryConfusionAt, BerryDraw)> {
    let mut out = Vec::new();
    let mut entrant: Option<Who> = None;
    let mut in_residual = false;
    let mut eaten: Option<(Who, usize)> = None;
    let mut just_eaten: Option<(Who, String)> = None;
    for (position, event) in events.iter().enumerate() {
        let previous = just_eaten.take();
        just_eaten = berry_eaten(event);
        let kind = event["kind"].as_str().unwrap_or("");
        match kind {
            "switch" => entrant = serde_json::from_value::<Who>(event["pokemon"].clone()).ok(),
            "move" | "cant" => entrant = None,
            "residual" => in_residual = true,
            "turn" => in_residual = false,
            _ => {}
        }
        // Starf Berry's boost follows its `-enditem` at once.
        let (eater, at, draw) =
            if let Some((eater, Some(draw))) = BerryDraw::berry_boost(previous.as_ref(), event) {
                (eater, position.saturating_sub(1), draw)
            } else {
                if let Some(eater) = confusion_berry_eaten(event) {
                    eaten = Some((eater, position));
                    continue;
                }
                let Some((eater, at)) = eaten.take() else {
                    continue;
                };
                if kind == "heal" {
                    eaten = Some((eater, at));
                    continue;
                }
                // A lock's fatigue right after a berry that did not confuse (Gen 3's Iapapa Berry
                // healing a Pokémon whose Outrage ends in the same residual) is not the berry's
                // (R-M8-79).
                if kind != "start"
                    || event["args"][1] != "confusion"
                    || event["args"]
                        .as_array()
                        .is_some_and(|args| args.iter().any(|arg| arg == "[fatigue]"))
                    || who_from_text(&event["args"][0]).as_ref() != Some(&eater)
                {
                    continue;
                }
                (eater, at, BerryDraw::Confusion)
            };
        let place = if in_residual {
            let weather = at.checked_sub(1).is_some_and(|before| {
                events[before]["from"]
                    .as_str()
                    .is_some_and(|from| from == "[from] Sandstorm" || from == "[from] Hail")
            });
            if after_weather && weather {
                BerryConfusionAt::AfterWeather
            } else {
                BerryConfusionAt::ResidualEnd
            }
        } else if entrant.is_some() {
            // At an entry: the entrant's berry (on the hazards), or another's its `Update` fed
            // with the entry (a Gen 4 pinch berry after a replacement came in, R-M8-107).
            BerryConfusionAt::Entry
        } else {
            continue;
        };
        out.push((position, eater, place, draw));
    }
    out
}

/// Whether `event` is a Pokémon eating a confusion berry; the Pokémon.
pub(super) fn confusion_berry_eaten(event: &Value) -> Option<Who> {
    berry_eaten(event)
        .filter(|(_, berry)| CONFUSION_BERRIES.contains(&berry.as_str()))
        .map(|(eater, _)| eater)
}

/// A sleeping Pokémon's Snore or Sleep Talk (`sleepUsable`, DESIGN.md §42.1): its `cant|p|slp`
/// opened the very action that goes on with the user's next BeforeMove check or its move —
/// a sleeping Pokémon's other moves stop at the `cant`.
pub(super) fn sleeps_through(out: &mut [ActionOracle], user: &Who) {
    if let Some(last) = out.last_mut()
        && last.user == *user
        && last.cant.as_deref() == Some("slp")
        && last.move_name.is_none()
    {
        last.cant = None;
        last.hit = true;
        last.slept_through = true;
    }
}

/// The place of the called move in its caller's candidate table (Showdown's `sample`): the
/// replay reads it back from the `move` line the log prints (`[from] move: Metronome`).
/// The name a called move takes when the log does not give one: a sound move Throat Chop
/// refused (`cant|user|move: Throat Chop`, R-M8-148).
pub(super) const UNNAMED_SOUND_MOVE: &str = "(a sound move)";

pub(super) fn called_draw_index(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    actor: PokemonId,
    caller: &str,
    called: &str,
) -> Option<u16> {
    let candidates = match caller {
        "Metronome" => {
            pokemon_battle_engine::engine::metronome_candidates(state, rules, dex, actor)
        }
        "Assist" => pokemon_battle_engine::engine::assist_candidates(state, rules, dex, actor),
        "Sleep Talk" => {
            pokemon_battle_engine::engine::sleep_talk_candidates(dex, state.pokemon(actor)?)
        }
        _ => return None,
    };
    if candidates.len() < 2 {
        return None;
    }
    // A sound move Throat Chop refused, unnamed: any sound move among the candidates does the
    // same — refused before its `move` line, nothing drawn (R-M8-148).
    if called == UNNAMED_SOUND_MOVE {
        return candidates
            .iter()
            .position(|id| dex.move_by_id(*id).is_some_and(|m| m.flags.sound))
            .map(|index| index as u16);
    }
    // By id: a `move` line names the move, Gen 3 Sleep Talk's `cant … nopp` its id.
    let called_id = dex
        .moves()
        .iter()
        .find(|move_def| {
            pokemon_battle_engine::data::to_id(&move_def.name)
                == pokemon_battle_engine::data::to_id(called)
        })
        .map(|move_def| move_def.id)?;
    Some(candidates.iter().position(|id| *id == called_id)? as u16)
}

/// The draws of an action's chain of called moves: for each caller in turn (the action's own
/// move first, then what it called), the place of the move it called in its candidate table,
/// read back from the `[from] move: …` lines the log prints. A caller with no table of its own
/// (Copycat, Mirror Move, Nature Power) draws nothing.
pub(super) fn called_draws(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    actor: PokemonId,
    oracle: &ActionOracle,
) -> Vec<Option<RandomOverride>> {
    let mut tape = Vec::new();
    let mut callers: Vec<String> = Vec::new();
    if let Some(name) = oracle.move_name.clone() {
        callers.push(name);
    }
    for index in 0..oracle.called.len() {
        let caller = callers[index].clone();
        let called = oracle.called[index].name.clone();
        if let Some(draw) = called_draw_index(state, rules, dex, actor, &caller, &called) {
            tape.push(Some(RandomOverride::range(RandomRequestKind::Sample, draw)));
        }
        callers.push(called);
    }
    tape
}

/// Whether a thaw starts the thawed Pokémon's own action: its own thaw is the first of its
/// BeforeMove's checks, so the events after it, up to its `move` or `cant` — or its confusion
/// striking, which ends the action in a self-hit — are all its own (a confusion wearing off, an
/// attraction, Struggle's `-activate`). Anything about another Pokémon, a switch, the turn's end
/// comes after a hit's thaw instead (R-M9-52).
pub(super) fn own_action_follows(rest: &[Value], thawed: &Who) -> bool {
    let is =
        |value: &Value| serde_json::from_value::<Who>(value.clone()).ok().as_ref() == Some(thawed);
    for event in rest {
        match event["kind"].as_str().unwrap_or("") {
            "move" => return is(&event["user"]),
            "cant" => return is(&event["pokemon"]),
            // Confusion striking: the action ends in the self-hit, with no `move` or `cant`.
            "activate" if event["args"][1] == "confusion" => {
                return who_from_text(&event["args"][0]).as_ref() == Some(thawed);
            }
            "turn" | "upkeep" | "residual" | "switch" | "drag" => return false,
            _ => {
                let about_it = is(&event["pokemon"])
                    || who_from_text(&event["args"][0]).as_ref() == Some(thawed);
                if !about_it {
                    return false;
                }
            }
        }
    }
    false
}
