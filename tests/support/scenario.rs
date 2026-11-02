//! Neutral scenario compiler and the oracle-guided replay driver (TESTING.md §4–6, §10).
//!
//! A scenario names teams and choices by name; the compiler resolves them through the dex and
//! the stat formula. Replay drives the Rust engine turn by turn against the Showdown adapter's
//! output: Showdown's per-event log fixes the semantic random decisions (miss / crit /
//! secondary), and the damage roll is recovered by trying all sixteen rolls on a snapshot and
//! keeping the one whose committed HP equals Showdown's. Frames are compared at every request
//! boundary; the first divergence is reported with the surrounding facts.

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

use super::{capture, compare_json};

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

fn default_level() -> u8 {
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

fn spread(map: &Option<BTreeMap<String, u8>>, default: u8) -> StatSpread {
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
    /// Scenario labels per side, parallel to `teams` (Showdown names Pokémon by label).
    pub labels: [Vec<String>; 2],
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
fn flush_trace(compiled: &mut Compiled) {
    let events = compiled.engine.take_trace_events();
    compiled.log.extend(events);
}

/// The engine id of the Pokémon Showdown calls `who` (by side and label).
fn label_to_id(compiled: &Compiled, who: &Who) -> Option<PokemonId> {
    let side = who.side.min(1);
    compiled.labels[side]
        .iter()
        .position(|label| *label == who.name)
        .map(|index| compiled.teams[side][index])
}

/// A `move` choice's move (an index or a name) and its target: Showdown reads a trailing
/// `[-+]1..3` as the target location (except in Conversion 2's name).
fn split_move_choice(text: &str) -> (String, Option<i32>) {
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
fn switch_index(compiled: &Compiled, side: SideId, text: &str) -> usize {
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
fn take_switch_target(compiled: &mut Compiled, side: SideId, slot: usize, n: usize) -> PokemonId {
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
fn follow_position_swaps(compiled: &mut Compiled, from: usize) {
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
fn follow_drags(compiled: &mut Compiled, from: usize) {
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
fn residual_drag_tape(
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
fn settle_party_swaps(compiled: &mut Compiled, from: usize) {
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
fn declared_target(
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
fn team_choice(text: Option<&str>) -> TeamChoice {
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
fn compile_format(scenario: &Scenario, format_id: &str) -> Compiled {
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
    Compiled {
        rules: Arc::new(format.rules().unwrap_or_else(|e| fail(&e))),
        dex: format.dex().clone(),
        engine,
        pending_swaps: Vec::new(),
        party: teams.clone(),
        labels,
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
        teams,
    }
}

/// The gimmick suffix of a move choice (`move 1 terastallize`), as `MechanicChoice`
/// (DESIGN.md §44.8): Showdown's `chooseMove` event suffix, stripped off the slot text.
fn split_mechanic(choice: &str) -> (String, Option<MechanicChoice>) {
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
fn parse_choice(compiled: &mut Compiled, side: SideId, text: Option<&str>) -> SideChoice {
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
fn acts_on_its_own(from: &str) -> bool {
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
    fn new(
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
struct CalledOutcomes<'a> {
    missed: &'a [Who],
    immune: &'a [Who],
    applied: &'a [(Who, String)],
}

fn called_outcomes(action: &ActionOracle, index: usize) -> CalledOutcomes<'_> {
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
    fn new(user: Who) -> Self {
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
    fn used_move(&self) -> Option<&String> {
        self.called
            .last()
            .map(|called| &called.name)
            .or(self.move_name.as_ref())
    }
}

/// The abilities that stop another Pokémon's move once it started (`onAnyTryMove` /
/// `onFoeTryMove`: `cant|holder|ability: …|move|[of] user`): the move's user does not go ahead.
const TRY_MOVE_STOPPERS: [&str; 4] = [
    "ability: Damp",
    "ability: Dazzling",
    "ability: Queenly Majesty",
    "ability: Armor Tail",
];

/// The berries that confuse a holder whose nature dislikes them.
const CONFUSION_BERRIES: [&str; 5] = [
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
    fn berry_boost(eaten: Option<&(Who, String)>, event: &Value) -> Option<(Who, Option<Self>)> {
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
fn flung(events: &[Value], target: &Value) -> Option<(Who, String)> {
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
fn berry_eaten(event: &Value) -> Option<(Who, String)> {
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
enum BerryConfusionAt {
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
fn outside_berry_confusions(
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
fn confusion_berry_eaten(event: &Value) -> Option<Who> {
    berry_eaten(event)
        .filter(|(_, berry)| CONFUSION_BERRIES.contains(&berry.as_str()))
        .map(|(eater, _)| eater)
}

/// A sleeping Pokémon's Snore or Sleep Talk (`sleepUsable`, DESIGN.md §42.1): its `cant|p|slp`
/// opened the very action that goes on with the user's next BeforeMove check or its move —
/// a sleeping Pokémon's other moves stop at the `cant`.
fn sleeps_through(out: &mut [ActionOracle], user: &Who) {
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
const UNNAMED_SOUND_MOVE: &str = "(a sound move)";

fn called_draw_index(
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
fn called_draws(
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
fn own_action_follows(rest: &[Value], thawed: &Who) -> bool {
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

fn current(out: &mut Vec<ActionOracle>, user: Who) -> &mut ActionOracle {
    let fresh = match out.last() {
        None => true,
        Some(last) => last.user != user || last.move_name.is_some() || last.cant.is_some(),
    };
    if fresh {
        out.push(ActionOracle::new(user));
    }
    out.last_mut().expect("just pushed")
}

/// What reading a generation's log needs to know about its rules.
#[derive(Debug, Clone, Default)]
pub struct LogReading {
    /// The generation's poison / burn and Leech Seed act right after the user's own action
    /// (Gen 2, `StatusResidualTiming::AfterOwnAction`), so their HP commits before the residual
    /// marker belong to that action.
    pub status_after_action: bool,
    /// Moves whose hit thaws a frozen target (`engine::thaws_target_on_hit`), by name: their
    /// target's `-curestatus … frz` belongs to the hit, not to the target's own turn.
    pub thawing_moves: std::collections::BTreeSet<String>,
    /// Moves with a flinch secondary, by name: a victim's `cant … flinch` belongs to the last of
    /// them that hit it, not to a later hit that cannot flinch (Lava Plume after Stomp).
    pub flinching_moves: std::collections::BTreeSet<String>,
    /// Gen 1 (`ConditionRules::flinch_clears_recharge`, the gen1 mod's `flinch.onStart`): a
    /// flinch clears a recharge, so a frame without the `mustrecharge` a recharge move left
    /// shows a flinch nothing else does (R-M8-105).
    pub flinch_clears_recharge: bool,
    /// Moves their user must recharge after (Showdown `recharge`: Hyper Beam), by name.
    pub recharge_moves: std::collections::BTreeSet<String>,
    /// Moves with crash damage, by name: damage on their user with no source (Gen 1-4) or from
    /// a crash condition (`[from] highjumpkick`, Gen 5+) is their crash.
    pub crashing_moves: std::collections::BTreeSet<String>,
    /// Moves that cure their target's status (Purify; Jungle Healing and Lunar Blessing on their
    /// user's side), by name, each with the one status it cures, if only one (Worry Seed's
    /// sleep, R-M8-103): the target's `-curestatus` of it within them is the move's, not the
    /// target waking up or thawing on its own turn.
    pub curing_moves: BTreeMap<String, Option<&'static str>>,
    /// The curing moves aimed at the user's own side (Jungle Healing, Lunar Blessing: `allies`;
    /// Aromatherapy, Heal Bell: `allyTeam`): a foe's `-curestatus` within them is its own
    /// (R-M8-113).
    pub own_side_curing_moves: std::collections::BTreeSet<String>,
    /// Abilities whose `Update` cures a status (Immunity, Insomnia, Vital Spirit, Limber, Water
    /// Veil, Magma Armor …), by name: the `-curestatus` right after their `-activate` is the
    /// ability's, not the holder waking or thawing on its own turn. Another ability's
    /// `-activate` (Quick Draw's priority chance) says nothing about a cure that follows
    /// (DESIGN.md R-M8-49).
    pub curing_abilities: std::collections::BTreeSet<String>,
}

impl LogReading {
    pub fn for_rules(rules: &ResolvedRules, dex: &ResolvedDex) -> Self {
        Self {
            status_after_action: rules.status().residual_timing
                == pokemon_battle_engine::rules::StatusResidualTiming::AfterOwnAction,
            // (Gen 2 Tri Attack thaws when its `statusRoll` is a burn.)
            thawing_moves: dex
                .moves()
                .iter()
                .filter(|m| {
                    pokemon_battle_engine::engine::thaws_target_on_hit(rules, m, None)
                        || (m.behavior.status_pick_before_chance
                            && pokemon_battle_engine::engine::thaws_target_on_hit(
                                rules,
                                m,
                                Some(NonVolatileStatus::Burn),
                            ))
                })
                .map(|m| m.name.clone())
                .collect(),
            flinching_moves: dex
                .moves()
                .iter()
                .filter(|m| {
                    m.secondaries.iter().any(|s| {
                        s.effect == SecondaryEffect::Volatile(VolatileConditionKind::Flinch)
                    })
                })
                .map(|m| m.name.clone())
                .collect(),
            flinch_clears_recharge: rules.conditions().flinch_clears_recharge,
            recharge_moves: dex
                .moves()
                .iter()
                .filter(|m| m.flags.recharge)
                .map(|m| m.name.clone())
                .collect(),
            crashing_moves: dex
                .moves()
                .iter()
                .filter(|m| m.behavior.crash_damage)
                .map(|m| m.name.clone())
                .collect(),
            curing_abilities: dex
                .abilities()
                .iter()
                .filter(|ability| {
                    matches!(
                        pokemon_battle_engine::engine::ability_behavior(rules, ability.id).update,
                        Some(
                            pokemon_battle_engine::effects::abilities::AbilityUpdate::CureStatus(_)
                        )
                    )
                })
                .map(|ability| ability.name.clone())
                .collect(),
            own_side_curing_moves: dex
                .moves()
                .iter()
                .filter(|m| {
                    matches!(
                        m.target,
                        pokemon_battle_engine::rules::TargetPattern::UserAndAllies
                            | pokemon_battle_engine::rules::TargetPattern::UserTeam
                            | pokemon_battle_engine::rules::TargetPattern::UserSide
                    )
                })
                .map(|m| m.name.clone())
                .collect(),
            curing_moves: dex
                .moves()
                .iter()
                .filter_map(|m| {
                    let any = matches!(
                        m.primary_effect,
                        PrimaryEffect::Status(StatusMoveEffect::CureTarget(_))
                    ) || pokemon_battle_engine::effects::moves::cures_targets(m.id);
                    // Worry Seed cures the sleep its Insomnia would (`onHit`: `target.status ===
                    // 'slp'`); Wake-Up Slap, Smelling Salts their own status as they hit
                    // (R-M8-93); Gen 5+ Uproar wakes the sleepers as it hits.
                    let only = if m.id == MoveId::WORRY_SEED
                        || (m.behavior.uproar
                            && rules.conditions().uproar.wake
                                == pokemon_battle_engine::rules::UproarWake::OnHit)
                    {
                        Some("slp")
                    } else {
                        m.behavior.cures_target_status.map(status_id)
                    };
                    (any || only.is_some()).then(|| (m.name.clone(), only))
                })
                .collect(),
        }
    }
}

/// The Pokémon that entered the field during a boundary (`switch` / `drag`), each with the
/// number of actions `extract_decisions` had read before it: the entry precedes that action.
pub fn extract_entries(events: &[Value], reading: &LogReading) -> Vec<(usize, Who)> {
    events
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event["kind"].as_str(), Some("switch" | "drag")))
        .filter_map(|(position, event)| {
            let entrant = serde_json::from_value::<Who>(event["pokemon"].clone()).ok()?;
            Some((
                extract_decisions(&events[..position], reading).len(),
                entrant,
            ))
        })
        .collect()
}

pub fn extract_decisions(events: &[Value], reading: &LogReading) -> Vec<ActionOracle> {
    extract_decisions_with(events, reading, &|_, _| false, &|_, _, _| false)
}

/// `extract_decisions`, knowing whose item or ability gives its moves a flinch of their own
/// (King's Rock, Razor Fang, Stench): a silent flinch goes to that Pokémon's hit before a later
/// hit that cannot flinch.
pub fn extract_decisions_with(
    events: &[Value],
    reading: &LogReading,
    adds_flinch: &dyn Fn(&Who, Option<&str>) -> bool,
    // Gen 1: whether this Pokémon's stat (`atk` …) reads 999 (`true`: a raise) or 1 (`false`:
    // a drop) — where its change prints an echo the other way (R-M9-42).
    stat_stuck: &dyn Fn(&Who, &str, bool) -> bool,
) -> Vec<ActionOracle> {
    let status_after_action = reading.status_after_action;
    let who = |v: &Value| serde_json::from_value::<Who>(v.clone()).ok();
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    let mut out: Vec<ActionOracle> = Vec::new();
    // `-crit|<target>` precedes the hit it belongs to; it is attached when that target's damage
    // (HP or Substitute) is reported. A spread move reports every target's `-crit` before any
    // damage (Showdown computes all the damage first), so the crit is matched by target.
    let mut pending_crits: Vec<Who> = Vec::new();
    let take_crit =
        |pending: &mut Vec<Who>, target: &Who| match pending.iter().position(|p| p == target) {
            Some(index) => {
                pending.remove(index);
                true
            }
            None => false,
        };
    // A cure announced by Heal Bell / Shed Skin / Natural Cure is not the actor waking up: the
    // side it covers (Heal Bell cures its user's team only; a foe thawing right after is its
    // own thaw).
    let mut announced_cure: Option<usize> = None;
    // After the `residual` marker nothing is an action's outcome any more.
    let mut in_residual = false;
    // A Pokémon dragged in by the current action (Roar, Dragon Tail, Red Card) takes its entry
    // hazards inside that action.
    let mut dragging = false;
    // So does the one whose switch an intercepting Pursuit held up (§15.11): the switch goes on
    // inside the Pursuit's step — Gen 2-4 even after the Pursuit knocked the Pokémon out.
    let mut pursuit_held_switch = false;
    let mut pursued_entrant: Option<Who> = None;
    // The holder whose Synchronize just announced it passes a status back.
    let mut synchronizing: Option<Who> = None;
    // The Pokémon whose own ability just announced a stat change (`-ability|p|Justified|boost`):
    // the stage changes on it that follow are the ability's, not the move's secondary.
    let mut ability_stat_change: Option<Who> = None;
    // An item used up (`-enditem|p|X` with no `[from]`: Snowball, Weakness Policy …): the stat
    // changes that follow on its holder are the item's, no secondary's.
    let mut item_stat_change: Option<Who> = None;
    // The Pokémon an Instruct just told to use its move again (`-singleturn|p|move: Instruct`).
    let mut instructed_next: Option<Who> = None;
    let mut instructed_chances: Vec<PriorityChance> = Vec::new();
    // Cotton Down's `-ability|holder|Cotton Down` and the stat drops right after it, on any
    // Pokémon: the ability's, not the move's secondary (DESIGN.md R269).
    let mut cotton_down = false;
    // Powder blowing up the user's Fire move (`-activate|user|move: Powder`): the silent
    // `-damage` on the user that follows is the action's HP commit (DESIGN.md R275).
    let mut powder_blowup: Option<Who> = None;
    // The Pokémon that just ate a berry (`-enditem|p|Berry|[eat]`): the cure that follows is
    // the berry's, not a wake-up or a thaw of its own.
    let mut ate_berry: Option<Who> = None;
    // The Pokémon that just ate a confusion berry: its `-start|p|confusion` (after the heal) is
    // the berry's.
    let mut confusion_berry: Option<Who> = None;
    // The Pokémon that just ate a berry, and the berry: a stage change right after is the
    // berry's.
    let mut eaten_berry: Option<(Who, String)> = None;
    // The Pokémon a switch (not a drag) brought in since the last move: its switch-in is not
    // part of an action.
    let mut entrant: Option<Who> = None;
    // `-activate|target|move: Pursuit`: the next Pursuit strikes a Pokémon as it switches out.
    let mut pursuit_intercept = false;
    // A berry Fling threw and the Pokémon it struck, who eats it with no `-enditem` of its own:
    // the thrower's `-enditem|user|X Berry|[from] move: Fling` only follows at the `Update`, so
    // it is read ahead as the move starts.
    let mut flung_berry: Option<(Who, String)> = None;
    // The Pokémon whose *curing* ability just announced itself (`-activate|p|ability: Vital
    // Spirit`): a cure right after is the ability's (its `Update`), not a wake-up of its own. A
    // `-activate` of any other ability (Quick Draw's priority chance) says nothing about a cure
    // that follows (DESIGN.md R-M8-49).
    let mut ability_announced: Option<Who> = None;
    for (position, event) in events.iter().enumerate() {
        let kind = event["kind"].as_str().unwrap_or("");
        match kind {
            "switch" => {
                entrant = who(&event["pokemon"]);
                pursued_entrant = std::mem::take(&mut pursuit_held_switch)
                    .then(|| entrant.clone())
                    .flatten();
            }
            "move" | "cant" | "turn" => {
                entrant = None;
                pursuit_held_switch = false;
                pursued_entrant = None;
            }
            _ => {}
        }
        if kind == "move" {
            flung_berry = (text(&event["move"]) == "Fling")
                .then(|| flung(&events[position + 1..], &event["target"]))
                .flatten();
        }
        let after_ability = ability_announced.take();
        let after_synchronize = synchronizing.take();
        let ability_boosting = ability_stat_change.take();
        let item_boosting = item_stat_change.take();
        let cotton_down_drops = std::mem::take(&mut cotton_down);
        let berry_eater = ate_berry.take();
        let confusion_berry_eater = confusion_berry.take();
        let previous_berry = eaten_berry.take().or_else(|| {
            // The flung berry's own stage change follows the throw's damage.
            (kind == "boost")
                .then(|| flung_berry.take())
                .flatten()
                .filter(|(target, _)| who(&event["pokemon"]).as_ref() == Some(target))
        });
        eaten_berry = berry_eaten(event);
        if event["args"][2] == "[from] stealeat"
            && let Some((eater, _)) = &eaten_berry
            && let Some(action) = out.last_mut()
        {
            action.stolen_berry = Some(eater.clone());
        }
        if let Some(eater) = confusion_berry_eaten(event) {
            confusion_berry = Some(eater);
        } else if kind == "heal"
            && confusion_berry_eater.is_some()
            && who(&event["pokemon"]) == confusion_berry_eater
        {
            confusion_berry = confusion_berry_eater.clone();
        } else if kind == "heal"
            && event["from"].as_str().is_some_and(|from| {
                from.strip_prefix("[from] item: ")
                    .is_some_and(|berry| CONFUSION_BERRIES.contains(&berry))
            })
        {
            // A confusion berry Fling threw: no `-enditem` of the eater's own, its heal names it
            // (the throw hurt the target, so there is always a heal).
            confusion_berry = who(&event["pokemon"]);
        }
        if kind == "start"
            && !in_residual
            && event["args"][1] == "confusion"
            && let Some(eater) = confusion_berry_eater
                .filter(|eater| who_from_text(&event["args"][0]).as_ref() == Some(eater))
        {
            if entrant.as_ref() != Some(&eater)
                && let Some(action) = out.last_mut()
            {
                let hits = action.crits.len();
                action
                    .berry_draws
                    .push((eater.clone(), hits, BerryDraw::Confusion));
                action.applied.push((eater, "start:confusion".into()));
            }
            continue;
        }
        // A pinch berry's stat change right after it was eaten (`-boost|p|atk|1|[from] item:
        // Liechi Berry`) is not the move's; Starf Berry's names the stat it drew.
        if !in_residual
            && let Some((eater, draw)) = BerryDraw::berry_boost(previous_berry.as_ref(), event)
        {
            if let Some(draw) = draw
                && entrant.as_ref() != Some(&eater)
                && let Some(action) = out.last_mut()
            {
                let hits = action.crits.len();
                action.berry_draws.push((eater, hits, draw));
            }
            continue;
        }
        // An item's own stat change (`-boost|p|spa|1|[from] item: Absorb Bulb`, Weakness Policy,
        // a berry eaten after a hit) is not the move's either (DESIGN.md R198).
        if matches!(kind, "boost" | "unboost")
            && event["from"]
                .as_str()
                .is_some_and(|from| from.starts_with("[from] item: "))
        {
            continue;
        }
        if kind == "residual" {
            in_residual = true;
            continue;
        }
        if kind == "turn" {
            in_residual = false;
        }
        // (A foreseen attack's strike reports its own `-miss`, `-immune`, `-crit` there too:
        // `extract_future_strikes` reads them, DESIGN.md R277.)
        if in_residual
            && matches!(
                kind,
                "damage"
                    | "heal"
                    | "status"
                    | "curestatus"
                    | "boost"
                    | "unboost"
                    | "start"
                    | "miss"
                    | "immune"
                    | "crit"
            )
        {
            continue;
        }
        // A foreseen attack striking a Substitute in the residual is the strike's, not the
        // last action's (R-M8-84).
        if in_residual
            && matches!(kind, "activate" | "end")
            && event["args"][1]
                .as_str()
                .is_some_and(|what| what.ends_with("Substitute"))
        {
            continue;
        }
        // So is a hold on 1 HP against it (`ShownHolds`, R-M10-41): a Focus Band's, Sturdy's.
        if in_residual
            && ((kind == "activate" && event["args"][1] == "item: Focus Band")
                || (kind == "ability"
                    && event["args"][1] == "Sturdy"
                    && event["args"].get(2).is_none()))
        {
            continue;
        }
        match kind {
            "move" => {
                announced_cure = None;
                dragging = false;
                pending_crits.clear();
                // A called move (`[from] move: Metronome`, `[from] Magic Coat`, `[from] ability:
                // Magic Bounce`): Showdown prints it inside the calling action, and the user may
                // be another Pokémon than the actor's (a reflection, §42.8); the replay reads it
                // off that action.
                // A `[from]` marker the Shell runs inside the same action (Metronome and its
                // kind, `useMove`) attaches to the calling action; one that is its own action
                // (a combined Pledge's promotion, Dancer's `runMove`) starts a new one.
                // A locked move going on is the Pokémon's own action, not a move another one
                // called: Gen 1's `runMove` gives the move itself as its source (`|move|p|Skull
                // Bash|…|[from] Skull Bash`; no caller calls itself), Gen 2+ the lock
                // (`[from] lockedmove`: a two-turn move's attack, Outrage's later turns —
                // R-M8-63).
                let continues_itself = event["from"].as_str().is_some_and(|from| {
                    from == format!("[from] {}", text(&event["move"]))
                        || from == "[from] lockedmove"
                });
                // Shell Side Arm's category as the move starts (`[anim] Shell Side Arm
                // Physical`, its `onPrepareHit`): there even when a protection stops the move
                // before its `-hint` (R-M8-166; older recordings carry no `anim`).
                let shell_side_arm = event["anim"]
                    .as_str()
                    .and_then(|anim| anim.strip_prefix("Shell Side Arm "))
                    .map(|category| category == "Physical");
                if event["from"]
                    .as_str()
                    .is_some_and(|from| from.starts_with("[from]") && !acts_on_its_own(from))
                    && !continues_itself
                    && let Some(user) = who(&event["user"])
                    && let Some(action) = out.last_mut()
                    && action.move_name.is_some()
                {
                    let called = CalledMove::new(
                        user,
                        text(&event["move"]),
                        who(&event["target"]),
                        event["from"].as_str().map(str::to_string),
                        action,
                    );
                    action.called.push(called);
                    action.move_target = who(&event["target"]);
                    if shell_side_arm.is_some() {
                        action.physical_hint = shell_side_arm;
                    }
                    // A called move whose target is named without a position (Metronome's Poison
                    // Gas at a foe Healing Wish took down, `p2: Mareep|[notarget]`) draws
                    // nothing either (R-M8-86).
                    if action
                        .move_target
                        .as_ref()
                        .is_some_and(|t| t.slot.is_none())
                    {
                        action.failed = true;
                        action.no_target = true;
                        action.hit = false;
                    }
                    continue;
                }
                if let Some(user) = who(&event["user"]) {
                    sleeps_through(&mut out, &user);
                    let instructed = instructed_next.take().as_ref() == Some(&user);
                    let chances = std::mem::take(&mut instructed_chances);
                    let action = current(&mut out, user);
                    action.instructed |= instructed;
                    if instructed {
                        action.instructed_chances = chances;
                    }
                    // A Z status move is logged as `Z-<move>` with the move as its `[anim]`
                    // (Showdown `useMoveInner`): it is that move, Z-powered — Z-Rest heals as Rest
                    // does (DESIGN.md R-M9-30).
                    let logged = text(&event["move"]);
                    action.move_name = Some(
                        event["anim"]
                            .as_str()
                            .filter(|anim| logged.strip_prefix("Z-") == Some(*anim))
                            .map_or(logged.clone(), str::to_owned),
                    );
                    action.intercepted =
                        std::mem::take(&mut pursuit_intercept) && text(&event["move"]) == "Pursuit";
                    pursuit_held_switch = action.intercepted;
                    action.move_target = who(&event["target"]);
                    action.own_target = who(&event["target"]);
                    if logged == "Fling" {
                        action.flung_berry = flung_berry.as_ref().map(|(target, _)| target.clone());
                    }
                    if shell_side_arm.is_some() {
                        action.physical_hint = shell_side_arm;
                    }
                    // A target named without a position (`p1: Maushold|[notarget]`): the one
                    // it was aimed at is gone and nobody took its place; the move draws nothing.
                    // A foreseen attack goes on at the fallen foe (`getMoveTargets` keeps it for
                    // a `futuremove`, R277): Gen 4 draws its damage then (R-M8-89).
                    if action
                        .move_target
                        .as_ref()
                        .is_some_and(|t| t.slot.is_none())
                        && !matches!(
                            action.move_name.as_deref(),
                            Some("Future Sight" | "Doom Desire")
                        )
                    {
                        action.failed = true;
                        action.no_target = true;
                        action.hit = false;
                    }
                }
            }
            "cant" => {
                let detail = text(&event["detail"]);
                // Damp stopping the move in progress (`cant|holder|ability: Damp|move|[of] user`):
                // the user's action, not the holder's, is the one that does not go ahead.
                // Dazzling, Queenly Majesty, Armor Tail stop a foe's priority move the same way.
                let stopper = TRY_MOVE_STOPPERS
                    .into_iter()
                    .find(|ability| detail.starts_with(ability));
                if let Some(stopper) = stopper
                    && let Some(action) = out.last_mut()
                    && action.move_name.is_some()
                {
                    action.cant = Some(stopper.to_string());
                    action.hit = false;
                } else if let Some(user) = who(&event["pokemon"])
                    && let Some(action) = out.last_mut()
                    && action.user == user
                    && action.move_name.as_deref() == detail.split('|').next()
                {
                    // The move stopping itself once it started (Gen 3-4 Focus Punch's `onTry`,
                    // Shell Trap's `onTryMove`: `move|user|Shell Trap` then `cant|user|Shell
                    // Trap|Shell Trap`): that very action does not go ahead.
                    action.cant = detail.split('|').next().map(str::to_string);
                    action.hit = false;
                } else if let Some(user) = who(&event["pokemon"]) {
                    let reason = detail.split('|').next().unwrap_or("").to_string();
                    if reason != "slp" {
                        sleeps_through(&mut out, &user);
                    }
                    // Pollen Puff stopped by Heal Block after it was announced (its user's, at
                    // TryMove, or its ally target's, as the heal fails): the move's own failure.
                    if reason == "move: Heal Block"
                        && let Some(last) = out.last_mut()
                        && last.user == user
                        && last.move_name.as_deref() == detail.split('|').nth(1)
                    {
                        last.failed = true;
                        continue;
                    }
                    // A move a caller called, refused as it is modified (Heal Block, Gravity:
                    // `onModifyMove`, before its own `move` line): the caller's action goes on
                    // to its end, the move it called having failed (R-M8-70).
                    // Gen 3 Sleep Talk drawing a move with no PP left (`cant|user|nopp|sketch`,
                    // issues.md I116): the move it called, which does not go ahead (R-M8-72).
                    // Another move's (an instructed one right before) is not: the `cant` is
                    // the user's own action (R-M8-84).
                    if reason == "nopp"
                        && let Some(last) = out.last_mut()
                        && last.user == user
                        && last.move_name.as_deref() == Some("Sleep Talk")
                        && last.called.is_empty()
                        && let Some(called) = detail.split('|').nth(1)
                    {
                        let called = CalledMove::new(user, called.to_string(), None, None, last);
                        last.called.push(called);
                        last.cant = Some(reason);
                        last.hit = false;
                        continue;
                    }
                    // Throat Chop's refusal names no move (`cant|user|move: Throat Chop`): after
                    // the user's caller, the move it called was a sound move (R-M8-148).
                    if reason == "move: Throat Chop"
                        && detail.split('|').nth(1).is_none()
                        && let Some(last) = out.last_mut()
                        && last.user == user
                        && last.move_name.is_some()
                        && last.called.is_empty()
                    {
                        let called =
                            CalledMove::new(user, UNNAMED_SOUND_MOVE.to_string(), None, None, last);
                        last.called.push(called);
                        last.failed = true;
                        last.hit = false;
                        continue;
                    }
                    if matches!(reason.as_str(), "move: Heal Block" | "move: Gravity")
                        && let Some(last) = out.last_mut()
                        && last.user == user
                        && last.move_name.is_some()
                        && let Some(called) = detail.split('|').nth(1)
                    {
                        let called = CalledMove::new(user, called.to_string(), None, None, last);
                        last.called.push(called);
                        last.failed = true;
                        last.hit = false;
                        continue;
                    }
                    let action = current(&mut out, user);
                    action.cant = Some(reason);
                    action.hit = false;
                }
            }
            "curestatus" if berry_eater.is_some() && berry_eater == who(&event["pokemon"]) => {}
            "enditem" if berry_eaten(event).is_some() => {
                ate_berry = berry_eaten(event).map(|(eater, _)| eater);
            }
            // A Focus Sash that held (no `[from]`: one taken by Knock Off or a thief says so).
            "enditem" if event["args"].get(2).is_none() && event["args"][1] != "Focus Sash" => {
                item_stat_change = event["args"].get(0).and_then(who_from_text);
            }
            "enditem" if event["args"][1] == "Focus Sash" && event["args"].get(2).is_none() => {
                if let (Some(action), Some(holder)) =
                    (out.last_mut(), event["args"].get(0).and_then(who_from_text))
                {
                    action.focus_sash_held.push(holder);
                }
            }
            "curestatus" => {
                let status = text(&event["status"]);
                let from = event["from"].as_str().unwrap_or("");
                // Gen 3-4 (issues.md I63): an uproar wakes the sleeper as it is about to move —
                // no wake-up of its own (`sleep_duration_hint`).
                if from == "[from] move: Uproar"
                    && let Some(woken) = who(&event["pokemon"])
                {
                    current(&mut out, woken.clone())
                        .applied
                        .push((woken, format!("uproar:{status}")));
                    continue;
                }
                if status == "frz"
                    && let Some(cured) = who(&event["pokemon"])
                {
                    // Thawed by the hit of the move in progress (a Fire move, Scald …): the
                    // frame checks it, no action of its own. A move that is Fire only as it is
                    // used (Hidden Power, Weather Ball …) is not in `thawing_moves`: a thaw that
                    // does not start the cured Pokémon's own action (`own_action_follows`) is that
                    // hit's too (R-M9-52). Where its own action does follow, the thaw
                    // roll planned there asks the engine whether it is still frozen.
                    if let Some(action) = out.last_mut()
                        && action.user != cured
                        && action.damages.iter().any(|(who, _)| *who == cured)
                        && (action
                            .move_name
                            .as_ref()
                            .is_some_and(|m| reading.thawing_moves.contains(m))
                            || (from == "[msg]"
                                && berry_eater.as_ref() != Some(&cured)
                                && !own_action_follows(&events[position + 1..], &cured)))
                    {
                        action.thawed.push(cured);
                        continue;
                    }
                    // Thawed by the berry a Fling just threw at it (Aspear Berry: the flung
                    // berry's effect, `[msg]` like a thaw roll, before the thrower's
                    // `-enditem`): the frame checks it, no action of its own (R-M8-139). After
                    // that `-enditem`, or with no berry flung, it is the target's own thaw roll as
                    // its action starts (a Gen 9 Dunsparce after a Miracle Seed, R-M10-67).
                    if let Some(action) = out.last_mut()
                        && action.user != cured
                        && action.move_name.as_deref() == Some("Fling")
                        && action.damages.iter().any(|(who, _)| *who == cured)
                        && flung(&events[position + 1..], &event["pokemon"]).is_some()
                    {
                        continue;
                    }
                    // Gen 2: thawed by its own `defrost` move once it landed.
                    if out
                        .last()
                        .is_some_and(|action| action.user == cured && action.move_name.is_some())
                    {
                        continue;
                    }
                    // Gen 3+: its `defrost` move thaws it as the move starts (`[from] move:`),
                    // which is not the thaw roll.
                    if from.starts_with("[from] move:") {
                        current(&mut out, cured).defrosted = true;
                        continue;
                    }
                }
                // A wake-up or thaw of its own says `[msg]`; a `[silent]` cure is an effect's (Gen 1
                // Haze curing the foe, R-M8-93).
                let by_effect = announced_cure
                    .is_some_and(|side| who(&event["pokemon"]).is_some_and(|w| w.side == side))
                    || from == "[silent]"
                    || from.contains("ability:")
                    || from.contains("item:")
                    || (after_ability.is_some() && after_ability == who(&event["pokemon"]));
                // Cured by the move in progress (Purify on it; a Worry Seed Metronome called): the
                // sleep or freeze ended without a wake-up or thaw of its own. Not a status the
                // move does not cure: a freeze the Worry Seed target thaws from at its own
                // BeforeMove is its thaw (R-M8-103).
                // Nor a foe's cure within a move on the user's own side (a frozen foe thawing at
                // its BeforeMove after the user's Jungle Healing, R-M8-113).
                if let Some(cured) = who(&event["pokemon"])
                    && let Some(action) = out.last_mut()
                    && action.user != cured
                    && action
                        .used_move()
                        .and_then(|m| reading.curing_moves.get(m))
                        .is_some_and(|only| only.is_none_or(|only| only == status))
                    && !(cured.side != action.user.side
                        && action
                            .used_move()
                            .is_some_and(|m| reading.own_side_curing_moves.contains(m)))
                {
                    action.applied.push((cured, format!("curestatus:{status}")));
                    continue;
                }
                // Psycho Shift cures its user once the status passed on (`source.cureStatus()`,
                // `[msg]` like a wake-up): a sleeper's, through Sleep Talk, is the move's, not a
                // wake-up of its own (R-M8-141).
                if let Some(cured) = who(&event["pokemon"])
                    && out.last().is_some_and(|action| {
                        action.user == cured
                            && action.used_move().map(String::as_str) == Some("Psycho Shift")
                    })
                {
                    continue;
                }
                if matches!(status.as_str(), "slp" | "frz")
                    && !by_effect
                    && let Some(user) = who(&event["pokemon"])
                {
                    current(&mut out, user).cured = Some(status);
                }
            }
            "activate" | "end"
                if {
                    let args = event["args"].as_array().cloned().unwrap_or_default();
                    args.get(1)
                        .and_then(Value::as_str)
                        .is_some_and(|w| w.ends_with("Substitute"))
                } =>
            {
                // A hit absorbed by (or breaking) a Substitute is a landed hit without an HP
                // commit; it still consumed a critical-hit roll.
                if let Some(action) = out.last_mut() {
                    let holder = who_from_text(&event["args"][0]);
                    action.crits.push(
                        holder
                            .as_ref()
                            .is_some_and(|h| take_crit(&mut pending_crits, h)),
                    );
                    action.substitute_hits.push(true);
                    action.hit_targets.push(holder.clone());
                    action.hit_hp.push(None);
                    if let Some(holder) = holder {
                        action
                            .substitute_struck
                            .push((holder, event["kind"] == "end"));
                    }
                }
            }
            // Bide's release (`-end|user|move: Bide`, Gen 1 `-end|user|Bide`), not its silent
            // removal after: the holder's action, whose hit the damage that follows is.
            "end"
                if {
                    let args = event["args"].as_array().cloned().unwrap_or_default();
                    matches!(
                        args.get(1).and_then(Value::as_str),
                        Some("move: Bide" | "Bide")
                    ) && args.get(2).and_then(Value::as_str) != Some("[silent]")
                } =>
            {
                if let Some(user) = who_from_text(&event["args"][0]) {
                    current(&mut out, user).move_name = Some("Bide".into());
                }
            }
            "activate" if event["args"].get(1).and_then(Value::as_str) == Some("move: Powder") => {
                if let Some(user) = who_from_text(&event["args"][0])
                    && let Some(action) = out.last_mut()
                    && action.user == user
                {
                    action.cant = Some("move: Powder".into());
                    action.hit = false;
                    powder_blowup = Some(user);
                }
            }
            // A Pledge that waits does nothing at all (`-waiting`): no roll, no hit.
            "waiting" => {
                if let Some(action) = out.last_mut() {
                    action.failed = true;
                    action.no_target = true;
                    action.hit = false;
                }
            }
            "singleturn" => {
                if event["move"] == "move: Instruct" {
                    instructed_next = who(&event["pokemon"]);
                } else if let (Some(action), Some(user)) = (out.last_mut(), who(&event["pokemon"]))
                    && action.user == user
                {
                    action.protected = true;
                }
            }
            "activate" => {
                let args = event["args"].as_array().cloned().unwrap_or_default();
                let what = args.get(1).and_then(Value::as_str).unwrap_or("");
                // A priority chance of the action Instruct is building for its target.
                if instructed_next.is_some()
                    && args.first().and_then(who_from_text) == instructed_next
                    && let Some(chance) = announced_chance(what)
                {
                    instructed_chances.push(chance);
                }
                if what == "ability: Forewarn"
                    && let Some(action) = out.last_mut()
                    && (action.ability_traded
                        || matches!(
                            action
                                .called
                                .last()
                                .map(|called| called.name.as_str())
                                .or(action.move_name.as_deref()),
                            Some(
                                "Skill Swap" | "Role Play" | "Entrainment" | "Doodle" | "Transform"
                            )
                        ))
                {
                    action.forewarned.push(
                        args.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect(),
                    );
                }
                if let Some(action) = out.last_mut()
                    && args.first().and_then(who_from_text).as_ref() == Some(&action.user)
                {
                    match what {
                        "move: Magnitude" => {
                            action.magnitude = args
                                .get(2)
                                .and_then(Value::as_str)
                                .and_then(|n| n.parse().ok());
                        }
                        "move: Fickle Beam" => action.power_chance = true,
                        _ => {}
                    }
                }
                if what == "ability: Synchronize" {
                    synchronizing = args.first().and_then(who_from_text);
                }
                // Lightning Rod / Storm Drain drew the move whose `move` line came last
                // (`onAnyRedirectTarget` announces only a target it changed): the line names
                // the holder, and the move was aimed at another first (R-M11-7).
                if matches!(what, "ability: Lightning Rod" | "ability: Storm Drain")
                    && let Some(action) = out.last_mut()
                    && let Some(holder) = args.first().and_then(who_from_text)
                {
                    match action.called.last_mut() {
                        Some(called) => called.redirected_by = Some(holder),
                        None => action.redirected_by = Some(holder),
                    }
                }
                // A contact reaction taking the user's ability away for the rest of its move.
                if matches!(
                    what,
                    "ability: Mummy" | "ability: Lingering Aroma" | "ability: Wandering Spirit"
                ) && let Some(action) = out.last_mut()
                    && args.get(2).and_then(who_from_text).as_ref() == Some(&action.user)
                    && action.ability_taken_after.is_none()
                {
                    action.ability_taken_after = Some(action.crits.len());
                }
                // Wandering Spirit trades through Showdown's `skillSwap`, which names the attacker
                // first: `-activate|user|Skill Swap|…|[of] holder` — not the Skill Swap move's
                // own trade (R-M8-133).
                if what == "Skill Swap"
                    && let Some(action) = out.last_mut()
                    && action.move_name.as_deref() != Some("Skill Swap")
                    && args.first().and_then(who_from_text).as_ref() == Some(&action.user)
                    && action.ability_taken_after.is_none()
                {
                    action.ability_taken_after = Some(action.crits.len());
                    action.ability_traded = true;
                }
                // A Dancer about to dance along: its dance (or the BeforeMove that stops it) is
                // an action of its own.
                if what == "ability: Dancer"
                    && let Some(dancer) = args.first().and_then(who_from_text)
                {
                    let mut dance = ActionOracle::new(dancer);
                    dance.danced = true;
                    out.push(dance);
                }
                if let Some(name) = what.strip_prefix("ability: ") {
                    ability_announced = reading
                        .curing_abilities
                        .contains(name)
                        .then(|| args.first().and_then(who_from_text))
                        .flatten();
                }
                if what == "move: Spite"
                    && let Some(action) = out.last_mut()
                    && action.used_move().is_some_and(|name| name == "Spite")
                {
                    action.spite_drop = args
                        .get(3)
                        .and_then(Value::as_str)
                        .and_then(|n| n.parse().ok());
                }
                if what == "item: Focus Band"
                    && let (Some(action), Some(holder)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                {
                    let hits = action.crits.len();
                    action.focus_band_held.push((holder, hits));
                }
                if what == "move: Pursuit" {
                    pursuit_intercept = true;
                }
                // Bide storing energy (`-activate|user|move: Bide`, Gen 1 `-activate|user|Bide`):
                // the holder's action, with no move line.
                if matches!(what, "move: Bide" | "Bide")
                    && args.len() == 2
                    && let Some(user) = args.first().and_then(who_from_text)
                {
                    current(&mut out, user).move_name = Some("Bide".into());
                }
                if what == "confusion"
                    && let Some(user) = args.first().and_then(who_from_text)
                {
                    sleeps_through(&mut out, &user);
                    current(&mut out, user);
                }
                // `-activate|target|move: Bind|[of] source`: the target got bound by this hit.
                // (`-activate|p|move: Attract|[of] …` is an infatuated Pokémon's BeforeMove.)
                if what.starts_with("move: ")
                    && what != "move: Attract"
                    && args
                        .get(2)
                        .and_then(Value::as_str)
                        .is_some_and(|of| of.starts_with("[of]"))
                    && let (Some(action), Some(target)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                {
                    if action.damages.iter().any(|(t, _)| *t == target) {
                        action.secondary_applied = true;
                        action.secondary_on.push((
                            target.clone(),
                            "start:partiallytrapped".to_string(),
                            action.crits.len(),
                        ));
                    }
                    action
                        .applied
                        .push((target, "start:partiallytrapped".to_string()));
                }
                if matches!(
                    what,
                    "move: Heal Bell" | "move: Aromatherapy" | "ability: Shed Skin"
                ) {
                    announced_cure = args.first().and_then(who_from_text).map(|w| w.side);
                }
                // Protect / Detect / Telepathy stopped the move at TryHit on this target:
                // like an immunity, no accuracy, critical or damage decision is drawn for it.
                // (Gen 2-4 report the guard without the `move: ` prefix.)
                if matches!(
                    what.strip_prefix("move: ").unwrap_or(what),
                    "Protect"
                        | "Detect"
                        // Max Guard (DESIGN.md §44.5): Dynamax's protection, `-activate|p|move:
                        // Max Guard`.
                        | "Max Guard"
                        | "Wide Guard"
                        | "Quick Guard"
                        | "Crafty Shield"
                        | "Mat Block"
                        | "Psychic Terrain"
                        | "ability: Telepathy"
                        | "item: Safety Goggles"
                ) && let (Some(action), Some(target)) =
                    (out.last_mut(), args.first().and_then(who_from_text))
                    && target != action.user
                {
                    action.guarded.push(target.clone());
                    action.try_hit_blocked.push(target.clone());
                    action.immune.push(target);
                }
            }
            "crit" => {
                if let Some(target) = who(&event["pokemon"]) {
                    pending_crits.push(target);
                }
            }
            // Lightning Rod / Storm Drain / Motor Drive / Well-Baked Body / Sap Sipper / Wind Rider
            // took the move instead of being hit (`-ability|target|Lightning Rod|boost`): like an
            // immunity, the target draws no decision.
            "ability" => {
                let args = event["args"].as_array().cloned().unwrap_or_default();
                let ability = args.get(1).and_then(Value::as_str).unwrap_or("");
                // `-ability|p|New|Old|[from] …` changes an ability (Role Play, Trace …): it is
                // no absorption, and no Mirror Armor sending a drop back.
                let changes_ability = args
                    .iter()
                    .any(|arg| arg.as_str().is_some_and(|arg| arg.starts_with("[from]")));
                if !changes_ability
                    && matches!(
                        ability,
                        "Lightning Rod"
                            | "Storm Drain"
                            | "Motor Drive"
                            | "Well-Baked Body"
                            | "Sap Sipper"
                            | "Wind Rider"
                    )
                    && let (Some(action), Some(target)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                    && target != action.user
                    && action.move_name.is_some()
                {
                    action.hit = false;
                    action.try_hit_blocked.push(target.clone());
                    action.immune.push(target);
                }
                if ability == "Cotton Down" {
                    cotton_down = true;
                }
                if ability == "Sturdy"
                    && args.len() == 2
                    && let (Some(action), Some(holder)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                {
                    action.sturdy_held.push(holder);
                }
                if args.get(2).and_then(Value::as_str) == Some("boost")
                    && !matches!(
                        ability,
                        "Lightning Rod"
                            | "Storm Drain"
                            | "Motor Drive"
                            | "Well-Baked Body"
                            | "Sap Sipper"
                            | "Wind Rider"
                    )
                {
                    ability_stat_change = args.first().and_then(who_from_text);
                }
                // Mirror Armor sent a stat drop back (`-ability|target|Mirror Armor`): on a
                // target the move damaged, that drop was the move's secondary landing, though
                // the protocol then shows it on the user or, when the user refuses it too, not
                // at all.
                if ability == "Mirror Armor"
                    && !changes_ability
                    && let (Some(action), Some(target)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                    && target != action.user
                    && action.damages.iter().any(|(t, _)| *t == target)
                {
                    action.secondary_applied = true;
                    let hits = action.crits.len();
                    action
                        .secondary_on
                        .push((target, "reflected".to_string(), hits));
                }
            }
            // `-fail|target|heal`: a move's heal found its target at full HP (Present's heal;
            // Gen 2's `moveHit` prints a bare `-fail|target`, below).
            "fail" if event["detail"].as_str() == Some("heal") => {
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"])) {
                    action.applied.push((target, "fail:heal".to_string()));
                }
            }
            "fail" => {
                // `-fail|user|tox`: a status bounced back on the user (Synchronize) found it
                // already statused — the user's move did not fail. Showdown `setStatus` prints
                // that when the status is the same; a different one it refuses with a bare
                // `-fail|user` (and `[still]` on the user's move line) right after the user's
                // own Synchronize announced itself. So is `-fail|user|slp|[from] Uproar`: the
                // sleep a reaction to its hit (Effect Spore) would have put on it, refused by an
                // uproar (§15.12).
                let status_refused = matches!(
                    event["detail"]
                        .as_str()
                        .and_then(|detail| detail.split('|').next()),
                    Some("psn" | "tox" | "brn" | "par" | "slp" | "frz")
                );
                // `-fail|user|unboost|[from] ability: Clear Body`: a stat drop on the user was
                // refused (the Intimidate of a Pokémon its Dragon Tail dragged in), not its move.
                let stat_change_refused = event["detail"].as_str().is_some_and(|detail| {
                    detail.starts_with("unboost") || detail.starts_with("boost")
                });
                // A bare `-fail|target` on someone else: the move's effect failed on that target
                // (Gen 2's `moveHit`: Present's heal at full HP).
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"]))
                    && action.user != target
                    && action.move_name.is_some()
                    && event["detail"].is_null()
                {
                    action.applied.push((target, "fail".to_string()));
                }
                if let (Some(action), Some(user)) = (out.last_mut(), who(&event["pokemon"]))
                    && action.user == user
                    && action.move_name.is_some()
                    && !status_refused
                    && !stat_change_refused
                    && after_synchronize.as_ref() != Some(&user)
                {
                    action.failed = true;
                    if event["detail"]
                        .as_str()
                        .is_some_and(|detail| detail.starts_with("move: "))
                    {
                        action.fail_named_the_move = true;
                    }
                }
            }
            "disableslot" => {
                if let Some(action) = out.last_mut() {
                    action.disabled_slot = event["slot"].as_u64().map(|slot| slot as usize);
                }
            }
            // No Pokémon left to aim at (`-notarget`: the target fainted earlier in the turn and
            // nobody replaced it): the move draws nothing, like a failure.
            "notarget" => {
                if let (Some(action), Some(user)) = (out.last_mut(), who(&event["pokemon"]))
                    && action.user == user
                {
                    action.failed = true;
                    action.no_target = true;
                    action.hit = false;
                }
            }
            // A format clause refusing the status the action rolled (Freeze Clause Mod, Sleep
            // Clause Mod, M9.5): the roll landed, and the clause stopped it.
            "message" => {
                let status = match event["text"].as_str().unwrap_or("") {
                    "Freeze Clause activated." => Some("frz"),
                    "Sleep Clause Mod activated." => Some("slp"),
                    _ => None,
                };
                if let (Some(status), Some(action)) = (status, out.last_mut())
                    && let Some(target) = action
                        .damages
                        .last()
                        .map(|(target, _)| target.clone())
                        .or_else(|| action.move_target.clone())
                {
                    action.secondary_applied = true;
                    let hits = action.crits.len();
                    action
                        .secondary_on
                        .push((target, format!("status:{status}"), hits));
                }
            }
            // Gen 1-3 print no `-fail` when a move's `onTry` fails; Fake Out leaves its hint.
            // Gen 1: a confusion self-hit behind a Substitute that hurts nothing on the user's
            // side is only a hint too. The same hint follows a crash behind a Substitute (Hi
            // Jump Kick), which comes after the move line; a self-hit comes before any.
            "hint" => {
                let text = event["text"].as_str().unwrap_or("");
                if let Some(action) = out.last_mut() {
                    if action.move_name.is_some()
                        && text.contains("only works on your first turn out")
                    {
                        action.failed = true;
                    }
                    if action.move_name.is_some() && text.ends_with(" Shell Side Arm") {
                        action.physical_hint = Some(text.starts_with("Physical"));
                    }
                    if action.move_name.is_none() && text.contains("hurts itself due to confusion")
                    {
                        action.cant = Some("confusion".into());
                        action.hit = false;
                    }
                }
            }
            // A drag after the residual (a Red Card a foreseen attack set off) is the residual's
            // (`residual_drag_tape`), not the last action's (R-M8-141).
            "drag" if in_residual => {}
            "drag" => {
                if let (Some(action), Some(entrant)) = (out.last_mut(), who(&event["pokemon"])) {
                    action.dragged_in.push(entrant);
                    dragging = true;
                }
            }
            "hitcount" => {
                if let Some(action) = out.last_mut() {
                    action.hit_count = event["count"].as_u64().map(|n| n as u16);
                }
            }
            "miss" => {
                if let Some(action) = out.last_mut() {
                    action.hit = false;
                    if let Some(target) = who_from_text(&event["detail"]) {
                        action.missed.push(target);
                    }
                }
            }
            "immune" => {
                if let Some(action) = out.last_mut() {
                    let immune = who(&event["pokemon"]);
                    // The user shrugging off a reaction to its own move (Synchronize passing a
                    // paralysis back to an Electric type) is not the move being blocked.
                    let reaction_on_user = immune.as_ref().is_some_and(|who| {
                        *who == action.user && action.move_target.as_ref() != Some(who)
                    });
                    // Nor is one on a target the move already hit (Synchronize passing the
                    // attacker's Static paralysis on to an Electric target).
                    let already_hit = immune
                        .as_ref()
                        .is_some_and(|who| action.damages.iter().any(|(hit, _)| hit == who));
                    // Own Tempo's `onHit` (`-immune|p|confusion|[from] ability: Own Tempo`, the
                    // one `-immune` naming an effect) speaks once the move landed: Swagger's
                    // boost stands, and its accuracy was drawn (DESIGN.md R-M9-29). So did a
                    // move that dragged a Pokémon in: an `-immune` after the drag is the
                    // entrant's entry (its Intimidate meeting a Substitute), the move's last
                    // effect long done (R-M10-9).
                    let landed = dragging
                        || event["detail"]
                            .as_str()
                            .is_some_and(|detail| detail.starts_with("confusion|"));
                    if !reaction_on_user && !already_hit && !landed {
                        action.hit = false;
                        if let Some(target) = immune {
                            // An absorbing ability's refusal (`[from] ability: Volt Absorb` at
                            // full HP) is its TryHit; Levitate's is the natural immunity.
                            if event["detail"].as_str().is_some_and(|d| {
                                d.contains("[from] ability:") && !d.contains("Levitate")
                            }) {
                                action.try_hit_blocked.push(target.clone());
                            }
                            action.immune.push(target);
                        }
                    }
                }
            }
            "damage"
                if powder_blowup.is_some()
                    && powder_blowup.as_ref() == who(&event["pokemon"]).as_ref() =>
            {
                let hp = event["hp"].as_u64().unwrap_or(0) as u16;
                if let (Some(action), Some(target)) = (out.last_mut(), powder_blowup.take()) {
                    action.damages.push((target, hp));
                }
            }
            "damage" => {
                let hp = event["hp"].as_u64().unwrap_or(0) as u16;
                let from = event["from"].as_str();
                // Gen 1-2: a Pokémon whose turn shows nothing (Gen 1: held by a trapping move that
                // goes on) still takes its after-own-action residual (Leech Seed, poison, burn) as
                // its turn ends: that is its own silent action, not the one before (R-M8-93);
                // first in the turn too, ahead of a tied foe's move (R-M8-108) — not right after
                // an entry (a Baton Pass's: the passer's own residual on its replacement).
                // Gen 2's own switch-in residual (`onAfterSwitchInSelf` of poison and burn) on the
                // Pokémon a Pursuit held back as it came in: the pursuer's action, as its
                // hazards are (R-M8-168).
                let switch_in_residual = status_after_action
                    && from.is_some_and(|f| f == "[from] psn" || f == "[from] brn");
                if status_after_action
                    && let Some(target) = who(&event["pokemon"])
                    && pursued_entrant.as_ref() != Some(&target)
                    && out
                        .last()
                        .map_or(entrant.is_none(), |action| action.user != target)
                    && !out.iter().any(|action| action.user == target)
                    && from.is_some_and(|f| {
                        f == "[from] psn" || f == "[from] brn" || f.contains("Leech Seed")
                    })
                {
                    current(&mut out, target.clone()).damages.push((target, hp));
                    continue;
                }
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"])) {
                    let hazard = from
                        .is_some_and(|f| ["Stealth Rock", "Spikes"].iter().any(|h| f.contains(h)));
                    let pursued = pursued_entrant.as_ref() == Some(&target);
                    if ((dragging || pursued) && hazard) || (pursued && switch_in_residual) {
                        action.damages.push((target, hp));
                    } else if from.is_some_and(|f| f.contains("confusion")) {
                        action.damages.push((target, hp));
                        action.cant = Some("confusion".into());
                        action.hit = false;
                    } else if target == action.user
                        && action
                            .used_move()
                            .is_some_and(|name| reading.crashing_moves.contains(name))
                        && from.is_none_or(|f| {
                            reading.crashing_moves.iter().any(|name| {
                                f == format!("[from] {}", pokemon_battle_engine::data::to_id(name))
                            })
                        })
                    {
                        // The move's crash (Axe Kick's is `[from] highjumpkick`).
                        action.damages.push((target, hp));
                        action.crashed = true;
                    } else if from.is_none()
                        && action.called.last().is_some_and(|called| {
                            called.user == target && called.user != action.user
                        })
                    {
                        // What a move another Pokémon took over costs its new user (a snatched
                        // Belly Drum or Substitute: `directDamage`, no `[from]`): its own HP cost,
                        // as the actor's own would be — no hit (R-M8-79).
                    } else if from.is_some_and(|f| f.starts_with("[from] pokemon: ")) {
                        // A busted Disguise's price (`this.damage`, `[from] pokemon:
                        // Mimikyu-Busted`) right after the hit that broke it: an HP commit of
                        // the action, no hit (DESIGN.md R-M8-91).
                        action.damages.push((target, hp));
                    } else if from.is_none() && target != action.user {
                        action.damages.push((target.clone(), hp));
                        action.crits.push(take_crit(&mut pending_crits, &target));
                        action.substitute_hits.push(false);
                        action.hit_targets.push(Some(target.clone()));
                        action.hit_hp.push(Some(hp));
                    } else if target != action.user
                        && action.used_move().is_some_and(|name| {
                            from == Some(
                                format!("[from] {}", pokemon_battle_engine::data::to_id(name))
                                    .as_str(),
                            )
                        })
                    {
                        // The move's own damage on someone it did not hit (Flame Burst's
                        // splash on the target's partner: `[from] flameburst|[of] user`).
                        action.damages.push((target, hp));
                    } else if target == action.user
                        // (Struggle's recoil is `[from] recoil`, lowercase.)
                        && (from.is_some_and(|f| f.eq_ignore_ascii_case("[from] recoil"))
                            || from.is_some_and(|f| f.contains("item: Life Orb"))
                            // The move's own after-move self damage (Mind Blown, Steel Beam:
                            // `[from] mindblown`), a called one's too.
                            || action.used_move().is_some_and(|name| {
                                from == Some(
                                    format!("[from] {}", pokemon_battle_engine::data::to_id(name))
                                        .as_str(),
                                )
                            })
                            // Gen 2: poison / burn, Leech Seed, Nightmare and Curse right after
                            // the user's own action (`onAfterMoveSelf`).
                            || (status_after_action
                                && from.is_some_and(|f| {
                                    f == "[from] psn"
                                        || f == "[from] brn"
                                        || f.contains("Leech Seed")
                                        || f == "[from] Nightmare"
                                        || f == "[from] Curse"
                                }))
                            || (event["of"].as_str().is_some()
                                && from.is_some_and(|f| {
                                    f.contains("ability:")
                                        || f.contains("item:")
                                        // A protection's contact punishment.
                                        || f == "[from] Spiky Shield"
                                })))
                    {
                        // Recoil, Life Orb, the move's own self damage, or a reaction the target
                        // dealt back (`[from]
                        // ability: Rough Skin|[of] …`): the action's own HP commits on its
                        // user. Residual drains come after the `residual` marker and are
                        // skipped above.
                        action.damages.push((target, hp));
                    }
                }
            }
            "boost" | "unboost"
                if ability_boosting.is_some() && ability_boosting == who(&event["pokemon"]) =>
            {
                ability_stat_change = ability_boosting;
            }
            "unboost" if cotton_down_drops => cotton_down = true,
            // Gen 1's raise of a stat at 999 (a drop of one at 1) prints the change and then its
            // echo the other way by 1 (the gen1 `boost`): the echo is the same change's, no effect
            // of its own — not an Aurora Beam drop after a Rage boost (R-M9-42).
            "boost" | "unboost"
                if position > 0 && {
                    let previous = &events[position - 1];
                    let raised = previous["kind"] == "boost";
                    previous["kind"] != event["kind"]
                        && matches!(previous["kind"].as_str(), Some("boost" | "unboost"))
                        && previous["pokemon"] == event["pokemon"]
                        && previous["stat"] == event["stat"]
                        && event["amount"].as_u64() == Some(1)
                        && who(&event["pokemon"]).is_some_and(|target| {
                            stat_stuck(&target, event["stat"].as_str().unwrap_or(""), raised)
                        })
                } => {}
            "status" | "boost" | "unboost" | "heal" => {
                let kind = event["kind"].as_str().unwrap_or("");
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"])) {
                    // The target's ability statused the actor on contact (Static …). Gen 3-4
                    // print Effect Spore's sleep bare (their `slp` names no ability): a sleep on
                    // the actor after one of its own hits is that.
                    let bare_sleep_after_a_hit = event["from"].is_null()
                        && event["status"].as_str() == Some("slp")
                        && action.move_name.is_some()
                        && !action.crits.is_empty();
                    if kind == "status"
                        && target == action.user
                        && (event["from"]
                            .as_str()
                            .is_some_and(|f| f.contains("ability:"))
                            || bare_sleep_after_a_hit)
                    {
                        action.contact_status = Some(text(&event["status"]));
                        action.contact_status_after_hits = action.crits.len();
                        action.contact_status_hits.push(action.crits.len());
                        action.contact_status_of = event["of"].as_str().and_then(|of| {
                            who_from_text(&Value::String(of.trim_start_matches("[of] ").into()))
                        });
                    }
                    // Drain and self-heal moves commit HP the engine traces as HealApplied, and so
                    // does an absorbing ability on a target (Water Absorb, Volt Absorb), which
                    // also stops the move there like an immunity. Residual heals come after the
                    // `residual` marker and are skipped above.
                    if kind == "heal" {
                        let from = event["from"].as_str();
                        let absorbed = from.is_some_and(|f| {
                            f.contains("ability: Water Absorb")
                                || f.contains("ability: Volt Absorb")
                                || f.contains("ability: Dry Skin")
                                || f.contains("ability: Earth Eater")
                        });
                        // Gen 2's Leech Seed heals the seeder silently within the action.
                        let seeded = status_after_action && from == Some("[silent]");
                        // A berry (or Berry Juice) an `Update` inside the action fed its holder
                        // (a recording without the residual marker shows Leftovers here too);
                        // Shell Bell's heal after the user's move (DESIGN.md R232).
                        let item = from.is_some_and(|f| {
                            f.strip_prefix("[from] item: ").is_some_and(|item| {
                                item.ends_with(" Berry")
                                    || item == "Berry Juice"
                                    || item == "Shell Bell"
                            })
                        });
                        // Rest heals its user silently (DESIGN.md R170) — a called one too
                        // (R-M8-70), a snatched one its snatcher.
                        let rested = from == Some("[silent]")
                            && (target == action.user
                                || action
                                    .called
                                    .last()
                                    .is_some_and(|called| called.user == target))
                            && action.used_move().is_some_and(|name| name == "Rest");
                        // Cheek Pouch after a berry the action's `Update` fed its holder.
                        let pouch = from == Some("[from] ability: Cheek Pouch");
                        // Hospitality a Transform (or another ability change) started within
                        // the action heals the partner (R-M8-84).
                        let hospitality = from == Some("[from] ability: Hospitality");
                        // A wish waiting on a slot an Ally Switch moved a Pokémon into (Gen 8+
                        // `onSwap`, R-M8-86).
                        let wished = matches!(
                            from,
                            Some("[from] move: Lunar Dance" | "[from] move: Healing Wish")
                        );
                        // A Z status move's own heal (Z-Refresh's `zMove.effect: 'heal'`) as the
                        // move begins (DESIGN.md R-M9-22).
                        let z_effect = from == Some("[zeffect]");
                        if from.is_none()
                            || from.is_some_and(|f| f.contains("drain"))
                            || absorbed
                            || seeded
                            || item
                            || rested
                            || pouch
                            || hospitality
                            || wished
                            || z_effect
                        {
                            let hp = event["hp"].as_u64().unwrap_or(0) as u16;
                            action.damages.push((target.clone(), hp));
                        }
                        // Stopped there, by the move it absorbed: one sent back at the action's
                        // own user (Magic Bounce) is its reflector's, so the user absorbs it too.
                        let mover = action.called.last().map_or(&action.user, |c| &c.user);
                        if absorbed && target != *mover {
                            action.try_hit_blocked.push(target.clone());
                            action.immune.push(target.clone());
                        }
                    }
                    // A stat change names its stat: Flash Cannon's Special Defense drop is not the
                    // Attack boost a Contrary target takes from an Intimidate in the same action.
                    let label = match kind {
                        "status" => format!("status:{}", text(&event["status"])),
                        "boost" | "unboost" => format!("{kind}:{}", text(&event["stat"])),
                        // A heal from something else within the action (Gen 2's Leech Seed after
                        // the user's move heals the seeder `[silent]`) is not the move's own.
                        "heal" if !event["from"].is_null() => "heal:from".to_string(),
                        other => other.to_string(),
                    };
                    // Poison Touch / Toxic Chain on a target: the user's reaction to the hit, not
                    // the move's own (a Poison Tail's poison, R-M10-41).
                    let dealt = kind == "status"
                        && target != action.user
                        && event["from"].as_str().is_some_and(|f| {
                            f.contains("ability: Poison Touch")
                                || f.contains("ability: Toxic Chain")
                        });
                    if dealt {
                        action
                            .dealt_status
                            .push((target.clone(), action.crits.len()));
                    }
                    // No status an ability gave is the move's roll: a Synchronize sending the
                    // user's status back at the target the move hit (a Gen 3 Kadabra paralysed
                    // by Static after Thunder Punch, R-M10-60) among them.
                    let by_ability = kind == "status"
                        && event["from"]
                            .as_str()
                            .is_some_and(|f| f.contains("ability: "));
                    let item_stat = matches!(kind, "boost" | "unboost")
                        && item_boosting.as_ref() == Some(&target);
                    if item_stat {
                        item_stat_change = item_boosting.clone();
                    }
                    if action.damages.iter().any(|(t, _)| *t == target)
                        && kind != "heal"
                        && !item_stat
                        && !by_ability
                    {
                        action.secondary_applied = true;
                        let hits = action.crits.len();
                        action
                            .secondary_on
                            .push((target.clone(), label.clone(), hits));
                    }
                    if target == action.user {
                        let hits = action.crits.len();
                        action.user_applied_hits.push((label.clone(), hits));
                    }
                    action.applied.push((target, label));
                }
            }
            "start" => {
                let args = event["args"].as_array().cloned().unwrap_or_default();
                // Flash Fire took the move (`-start|target|ability: Flash Fire`): like an
                // absorbing ability's `-ability … boost`, the target draws no decision
                // (DESIGN.md R172). The user's own Flash Fire too when the move in progress is
                // aimed at it — Magic Bounce or Magic Coat sent its Will-O-Wisp back (R-M9-54).
                if args.get(1).and_then(Value::as_str) == Some("ability: Flash Fire")
                    && let (Some(action), Some(target)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                    && (target != action.user || action.move_target.as_ref() == Some(&target))
                    && action.move_name.is_some()
                {
                    action.hit = false;
                    action.try_hit_blocked.push(target.clone());
                    action.immune.push(target);
                }
                if let (Some(action), Some(target)) =
                    (out.last_mut(), args.first().and_then(who_from_text))
                {
                    let what = args.get(1).and_then(Value::as_str).unwrap_or("");
                    // Gen 1 Mimic learnt a move (`-start|p|Mimic|Metronome`): which of the
                    // target's slots it drew, read off the move it learned.
                    if what == "Mimic" {
                        action.mimicked = args.get(2).and_then(Value::as_str).map(str::to_owned);
                        continue;
                    }
                    // Uproar starting on its user: the move's own lock, no secondary.
                    if what == "Uproar" && target == action.user {
                        continue;
                    }
                    // A lock's fatigue (Thrash and its kind): the holder's own confusion, no
                    // effect of the move's. Two locks may end on the same turn, so the line's
                    // Pokémon is matched to *its* action, not the last one (DESIGN.md R-M8-44).
                    if what == "confusion"
                        && args
                            .iter()
                            .any(|arg| matches!(arg.as_str(), Some("[fatigue]" | "[silent]")))
                    {
                        let owner = out
                            .iter()
                            .rposition(|action| action.user == target)
                            .unwrap_or(out.len().saturating_sub(1));
                        out[owner].fatigue = true;
                        continue;
                    }
                    // Cursed Body on the user: the target's reaction to the hit.
                    if what == "Disable"
                        && target == action.user
                        && args
                            .iter()
                            .any(|arg| arg.as_str() == Some("[from] ability: Cursed Body"))
                    {
                        let of = args.iter().find_map(|arg| {
                            arg.as_str()?
                                .strip_prefix("[of] ")
                                .map(|of| who_from_text(&Value::String(of.to_string())))
                        });
                        action.cursed_body = Some((action.crits.len(), of.flatten()));
                    }
                    // Cute Charm on the user: the target's reaction to the hit.
                    if what == "Attract"
                        && target == action.user
                        && args
                            .iter()
                            .any(|arg| arg.as_str() == Some("[from] ability: Cute Charm"))
                    {
                        let of = args.iter().find_map(|arg| {
                            arg.as_str()?
                                .strip_prefix("[of] ")
                                .map(|of| who_from_text(&Value::String(of.to_string())))
                        });
                        action.cute_charm = Some((action.crits.len(), of.flatten()));
                        action.cute_charm_hits.push(action.crits.len());
                    }
                    // (Or past a doll the hit left standing: Gen 1's confusion, R-M8-93.)
                    if action.damages.iter().any(|(t, _)| *t == target)
                        || action
                            .substitute_struck
                            .iter()
                            .any(|(struck, broken)| *struck == target && !broken)
                    {
                        action.secondary_applied = true;
                        action.secondary_on.push((
                            target.clone(),
                            format!("start:{what}"),
                            action.crits.len(),
                        ));
                    }
                    if what == "Disable" {
                        action.disabled_move =
                            args.get(2).and_then(Value::as_str).map(str::to_owned);
                    }
                    if what == "typechange"
                        && !args.iter().any(|arg| {
                            arg.as_str().is_some_and(|a| {
                                a == "[silent]" || a.starts_with("[from] ability:")
                            })
                        })
                        && let Some(type_name) = args.get(2).and_then(Value::as_str)
                    {
                        action
                            .typechanges
                            .push((target.clone(), type_name.to_owned()));
                    }
                    if target == action.user {
                        let hits = action.crits.len();
                        action
                            .user_applied_hits
                            .push((format!("start:{what}"), hits));
                    }
                    action.applied.push((target, format!("start:{what}")));
                }
            }
            // `-item|user|item|[from] move: Thief|[of] target`: Gen 2 Thief's secondary took the
            // target's item; `[from] ability: Magician`: the actor's Magician did.
            "item" => {
                let args = event["args"].as_array().cloned().unwrap_or_default();
                let from = |source: &str| args.iter().any(|arg| arg.as_str() == Some(source));
                let of = args.iter().find_map(|arg| {
                    arg.as_str()?
                        .strip_prefix("[of] ")
                        .and_then(|of| who_from_text(&Value::String(of.to_string())))
                });
                if from("[from] ability: Magician")
                    && let Some(action) = out.last_mut()
                {
                    action.magician_from = of.clone();
                }
                if from("[from] move: Thief")
                    && let Some(target) = of
                    && let Some(action) = out.last_mut()
                {
                    action.secondary_applied = true;
                    action
                        .secondary_on
                        .push((target, "stolen".to_string(), action.crits.len()));
                }
            }
            _ => {}
        }
    }
    // A flinch is silent in the protocol (no `-start`), so the secondary that caused it is only
    // visible through the victim's later `cant flinch`: mark the most recent action that
    // damaged the victim.
    for index in 0..out.len() {
        if out[index].cant.as_deref() != Some("flinch") {
            continue;
        }
        let victim = out[index].user.clone();
        let (earlier, _) = out.split_at_mut(index);
        if let Some(cause) =
            flinch_cause(earlier, &victim, reading, adds_flinch).map(|at| &mut earlier[at])
        {
            cause.secondary_applied = true;
            cause
                .secondary_on
                .push((victim, "flinch".to_string(), usize::MAX));
        }
    }
    out
}

/// Which of `actions` flinched `victim`: the last hit on it that can flinch — a flinching
/// move (the one the action used: Copycat's Bite, R-M8-86), or one its user's item or ability
/// adds a flinch to (`adds_flinch`: King's Rock only on the Gen 2-4 list, R-M9-34); failing
/// that, the last hit on it.
fn flinch_cause(
    actions: &[ActionOracle],
    victim: &Who,
    reading: &LogReading,
    adds_flinch: &dyn Fn(&Who, Option<&str>) -> bool,
) -> Option<usize> {
    let hit_victim =
        |action: &ActionOracle| action.damages.iter().any(|(target, _)| target == victim);
    actions
        .iter()
        .rposition(|action| {
            hit_victim(action)
                && (action
                    .used_move()
                    .is_some_and(|name| reading.flinching_moves.contains(name))
                    || adds_flinch(&action.user, action.used_move().map(String::as_str)))
        })
        .or_else(|| actions.iter().rposition(hit_victim))
}

/// A flinch is silent; besides the victim's later `cant flinch`, a frame taken before the turn
/// ends (a mid-turn request) shows it on a victim that had already moved, and a Gen 1 frame by
/// the recharge it cleared. Only a hit after the victim's own action can have left it there (an
/// earlier one would have stopped that action).
fn attribute_frame_flinches(
    decisions: &mut [ActionOracle],
    frame: &Value,
    reading: &LogReading,
    adds_flinch: &dyn Fn(&Who, Option<&str>) -> bool,
) {
    let mut victims: Vec<Who> = Vec::new();
    for (victim, _) in decisions.iter().flat_map(|action| &action.damages) {
        if !victims.contains(victim) {
            victims.push(victim.clone());
        }
    }
    for victim in victims {
        let side = &frame["sides"][victim.side];
        let volatiles = victim
            .slot
            .and_then(|slot| side["active"][slot].as_u64())
            .map(|party| &side["party"][party as usize]["volatiles"]);
        // A victim whose own action something checked before the flinch stopped (recharge 11,
        // sleep and freeze 10, Truant 9; the flinch is 8 in `BeforeMove`) never ran the flinch's
        // `onBeforeMove`: a hit before that action may have left it too (DESIGN.md R-M9-28).
        let since = decisions
            .iter()
            .rposition(|action| action.user == victim)
            .map_or(0, |at| {
                let stopped_first = decisions[at].cant.as_deref().is_some_and(|reason| {
                    matches!(reason, "recharge" | "slp" | "frz" | "ability: Truant")
                });
                if stopped_first { 0 } else { at + 1 }
            });
        // Gen 1: a flinch clears the recharge the victim's own recharge move left this boundary
        // (`flinch.onStart`): the frame shows no `mustrecharge` after a later flinching hit. A
        // recharge the move never left (a knock-out, a miss) makes the flinch as invisible as it
        // is to a Pokémon that already moved (R-M8-105).
        let recharge_cleared = reading.flinch_clears_recharge
            && volatiles.is_some_and(|volatiles| volatiles.get("mustrecharge").is_none())
            && decisions[..since].iter().any(|action| {
                action.user == victim
                    && action
                        .used_move()
                        .is_some_and(|name| reading.recharge_moves.contains(name))
            })
            && decisions[since..].iter().any(|action| {
                action.damages.iter().any(|(target, _)| *target == victim)
                    && action
                        .used_move()
                        .is_some_and(|name| reading.flinching_moves.contains(name))
            });
        let flinched = volatiles.is_some_and(|volatiles| !volatiles["flinch"].is_null());
        if !(flinched || recharge_cleared) {
            continue;
        }
        let later = &mut decisions[since..];
        if let Some(cause) =
            flinch_cause(later, &victim, reading, adds_flinch).map(|at| &mut later[at])
            && !cause
                .secondary_on
                .iter()
                .any(|(who, label, _)| *who == victim && label == "flinch")
        {
            cause
                .secondary_on
                .push((victim, "flinch".to_string(), usize::MAX));
        }
    }
}

/// What Showdown reported for the end of the turn that needs a decision: who Shed Skin cured
/// (`-activate|p|ability: Shed Skin`), who thawed in the residual (Gen 2:
/// `-curestatus|p|frz|[msg]` after the turn's `residual` marker), who fell asleep there
/// (`-status|p|slp`: Yawn ran out), whom a Healer cured (the `-curestatus` right after its
/// `-activate|p|ability: Healer`), and who Harvest gave its berry back
/// (`-item|p|X|[from] ability: Harvest`).
pub fn extract_residual_cures(events: &[Value]) -> Vec<(Who, ResidualKind)> {
    let mut out = Vec::new();
    let mut in_residual = false;
    let mut healer_activated = false;
    for event in events {
        let healer = std::mem::take(&mut healer_activated);
        if event["kind"] == "residual" {
            in_residual = true;
        } else if event["kind"] == "activate"
            && let Some(args) = event["args"].as_array()
            && args.get(1).and_then(Value::as_str) == Some("ability: Healer")
        {
            healer_activated = true;
        } else if healer
            && event["kind"] == "curestatus"
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
        {
            out.push((who, ResidualKind::Healer));
        } else if event["kind"] == "activate"
            && let Some(args) = event["args"].as_array()
            && args.get(1).and_then(Value::as_str) == Some("ability: Shed Skin")
            && let Some(who) = args.first().and_then(who_from_text)
        {
            out.push((who, ResidualKind::ShedSkin));
        } else if event["kind"] == "item"
            && let Some(args) = event["args"].as_array()
            && args.get(2).and_then(Value::as_str) == Some("[from] ability: Harvest")
            && let Some(who) = args.first().and_then(who_from_text)
        {
            out.push((who, ResidualKind::Harvest));
        } else if in_residual
            && event["kind"] == "curestatus"
            && event["status"] == "frz"
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
        {
            out.push((who, ResidualKind::FreezeThaw));
        } else if in_residual
            && event["kind"] == "curestatus"
            && event["from"] == "[from] move: Uproar"
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
        {
            // Gen 3-4 (issues.md I63): woken by an uproar's turn-end step.
            out.push((who, ResidualKind::Uproar));
        } else if in_residual
            && event["kind"] == "status"
            && event["status"] == "slp"
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
        {
            out.push((who, ResidualKind::Yawn));
        } else if in_residual
            && event["kind"] == "start"
            && event["args"][1] == "confusion"
            && event["args"][2] == "[fatigue]"
            && let Some(who) = who_from_text(&event["args"][0])
        {
            // A lock (Thrash and its kind) that ended in the residual: its fatigue.
            out.push((who, ResidualKind::Rampage));
        }
    }
    out
}

/// Who fell in the residual, and at which handler (whose holder): the source of the last damage
/// the log shows it before its `faint` — the weather (Sandstorm, Hail, Dry Skin, Solar Power), its
/// own poison, burn, Leech Seed, binding, Nightmare, Curse, Salt Cure, Sticky Barb, Black Sludge,
/// a foreseen strike, a foe's Bad Dreams — or Perish Song's count. A fall the log does not tie to
/// a handler is left out (R-M9-51).
pub fn extract_residual_faints(events: &[Value]) -> Vec<(Who, ResidualKind, Who)> {
    let mut out = Vec::new();
    let mut in_residual = false;
    let mut last: Vec<(Who, ResidualKind, Who)> = Vec::new();
    let mut foreseen: Option<Who> = None;
    for event in events {
        let kind = event["kind"].as_str().unwrap_or("");
        if kind == "residual" {
            in_residual = true;
            continue;
        }
        if !in_residual {
            continue;
        }
        let pokemon = serde_json::from_value::<Who>(event["pokemon"].clone()).ok();
        let args = event["args"].as_array().cloned().unwrap_or_default();
        let arg = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or("");
        let mut mark = |who: Who, what: ResidualKind, holder: Who| {
            last.retain(|(w, _, _)| *w != who);
            last.push((who, what, holder));
        };
        match kind {
            "end" if arg(1).contains("Future Sight") || arg(1).contains("Doom Desire") => {
                foreseen = who_from_text(&args[0]);
            }
            "damage" => {
                let Some(who) = pokemon else { continue };
                let from = event["from"].as_str().unwrap_or("");
                let of = event["of"].as_str().unwrap_or("");
                let what = if from.is_empty() && foreseen.as_ref() == Some(&who) {
                    Some(ResidualKind::FutureAttack)
                } else if ["Sandstorm", "Hail", "Dry Skin", "Solar Power"]
                    .iter()
                    .any(|weather| from.contains(weather))
                {
                    Some(ResidualKind::Weather)
                } else if from == "[from] psn" {
                    Some(ResidualKind::Poison)
                } else if from == "[from] brn" {
                    Some(ResidualKind::Burn)
                } else if from.contains("Leech Seed") {
                    Some(ResidualKind::LeechSeed)
                } else if of == "[partiallytrapped]" {
                    Some(ResidualKind::PartiallyTrapped)
                } else if from.contains("Nightmare") {
                    Some(ResidualKind::Nightmare)
                } else if from.contains("Curse") {
                    Some(ResidualKind::Curse)
                } else if from.contains("Salt Cure") {
                    Some(ResidualKind::SaltCure)
                } else if from.contains("Sticky Barb") {
                    Some(ResidualKind::StickyBarb)
                } else if from.contains("Black Sludge") {
                    Some(ResidualKind::BlackSludge)
                } else if from.contains("Bad Dreams") {
                    // The handler is the foe's: its holder is the `[of]`.
                    if let Some(holder) =
                        who_from_text(&Value::from(of.strip_prefix("[of] ").unwrap_or(of)))
                    {
                        mark(who, ResidualKind::BadDreams, holder);
                    }
                    continue;
                } else {
                    None
                };
                match what {
                    Some(what) => mark(who.clone(), what, who),
                    None => last.retain(|(w, _, _)| *w != who),
                }
            }
            "start" if arg(1) == "perish0" => {
                if let Some(who) = who_from_text(&args[0]) {
                    mark(who.clone(), ResidualKind::PerishSong, who);
                }
            }
            "faint" => {
                if let Some(who) = pokemon
                    && let Some(fall) = last.iter().find(|(w, _, _)| *w == who)
                {
                    out.push(fall.clone());
                }
            }
            _ => {}
        }
    }
    out
}

/// A foreseen attack's strike as the log shows it (`-end|<target>|move: Future Sight`, DESIGN.md
/// §15.22), with what followed it for that target.
#[derive(Debug, Clone)]
struct FutureStrikeOracle {
    target: Who,
    missed: bool,
    immune: bool,
    critical: bool,
    /// The target's Focus Band held it (`-activate|<target>|item: Focus Band`).
    focus_band_held: bool,
    /// What a berry the strike's `Update` fed the target drew (Starf Berry's stat).
    berry_draw: Option<BerryDraw>,
    /// The target's HP after the strike's damage; none when it took none (a miss, an immunity,
    /// a Substitute).
    hp_after: Option<u16>,
}

/// The foreseen attacks this boundary's residual struck with, in log order.
fn extract_future_strikes(events: &[Value]) -> Vec<FutureStrikeOracle> {
    let mut out: Vec<FutureStrikeOracle> = Vec::new();
    let mut open = false;
    let mut fed = false;
    let mut eaten: Option<(Who, String)> = None;
    for event in events {
        let kind = event["kind"].as_str().unwrap_or("");
        if kind == "end"
            && let Some(args) = event["args"].as_array()
            && matches!(
                args.get(1).and_then(Value::as_str),
                Some("move: Future Sight" | "move: Doom Desire")
            )
            && let Some(target) = args.first().and_then(who_from_text)
        {
            out.push(FutureStrikeOracle {
                target,
                missed: false,
                immune: false,
                critical: false,
                focus_band_held: false,
                berry_draw: None,
                hp_after: None,
            });
            open = true;
            fed = true;
            continue;
        }
        // The strike's `Update` (DESIGN.md R277): a berry it fed its target draws.
        let previous = eaten.take();
        eaten = berry_eaten(event);
        if fed
            && let Some(strike) = out.last_mut()
            && let Some((eater, Some(draw))) = BerryDraw::berry_boost(previous.as_ref(), event)
            && eater == strike.target
        {
            strike.berry_draw = Some(draw);
        }
        if matches!(kind, "residual" | "turn" | "move") {
            fed = false;
        }
        if !open {
            continue;
        }
        let Some(strike) = out.last_mut() else {
            continue;
        };
        let about = |field: &str| {
            serde_json::from_value::<Who>(event[field].clone())
                .ok()
                .is_some_and(|who| who.side == strike.target.side && who.name == strike.target.name)
        };
        match kind {
            // `-miss|source|target`: the target in `detail`.
            "miss" => {
                strike.missed = true;
                open = false;
            }
            "immune" if about("pokemon") => {
                strike.immune = true;
                open = false;
            }
            "crit" if about("pokemon") => strike.critical = true,
            "activate"
                if event["args"].get(1).and_then(Value::as_str) == Some("item: Focus Band")
                    && event["args"]
                        .get(0)
                        .and_then(who_from_text)
                        .is_some_and(|who| who == strike.target) =>
            {
                strike.focus_band_held = true;
            }
            "damage" if about("pokemon") && event["from"].is_null() => {
                strike.hp_after = event["hp"].as_u64().map(|hp| hp as u16);
                open = false;
            }
            "residual" | "turn" => open = false,
            _ => {}
        }
    }
    out
}

/// Whether an action broke a Substitute: a Pokémon that had one before has none after.
fn broke_a_doll(before: &BattleState, after: &BattleState) -> bool {
    before.all_pokemon().iter().any(|pokemon| {
        substitute_hp(before, pokemon.id()).is_some()
            && substitute_hp(after, pokemon.id()).is_none()
    })
}

/// Whether Gen 1's `lastDamage` is read from boundary `from` on: a Bide release (`-end|p|Bide`),
/// a Counter (R-M8-102), or a binding lock's next turn (`move|p|Wrap|…|[from] Wrap`), which
/// repeats the damage its first hit left there (R-M8-143).
fn last_damage_read_later(frames: &[Option<&Value>], from: usize) -> bool {
    frames[from.min(frames.len())..]
        .iter()
        .flatten()
        .filter_map(|frame| frame["events"].as_array())
        .flatten()
        .any(|event| {
            (event["kind"] == "end"
                && matches!(
                    event["args"].get(1).and_then(Value::as_str),
                    Some("Bide" | "move: Bide")
                ))
                || (event["kind"] == "move" && event["move"] == "Counter")
                || (event["kind"] == "move"
                    && event["move"]
                        .as_str()
                        .is_some_and(|name| event["from"] == format!("[from] {name}").as_str()))
        })
}

/// Gen 1's open choice after a roll that broke a Substitute (`OpenChoices`, R-M8-102): the
/// other roll sets that reproduce the action as well as `chosen` — the same HP commits and
/// Substitutes, the plan used up — but leave another `lastDamage`, one per value.
fn other_last_damages(
    compiled: &Compiled,
    before: &BattleSnapshot,
    trace_len: usize,
    tape: &[RandomOverride],
    chosen: &BattleEngine,
) -> Vec<Vec<RandomOverride>> {
    let damage = compiled.rules.damage();
    let dolls = |engine: &BattleEngine| {
        let state = engine.state();
        state
            .all_pokemon()
            .iter()
            .map(|pokemon| substitute_hp(state, pokemon.id()))
            .collect::<Vec<_>>()
    };
    let dealt = damages_in(chosen, trace_len);
    // The roll's own range: Psywave's damage is the roll (`psywave_range` by its user's level),
    // any other move's roll the damage formula's random factor (R-M9-48).
    let psywave_level = chosen.trace().events()[trace_len..]
        .iter()
        .find_map(|event| match event {
            TraceEvent::MoveUsed { actor, move_id, .. }
                if compiled.dex.move_by_id(*move_id).is_some_and(|m| {
                    m.fixed_damage == Some(pokemon_battle_engine::dex::FixedDamage::Psywave)
                }) =>
            {
                before.state.pokemon(*actor).map(|pokemon| pokemon.level)
            }
            _ => None,
        });
    let (min, max) = psywave_level.map_or((damage.random_min, damage.random_max), |level| {
        pokemon_battle_engine::engine::psywave_range(&compiled.rules, level)
    });
    let mut values = vec![chosen.state().last_damage];
    let mut found = Vec::new();
    for (at, decision) in tape.iter().enumerate() {
        if decision.kind != RandomRequestKind::DamageRoll {
            continue;
        }
        for roll in (min..=max).rev() {
            let mut other = tape.to_vec();
            other[at] = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
            if decision.optional {
                other[at] = other[at].optional();
            }
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut candidate = restore_with_tape(compiled, before, other.clone());
                candidate.step().ok().map(|_| candidate)
            }));
            let Ok(Some(candidate)) = run else {
                continue;
            };
            let last_damage = candidate.state().last_damage;
            if damages_in(&candidate, trace_len) == dealt
                && dolls(&candidate) == dolls(chosen)
                && candidate.state().rng.semantic_tape_exhausted()
                && !values.contains(&last_damage)
            {
                values.push(last_damage);
                found.push(other);
            }
        }
    }
    found
}

/// Gen 4 Future Sight / Doom Desire (§15.22): the damage the strike dealt to the foreseen slot
/// (`target`'s side and slot), read from the boundary frames after `from` — its occupant's HP
/// before the strike (its last HP in that boundary's events, or the frame before's) less the HP
/// after — and whether it knocked the occupant out (any larger damage then fits). None when no
/// strike shows HP damage (a miss, an immunity, a battle over before it). A Substitute that took
/// the strike shows it by its HP; one another move struck earlier that turn leaves the strike's
/// share open, at most the turn's whole loss (`ForeseenDamage::AtMost`, R-M8-164).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForeseenDamage {
    /// What the strike dealt, and whether it knocked out (or broke the doll).
    Dealt(u16, bool),
    /// A doll struck earlier in the strike's turn too: the strike took at most this much (or,
    /// when it broke the doll, anything).
    AtMost(u16, bool),
}

fn future_damage_hint(
    frames: &[Option<&Value>],
    from: usize,
    target: &Who,
) -> Option<ForeseenDamage> {
    let same = |event: &Value, who: &Who| {
        serde_json::from_value::<Who>(event["pokemon"].clone())
            .ok()
            .is_some_and(|other| other.side == who.side && other.name == who.name)
    };
    for index in from + 1..frames.len() {
        let Some(frame) = frames[index] else {
            continue;
        };
        let events = frame["events"].as_array()?;
        for (at, event) in events.iter().enumerate() {
            let Some(args) = event["args"].as_array() else {
                continue;
            };
            if event["kind"] != "end"
                || !matches!(
                    args.get(1).and_then(Value::as_str),
                    Some("move: Future Sight" | "move: Doom Desire")
                )
            {
                continue;
            }
            let Some(struck) = args.first().and_then(who_from_text) else {
                continue;
            };
            // A target named without a place (a fainted foe, `p2: Slaking`): the slot the use
            // aimed at, the first (`foreseen_spot`, R-M8-89).
            if struck.side != target.side || struck.slot != target.slot.or(Some(0)) {
                continue;
            }
            // A strike a Substitute took (`-activate|p|Substitute|[damage]`, `-end|p|Substitute`
            // as it breaks): the doll's HP the frames show, before and at this boundary — a
            // broken doll took at least what it had (R-M8-101).
            let on_doll = events[at + 1..]
                .iter()
                .take_while(|later| later["kind"] != "residual" && later["kind"] != "damage")
                .find_map(|later| {
                    let args = later["args"].as_array()?;
                    let doll = args.first().and_then(who_from_text)? == struck
                        && args.get(1).and_then(Value::as_str) == Some("Substitute");
                    match later["kind"].as_str() {
                        Some("activate") if doll => Some(false),
                        Some("end") if doll => Some(true),
                        _ => None,
                    }
                });
            if let Some(broken) = on_doll {
                let doll_hp = |frame: &Value| {
                    frame_pokemon(frame, &struck)?["volatiles"]["substitute"].as_u64()
                };
                let previous = frames[..index].iter().rev().find_map(|frame| *frame)?;
                // A doll put up earlier in the strike's turn (`-start|p|Substitute`) is in no
                // earlier frame: it started with a quarter of the max HP (`floor(maxhp / 4)`,
                // DESIGN.md R-M9-28).
                let made_this_turn = || {
                    events[..at].iter().any(|earlier| {
                        earlier["kind"] == "start"
                            && earlier["args"].as_array().is_some_and(|args| {
                                args.first().and_then(who_from_text).as_ref() == Some(&struck)
                                    && args.get(1).and_then(Value::as_str) == Some("Substitute")
                            })
                    })
                };
                let had = match doll_hp(previous) {
                    Some(hp) => hp as u16,
                    None if made_this_turn() => {
                        (frame_pokemon(frame, &struck)?["max_hp"].as_u64()? / 4) as u16
                    }
                    None => return None,
                };
                // Another move struck the doll earlier that turn: the loss is both hits'.
                let struck_before = events[..at].iter().any(|earlier| {
                    earlier["kind"] == "activate"
                        && earlier["args"].as_array().is_some_and(|args| {
                            args.first().and_then(who_from_text).as_ref() == Some(&struck)
                                && args.get(1).and_then(Value::as_str) == Some("Substitute")
                        })
                });
                let lost = if broken {
                    had
                } else {
                    had.saturating_sub(doll_hp(frame)? as u16)
                };
                return Some(if struck_before {
                    ForeseenDamage::AtMost(lost, broken)
                } else {
                    ForeseenDamage::Dealt(lost, broken)
                });
            }
            let before = events[..at]
                .iter()
                .rev()
                .filter(|earlier| same(earlier, &struck))
                .find_map(|earlier| earlier["hp"].as_u64())
                .or_else(|| {
                    let previous = frames[..index].iter().rev().find_map(|frame| *frame)?;
                    frame_pokemon(previous, &struck)?["hp"].as_u64()
                })? as u16;
            let after = events[at + 1..]
                .iter()
                .take_while(|later| later["kind"] != "residual" && later["kind"] != "end")
                .find(|later| {
                    later["kind"] == "damage" && same(later, &struck) && later["from"].is_null()
                })?["hp"]
                .as_u64()? as u16;
            return Some(ForeseenDamage::Dealt(
                before.saturating_sub(after),
                after == 0,
            ));
        }
    }
    None
}

/// `args` entries are raw protocol fields such as `p2a: Mew`.
/// Neutralizing Gas clearing in this boundary (`-end|<holder>|ability: Neutralizing Gas`,
/// DESIGN.md R203): the Pokémon that draw a `SpeedTie` as the others' abilities start again
/// (`BattleEngine::neutralizing_gas_restart_ties`), each ranked by the order the restarted
/// abilities announce themselves after it (`-ability`, a weather's or terrain's `[of]`).
/// Optional: the step may not be the one the gas clears in.
fn neutralizing_gas_tape(
    compiled: &Compiled,
    before: &BattleSnapshot,
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    let events = oracle_frame["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let Some(at) = events
        .iter()
        .position(|e| e["kind"] == "end" && e["args"][1] == "ability: Neutralizing Gas")
    else {
        return Vec::new();
    };
    let Some(holder) =
        who_from_text(&events[at]["args"][0]).and_then(|who| who_to_id(&before.state, &who))
    else {
        return Vec::new();
    };
    let drawing =
        restore_with_tape(compiled, before, Vec::new()).neutralizing_gas_restart_ties(holder);
    let mut shown: Vec<PokemonId> = Vec::new();
    for event in &events[at + 1..] {
        let args = event["args"].as_array();
        let who = match event["kind"].as_str() {
            Some("ability") => args.and_then(|args| args.first()).and_then(who_from_text),
            Some("weather" | "fieldstart") => args
                .into_iter()
                .flatten()
                .find_map(|arg| arg.as_str()?.strip_prefix("[of] ").map(str::to_owned))
                .and_then(|text| who_from_text(&Value::String(text))),
            _ => None,
        };
        if let Some(id) = who.and_then(|who| who_to_id(&before.state, &who))
            && !shown.contains(&id)
        {
            shown.push(id);
        }
    }
    let mut ranked = drawing.clone();
    ranked.sort_by_key(|id| shown.iter().position(|s| s == id).unwrap_or(usize::MAX));
    drawing
        .iter()
        .map(|id| {
            let rank = ranked.iter().position(|r| r == id).unwrap_or(0);
            RandomOverride::raw(RandomRequestKind::SpeedTie, (drawing.len() - rank) as u64)
                .optional()
        })
        .collect()
}

fn who_from_text(value: &Value) -> Option<Who> {
    let text = value.as_str()?;
    let (position, name) = text.split_once(": ")?;
    let side = match position.chars().nth(1)? {
        '1' => 0,
        '2' => 1,
        _ => return None,
    };
    let slot = position.chars().nth(2).map(|c| usize::from(c as u8 - b'a'));
    Some(Who {
        side,
        slot,
        name: name.to_string(),
    })
}

// ---------------------------------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ReplayReport {
    pub frames_compared: usize,
    pub actions_replayed: usize,
    pub divergences: Vec<String>,
    /// Rust frames at every compared boundary, in order (for the failure bundle).
    pub rust_frames: Vec<Value>,
    /// The engine's full trace after replay, one event per line.
    pub trace: Vec<String>,
    /// Rules / dex fingerprints the replay ran against.
    pub provenance: Value,
    /// The Gen 1 open choices this replay met and the ones it was told to take (R-M8-102).
    pub open_choices: OpenChoices,
}

/// Choices the log leaves open until later. Gen 1 (R-M8-102): a roll that broke a Substitute
/// sets `lastDamage` to the damage uncapped (Showdown's gen1 Substitute), which no frame shows
/// until a Bide release or a Counter reads it. Gen 4 (R-M8-164): the roll a Future Sight stored
/// its damage with, when another move struck the doll the strike lands on. Each such action is
/// a choice between the roll sets that leave another outcome; `replay_against_oracle` runs a
/// diverging replay again with them taken.
#[derive(Debug, Default, Clone)]
pub struct OpenChoices {
    /// The alternative to take at an open choice, by its place among the choices met.
    pins: BTreeMap<usize, usize>,
    /// How many alternatives each choice met so far had, in order.
    met: Vec<usize>,
}

fn restore_with_tape(
    compiled: &Compiled,
    snapshot: &BattleSnapshot,
    tape: Vec<RandomOverride>,
) -> BattleEngine {
    let mut snapshot = snapshot.clone();
    snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
    BattleEngine::restore(compiled.rules.clone(), compiled.dex.clone(), snapshot).expect("restore")
}

/// Which of `candidates` a random target draw picked, from the target the `move` line names:
/// that one — unless the Lightning Rod / Storm Drain holder named there drew the move to itself
/// (`redirected_by`), and then the draw was another candidate, the first (any other is drawn
/// to the holder alike, and the holder's ability is announced either way, R-M11-7). `None`: the
/// named target is no candidate.
fn drawn_target(
    state: &BattleState,
    candidates: &[PokemonId],
    named: Option<&Who>,
    redirected_by: Option<&Who>,
) -> Option<usize> {
    let named = named.and_then(|who| who_to_id(state, who))?;
    if redirected_by.and_then(|who| who_to_id(state, who)) == Some(named) {
        return candidates.iter().position(|candidate| *candidate != named);
    }
    candidates.iter().position(|candidate| *candidate == named)
}

fn who_to_id(state: &BattleState, who: &Who) -> Option<PokemonId> {
    let side = if who.side == 0 {
        SideId::P1
    } else {
        SideId::P2
    };
    state.side(side).active.get(who.slot?).copied().flatten()
}

/// Where the Pokémon `who` names in `state` (the state an action began in) stands in a later
/// boundary `frame`: its party index there (the frame's `party` is in team order), if it is
/// still on the field. Its slot may hold another Pokémon by then — an Ally Switch later in the
/// turn swaps the two (R-M8-175), a switch brings a replacement in.
fn frame_member(state: &BattleState, frame: &Value, who: &Who) -> Option<usize> {
    let id = who_to_id(state, who)?;
    let side = if who.side == 0 {
        SideId::P1
    } else {
        SideId::P2
    };
    let member = state
        .side(side)
        .team
        .iter()
        .position(|member| *member == id)?;
    frame["sides"][who.side]["active"]
        .as_array()?
        .iter()
        .any(|index| index.as_u64() == Some(member as u64))
        .then_some(member)
}

/// Whether two names are the same Pokémon: by side and name, whatever slot each was named in
/// (it may have moved within the turn, or come back into another slot after a switch).
fn same_pokemon(a: &Who, b: &Who) -> bool {
    a.side == b.side && a.name == b.name
}

/// The party index of the Pokémon in `who`'s slot in `frame` (the boundary `who` was read at).
fn frame_member_at_slot(frame: &Value, who: &Who) -> Option<usize> {
    frame["sides"][who.side]["active"][who.slot?]
        .as_u64()
        .map(|index| index as usize)
}

/// The semantic decisions one action will ask for, in engine order (BeforeMove first, then the
/// move), with the damage roll left open (`None`) to be searched.
/// What Showdown reported for one Pokémon target of an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetFate {
    Hit,
    Miss,
    /// Not affected at all: no accuracy check is made (type immunity precedes it).
    Immune,
    /// In the air / underground and out of the move's reach: missed before any check.
    Dodged,
}

/// A target of a spread move that sent it back (Magic Coat, Magic Bounce): the reflection is
/// `called[called]` of the action, and strikes `fates[back]`, each checked by the reflector with
/// its own move (R-M8-176).
#[derive(Debug, Clone)]
struct Reflection {
    reflector: PokemonId,
    called: usize,
    back: std::ops::Range<usize>,
}

/// Which rolled duration a hint is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurationKind {
    Confusion,
    Sleep,
    Bind,
    /// Taunt / Encore / Disable, rolled before Gen 5.
    Timed,
    /// Gen 1's binding lock, on its user (`partialtrappinglock`).
    BindingLock,
    /// Bide on its user: Gen 2's duration, Gen 1's time.
    Bide,
    /// Thrash and its kind on their user: the lock's length (Gen 1 uses, Gen 3-4 turns, Gen 5+
    /// hidden length).
    Rampage,
    /// Uproar on its user: the turns Gen 3-4 draw.
    Uproar,
}

/// The stages of a damaging move's decisions: for a spread move each stage covers every
/// target before the next begins (Showdown `spreadMoveHit`); otherwise one target's decisions
/// all come before the next target's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Gate,
    SubstituteDamage,
    Damage,
    /// The damage landing on the target's body (a spread move lands every target's after all
    /// of them are computed): Focus Band.
    Land,
    Effect,
    Reaction,
}

struct StagedTape {
    spread: bool,
    /// Dragon Darts split between two targets: every target's hit steps come first, then each
    /// target's darts in turn.
    gates_first: bool,
    stages: [Vec<Option<RandomOverride>>; 6],
    tape: Vec<Option<RandomOverride>>,
    held: Vec<Option<RandomOverride>>,
}

impl StagedTape {
    fn new(spread: bool) -> Self {
        Self {
            spread,
            gates_first: false,
            stages: Default::default(),
            tape: Vec::new(),
            held: Vec::new(),
        }
    }

    fn slot(&mut self, stage: Stage) -> &mut Vec<Option<RandomOverride>> {
        &mut self.stages[stage as usize]
    }

    fn push(&mut self, stage: Stage, decision: RandomOverride) {
        self.slot(stage).push(Some(decision));
    }

    /// The damage roll, left open for the search.
    fn push_roll(&mut self, stage: Stage) {
        self.slot(stage).push(None);
    }

    /// One hit (or one target) done: outside a spread move its decisions go out as they came.
    fn end_hit(&mut self) {
        if self.spread {
            return;
        }
        if self.gates_first {
            let (gate, rest) = self.stages.split_at_mut(1);
            self.tape.append(&mut gate[0]);
            for stage in rest {
                self.held.append(stage);
            }
            return;
        }
        for stage in &mut self.stages {
            self.tape.append(stage);
        }
    }

    fn finish(mut self) -> Vec<Option<RandomOverride>> {
        for stage in &mut self.stages {
            self.tape.append(stage);
        }
        self.tape.append(&mut self.held);
        self.tape
    }
}

/// Whether the move's own callbacks roll the stall counter (Showdown `runEvent('StallMove')`)
/// when its user has one: the stalling moves (`stallingMove`) whose `onPrepareHit` is the roll
/// (Protect and its variants, Endure) and, where the generation has it, Wide Guard / Quick
/// Guard's `onTry` (`ConditionRules::guards_roll_stall`). Mat Block is a stalling move without a
/// roll.
/// The engine's accuracy chance as the hit gate computes it, with the move under way
/// (`BattleState::active_move`): a move that ignores abilities (Mold Breaker's, Moongeist Beam)
/// leaves the target's breakable evasion abilities (Tangled Feet, Sand Veil …) out. The driver's
/// state is the one before the action, with no move under way.
/// Whether `actor`'s move draws an accuracy decision against `target` on `state` (no target:
/// the move's own accuracy decides).
fn accuracy_draws(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    target: Option<PokemonId>,
    move_def: &ResolvedMove,
) -> bool {
    match target {
        None => {
            !pokemon_battle_engine::engine::sure_hit(state, rules, actor, move_def)
                && move_def.accuracy.is_some_and(|accuracy| accuracy < 100)
        }
        Some(id) => accuracy_in_move(state, rules, actor, id, move_def) < 10_000,
    }
}

fn accuracy_in_move(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
) -> u32 {
    pokemon_battle_engine::engine::accuracy_basis_points(
        &move_under_way(state, rules, actor, move_def),
        rules,
        actor,
        target,
        move_def,
    )
}

/// `state` as `actor`'s move sees it: the move under way (`active_move`), so an ability the
/// move ignores (Mold Breaker, Teravolt …) answers nothing — accuracy, a critical hit, a
/// secondary's roll (Shield Dust) are asked on it (DESIGN.md R251); and the user's Protean /
/// Libero has given it the move's type (`onPrepareHit`, before the hit steps), so what the move
/// then draws (Conversion's candidates) is planned on the types it leaves (DESIGN.md R263).
fn move_under_way(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    move_def: &ResolvedMove,
) -> BattleState {
    let mut state = state.clone();
    state.active_move = Some(ActiveMove {
        user: actor,
        move_id: move_def.id,
        category: move_def.category,
        ignores_ability: move_def.ignores_ability,
        stellar_boosted: false,
        z_powered: false,
        base_move: None,
        prankster_boosted: false,
    });
    if let Some(type_) =
        pokemon_battle_engine::engine::protean_type(&state, rules, actor, move_def.move_type)
        && let Some(user) = state.pokemon_mut(actor)
    {
        user.set_types(pokemon_battle_engine::state::PokemonTypes::mono(type_));
    }
    // Explosion and its kin (`selfdestruct: 'always'`, Gen 2-9): the user is down before the hit
    // steps, and its abilities with it — an Unaware user no longer hides its target's evasion
    // from the accuracy check (R-M8-146).
    if move_def.self_destruct == Some(pokemon_battle_engine::dex::SelfDestruct::Always)
        && rules.conditions().self_faint.timing
            != pokemon_battle_engine::rules::SelfDestructTiming::AfterMove
        && let Some(user) = state.pokemon_mut(actor)
    {
        user.hp = 0;
    }
    state
}

fn rolls_stall(rules: &ResolvedRules, move_def: &ResolvedMove) -> bool {
    move_def.flags.stalling
        && pokemon_battle_engine::effects::moves::custom(move_def.id).is_some_and(|primary| {
            primary.source_callbacks.contains(&"onPrepareHit")
                || (rules.conditions().guards_roll_stall
                    && matches!(move_def.id, MoveId::WIDE_GUARD | MoveId::QUICK_GUARD))
        })
}

/// Whether the caller a called move's `[from]` names draws the move's target
/// (`engine::called_move_aims_at_random`): an ability's call is a reflection (Magic Bounce),
/// aimed back at the user; a move's, by the move (R-M8-144).
fn called_at_random(rules: &ResolvedRules, dex: &ResolvedDex, from: Option<&str>) -> bool {
    let Some(from) = from.and_then(|from| from.strip_prefix("[from] ")) else {
        return true;
    };
    if from.starts_with("ability:") {
        return false;
    }
    dex.move_by_name(from.trim_start_matches("move: "))
        .is_none_or(|caller| {
            pokemon_battle_engine::engine::called_move_aims_at_random(rules, caller.id)
        })
}

/// The priority chances an instructed use rolls as Instruct builds it (`resolveAction`'s
/// `FractionalPriority`, DESIGN.md R-M9-32): the ones `engine::fractional_priority` gives its
/// user's move, Quick Claw only while nothing went off before it; each went off when the log
/// announced it between the `-singleturn` and the move.
fn instructed_chance_tape(
    compiled: &Compiled,
    state: &BattleState,
    instructed: &ActionOracle,
) -> Vec<Option<RandomOverride>> {
    let Some(user) = who_to_id(state, &instructed.user) else {
        return Vec::new();
    };
    let category = instructed
        .move_name
        .as_deref()
        .and_then(|name| compiled.dex.move_by_name(name))
        .map(|def| def.category);
    let (mut fractional, chances) =
        pokemon_battle_engine::engine::fractional_priority(state, &compiled.rules, user, category);
    let mut rolls = Vec::new();
    for chance in chances {
        if chance == PriorityChance::QuickClaw && fractional > 0 {
            continue;
        }
        let rolled = instructed.instructed_chances.contains(&chance);
        rolls.push(Some(RandomOverride::bool(
            RandomRequestKind::PriorityChance,
            rolled,
        )));
        if rolled {
            fractional = 1;
        }
    }
    rolls
}

/// The priority chance an `-activate` announces (`item: Quick Claw`, `ability: Quick Draw`).
fn announced_chance(what: &str) -> Option<PriorityChance> {
    match what {
        "item: Quick Claw" => Some(PriorityChance::QuickClaw),
        "ability: Quick Draw" => Some(PriorityChance::QuickDraw),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn tape_template(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    actor: PokemonId,
    move_def: Option<&ResolvedMove>,
    fates: &[(Option<PokemonId>, TargetFate)],
    reflections: &[Reflection],
    oracle: &ActionOracle,
    duration_hints: &[(PokemonId, DurationKind, u16)],
    substitutes: &[(PokemonId, Option<u16>)],
) -> Vec<Option<RandomOverride>> {
    // The duration recovered for a victim; a target the replay cannot name takes the first.
    // One nothing shows (the victim was dragged out, cured or knocked out before the frame) is
    // still rolled: any legal value fits what was observed, so it takes the shortest.
    let duration_for = |target: Option<PokemonId>, class: DurationKind| {
        duration_hints
            .iter()
            .find(|(victim, c, _)| *c == class && target.is_none_or(|t| t == *victim))
            .map(|(_, _, hint)| *hint)
            .or(match class {
                DurationKind::Confusion => {
                    Some(u16::from(rules.status().lifecycle.confusion_time_min))
                }
                DurationKind::Sleep => Some(u16::from(rules.status().lifecycle.sleep_time_min)),
                DurationKind::Bind => Some(u16::from(rules.conditions().bind_turns_min)),
                DurationKind::BindingLock => Some(2),
                DurationKind::Bide => Some(u16::from(rules.conditions().retaliation.bide.turns.0)),
                DurationKind::Rampage => Some(2),
                DurationKind::Uproar => Some(u16::from(rules.conditions().uproar.turns.0)),
                DurationKind::Timed => None,
            })
    };
    // Gen 3+ Thrash's lock ending with its move: the fatigue's confusion duration.
    let fatigue_time = || {
        (oracle.fatigue && !rules.conditions().rampage.counter.counts_uses())
            .then(|| duration_for(Some(actor), DurationKind::Confusion))
            .flatten()
    };
    let mut tape = Vec::new();
    let Some(pokemon) = state.pokemon(actor) else {
        return tape;
    };
    let cant = oracle.cant.as_deref();
    // The state the move's own checks see (`move_under_way`), with the status the actor shook
    // off as its action began gone (a Gen 2-3 Beat Up counts it a member again, R-M8-134).
    let in_move_state = move_def.map(|m| {
        let mut in_move = move_under_way(state, rules, actor, m);
        if (oracle.cured.is_some() || oracle.defrosted)
            && let Some(user) = in_move.pokemon_mut(actor)
        {
            user.status = None;
        }
        in_move
    });
    let in_move = in_move_state.as_ref().unwrap_or(state);
    // The eaters whose berry draw a hit already planned.
    let mut berry_confusions_planned: Vec<Option<PokemonId>> = Vec::new();

    // Gen 3-4 Pursuit striking a Pokémon as it switches out runs without BeforeMove (§15.11):
    // Showdown's gen3 / gen4 Pursuit condition calls `useMove`, not `runMove`. Read from the
    // generation, not the engine's rule table, so that the replay checks the rule.
    let skips_before_move = oracle.intercepted
        && matches!(
            rules.generation(),
            pokemon_battle_engine::spec::Generation::Gen3
                | pokemon_battle_engine::spec::Generation::Gen4
        );
    if !skips_before_move {
        // 0. The recharge turn comes before any status check (`mustrecharge` `onBeforeMovePriority`
        //    11 over `frz`'s 10; Gen 1's freeze never thaws on its own): it asks nothing. Gen 3-4
        //    on the cartridge check it after sleep and freeze (issues.md I66): step 2.
        if cant == Some("recharge")
            && rules.conditions().recharge_check
                != pokemon_battle_engine::rules::RechargeCheck::AfterSleepAndFreeze
        {
            return tape;
        }
        // 1. Major status. Sleep asks nothing; freeze asks Thaw. Waking with no move and no other
        //    reason (Gen 1) spends the turn; a Pokémon that wakes then meets its confusion check.
        if let Some(status) = pokemon.status {
            match status.kind {
                NonVolatileStatus::Sleep => {
                    if cant == Some("slp")
                        || (oracle.cured.as_deref() == Some("slp")
                            && oracle.move_name.is_none()
                            && cant.is_none())
                    {
                        return tape;
                    }
                }
                NonVolatileStatus::Freeze => {
                    let thawed = oracle.cured.as_deref() == Some("frz") && !oracle.defrosted;
                    // The engine says whether a thaw is rolled here (Gen 2: in the residual; a
                    // `defrost` move from Gen 5: no roll).
                    let rolls = match move_def {
                        Some(m) => {
                            pokemon_battle_engine::engine::freeze_thaw_rolls(state, rules, actor, m)
                        }
                        None => {
                            rules.status().lifecycle.thaw_timing
                                == pokemon_battle_engine::rules::ThawTiming::BeforeMove
                        }
                    };
                    if rolls {
                        tape.push(Some(RandomOverride::bool(RandomRequestKind::Thaw, thawed)));
                    }
                    let defrost_moves = move_def.is_some_and(|m| {
                        pokemon_battle_engine::engine::defrost_move_for(state, rules, actor, m)
                    }) && rules.status().lifecycle.defrost_user
                        != pokemon_battle_engine::rules::DefrostUser::Never;
                    if !thawed && !defrost_moves {
                        return tape;
                    }
                }
                _ => {}
            }
        }
        // 2. Flinch asks nothing; neither do the recharge turn, Disable, Taunt or a Choice lock
        //    (`cant … recharge`, `cant … Disable`, `cant … move: Taunt`). A move with no PP left
        //    (`cant … nopp`) passed them all first.
        // An ability that stops the move at TryMove (Damp) comes after BeforeMove and after a
        // call (R-M8-70), and so does Powder blowing up a Fire move (a Metronome's Heat Wave,
        // R-M8-108): below.
        if cant.is_some_and(|reason| {
            !matches!(
                reason,
                "par" | "confusion" | "slp" | "frz" | "Attract" | "nopp" | "move: Powder"
            ) && !TRY_MOVE_STOPPERS.contains(&reason)
        }) {
            return tape;
        }
        // 3. Confusion: after the tick, a remaining counter rolls for a self-hit.
        if pokemon
            .volatiles
            .confusion_time()
            .is_some_and(|time| time > 1)
        {
            let self_hit = cant == Some("confusion");
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::ConfusionSelfHit,
                self_hit,
            )));
            if self_hit {
                if rules.status().lifecycle.confusion_self_hit_varies {
                    tape.push(None);
                }
                // Gen 1-2's self-hit is a `directDamage` no Focus Band sees (read from the
                // generation, so that the replay checks `confusion_self_hit_direct`).
                if let Some(decision) = self_focus_band(state, rules, actor, oracle)
                    && rules.generation() >= Generation::Gen3
                {
                    tape.push(Some(decision));
                }
                // A berry the self-hit's `Update` fed it: Starf Berry's stat (DESIGN.md R281).
                for (who, _, draw) in &oracle.berry_draws {
                    if who_to_id(state, who) == Some(actor)
                        && let Some(decision) = berry_decision(*draw, || {
                            duration_for(Some(actor), DurationKind::Confusion)
                        })
                    {
                        tape.push(Some(decision));
                    }
                }
                return tape;
            }
        }
        // 3a. Attract: an infatuated Pokémon draws before paralysis (Gen 2: after it); `cant …
        //     Attract` is the one that took.
        let infatuated = pokemon.volatiles.contains(VolatileConditionKind::Attract);
        let late_attract = rules.conditions().attract_after_paralysis;
        let love = cant == Some("Attract");
        if infatuated && !late_attract {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::Infatuation,
                love,
            )));
            if love {
                return tape;
            }
        }
        // 4. Full paralysis.
        if pokemon_battle_engine::engine::rolls_full_paralysis(state, rules, pokemon) {
            let fully = cant == Some("par");
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::FullParalysis,
                fully,
            )));
            if fully {
                return tape;
            }
        }
        if infatuated && late_attract {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::Infatuation,
                love,
            )));
            if love {
                return tape;
            }
        }
    }
    // The moves this action called, one after another (Metronome, or Copycat copying a
    // Metronome, §42.1): each caller's draw among its candidates, in the callers' order, whose
    // place the log's called move names.
    tape.extend(called_draws(state, rules, dex, actor, oracle));
    // The called move's target: a caller that names none (Metronome, Assist, Copycat …:
    // Showdown `useMove` without a target, `getRandomTarget`) has it drawn among the adjacent
    // foes, after the draw that picked the move; the `move` line names the one it went to. A
    // caller that names one (a reflection, Me First, Gen 5+ Mirror Move) draws nothing
    // (`engine::called_move_aims_at_random`): an entry planned for it anyway was taken by the
    // next RandomTarget the action drew — a Roar it called dragging in another Pokémon
    // (R-M8-144). The entry stays optional (DESIGN.md R-M8-55). The draw is the last move the
    // action's own user was called to use: a reflection after it (another user's, §42.8) aims
    // where it was told, and its line names another target (R-M8-67). A snatched move is the
    // snatcher's, and Snatch names no target either: the snatcher draws it among its own
    // adjacent foes (Gen 3-4 Psych Up, R-M9-33). The draw is for the move as called, before
    // `ModifyMove` (the dex's: a Metronome's Expanding Force aims at one foe as it is drawn,
    // all of them only once Psychic Terrain spreads it, R-M10-56).
    if let Some(called) = oracle.called.iter().rev().find(|called| {
        called.user == oracle.user || called.from.as_deref() == Some("[from] Snatch")
    }) && called_at_random(rules, dex, called.from.as_deref())
        && let Some(def) = move_def
        && let Some(user) = who_to_id(state, &called.user)
        && let Some(candidates) = pokemon_battle_engine::engine::random_retarget_candidates(
            state,
            rules,
            user,
            dex.move_by_id(def.id).unwrap_or(def),
            None,
        )
        && candidates.len() > 1
    {
        let picked = drawn_target(
            state,
            &candidates,
            called.target.as_ref(),
            called.redirected_by.as_ref(),
        )
        .unwrap_or(0);
        tape.push(Some(
            RandomOverride::range(RandomRequestKind::RandomTarget, picked as u16).optional(),
        ));
    }
    // A move with no PP left as its turn came (§15.15) stops right after BeforeMove; one an
    // ability or Powder stops at TryMove (Damp: a called Explosion too) after its call.
    if cant.is_some_and(|reason| {
        reason == "nopp" || reason == "move: Powder" || TRY_MOVE_STOPPERS.contains(&reason)
    }) {
        return tape;
    }
    // Snatch (`onAnyPrepareHit`): the holders sharing a handler speed draw a SpeedTie each, in
    // party order (`snatch_steals`); the one whose steal took the move — the first called move's
    // other user — draws the larger (DESIGN.md R-M8-93).
    if let Some(def) = move_def
        && def.flags.snatch
        && let Some(snatcher) = oracle
            .called
            .iter()
            .find(|called| called.user != oracle.user)
            .and_then(|called| who_to_id(state, &called.user))
    {
        let holders: Vec<(PokemonId, u32)> = state
            .all_pokemon()
            .iter()
            .filter(|pokemon| {
                pokemon.id() != actor
                    && !pokemon.is_fainted()
                    && state
                        .active_position(pokemon.id(), rules.topology())
                        .is_some()
                    && pokemon
                        .volatiles
                        .contains(pokemon_battle_engine::state::VolatileConditionKind::Snatch)
            })
            .map(|pokemon| (pokemon.id(), state.handler_speed(pokemon.id())))
            .collect();
        if holders.iter().any(|(id, _)| *id == snatcher) {
            for &(id, speed) in &holders {
                if holders.iter().filter(|(_, other)| *other == speed).count() > 1 {
                    let rank = if id == snatcher { 2 } else { 1 };
                    tape.push(Some(
                        RandomOverride::raw(RandomRequestKind::SpeedTie, rank).optional(),
                    ));
                }
            }
        }
    }
    // A reflection (the last called move another Pokémon used, §42.8): where the generation
    // rolls accuracy before TryHit (Gen 3-4, `HitStepOrder`), the move it sends back rolled its
    // accuracy at the reflector first, and hit (R-M8-69).
    // A spread move some of whose targets reflected it (`reflections`): the user's check at each
    // reflector comes in its turn among the targets, right before the reflection's (the status
    // moves' accuracy loop, R-M8-173, R-M8-176).
    let accuracy_before_try_hit = matches!(
        rules.hits().step_order,
        HitStepOrder::AccuracyFirst | HitStepOrder::AccuracyBeforeTryHit
    );
    let check_at = |reflector: PokemonId| -> Option<Option<RandomOverride>> {
        let def = move_def?;
        (accuracy_before_try_hit && accuracy_in_move(state, rules, actor, reflector, def) < 10_000)
            .then_some(Some(RandomOverride::bool(
                RandomRequestKind::Accuracy,
                true,
            )))
    };
    if reflections.is_empty()
        && let Some(reflection) = oracle
            .called
            .last()
            .filter(|called| called.user != oracle.user)
        && let Some(reflector) = who_to_id(state, &reflection.user)
        && let Some(check) = check_at(reflector)
    {
        tape.push(check);
    }
    // Each reflection strikes with the move as its reflector's (`modified_move`, as below).
    let reflected_moves: Vec<Option<ResolvedMove>> = reflections
        .iter()
        .map(|reflection| {
            move_def.and_then(|def| dex.move_by_id(def.id)).map(|own| {
                pokemon_battle_engine::engine::modified_move(
                    state,
                    rules,
                    reflection.reflector,
                    own,
                    false,
                )
                .0
            })
        })
        .collect();
    let user = actor;
    let own_move_def = move_def;
    // A move another Pokémon took over (a reflection, a snatch: the last called move has another
    // user) is that Pokémon's: its accuracy and the rest read its user's stages (R-M8-81), and
    // the move is the new user's (Showdown `getActiveMove(move.id)` that its own `ModifyMove`
    // shapes: a Turboblaze reflector's move ignores the target's Wonder Skin, R-M8-92).
    let taker = oracle
        .called
        .last()
        .filter(|called| called.user != oracle.user)
        .and_then(|called| who_to_id(state, &called.user));
    let taken_over = taker.zip(move_def).and_then(|(taker, def)| {
        dex.move_by_id(def.id).map(|own| {
            pokemon_battle_engine::engine::modified_move(state, rules, taker, own, false).0
        })
    });
    let move_def = taken_over.as_ref().or(move_def);
    let actor = taker.unwrap_or(actor);
    // 5. The move itself. A charging turn (Showdown `-prepare`) asks nothing more, nor does a
    //    damaging move that failed before hitting (`-fail|user`).
    // Bide's later turns (§15.10): storing asks nothing; the release draws its random target
    // when the Pokémon that damaged the holder last is gone (Gen 1: always), then — in Gen 2-3,
    // through the hit steps — its accuracy.
    if let Some(bide) = pokemon.volatiles.bide().copied() {
        let bide_rules = rules.conditions().retaliation.bide;
        let releases = if bide_rules.gen1 {
            bide.time <= 1
        } else {
            bide.turns == Some(1)
        };
        let stored = if bide_rules.gen1 {
            bide.damage + u32::from(state.last_damage)
        } else {
            bide.damage
        };
        if !releases || stored == 0 || (!bide_rules.gen1 && bide.source.is_none()) {
            return tape;
        }
        let Some(mut release) = dex.move_by_id(MoveId::BIDE).cloned() else {
            return tape;
        };
        release.target = pokemon_battle_engine::rules::TargetPattern::SelectedAdjacent;
        // Whom the release went at: the Pokémon it damaged, missed, or that was immune to it
        // (Wonder Guard on a Gen 3 target the random retarget drew, R-M8-105) — or whose
        // Substitute took it, no HP commit (R-M10-3).
        let struck = oracle
            .damages
            .iter()
            .map(|(who, _)| who)
            .chain(oracle.missed.iter())
            .chain(oracle.immune.iter())
            .chain(oracle.hit_targets.iter().flatten())
            .find_map(|who| who_to_id(state, who));
        let source = bide.source.filter(|source| {
            !bide_rules.gen1
                && state.active_position(*source, rules.topology()).is_some()
                && state.pokemon(*source).is_some_and(|p| !p.is_fainted())
        });
        let target = match source {
            Some(source) => Some(source),
            None => {
                let candidates = pokemon_battle_engine::engine::random_retarget_candidates(
                    state, rules, actor, &release, None,
                )
                .unwrap_or_default();
                if candidates.len() > 1 {
                    let index = struck
                        .and_then(|target| candidates.iter().position(|c| *c == target))
                        .unwrap_or(0);
                    tape.push(Some(RandomOverride::range(
                        RandomRequestKind::RandomTarget,
                        index as u16,
                    )));
                }
                struck.or_else(|| candidates.first().copied())
            }
        };
        // (A type immunity stops the release before its accuracy check, and so does a target
        // out of reach — underwater, underground: the invulnerability step, R-M10-49; Gen 2
        // reports a miss with no target.)
        if bide_rules.release_hit_steps
            && oracle.immune.is_empty()
            && let Some(target) = target
            && two_turn::dodged_by(state, rules, actor, target, &release).is_none()
            && accuracy_in_move(state, rules, actor, target, &release) < 10_000
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::Accuracy,
                oracle.hit,
            )));
        }
        // The release is a contact hit of fixed damage (no critical hit, no roll): the struck
        // Pokémon's reactions to it roll as after any hit — Static and its kind, then Effect
        // Spore, Cute Charm … (R190). Not off a Substitute.
        if oracle.hit
            && oracle.immune.is_empty()
            && let Some(target) = target
            && state
                .pokemon(target)
                .is_some_and(|p| !p.volatiles.contains(VolatileConditionKind::Substitute))
        {
            let mut staged = StagedTape::new(false);
            // Focus Band against the release, as against any hit (R196, I38): Gen 3-4 roll it
            // on the hit; elsewhere only a release that would knock the holder out does.
            if pokemon_battle_engine::engine::held_item_id(state, rules, target)
                == Some(pokemon_battle_engine::ids::ItemId::FOCUS_BAND)
            {
                let held = oracle
                    .focus_band_held
                    .iter()
                    .any(|(who, hits)| *hits == 0 && who_to_id(state, who) == Some(target));
                let decision = RandomOverride::bool(RandomRequestKind::FocusBand, held);
                if matches!(rules.generation(), Generation::Gen3 | Generation::Gen4) {
                    staged.push(Stage::Land, decision);
                } else if held || oracle.hit_hp.first() == Some(&Some(0)) {
                    staged.push(Stage::Land, decision.optional());
                }
            }
            if release.flags.contact
                && state.pokemon(target).is_some_and(|p| {
                    pokemon_battle_engine::engine::ability_handlers(state, rules, p)
                        .contact_status
                        .is_some()
                })
            {
                let took = oracle.contact_status.is_some()
                    && oracle
                        .contact_status_of
                        .as_ref()
                        .is_none_or(|who| who_to_id(state, who) == Some(target));
                staged.push(
                    Stage::Reaction,
                    RandomOverride::bool(RandomRequestKind::ContactReaction, took),
                );
            }
            chance_reactions(
                &mut staged,
                ChanceReactionHit {
                    state: in_move,
                    rules,
                    actor,
                    target,
                    move_def: &release,
                    oracle,
                    behind_substitute: false,
                    dealt: oracle.hit_hp.first().copied().flatten().map_or(1, |after| {
                        state
                            .pokemon(target)
                            .map_or(after, |p| p.hp)
                            .saturating_sub(after)
                    }),
                    reported_after: &|_| true,
                    sleep_duration: duration_for(Some(actor), DurationKind::Sleep),
                    user_keeps_ability: true,
                },
                &mut false,
            );
            // A berry the hit's `Update` fed the struck Pokémon (or the holder, hurt by its
            // reaction): Starf Berry's stat, a confusion berry's duration.
            for (who, _, draw) in &oracle.berry_draws {
                let eater = who_to_id(state, who);
                if (eater == Some(target) || eater == Some(actor))
                    && let Some(decision) =
                        berry_decision(*draw, || duration_for(eater, DurationKind::Confusion))
                {
                    staged.push(Stage::Reaction, decision);
                }
            }
            tape.extend(staged.finish());
        }
        return tape;
    }
    if let Some(move_def) = move_def {
        // A power drawn as the move starts (Magnitude), announced before it hits.
        // Shell Side Arm's coin flip on a tie, as its hint names the category — not with
        // nobody to aim at (`[notarget]`: its `onModifyMove` returns without a target, R-M8-155).
        if move_def.behavior.category_by_estimate
            && !oracle.no_target
            && let Some(target) = fates.first().and_then(|(target, _)| *target)
            && pokemon_battle_engine::engine::physical_by_estimate(
                state, rules, actor, target, move_def,
            )
            .is_none()
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::CategoryTie,
                oracle.physical_hint == Some(true),
            )));
        }
        if let Some(random) = move_def.behavior.random_power {
            tape.push(Some(RandomOverride::range(
                RandomRequestKind::Sample,
                drawn_value(random, oracle, behind_substitute(state, fates)),
            )));
        }
        // Gen 1-2 Thrash's lock as the move starts (§15.12): its uses drawn on the first, the
        // fatigue's confusion on the last.
        if move_def.behavior.rampage && rules.conditions().rampage.counter.counts_uses() {
            if pokemon.volatiles.rampage().is_none() {
                if let Some(uses) = duration_for(Some(actor), DurationKind::Rampage) {
                    tape.push(Some(RandomOverride::range(
                        RandomRequestKind::Duration,
                        uses,
                    )));
                }
            } else if oracle.fatigue
                && let Some(time) = duration_for(Some(actor), DurationKind::Confusion)
            {
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::Duration,
                    time,
                )));
            }
        }
        if two_turn::will_charge(state, rules, actor, move_def) {
            return tape;
        }
        if (oracle.failed && move_def.category != MoveCategory::Status) || oracle.no_target {
            // A lock's turn with nobody to aim at (`[notarget]`) or that failed still ends the
            // lock as the move is over (Gen 5+ `onAfterMove`): its fatigue's confusion.
            if let Some(time) = fatigue_time() {
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::Duration,
                    time,
                )));
            }
            return tape;
        }
        // A move its `onTry` stops (Dark Void for anyone but Darkrai) draws nothing more — as
        // the user is once its BeforeMove is over: one that woke up there no longer snores; nor
        // does one a Gen 3-4 uproar wakes there (the cartridge's `CANCELLER_ASLEEP`, its
        // `-curestatus` printed with the uproar's start, R-M8-160).
        let uproar_woke =
            pokemon_battle_engine::engine::uproar_wakes_before_move(state, rules, actor);
        let woke = (oracle.cured.as_deref() == Some("slp") || uproar_woke).then(|| {
            let mut woke = in_move.clone();
            if let Some(pokemon) = woke.pokemon_mut(actor) {
                pokemon.status = None;
            }
            woke
        });
        // The `onTry` runs once the action spent its PP, which marked the chosen move's slot
        // used (`deductPP`): Copycat's Last Resort counts Copycat as used (R-M8-167).
        let mut tried = woke.clone().unwrap_or_else(|| state.clone());
        if let Some(chosen) = oracle
            .move_name
            .as_deref()
            .and_then(|name| dex.move_by_name(name))
            && let Some(user) = tried.pokemon_mut(actor)
        {
            for slot in user.move_slots_mut() {
                if slot.move_id == chosen.id {
                    slot.used = true;
                }
            }
        }
        if let Some(try_use) = move_def.behavior.try_use
            && !pokemon_battle_engine::engine::try_use_holds(
                &tried,
                rules,
                actor,
                fates.first().and_then(|(target, _)| *target),
                try_use,
            )
        {
            return tape;
        }
        // Sky Drop's carrying turn (§15.23): sure to hit, no damage — nothing to draw.
        if move_def.behavior.sky_drop && !pokemon.volatiles.contains(VolatileConditionKind::SkyDrop)
        {
            return tape;
        }
        // Future Sight, Doom Desire (§15.22): the use only foresees; Gen 2-4 compute the damage
        // then (planned below from the strike's), Gen 5+ at the strike (`residual_tape`).
        if move_def.behavior.future_attack {
            // Gen 4 draws the computed damage's random factor at use: an open roll, settled
            // from the strike's damage (`future_damage_hint`).
            if rules.conditions().future_attack.damage_at_use
                == Some(pokemon_battle_engine::rules::ForeseenRoll::AtUse)
                && !oracle.failed
            {
                tape.push(None);
            }
            return tape;
        }
        // Whether an accuracy decision is drawn against a target: the engine's own chance with
        // the current accuracy / evasion stages (Muddy Water's accuracy drop makes a 100 %
        // move miss-able), not the move's base accuracy.
        let rolls_accuracy = |target: Option<PokemonId>| match target {
            Some(id) => accuracy_in_move(state, rules, actor, id, move_def) < 10_000,
            None => {
                !pokemon_battle_engine::engine::sure_hit(state, rules, actor, move_def)
                    && move_def.accuracy.is_some_and(|accuracy| accuracy < 100)
            }
        };
        // A `multiaccuracy` move's checks after the first: on the state the first check left
        // (Micle Berry's boost and a Lock-On on the target used up, `engine::spend_accuracy_check`).
        let rolls_later_accuracy = |target: Option<PokemonId>| match target {
            Some(id) => {
                let mut after = state.clone();
                pokemon_battle_engine::engine::spend_accuracy_check(
                    &mut after, rules, actor, id, move_def,
                );
                accuracy_in_move(&after, rules, actor, id, move_def) < 10_000
            }
            None => rolls_accuracy(None),
        };
        // A target that did not take the move (`Immune`: a type immunity, or a TryHit block such
        // as Protect or an absorbing ability) may still have drawn an accuracy decision, by the
        // generation's hit-step order (engine `HitStepOrder`): Gen 4 rolls accuracy before
        // TryHit, Gen 3 even before reporting a type immunity (a hit then meets TryHit, so a
        // TryHit block means the roll hit). Returns the decision's value.
        let accuracy_before_block = |target: Option<PokemonId>| -> Option<bool> {
            let id = target?;
            if pokemon_battle_engine::engine::ohko_blocked(state, rules, actor, id, move_def)
                || !rolls_accuracy(target)
            {
                return None;
            }
            // Gen 7+'s Dark-type immunity to a Prankster status move is the type check's too.
            let type_immune =
                pokemon_battle_engine::engine::type_immunity(state, rules, id, move_def)
                    .1
                    .is_some()
                    || pokemon_battle_engine::engine::prankster_immune(
                        state,
                        rules,
                        actor,
                        id,
                        move_def.category == MoveCategory::Status,
                    );
            // A type-immune target reports `-immune` whether the roll hit or missed; one that
            // TryHit stopped (Volt Absorb on a Ground type) was hit.
            let stopped_at_try_hit = oracle
                .try_hit_blocked
                .iter()
                .any(|who| who_to_id(state, who) == Some(id));
            // A status move's own status immunity (Poison Gas on a Steel type; Toxic Thread's
            // after its Speed drop) only shows when the status lands, after the accuracy check —
            // unless the gate stopped it first (a Grass type against a powder move from Gen 6).
            let powder_blocked = move_def.flags.powder
                && pokemon_battle_engine::engine::powder_immunity(state, rules, id).is_some();
            if !type_immune
                && !powder_blocked
                && let PrimaryEffect::Status(
                    StatusMoveEffect::SetStatus(status)
                    | StatusMoveEffect::StagesThen(_, FollowUp::Status(status)),
                ) = move_def.primary_effect
                && ((!stopped_at_try_hit
                    && pokemon_battle_engine::engine::status_type_immune(state, rules, id, status))
                    // Or an ability refusing the status as it is set (Limber, Pastel Veil):
                    // its `-immune … [from] ability:` comes once the move landed (R277) —
                    // unless a guard (Protect, Max Guard) stopped the move first (R-M8-109).
                    || (!oracle
                        .guarded
                        .iter()
                        .any(|who| who_to_id(state, who) == Some(id))
                        && pokemon_battle_engine::engine::ability_refuses_status(
                            state,
                            rules,
                            id,
                            status,
                            pokemon_battle_engine::engine::EffectSource::Move {
                                pokemon: actor,
                                move_id: move_def.id,
                            },
                        )))
            {
                return Some(true);
            }
            // Gen 3 reports the move's own immunity (Dream Eater on an awake target) with the
            // type's; Gen 4 checks it right after a hit.
            let move_immune =
                pokemon_battle_engine::engine::move_immune(state, rules, actor, id, move_def);
            match rules.hits().step_order {
                // Gen 3 Rage against a type immunity: its flag is the roll's (§15.19).
                HitStepOrder::AccuracyFirst
                    if type_immune
                        && !stopped_at_try_hit
                        && move_def.behavior.rage
                        && rules.conditions().rage.accuracy_sets =>
                {
                    Some(oracle.raging_after.unwrap_or(false))
                }
                HitStepOrder::AccuracyFirst => {
                    Some(!(type_immune || move_immune) || stopped_at_try_hit)
                }
                HitStepOrder::AccuracyBeforeTryHit if !type_immune => Some(true),
                _ => None,
            }
        };
        let targets: Vec<(Option<PokemonId>, TargetFate)> = if fates.is_empty() {
            vec![(
                None,
                if oracle.hit {
                    TargetFate::Hit
                } else {
                    TargetFate::Miss
                },
            )]
        } else {
            fates.to_vec()
        };
        // A status move checks accuracy once per Pokémon target (immune targets ask nothing),
        // and sleep, confusion, Taunt and Encore roll their duration on each target they land
        // on, right after its check.
        if move_def.primary_effect != PrimaryEffect::Damage {
            let rolled = |rule: pokemon_battle_engine::rules::TimedVolatileRules| {
                matches!(
                    rule.duration,
                    pokemon_battle_engine::rules::TimedDuration::Rolled { .. }
                )
            };
            let duration_roll = match &move_def.primary_effect {
                PrimaryEffect::Status(StatusMoveEffect::SetStatus(NonVolatileStatus::Sleep)) => {
                    Some(("status:slp", DurationKind::Sleep))
                }
                PrimaryEffect::Status(
                    StatusMoveEffect::Volatile(VolatileConditionKind::Confusion)
                    | StatusMoveEffect::StagesThen(
                        _,
                        FollowUp::Volatile(VolatileConditionKind::Confusion),
                    ),
                ) => Some(("start:confusion", DurationKind::Confusion)),
                // Gen 3-4 roll Taunt / Encore durations when they take.
                PrimaryEffect::Status(StatusMoveEffect::Volatile(VolatileConditionKind::Taunt))
                    if rolled(rules.conditions().taunt) =>
                {
                    Some(("start:move: Taunt", DurationKind::Timed))
                }
                PrimaryEffect::Status(StatusMoveEffect::Volatile(
                    VolatileConditionKind::Encore,
                )) if rolled(rules.conditions().encore) => {
                    Some(("start:Encore", DurationKind::Timed))
                }
                _ => None,
            };
            // A spread move checks each target's accuracy in turn on the state the earlier ones
            // left: the first check spends the user's Micle Berry, so a later target rolls the
            // plain accuracy (Showdown `hitStepAccuracy`'s loop, DESIGN.md R-M8-44).
            let mut accuracy_state = state.clone();
            for (index, (target, fate)) in targets.iter().enumerate() {
                // A spread move some of its targets sent back: the targets the move strikes
                // itself are the user's, checked by it with its move; a reflection's are its
                // reflector's, with the reflector's move, after the user's check at the reflector
                // (R-M8-173), each reported in its own stretch of the action (R-M8-176).
                let reflection = reflections
                    .iter()
                    .position(|reflection| reflection.back.contains(&index));
                if let Some(at) = reflection
                    && reflections[at].back.start == index
                    && let Some(check) = check_at(reflections[at].reflector)
                {
                    tape.push(check);
                }
                let (actor, move_def) = match reflection {
                    Some(at) => (
                        reflections[at].reflector,
                        reflected_moves[at].as_ref().unwrap_or(move_def),
                    ),
                    None if !reflections.is_empty() => (user, own_move_def.unwrap_or(move_def)),
                    None => (actor, move_def),
                };
                let applied = match reflection {
                    Some(at) => called_outcomes(oracle, reflections[at].called).applied,
                    None => &oracle.applied[..],
                };
                if *fate == TargetFate::Immune {
                    if let Some(hit) = accuracy_before_block(*target) {
                        tape.push(Some(RandomOverride::bool(RandomRequestKind::Accuracy, hit)));
                    }
                } else if *fate != TargetFate::Dodged
                    && match target {
                        // No target the replay can name: the move's own accuracy decides (a
                        // sure-hit move and one of 100 never draw).
                        None => {
                            !pokemon_battle_engine::engine::sure_hit(
                                &accuracy_state,
                                rules,
                                actor,
                                move_def,
                            ) && move_def.accuracy.is_some_and(|accuracy| accuracy < 100)
                        }
                        Some(id) => {
                            accuracy_in_move(&accuracy_state, rules, actor, *id, move_def) < 10_000
                        }
                    }
                {
                    tape.push(Some(RandomOverride::bool(
                        RandomRequestKind::Accuracy,
                        *fate == TargetFate::Hit,
                    )));
                }
                if let Some(id) = target
                    && matches!(*fate, TargetFate::Hit | TargetFate::Miss)
                {
                    pokemon_battle_engine::engine::spend_accuracy_check(
                        &mut accuracy_state,
                        rules,
                        actor,
                        *id,
                        move_def,
                    );
                }
                if *fate == TargetFate::Hit
                    && let Some((label, class)) = duration_roll
                    && applied.iter().any(|(who, kind)| {
                        kind == label && target.is_none_or(|id| who_to_id(state, who) == Some(id))
                    })
                    && let Some(duration) = duration_for(*target, class)
                {
                    tape.push(Some(RandomOverride::range(
                        RandomRequestKind::Duration,
                        duration,
                    )));
                }
                // Poison Puppeteer: the poison confuses the target too, rolling its duration —
                // only the holder's (a confusion berry the poison fed it rolls its own, R-M9-35).
                if *fate == TargetFate::Hit
                    && pokemon_battle_engine::engine::poison_puppeteer_confuses(state, rules, actor)
                    && matches!(
                        move_def.primary_effect,
                        PrimaryEffect::Status(StatusMoveEffect::SetStatus(
                            NonVolatileStatus::Poison | NonVolatileStatus::Toxic
                        ))
                    )
                    && applied.iter().any(|(who, kind)| {
                        kind == "start:confusion"
                            && target.is_none_or(|id| who_to_id(state, who) == Some(id))
                    })
                    && let Some(duration) = duration_for(*target, DurationKind::Confusion)
                {
                    tape.push(Some(RandomOverride::range(
                        RandomRequestKind::Duration,
                        duration,
                    )));
                }
            }
        }
        // Gen 1 Mimic draws among the target's move slots as it learns (`this.sample(target.moves)`,
        // `copyable`): the position of the move it learned, read off the log's `-start|…|Mimic|…`
        // line. The draw comes after the move's own accuracy check (Gen 1 rolls the byte even for
        // a move of 100), and a Mimic that failed draws nothing — the entry is optional, like any
        // decision an action that never got there does not ask for.
        // The move the action actually ends up using: Mimic itself, or the Mimic a caller
        // (Mirror Move, Metronome) called — both draw it.
        let innermost_move = oracle
            .called
            .last()
            .map(|called| called.name.as_str())
            .or(oracle.move_name.as_deref());
        if innermost_move == Some("Mimic")
            && rules.conditions().calls.mimic_draws_from_the_slots
            && let Some(learned) = oracle.mimicked.as_deref()
            && let Some(target) = oracle
                .move_target
                .as_ref()
                .and_then(|who| who_to_id(state, who))
            && let Some(target_state) = state.pokemon(target)
            && let Some(learned_def) = dex.moves().iter().find(|def| def.name == learned)
            && let Some(index) = target_state
                .move_slots()
                .iter()
                .position(|slot| slot.move_id == learned_def.id)
        {
            tape.push(Some(
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ));
        }
        // Gen 1 Disable picks one of the target's moves with PP (drawn when there is a choice),
        // then its `time`.
        if move_def.id == MoveId::DISABLE
            && rules.conditions().disable_gen1
            && let Some(sealed) = oracle.disabled_move.as_deref()
            && let Some(victim) = targets
                .first()
                .and_then(|(target, _)| *target)
                .and_then(|id| state.pokemon(id))
        {
            let candidates: Vec<(usize, &str)> = victim
                .move_slots()
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.pp > 0)
                .filter_map(|(index, slot)| {
                    dex.move_by_id(slot.move_id)
                        .map(|m| (index, m.name.as_str()))
                })
                .collect();
            if candidates.len() > 1 {
                // The slot it drew where the oracle names it (a Mimic's copy beside the
                // original, R-M8-161); otherwise the first slot holding the move.
                let index = oracle
                    .disabled_slot
                    .and_then(|drawn| candidates.iter().position(|(slot, _)| *slot == drawn))
                    .or_else(|| candidates.iter().position(|(_, name)| *name == sealed))
                    .unwrap_or(0);
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::DisableSlot,
                    index as u16,
                )));
            }
            if let Some(duration) = duration_for(Some(victim.id()), DurationKind::Timed) {
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::Duration,
                    duration,
                )));
            }
        }
        // Gen 2-3 Spite (§15.15): the PP it took, drawn once its checks passed — the number its
        // `-activate` shows.
        if move_def.id == MoveId::SPITE
            && matches!(
                rules.conditions().pp.spite.amount,
                pokemon_battle_engine::rules::SpiteAmount::Rolled { .. }
            )
            && let Some(drop) = oracle.spite_drop
        {
            tape.push(Some(RandomOverride::range(RandomRequestKind::PpDrop, drop)));
        }
        // Disable (a Custom move) rolls its duration when it takes, before Gen 5.
        if move_def.id == MoveId::DISABLE
            && !rules.conditions().disable_gen1
            && oracle
                .applied
                .iter()
                .any(|(_, kind)| kind.starts_with("start:Disable"))
            && matches!(
                rules.conditions().disable.duration,
                pokemon_battle_engine::rules::TimedDuration::Rolled { .. }
            )
            && let Some(duration) = duration_for(
                targets.first().and_then(|(target, _)| *target),
                DurationKind::Timed,
            )
        {
            tape.push(Some(RandomOverride::range(
                RandomRequestKind::Duration,
                duration,
            )));
        }
        // Bide (a Custom move) as it starts: Gen 2's duration, Gen 1's time (§15.10).
        let bide_turns = rules.conditions().retaliation.bide.turns;
        if move_def.id == MoveId::BIDE
            && bide_turns.0 != bide_turns.1
            && oracle
                .applied
                .iter()
                .any(|(_, kind)| matches!(kind.as_str(), "start:move: Bide" | "start:Bide"))
            && let Some(duration) = duration_for(Some(actor), DurationKind::Bide)
        {
            tape.push(Some(RandomOverride::range(
                RandomRequestKind::Duration,
                duration,
            )));
        }
        // Acupressure: the stat `sample` drew (R166) — on its snatcher, when another Pokémon
        // took it over (Snatch uses it anew, at its own default target: itself).
        if move_def.id == MoveId::ACUPRESSURE
            && let Some(victim) = taker.or_else(|| {
                targets.first().and_then(|(target, _)| *target).or_else(|| {
                    oracle
                        .move_target
                        .as_ref()
                        .and_then(|who| who_to_id(state, who))
                })
            })
        {
            tape.extend(acupressure_tape(state, victim, oracle));
        }
        // Conversion, Conversion 2: the type `sample` drew (R254) — once the move reached its
        // `onHit`: not on a target that dodged, was missed or was not affected (Good as Gold,
        // Protect; DESIGN.md R263).
        if matches!(move_def.id, MoveId::CONVERSION | MoveId::CONVERSION_2)
            && targets
                .first()
                .is_none_or(|(_, fate)| *fate == TargetFate::Hit)
        {
            let target = targets.first().and_then(|(target, _)| *target).or_else(|| {
                oracle
                    .move_target
                    .as_ref()
                    .and_then(|who| who_to_id(state, who))
            });
            tape.extend(conversion_tape(
                in_move,
                rules,
                dex,
                actor,
                target,
                move_def.id,
                oracle,
            ));
        }
        // Gen 9 Ally Switch (`onPrepareHit`): 1 / its own counter while one stands, rolled
        // before anything else the move does. A failed roll is `PrepareHit`'s bare `-fail`; a
        // roll that went through can still fail the swap itself (no partner: a single battle),
        // which names the move (`-fail|p|move: Ally Switch`, R-M8-58).
        if rules.conditions().calls.ally_switch_consecutive_chance
            && move_def.id == MoveId::ALLY_SWITCH
            && pokemon.volatiles.ally_switch_counter().is_some()
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::ProtectSuccess,
                !oracle.failed || oracle.fail_named_the_move,
            )));
        }
        // A stalling move right after a use (Protect and its variants, Endure; Gen 5's guards —
        // `rolls_stall`): 1 / stall counter, only rolled when somebody else still acts
        // this turn (otherwise the move just fails).
        // The roll held when the move went on: its `-singleturn`, or — a second use in the
        // turn (Instruct), the protection already up — no `-fail` (R-M8-84).
        // An instructed use (`queue.willAct()`: any action left) counts its user's own action
        // still to come (R-M8-149).
        if rolls_stall(rules, move_def)
            && pokemon.volatiles.stall_counter().is_some()
            && state
                .pending_movers
                .iter()
                .any(|mover| *mover != actor || oracle.instructed)
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::ProtectSuccess,
                oracle.protected || !oracle.failed,
            )));
        }
        if move_def.primary_effect == PrimaryEffect::Damage {
            // Hit sequence per target, in target order: the count is fixed to what Showdown
            // reported (a sequence cut short by a faint reports the hits that landed, which
            // reproduces the same outcome), then every hit rolls critical + damage (Gen 1: once
            // for the whole sequence) and its per-hit secondaries. `-crit` events are consumed
            // in order across targets.
            let landed = oracle.hit_count.unwrap_or(1).max(1);
            let same_damage = rules.hits().same_damage_each_hit;
            // The same per-target accuracy chain as above: the first check spends the Micle
            // Berry (DESIGN.md R-M8-44).
            let mut accuracy_state = state.clone();
            // Each target's hits, in order: a hit reported for a Pokémon matches that target;
            // one the replay cannot place is taken in order.
            let mut taken = vec![false; oracle.crits.len()];
            // Gen 3-4 Focus Bands this move armed (they roll no more).
            let mut bands_armed: Vec<PokemonId> = Vec::new();
            // A spread move resolves stage by stage (engine `apply_spread_hit`): every target's
            // accuracy, then the Substitutes' damage, the other targets' damage, the
            // secondaries and the reactions, each stage in target order. Gen 1-3 hit each
            // target in full before the next (`staged_spread_hits`).
            let spread = rules.hits().staged_spread_hits
                && move_def.multi_hit == MultiHit::Single
                && targets
                    .iter()
                    .filter(|(target, _)| target.is_some())
                    .count()
                    > 1;
            let mut staged = StagedTape::new(spread);
            staged.gates_first = move_def.smart_target && targets.len() > 1;
            // A user-directed secondary rolls once per target and hit; the user shows one
            // effect per success (Gen 6 Diamond Storm's Defense +1 on each foe it rolls for):
            // that many rolls landed, the first ones — which ones cannot be told, nor matters.
            let mut user_effects_left: Vec<Option<usize>> = vec![None; move_def.secondaries.len()];
            // Thrash's or Uproar's lock is drawn as the first hit starts it; a later hit (Parental
            // Bond's second) adds the volatile again, which draws nothing (DESIGN.md R281).
            let mut lock_drawn = false;
            // Cursed Body rolls until it disables the user's move, on whichever target.
            let mut cursed_body_done = false;
            for (target, fate) in &targets {
                // Dragon Darts shows no `-hitcount`: its darts per target are the hits the log
                // names on it.
                let landed = if move_def.smart_target {
                    let on_target = oracle
                        .hit_targets
                        .iter()
                        .filter(|who| who.as_ref().and_then(|w| who_to_id(state, w)) == *target)
                        .count() as u16;
                    on_target.max(1)
                } else {
                    landed
                };
                // Static / Flame Body / Poison Point roll after every hit that reaches the
                // target's HP on a contact move (a Substitute takes the hit instead).
                // Per hit, only a hit on the body reacts: a multi-hit move that breaks a
                // Substitute meets the ability on the hits after it. A move counts as contact as
                // the engine has it (`checkMoveMakesContact`: an attacker's Protective Pads take
                // the contact away, R-M8-153).
                let contact_reaction = target.is_some_and(|id| {
                    pokemon_battle_engine::engine::move_makes_contact(state, rules, actor, move_def)
                        && state.pokemon(id).is_some_and(|p| {
                            pokemon_battle_engine::engine::ability_handlers(state, rules, p)
                                .contact_status
                                .is_some()
                        })
                });
                match fate {
                    TargetFate::Immune => {
                        if let Some(hit) = accuracy_before_block(*target) {
                            staged.push(
                                Stage::Gate,
                                RandomOverride::bool(RandomRequestKind::Accuracy, hit),
                            );
                        }
                        continue;
                    }
                    TargetFate::Dodged => continue,
                    TargetFate::Miss => {
                        if accuracy_draws(&accuracy_state, rules, actor, *target, move_def) {
                            staged.push(
                                Stage::Gate,
                                RandomOverride::bool(RandomRequestKind::Accuracy, false),
                            );
                        }
                        continue;
                    }
                    TargetFate::Hit => {
                        if accuracy_draws(&accuracy_state, rules, actor, *target, move_def) {
                            staged.push(
                                Stage::Gate,
                                RandomOverride::bool(RandomRequestKind::Accuracy, true),
                            );
                        }
                    }
                }
                if let Some(id) = target
                    && matches!(fate, TargetFate::Hit | TargetFate::Miss)
                {
                    pokemon_battle_engine::engine::spend_accuracy_check(
                        &mut accuracy_state,
                        rules,
                        actor,
                        *id,
                        move_def,
                    );
                }
                // Present's heal, Pollen Puff on an ally: no critical hit, no damage roll.
                let drawn = move_def.behavior.random_power.map(|random| {
                    random.effect(drawn_value(random, oracle, behind_substitute(state, fates)))
                });
                if target.is_some_and(|id| {
                    pokemon_battle_engine::engine::heals_instead(
                        state, rules, actor, id, move_def, drawn,
                    )
                    .is_some()
                }) {
                    continue;
                }
                if let MultiHit::Range { min, .. } = move_def.multi_hit {
                    // A sequence cut short by a faint after fewer hits than the minimum: any
                    // legal count stops at the same hit.
                    staged.push(
                        Stage::Gate,
                        RandomOverride::range(
                            RandomRequestKind::MultiHitCount,
                            landed.max(u16::from(min)),
                        ),
                    );
                }
                // A power of nothing (Gen 2-4 Beat Up without members, a Frustration of full
                // happiness in Gen 2) stops before the hit: no critical hit, no roll, no
                // secondary (not even an item's flinch).
                if target.is_some_and(|id| {
                    pokemon_battle_engine::engine::zero_power_does_nothing(
                        in_move, rules, dex, actor, id, move_def,
                    )
                }) {
                    continue;
                }
                // Fixed damage and one-hit KOs roll neither a critical hit nor damage; the
                // engine says whether a critical-hit decision is drawn (`willCrit`, a ratio of 0,
                // Gen 1 without a species ask nothing).
                let formula = target.is_none_or(|id| {
                    pokemon_battle_engine::engine::fixed_damage(state, rules, actor, id, move_def)
                        .is_none()
                        && pokemon_battle_engine::engine::binding_damage(
                            state, rules, actor, id, move_def,
                        )
                        .is_none()
                        && move_def.fixed_damage.is_none()
                        && !pokemon_battle_engine::engine::zero_power_does_nothing(
                            in_move, rules, dex, actor, id, move_def,
                        )
                });
                let crit_rolled = matches!(
                    pokemon_battle_engine::engine::critical_roll(
                        in_move,
                        rules,
                        dex,
                        actor,
                        target.unwrap_or(actor),
                        move_def
                    ),
                    pokemon_battle_engine::engine::CriticalRoll::Chance(..)
                );
                // The hits the sequence was decided to have (Beat Up: one per member, as the
                // engine fixes them when the move starts); Gen 1 plans its one damage roll on
                // the first hit and its secondaries with it, as of the hit that rolls them.
                let planned = match move_def.multi_hit {
                    _ if move_def.behavior.beat_up => {
                        pokemon_battle_engine::engine::beat_up_members(in_move, rules, actor).len()
                            as u16
                    }
                    MultiHit::Single => 1,
                    MultiHit::Fixed(count) => u16::from(count),
                    MultiHit::Range { min, .. } => landed.max(u16::from(min)),
                };
                for hit in 0..usize::from(landed) {
                    if hit > 0 && same_damage {
                        break;
                    }
                    let hit_number = if same_damage {
                        usize::from(landed)
                    } else {
                        hit + 1
                    };
                    // Showdown `multiaccuracy` (Triple Kick): every hit after the first checks
                    // accuracy again.
                    if hit > 0 && move_def.multi_accuracy && rolls_later_accuracy(*target) {
                        staged.push(
                            Stage::Gate,
                            RandomOverride::bool(RandomRequestKind::Accuracy, true),
                        );
                    }
                    let reached = |index: usize| {
                        oracle
                            .hit_targets
                            .get(index)
                            .cloned()
                            .flatten()
                            .and_then(|who| who_to_id(state, &who))
                    };
                    let index = (0..taken.len())
                        .find(|&i| !taken[i] && target.is_some() && reached(i) == *target)
                        .or_else(|| (0..taken.len()).find(|&i| !taken[i]));
                    if let Some(index) = index {
                        taken[index] = true;
                    }
                    let crit = index.is_some_and(|i| oracle.crits[i]);
                    let behind_substitute =
                        index.is_some_and(|i| oracle.substitute_hits.get(i) == Some(&true));
                    // The HP this hit took off its target: from the HP the target's previous
                    // hit left it (or it had before the move) to this hit's. A Focus Band holding
                    // a target already at 1 HP takes nothing (`damage` 0 for the Gen 3-4 contact
                    // reactions).
                    let before_after = match (*target, index) {
                        (Some(id), Some(i)) => oracle.hit_hp[i].map(|after| {
                            let before = (0..i)
                                .rev()
                                .find(|&j| reached(j) == Some(id) && oracle.hit_hp[j].is_some())
                                .and_then(|j| oracle.hit_hp[j])
                                .or_else(|| state.pokemon(id).map(|p| p.hp))
                                .unwrap_or(after);
                            (before, after)
                        }),
                        _ => None,
                    };
                    let dealt =
                        before_after.map_or(1, |(before, after)| before.saturating_sub(after));
                    // The hit left its target's HP exactly as it was (not healed in between).
                    let took_nothing = before_after.is_some_and(|(before, after)| before == after);
                    let damage_stage = if behind_substitute {
                        Stage::SubstituteDamage
                    } else {
                        Stage::Damage
                    };
                    if formula {
                        if crit_rolled {
                            staged.push(
                                damage_stage,
                                RandomOverride::bool(RandomRequestKind::Critical, crit),
                            );
                        }
                        // A BasePower participant's own chance (Fickle Beam), after the
                        // critical-hit check.
                        if move_def.behavior.power_chance.is_some() {
                            staged.push(
                                damage_stage,
                                RandomOverride::bool(
                                    RandomRequestKind::PowerChance,
                                    oracle.power_chance,
                                ),
                            );
                        }
                        // A hit that took nothing off a target in the open may draw no roll: a
                        // BasePower event that left 0 stops Showdown's `getDamage` before it
                        // (R-M8-84). Its roll is searched as any other, but optional.
                        let open = !move_def.no_damage_variance;
                        if open && took_nothing && !behind_substitute {
                            staged.push(damage_stage, open_optional_roll());
                        } else if open {
                            staged.push_roll(damage_stage);
                        }
                    } else if move_def.fixed_damage
                        == Some(pokemon_battle_engine::dex::FixedDamage::Psywave)
                        && let Some(id) = *target
                    {
                        // Psywave's damage is the roll: read back from the HP it took — or, a
                        // hold on 1 HP the log names (a Focus Band, a Focus Sash, Sturdy), the
                        // smallest roll that would have knocked the target out (R-M10-48).
                        let named = |who: &Who| who_to_id(state, who) == Some(id);
                        let held = oracle
                            .focus_band_held
                            .iter()
                            .any(|(who, hits)| Some(*hits) == index && named(who))
                            || oracle.focus_sash_held.iter().any(named)
                            || oracle.sturdy_held.iter().any(named);
                        let (dealt, after) = if held {
                            (state.pokemon(id).map_or(0, |p| p.hp), Some(0))
                        } else {
                            (dealt, index.and_then(|i| oracle.hit_hp[i]))
                        };
                        staged.push(
                            damage_stage,
                            RandomOverride::range(
                                RandomRequestKind::DamageRoll,
                                psywave_roll(state, rules, actor, id, dealt, after, substitutes),
                            ),
                        );
                    }
                    // Focus Band (DESIGN.md R196): where it lasts the move (Gen 3-4, issues.md
                    // I38) every hit rolls until one succeeds — the success nothing shows is
                    // planned on the hit the band held (a Substitute's hit rolls with its damage);
                    // elsewhere only a hit that would knock the holder out rolls: one it held or
                    // one that left it fainted.
                    if let (Some(id), Some(i)) = (*target, index)
                        && pokemon_battle_engine::engine::held_item_id(state, rules, id)
                            == Some(pokemon_battle_engine::ids::ItemId::FOCUS_BAND)
                    {
                        let held = oracle
                            .focus_band_held
                            .iter()
                            .any(|(who, hits)| *hits == i && who_to_id(state, who) == Some(id));
                        let decision = RandomOverride::bool(RandomRequestKind::FocusBand, held);
                        // Read from the generation (I38), not the engine's rule table, so that
                        // the replay checks the rule.
                        if matches!(rules.generation(), Generation::Gen3 | Generation::Gen4) {
                            if !bands_armed.contains(&id) {
                                staged.push(
                                    if behind_substitute {
                                        Stage::SubstituteDamage
                                    } else {
                                        Stage::Land
                                    },
                                    decision,
                                );
                                if held {
                                    bands_armed.push(id);
                                }
                            }
                        } else if !behind_substitute {
                            // Optional, on every hit: only a damage that would knock the holder
                            // out draws it, and the damage-roll search tries rolls that would or
                            // would not — the engine then goes on to the hit's next draw (a
                            // secondary's chance, the next hit's critical-hit check). A hit the
                            // log shows the holder standing through without the band held it
                            // with no draw; a roll that would draw there is a wrong roll, and
                            // the planned failure says so (DESIGN.md R251).
                            staged.push(Stage::Land, decision.optional());
                        }
                    }
                    // Thrash's lock starts with the hit (Gen 3+ `selfDrops`, before the
                    // secondaries, §15.12): its length drawn.
                    if move_def.behavior.rampage
                        && !rules.conditions().rampage.counter.counts_uses()
                        && pokemon.volatiles.rampage().is_none()
                        && !std::mem::replace(&mut lock_drawn, true)
                        && let Some(length) = duration_for(Some(actor), DurationKind::Rampage)
                    {
                        staged.push(
                            Stage::Effect,
                            RandomOverride::range(RandomRequestKind::Duration, length),
                        );
                    }
                    // So does Uproar (Gen 3-4: its turns drawn).
                    let uproar_turns = rules.conditions().uproar.turns;
                    if move_def.behavior.uproar
                        && uproar_turns.0 != uproar_turns.1
                        && pokemon.volatiles.uproar().is_none()
                        && !std::mem::replace(&mut lock_drawn, true)
                        && let Some(turns) = duration_for(Some(actor), DurationKind::Uproar)
                    {
                        staged.push(
                            Stage::Effect,
                            RandomOverride::range(RandomRequestKind::Duration, turns),
                        );
                    }
                    // A multi-hit move's own `self` lands on every hit, before the hit's
                    // secondaries (Showdown `selfDrops`); a single-hit move's once the hits are
                    // over (below). `selfBoost` never per hit.
                    let multi_hit = move_def.multi_hit != MultiHit::Single;
                    let per_hit = move_def
                        .secondaries
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| multi_hit && s.trigger == SecondaryTrigger::OnceAfterMove)
                        .chain(
                            move_def
                                .secondaries
                                .iter()
                                .enumerate()
                                .filter(|(_, s)| s.trigger == SecondaryTrigger::PerTargetHit),
                        );
                    // A bare `-fail|target` on a target the move dealt nothing: the hit came to
                    // nothing there (Gen 2's `moveHit` returns false — a Counter with nothing to
                    // answer, Present's heal at full HP) before any secondary, King's Rock's flinch
                    // included. (A target the move damaged may show one after its Synchronize
                    // failed to send a status back: its secondaries rolled.)
                    // So does a hit on a doll that shows nothing at all — no strike on it, no
                    // damage: Gen 2's Substitute (`onTryPrimaryHit`) returns null without a word
                    // when the move's damage is nothing (a Mirror Coat with nothing to answer,
                    // R-M8-179).
                    let came_to_nothing = target.is_some_and(|id| {
                        let on_target = |who: &Who| who_to_id(state, who) == Some(id);
                        let undamaged = !oracle.damages.iter().any(|(who, _)| on_target(who));
                        let failed = oracle
                            .applied
                            .iter()
                            .any(|(who, kind)| kind == "fail" && on_target(who));
                        let doll_untouched = substitute_hp(state, id).is_some()
                            && !oracle
                                .substitute_struck
                                .iter()
                                .any(|(who, _)| on_target(who));
                        undamaged && (failed || doll_untouched)
                    });
                    // Gen 1: a confusion secondary reaches past a doll the hit left standing
                    // (`ConditionRules::confusion_secondary_through_substitute`, R-M8-93).
                    let doll_stands = target.is_some_and(|id| {
                        substitutes
                            .iter()
                            .any(|(holder, hp)| *holder == id && hp.is_some_and(|hp| hp > 0))
                    });
                    for (secondary_index, secondary) in per_hit {
                        let past_the_doll =
                            rules.conditions().confusion_secondary_through_substitute
                                && doll_stands
                                && secondary.effect
                                    == SecondaryEffect::Volatile(VolatileConditionKind::Confusion);
                        // A hit on a Substitute rolls nothing for the target behind it.
                        if came_to_nothing
                            || (behind_substitute
                                && secondary.target == SecondaryTarget::Target
                                && !past_the_doll)
                            || pokemon_battle_engine::engine::secondary_withheld(
                                rules,
                                move_def,
                                hit_number as u8,
                                planned as u8,
                                secondary,
                            )
                        {
                            continue;
                        }
                        // Did this secondary land on this target? Per target for a
                        // target-directed one (spread moves roll each target separately); on the
                        // user for a self-directed one.
                        // What this secondary shows when it lands on its target. A stage
                        // change goes the secondary's way (the other with Contrary): a Rage
                        // user's Attack boost from the hit is not Aurora Beam's drop (R-M8-151).
                        let shows = |label: &str, contrary: bool| match secondary.effect {
                            SecondaryEffect::Status(status) => {
                                label == format!("status:{}", status_id(status))
                            }
                            SecondaryEffect::StatStage { stat, delta } => {
                                (stat_changed(label, stat)
                                    && label.starts_with(if (delta < 0) != contrary {
                                        "unboost:"
                                    } else {
                                        "boost:"
                                    }))
                                    || (delta < 0 && label == "reflected")
                            }
                            SecondaryEffect::Volatile(VolatileConditionKind::Flinch) => {
                                label == "flinch"
                            }
                            SecondaryEffect::Volatile(VolatileConditionKind::Confusion) => {
                                label == "start:confusion"
                            }
                            SecondaryEffect::Volatile(VolatileConditionKind::PartiallyTrapped) => {
                                label == "start:partiallytrapped"
                            }
                            SecondaryEffect::Volatile(_) => label.starts_with("start:"),
                            SecondaryEffect::RandomStatus(pool) => pool
                                .iter()
                                .any(|status| label == format!("status:{}", status_id(*status))),
                            SecondaryEffect::StealItem => label == "stolen",
                            // A certain one (Eerie Spell's): nothing is planned from it.
                            SecondaryEffect::EerieSpell => false,
                        };
                        let landed = match (secondary.target, target) {
                            // A multi-hit move's roll after this hit: what showed before the next.
                            (SecondaryTarget::Target, Some(id)) => {
                                // The target's Contrary as the move meets it: a Mold Breaker
                                // user's move ignores it (`ability_running` in the move, R-M10-57).
                                let contrary = in_move.pokemon(*id).is_some_and(|pokemon| {
                                    pokemon_battle_engine::engine::ability_running(
                                        in_move, rules, pokemon,
                                    ) == Some(pokemon_battle_engine::ids::AbilityId::CONTRARY)
                                });
                                oracle.secondary_on.iter().any(|(who, label, hits)| {
                                    who_to_id(state, who) == Some(*id)
                                        && shows(label, contrary)
                                        && (!multi_hit
                                            || *hits == usize::MAX
                                            || *hits == hit_number)
                                })
                            }
                            // Only the effect this secondary causes counts: a contact
                            // reaction's paralysis on the user is not Metal Claw's boost.
                            (SecondaryTarget::User, _) => {
                                // (either direction: Contrary turns a boost into a drop).
                                let caused = |kind: &str| match secondary.effect {
                                    SecondaryEffect::StatStage { stat, .. } => {
                                        stat_changed(kind, stat)
                                    }
                                    SecondaryEffect::Status(_)
                                    | SecondaryEffect::RandomStatus(_) => {
                                        kind.starts_with("status")
                                    }
                                    SecondaryEffect::Volatile(_) => kind.starts_with("start"),
                                    SecondaryEffect::StealItem | SecondaryEffect::EerieSpell => {
                                        false
                                    }
                                };
                                if multi_hit {
                                    // A multi-hit move's roll after this hit: the effect shown
                                    // before the next (Parental Bond Fiery Dance's second).
                                    oracle
                                        .user_applied_hits
                                        .iter()
                                        .any(|(kind, hits)| *hits == hit_number && caused(kind))
                                } else {
                                    let left = user_effects_left[secondary_index]
                                        .get_or_insert_with(|| {
                                            oracle
                                                .applied
                                                .iter()
                                                .filter(|(who, kind)| {
                                                    *who == oracle.user && caused(kind)
                                                })
                                                .count()
                                        });
                                    let landed = *left > 0;
                                    *left = left.saturating_sub(1);
                                    landed
                                }
                            }
                            (SecondaryTarget::Target, None) => oracle.secondary_applied,
                        };
                        // Serene Grace doubles the chance; a certain one asks nothing, nor does
                        // one Gen 2 skips before the roll.
                        let affected = match secondary.target {
                            SecondaryTarget::Target => target.unwrap_or(actor),
                            SecondaryTarget::User => actor,
                        };
                        // A random status (Tri Attack, Dire Claw): which one was drawn. Showdown
                        // shows it when it lands; otherwise the draw is one that changes nothing
                        // — Gen 2 Tri Attack on a frozen target draws a burn exactly when its hit
                        // thaws the target.
                        let random_pick = match (secondary.effect, target) {
                            (SecondaryEffect::RandomStatus(pool), Some(id)) => {
                                let shown = pool.iter().position(|status| {
                                    oracle.secondary_on.iter().any(|(who, label, _)| {
                                        who_to_id(state, who) == Some(*id)
                                            && *label == format!("status:{}", status_id(*status))
                                    })
                                });
                                let refused = || {
                                    let pokemon = state.pokemon(*id)?;
                                    if pokemon.has_status(NonVolatileStatus::Freeze)
                                        && move_def.behavior.status_pick_before_chance
                                    {
                                        let thawed = oracle
                                            .thawed
                                            .iter()
                                            .any(|who| who_to_id(state, who) == Some(*id));
                                        return pool.iter().position(|status| {
                                            (*status == NonVolatileStatus::Burn) == thawed
                                        });
                                    }
                                    if pokemon.status.is_some() {
                                        return Some(0);
                                    }
                                    pool.iter().position(|status| {
                                        pokemon_battle_engine::engine::status_type_immune(
                                            state, rules, *id, *status,
                                        )
                                    })
                                };
                                Some((shown.or_else(refused).unwrap_or(0) as u16, pool, shown))
                            }
                            _ => None,
                        };
                        // Gen 2 Tri Attack's `onHit` draws before the chance, every hit.
                        if let Some((pick, ..)) = random_pick
                            && move_def.behavior.status_pick_before_chance
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Sample, pick),
                            );
                        }
                        let roll = pokemon_battle_engine::engine::secondary_roll(
                            in_move, rules, actor, affected, move_def, secondary,
                        );
                        // A roll the next secondaries share (Silver Wind's five boosts) landed if
                        // any of them shows: a stat already at +6 shows nothing.
                        let group_landed = landed
                            || (secondary.target == SecondaryTarget::User
                                && move_def.secondaries[secondary_index + 1..]
                                    .iter()
                                    .take_while(|next| next.shares_roll)
                                    .any(|next| match next.effect {
                                        // A multi-hit move's: shown after this hit (a Parental
                                        // Bond Ancient Power's boosts after its second hit are
                                        // the second's roll, R-M8-108).
                                        SecondaryEffect::StatStage { stat, .. } if multi_hit => {
                                            oracle.user_applied_hits.iter().any(|(kind, hits)| {
                                                *hits == hit_number && stat_changed(kind, stat)
                                            })
                                        }
                                        SecondaryEffect::StatStage { stat, .. } => {
                                            oracle.applied.iter().any(|(who, kind)| {
                                                *who == oracle.user && stat_changed(kind, stat)
                                            })
                                        }
                                        _ => false,
                                    }));
                        if !secondary.shares_roll
                            && let pokemon_battle_engine::engine::SecondaryRoll::Chance(..) = roll
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::bool(RandomRequestKind::Secondary, group_landed),
                            );
                        }
                        if let Some((pick, pool, shown)) = random_pick {
                            // Otherwise the draw follows a chance that succeeded.
                            if !move_def.behavior.status_pick_before_chance
                                && (landed
                                    || roll
                                        == pokemon_battle_engine::engine::SecondaryRoll::Certain)
                            {
                                staged.push(
                                    Stage::Effect,
                                    RandomOverride::range(RandomRequestKind::Sample, pick),
                                );
                            }
                            // A sleep it put the target to rolls its duration.
                            if shown.is_some_and(|index| pool[index] == NonVolatileStatus::Sleep)
                                && let Some(duration) =
                                    duration_for(Some(affected), DurationKind::Sleep)
                            {
                                staged.push(
                                    Stage::Effect,
                                    RandomOverride::range(RandomRequestKind::Duration, duration),
                                );
                            }
                        }
                        // A sleep secondary that took (Secret Power in Grassy Terrain) rolls its
                        // duration — not one a clause stopped after its chance (Sleep Clause Mod:
                        // no `-status`, R-M10-83).
                        if landed
                            && secondary.effect == SecondaryEffect::Status(NonVolatileStatus::Sleep)
                            && oracle.applied.iter().any(|(who, kind)| {
                                kind == "status:slp" && who_to_id(state, who) == Some(affected)
                            })
                            && let Some(duration) =
                                duration_for(Some(affected), DurationKind::Sleep)
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Duration, duration),
                            );
                        }
                        // Poison Puppeteer: a poison secondary that took confuses the target too —
                        // only the holder's (a confusion berry the poison fed it rolls its own,
                        // R-M9-35).
                        if landed
                            && pokemon_battle_engine::engine::poison_puppeteer_confuses(
                                state, rules, actor,
                            )
                            && matches!(
                                secondary.effect,
                                SecondaryEffect::Status(
                                    NonVolatileStatus::Poison | NonVolatileStatus::Toxic
                                )
                            )
                            && target.is_some_and(|id| {
                                oracle.applied.iter().any(|(who, kind)| {
                                    kind == "start:confusion" && who_to_id(state, who) == Some(id)
                                })
                            })
                            && let Some(duration) =
                                duration_for(Some(affected), DurationKind::Confusion)
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Duration, duration),
                            );
                        }
                        // Applying confusion rolls its duration right after the secondary lands.
                        let confuses = matches!(
                            secondary.effect,
                            SecondaryEffect::Volatile(VolatileConditionKind::Confusion)
                        );
                        if landed
                            && confuses
                            && let Some(duration) =
                                duration_for(Some(affected), DurationKind::Confusion)
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Duration, duration),
                            );
                        } else if confuses
                            && !landed
                            && oracle.damages.iter().any(|(who, hp)| {
                                *hp == 0
                                    && target.is_some_and(|id| who_to_id(state, who) == Some(id))
                            })
                            && pokemon_battle_engine::engine::secondary_roll(
                                in_move, rules, actor, affected, move_def, secondary,
                            ) == pokemon_battle_engine::engine::SecondaryRoll::Certain
                        {
                            // A certain confusion the knock-out kept off (Dynamic Punch): a
                            // smaller damage roll the search tries leaves the target standing and
                            // confused, asking for a duration — optional.
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(
                                    RandomRequestKind::Duration,
                                    u16::from(rules.status().lifecycle.confusion_time_min),
                                )
                                .optional(),
                            );
                        }
                        // A bind (Bind / Wrap / Fire Spin …) rolls its duration when it takes.
                        // One that did not because the hit knocked the target out may still
                        // be asked for by a smaller damage roll the search tries: optional.
                        let binds = matches!(
                            secondary.effect,
                            SecondaryEffect::Volatile(VolatileConditionKind::PartiallyTrapped)
                        );
                        let knocked_out = oracle.damages.iter().any(|(who, hp)| {
                            *hp == 0 && target.is_some_and(|id| who_to_id(state, who) == Some(id))
                        });
                        // A multi-hit move binds on the hit that shows it; the later hits find
                        // the target bound and roll nothing (Parental Bond Infestation).
                        let bound = if multi_hit {
                            oracle.secondary_on.iter().any(|(who, label, hits)| {
                                label == "start:partiallytrapped"
                                    && *hits == hit_number
                                    && target.is_some_and(|id| who_to_id(state, who) == Some(id))
                            })
                        } else {
                            oracle
                                .applied
                                .iter()
                                .any(|(_, kind)| kind == "start:partiallytrapped")
                        };
                        // A trapper holding Grip Claw rolls no duration (its fixed turns).
                        let grip_claw =
                            pokemon_battle_engine::engine::held_item_id(state, rules, actor)
                                == Some(pokemon_battle_engine::ids::ItemId::GRIP_CLAW);
                        if binds
                            && bound
                            && !grip_claw
                            && let Some(duration) = duration_for(*target, DurationKind::Bind)
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Duration, duration),
                            );
                        } else if binds
                            && knocked_out
                            && !rules.conditions().switching.binding_locks_user
                        {
                            // (Gen 1's bind rolls nothing of its own: the lock's turns are
                            // rolled after the hit, KO or not.)
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(
                                    RandomRequestKind::Duration,
                                    u16::from(rules.conditions().bind_turns_min),
                                )
                                .optional(),
                            );
                        }
                    }
                    // Wandering Spirit traded on an earlier hit: the target now holds the user's
                    // ability, whose own contact reaction answers the later hits (R-M9-35).
                    let contact_reaction = match oracle.ability_taken_after {
                        Some(after) if oracle.ability_traded && hit >= after => {
                            pokemon_battle_engine::engine::move_makes_contact(
                                state, rules, actor, move_def,
                            ) && state.pokemon(actor).is_some_and(|user| {
                                pokemon_battle_engine::engine::ability_behavior(
                                    rules,
                                    pokemon_battle_engine::engine::raw_ability(user),
                                )
                                .contact_status
                                .is_some()
                            })
                        }
                        _ => contact_reaction,
                    };
                    // A hit on the Substitute asks nothing; but a candidate damage roll that
                    // breaks it sooner than Showdown did reaches the body here and rolls, so
                    // the search meets an optional failed roll instead of a desynchronized plan.
                    if contact_reaction && behind_substitute {
                        staged.push(
                            Stage::Reaction,
                            RandomOverride::bool(RandomRequestKind::ContactReaction, false)
                                .optional(),
                        );
                    }
                    if contact_reaction
                        && !behind_substitute
                        && !(rules.hits().contact_reaction_needs_damage && dealt == 0)
                    {
                        // The reaction that took is the one after the hit it followed; the
                        // other hits' rolls failed — a later one's changes nothing while the
                        // status stays, and one something cured in between (Lum Berry) would
                        // show a second report (R-M8-134). A spread move reports every hit
                        // before the reactions: the one that took is the holder's.
                        let holder = oracle.contact_status_of.as_ref().is_none_or(|who| {
                            target.is_none_or(|id| who_to_id(state, who) == Some(id))
                        });
                        let took = oracle.contact_status.is_some()
                            && holder
                            && (spread
                                || index
                                    .is_none_or(|i| oracle.contact_status_hits.contains(&(i + 1))));
                        staged.push(
                            Stage::Reaction,
                            RandomOverride::bool(RandomRequestKind::ContactReaction, took),
                        );
                    }
                    if let Some(id) = *target {
                        chance_reactions(
                            &mut staged,
                            ChanceReactionHit {
                                // The user as the hit lands: Protean's type already its own (a
                                // Grass type shrugs off Effect Spore's powder, R-M8-180).
                                state: in_move,
                                rules,
                                actor,
                                target: id,
                                move_def,
                                oracle,
                                behind_substitute,
                                dealt,
                                // Which hit the reported reaction followed: a spread move reports
                                // every hit before the reactions.
                                reported_after: &|after| {
                                    spread || index.is_none_or(|i| i + 1 == after)
                                },
                                sleep_duration: duration_for(Some(actor), DurationKind::Sleep),
                                user_keeps_ability: spread
                                    || oracle
                                        .ability_taken_after
                                        .is_none_or(|after| index.is_none_or(|i| i < after)),
                            },
                            &mut cursed_body_done,
                        );
                    }
                    // A berry the `Update` after this hit fed the target (or the user, hurt by the
                    // hit's reaction) draws after the reactions: a confusion berry's duration,
                    // Starf Berry's stat.
                    for (who, hits, draw) in &oracle.berry_draws {
                        let eater = who_to_id(state, who);
                        if index.is_some_and(|i| i + 1 == *hits)
                            && (eater == *target || eater == Some(actor))
                            && !berry_confusions_planned.contains(&eater)
                            && let Some(decision) = berry_decision(*draw, || {
                                duration_for(eater, DurationKind::Confusion)
                            })
                        {
                            berry_confusions_planned.push(eater);
                            // A berry the move itself stole and ate (Pluck, R-M9-48) or threw
                            // (Fling, R-M10-3) draws in its `onHit`, ahead of the hit's reactions.
                            let stage = if oracle.stolen_berry.as_ref() == Some(who)
                                || oracle.flung_berry.as_ref() == Some(who)
                            {
                                Stage::Effect
                            } else {
                                Stage::Reaction
                            };
                            staged.push(stage, decision);
                        }
                    }
                    staged.end_hit();
                }
                // A `multiaccuracy` sequence cut short with both sides standing missed its next
                // hit (nothing shows it but the hit count) — unless a hit's reaction put the user
                // to sleep, which stops the sequence before the check (DESIGN.md R263).
                if move_def.multi_accuracy
                    && let MultiHit::Fixed(count) = move_def.multi_hit
                    && landed < u16::from(count)
                    && oracle.contact_status.as_deref() != Some("slp")
                    && rolls_later_accuracy(*target)
                    && !oracle.damages.iter().any(|(who, hp)| {
                        *hp == 0
                            && (*who == oracle.user
                                || target.is_some_and(|id| who_to_id(state, who) == Some(id)))
                    })
                {
                    staged.push(
                        Stage::Gate,
                        RandomOverride::bool(RandomRequestKind::Accuracy, false),
                    );
                }
                staged.end_hit();
            }
            tape.extend(staged.finish());
            // Gen 1: a binding move that reached its target locks a user not locked onto it yet
            // (§30.9), rolling the lock's turns last (a knocked-out target too: the roll comes
            // before the lock's `onStart` finds no foe).
            if rules.conditions().switching.binding_locks_user
                && move_def.behavior.binding_lock
                && let Some((Some(target), TargetFate::Hit)) = targets.first().copied()
                && (oracle
                    .damages
                    .iter()
                    .any(|(who, _)| who_to_id(state, who) == Some(target))
                    || oracle.substitute_hits.iter().any(|hit| *hit))
                && pokemon
                    .volatiles
                    .partial_trapping_lock()
                    .is_none_or(|(locked, ..)| locked != target)
                && let Some(duration) = duration_for(Some(actor), DurationKind::BindingLock)
            {
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::Duration,
                    duration,
                )));
            }
            // The move's own `self` with a chance of its own (Gen 7+ Diamond Storm) rolls once the
            // hits are over, if one landed: it landed when the user shows the change (without a
            // landed hit nothing asks).
            for secondary in move_def.secondaries.iter().filter(|secondary| {
                secondary.trigger == SecondaryTrigger::OnceAfterMove
                    && !secondary.shares_roll
                    && move_def.multi_hit == MultiHit::Single
            }) {
                if let pokemon_battle_engine::engine::SecondaryRoll::Chance(..) =
                    pokemon_battle_engine::engine::secondary_roll(
                        in_move, rules, actor, actor, move_def, secondary,
                    )
                {
                    let shown = oracle.applied.iter().any(|(who, kind)| {
                        *who == oracle.user
                            && matches!(secondary.effect,
                                SecondaryEffect::StatStage { stat, .. } if stat_changed(kind, stat))
                    });
                    let decision = RandomOverride::bool(RandomRequestKind::Secondary, shown);
                    tape.push(Some(if shown { decision } else { decision.optional() }));
                }
            }
            // Gen 2-4 crash (`HitRules::crash`): what the move would have dealt its target is
            // computed afresh — a critical-hit check nothing shows (planned as not, the search
            // retries it as one) and a damage roll the crash's HP settles. A target immune to
            // the move's type computes nothing.
            let first_target = targets.iter().find_map(|(target, _)| *target);
            if oracle.crashed
                && matches!(
                    rules.hits().crash,
                    pokemon_battle_engine::rules::CrashDamage::ShareOfDamage { .. }
                )
                && let Some(target) = first_target
                && pokemon_battle_engine::engine::type_immunity(state, rules, target, move_def)
                    .1
                    .is_none()
            {
                if matches!(
                    pokemon_battle_engine::engine::critical_roll(
                        in_move, rules, dex, actor, target, move_def
                    ),
                    pokemon_battle_engine::engine::CriticalRoll::Chance(..)
                ) {
                    tape.push(Some(
                        RandomOverride::bool(RandomRequestKind::Critical, false).optional(),
                    ));
                }
                if !move_def.no_damage_variance {
                    tape.push(None);
                }
                if let Some(decision) = self_focus_band(state, rules, actor, oracle) {
                    tape.push(Some(decision));
                }
            }
        }
    }
    // Forewarn starting as the move hands it over (`ActionOracle::forewarned`): its draw among
    // the holder's foes' strongest moves, which the hand-over leaves as they were.
    for args in &oracle.forewarned {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let id =
            |text: &str| who_from_text(&Value::from(text)).and_then(|who| who_to_id(state, &who));
        if let Some((_, decision)) = forewarn_pick(rules, dex, state, &args, id) {
            tape.push(Some(decision));
        }
    }
    // Magician (the user's `onAfterMoveSecondarySelf`, once the move is over): the Pokémon it hit
    // that share a handler speed each draw a SpeedTie decision (Showdown `speedSort`); the one
    // the item came from goes first. Optional: the ability may not run (an item in hand, a gem).
    if let Some(move_def) = move_def
        && move_def.category != MoveCategory::Status
        && state.pokemon(actor).is_some_and(|user| {
            user.held_item.is_none()
                && pokemon_battle_engine::engine::raw_ability(user)
                    == pokemon_battle_engine::ids::AbilityId::MAGICIAN
        })
    {
        let hit: Vec<PokemonId> = fates
            .iter()
            .filter(|(_, fate)| *fate == TargetFate::Hit)
            .filter_map(|(target, _)| *target)
            .filter(|target| *target != actor)
            .collect();
        let from = oracle
            .magician_from
            .as_ref()
            .and_then(|who| who_to_id(state, who));
        for target in &hit {
            let speed = state.handler_speed(*target);
            if hit
                .iter()
                .any(|other| other != target && state.handler_speed(*other) == speed)
            {
                let rank = if Some(*target) == from { 2 } else { 1 };
                tape.push(Some(
                    RandomOverride::raw(RandomRequestKind::SpeedTie, rank).optional(),
                ));
            }
        }
    }
    // Gen 3+ Thrash's lock ending with its move (Gen 5+ `onAfterMove`, §15.12): the fatigue's
    // confusion. (Gen 1's came as the move started; Gen 3-4's and a stopped Gen 5+ move's come
    // in the residual.)
    if let Some(time) = fatigue_time() {
        tape.push(Some(RandomOverride::range(
            RandomRequestKind::Duration,
            time,
        )));
    }
    // The rest of the action's berry draws (a status move's target; the user, hurt by its
    // recoil; a Pokémon it dragged in onto hazards): drawn at the move's or the action's
    // closing `Update`.
    for (who, _, draw) in &oracle.berry_draws {
        let eater = who_to_id(state, who);
        if !berry_confusions_planned.contains(&eater)
            && let Some(decision) =
                berry_decision(*draw, || duration_for(eater, DurationKind::Confusion))
        {
            berry_confusions_planned.push(eater);
            tape.push(Some(decision));
        }
    }
    tape
}

/// The Focus Band roll of damage the user's own action dealt it (a confusion self-hit, a crash):
/// drawn when it would knock the user out — the band held it, or the user fainted.
fn self_focus_band(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    oracle: &ActionOracle,
) -> Option<RandomOverride> {
    if pokemon_battle_engine::engine::held_item_id(state, rules, actor)
        != Some(pokemon_battle_engine::ids::ItemId::FOCUS_BAND)
    {
        return None;
    }
    let held = oracle
        .focus_band_held
        .iter()
        .any(|(who, _)| *who == oracle.user);
    let fainted = oracle
        .damages
        .iter()
        .any(|(who, hp)| *who == oracle.user && *hp == 0);
    // Optional, as a hit's: a crash's damage follows the move's damage roll, and the rolls the
    // search tries need not knock the user out.
    (held || fainted).then(|| RandomOverride::bool(RandomRequestKind::FocusBand, held).optional())
}

/// The decision a berry's eating draws: a confusion berry's duration (as `confusion` finds it),
/// Starf Berry's stat.
fn berry_decision(
    draw: BerryDraw,
    confusion: impl FnOnce() -> Option<u16>,
) -> Option<RandomOverride> {
    match draw {
        BerryDraw::Confusion => {
            confusion().map(|duration| RandomOverride::range(RandomRequestKind::Duration, duration))
        }
        BerryDraw::Stat(code) => Some(RandomOverride::range(RandomRequestKind::Sample, code)),
    }
}

/// Recovers the confusion `time` Showdown rolled when `decisions[index]` confused `victim`:
/// the value shown in the boundary frame plus one for every BeforeMove tick the victim went
/// through afterwards this turn (a tick happens unless sleep, freeze or flinch stopped the
/// action first). If the confusion already ended within the turn, the ticks alone give it.
/// `member`: the victim's party index in the frame (`frame_member`).
fn confusion_duration_hint(
    rules: &ResolvedRules,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    index: usize,
    victim: &Who,
    member: Option<usize>,
) -> Option<u16> {
    confusion_hint_after(rules, oracle_frame, &decisions[index + 1..], victim, member)
}

/// The confusion `time` rolled for `victim` before the actions `later`: the frame's, plus a tick
/// for each of them that reached its BeforeMove confusion check. A later lock's fatigue that
/// confuses the victim afresh (Gen 1's `-start|p|confusion|[silent]` replaces the time) leaves
/// the frame to the new confusion: this one only had to outlast the checks up to it.
/// The victim is read in the frame at `member`, its party index there (`None`: off the field);
/// its actions and events are matched by side and name, as an Ally Switch may move it to the
/// other slot within the turn (R-M8-175).
fn confusion_hint_after(
    rules: &ResolvedRules,
    oracle_frame: &Value,
    later: &[ActionOracle],
    victim: &Who,
    member: Option<usize>,
) -> Option<u16> {
    if let Some(replaced) = later
        .iter()
        .position(|action| same_pokemon(&action.user, victim) && action.fatigue)
    {
        let ticks = confusion_ticks(&later[..=replaced], victim);
        return Some((ticks + 1).max(u16::from(rules.status().lifecycle.confusion_time_min)));
    }
    let later_ticks = confusion_ticks(later, victim);
    // Off the field (the victim fell, no one in its place yet) shows no confusion.
    let at_boundary = member.and_then(|member| {
        oracle_frame["sides"][victim.side]["party"][member]["volatiles"]["confusion"].as_u64()
    });
    match at_boundary {
        Some(time) => Some(time as u16 + later_ticks),
        None => {
            let events = oracle_frame["events"].as_array()?;
            let mut ended = None;
            for (index, event) in events.iter().enumerate() {
                if event["kind"] == "end"
                    && event["args"][1] == "confusion"
                    && who_from_text(&event["args"][0])
                        .is_some_and(|who| same_pokemon(&who, victim))
                {
                    ended = Some(index);
                }
            }
            // Ended by a berry the victim ate (Persim; Lum after its status cure): any time
            // behaves the same.
            let eaten = |index: usize| {
                events[..index]
                    .iter()
                    .rev()
                    .take(2)
                    .find(|event| event["kind"] != "curestatus")
                    .is_some_and(|event| {
                        berry_eaten(event).is_some_and(|(eater, _)| eater == *victim)
                    })
            };
            // Ended by none of the victim's own checks (Gen 1 Haze, before it moved again):
            // any time behaves the same.
            // (An ability that cures it at once, Own Tempo behind a Mold Breaker's Teeter Dance:
            // no check ever ticked it — any time in the range behaves the same, R-M8-127.)
            let shortest = u16::from(rules.status().lifecycle.confusion_time_min);
            match ended {
                Some(index) => (!eaten(index))
                    .then(|| later_ticks.max(shortest))
                    .filter(|ticks| *ticks > 0),
                // Never ended: still confused as it left the field (fainted, switched or dragged
                // out — the volatile went with it), so longer than the checks it went through —
                // a dance's and its own action's (R-M8-156).
                None => Some((later_ticks + 1).max(shortest)),
            }
        }
    }
}

/// The actions of `victim` among `actions` that reached its BeforeMove confusion check.
fn confusion_ticks(actions: &[ActionOracle], victim: &Who) -> u16 {
    actions
        .iter()
        .filter(|action| same_pokemon(&action.user, victim))
        // A turn stopped before confusion's own BeforeMove step (Showdown priority 3: recharge,
        // sleep, freeze, Truant, flinch, Disable, Heal Block, Throat Chop, Gravity, Taunt,
        // Imprison; the cartridges' order alike, issues.md I66, I105) does not tick it — nor does
        // a Gen 1 turn spent waking up.
        .filter(|action| {
            let before_confusion = matches!(
                action.cant.as_deref(),
                Some(
                    "recharge"
                        | "slp"
                        | "frz"
                        | "ability: Truant"
                        | "flinch"
                        | "Disable"
                        | "move: Heal Block"
                        | "move: Throat Chop"
                        | "move: Gravity"
                        | "move: Taunt"
                        | "move: Imprison"
                )
            );
            !before_confusion && !woke_without_acting(action)
        })
        .count() as u16
}

/// A Gen 1 turn spent waking up: the gen1 sleep check refuses the move even as the Pokémon
/// wakes (no `cant`, a `-curestatus`, no move), before any later BeforeMove check counts.
fn woke_without_acting(action: &ActionOracle) -> bool {
    action.cured.as_deref() == Some("slp") && action.move_name.is_none() && action.cant.is_none()
}

/// Recovers Gen 1 Disable's `time` (1-8), counted down at every BeforeMove of the victim that
/// gets past sleep (the waking turn included), freeze, flinch, a partial trap and the recharge
/// (Showdown priorities 10-7 over Disable's 6): the frame's time plus the victim's later actions in the boundary. Ended by
/// the frame (`-end`): the actions that counted it out; gone without ending (the victim left):
/// one more, as any larger time behaves the same.
fn gen1_disable_hint(
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    index: usize,
    victim: &Who,
) -> Option<u16> {
    let later_ticks = decisions[index + 1..]
        .iter()
        .filter(|action| action.user == *victim)
        .filter(|action| {
            !matches!(
                action.cant.as_deref(),
                Some("slp" | "frz" | "flinch" | "partiallytrapped" | "recharge")
            ) && !woke_without_acting(action)
        })
        .count() as u16;
    let side = &oracle_frame["sides"][victim.side];
    let at_boundary = victim
        .slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|party_index| {
            side["party"][party_index as usize]["volatiles"]["disable"].as_u64()
        });
    let ended = oracle_frame["events"].as_array().is_some_and(|events| {
        events.iter().any(|event| {
            event["kind"] == "end"
                && event["args"][1] == "Disable"
                && who_from_text(&event["args"][0]).as_ref() == Some(victim)
        })
    });
    Some(match at_boundary {
        Some(time) => time as u16 + later_ticks,
        None if ended => later_ticks,
        None => later_ticks + 1,
    })
}

/// Recovers the Taunt / Encore / Disable duration Showdown rolled (Gen 3-4): the boundary
/// frame's turns plus the end-of-turn tick already taken (none at a mid-turn request, nor when
/// `ticked` says the battle ended in the residual before it), minus the `onStart` adjustment the
/// engine computes for the target at that moment. Gone by the frame (the victim left): the
/// rule's minimum reproduces the same frames.
fn timed_duration_hint(
    oracle_frame: &Value,
    before: &BattleState,
    victim: &Who,
    volatile: &str,
    rule: pokemon_battle_engine::rules::TimedVolatileRules,
    ticked: bool,
) -> Option<u16> {
    let pokemon_battle_engine::rules::TimedDuration::Rolled { min, max } = rule.duration else {
        return None;
    };
    let side = &oracle_frame["sides"][victim.side];
    let party_index = side["active"][victim.slot?].as_u64()? as usize;
    let at_boundary = side["party"][party_index]["volatiles"][volatile].as_u64();
    let target = who_to_id(before, victim)?;
    let adjust = pokemon_battle_engine::engine::duration_adjustment(before, rule.adjust, target);
    let ticked = i16::from(ticked && residual_passed(oracle_frame));
    let rolled = match at_boundary {
        Some(turns) => turns as i16 + ticked - adjust,
        None => i16::from(min),
    };
    Some(rolled.clamp(i16::from(min), i16::from(max)) as u16)
}

/// A turn-end tick the battle, decided in the residual, never got to (R-M8-121).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissedTick {
    /// A residual handler's: Taunt, Encore, Disable, a bind, a Gen 3-4 rampage.
    Residual(ResidualKind),
    /// Gen 2+ Bide's `duration`, a duration-only handler: Showdown runs the one handler after the
    /// knock-out that decided it (Perish Song's `onEnd` prints no faint until the next handler is
    /// over), so whether it ticked depends on the order — read from the engine (R-M8-158).
    Bide,
}

/// The Gen 2+ Bide users whose Bide started in these trace events but whose `duration` the
/// engine left as the plan drew it (`bide_duration_hint` with its tick): the residual that
/// decided the battle stopped before their turn-end tick.
fn bides_left_unticked(
    state: &BattleState,
    rules: &ResolvedRules,
    oracle_frame: &Value,
    events: &[TraceEvent],
) -> Vec<(PokemonId, MissedTick)> {
    use pokemon_battle_engine::state::VolatileConditionKind as V;
    if rules.conditions().retaliation.bide.gen1 {
        return Vec::new();
    }
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::VolatileStarted {
                target,
                kind: V::Bide,
            } => Some(*target),
            _ => None,
        })
        .filter(|target| {
            let Some(pokemon) = state.pokemon(*target) else {
                return false;
            };
            let Some(spec) = state
                .active_position(*target, rules.topology())
                .and_then(|position| rules.topology().spec(position))
            else {
                return false;
            };
            let who = Who {
                side: spec.side.index(),
                slot: Some(usize::from(spec.slot)),
                name: String::new(),
            };
            let drawn = bide_duration_hint(oracle_frame, rules, &who, true);
            pokemon
                .volatiles
                .bide()
                .and_then(|bide| bide.turns)
                .is_some_and(|turns| u16::from(turns) == drawn)
        })
        .map(|target| (target, MissedTick::Bide))
        .collect()
}

/// The Pokémon whose Taunt / Encore / Disable, bind, Gen 3-4 rampage or Uproar started in these trace
/// events but never reached its end-of-turn tick (the battle was decided in the residual before
/// it), with the tick it missed.
fn timed_volatiles_left_unticked(events: &[TraceEvent]) -> Vec<(PokemonId, MissedTick)> {
    use pokemon_battle_engine::rules::ResidualKind;
    use pokemon_battle_engine::state::VolatileConditionKind as V;
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::VolatileStarted { target, kind } => match kind {
                V::Taunt => Some((*target, ResidualKind::Taunt)),
                V::Encore => Some((*target, ResidualKind::Encore)),
                V::Disable => Some((*target, ResidualKind::Disable)),
                V::PartiallyTrapped => Some((*target, ResidualKind::PartiallyTrapped)),
                V::Rampage => Some((*target, ResidualKind::Rampage)),
                V::Uproar => Some((*target, ResidualKind::Uproar)),
                _ => None,
            },
            _ => None,
        })
        .filter(|(target, residual)| {
            !events.iter().any(|event| {
                matches!(event, TraceEvent::ResidualEffect { holder, kind, .. }
                    if holder == target && kind == residual)
            })
        })
        .map(|(target, residual)| (target, MissedTick::Residual(residual)))
        .collect()
}

/// Whether the boundary frame comes after the end-of-turn residual (its events carry the next
/// `turn`, or the `residual` marker; fixtures recorded before the marker existed have only the
/// former): a frame taken at a mid-turn request (U-turn, Volt Switch), or when the battle was
/// won before the end of the turn, comes before the duration ticks.
fn residual_passed(oracle_frame: &Value) -> bool {
    oracle_frame["events"].as_array().is_some_and(|events| {
        let turn = events.iter().any(|event| event["kind"] == "turn");
        // A battle won before the end of the turn prints the marker (`win`'s blank line) and
        // nothing after it; a residual that printed nothing before a replacement request is
        // followed by nothing either, but the battle goes on.
        let decided = !oracle_frame["outcome"].is_null();
        let residual_ran = events
            .iter()
            .position(|event| event["kind"] == "residual")
            .is_some_and(|marker| marker + 1 < events.len() || !decided);
        turn || residual_ran
    })
}

/// Recovers the bind duration Showdown rolled: the boundary frame's `partiallytrapped` plus the
/// tick the end of that turn already took (none yet at a mid-turn request, nor when `ticked`
/// says a battle decided in the residual stopped before it). A bind that ended within the turn
/// (the trapper left) leaves no trace; the generation's minimum then reproduces the same frames.
/// The victim is read at `member`, its party index in the frame (`frame_member`).
fn bind_duration_hint(
    oracle_frame: &Value,
    rules: &ResolvedRules,
    victim: &Who,
    member: Option<usize>,
    ticked: bool,
) -> Option<u16> {
    // The victim may be gone from the field by the boundary (fainted to the bind itself).
    let at_boundary = member.and_then(|member| {
        oracle_frame["sides"][victim.side]["party"][member]["volatiles"]["partiallytrapped"]
            .as_u64()
    });
    let ticked = u16::from(ticked && residual_passed(oracle_frame));
    let (min, max) = (
        u16::from(rules.conditions().bind_turns_min),
        u16::from(rules.conditions().bind_turns_max),
    );
    // A first reading that assumed a tick the residual never reached may overshoot the range;
    // the boundary is then read again unticked.
    Some(match at_boundary {
        Some(turns) => (turns as u16 + ticked).clamp(min, max),
        None => min,
    })
}

/// Recovers the turns Gen 1's binding lock rolled: the user's `partialtrappinglock` at the
/// boundary plus the end-of-turn tick (none when a faint cut the turn short). A lock gone by the
/// boundary (its target knocked out) leaves no trace; the shortest then reproduces the frames.
fn binding_lock_hint(oracle_frame: &Value, user: &Who) -> u16 {
    let side = &oracle_frame["sides"][user.side];
    let left = user
        .slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|index| {
            side["party"][index as usize]["volatiles"]["partialtrappinglock"].as_u64()
        });
    match left {
        Some(turns) => (turns as u16 + u16::from(residual_passed(oracle_frame))).clamp(2, 5),
        None => 2,
    }
}

/// The `volatiles` of `who` in a boundary frame.
fn frame_volatiles<'a>(frame: &'a Value, who: &Who) -> Option<&'a Value> {
    frame_pokemon(frame, who).map(|pokemon| &pokemon["volatiles"])
}

/// The active Pokémon `who` names, as the frame shows it.
fn frame_pokemon<'a>(frame: &'a Value, who: &Who) -> Option<&'a Value> {
    let side = &frame["sides"][who.side];
    let index = side["active"][who.slot?].as_u64()? as usize;
    Some(&side["party"][index])
}

/// Recovers the length a lock (Thrash and its kind, §15.12) drew as it started this turn, from
/// `frames` / `decisions` / `residuals` (this boundary's first). Gen 1: the uses left in the
/// frame (`time`, not yet counted down); Gen 2 one more. Gen 3-4 (the cartridge's count): the frame's count plus
/// the end of the turn's, if it ran (not at a replacement request mid-turn). Gen 5+ (the hidden length): only a later turn tells — a fatigue there
/// (in the user's action or the residual) means 2, a lock still there after it 3, a lock that
/// ended silently after a hit 2 (the confusion did not take) unless the user fell asleep
/// unconfused (the sleep ended it in the residual: 3), otherwise 3; a lock gone with no action
/// of its user's and no fatigue ran out on a 3. Where nothing tells, any length behaves the
/// same: 2.
fn rampage_length_hint(
    rules: &ResolvedRules,
    frames: &[Option<&Value>],
    decisions: &[Vec<ActionOracle>],
    residuals: &[Vec<(Who, ResidualKind)>],
    who: &Who,
    ticked: bool,
) -> u16 {
    use pokemon_battle_engine::rules::RampageCounter;
    let locked = |frame: Option<&Value>| {
        frame
            .and_then(|frame| frame_volatiles(frame, who))
            .and_then(|volatiles| volatiles["lockedmove"].as_u64())
    };
    match rules.conditions().rampage.counter {
        RampageCounter::Gen1Uses => {
            locked(frames.first().copied().flatten()).map_or(2, |uses| uses.clamp(2, 3) as u16)
        }
        // Gen 2: the uses after the first (the drawn length less one).
        RampageCounter::Gen2Uses => locked(frames.first().copied().flatten())
            .map_or(2, |more| (more as u16 + 1).clamp(2, 3)),
        RampageCounter::Cartridge => {
            let frame = frames.first().copied().flatten();
            // The end of the turn's count-down has not run at a replacement request the
            // turn's faint made (Showdown: before the residual), nor when the battle was decided
            // in the residual before it (`ticked`: `timed_volatiles_left_unticked`, R-M8-121).
            let ticked = ticked && frame.is_some_and(residual_passed);
            locked(frame).map_or(2, |count| (count as u16 + u16::from(ticked)).clamp(2, 3))
        }
        RampageCounter::HiddenLength => {
            // The user by its place in the party: an Ally Switch moves it to the other slot,
            // where the later turns' actions, residuals and frames name it (R-M8-152).
            let member = frames
                .first()
                .copied()
                .flatten()
                .and_then(|frame| frame["sides"][who.side]["active"][who.slot?].as_u64());
            fn slot_of(frame: &Value, who: &Who, member: Option<u64>) -> Option<usize> {
                match member {
                    Some(member) => frame["sides"][who.side]["active"]
                        .as_array()?
                        .iter()
                        .position(|index| index.as_u64() == Some(member)),
                    None => who.slot,
                }
            }
            // The user as `frame` shows it: none once it has left the field.
            fn user_in<'f>(frame: &'f Value, who: &Who, member: Option<u64>) -> Option<&'f Value> {
                match member {
                    Some(member) => slot_of(frame, who, Some(member))
                        .map(|_| &frame["sides"][who.side]["party"][member as usize]),
                    None => frame_pokemon(frame, who),
                }
            }
            let slot_in = |frame: &Value| slot_of(frame, who, member);
            let locked = |frame: Option<&Value>| {
                frame
                    .and_then(|frame| user_in(frame, who, member))
                    .and_then(|pokemon| pokemon["volatiles"]["lockedmove"].as_u64())
            };
            // The user's stopped action at a replacement request mid-turn (a Baton Pass after it):
            // the end of that turn, in the next boundary, still has to tell.
            let mut carried: Option<&ActionOracle> = None;
            for (index, frame) in frames.iter().enumerate().skip(1) {
                // Where it stood as the turn ran: its slot at the boundary, or — dragged out or
                // gone by then — the one it had at the boundary before.
                let user = Who {
                    side: who.side,
                    slot: frame
                        .and_then(slot_in)
                        .or_else(|| frames[index - 1].and_then(slot_in))
                        .or(who.slot),
                    name: who.name.clone(),
                };
                if residuals.get(index).is_some_and(|r| {
                    r.iter()
                        .any(|(w, k)| *w == user && *k == ResidualKind::Rampage)
                }) {
                    return 2;
                }
                let action = decisions
                    .get(index)
                    .and_then(|actions| actions.iter().find(|action| action.user == user))
                    .or(carried.take());
                if let Some(action) = action {
                    if action.fatigue {
                        return 2;
                    }
                    if locked(*frame).is_some() {
                        // A turn the lock was stopped (Disable's `cant`) ends it with the turn's
                        // residual, not in the action (no `onRestart`, no `onAfterMove`): before
                        // that, the lock is still there whatever its length — a 2 shows its
                        // fatigue there (R-M8-105). A use that went on decided in its AfterMove.
                        if action.cant.is_some()
                            && frame.is_some_and(|frame| !residual_passed(frame))
                        {
                            carried = Some(action);
                            continue;
                        }
                        return 3;
                    }
                    // Off the field by the boundary after it moved (a Dancer's dance knocked it
                    // out; a Red Card dragged it out, another standing in its slot, R-M8-136): its
                    // lock went through AfterMove, where a 2 shows its fatigue — unless it was
                    // confused already, when either length leaves the same (R-M8-78).
                    let in_slot = |frame: Option<&Value>| {
                        frame.and_then(|frame| {
                            frame["sides"][who.side]["active"][who.slot?].as_u64()
                        })
                    };
                    let gone = frame.is_some_and(|frame| {
                        user_in(frame, who, member).is_none_or(|p| p["hp"] == 0)
                    }) || (member.is_none()
                        && in_slot(*frame) != in_slot(frames[index - 1]));
                    if gone {
                        let confused_before = frames[index - 1].is_some_and(|frame| {
                            user_in(frame, who, member)
                                .is_some_and(|p| p["volatiles"].get("confusion").is_some())
                        });
                        return if confused_before { 2 } else { 3 };
                    }
                    let landed = action.damages.iter().any(|(target, _)| *target != user)
                        || action.substitute_hits.iter().any(|hit| *hit);
                    // A lock gone silently from a user asleep and not confused at the
                    // boundary went on after its hit and lost to the sleep at the end of the
                    // turn (`lockedmove.onResidual`): 3. (Confused already, the user would
                    // show a 2's fatigue no more — either length leaves the same then.)
                    let asleep_unconfused = frame.is_some_and(|frame| {
                        user_in(frame, who, member).is_some_and(|pokemon| {
                            pokemon["status"]["kind"] == "slp"
                                && pokemon["volatiles"].get("confusion").is_none()
                        })
                    });
                    return if landed && !asleep_unconfused { 2 } else { 3 };
                }
                // Gone with no action of its user's this turn — one Sky Drop carries cannot move,
                // and its `onFoeBeforeMove` says nothing — and no fatigue in the residual (above):
                // its `duration` ran out on a hidden length still above 1, so 3 (R-M10-76). A
                // fatigue the confusion could not take, or a lock the user lost leaving the field,
                // leaves the same with either.
                if locked(*frame).is_none() {
                    return 3;
                }
            }
            2
        }
    }
}

/// Recovers Bide's draw (§15.10): the user's `bide` at the boundary — Gen 1's `time`, which only
/// its later turns count down, or Gen 2's `duration` plus the end-of-turn tick. Gone by the
/// boundary (the user switched out or fainted), the shortest reproduces the frames.
fn bide_duration_hint(
    oracle_frame: &Value,
    rules: &ResolvedRules,
    user: &Who,
    ticked: bool,
) -> u16 {
    let bide = rules.conditions().retaliation.bide;
    let side = &oracle_frame["sides"][user.side];
    let left = user
        .slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|index| side["party"][index as usize]["volatiles"]["bide"].as_u64());
    // `ticked`: not when a battle the residual decided stopped before Bide's `duration`
    // (`MissedTick::Bide`, R-M8-158).
    let ticked = u16::from(!bide.gen1 && ticked && residual_passed(oracle_frame));
    match left {
        Some(turns) => {
            (turns as u16 + ticked).clamp(u16::from(bide.turns.0), u16::from(bide.turns.1))
        }
        None => u16::from(bide.turns.0),
    }
}

/// Recovers the turns Gen 3-4 Uproar drew as it started (`UproarRules::turns`): the user's
/// `uproar` at the boundary, plus the tick its turn-end step already took — none when a battle
/// the residual decided stopped before it (`ticked`: `timed_volatiles_left_unticked`,
/// R-M8-163). Gone by then (the user fainted, or Gen 3 cancelled it): any value behaves the
/// same, so the shortest.
fn uproar_turns_hint(oracle_frame: &Value, rules: &ResolvedRules, user: &Who, ticked: bool) -> u16 {
    let (low, high) = rules.conditions().uproar.turns;
    let side = &oracle_frame["sides"][user.side];
    let left = user
        .slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|index| side["party"][index as usize]["volatiles"]["uproar"].as_u64());
    let ticked = u16::from(ticked && residual_passed(oracle_frame));
    match left {
        Some(turns) => (turns as u16 + ticked).clamp(u16::from(low), u16::from(high)),
        None => u16::from(low),
    }
}

/// Recovers the sleep `time` Showdown rolled when `all[turn][index]` put `victim` to sleep:
/// one more than the number of `cant slp` actions before the `-curestatus slp`. If the victim
/// is still asleep when the scenario ends, any time of at least that many counts behaves
/// identically within the scenario, so the smallest such value (capped by the generation's
/// maximum) is used. Each `cant slp` took its turn's `step_at` off the time (Early Bird: 2 —
/// a Skill Swap may give it or take it away during the sleep), so a wake-up after those says
/// the time was above what they took and at most one more step; the top of that range is used.
#[allow(clippy::too_many_arguments)]
fn sleep_duration_hint(
    all: &[Vec<ActionOracle>],
    entries: &[Vec<(usize, Who)>],
    residuals: &[Vec<(Who, ResidualKind)>],
    turn: usize,
    index: usize,
    victim: &Who,
    resets_on_switch_in: bool,
    sleep_usable_turns_come_back: bool,
    min_time: u16,
    max_time: u16,
    step_at: &dyn Fn(usize) -> u16,
) -> Option<u16> {
    // The same Pokémon by side and name: it may come back into another slot after a switch.
    let same = |who: &Who| who.side == victim.side && who.name == victim.name;
    let mut cants: u16 = 0;
    // What the `cant slp`s took off the time so far.
    let mut taken: u16 = 0;
    // Gen 5 restarts the count at each entry; a stretch that ended without waking up still
    // says the rolled time is longer than it.
    let mut at_least: u16 = 0;
    // Gen 3: the turns spent on Snore / Sleep Talk since sleep last stopped it, given back at
    // the next entry (`skippedTime`, R-M8-79).
    let mut skipped: u16 = 0;
    for (t, decisions) in all.iter().enumerate().skip(turn) {
        let start = if t == turn { index + 1 } else { 0 };
        let entered = entries.get(t).map_or(&[][..], Vec::as_slice);
        for position in start..=decisions.len() {
            let comes_in = entered
                .iter()
                .any(|(before, who)| *before == position && same(who));
            // Gen 5: the counter goes back to the rolled time as the sleeper comes in.
            if resets_on_switch_in && comes_in {
                at_least = at_least.max(taken + 1);
                cants = 0;
                taken = 0;
            }
            if sleep_usable_turns_come_back && comes_in {
                // The sleep the turns given back had not ended still says the time is longer
                // than what was taken before them (R-M8-152).
                at_least = at_least.max(taken + 1);
                taken = taken.saturating_sub(skipped);
                skipped = 0;
            }
            let Some(action) = decisions.get(position) else {
                // The turn's residual cured it (Healer, Shed Skin, an uproar) or put it to sleep
                // anew (Yawn): the sleep ended without a wake-up of its own, so any time above
                // the turns seen behaves the same.
                if residuals.get(t).is_some_and(|done| {
                    done.iter().any(|(who, kind)| {
                        same(who)
                            && matches!(
                                kind,
                                ResidualKind::Healer
                                    | ResidualKind::ShedSkin
                                    | ResidualKind::Yawn
                                    | ResidualKind::Uproar
                            )
                    })
                }) {
                    return Some((taken + 1).max(at_least).clamp(min_time, max_time));
                }
                break;
            };
            // Put to sleep again, or cured by another's move (Purify): the first sleep ended
            // without a wake-up of its own (Shed Skin, a switch-out cure …), so any time above
            // the turns seen behaves the same.
            if action
                .applied
                .iter()
                .any(|(who, kind)| same(who) && (kind == "status:slp" || kind == "curestatus:slp"))
            {
                return Some((taken + 1).max(at_least).clamp(min_time, max_time));
            }
            if !same(&action.user) {
                continue;
            }
            // Gen 3-4 (issues.md I63): an uproar woke it as it was about to move, before this
            // BeforeMove counted: any time that does not run out here behaves the same.
            if action
                .applied
                .iter()
                .any(|(who, kind)| same(who) && kind == "uproar:slp")
            {
                return Some(
                    (taken + step_at(t) + 1)
                        .max(at_least)
                        .clamp(min_time, max_time),
                );
            }
            if action.cant.as_deref() == Some("slp") || action.slept_through {
                cants += 1;
                taken += step_at(t);
                skipped = if action.slept_through { skipped + 1 } else { 0 };
                continue;
            }
            if action.cured.as_deref() == Some("slp") {
                // Early Bird's range may reach past the generation's maximum.
                return Some((taken + step_at(t)).min(max_time.max(cants + 1)));
            }
        }
    }
    // Never woke within the scenario (switched out, knocked out, battle over): any time above
    // the sleeping turns seen behaves the same; keep it inside the generation's range.
    Some((taken + 1).max(at_least).clamp(min_time, max_time))
}

/// How much each BeforeMove takes off `victim`'s sleep (Early Bird: 2), turn by turn: as the
/// engine has it now for this turn, and for a later one as the frame before it
/// (`boundary_frames[t - 1]`) shows the Pokémon — its ability (a Skill Swap during the sleep),
/// Gastro Acid, and a Neutralizing Gas on the field.
fn sleep_steps<'a>(
    state: &'a BattleState,
    rules: &'a ResolvedRules,
    victim: &'a Who,
    turn: usize,
    boundary_frames: &'a [Option<&'a Value>],
) -> impl Fn(usize) -> u16 + 'a {
    let id = who_to_id(state, victim);
    let pokemon = id.and_then(|id| state.pokemon(id));
    let now = pokemon.is_some_and(|p| {
        pokemon_battle_engine::engine::ability_active(state, rules, p)
            == Some(pokemon_battle_engine::ids::AbilityId::EARLY_BIRD)
    });
    let party = id.and_then(|id| {
        state
            .side(if victim.side == 0 {
                SideId::P1
            } else {
                SideId::P2
            })
            .team
            .iter()
            .position(|member| *member == id)
    });
    move |t: usize| {
        let frame = (t > turn)
            .then(|| t.checked_sub(1))
            .flatten()
            .and_then(|before| boundary_frames.get(before).copied().flatten());
        let shown = frame
            .zip(party)
            .map(|(frame, party)| &frame["sides"][victim.side]["party"][party]);
        let early = match (frame, shown) {
            // As the frame before that turn has it: the ability it shows (a Skill Swap during
            // the sleep), not set aside by Gastro Acid nor by a Neutralizing Gas on the field
            // (one that has left since the sleep began counts no more, R-M8-126) — save behind
            // Ability Shield.
            (Some(frame), Some(entry)) if entry["ability"].as_str().is_some() => {
                entry["ability"] == "earlybird"
                    && entry["volatiles"].get("gastroacid").is_none()
                    && (entry["item"] == "abilityshield" || !neutralizing_gas_shown(frame))
            }
            _ => now,
        };
        if early { 2 } else { 1 }
    }
}

/// Whether a boundary frame shows a Neutralizing Gas at work: an active Pokémon standing with the
/// ability, not under Gastro Acid.
fn neutralizing_gas_shown(frame: &Value) -> bool {
    frame["sides"].as_array().is_some_and(|sides| {
        sides.iter().any(|side| {
            side["active"].as_array().is_some_and(|active| {
                active.iter().filter_map(Value::as_u64).any(|index| {
                    let pokemon = &side["party"][index as usize];
                    pokemon["ability"] == "neutralizinggas"
                        && pokemon["hp"].as_u64().is_some_and(|hp| hp > 0)
                        // (A volatile with no value shows as `null`: its key is what counts.)
                        && pokemon["volatiles"].get("gastroacid").is_none()
                })
            })
        })
    })
}

/// One hit that reached a target's body, for the chance reactions that follow it.
struct ChanceReactionHit<'a> {
    state: &'a BattleState,
    rules: &'a ResolvedRules,
    actor: PokemonId,
    target: PokemonId,
    move_def: &'a ResolvedMove,
    oracle: &'a ActionOracle,
    behind_substitute: bool,
    /// The HP the hit took off its target.
    dealt: u16,
    /// Whether a reaction reported after this many hits belongs to this hit (R-M8-134: not a
    /// later one, whose roll failed — Mental Herb may have cured what an earlier one took).
    reported_after: &'a dyn Fn(usize) -> bool,
    sleep_duration: Option<u16>,
    /// The user still has the ability it attacked with: no Mummy (or its kind) took it on an
    /// earlier hit (`ActionOracle::ability_taken_after`).
    user_keeps_ability: bool,
}

/// The chance reactions after a hit's ordered ones (engine `hit_reactions`), in the engine's
/// order: the target's Effect Spore or Cursed Body, then the user's Poison Touch / Toxic Chain.
/// The one that took is the one reported after this hit; the other hits' rolls failed (a later
/// one changes nothing while the status stays; Cursed Body stops rolling once the move is
/// disabled). A hit on a Substitute reacts to
/// nothing, but a candidate damage roll that breaks it sooner meets them: optional decisions.
fn chance_reactions(
    staged: &mut StagedTape,
    hit: ChanceReactionHit<'_>,
    cursed_body_done: &mut bool,
) {
    use pokemon_battle_engine::rules::EffectSporeRoll;
    let ChanceReactionHit {
        state,
        rules,
        actor,
        target,
        move_def,
        oracle,
        behind_substitute,
        dealt,
        reported_after,
        sleep_duration,
        user_keeps_ability,
    } = hit;
    let named = |who: &Option<Who>, id: PokemonId| {
        who.as_ref()
            .is_none_or(|who| who_to_id(state, who) == Some(id))
    };
    let push = |staged: &mut StagedTape, decision: RandomOverride| {
        staged.push(
            Stage::Reaction,
            if behind_substitute {
                decision.optional()
            } else {
                decision
            },
        );
    };
    let mut reactors = pokemon_battle_engine::engine::chance_reactors(state, rules, actor, target);
    // Core Enforcer's `onHit` runs before the hit's `DamagingHit` (`runMoveEffects`): an
    // ability it suppresses reacts to nothing.
    if !behind_substitute
        && move_def.behavior.hit_or_sub
            == Some(pokemon_battle_engine::effects::moves::HitOrSubEffect::SuppressAbilityOfMoved)
        && pokemon_battle_engine::engine::core_enforcer_suppresses(state, rules, target)
    {
        reactors.holder = None;
    }
    if let Some(roll) = pokemon_battle_engine::engine::effect_spore_roll(
        state, rules, reactors, actor, move_def, dealt,
    ) {
        let shown = oracle
            .contact_status
            .as_deref()
            .filter(|_| {
                !behind_substitute
                    && named(&oracle.contact_status_of, target)
                    && oracle
                        .contact_status_hits
                        .iter()
                        .any(|after| reported_after(*after))
            })
            .and_then(|shown| {
                [
                    NonVolatileStatus::Sleep,
                    NonVolatileStatus::Paralysis,
                    NonVolatileStatus::Poison,
                ]
                .into_iter()
                .find(|status| status_id(*status) == shown)
            });
        match roll {
            EffectSporeRoll::ChanceThenSample(_) => {
                push(
                    staged,
                    RandomOverride::bool(RandomRequestKind::HitReaction, shown.is_some()),
                );
                if let Some(status) = shown {
                    let index = pokemon_battle_engine::engine::EFFECT_SPORE_SAMPLE
                        .iter()
                        .position(|s| *s == status)
                        .unwrap_or(0);
                    push(
                        staged,
                        RandomOverride::range(RandomRequestKind::Sample, index as u16),
                    );
                }
            }
            EffectSporeRoll::Percent => {
                let value = (0..100)
                    .find(|roll| pokemon_battle_engine::engine::effect_spore_status(*roll) == shown)
                    .unwrap_or(99);
                push(
                    staged,
                    RandomOverride::range(RandomRequestKind::EffectSpore, value),
                );
            }
        }
        if shown == Some(NonVolatileStatus::Sleep)
            && let Some(duration) = sleep_duration
        {
            push(
                staged,
                RandomOverride::range(RandomRequestKind::Duration, duration),
            );
        }
    }
    if !*cursed_body_done
        && pokemon_battle_engine::engine::cursed_body_rolls(state, reactors, actor, move_def)
    {
        let took = !behind_substitute
            && oracle
                .cursed_body
                .as_ref()
                .is_some_and(|(after, of)| named(of, target) && reported_after(*after));
        push(
            staged,
            RandomOverride::bool(RandomRequestKind::HitReaction, took),
        );
        *cursed_body_done = took;
    }
    if pokemon_battle_engine::engine::cute_charm_rolls(
        state, rules, reactors, actor, move_def, dealt,
    ) {
        let took = !behind_substitute
            && oracle.cute_charm.as_ref().is_some_and(|(_, of)| {
                named(of, target)
                    && oracle
                        .cute_charm_hits
                        .iter()
                        .any(|after| reported_after(*after))
            });
        push(
            staged,
            RandomOverride::bool(RandomRequestKind::HitReaction, took),
        );
    }
    if user_keeps_ability
        && pokemon_battle_engine::engine::dealt_hit_status(state, rules, reactors, target, move_def)
            .is_some()
    {
        let took = !behind_substitute
            && oracle.dealt_status.iter().any(|(who, after)| {
                who_to_id(state, who) == Some(target) && reported_after(*after)
            });
        push(
            staged,
            RandomOverride::bool(RandomRequestKind::HitReaction, took),
        );
    }
}

/// Whether an effect label shows `stat` changed, in either direction (Contrary turns a boost
/// into a drop): `boost:spd`, `unboost:spd`.
fn stat_changed(label: &str, stat: pokemon_battle_engine::state::Stat) -> bool {
    use pokemon_battle_engine::state::Stat;
    let id = match stat {
        Stat::Attack => "atk",
        Stat::Defense => "def",
        Stat::SpecialAttack => "spa",
        Stat::SpecialDefense => "spd",
        Stat::Speed => "spe",
        Stat::Accuracy => "accuracy",
        Stat::Evasion => "evasion",
    };
    label
        .strip_prefix("boost:")
        .or_else(|| label.strip_prefix("unboost:"))
        == Some(id)
}

/// Showdown's id for a major status (`-status|p|brn`).
pub(super) fn status_id(status: NonVolatileStatus) -> &'static str {
    match status {
        NonVolatileStatus::Burn => "brn",
        NonVolatileStatus::Freeze => "frz",
        NonVolatileStatus::Paralysis => "par",
        NonVolatileStatus::Poison => "psn",
        NonVolatileStatus::Toxic => "tox",
        NonVolatileStatus::Sleep => "slp",
    }
}

fn normalise_oracle_frame(frame: &Value) -> Value {
    let mut frame = frame.clone();
    // Showdown reports one request kind per side (`wait|forced_switch`); the engine's request
    // is the side that is asked, so the first non-`wait` part is the comparable one.
    if let Some(request) = frame["request"].as_str() {
        let parts: Vec<&str> = request.split('|').collect();
        let asked = parts
            .iter()
            .find(|part| **part != "wait")
            .or(parts.first())
            .map(|part| part.to_string())
            .unwrap_or_default();
        frame["request"] = Value::String(asked);
    }
    // Each observer's knowledge (`views`, M10.4c) is compared on its own, against the
    // `BattleView`s.
    frame.as_object_mut().map(|o| {
        o.remove("events");
        o.remove("views")
    });
    for side in frame
        .get_mut("sides")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        for pokemon in side
            .get_mut("party")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(doll) = pokemon
                .get_mut("volatiles")
                .and_then(|volatiles| volatiles.get_mut("substitute"))
                && let Some(hp) = substitute_hp_value(doll)
            {
                *doll = Value::from(hp);
            }
        }
    }
    frame
}

/// A Substitute's HP in a Showdown frame, as the engine keeps it. Showdown's Gen 3+ Psywave
/// leaves a fraction on a doll: its `damageCallback` is not rounded (the HP damage is, in
/// `spreadDamage`), and `substitute.onTryPrimaryHit` subtracts it as it is. The cartridge's
/// damage is whole, and a doll at 1.79 breaks under the same whole hits as one at 2, so the
/// fraction (in hundredths: Psywave's `random(50, 151) × level / 100`) reads rounded up
/// (DESIGN.md R72).
fn substitute_hp_value(doll: &Value) -> Option<u16> {
    let hundredths = (doll.as_f64()? * 100.0).round() as u32;
    Some(hundredths.div_ceil(100) as u16)
}

fn compare_frame(
    report: &mut ReplayReport,
    checkpoint: &str,
    rust: &super::SemanticFrame,
    oracle: &Value,
) {
    let (rust, diffs) = frame_diffs(rust, oracle);
    report.rust_frames.push(rust);
    report.frames_compared += 1;
    for diff in diffs.iter().take(10) {
        report.divergences.push(format!(
            "{checkpoint}: {} rust={} showdown={}",
            diff.path, diff.actual, diff.expected
        ));
    }
}

/// The engine's frame as compared, and where it differs from the oracle's.
fn frame_diffs(
    rust: &super::SemanticFrame,
    oracle: &Value,
) -> (Value, Vec<super::compare::SemanticDiff>) {
    let mut rust = serde_json::to_value(rust).expect("serialize frame");
    let expected = normalise_oracle_frame(oracle);
    // A recording made before the frames carried `types` / `ability` / `item` (2026-10-01),
    // `pp` (2026-10-02) or `species` (2026-10-04) has none: those fields are not compared.
    for (side, sides) in expected["sides"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (index, recorded) in sides["party"].as_array().into_iter().flatten().enumerate() {
            for field in ["types", "ability", "item", "pp", "species"] {
                if recorded.get(field).is_none()
                    && let Some(pokemon) = rust["sides"][side]["party"][index].as_object_mut()
                {
                    pokemon.remove(field);
                }
            }
        }
    }
    // Showdown is the expected side, the engine the actual one.
    let diffs = compare_json(&expected, &rust, &["$.checkpoint"]);
    (rust, diffs)
}

/// Every HP change one step committed, in order (damage, recoil, drain, self heal).
fn substitute_hp(state: &BattleState, pokemon: PokemonId) -> Option<u16> {
    state
        .pokemon(pokemon)?
        .volatiles
        .entries()
        .iter()
        .find_map(|volatile| match volatile {
            VolatileCondition::Substitute { hp, .. } => Some(*hp),
            _ => None,
        })
}

/// The Substitute HP each Pokémon whose Substitute `decisions[index]` struck must be left
/// Each Rage user's rage at the boundary (`ActionOracle::raging_after`): nothing a later action
/// of the turn does starts or ends a Gen 3 rage, so the frame tells what its Rage left —
/// raging after a passed accuracy check, not after a miss (a fainted user shows neither).
fn attribute_frame_rage(decisions: &mut [ActionOracle], oracle_frame: &Value) {
    for decision in decisions.iter_mut() {
        if decision.move_name.as_deref() != Some("Rage") {
            continue;
        }
        let side = &oracle_frame["sides"][decision.user.side];
        decision.raging_after = decision
            .user
            .slot
            .and_then(|slot| side["active"][slot].as_u64())
            .map(|party| {
                side["party"][party as usize]["volatiles"]
                    .as_object()
                    .is_some_and(|volatiles| volatiles.contains_key("rage"))
            });
    }
}

/// with: gone if the hit broke it; otherwise, when no later action this turn strikes it
/// again, what the boundary frame shows (nothing to say if it is gone by then). A Gen 4
/// foreseen attack that strikes it in the residual deals the damage stored at use: the frame's
/// HP plus that, when the doll outlasts the strike (R-M8-164).
fn substitute_hint(
    state: &BattleState,
    rules: &ResolvedRules,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    index: usize,
) -> Vec<(PokemonId, Option<u16>)> {
    let mut hint = Vec::new();
    for (holder, broken) in &decisions[index].substitute_struck {
        let Some(id) = who_to_id(state, holder) else {
            continue;
        };
        if hint.iter().any(|(seen, _)| *seen == id) {
            hint.retain(|(seen, _)| *seen != id);
        }
        if *broken {
            hint.push((id, None));
            continue;
        }
        if decisions[index + 1..]
            .iter()
            .any(|later| later.substitute_struck.iter().any(|(who, _)| who == holder))
        {
            continue;
        }
        let side = &oracle_frame["sides"][holder.side];
        let at_boundary = holder
            .slot
            .and_then(|slot| side["active"][slot].as_u64())
            .and_then(|party| {
                substitute_hp_value(&side["party"][party as usize]["volatiles"]["substitute"])
            });
        let foreseen = if residual_substitute_strike(oracle_frame, holder) {
            match foreseen_strike_damage(state, rules, holder) {
                Some(damage) => Some(damage),
                None => continue,
            }
        } else {
            None
        };
        if let Some(hp) = at_boundary {
            hint.push((id, Some(hp + foreseen.unwrap_or(0))));
        }
    }
    hint
}

/// The damage a Gen 4 foreseen attack (`ForeseenRoll::AtUse`) on `holder`'s slot stored at use
/// (§15.22): what its strike deals, a fixed amount. None in other generations, whose strike
/// draws its random factor (or computes the damage) as it strikes.
fn foreseen_strike_damage(state: &BattleState, rules: &ResolvedRules, holder: &Who) -> Option<u16> {
    if rules.conditions().future_attack.damage_at_use
        != Some(pokemon_battle_engine::rules::ForeseenRoll::AtUse)
    {
        return None;
    }
    let side = if holder.side == 0 {
        SideId::P1
    } else {
        SideId::P2
    };
    state
        .side(side)
        .future_attacks
        .iter()
        .find(|attack| attack.slot == holder.slot.unwrap_or(0))
        .and_then(|attack| attack.damage)
}

/// The Substitute HP the frame shows on the Pokémon in `who`'s place, if it has one.
fn frame_substitute_hp(oracle_frame: &Value, who: &Who) -> Option<u16> {
    let side = &oracle_frame["sides"][who.side];
    who.slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|party| {
            substitute_hp_value(&side["party"][party as usize]["volatiles"]["substitute"])
        })
}

/// Whether a foreseen attack strikes `holder`'s Substitute in the boundary's residual
/// (`-activate|holder|move: Substitute|[damage]` or its `-end` after the `residual` marker): the
/// frame's Substitute HP is after that strike, not after the turn's last action (R-M8-84).
fn residual_substitute_strike(oracle_frame: &Value, holder: &Who) -> bool {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .skip_while(|event| event["kind"] != "residual")
        .any(|event| {
            matches!(event["kind"].as_str(), Some("activate" | "end"))
                && event["args"][1]
                    .as_str()
                    .is_some_and(|what| what.ends_with("Substitute"))
                && who_from_text(&event["args"][0]).as_ref() == Some(holder)
        })
}

fn damages_in(engine: &BattleEngine, from: usize) -> Vec<(PokemonId, u16)> {
    engine.trace().events()[from..]
        .iter()
        .filter_map(|event| match event {
            // Regenerator heals in silence (`pokemon.heal` with no message): a holder a move
            // dragged out (Circle Throw) shows no HP line for it (R-M8-106).
            TraceEvent::HealApplied {
                source:
                    pokemon_battle_engine::engine::EffectSource::Ability {
                        ability: pokemon_battle_engine::ids::AbilityId::REGENERATOR,
                        ..
                    },
                ..
            } => None,
            TraceEvent::DamageApplied {
                target, hp_after, ..
            }
            | TraceEvent::HealApplied {
                target, hp_after, ..
            } => Some((*target, *hp_after)),
            _ => None,
        })
        .collect()
}

/// What `search_rolls` found for one action.
struct RollSearch {
    /// The engine after the action, its step, and the complete tape that produced it.
    chosen: Option<(BattleEngine, EngineStep, Vec<RandomOverride>)>,
    /// The attempts of the slot that could not be matched (for the report).
    seen: Vec<(u16, Vec<(PokemonId, u16)>)>,
    /// Other complete tapes that reproduce the action just as well but leave a deferred
    /// Substitute (struck again later in the boundary, so the frame says nothing about it
    /// yet) with other HP: one per HP value, for the backtracking in `drive_to_boundary`.
    alternatives: Vec<Vec<RandomOverride>>,
}

/// How well a roll set reproduces an action: the hits whose landing on a Substitute or a body
/// matches the log, in order; the HP commits matched in order; whether the count matches; the
/// Substitutes matched; and how close the rest are.
type RollScore = (usize, usize, bool, usize, std::cmp::Reverse<u32>);

/// A turn submitted with an undecided Gen 2-3 Quick Claw roll: the snapshot before it, the
/// choices, the tape used and the other outcome's.
type TurnRetry = (
    BattleSnapshot,
    [SideChoice; 2],
    Vec<RandomOverride>,
    Vec<RandomOverride>,
);

/// A damage roll left open for the search, but optional: the hit may not draw it (DESIGN.md
/// R-M8-84). Marked by a value no roll takes; `search_rolls` fills it like an open slot.
fn open_optional_roll() -> RandomOverride {
    RandomOverride::range(RandomRequestKind::DamageRoll, u16::MAX).optional()
}

fn is_open_optional_roll(entry: &Option<RandomOverride>) -> bool {
    entry.is_some_and(|entry| entry == open_optional_roll())
}

/// The holds on 1 HP an action's log names: whose Focus Sash held (`-enditem|p|Focus Sash`), whose
/// Focus Band (`-activate|p|item: Focus Band`) and whose Sturdy (`-ability|p|Sturdy`). A roll that
/// leaves a Pokémon on 1 by itself shows the same HP; the search keeps the rolls under which each
/// held where the log says, nowhere else (R-M10-41: a hold names its item or ability).
#[derive(Debug, Default, PartialEq, Eq)]
struct ShownHolds {
    sashes: Vec<PokemonId>,
    bands: Vec<PokemonId>,
    sturdy: Vec<PokemonId>,
}

impl ShownHolds {
    /// The holds a candidate's trace shows (its `DamageReduced` by each; Gen 4's sash holds
    /// every hit of the move).
    fn of(events: &[TraceEvent]) -> Self {
        use pokemon_battle_engine::engine::EffectSource;
        use pokemon_battle_engine::ids::{AbilityId, ItemId};
        let mut holds = Self::default();
        for event in events {
            let TraceEvent::DamageReduced { target, by, .. } = event else {
                continue;
            };
            let list = match by {
                EffectSource::Item {
                    item: ItemId::FOCUS_SASH,
                    ..
                } => &mut holds.sashes,
                EffectSource::Item {
                    item: ItemId::FOCUS_BAND,
                    ..
                } => &mut holds.bands,
                EffectSource::Ability {
                    ability: AbilityId::STURDY,
                    ..
                } => &mut holds.sturdy,
                _ => continue,
            };
            list.push(*target);
        }
        holds.settled()
    }

    /// Each holder once, in order.
    fn settled(mut self) -> Self {
        for list in [&mut self.sashes, &mut self.bands, &mut self.sturdy] {
            list.sort();
            list.dedup();
        }
        self
    }
}

/// Recovers the damage rolls of one action. Each open slot (`None`) in the template is a hit's
/// roll; a hit's roll only affects the HP commits from that hit on, so the slots are settled
/// front to back (in three sweeps), keeping for each the roll that extends the matching prefix
/// of Showdown's HP sequence the furthest. Returns the engine that reproduced the whole
/// sequence, or the attempts of the slot that could not be matched.
#[allow(clippy::too_many_arguments)]
fn search_rolls(
    compiled: &Compiled,
    before: &BattleSnapshot,
    trace_len: usize,
    template: &[Option<RandomOverride>],
    expected: &[(PokemonId, u16)],
    hits: &[(PokemonId, bool)],
    substitutes: &[(PokemonId, Option<u16>)],
    deferred: &[PokemonId],
    holds: &ShownHolds,
    unused: &mut Option<String>,
) -> RollSearch {
    let damage = compiled.rules.damage();
    let candidates: Vec<u16> = (damage.random_min..=damage.random_max).collect();
    let open = template
        .iter()
        .filter(|entry| entry.is_none() || is_open_optional_roll(entry))
        .count();
    let mut rolls = vec![damage.random_max; open];
    let mut seen = Vec::new();

    // Gen 1-2 roll no variance for a damage of 1 (Showdown `damage > 1`): the plan cannot know
    // before the damage is computed, so their rolls are optional decisions.
    let optional_roll = matches!(
        compiled.rules.damage().algorithm,
        pokemon_battle_engine::rules::DamageAlgorithm::Gen1Legacy
            | pokemon_battle_engine::rules::DamageAlgorithm::Gen2
    );
    // A roll set can make the engine ask for a decision out of the plan's order — a berry the
    // HP threshold feeds one hit early because the rolls are too high (DESIGN.md R269) — and
    // the plan's tape panics: such a set is only a candidate that does not reproduce the action.
    // The first such panic is raised again when no roll set ran at all.
    let first_panic: std::cell::RefCell<Option<Box<dyn std::any::Any + Send>>> =
        std::cell::RefCell::new(None);
    let ran = std::cell::Cell::new(false);
    let run = |rolls: &[u16]| {
        let mut next = 0;
        let tape = template
            .iter()
            .map(|entry| {
                let optional = is_open_optional_roll(entry);
                match entry {
                    Some(entry) if !optional => *entry,
                    _ => {
                        let roll = rolls[next];
                        next += 1;
                        let decision = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
                        if optional_roll || optional {
                            decision.optional()
                        } else {
                            decision
                        }
                    }
                }
            })
            .collect::<Vec<_>>();
        let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut candidate = restore_with_tape(compiled, before, tape.clone());
            let step = candidate.step().expect("step");
            (candidate, step)
        }));
        match stepped {
            Ok((candidate, step)) => {
                ran.set(true);
                let dealt = damages_in(&candidate, trace_len);
                Some((candidate, step, dealt, tape))
            }
            Err(payload) => {
                first_panic.borrow_mut().get_or_insert(payload);
                None
            }
        }
    };
    let raise_if_none_ran = || {
        if !ran.get()
            && let Some(payload) = first_panic.borrow_mut().take()
        {
            std::panic::resume_unwind(payload);
        }
    };
    // A hit on a Substitute commits no HP: the boundary frame's Substitute HP settles its roll.
    // So does a hold on 1 HP the log names (`ShownHolds`: a sash's hold, not its loss — a sash
    // flung goes too).
    let substitutes_match = |candidate: &BattleEngine| {
        substitutes
            .iter()
            .all(|(id, hp)| substitute_hp(candidate.state(), *id) == *hp)
            && ShownHolds::of(&candidate.trace().events()[trace_len..]) == *holds
            // A deferred doll stood after this action (the log shows it struck, not broken):
            // a roll set that breaks it here leaves the later action that strikes it again
            // hitting the body (R-M8-183).
            && deferred
                .iter()
                .all(|id| substitute_hp(candidate.state(), *id).is_some())
    };
    let substitutes_matched = |candidate: &BattleEngine| {
        substitutes
            .iter()
            .filter(|(id, hp)| substitute_hp(candidate.state(), *id) == *hp)
            .count()
    };
    // How far the Substitutes' HP is from the frame's: several hits on one doll settle their
    // rolls together (Spike Cannon), so a roll that brings the sum closer is progress.
    let substitute_distance = |candidate: &BattleEngine| {
        substitutes
            .iter()
            .map(
                |(id, hp)| match (substitute_hp(candidate.state(), *id), *hp) {
                    (Some(actual), Some(want)) => u32::from(actual.abs_diff(want)),
                    (None, None) => 0,
                    _ => u32::from(u16::MAX),
                },
            )
            .sum::<u32>()
    };
    let prefix = |dealt: &[(PokemonId, u16)]| {
        dealt
            .iter()
            .zip(expected)
            .take_while(|(a, b)| a == b)
            .count()
    };
    // Where each of the move's hits landed, Substitute (`true`) or body, as the log shows it:
    // a roll that breaks a Substitute one hit too late can still match an HP commit of the
    // hit after, and must not win over the one that breaks it where Showdown did.
    let hits_matched = |candidate: &BattleEngine| {
        candidate.trace().events()[trace_len..]
            .iter()
            .filter_map(|event| match event {
                TraceEvent::SubstituteDamaged { target, .. } => Some((*target, true)),
                TraceEvent::DamageApplied {
                    target,
                    source: pokemon_battle_engine::engine::EffectSource::Move { pokemon, .. },
                    ..
                } if target != pokemon => Some((*target, false)),
                _ => None,
            })
            .zip(hits)
            .take_while(|(a, b)| a == *b)
            .count()
    };

    // A tape the engine did not use up planned a decision the engine never asked for: the
    // plan and the engine disagree about the rules, which the HP alone may not show.
    let mut used_up = |candidate: &BattleEngine| {
        let rng = &candidate.state().rng;
        if rng.semantic_tape_exhausted() {
            return true;
        }
        let asked = rng.semantic_tape_cursor();
        let left: Vec<String> = template[asked.min(template.len())..]
            .iter()
            .map(|entry| match entry {
                Some(decision) => format!("{:?}={:?}", decision.kind, decision.value),
                None => "DamageRoll".into(),
            })
            .collect();
        *unused = Some(format!(
            "asked {asked} of {} planned decisions; left {left:?}",
            template.len()
        ));
        if std::env::var_os("REPLAY_DEBUG").is_some() {
            for pokemon in before.state.all_pokemon() {
                if pokemon.status.is_some() {
                    eprintln!("    before: {:?} {:?}", pokemon.id(), pokemon.status);
                }
            }
            for event in &candidate.trace().events()[trace_len..] {
                eprintln!("    trace: {event:?}");
            }
        }
        false
    };

    // Every other roll set that reproduces the action as well, one per HP it leaves on the
    // deferred Substitutes. Each slot's rolls that change that HP on their own stand for its
    // outcomes (a hit's 4 or 5), and every combination of them is tried: three hits that must
    // all deal their least leave an HP no single roll reaches (R-M8-178).
    let alternatives = |rolls: &[u16], chosen: &BattleEngine| {
        let key = |engine: &BattleEngine| {
            deferred
                .iter()
                .map(|id| substitute_hp(engine.state(), *id))
                .collect::<Vec<_>>()
        };
        let mut keys = vec![key(chosen)];
        let mut found = Vec::new();
        if deferred.is_empty() {
            return found;
        }
        let outcomes: Vec<Vec<u16>> = (0..rolls.len())
            .map(|slot| {
                let mut seen = vec![key(chosen)];
                let mut kept = vec![rolls[slot]];
                for &roll in &candidates {
                    let mut other = rolls.to_vec();
                    other[slot] = roll;
                    if let Some((candidate, ..)) = run(&other)
                        && !seen.contains(&key(&candidate))
                    {
                        seen.push(key(&candidate));
                        kept.push(roll);
                    }
                }
                kept
            })
            .collect();
        // The combinations, as indices into each slot's outcomes (a bounded number).
        let combinations = outcomes
            .iter()
            .try_fold(1usize, |count, kept| count.checked_mul(kept.len()))
            .filter(|count| *count <= 4096)
            .unwrap_or(0);
        for mut index in 1..combinations {
            let mut other = Vec::with_capacity(rolls.len());
            for kept in &outcomes {
                other.push(kept[index % kept.len()]);
                index /= kept.len();
            }
            let Some((candidate, _, dealt, tape)) = run(&other) else {
                continue;
            };
            if dealt == expected
                && substitutes_match(&candidate)
                && candidate.state().rng.semantic_tape_exhausted()
                && !keys.contains(&key(&candidate))
            {
                keys.push(key(&candidate));
                found.push(tape);
            }
        }
        found
    };

    if open == 0 {
        let Some((candidate, step, dealt, tape)) = run(&rolls) else {
            raise_if_none_ran();
            unreachable!("a roll set that panicked");
        };
        if dealt == expected && substitutes_match(&candidate) && used_up(&candidate) {
            return RollSearch {
                chosen: Some((candidate, step, tape)),
                seen,
                alternatives: Vec::new(),
            };
        }
        seen.push((0, dealt));
        return RollSearch {
            chosen: None,
            seen,
            alternatives: Vec::new(),
        };
    }

    let score_of = |candidate: &BattleEngine, dealt: &[(PokemonId, u16)]| {
        (
            hits_matched(candidate),
            prefix(dealt),
            dealt.len() == expected.len(),
            substitutes_matched(candidate),
            std::cmp::Reverse(substitute_distance(candidate)),
        )
    };
    // Slot by slot, keep the roll that reproduces the longest prefix of Showdown's HP commits;
    // among equals, one that makes as many commits (a roll on a Substitute commits nothing, but
    // decides whether the next hit still lands on it), then one that leaves the Substitutes as
    // the boundary frame shows them, or closest to it. Hits whose rolls only add up on a doll
    // settle together, so the slots are swept again from the rolls the last sweep kept. The
    // sweeps start from the highest rolls; when they settle nowhere, they start again from the
    // uniform roll set that comes closest: hits on a doll can need every slot to move at once
    // (two hits of 12 where a hit's roll gives 10, 12 or 13), out of reach one slot at a time.
    for pass in 0..2 {
        if pass == 1 {
            if open < 2 {
                break;
            }
            let mut best: Option<(RollScore, u16)> = None;
            for &roll in &candidates {
                let uniform = vec![roll; open];
                let Some((candidate, step, dealt, tape)) = run(&uniform) else {
                    continue;
                };
                if dealt == expected && substitutes_match(&candidate) && used_up(&candidate) {
                    let alternatives = alternatives(&uniform, &candidate);
                    return RollSearch {
                        chosen: Some((candidate, step, tape)),
                        seen,
                        alternatives,
                    };
                }
                let score = score_of(&candidate, &dealt);
                if best.is_none_or(|(best_score, _)| score > best_score) {
                    best = Some((score, roll));
                }
            }
            if let Some((_, roll)) = best {
                rolls = vec![roll; open];
            }
        }
        for slot in (0..3).flat_map(|_| 0..open) {
            let mut best: Option<(RollScore, u16)> = None;
            seen.clear();
            for &roll in &candidates {
                rolls[slot] = roll;
                let Some((candidate, step, dealt, tape)) = run(&rolls) else {
                    seen.push((roll, Vec::new()));
                    continue;
                };
                if dealt == expected && substitutes_match(&candidate) && used_up(&candidate) {
                    let alternatives = alternatives(&rolls, &candidate);
                    return RollSearch {
                        chosen: Some((candidate, step, tape)),
                        seen,
                        alternatives,
                    };
                }
                if std::env::var_os("REPLAY_DEBUG_SUB").is_some() {
                    eprintln!(
                        "    slot {slot} rolls {rolls:?}: dealt {dealt:?}, substitutes {:?}",
                        substitutes
                            .iter()
                            .map(|(id, _)| substitute_hp(candidate.state(), *id))
                            .collect::<Vec<_>>()
                    );
                }
                if std::env::var_os("REPLAY_DEBUG").is_some() && dealt == expected {
                    eprintln!(
                        "    slot {slot} roll {roll}: HP matches, substitutes {} (want {substitutes:?})",
                        substitutes_match(&candidate)
                    );
                }
                // Substitutes count one by one: a spread move breaking two dolls settles each
                // target's roll in its own slot.
                let score = score_of(&candidate, &dealt);
                if best.is_none_or(|(best_score, _)| score > best_score) {
                    best = Some((score, roll));
                }
                seen.push((roll, dealt));
            }
            rolls[slot] = best.map_or(damage.random_max, |(_, roll)| roll);
        }
    }
    // Hits on a doll that only their sum settles can need several slots to move at once from
    // wherever the sweeps settled: two hits of 33 and 36 where, beside the other's kept roll,
    // each slot alone deals 31, 34 or 37 (a Rock Blast leaving a Substitute at 6, R-M10-79).
    // Every combination of the outcomes each slot has on its own, from the rolls the sweeps
    // kept — the rolls that leave the Substitutes a different HP stand for them — is tried (a
    // bounded number).
    if !substitutes.is_empty() && open >= 2 {
        let outcomes: Vec<Vec<u16>> = (0..open)
            .map(|slot| {
                let mut dolls: Vec<Vec<Option<u16>>> = Vec::new();
                let mut kept = Vec::new();
                for &roll in &candidates {
                    let mut other = rolls.clone();
                    other[slot] = roll;
                    if let Some((candidate, ..)) = run(&other) {
                        let doll: Vec<Option<u16>> = substitutes
                            .iter()
                            .map(|(id, _)| substitute_hp(candidate.state(), *id))
                            .collect();
                        if !dolls.contains(&doll) {
                            dolls.push(doll);
                            kept.push(roll);
                        }
                    }
                }
                kept
            })
            .collect();
        let combinations = outcomes
            .iter()
            .try_fold(1usize, |count, kept| count.checked_mul(kept.len()))
            .filter(|count| *count <= 4096)
            .unwrap_or(0);
        for mut index in 0..combinations {
            let mut other = Vec::with_capacity(open);
            for kept in &outcomes {
                other.push(kept[index % kept.len()]);
                index /= kept.len();
            }
            let Some((candidate, step, dealt, tape)) = run(&other) else {
                continue;
            };
            if dealt == expected && substitutes_match(&candidate) && used_up(&candidate) {
                let alternatives = alternatives(&other, &candidate);
                return RollSearch {
                    chosen: Some((candidate, step, tape)),
                    seen,
                    alternatives,
                };
            }
        }
    }
    raise_if_none_ran();
    RollSearch {
        chosen: None,
        seen,
        alternatives: Vec::new(),
    }
}

/// Replays `steps[from]` with `tape` and every later step with the tape it used, returning the
/// engine after the last one and the steps as rebuilt (their starting snapshots updated).
fn replay_steps(
    compiled: &Compiled,
    steps: &[ReplayedStep],
    from: usize,
    tape: &[RandomOverride],
) -> (BattleEngine, Vec<ReplayedStep>, Vec<TraceEvent>) {
    // What the replayed steps traced, in order: the log from `steps[from].log_len` on (R-M10-6).
    let mut events: Vec<TraceEvent> = Vec::new();
    let mut rebuilt = steps[..from].to_vec();
    let mut engine = restore_with_tape(compiled, &steps[from].before, tape.to_vec());
    engine.step().expect("step");
    rebuilt.push(ReplayedStep {
        before: steps[from].before.clone(),
        log_len: steps[from].log_len,
        tape: tape.to_vec(),
        alternatives: Vec::new(),
    });
    for step in &steps[from + 1..] {
        events.extend(engine.take_trace_events());
        let before = engine.snapshot();
        engine = restore_with_tape(compiled, &before, step.tape.clone());
        engine.step().expect("step");
        rebuilt.push(ReplayedStep {
            before,
            log_len: steps[from].log_len + events.len(),
            tape: step.tape.clone(),
            alternatives: step.alternatives.clone(),
        });
    }
    events.extend(engine.take_trace_events());
    (engine, rebuilt, events)
}

/// One engine step of a boundary as the replay took it.
#[derive(Clone)]
struct ReplayedStep {
    before: BattleSnapshot,
    /// The battle's log (`Compiled::log`) at `before`: a backtrack cuts it back there.
    log_len: usize,
    tape: Vec<RandomOverride>,
    alternatives: Vec<Vec<RandomOverride>>,
}

/// The scenario with the gender Showdown drew (as it built the teams) for every set that names
/// none, read off the details of the Pokémon's `switch` / `drag` lines (`Snorlax, L50, M`; no
/// gender for a genderless one), and of the member an Illusion's disguise copies off the details
/// it shows. A Pokémon that never shows keeps none: nothing can tell.
fn with_oracle_genders(scenario: &Scenario, oracle: &Value) -> Scenario {
    let mut scenario = scenario.clone();
    let events = oracle["frames"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|frame| frame["events"].as_array().into_iter().flatten());
    for event in events {
        if !matches!(event["kind"].as_str(), Some("switch" | "drag")) {
            continue;
        }
        // A `switch` into an Illusion shows the disguise's details; `illusion` holds the
        // entrant's own (DESIGN.md R-K.10c).
        let (Some(side), Some(name), Some(details)) = (
            event["pokemon"]["side"].as_u64(),
            event["pokemon"]["name"].as_str(),
            event["illusion"].as_str().or(event["details"].as_str()),
        ) else {
            continue;
        };
        let gender = details
            .split(", ")
            .find(|part| matches!(*part, "M" | "F"))
            .unwrap_or("N");
        let sets = if side == 0 {
            &mut scenario.teams.p1
        } else {
            &mut scenario.teams.p2
        };
        if let Some(set) = sets
            .iter_mut()
            .find(|set| set.label.as_deref().unwrap_or(&set.species) == name)
            && set.gender.is_none()
        {
            set.gender = Some(gender.to_string());
        }
        // The disguise's details show the gender Showdown drew for the member it copies (the
        // one of its species), which may never come in itself.
        if let (Some(_), Some(details)) = (event["illusion"].as_str(), event["details"].as_str()) {
            let species = details.split(", ").next().unwrap_or("");
            let gender = details
                .split(", ")
                .find(|part| matches!(*part, "M" | "F"))
                .unwrap_or("N");
            if let Some(set) = sets
                .iter_mut()
                .find(|set| set.species == species && set.gender.is_none())
            {
                set.gender = Some(gender.to_string());
            }
        }
    }
    scenario
}

/// Showdown gives a set that names no gender its species' fixed gender (`species.gender`), as
/// `build_pokemon` does: the gender the log shows for such a set must be that one.
fn fixed_gender_mismatches(declared: &Scenario, read: &Scenario, dex: &ResolvedDex) -> Vec<String> {
    let mut out = Vec::new();
    for (declared, read) in [
        (&declared.teams.p1, &read.teams.p1),
        (&declared.teams.p2, &read.teams.p2),
    ] {
        for (declared, read) in declared.iter().zip(read) {
            if declared.gender.is_none()
                && let Some(fixed) = dex.species_by_name(&read.species).and_then(|s| s.gender)
                && let Some(logged) = read.to_set().gender
                && logged != fixed
            {
                out.push(format!(
                    "gender: the log shows {} as {logged:?}, but the species is always {fixed:?}",
                    read.species
                ));
            }
        }
    }
    out
}

/// Replays `scenario` against Showdown's recording, boundary by boundary. A replay that
/// diverges after open choices (`OpenChoices`) runs again with each other alternative of one
/// choice taken, the latest choice first (R-M8-102); one that gets further keeps it, and the
/// choices met after it are tried on top — a choice may only show its fit once an earlier one
/// is taken right (R-M8-164). The first run that diverges nowhere is the report, else the
/// first run's.
pub fn replay_against_oracle(scenario: &Scenario, oracle: &Value) -> ReplayReport {
    let report = replay_pinned(scenario, oracle, BTreeMap::new());
    if report.divergences.is_empty() {
        return report;
    }
    let progress = |report: &ReplayReport| (report.frames_compared, report.actions_replayed);
    let mut pins: BTreeMap<usize, usize> = BTreeMap::new();
    let mut furthest = progress(&report);
    let mut met = report.open_choices.met.clone();
    'refine: loop {
        for (choice, &count) in met.iter().enumerate().rev() {
            for pick in 0..count {
                if pins.get(&choice) == Some(&pick) {
                    continue;
                }
                // The choices after this one may come in another order once it changes.
                let mut tried: BTreeMap<usize, usize> =
                    pins.range(..choice).map(|(&c, &p)| (c, p)).collect();
                tried.insert(choice, pick);
                let again = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    replay_pinned(scenario, oracle, tried.clone())
                }));
                let Ok(again) = again else {
                    continue;
                };
                if again.divergences.is_empty() {
                    return again;
                }
                if progress(&again) > furthest {
                    furthest = progress(&again);
                    pins = tried;
                    met = again.open_choices.met.clone();
                    continue 'refine;
                }
            }
        }
        break;
    }
    report
}

/// `replay_against_oracle` with the open choices in `pins` taken.
fn replay_pinned(
    scenario: &Scenario,
    oracle: &Value,
    pins: BTreeMap<usize, usize>,
) -> ReplayReport {
    assert_eq!(
        scenario.schema, 1,
        "{}: unsupported scenario schema",
        scenario.id
    );
    let mut report = ReplayReport::default();
    report.open_choices.pins = pins;
    let declared = scenario;
    let scenario = &with_oracle_genders(scenario, oracle);
    let mut compiled = compile(scenario);
    report
        .divergences
        .extend(fixed_gender_mismatches(declared, scenario, &compiled.dex));
    report.provenance = serde_json::json!({
        "scenario": scenario.id,
        "generation": scenario.battle.generation,
        "rules_fingerprint": compiled.rules.fingerprint().0,
        "dex_fingerprint": compiled.dex.fingerprint(),
    });
    let frames = oracle["frames"].as_array().expect("frames");
    let frame_named = |name: &str| frames.iter().find(|f| f["checkpoint"] == name);
    let reading = LogReading::for_rules(&compiled.rules, &compiled.dex);
    let all_decisions: Vec<Vec<ActionOracle>> = (0..scenario.script.len())
        .map(|i| {
            frame_named(&format!("boundary_{}", i + 1))
                .and_then(|f| f["events"].as_array())
                .map(|events| extract_decisions(events, &reading))
                .unwrap_or_default()
        })
        .collect();
    let all_entries: Vec<Vec<(usize, Who)>> = (0..scenario.script.len())
        .map(|i| {
            frame_named(&format!("boundary_{}", i + 1))
                .and_then(|f| f["events"].as_array())
                .map(|events| extract_entries(events, &reading))
                .unwrap_or_default()
        })
        .collect();

    // What each boundary's residual did to whom (Yawn's sleep, Healer's and Shed Skin's cures):
    // a sleep that ends there has no wake-up of its own to read its time from.
    let all_residuals: Vec<Vec<(Who, ResidualKind)>> = (0..scenario.script.len())
        .map(|i| {
            frame_named(&format!("boundary_{}", i + 1))
                .and_then(|f| f["events"].as_array())
                .map(|events| extract_residual_cures(events))
                .unwrap_or_default()
        })
        .collect();

    // Every boundary's frame, for the decisions only a later turn tells (a lock's length).
    let boundary_frames: Vec<Option<&Value>> = (0..scenario.script.len())
        .map(|i| frame_named(&format!("boundary_{}", i + 1)))
        .collect();

    // A real format's Team Preview, before any battle (M9.4).
    if let (Some(state), Some(frame)) = (&compiled.preview, frame_named("team_preview")) {
        let rust = super::capture_state(
            state,
            &compiled.rules,
            &compiled.dex,
            "team_preview",
            Some("team_preview".into()),
        );
        compare_frame(&mut report, "team_preview", &rust, frame);
    }

    // Showdown runs the leads' switch-in (Intimidate, a weather …) before the first request;
    // the initial frame is captured after it. Tied leads go in the order it reports them.
    let mut lead_tape = lead_tie_tape(&compiled, frame_named("initial"));
    if let Some(initial) = frame_named("initial") {
        let leads = frisk_order(&compiled, compiled.engine.state(), initial);
        lead_tape.extend(lead_ability_tape(
            &compiled,
            compiled.engine.state(),
            &leads,
            initial,
        ));
    }
    flush_trace(&mut compiled);
    let snapshot = compiled.engine.snapshot();
    compiled.engine = restore_with_tape(&compiled, &snapshot, lead_tape);
    compiled.engine.run_lead_switch_ins();

    let step = compiled.engine.step().expect("initial step");
    if let Some(initial) = frame_named("initial") {
        let rust = capture(&compiled.engine, "initial", Some(&step));
        compare_frame(&mut report, "initial", &rust, initial);
        let frame = report.rust_frames.last().cloned().unwrap_or_default();
        super::views::compare_views(&mut report, "initial", &compiled, &frame, initial);
    }

    // What each actor intends to use this turn, so the tape can be planned even when the
    // action ends up blocked before the move is announced. Kept across a mid-turn suspension:
    // the actions still pending after a replacement were declared by the earlier entry.
    let mut intended: BTreeMap<PokemonId, (MoveId, Option<DeclaredTarget>)> = BTreeMap::new();
    // The gimmick each choice asked for: a Z-Move runs in place of the slot's move, so the plan
    // must be built on the move the engine will actually use (DESIGN.md §44.4).
    let mut intended_mechanics: BTreeMap<PokemonId, MechanicChoice> = BTreeMap::new();
    let mut last_step = step;
    for (entry_index, entry) in scenario.script.iter().enumerate() {
        let checkpoint = format!("boundary_{}", entry_index + 1);
        let mut turn_retry: Option<TurnRetry> = None;
        flush_trace(&mut compiled);
        let boundary_trace = compiled.log.len();
        let Some(oracle_frame) = frame_named(&checkpoint) else {
            report
                .divergences
                .push(format!("{checkpoint}: missing from oracle output"));
            break;
        };
        let events = oracle_frame["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // Whether the user's item or ability adds a flinch to this move (King's Rock, Razor
        // Fang; Gen 5+ Stench, R-M8-102), as the battle stands: the move as the engine edits it
        // carries the flinch — Gen 2-4 the items only on their list (R-M9-34).
        let adds_flinch = |who: &Who, move_name: Option<&str>| {
            let state = compiled.engine.state();
            let (Some(id), Some(def)) = (
                who_to_id(state, who),
                move_name.and_then(|name| compiled.dex.move_by_name(name)),
            ) else {
                return false;
            };
            pokemon_battle_engine::engine::modified_move(state, &compiled.rules, id, def, false)
                .0
                .secondaries
                .iter()
                .any(|secondary| {
                    secondary.effect
                        == pokemon_battle_engine::dex::SecondaryEffect::Volatile(
                            VolatileConditionKind::Flinch,
                        )
                })
        };
        // Gen 1 stats at their bounds as the turn begins (Showdown `modifiedStats`): 999 to raise,
        // 1 to drop (R-M9-42).
        let stat_stuck = |who: &Who, stat: &str, raised: bool| {
            use pokemon_battle_engine::state::Stat;
            let state = compiled.engine.state();
            let stat = match stat {
                "atk" => Stat::Attack,
                "def" => Stat::Defense,
                "spa" => Stat::SpecialAttack,
                "spd" => Stat::SpecialDefense,
                "spe" => Stat::Speed,
                _ => return false,
            };
            who_to_id(state, who)
                .and_then(|id| state.pokemon(id))
                .and_then(|pokemon| pokemon.modified_stats)
                .and_then(|stats| stats.get(stat))
                .is_some_and(|value| if raised { value == 999 } else { value == 1 })
        };
        let mut decisions = extract_decisions_with(&events, &reading, &adds_flinch, &stat_stuck);
        attribute_frame_flinches(&mut decisions, oracle_frame, &reading, &adds_flinch);
        attribute_frame_rage(&mut decisions, oracle_frame);

        // A script entry answers whatever the engine is waiting for: the turn's choices, or
        // the replacements of a turn suspended mid-way (U-turn / Volt Switch) or ended by a
        // faint. Both engines read the same entry, so the boundaries stay aligned.
        if let EngineStep::Awaiting(BattleRequest::Revive { side, .. }) = &last_step {
            // Revival Blessing: the asked side names a fainted Pokémon with Showdown's `switch N`
            // (`pass` for its other slots); the party index names it, no swap follows.
            let side = *side;
            let text = if side == SideId::P1 {
                entry.p1.as_deref()
            } else {
                entry.p2.as_deref()
            };
            let Some(named) = revive_answer(&compiled, side, text) else {
                report.divergences.push(format!(
                    "{checkpoint}: rust asks {side:?} to name a Pokémon to revive but the script answers {text:?}"
                ));
                break;
            };
            if let Err(error) = answer_revive(&mut compiled, side, &named, oracle_frame) {
                report.divergences.push(format!(
                    "{checkpoint}: rust refused the script's revive: {error:?}"
                ));
                break;
            }
        } else if let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) =
            &last_step
        {
            // The engine wants replacements; the script must answer with switches. Anything
            // else means the request boundaries drifted apart: report where and stop.
            let answers_switch = |text: Option<&str>| {
                text.is_none_or(|t| {
                    t.split(',').map(str::trim).all(|part| {
                        part.is_empty() || part == "pass" || part.starts_with("switch ")
                    })
                })
            };
            if !answers_switch(entry.p1.as_deref()) || !answers_switch(entry.p2.as_deref()) {
                report.divergences.push(format!(
                    "{checkpoint}: rust asks for a forced switch ({:?}) but the script answers a turn ({:?} / {:?})",
                    replacements
                        .iter()
                        .map(|r| (r.side, r.required_count))
                        .collect::<Vec<_>>(),
                    entry.p1,
                    entry.p2
                ));
                break;
            }
            let choices = [
                parse_replacement(&mut compiled, replacements, SideId::P1, entry.p1.as_deref()),
                parse_replacement(&mut compiled, replacements, SideId::P2, entry.p2.as_deref()),
            ];
            // The entrants' confusion berries (hazards on the way in) draw their durations; a
            // Gen 4-5 Frisk entrant its foe first, Trace its foe (probed: which entrants came in).
            // Entrants sharing a speed draw their order first (R-M8-140).
            let berries = entry_berry_durations(&outside_berry_confusion_hints(
                &compiled,
                oracle_frame,
                &decisions,
            ));
            let mut entry_tape = Vec::new();
            let ability_draws = compiled.rules.conditions().frisk_one_random_foe
                || traces(oracle_frame)
                || forewarns(oracle_frame);
            let together = choices
                .iter()
                .map(|choice| choice.replacements.len())
                .sum::<usize>()
                > 1;
            if ability_draws || together {
                flush_trace(&mut compiled);
                let snapshot = compiled.engine.snapshot();
                let trace_len = compiled.engine.trace().events().len();
                let mut probe = restore_with_tape(&compiled, &snapshot, berries.clone());
                if probe.resolve_forced_switches(choices.clone()).is_ok() {
                    let events = &probe.trace().events()[trace_len..];
                    entry_tape = entry_tie_tape(&compiled, events, oracle_frame);
                    if ability_draws {
                        entry_tape.extend(entrant_ability_tape(
                            &compiled,
                            compiled.engine.state(),
                            probe.state(),
                            &entrants(events),
                            oracle_frame,
                        ));
                    }
                }
            }
            entry_tape.extend(berries);
            if !entry_tape.is_empty() {
                flush_trace(&mut compiled);
                let snapshot = compiled.engine.snapshot();
                compiled.engine = restore_with_tape(&compiled, &snapshot, entry_tape);
            }
            let resolved = compiled.engine.resolve_forced_switches(choices);
            // Showdown asks a mid-turn switch and a Revival Blessing of one side in one request
            // (one part per slot) and runs the switch first; the engine asks the revive next, and
            // this entry's part for the reviving user's slot answers it (R-M8-171).
            let resolved = resolved.and_then(|result| {
                if compiled.engine.state().phase
                    != pokemon_battle_engine::state::BattlePhase::Revive
                {
                    return Ok(result);
                }
                for side in [SideId::P1, SideId::P2] {
                    let Some(user) = compiled.engine.state().side(side).reviving else {
                        continue;
                    };
                    let text = if side == SideId::P1 {
                        entry.p1.as_deref()
                    } else {
                        entry.p2.as_deref()
                    };
                    let slot = reviving_slot(&compiled, user);
                    let named = text.and_then(|t| {
                        t.split(',')
                            .map(str::trim)
                            .nth(slot?)
                            .and_then(|part| part.strip_prefix("switch "))
                            .map(str::to_string)
                    });
                    if let Some(named) = named {
                        answer_revive(&mut compiled, side, &named, oracle_frame)?;
                    }
                }
                Ok(result)
            });
            if let Err(error) = resolved {
                report.divergences.push(format!(
                    "{checkpoint}: rust refused the script's replacements: {error:?} (rust asked {:?})",
                    replacements
                        .iter()
                        .map(|r| (r.side, r.required_count))
                        .collect::<Vec<_>>()
                ));
                let rust = capture(&compiled.engine, &checkpoint, Some(&last_step));
                report
                    .rust_frames
                    .push(serde_json::to_value(rust).expect("frame"));
                break;
            }
        } else {
            let choices = [
                parse_choice(&mut compiled, SideId::P1, entry.p1.as_deref()),
                parse_choice(&mut compiled, SideId::P2, entry.p2.as_deref()),
            ];
            intended.clear();
            intended_mechanics.clear();
            for choice in &choices {
                for slot in &choice.actions {
                    if let ActionChoice::Move {
                        slot: move_slot,
                        target,
                        mechanic,
                    } = slot.action
                        && let Some(pokemon) = compiled.engine.state().pokemon(slot.actor)
                        // A hard lock onto a move the slots do not hold (a charged move a
                        // Metronome called): the named slot means the lock, as the engine's
                        // choice validation reads it.
                        && let Some(id) = pokemon
                            .hard_lock()
                            .map(|lock| lock.move_id)
                            .filter(|locked| {
                                pokemon.move_slots().iter().all(|m| m.move_id != *locked)
                            })
                            .or_else(|| pokemon.move_slot(move_slot).map(|m| m.move_id))
                    {
                        intended.insert(slot.actor, (id, target));
                        if let Some(mechanic) = mechanic {
                            intended_mechanics.insert(slot.actor, mechanic);
                        }
                    }
                }
            }

            // Speed ties: the engine says which actions will draw a SpeedTie decision; the
            // oracle's action order says who won each tie. Earlier movers get the larger draw.
            // The turn's actions run on past a mid-turn replacement (Gen 1-3 after a faint,
            // U-turn): the later boundaries of the same turn carry them, named by label since
            // the replacement changed who stands in a slot.
            flush_trace(&mut compiled);
            let snapshot = compiled.engine.snapshot();
            let frame_turn = |boundary: usize| {
                let name = if boundary == 0 {
                    "initial".to_owned()
                } else {
                    format!("boundary_{boundary}")
                };
                frame_named(&name).and_then(|f| f["turn"].as_u64())
            };
            let turn = frame_turn(entry_index);
            let turn_decisions: Vec<&ActionOracle> = decisions
                .iter()
                .chain(
                    (entry_index + 1..all_decisions.len())
                        .take_while(|&later| frame_turn(later) == turn)
                        .flat_map(|later| all_decisions[later].iter()),
                )
                .collect();
            // Where each declared action shows in Showdown's log: a move by its user's first
            // action, a switch by its entrant's `switch` line (tied switches go in that order).
            let entrants: Vec<PokemonId> = all_entries
                .get(entry_index)
                .into_iter()
                .flatten()
                .filter_map(|(_, who)| label_to_id(&compiled, who))
                .collect();
            let switching_in = |actor: PokemonId| {
                choices
                    .iter()
                    .flat_map(|c| &c.actions)
                    .find_map(|slot| match slot.action {
                        ActionChoice::Switch { pokemon } if slot.actor == actor => Some(pokemon),
                        _ => None,
                    })
            };
            let first_move = |actor: PokemonId| {
                if let Some(entrant) = switching_in(actor) {
                    return entrants
                        .iter()
                        .position(|id| *id == entrant)
                        .unwrap_or(usize::MAX);
                }
                let is_actor = |who: &Who| {
                    label_to_id(&compiled, who).or_else(|| who_to_id(&snapshot.state, who))
                        == Some(actor)
                };
                // A move by its user's first action — a dance is another's (R-M8-78); a Pokémon
                // knocked out before it could act comes right after the action that knocked it out.
                turn_decisions
                    .iter()
                    .position(|d| !d.danced && !d.instructed && is_actor(&d.user))
                    .map(|index| 2 * index + 2)
                    .or_else(|| {
                        turn_decisions
                            .iter()
                            .position(|d| {
                                d.damages.iter().any(|(who, hp)| *hp == 0 && is_actor(who))
                            })
                            .map(|index| 2 * index + 3)
                    })
                    .or_else(|| {
                        // Sky Drop's carried Pokémon shows nothing: its action came to nothing
                        // before its carrier let it go — after, it would have acted (DESIGN.md
                        // R281).
                        let carrier = snapshot.state.pokemon(actor)?.volatiles.sky_dropped_by()?;
                        turn_decisions
                            .iter()
                            .position(|d| {
                                label_to_id(&compiled, &d.user)
                                    .or_else(|| who_to_id(&snapshot.state, &d.user))
                                    == Some(carrier)
                            })
                            .map(|index| 2 * index + 1)
                    })
                    .unwrap_or(usize::MAX)
            };
            // Priority chances (DESIGN.md §12.4): a Quick Claw or Quick Draw that went off
            // announces itself as the turn starts; Gen 2-3's turn roll is silent, so it is read
            // from the order — it went off when the order without it contradicts the oracle's.
            // One Instruct's `resolveAction` rolled for its target (right after the
            // `-singleturn`) is that action's, not the turn's (R-M9-32).
            let instructed_at = |index: usize| {
                events[..index]
                    .iter()
                    .rev()
                    .find(|event| {
                        event["kind"] != "activate"
                            || event["args"]
                                .get(1)
                                .and_then(Value::as_str)
                                .and_then(announced_chance)
                                .is_none()
                    })
                    .is_some_and(|event| {
                        event["kind"] == "singleturn" && event["move"] == "move: Instruct"
                    })
            };
            let announced: Vec<(PokemonId, PriorityChance)> = events
                .iter()
                .enumerate()
                .filter(|(index, event)| event["kind"] == "activate" && !instructed_at(*index))
                .filter_map(|(_, event)| {
                    let args = event["args"].as_array()?;
                    let chance = announced_chance(args.get(1).and_then(Value::as_str)?)?;
                    let who = args.first().and_then(who_from_text)?;
                    let id = label_to_id(&compiled, &who)
                        .or_else(|| who_to_id(&snapshot.state, &who))?;
                    Some((id, chance))
                })
                .collect();
            let preview = |turn_roll: bool| {
                compiled
                    .engine
                    .preview_action_order(&choices, &|actor, chance| match chance {
                        PriorityChance::QuickClawTurn => turn_roll,
                        _ => announced.contains(&(actor, chance)),
                    })
                    .unwrap_or_else(|error| panic!("{}: {checkpoint}: {error:?}", scenario.id))
            };
            // Whether Showdown's order fits the brackets: every actor of an earlier bracket could
            // have come before every actor of a later one. An actor that acted sits at its
            // action; one knocked out before acting only after the action that knocked it out.
            let consistent = |order: &[Vec<PokemonId>]| {
                let spans: Vec<Vec<(usize, usize)>> = order
                    .iter()
                    .map(|bracket| {
                        bracket
                            .iter()
                            .filter(|actor| switching_in(**actor).is_none())
                            .map(|actor| first_move(*actor))
                            .filter(|position| *position != usize::MAX)
                            .map(|position| {
                                if position % 2 == 0 {
                                    (position, position)
                                } else {
                                    (position, usize::MAX)
                                }
                            })
                            .collect()
                    })
                    .collect();
                spans.iter().enumerate().all(|(index, earlier)| {
                    spans[index + 1..]
                        .iter()
                        .flatten()
                        .all(|later| earlier.iter().all(|(earliest, _)| *earliest < later.1))
                })
            };
            // The turn roll also sets the holders' Speed for the residual: when the action order
            // cannot tell, the other outcome is kept to retry the boundary with.
            let mut plan = preview(false);
            let mut undecided = None;
            if plan
                .chances
                .iter()
                .any(|(_, chance, _)| *chance == PriorityChance::QuickClawTurn)
            {
                let rolled = preview(true);
                match (consistent(&plan.order), consistent(&rolled.order)) {
                    (false, true) => plan = rolled,
                    (true, true) => undecided = Some(rolled),
                    _ => {}
                }
            }
            let tape_for = |plan: &pokemon_battle_engine::engine::ActionOrderPreview| {
                let ties = plan.ties.iter().map(|(actor, group)| {
                    let mut members: Vec<PokemonId> = plan
                        .ties
                        .iter()
                        .filter(|(_, g)| g == group)
                        .map(|(a, _)| *a)
                        .collect();
                    members.sort_by_key(|a| first_move(*a));
                    let rank = members.iter().position(|a| a == actor).unwrap_or(0);
                    RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
                });
                plan.chances
                    .iter()
                    .map(|(_, _, rolled)| {
                        RandomOverride::bool(RandomRequestKind::PriorityChance, *rolled)
                    })
                    .chain(ties)
                    .collect::<Vec<RandomOverride>>()
            };
            let tie_tape = tape_for(&plan);
            if std::env::var_os("REPLAY_DEBUG").is_some() {
                eprintln!(
                    "  {checkpoint}: priority chances {:?}, speed ties {:?} → {tie_tape:?}",
                    plan.chances, plan.ties
                );
            }
            if let Some(other) = &undecided {
                turn_retry = Some((
                    snapshot.clone(),
                    choices.clone(),
                    tie_tape.clone(),
                    tape_for(other),
                ));
            }
            compiled.engine = restore_with_tape(&compiled, &snapshot, tie_tape);
            if let Err(error) = compiled.engine.submit_turn_choices(choices) {
                report.divergences.push(format!(
                    "{checkpoint}: rust refused the script's turn choices: {error:?}"
                ));
                break;
            }
        }

        let marks = (
            compiled.party.clone(),
            compiled.pending_swaps.clone(),
            report.divergences.len(),
            report.actions_replayed,
            report.frames_compared,
            report.rust_frames.len(),
            // The log as the turn's choices went in: a retry from the turn's snapshot drops what
            // the failed drive traced (R-M10-6).
            compiled.log.len(),
        );
        let drive = |compiled: &mut Compiled, report: &mut ReplayReport| {
            drive_and_compare(
                compiled,
                report,
                &checkpoint,
                oracle_frame,
                &decisions,
                &all_decisions,
                &all_entries,
                &all_residuals,
                &boundary_frames,
                entry_index,
                &intended,
                &intended_mechanics,
                boundary_trace,
            )
        };
        let mut step = drive(&mut compiled, &mut report);
        // The other Quick Claw outcome, then (both failing) the first again, for its report.
        if report.divergences.len() > marks.2
            && let Some((snapshot, choices, first, other)) = turn_retry
        {
            for tape in [other, first] {
                compiled.party = marks.0.clone();
                compiled.pending_swaps = marks.1.clone();
                report.divergences.truncate(marks.2);
                report.actions_replayed = marks.3;
                report.frames_compared = marks.4;
                report.rust_frames.truncate(marks.5);
                compiled.log.truncate(marks.6);
                compiled.engine = restore_with_tape(&compiled, &snapshot, tape);
                compiled
                    .engine
                    .submit_turn_choices(choices.clone())
                    .expect("the same choices were accepted");
                step = drive(&mut compiled, &mut report);
                if report.divergences.len() == marks.2 {
                    break;
                }
            }
        }
        let finished = matches!(step, EngineStep::Finished(_));
        last_step = step;
        // A diverged battle stops at that boundary: the later ones replay a different battle,
        // and a panic there would hide the divergence that explains it.
        if finished || report.divergences.len() > marks.2 {
            break;
        }
    }
    flush_trace(&mut compiled);
    report.trace = compiled
        .log
        .iter()
        .map(|event| format!("{event:?}"))
        .collect();
    report
}

/// Drives a boundary from the answered request and compares its frame (DESIGN.md §31.5): a
/// Taunt / Encore / Disable duration is read off the frame as if the end of the turn ticked it;
/// a battle decided within the residual may stop before that tick — the engine says whether it
/// got there, and the boundary is driven again with those durations read as unticked.
#[allow(clippy::too_many_arguments)]
fn drive_and_compare(
    compiled: &mut Compiled,
    report: &mut ReplayReport,
    checkpoint: &str,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    all_decisions: &[Vec<ActionOracle>],
    all_entries: &[Vec<(usize, Who)>],
    all_residuals: &[Vec<(Who, ResidualKind)>],
    boundary_frames: &[Option<&Value>],
    entry_index: usize,
    intended: &BTreeMap<PokemonId, (MoveId, Option<DeclaredTarget>)>,
    intended_mechanics: &BTreeMap<PokemonId, MechanicChoice>,
    boundary_trace: usize,
) -> EngineStep {
    flush_trace(compiled);
    let driven_from = (
        compiled.engine.snapshot(),
        compiled.party.clone(),
        compiled.pending_swaps.clone(),
        report.divergences.len(),
        report.actions_replayed,
        compiled.log.len(),
    );
    let mut step = drive_to_boundary(
        compiled,
        report,
        checkpoint,
        oracle_frame,
        decisions,
        all_decisions,
        all_entries,
        all_residuals,
        boundary_frames,
        entry_index,
        intended,
        intended_mechanics,
        &[],
    );
    let untick = if matches!(step, EngineStep::Finished(_)) {
        flush_trace(compiled);
        let events = &compiled.log[boundary_trace..];
        let mut untick = timed_volatiles_left_unticked(events);
        untick.extend(bides_left_unticked(
            compiled.engine.state(),
            &compiled.rules,
            oracle_frame,
            events,
        ));
        untick
    } else {
        Vec::new()
    };
    if !untick.is_empty() {
        let (snapshot, party, pending_swaps, divergences, actions, log) = driven_from;
        compiled.log.truncate(log);
        compiled.engine =
            BattleEngine::restore(compiled.rules.clone(), compiled.dex.clone(), snapshot)
                .expect("restore");
        compiled.party = party;
        compiled.pending_swaps = pending_swaps;
        report.divergences.truncate(divergences);
        report.actions_replayed = actions;
        step = drive_to_boundary(
            compiled,
            report,
            checkpoint,
            oracle_frame,
            decisions,
            all_decisions,
            all_entries,
            all_residuals,
            boundary_frames,
            entry_index,
            intended,
            intended_mechanics,
            &untick,
        );
    }
    settle_party_swaps(compiled, boundary_trace);
    let rust = capture(&compiled.engine, checkpoint, Some(&step));
    compare_frame(report, checkpoint, &rust, oracle_frame);
    let frame = report.rust_frames.last().cloned().unwrap_or_default();
    super::views::compare_views(report, checkpoint, compiled, &frame, oracle_frame);
    step
}

thread_local! {
    /// The boundary and action being replayed, so a panic inside the engine can be reported
    /// where it happened (`tests/campaign.rs`).
    pub static REPLAY_CONTEXT: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
}

/// Drives the engine from a freshly answered request to the next one, action by action: each
/// step is probed to learn whose action it is, the oracle's matching decision fixes the
/// semantic random calls, and the damage roll is searched. Returns the step that stopped the
/// drive (a request, or the end of the battle).
#[allow(clippy::too_many_arguments)]
fn drive_to_boundary(
    compiled: &mut Compiled,
    report: &mut ReplayReport,
    checkpoint: &str,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    all_decisions: &[Vec<ActionOracle>],
    all_entries: &[Vec<(usize, Who)>],
    all_residuals: &[Vec<(Who, ResidualKind)>],
    boundary_frames: &[Option<&Value>],
    turn_index: usize,
    intended: &BTreeMap<PokemonId, (MoveId, Option<DeclaredTarget>)>,
    intended_mechanics: &BTreeMap<PokemonId, MechanicChoice>,
    untick: &[(PokemonId, MissedTick)],
) -> EngineStep {
    let residual_cures = extract_residual_cures(
        &oracle_frame["events"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    let residual_faints = extract_residual_faints(
        &oracle_frame["events"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    // The sleep a Yawn brought on in this residual: its duration, recovered from the turns
    // after this one like a move's.
    let lifecycle = compiled.rules.status().lifecycle;
    let yawn_sleeps: Vec<(Who, u16)> = residual_cures
        .iter()
        .filter(|(_, kind)| *kind == ResidualKind::Yawn)
        .filter_map(|(who, _)| {
            let after_last_action = all_decisions.get(turn_index).map_or(0, Vec::len);
            sleep_duration_hint(
                all_decisions,
                all_entries,
                all_residuals,
                turn_index,
                after_last_action,
                who,
                lifecycle.sleep_resets_on_switch_in,
                lifecycle.sleep_usable_turns_come_back,
                u16::from(lifecycle.sleep_time_min),
                u16::from(lifecycle.sleep_time_max),
                &sleep_steps(
                    compiled.engine.state(),
                    &compiled.rules,
                    who,
                    turn_index,
                    boundary_frames,
                ),
            )
            .map(|time| (who.clone(), time))
        })
        .collect();
    // The locks (Thrash and its kind) the residual ended with their fatigue: the confusion's
    // duration from the frame.
    let fatigues: Vec<(Who, u16)> = residual_cures
        .iter()
        .filter(|(_, kind)| *kind == ResidualKind::Rampage)
        .map(|(who, _)| {
            let member = frame_member_at_slot(oracle_frame, who);
            let time = confusion_hint_after(&compiled.rules, oracle_frame, &[], who, member)
                .unwrap_or(u16::from(lifecycle.confusion_time_min));
            (who.clone(), time)
        })
        .collect();
    // The confusion berries eaten outside the actions, with their durations: in the residual
    // (the frame shows the time as rolled) and in a switch-in (plus the entrant's later ticks).
    let berry_confusions = outside_berry_confusion_hints(compiled, oracle_frame, decisions);
    let mut decision_index = 0;
    // Every step of the boundary so far, for the Substitute backtracking below.
    let mut steps: Vec<ReplayedStep> = Vec::new();
    let step = loop {
        flush_trace(compiled);
        let before = compiled.engine.snapshot();
        let trace_len = compiled.engine.trace().events().len();

        // A Gen 8+ step re-sorts the queue first and reshuffles its speed ties: the engine says
        // which pending actions tie, the oracle's remaining order says who won (earlier movers
        // get the larger draw, as at the start of the turn).
        let resort_ties: Vec<RandomOverride> = {
            let tied = compiled.engine.pending_speed_ties();
            let next_move = |actor: PokemonId| {
                decisions[decision_index.min(decisions.len())..]
                    .iter()
                    .filter(|d| !d.danced && !d.instructed)
                    .position(|d| {
                        label_to_id(compiled, &d.user).or_else(|| who_to_id(&before.state, &d.user))
                            == Some(actor)
                    })
                    .unwrap_or(usize::MAX)
            };
            tied.iter()
                .map(|(actor, group)| {
                    let mut members: Vec<PokemonId> = tied
                        .iter()
                        .filter(|(_, g)| g == group)
                        .map(|(a, _)| *a)
                        .collect();
                    members.sort_by_key(|a| next_move(*a));
                    let rank = members.iter().position(|a| a == actor).unwrap_or(0);
                    RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
                })
                .collect()
        };

        // Probe with only those decisions (PRNG fallback after) to learn whose action this
        // step is.
        let mut probe = restore_with_tape(compiled, &before, resort_ties.clone());

        let probe_step = probe.step().expect("step");
        let acting = probe.trace().events()[trace_len..]
            .iter()
            .find_map(|event| match event {
                TraceEvent::BeforeMoveResolved { actor, .. } => Some(*actor),
                _ => None,
            });

        // Terastallization's own queue step (DESIGN.md §44.2): Showdown logs `-terastallize`
        // before the move it belongs to, and no `move` line of its own, so it consumes no
        // oracle action — the move that follows does. Mega Evolution's and Ultra Burst's steps
        // are the same (`-mega` / `-burst`), and in Gen 7 they re-queue the Pokémon's move.
        let gimmick = probe.trace().events()[trace_len..]
            .iter()
            .find_map(|event| match event {
                TraceEvent::Terastallized { .. } | TraceEvent::MegaEvolved { .. } => Some(()),
                _ => None,
            });
        if gimmick.is_some() {
            steps.push(ReplayedStep {
                before,
                log_len: compiled.log.len(),
                tape: resort_ties,
                alternatives: Vec::new(),
            });
            compiled.engine = probe;
            if !matches!(probe_step, EngineStep::Progressed { .. }) {
                break probe_step;
            }
            continue;
        }

        let Some(actor) = acting else {
            // Gen 1's Pokémon held by a trapping move that goes on (`cannotmove`): its action is
            // its own after-action residual (poison, burn, Leech Seed), which the log shows as a
            // silent action of its own — first in the turn when it won a tie (R-M8-108).
            if let Some(next) = decisions.get(decision_index)
                && next.move_name.is_none()
                && next.cant.is_none()
                && let Some(silent) = who_to_id(&before.state, &next.user)
                && probe.trace().events()[trace_len..].iter().any(|event| {
                    matches!(event, TraceEvent::DamageApplied { target, .. } if *target == silent)
                })
            {
                decision_index += 1;
            }
            // Not an action: the end of the turn (or a switch). Its speed ties come first
            // (Showdown shuffles tied residual handlers, then the weather's tied targets: the
            // closing events show who went first), then its decisions (Shed Skin, per statused
            // holder in schedule order).
            let ends_turn = probe.trace().events()[trace_len..].iter().any(|event| {
                matches!(
                    event,
                    TraceEvent::PhaseChanged {
                        to: pokemon_battle_engine::state::BattlePhase::EndTurn,
                        ..
                    }
                )
            });
            // The foreseen attacks the residual strikes with (§15.22): their damage rolls,
            // searched below against the log's HP.
            let strikes = if ends_turn {
                extract_future_strikes(
                    oracle_frame["events"]
                        .as_array()
                        .map_or(&[][..], Vec::as_slice),
                )
            } else {
                Vec::new()
            };
            // The entrants this step brings in (a switch-only turn: the switches run inside the
            // same step as the end of turn).
            let entrants_now = entrants(&probe.trace().events()[trace_len..]);
            // The entrants' switch-in draws (Frisk's reveal, Trace's copy): they run before the
            // step's other handlers — the switches a turn ends with come before its residual
            // (DESIGN.md R-M8-48) — and are optional, so a step that brings nobody only passes
            // over them when its own ask is of another kind.
            let entrant_tape = entrant_ability_tape(
                compiled,
                &before.state,
                probe.state(),
                &entrants_now,
                oracle_frame,
            );
            // The residual's tie decisions: the plan first, then the other orders of the ties the
            // log does not show.
            let tie_tapes = if ends_turn {
                residual_tie_tapes(compiled, &before.state, oracle_frame)
            } else {
                vec![Vec::new()]
            };
            let build_residual = |rolls: &[Option<u16>], ties: &[RandomOverride]| {
                let mut residual = resort_ties.clone();
                residual.extend(entrant_tape.iter().copied());
                // Neutralizing Gas clearing (a switch: at once; the end of the turn: as its holder
                // faints, after the residual's own ties).
                let gas = neutralizing_gas_tape(compiled, &before, oracle_frame);
                if !ends_turn {
                    residual.extend(gas.iter().copied());
                }
                residual.extend(ties.iter().copied());
                let moody = moody_tape(&before.state, &compiled.rules, &compiled.dex, oracle_frame);
                residual.extend(residual_tape(
                    &before.state,
                    &compiled.rules,
                    &compiled.dex,
                    &residual_cures,
                    &residual_faints,
                    &yawn_sleeps,
                    &berry_confusions,
                    &fatigues,
                    &strikes,
                    rolls,
                    &moody,
                ));
                residual.extend(residual_end_berry_tape(
                    &before.state,
                    &compiled.rules,
                    &berry_confusions,
                ));
                if ends_turn {
                    residual.extend(residual_drag_tape(compiled, &before.state, oracle_frame));
                    residual.extend(gas);
                }
                // A switch action: the confusion berry (optional: the step may be another's).
                if !ends_turn {
                    residual.extend(entry_berry_durations(&berry_confusions));
                }
                residual
            };
            let mut rolls: Vec<Option<u16>> = vec![None; strikes.len()];
            // Each strike's roll in turn, the one that leaves its target the HP the log shows —
            // or, a strike on a Substitute, the Substitute HP the frame shows (none: broken),
            // the turn's last word on it (R-M8-84).
            for index in 0..strikes.len() {
                let strike = &strikes[index];
                let Some(target) = who_to_id(&before.state, &strike.target) else {
                    continue;
                };
                let on_substitute = strike.hp_after.is_none() && !strike.missed && !strike.immune;
                if on_substitute {
                    let expected = frame_substitute_hp(oracle_frame, &strike.target);
                    let damage = compiled.rules.damage();
                    for roll in (damage.random_min..=damage.random_max).rev() {
                        rolls[index] = Some(roll);
                        let mut candidate = restore_with_tape(
                            compiled,
                            &before,
                            build_residual(&rolls, &tie_tapes[0]),
                        );
                        if candidate.step().is_ok()
                            && substitute_hp(candidate.state(), target) == expected
                        {
                            break;
                        }
                    }
                    continue;
                }
                let Some(expected) = strike.hp_after else {
                    continue;
                };
                let damage = compiled.rules.damage();
                for roll in (damage.random_min..=damage.random_max).rev() {
                    rolls[index] = Some(roll);
                    let mut candidate =
                        restore_with_tape(compiled, &before, build_residual(&rolls, &tie_tapes[0]));
                    if candidate.step().is_err() {
                        continue;
                    }
                    let struck = candidate.trace().events()[trace_len..]
                        .iter()
                        .find_map(|event| match event {
                            TraceEvent::DamageApplied {
                                target: struck,
                                hp_after,
                                source:
                                    pokemon_battle_engine::engine::EffectSource::Move {
                                        move_id, ..
                                    },
                                ..
                            } if *struck == target
                                && matches!(
                                    *move_id,
                                    MoveId::FUTURE_SIGHT | MoveId::DOOM_DESIRE
                                ) =>
                            {
                                Some(*hp_after)
                            }
                            _ => None,
                        });
                    if struck == Some(expected) {
                        break;
                    }
                }
            }
            let mut residual = build_residual(&rolls, &tie_tapes[0]);
            // A battle the residual decides stops it where it is decided: whether a silent
            // handler tied with others counted down before shows in the last frame alone. Take
            // the tie order whose frame is the oracle's.
            if tie_tapes.len() > 1 {
                let fits = |tape: Vec<RandomOverride>| {
                    let mut candidate = restore_with_tape(compiled, &before, tape);
                    let Ok(step) = candidate.step() else {
                        return None;
                    };
                    candidate.state().outcome?;
                    let frame = capture(&candidate, checkpoint, Some(&step));
                    Some(frame_diffs(&frame, oracle_frame).1.is_empty())
                };
                if fits(residual.clone()) == Some(false)
                    && let Some(found) = tie_tapes[1..]
                        .iter()
                        .map(|ties| build_residual(&rolls, ties))
                        .find(|tape| fits(tape.clone()) == Some(true))
                {
                    residual = found;
                }
            }
            let (engine, step, tape) = if residual == resort_ties {
                (probe, probe_step, resort_ties)
            } else {
                let mut engine = restore_with_tape(compiled, &before, residual.clone());
                let step = engine.step().expect("step");
                (engine, step, residual)
            };

            steps.push(ReplayedStep {
                before,
                log_len: compiled.log.len(),
                tape,
                alternatives: Vec::new(),
            });
            compiled.engine = engine;
            follow_drags(compiled, trace_len);
            if !matches!(step, EngineStep::Progressed { .. }) {
                break step;
            }
            continue;
        };

        // Sky Drop's carried Pokémon (§15.23): its action comes to nothing without a word
        // (`onFoeBeforeMove` → `null`), so the log has no action for it.
        // Gen 1's Desync Clause Mod ends an action the same way, the log showing only its
        // `-hint` lines (R-M8-150).
        let sky_dropped = probe.trace().events()[trace_len..].iter().any(|event| {
            matches!(
                event,
                TraceEvent::MoveFailed {
                    actor: failed,
                    reason: pokemon_battle_engine::engine::MoveFailReason::BeforeMove(
                        pokemon_battle_engine::engine::BeforeMoveBlockReason::SkyDropped
                            | pokemon_battle_engine::engine::BeforeMoveBlockReason::Commanding
                    ) | pokemon_battle_engine::engine::MoveFailReason::Desync,
                    ..
                } if *failed == actor
            )
        });
        if sky_dropped {
            steps.push(ReplayedStep {
                before,
                log_len: compiled.log.len(),
                tape: resort_ties,
                alternatives: Vec::new(),
            });
            compiled.engine = probe;
            if !matches!(probe_step, EngineStep::Progressed { .. }) {
                break probe_step;
            }
            continue;
        }

        report.actions_replayed += 1;
        let Some(decision) = decisions.get(decision_index).cloned() else {
            report.divergences.push(format!(
                "{checkpoint}: rust executed an extra action ({actor:?}) the oracle did not report"
            ));
            compiled.engine = probe;
            continue;
        };
        // This action's place in the oracle's list; the dances that follow it are the engine's
        // inside it (Dancer's `runMove`, R-M8-78).
        let index = decision_index;
        decision_index += 1;
        let mut dances = Vec::new();
        while let Some(next) = decisions.get(decision_index).filter(|next| next.danced) {
            dances.push(next.clone());
            decision_index += 1;
        }
        REPLAY_CONTEXT.with(|context| {
            *context.borrow_mut() = format!(
                "{checkpoint}: {} by {}",
                decision.move_name.as_deref().unwrap_or("-"),
                decision.user.name
            )
        });
        if who_to_id(&before.state, &decision.user) != Some(actor) {
            report.divergences.push(format!(
                "{checkpoint}: action order: rust actor {actor:?} but oracle's next action is {:?} by {:?}",
                decision.move_name, decision.user
            ));
        }

        // An Encore that landed before the actor moved overrides its choice (Showdown
        // `OverrideAction`); the engine uses the Encored move, so the plan does too.
        let encored = before
            .state
            .pokemon(actor)
            .and_then(|p| p.volatiles.encored_move())
            .filter(|encored| {
                intended
                    .get(&actor)
                    .is_some_and(|(chosen, _)| chosen != encored)
            });
        // No move was left to choose: the engine's action is Struggle (`BattleState::struggling`,
        // decided at the choice).
        let struggling = before.state.struggling.contains(&actor);
        let encored = encored.filter(|_| !struggling);
        let move_def = (if struggling {
            Some(MoveId::STRUGGLE)
        } else {
            encored.or_else(|| intended.get(&actor).map(|(id, _)| *id))
        })
        // Gen 1: a user locked into its binding move uses it, whatever it chose (§30.9).
        .map(|id| {
            pokemon_battle_engine::engine::binding_locked_move(&before.state, actor).unwrap_or(id)
        })
        .and_then(|id| compiled.dex.move_by_id(id));
        // An action the engine runs with another move than the one chosen — Instruct's injected
        // action repeats the target's last move (DESIGN.md §42.9): the move the probe used at the
        // top level, when the log's action names it too (R-M8-57).
        let engine_used = probe.trace().events()[trace_len..]
            .iter()
            .find_map(|event| match event {
                TraceEvent::MoveUsed {
                    actor: user,
                    move_id,
                    declared_target,
                    called_by: None,
                    ..
                } if *user == actor => Some((*move_id, *declared_target)),
                _ => None,
            })
            .and_then(|(id, target)| compiled.dex.move_by_id(id).map(|def| (def, target)))
            .filter(|(used, _)| {
                move_def.is_none_or(|chosen| chosen.id != used.id)
                    && decision.move_name.as_deref() == Some(used.name.as_str())
            });
        let move_def = engine_used.map(|(used, _)| used).or(move_def);
        // Where the action aims: the choice's target, or the injected action's own (Instruct's
        // `lastMoveTargetLoc`).
        let chosen_target = match engine_used {
            Some((_, target)) => target,
            None => intended.get(&actor).and_then(|(_, target)| *target),
        };
        // The move the action chose (the caller, when it called another).
        let chosen_move = move_def;
        // The move the action called (Showdown's `[from] move: …` line, DESIGN.md §42.1): the
        // engine runs it instead, so the plan, the fates and the HP search are its.
        let move_def = decision
            .called
            .last()
            .and_then(|called| compiled.dex.moves().iter().find(|m| m.name == called.name))
            .or(move_def);
        let dex_move = move_def;
        // As the user will use it (Weather Ball's type, an -ate ability): the engine's own
        // `modified_move`, on the state before the action (Shell Side Arm's category comes once
        // its target is known, below).
        // A Z-Move replaces the slot's move (Showdown `getActiveZMove` before `ModifyMove`):
        // the engine's own resolution, so the plan sees the move it will run.
        let z_move = intended_mechanics
            .get(&actor)
            .filter(|mechanic| **mechanic == MechanicChoice::ZMove)
            .and_then(|_| {
                pokemon_battle_engine::engine::z_move_execution(
                    &before.state,
                    &compiled.rules,
                    &compiled.dex,
                    actor,
                    dex_move?,
                )
                .map(|z| z.move_def)
            });
        // A Dynamaxed Pokémon's move is its Max Move (DESIGN.md §44.5), the engine's own
        // resolution again.
        let max_move = {
            let modified = z_move.clone().or(dex_move.cloned()).map(|def| {
                pokemon_battle_engine::engine::modified_move(
                    &before.state,
                    &compiled.rules,
                    actor,
                    &def,
                    false,
                )
                .0
            });
            modified.and_then(|modified| {
                pokemon_battle_engine::engine::max_move_execution(
                    &before.state,
                    &compiled.dex,
                    actor,
                    &modified,
                )
                .map(|max| max.move_def)
            })
        };
        // Showdown `runMove` aims a Max or Z action by the move it runs (`getTarget(pokemon,
        // maxMove || zMove || move, targetLoc)`): Mirror Coat's Max Mindstorm keeps the chosen
        // foe, where the base move (`scripted`) would have one drawn (R-M9-45). An action that
        // called another aims by its own move, the caller.
        let aimed_move = if decision.called.is_empty() {
            max_move.clone().or_else(|| z_move.clone())
        } else {
            None
        };
        let move_def = max_move.or(z_move).or(dex_move.cloned()).map(|def| {
            pokemon_battle_engine::engine::modified_move(
                &before.state,
                &compiled.rules,
                actor,
                &def,
                false,
            )
            .0
        });
        // The Pokémon the move will strike, in the engine's target order, each with what
        // Showdown reported for it (a spread move misses or is immune per target).
        let (fates, reflections) = move_def
            .as_ref()
            .and_then(|def| {
                // An overriding Encore re-chooses the target (Showdown: `getRandomTarget`).
                let declared = chosen_target.filter(|_| encored.is_none() && !struggling);
                compiled
                    .rules
                    .targeting()
                    .resolve(
                        compiled.rules.topology(),
                        &before.state,
                        actor,
                        def.target,
                        declared,
                    )
                    .ok()
                    // The declared target is gone (fainted earlier this turn): the engine
                    // retargets, and Showdown's `move` event below names where it went.
                    .or_else(|| {
                        Some(pokemon_battle_engine::rules::ResolvedTargets {
                            targets: Vec::new(),
                        })
                    })
            })
            .map(|targets| {
                let mut ids: Vec<PokemonId> = targets
                    .targets
                    .iter()
                    .filter_map(|t| match t {
                        pokemon_battle_engine::rules::ResolvedTarget::Pokemon(p) if *p != actor => {
                            Some(*p)
                        }
                        _ => None,
                    })
                    .collect();
                // A single-target move goes where Showdown's `move` event says (Follow Me,
                // Lightning Rod and a fainted target change it at execution time).
                let single = matches!(
                    move_def.as_ref().map(|def| def.target),
                    Some(
                        TargetPattern::SelectedAny
                            | TargetPattern::SelectedAdjacent
                            | TargetPattern::SelectedFoe
                            | TargetPattern::SelectedAdjacentFoe
                            | TargetPattern::RandomAdjacentFoe
                    )
                );
                // A reflection (Magic Coat, Magic Bounce: the last called move another Pokémon
                // used) goes back at the actor itself (R-M8-68).
                let reflected_back = decision
                    .called
                    .last()
                    .is_some_and(|called| called.user != decision.user);
                if single
                    && let Some(actual) = decision.move_target.as_ref().and_then(|who| {
                        label_to_id(compiled, who).or(who_to_id(&before.state, who))
                    })
                    && (actual != actor || reflected_back)
                {
                    ids = vec![actual];
                }
                // A move with several targets some of them reflected: each reflection — aimed
                // back from its reflector, at the actor's side — takes the reflector's place.
                // Where every target's TryHit comes before any accuracy (Gen 5+, R-M8-77) they
                // strike first, in turn, then the move its other targets; Gen 3-4 roll each
                // target's accuracy before its TryHit, target by target, so each strikes in its
                // reflector's turn (R-M8-152). A reflection and the move never share a target, so
                // the action's lists tell their fates apart; two reflections strike the same
                // Pokémon, each reported in its own stretch (`called_outcomes`, R-M8-176).
                let mut reflections: Vec<Reflection> = Vec::new();
                // Each id with the reflection that strikes it (`None`: the move itself).
                let mut entries: Vec<(PokemonId, Option<usize>)> =
                    ids.iter().map(|id| (*id, None)).collect();
                if reflected_back
                    && !single
                    && let Some(def) = move_def.as_ref()
                {
                    let sent_back: Vec<(usize, PokemonId)> = decision
                        .called
                        .iter()
                        .enumerate()
                        .filter(|(_, called)| called.user != decision.user)
                        .filter_map(|(index, called)| {
                            who_to_id(&before.state, &called.user).map(|id| (index, id))
                        })
                        .filter(|(_, reflector)| ids.contains(reflector))
                        .collect();
                    let back_from = |reflector: PokemonId| -> Vec<PokemonId> {
                        compiled
                            .rules
                            .targeting()
                            .resolve(
                                compiled.rules.topology(),
                                &before.state,
                                reflector,
                                def.target,
                                before
                                    .state
                                    .active_position(actor, compiled.rules.topology())
                                    .map(DeclaredTarget),
                            )
                            .map(|targets| {
                                targets.pokemon_ids().filter(|p| *p != reflector).collect()
                            })
                            .unwrap_or_default()
                    };
                    let accuracy_before_try_hit = matches!(
                        compiled.rules.hits().step_order,
                        HitStepOrder::AccuracyFirst | HitStepOrder::AccuracyBeforeTryHit
                    );
                    let reflection_of = |id: PokemonId| {
                        sent_back.iter().position(|(_, reflector)| *reflector == id)
                    };
                    let strikes = |at: usize| {
                        let (_, reflector) = sent_back[at];
                        back_from(reflector)
                            .into_iter()
                            .map(move |id| (id, Some(at)))
                    };
                    entries = if accuracy_before_try_hit {
                        ids.iter()
                            .flat_map(|id| match reflection_of(*id) {
                                Some(at) => strikes(at).collect::<Vec<_>>(),
                                None => vec![(*id, None)],
                            })
                            .collect()
                    } else {
                        (0..sent_back.len())
                            .flat_map(strikes)
                            .chain(
                                ids.iter()
                                    .filter(|id| reflection_of(**id).is_none())
                                    .map(|id| (*id, None)),
                            )
                            .collect()
                    };
                    reflections = sent_back
                        .iter()
                        .enumerate()
                        .map(|(at, (called, reflector))| {
                            let first = entries.iter().position(|(_, by)| *by == Some(at));
                            let len = entries.iter().filter(|(_, by)| *by == Some(at)).count();
                            let first = first.unwrap_or(entries.len());
                            Reflection {
                                reflector: *reflector,
                                called: *called,
                                back: first..first + len,
                            }
                        })
                        .collect();
                }
                let listed = |whos: &[Who], id: PokemonId| {
                    whos.iter()
                        .any(|who| who_to_id(&before.state, who) == Some(id))
                };
                let mut fates: Vec<(Option<PokemonId>, TargetFate)> = entries
                    .iter()
                    .map(|(id, by)| {
                        // A reflection's target: by its reflector, in its own stretch.
                        let (attacker, missed, immune) = match by {
                            Some(at) => {
                                let outcomes = called_outcomes(&decision, reflections[*at].called);
                                (reflections[*at].reflector, outcomes.missed, outcomes.immune)
                            }
                            None => (actor, &decision.missed[..], &decision.immune[..]),
                        };
                        // The hit gate's own question: a sure hit, No Guard or a Lock-On
                        // reaches a hiding target.
                        let dodged = move_def.as_ref().is_some_and(|def| {
                            two_turn::dodged_by(&before.state, &compiled.rules, attacker, *id, def)
                                .is_some()
                        });
                        let fate = if dodged {
                            TargetFate::Dodged
                        } else if listed(immune, *id) {
                            TargetFate::Immune
                        } else if listed(missed, *id) {
                            TargetFate::Miss
                        } else {
                            TargetFate::Hit
                        };
                        (Some(*id), fate)
                    })
                    .collect();
                // A miss reported without a target applies to the first.
                if !decision.hit
                    && decision.cant.is_none()
                    && fates.iter().all(|(_, f)| *f == TargetFate::Hit)
                    && let Some((_, first)) = fates.first_mut()
                {
                    *first = TargetFate::Miss;
                }
                (fates, reflections)
            })
            .unwrap_or_default();
        // Dragon Darts (`smartTarget`) in doubles: the declared target and its partner, as the
        // engine splits them; one that took no dart failed its hit steps without a word —
        // immune, behind Protect, or else missed.
        let fates = match move_def.as_ref() {
            Some(def) if def.smart_target => {
                let declared = chosen_target
                    .and_then(|position| {
                        before
                            .state
                            .pokemon_at_position(position.0, compiled.rules.topology())
                    })
                    .or_else(|| fates.first().and_then(|(target, _)| *target));
                // A center of attention (Follow Me …) or a drawing ability holds both darts
                // (`engine::redirector_cancels_smart_target`, R-M8-169).
                let held = declared
                    .and_then(|target| {
                        pokemon_battle_engine::engine::deciding_redirector(
                            &before.state,
                            &compiled.rules,
                            &compiled.dex,
                            actor,
                            def,
                            Some(target),
                        )
                    })
                    .is_some_and(|(_, by)| {
                        pokemon_battle_engine::engine::redirector_cancels_smart_target(by)
                    });
                let split = declared
                    .filter(|_| !held)
                    .map(|target| {
                        pokemon_battle_engine::engine::smart_targets(
                            &before.state,
                            &compiled.rules,
                            actor,
                            target,
                        )
                    })
                    .unwrap_or_default();
                if split.len() > 1 {
                    split
                        .into_iter()
                        .map(|id| {
                            let struck = decision.hit_targets.iter().any(|who| {
                                who.as_ref().and_then(|w| who_to_id(&before.state, w)) == Some(id)
                            });
                            let blocked = pokemon_battle_engine::engine::type_immunity(
                                &before.state,
                                &compiled.rules,
                                id,
                                def,
                            )
                            .1
                            .is_some()
                                || before.state.pokemon(id).is_some_and(|p| {
                                    p.volatiles.contains(
                                        pokemon_battle_engine::state::VolatileConditionKind::Protect,
                                    )
                                });
                            let fate = if struck {
                                TargetFate::Hit
                            } else if blocked {
                                TargetFate::Immune
                            } else {
                                TargetFate::Miss
                            };
                            (Some(id), fate)
                        })
                        .collect()
                } else {
                    fates
                }
            }
            _ => fates,
        };
        // Shell Side Arm as it will strike: physical by its estimates against the first target,
        // a tie as its hint says (the plan reads contact reactions off the category's contact).
        let move_def = move_def.map(|def| match fates.first().and_then(|(target, _)| *target) {
            Some(target) if def.behavior.category_by_estimate => {
                let physical = pokemon_battle_engine::engine::physical_by_estimate(
                    &before.state,
                    &compiled.rules,
                    actor,
                    target,
                    &def,
                )
                .unwrap_or(decision.physical_hint == Some(true));
                match dex_move {
                    Some(dex_def) if physical => {
                        pokemon_battle_engine::engine::modified_move(
                            &before.state,
                            &compiled.rules,
                            actor,
                            dex_def,
                            true,
                        )
                        .0
                    }
                    _ => def,
                }
            }
            _ => def,
        });
        // The move as its `onPrepareHit` handlers leave it: Fling's and Natural Gift's power
        // and Fling's secondary from the item, Parental Bond's two strikes unless it hits
        // several targets.
        let move_def = move_def.map(|def| {
            pokemon_battle_engine::engine::prepared_move(
                &before.state,
                &compiled.rules,
                actor,
                def,
                fates.len(),
                fates.iter().find_map(|(target, _)| *target),
            )
        });
        // Duration hints, per victim (Teeter Dance confuses every Pokémon it reaches, each with
        // its own roll): confusion from the boundary frame, sleep from the later turns, binds
        // and Taunt / Encore / Disable (rolled before Gen 5) from the boundary frame.
        let mut duration_hints: Vec<(PokemonId, DurationKind, u16)> = Vec::new();
        for (who, kind) in &decision.applied {
            let Some(victim) = who_to_id(&before.state, who) else {
                continue;
            };
            let (class, hint) = if kind == "start:confusion" {
                (
                    DurationKind::Confusion,
                    confusion_duration_hint(
                        &compiled.rules,
                        oracle_frame,
                        decisions,
                        index,
                        who,
                        frame_member(&before.state, oracle_frame, who),
                    ),
                )
            } else if kind == "status:slp" {
                (
                    DurationKind::Sleep,
                    sleep_duration_hint(
                        all_decisions,
                        all_entries,
                        all_residuals,
                        turn_index,
                        index,
                        who,
                        compiled.rules.status().lifecycle.sleep_resets_on_switch_in,
                        compiled
                            .rules
                            .status()
                            .lifecycle
                            .sleep_usable_turns_come_back,
                        u16::from(compiled.rules.status().lifecycle.sleep_time_min),
                        u16::from(compiled.rules.status().lifecycle.sleep_time_max),
                        &sleep_steps(
                            &before.state,
                            &compiled.rules,
                            who,
                            turn_index,
                            boundary_frames,
                        ),
                    ),
                )
            } else if kind == "start:partiallytrapped" {
                let ticked = !who_to_id(&before.state, who).is_some_and(|id| {
                    untick.contains(&(id, MissedTick::Residual(ResidualKind::PartiallyTrapped)))
                });
                (
                    DurationKind::Bind,
                    bind_duration_hint(
                        oracle_frame,
                        &compiled.rules,
                        who,
                        frame_member(&before.state, oracle_frame, who),
                        ticked,
                    ),
                )
            } else {
                let conditions = compiled.rules.conditions();
                let (volatile, rule, residual) = if kind == "start:move: Taunt" {
                    ("taunt", conditions.taunt, ResidualKind::Taunt)
                } else if kind == "start:Encore" {
                    ("encore", conditions.encore, ResidualKind::Encore)
                } else if kind.starts_with("start:Disable") {
                    ("disable", conditions.disable, ResidualKind::Disable)
                } else {
                    continue;
                };
                let ticked = !who_to_id(&before.state, who)
                    .is_some_and(|id| untick.contains(&(id, MissedTick::Residual(residual))));
                let hint = if volatile == "disable" && conditions.disable_gen1 {
                    gen1_disable_hint(oracle_frame, decisions, index, who)
                } else {
                    timed_duration_hint(oracle_frame, &before.state, who, volatile, rule, ticked)
                };
                (DurationKind::Timed, hint)
            };
            if let Some(hint) = hint
                && !duration_hints
                    .iter()
                    .any(|(v, c, _)| *v == victim && *c == class)
            {
                duration_hints.push((victim, class, hint));
            }
        }
        // Gen 1's binding lock (§30.9): its turns from the user's `partialtrappinglock` at the
        // boundary, plus the tick the end of the turn already took.
        if compiled.rules.conditions().switching.binding_locks_user
            && move_def
                .as_ref()
                .is_some_and(|def| def.behavior.binding_lock)
        {
            let hint = binding_lock_hint(oracle_frame, &decision.user);
            duration_hints.push((actor, DurationKind::BindingLock, hint));
        }
        // Bide's duration (Gen 2) or time (Gen 1): the user's `bide` at the boundary.
        if move_def.as_ref().is_some_and(|def| def.id == MoveId::BIDE) {
            let ticked = !untick.contains(&(actor, MissedTick::Bide));
            let hint = bide_duration_hint(oracle_frame, &compiled.rules, &decision.user, ticked);
            duration_hints.push((actor, DurationKind::Bide, hint));
        }
        // Uproar (§15.12): Gen 3-4's turns, from the user's `uproar` at the boundary.
        if move_def.as_ref().is_some_and(|def| def.behavior.uproar) {
            let ticked = !untick.contains(&(actor, MissedTick::Residual(ResidualKind::Uproar)));
            let hint = uproar_turns_hint(oracle_frame, &compiled.rules, &decision.user, ticked);
            duration_hints.push((actor, DurationKind::Uproar, hint));
        }
        // Thrash and its kind (§15.12): the lock's length, and the fatigue's confusion.
        if move_def.as_ref().is_some_and(|def| def.behavior.rampage) {
            let hint = rampage_length_hint(
                &compiled.rules,
                &boundary_frames[turn_index..],
                &all_decisions[turn_index..],
                &all_residuals[turn_index..],
                &decision.user,
                !untick.contains(&(actor, MissedTick::Residual(ResidualKind::Rampage))),
            );
            duration_hints.push((actor, DurationKind::Rampage, hint));
        }
        if decision.fatigue
            && !duration_hints
                .iter()
                .any(|(v, c, _)| *v == actor && *c == DurationKind::Confusion)
            && let Some(hint) = confusion_duration_hint(
                &compiled.rules,
                oracle_frame,
                decisions,
                index,
                &decision.user,
                frame_member(&before.state, oracle_frame, &decision.user),
            )
        {
            duration_hints.push((actor, DurationKind::Confusion, hint));
        }
        let substitutes = substitute_hint(
            &before.state,
            &compiled.rules,
            oracle_frame,
            decisions,
            index,
        );
        // An intercepting Pursuit (§15.11) runs while its target is being called back: the
        // engine's accuracy (Gen 3+ never misses it) reads `called_back`.
        let intercept_state;
        let plan_state = if decision.intercepted {
            let mut state = before.state.clone();
            state.called_back = fates.first().and_then(|(target, _)| *target);
            intercept_state = state;
            &intercept_state
        } else if before.state.pending_movers.contains(&actor) && !decision.instructed {
            // The actor's action is under way: it no longer waits to move (Showdown's queue
            // shifted it out) — a move reflected back at it reads that (Zoom Lens:
            // `!queue.willMove(target)`, R-M8-125). An instructed use is an extra action: the
            // actor's own still waits (R-M8-149).
            let mut state = before.state.clone();
            state.pending_movers.retain(|pokemon| *pokemon != actor);
            intercept_state = state;
            &intercept_state
        } else {
            &before.state
        };
        let template = tape_template(
            plan_state,
            &compiled.rules,
            &compiled.dex,
            actor,
            move_def.as_ref(),
            &fates,
            &reflections,
            &decision,
            &duration_hints,
            &substitutes,
        );
        // A single-target move whose declared target is gone, or overridden by Encore, picks a
        // random adjacent foe first (Showdown `getRandomTarget`): the engine says whether it
        // draws, among whom; Showdown's `move` event names the one it picked.
        // The move as chosen, before `ModifyMove` (a non-Ghost's Curse aims at a random foe
        // there, its `self` target comes after: DESIGN.md R281) — the caller's, when it called
        // another: a called move's own target is drawn after the call (`tape_template`).
        let mut template = template;
        // The Curse glitch aims the action at its side's first slot as it starts
        // (`BattleState::curse_glitched`): no foe is drawn for it (R-M9-37).
        let curse_glitched = before.state.curse_glitched.contains(&actor);
        if let Some(def) = aimed_move
            .as_ref()
            .or(chosen_move)
            .filter(|_| !curse_glitched)
        {
            let declared = chosen_target.filter(|_| encored.is_none() && !struggling);
            if let Some(candidates) = pokemon_battle_engine::engine::random_retarget_candidates(
                &before.state,
                &compiled.rules,
                actor,
                def,
                declared,
            ) && candidates.len() > 1
                // The draw comes before BeforeMove: a move that never came (full paralysis, a
                // confusion self-hit) shows no target, and any pick behaves the same.
                // A target outside the candidates was redirected afterwards (Lightning Rod,
                // Follow Me): the draw still came, and any pick ends the same.
                // The action's own line names the pick: a reflection's names its own user
                // (R-M10-73).
                && let Some(picked) = match decision.move_name {
                    None => Some(0),
                    Some(_) => decision
                        .own_target
                        .as_ref()
                        .and_then(|who| who_to_id(&before.state, who))
                        .map(|_| {
                            drawn_target(
                                &before.state,
                                &candidates,
                                decision.own_target.as_ref(),
                                decision.redirected_by.as_ref(),
                            )
                            .unwrap_or(0)
                        }),
                }
            {
                template.insert(
                    0,
                    Some(RandomOverride::range(
                        RandomRequestKind::RandomTarget,
                        picked as u16,
                    )),
                );
            }
        }
        // Gen 9 Curse from a Ghost type aimed at an ally (§15.16): the foe it goes to, drawn
        // after BeforeMove — the last of its decisions (it has no others).
        if let Some(def) = move_def.as_ref()
            && decision.move_name.is_some()
            && let Some(candidates) = pokemon_battle_engine::engine::curse_redirect_candidates(
                &before.state,
                &compiled.rules,
                actor,
                def,
                chosen_target,
            )
            && candidates.len() > 1
        {
            let picked = decision
                .move_target
                .as_ref()
                .and_then(|who| who_to_id(&before.state, who))
                .and_then(|id| candidates.iter().position(|c| *c == id))
                .unwrap_or(0);
            template.push(Some(RandomOverride::range(
                RandomRequestKind::RandomTarget,
                picked as u16,
            )));
        }
        if std::env::var_os("REPLAY_DEBUG").is_some() {
            eprintln!("  {checkpoint}: {actor:?} plans {template:?}");
            eprintln!(
                "    oracle: move {:?} → {:?}, hit {}, immune {:?}, missed {:?}; fates {fates:?}",
                decision.move_name,
                decision.move_target,
                decision.hit,
                decision.immune,
                decision.missed
            );
        }
        // By slot, unless the Pokémon in that slot before the action is not the one named (a
        // Pokémon dragged in during the action): then by label.
        let expected: Vec<(PokemonId, u16)> = decision
            .damages
            .iter()
            .filter_map(|(who, hp)| {
                let by_slot = who_to_id(&before.state, who);
                let by_label = label_to_id(compiled, who);
                let id = match (by_slot, by_label) {
                    (Some(slot), Some(label)) if slot != label => label,
                    (Some(slot), _) => slot,
                    (None, label) => label?,
                };
                Some((id, *hp))
            })
            .collect();
        // The dances that followed it (Dancer's `runMove` inside this action, R-M8-78): each
        // dancer, the actor of its own move, uses it at the target its line names — planned as
        // an action's move is, on the state before the action — and its HP commits and hits
        // come after the action's own.
        let mut expected = expected;
        let mut dance_hits: Vec<(PokemonId, bool)> = Vec::new();
        for (offset, dance) in dances.iter().enumerate() {
            let Some(dancer) = who_to_id(&before.state, &dance.user) else {
                continue;
            };
            let modified = |name: &str| {
                compiled
                    .dex
                    .moves()
                    .iter()
                    .find(|m| m.name == name)
                    .map(|def| {
                        pokemon_battle_engine::engine::modified_move(
                            &before.state,
                            &compiled.rules,
                            dancer,
                            def,
                            false,
                        )
                        .0
                    })
            };
            let dance_def = dance.move_name.as_deref().and_then(modified);
            // A dance BeforeMove stopped (a confusion self-hit) prints no move line: it was the
            // action's move all the same, its random target drawn before the check (R-M8-122) —
            // nothing past BeforeMove (R-M8-126).
            let drawn_def = dance_def
                .clone()
                .or_else(|| decision.move_name.as_deref().and_then(modified));
            let listed = |whos: &[Who], id: PokemonId| {
                whos.iter()
                    .any(|who| who_to_id(&before.state, who) == Some(id))
            };
            let aimed = dance
                .move_target
                .as_ref()
                .and_then(|who| who_to_id(&before.state, who));
            // A dance BeforeMove stopped shows no move line; it aimed where Dancer sends it all the
            // same (`dancersTarget`, the engine's `dancer_target`), so a move that takes a target
            // has none drawn — only a `randomNormal` one draws (R-M9-47).
            let declared = aimed
                .or_else(|| {
                    let original = decision
                        .move_target
                        .as_ref()
                        .and_then(|who| who_to_id(&before.state, who));
                    Some(pokemon_battle_engine::engine::dancer_target(
                        &before.state,
                        actor,
                        dancer,
                        original,
                    ))
                })
                .and_then(|id| before.state.active_position(id, compiled.rules.topology()))
                .map(DeclaredTarget);
            let dance_fates: Vec<(Option<PokemonId>, TargetFate)> = dance_def
                .as_ref()
                .map(|def| {
                    let mut ids: Vec<PokemonId> = compiled
                        .rules
                        .targeting()
                        .resolve(
                            compiled.rules.topology(),
                            &before.state,
                            dancer,
                            def.target,
                            declared,
                        )
                        .map(|targets| targets.pokemon_ids().filter(|p| *p != dancer).collect())
                        .unwrap_or_default();
                    if matches!(
                        def.target,
                        TargetPattern::SelectedAny
                            | TargetPattern::SelectedAdjacent
                            | TargetPattern::SelectedFoe
                            | TargetPattern::SelectedAdjacentFoe
                            | TargetPattern::RandomAdjacentFoe
                    ) && let Some(actual) = aimed.filter(|id| *id != dancer)
                    {
                        ids = vec![actual];
                    }
                    ids.into_iter()
                        .map(|id| {
                            let fate = if listed(&dance.immune, id) {
                                TargetFate::Immune
                            } else if listed(&dance.missed, id) {
                                TargetFate::Miss
                            } else {
                                TargetFate::Hit
                            };
                            (Some(id), fate)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let dance_index = index + 1 + offset;
            let mut dance_hints: Vec<(PokemonId, DurationKind, u16)> = Vec::new();
            for (who, kind) in &dance.applied {
                if kind == "start:confusion"
                    && let Some(victim) = who_to_id(&before.state, who)
                    && let Some(hint) = confusion_duration_hint(
                        &compiled.rules,
                        oracle_frame,
                        decisions,
                        dance_index,
                        who,
                        frame_member(&before.state, oracle_frame, who),
                    )
                {
                    dance_hints.push((victim, DurationKind::Confusion, hint));
                }
            }
            // A dancer's lock (Petal Dance) is drawn, then dropped after the dance (`noLock`).
            if dance_def.as_ref().is_some_and(|def| def.behavior.rampage) {
                let hint = rampage_length_hint(
                    &compiled.rules,
                    &boundary_frames[turn_index..],
                    &all_decisions[turn_index..],
                    &all_residuals[turn_index..],
                    &dance.user,
                    true,
                );
                dance_hints.push((dancer, DurationKind::Rampage, hint));
            }
            let dance_substitutes = substitute_hint(
                &before.state,
                &compiled.rules,
                oracle_frame,
                decisions,
                dance_index,
            );
            // A `randomNormal` dance (Petal Dance) draws its target among the dancer's adjacent
            // foes first (`getRandomTarget`, before BeforeMove, R-M8-78); its line names the pick.
            if let Some(def) = drawn_def.as_ref()
                && let Some(candidates) = pokemon_battle_engine::engine::random_retarget_candidates(
                    &before.state,
                    &compiled.rules,
                    dancer,
                    def,
                    declared,
                )
                && candidates.len() > 1
            {
                let picked = aimed
                    .and_then(|id| candidates.iter().position(|c| *c == id))
                    .unwrap_or(0);
                template.push(Some(RandomOverride::range(
                    RandomRequestKind::RandomTarget,
                    picked as u16,
                )));
            }
            // A dancer the move it dances confused (Teeter Dance) meets that confusion at its
            // BeforeMove: the dance is planned on a state where it has it, as long as the action
            // gave it (R-M8-126).
            let confused_by_the_move;
            let dance_state = match duration_hints
                .iter()
                .find(|(victim, kind, _)| *victim == dancer && *kind == DurationKind::Confusion)
            {
                Some((_, _, time))
                    if before
                        .state
                        .pokemon(dancer)
                        .is_some_and(|p| p.volatiles.confusion_time().is_none()) =>
                {
                    let mut state = before.state.clone();
                    if let Some(pokemon) = state.pokemon_mut(dancer) {
                        pokemon.volatiles.set_confusion(actor, *time);
                    }
                    confused_by_the_move = state;
                    &confused_by_the_move
                }
                _ => &before.state,
            };
            template.extend(tape_template(
                dance_state,
                &compiled.rules,
                &compiled.dex,
                dancer,
                dance_def.as_ref(),
                &dance_fates,
                &[],
                dance,
                &dance_hints,
                &dance_substitutes,
            ));
            expected.extend(dance.damages.iter().filter_map(|(who, hp)| {
                let id = who_to_id(&before.state, who).or_else(|| label_to_id(compiled, who))?;
                Some((id, *hp))
            }));
            dance_hits.extend(
                dance
                    .hit_targets
                    .iter()
                    .zip(&dance.substitute_hits)
                    .filter_map(|(who, on_substitute)| {
                        Some((who_to_id(&before.state, who.as_ref()?)?, *on_substitute))
                    }),
            );
        }
        // Forced entries (Roar / Dragon Tail / Red Card): the engine picks from the healthy
        // bench in team order with a RandomTarget decision (none when the bench has one
        // Pokémon); the oracle's `drag` event names who came in. The drag also swaps the
        // Showdown party order like any switch. A Red Card drags as the action ends, after the
        // dances it set off: the log shows the `drag` after them, so it is read off the last
        // dance — the action's all the same, after the dances' draws (R-M9-53).
        let drags: Vec<&Who> = decision
            .dragged_in
            .iter()
            .chain(dances.iter().flat_map(|dance| dance.dragged_in.iter()))
            .collect();
        for entrant_who in drags {
            let Some(entrant) = label_to_id(compiled, entrant_who) else {
                continue;
            };
            let side = if entrant_who.side == 0 {
                SideId::P1
            } else {
                SideId::P2
            };
            let bench = before.state.healthy_bench(side);
            if bench.len() > 1
                && let Some(index) = bench.iter().position(|id| *id == entrant)
            {
                template.push(Some(RandomOverride::range(
                    RandomRequestKind::RandomTarget,
                    index as u16,
                )));
            }
            let slot = entrant_who.slot.unwrap_or(0);
            let outgoing = before.state.side(side).active.get(slot).copied().flatten();
            let party = &mut compiled.party[side.index()];
            if let (Some(from), Some(to)) = (
                outgoing.and_then(|o| party.iter().position(|p| *p == o)),
                party.iter().position(|p| *p == entrant),
            ) {
                party.swap(from, to);
            }
        }
        // The re-sort's tie decisions come before anything the action itself draws; the ties of
        // Neutralizing Gas clearing (Gastro Acid on its holder, or the holder fainting) after.
        template.splice(0..0, resort_ties.iter().copied().map(Some));
        template.extend(
            neutralizing_gas_tape(compiled, &before, oracle_frame)
                .into_iter()
                .map(Some),
        );
        // Instruct's target's use, built as this action ends (`resolveAction`): its priority
        // chances roll then, the last of this action's decisions (R-M9-32).
        if let Some(next) = decisions.get(decision_index).filter(|next| next.instructed) {
            template.extend(instructed_chance_tape(compiled, &before.state, next));
        }
        let hits: Vec<(PokemonId, bool)> = decision
            .hit_targets
            .iter()
            .zip(&decision.substitute_hits)
            .filter_map(|(who, on_substitute)| {
                Some((who_to_id(&before.state, who.as_ref()?)?, *on_substitute))
            })
            .chain(dance_hits)
            .collect();
        // A Substitute this action strikes and leaves standing, and a later action strikes
        // again: the boundary frame only shows it after both, so this action's roll on it stays
        // open. One a later hit of this action breaks (Rock Blast's second, the holder making a
        // new doll later) is not: it does not stand after the action (R-M8-183).
        let broken_here = |holder: &Who| {
            decision
                .substitute_struck
                .iter()
                .any(|(who, broken)| who == holder && *broken)
        };
        let deferred: Vec<PokemonId> = decision
            .substitute_struck
            .iter()
            .filter(|(holder, broken)| {
                !broken
                    && !broken_here(holder)
                    && decisions[decision_index..]
                        .iter()
                        .any(|later| later.substitute_struck.iter().any(|(who, _)| who == holder))
            })
            .filter_map(|(holder, _)| who_to_id(&before.state, holder))
            .collect();
        let ids = |whos: Vec<&Who>| -> Vec<PokemonId> {
            whos.into_iter()
                .filter_map(|who| who_to_id(&before.state, who))
                .collect()
        };
        let holds = ShownHolds {
            sashes: ids(decision.focus_sash_held.iter().collect()),
            bands: ids(decision
                .focus_band_held
                .iter()
                .map(|(holder, _)| holder)
                .collect()),
            sturdy: ids(decision.sturdy_held.iter().collect()),
        }
        .settled();
        let mut unused = None;
        let mut search = search_rolls(
            compiled,
            &before,
            trace_len,
            &template,
            &expected,
            &hits,
            &substitutes,
            &deferred,
            &holds,
            &mut unused,
        );
        // Present's drawn power shows only through the damage: its other bands.
        if search.chosen.is_none() {
            for variant in present_bands(&template, move_def.as_ref()) {
                let retry = search_rolls(
                    compiled,
                    &before,
                    trace_len,
                    &variant,
                    &expected,
                    &hits,
                    &substitutes,
                    &deferred,
                    &holds,
                    &mut unused,
                );
                if retry.chosen.is_some() {
                    search = retry;
                    break;
                }
            }
        }
        // Nothing fits: an earlier action of the boundary may have left a Substitute this one
        // strikes with the wrong HP. Retry it with its other HP values (latest first),
        // replaying the steps after it with the tapes they used, then search this one again.
        if search.chosen.is_none() {
            'backtrack: for from in (0..steps.len()).rev() {
                for tape in steps[from].alternatives.clone() {
                    let (engine, rebuilt, events) = replay_steps(compiled, &steps, from, &tape);
                    // Candidates restored from the rebuilt step's snapshot trace from 0.
                    let again = search_rolls(
                        compiled,
                        &engine.snapshot(),
                        0,
                        &template,
                        &expected,
                        &hits,
                        &substitutes,
                        &deferred,
                        &holds,
                        &mut None,
                    );
                    if again.chosen.is_some() {
                        // The log takes the rebuilt steps' events in place of the old ones.
                        compiled.log.truncate(steps[from].log_len);
                        compiled.log.extend(events);
                        steps = rebuilt;
                        search = again;
                        break 'backtrack;
                    }
                }
            }
        }

        // Nothing fits and the plan holds a crash's hidden critical-hit check (planned as not):
        // try it as one.
        if search.chosen.is_none()
            && let Some(index) = template.iter().position(|entry| {
                entry.is_some_and(|decision| {
                    decision.kind == RandomRequestKind::Critical && decision.optional
                })
            })
        {
            let mut critical = template.clone();
            critical[index] =
                Some(RandomOverride::bool(RandomRequestKind::Critical, true).optional());
            let again = search_rolls(
                compiled,
                &before,
                trace_len,
                &critical,
                &expected,
                &hits,
                &substitutes,
                &deferred,
                &holds,
                &mut None,
            );
            if again.chosen.is_some() {
                search = again;
            }
        }

        // Gen 1: a roll that broke a Substitute leaves `lastDamage` uncapped, which only a later
        // Bide release, Counter or binding lock's next turn shows: the roll sets leaving another
        // value are an open choice, taken where the replay was told to (`OpenChoices`, R-M8-102).
        if compiled.rules.conditions().retaliation.gen1_last_damage
            && let Some((engine, _, tape)) = &search.chosen
            && broke_a_doll(&before.state, engine.state())
            && last_damage_read_later(boundary_frames, turn_index)
        {
            let others = other_last_damages(compiled, &before, trace_len, tape, engine);
            if !others.is_empty() {
                let choice = report.open_choices.met.len();
                report.open_choices.met.push(others.len());
                if let Some(other) = report
                    .open_choices
                    .pins
                    .get(&choice)
                    .and_then(|pick| others.get(*pick))
                {
                    let mut again = restore_with_tape(compiled, &before, other.clone());
                    if let Ok(again_step) = again.step() {
                        search.chosen = Some((again, again_step, other.clone()));
                    }
                }
            }
        }

        // Gen 4 Future Sight / Doom Desire (§15.22): the damage computed at use drew its random
        // factor, which only the strike two turns later shows: the roll that stores that
        // damage (a strike that knocked its target out: any that stores enough).
        let foresight = compiled.rules.conditions().future_attack;
        if let Some(move_def) = move_def.as_ref()
            && move_def.behavior.future_attack
            && foresight.damage_at_use == Some(pokemon_battle_engine::rules::ForeseenRoll::AtUse)
            && let Some((_, _, tape)) = &search.chosen
            && let Some(roll_at) = tape
                .iter()
                .position(|decision| decision.kind == RandomRequestKind::DamageRoll)
            && let Some(target) = decision.move_target.as_ref()
            && let Some(foreseen) = future_damage_hint(boundary_frames, turn_index, target)
        {
            let damage = compiled.rules.damage();
            let tape = tape.clone();
            // A doll another move also struck that turn: every roll whose stored damage fits under
            // the turn's loss is an open choice, the highest first (`OpenChoices`, R-M8-164).
            let mut open: Vec<(BattleEngine, EngineStep, Vec<RandomOverride>)> = Vec::new();
            for roll in (damage.random_min..=damage.random_max).rev() {
                let mut again_tape = tape.clone();
                again_tape[roll_at] = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
                let mut again = restore_with_tape(compiled, &before, again_tape.clone());
                let Ok(again_step) = again.step() else {
                    continue;
                };
                // The attack this use foresaw: the user's on the target's slot (an earlier one
                // of the user's may still be waiting elsewhere, DESIGN.md R277).
                let side = if target.side == 0 {
                    SideId::P1
                } else {
                    SideId::P2
                };
                let stored = again
                    .state()
                    .side(side)
                    .future_attacks
                    .iter()
                    .find(|attack| {
                        attack.source == actor && Some(attack.slot) == target.slot.or(Some(0))
                    })
                    .and_then(|attack| attack.damage);
                match foreseen {
                    ForeseenDamage::Dealt(dealt, knocked_out) => {
                        let fits = stored.is_some_and(|stored| {
                            if knocked_out {
                                stored >= dealt
                            } else {
                                stored == dealt
                            }
                        });
                        if fits {
                            search.chosen = Some((again, again_step, again_tape));
                            break;
                        }
                    }
                    ForeseenDamage::AtMost(lost, broken) => {
                        if stored.is_some_and(|stored| broken || stored <= lost) {
                            open.push((again, again_step, again_tape));
                        }
                    }
                }
            }
            if !open.is_empty() {
                let choice = report.open_choices.met.len();
                report.open_choices.met.push(open.len() - 1);
                let pick = report
                    .open_choices
                    .pins
                    .get(&choice)
                    .map_or(0, |pick| pick + 1)
                    .min(open.len() - 1);
                search.chosen = Some(open.swap_remove(pick));
            }
        }

        // From Gen 5 a foreseen attack draws its random factor as it strikes (§15.22): one that
        // strikes, in the residual, a doll this action struck leaves the boundary frame showing
        // the doll after both, so the action's one roll is open — every roll that leaves the doll
        // standing above the frame's HP, the highest first (`OpenChoices`, R-M10-21). Gen 4's
        // strike deals what it stored at use, which `substitute_hint` adds.
        if foresight.damage_at_use != Some(pokemon_battle_engine::rules::ForeseenRoll::AtUse)
            && let Some((_, _, tape)) = &search.chosen
            && let [roll_at] = tape
                .iter()
                .enumerate()
                .filter(|(_, decision)| decision.kind == RandomRequestKind::DamageRoll)
                .map(|(at, _)| at)
                .collect::<Vec<_>>()[..]
            && let [(holder, false)] = &decision.substitute_struck[..]
            && residual_substitute_strike(oracle_frame, holder)
            && let Some(at_boundary) = frame_substitute_hp(oracle_frame, holder)
            && let Some(id) = who_to_id(&before.state, holder)
        {
            let damage = compiled.rules.damage();
            let tape = tape.clone();
            let mut open: Vec<(BattleEngine, EngineStep, Vec<RandomOverride>)> = Vec::new();
            let mut dolls: Vec<u16> = Vec::new();
            for roll in (damage.random_min..=damage.random_max).rev() {
                let mut again_tape = tape.clone();
                again_tape[roll_at] = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
                let mut again = restore_with_tape(compiled, &before, again_tape.clone());
                let Ok(again_step) = again.step() else {
                    continue;
                };
                // Rolls that leave the doll the same HP are one alternative.
                if let Some(doll) = substitute_hp(again.state(), id)
                    && doll > at_boundary
                    && !dolls.contains(&doll)
                {
                    dolls.push(doll);
                    open.push((again, again_step, again_tape));
                }
            }
            if !open.is_empty() {
                let choice = report.open_choices.met.len();
                report.open_choices.met.push(open.len() - 1);
                let pick = report
                    .open_choices
                    .pins
                    .get(&choice)
                    .map_or(0, |pick| pick + 1)
                    .min(open.len() - 1);
                search.chosen = Some(open.swap_remove(pick));
            }
        }

        // A Pokémon the action brings in (a Roar's drag) draws its switch-in ability's foe —
        // Trace's, Gen 4-5 Frisk's, Forewarn's — and so does a Trace holder that starts looking
        // during the action (its Neutralizing Gas holder fell), as the action's last draws
        // (DESIGN.md R263, R-M10-42): read from the chosen run, then run again.
        let search_chosen = search.chosen.map(|(engine, step, tape)| {
            let events = &engine.trace().events()[trace_len..];
            let mut lookers = entrants(events);
            if lookers.is_empty() && !traces(oracle_frame) {
                return (engine, step, tape);
            }
            for event in events {
                if let TraceEvent::AbilityChanged { pokemon, from, .. } = event
                    && *from == pokemon_battle_engine::ids::AbilityId::TRACE
                    && !lookers.contains(pokemon)
                {
                    lookers.push(*pokemon);
                }
            }
            let traced = entrant_ability_tape(
                compiled,
                &before.state,
                engine.state(),
                &lookers,
                oracle_frame,
            );
            if traced.is_empty() {
                return (engine, step, tape);
            }
            let mut again_tape = tape.clone();
            again_tape.extend(traced);
            let mut again = restore_with_tape(compiled, &before, again_tape.clone());
            match again.step() {
                Ok(again_step) => (again, again_step, again_tape),
                Err(_) => (engine, step, tape),
            }
        });
        match search_chosen {
            Some((engine, step, tape)) => {
                steps.push(ReplayedStep {
                    before,
                    log_len: compiled.log.len(),
                    tape,
                    alternatives: search.alternatives,
                });
                compiled.engine = engine;
                follow_position_swaps(compiled, trace_len);
                if !matches!(step, EngineStep::Progressed { .. }) {
                    break step;
                }
            }
            None => {
                report.divergences.push(match unused {
                    Some(note) => format!(
                        "{checkpoint}: {} by {:?} (cant={:?}): HP matched but the plan is not the engine's: {note}",
                        decision.move_name.as_deref().unwrap_or("-"),
                        decision.user,
                        decision.cant,
                    ),
                    None => format!(
                        "{checkpoint}: {} by {:?} (cant={:?}): no tape reproduces showdown's HP outcome {:?}; rust gave {:?}",
                        decision.move_name.as_deref().unwrap_or("-"),
                        decision.user,
                        decision.cant,
                        expected,
                        search.seen
                    ),
                });
                compiled.engine = probe;
                follow_position_swaps(compiled, trace_len);
                if !matches!(probe_step, EngineStep::Progressed { .. }) {
                    break probe_step;
                }
            }
        }
    };
    // An action the oracle reported that the engine never ran (a turn Gen 1 lost to
    // `cannotmove` where Showdown's Pokémon acted and was stopped): the boundary is over, the
    // log still has it (R190). Only a move or a `cant` counts: a bare cure of sleep is
    // read as a waking Pokémon's action even when another's move cured it (Haze, Wake-Up
    // Slap).
    if let Some(decision) = decisions[decision_index.min(decisions.len())..]
        .iter()
        .find(|decision| decision.move_name.is_some() || decision.cant.is_some())
    {
        report.divergences.push(format!(
            "{checkpoint}: the oracle's action {:?} by {:?} (cant={:?}) never ran in rust",
            decision.move_name, decision.user, decision.cant
        ));
    }
    step
}

/// The scheduler's sort key: order, faster first, sub-order, position.
type ResidualScheduleKey = (u8, std::cmp::Reverse<u32>, u8, usize, usize);

/// `outside_berry_confusions` with each draw's decision (a confusion's duration, Starf Berry's
/// stat).
fn outside_berry_confusion_hints(
    compiled: &Compiled,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
) -> Vec<(Who, RandomOverride, BerryConfusionAt)> {
    let reading = LogReading::for_rules(&compiled.rules, &compiled.dex);
    let events = oracle_frame["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    outside_berry_confusions(&events, compiled.rules.update().after_weather)
        .into_iter()
        .filter_map(|(position, who, at, draw)| {
            let before = extract_decisions(&events[..position], &reading).len();
            let later = decisions.get(before..).unwrap_or(&[]);
            berry_decision(draw, || {
                confusion_hint_after(
                    &compiled.rules,
                    oracle_frame,
                    later,
                    &who,
                    frame_member_at_slot(oracle_frame, &who),
                )
            })
            .map(|decision| (who, decision, at))
        })
        .collect()
}

/// The Pokémon a step brought in, in order.
fn entrants(events: &[TraceEvent]) -> Vec<PokemonId> {
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::Switched { incoming, .. } => Some(*incoming),
            _ => None,
        })
        .collect()
}

/// The leads in the order their Frisk lines come (Showdown runs the leads' switch-ins fastest
/// first); the ones without a line last.
fn frisk_order(compiled: &Compiled, state: &BattleState, initial: &Value) -> Vec<PokemonId> {
    let named: Vec<PokemonId> = frisk_reveals(compiled, state, initial)
        .into_iter()
        .filter_map(|(holder, _)| holder)
        .collect();
    let mut leads: Vec<PokemonId> = [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().flatten().copied())
        .collect();
    leads.sort_by_key(|lead| named.iter().position(|n| n == lead).unwrap_or(usize::MAX));
    leads
}

/// The oracle's Frisk lines in a frame: who frisked (`[of]`) and the item named.
fn frisk_reveals(
    compiled: &Compiled,
    state: &BattleState,
    oracle_frame: &Value,
) -> Vec<(
    Option<PokemonId>,
    Option<pokemon_battle_engine::ids::ItemId>,
)> {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["kind"] == "item")
        .filter_map(|event| {
            let args: Vec<&str> = event["args"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            if !args.contains(&"[from] ability: Frisk") {
                return None;
            }
            let holder = args
                .iter()
                .find_map(|arg| arg.strip_prefix("[of] "))
                .and_then(|text| who_from_text(&Value::from(text)))
                .and_then(|who| label_to_id(compiled, &who).or_else(|| who_to_id(state, &who)));
            let item = args
                .get(1)
                .and_then(|name| compiled.dex.item_by_name(name))
                .map(|item| item.id);
            Some((holder, item))
        })
        .collect()
}

/// The ability draws the initial frame's leads make, in the order the engine asks them: the
/// frame's Trace copies, Forewarn warnings (R-M8-103) and Frisk reveals in their log order — a
/// lead whose Trace copied a Frisk draws twice, the copy's `Start` running at once (Gen 4-5,
/// DESIGN.md R251). All are `Sample`s, so planning them in the wrong order would silently pick
/// another candidate. A Gen 4-5 Frisk that found nothing shows no line but still draws
/// (`side.randomFoe()`): a foe holding nothing, in its holder's turn among the leads (DESIGN.md
/// R-M8-84).
fn lead_ability_tape(
    compiled: &Compiled,
    state: &BattleState,
    leads: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    let id = |text: &str| {
        who_from_text(&Value::from(text))
            .and_then(|who| label_to_id(compiled, &who).or_else(|| who_to_id(state, &who)))
    };
    let frisk_draws = compiled.rules.conditions().frisk_one_random_foe;
    let foes_of = |seen: &BattleState, holder: PokemonId| -> Vec<PokemonId> {
        let Some(side) = seen.pokemon(holder).map(|pokemon| pokemon.side) else {
            return Vec::new();
        };
        seen.side(side.other())
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|foe| seen.pokemon(*foe).is_some_and(|p| !p.is_fainted()))
            .collect()
    };
    let mut seen = state.clone();
    // Each draw with the lead that makes it, in log order.
    let mut draws: Vec<(PokemonId, RandomOverride)> = Vec::new();
    let mut frisked: Vec<PokemonId> = Vec::new();
    let mut traced_frisk: Vec<PokemonId> = Vec::new();
    for event in oracle_frame["events"].as_array().into_iter().flatten() {
        let kind = event["kind"].as_str().unwrap_or("");
        let args: Vec<&str> = event["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let trace = args.contains(&"[from] ability: Trace");
        let frisk = args.contains(&"[from] ability: Frisk");
        if kind == "ability" && trace {
            let (Some(holder), Some(foe)) = (
                args.first().and_then(|text| id(text)),
                args.iter()
                    .find_map(|arg| arg.strip_prefix("[of] "))
                    .and_then(id),
            ) else {
                continue;
            };
            if leads.contains(&holder) {
                let candidates =
                    pokemon_battle_engine::engine::trace_candidates(&seen, &compiled.rules, holder);
                if candidates.len() >= 2
                    && let Some(index) = candidates.iter().position(|c| *c == foe)
                {
                    draws.push((
                        holder,
                        RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
                    ));
                }
                if args.get(1) == Some(&"Frisk") {
                    traced_frisk.push(holder);
                }
            }
            // The copy is made however the draw went: a foe the next lead's Trace may copy.
            if let Some(copied) = args
                .get(1)
                .and_then(|name| compiled.dex.ability_by_name(name))
                && let Some(pokemon) = seen.pokemon_mut(holder)
            {
                pokemon.change_ability(copied.id);
            }
        } else if kind == "transform" {
            // Imposter's copy as the leads come in (`-transform|USER|TARGET`): a lead whose
            // Trace looks after it sees the ability the user took (DESIGN.md R-M9-23).
            if let (Some(user), Some(target)) = (
                args.first().and_then(|text| id(text)),
                args.get(1).and_then(|text| id(text)),
            ) && let Some(ability) = seen
                .pokemon(target)
                .map(pokemon_battle_engine::engine::raw_ability)
                && let Some(pokemon) = seen.pokemon_mut(user)
            {
                pokemon.change_ability(ability);
            }
        } else if kind == "activate"
            && let Some((holder, draw)) = forewarn_draw(compiled, &seen, &args)
            && leads.contains(&holder)
        {
            draws.push((holder, draw));
        } else if kind == "item" && frisk && frisk_draws {
            let Some(holder) = args
                .iter()
                .find_map(|arg| arg.strip_prefix("[of] "))
                .and_then(id)
            else {
                continue;
            };
            if !leads.contains(&holder) {
                continue;
            }
            frisked.push(holder);
            let foes = foes_of(&seen, holder);
            if foes.len() < 2 {
                continue;
            }
            let named = args
                .get(1)
                .and_then(|name| compiled.dex.item_by_name(name))
                .map(|item| item.id);
            let index = foes
                .iter()
                .position(|foe| seen.pokemon(*foe).and_then(|p| p.held_item) == named)
                .unwrap_or(0);
            draws.push((
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ));
        }
    }
    if frisk_draws {
        for &holder in leads {
            let native = state.pokemon(holder).is_some_and(|pokemon| {
                pokemon_battle_engine::engine::ability_active(state, &compiled.rules, pokemon)
                    == Some(pokemon_battle_engine::ids::AbilityId::FRISK)
            });
            if !(native || traced_frisk.contains(&holder)) || frisked.contains(&holder) {
                continue;
            }
            let foes = foes_of(&seen, holder);
            if foes.len() < 2 {
                continue;
            }
            let index = foes
                .iter()
                .position(|foe| seen.pokemon(*foe).is_some_and(|p| p.held_item.is_none()))
                .unwrap_or(0);
            draws.push((
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ));
        }
    }
    // The leads start fastest first, the ties as Showdown reported them (`lead_tie_tape`); a
    // lead's own draws keep their order.
    let order = lead_order(compiled, Some(oracle_frame));
    draws.sort_by_key(|(holder, _)| order.iter().position(|lead| lead == holder));
    draws.into_iter().map(|(_, draw)| draw).collect()
}

/// Forewarn's draw (`sample` among the foes' strongest moves, two or more) that the frame's
/// `-activate|holder|ability: Forewarn|<move>|[of] <foe>` (`args`) shows: the holder and the
/// candidate it names, optional like Trace's (R-M8-103). The gen4 mod names no foe: the first
/// candidate with that move stands for it — they warn alike (R-M8-138).
fn forewarn_draw(
    compiled: &Compiled,
    state: &BattleState,
    args: &[&str],
) -> Option<(PokemonId, RandomOverride)> {
    let id = |text: &str| {
        who_from_text(&Value::from(text))
            .and_then(|who| label_to_id(compiled, &who).or_else(|| who_to_id(state, &who)))
    };
    forewarn_pick(&compiled.rules, &compiled.dex, state, args, id)
}

/// `forewarn_draw`'s reading, the line's Pokémon named through `id`.
fn forewarn_pick(
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    state: &BattleState,
    args: &[&str],
    id: impl Fn(&str) -> Option<PokemonId>,
) -> Option<(PokemonId, RandomOverride)> {
    if args.get(1) != Some(&"ability: Forewarn") {
        return None;
    }
    let holder = args.first().and_then(|text| id(text))?;
    let foe = args
        .iter()
        .find_map(|arg| arg.strip_prefix("[of] "))
        .and_then(&id);
    let named = dex.move_by_name(args.get(2)?)?.id;
    let candidates = pokemon_battle_engine::engine::forewarn_candidates(state, rules, holder);
    if candidates.len() < 2 {
        return None;
    }
    let index = candidates
        .iter()
        .position(|c| c.0 == named && foe.is_none_or(|foe| c.1 == foe))?;
    Some((
        holder,
        RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
    ))
}

/// Forewarn's draws for the holders among `entrants` (`forewarn_draw`), `state` the engine's
/// once they are in.
fn forewarn_draws(
    compiled: &Compiled,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<(PokemonId, RandomOverride)> {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["kind"] == "activate")
        .filter_map(|event| {
            let args: Vec<&str> = event["args"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            forewarn_draw(compiled, state, &args)
        })
        .filter(|(holder, _)| entrants.contains(holder))
        .collect()
}

/// Gen 4-5 Frisk's foe (`side.randomFoe()`: a `Sample` among two foes or more) for each Frisk
/// holder among `entrants`: the first foe holding the item the oracle's `-item||<item>|[from]
/// ability: Frisk|[of] <holder>` names — or, without that line, one holding nothing. `state`
/// is the engine's once they are in. Optional, and put before the entry berries: a berry eaten
/// on the hazards comes before the ability, one eaten at the item's turn after it.
fn frisk_draws(
    compiled: &Compiled,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<(PokemonId, RandomOverride)> {
    if !compiled.rules.conditions().frisk_one_random_foe {
        return Vec::new();
    }
    let revealed = frisk_reveals(compiled, state, oracle_frame);
    entrants
        .iter()
        .filter_map(|&holder| {
            let pokemon = state.pokemon(holder)?;
            if pokemon_battle_engine::engine::ability_active(state, &compiled.rules, pokemon)
                != Some(pokemon_battle_engine::ids::AbilityId::FRISK)
            {
                return None;
            }
            let foes: Vec<PokemonId> = state
                .side(pokemon.side.other())
                .active
                .iter()
                .flatten()
                .copied()
                .filter(|foe| state.pokemon(*foe).is_some_and(|p| !p.is_fainted()))
                .collect();
            if foes.len() < 2 {
                return None;
            }
            let named = revealed
                .iter()
                .find(|(by, _)| *by == Some(holder))
                .and_then(|(_, item)| *item);
            let index = foes
                .iter()
                .position(|foe| state.pokemon(*foe).and_then(|p| p.held_item) == named)
                .unwrap_or(0);
            Some((
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ))
        })
        .collect()
}

/// The entrants' switch-in ability draws (Trace's foe, Frisk's), in the order the engine runs
/// their switch-ins: fastest first (`engine::entry_speed`), one entrant's own draws in their
/// order (a Trace that copied Frisk draws for the copy after its own). Both are `Sample`s, so
/// a fixed Frisk-then-Trace order picked the other's foe whenever the Trace holder was the
/// faster (DESIGN.md R-M8-92). A Trace already on the field that seeks as a foe comes in looks
/// after the switch-ins, in the log's order.
fn entrant_ability_tape(
    compiled: &Compiled,
    before: &BattleState,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    let (mut entering, seeking): (Vec<_>, Vec<_>) =
        trace_draws(compiled, before, state, entrants, oracle_frame)
            .into_iter()
            .partition(|(holder, _)| entrants.contains(holder));
    entering.extend(frisk_draws(compiled, state, entrants, oracle_frame));
    entering.extend(forewarn_draws(compiled, state, entrants, oracle_frame));
    entering.sort_by_key(|(holder, _)| {
        std::cmp::Reverse(pokemon_battle_engine::engine::entry_speed(
            state,
            &compiled.rules,
            *holder,
        ))
    });
    entering
        .into_iter()
        .chain(seeking)
        .map(|(_, draw)| draw)
        .collect()
}

/// Whether the frame shows Forewarn naming a move.
fn forewarns(oracle_frame: &Value) -> bool {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|event| event["kind"] == "activate" && event["args"][1] == "ability: Forewarn")
}

/// Whether the frame shows Trace copying an ability.
fn traces(oracle_frame: &Value) -> bool {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|event| {
            event["kind"] == "ability"
                && event["args"]
                    .as_array()
                    .is_some_and(|args| args.iter().any(|arg| arg == "[from] ability: Trace"))
        })
}

/// Trace's foe (`this.sample(possibleTargets)`: a `Sample` among two foes or more, R237) for
/// each of the frame's `-ability|<holder>|<ability>|…|[from] ability: Trace|[of] <foe>` lines,
/// in order, whose holder looks in this step — one of its `entrants`, or a holder still seeking
/// in `before` (a foe came in): the foe's place among `engine::trace_candidates` on the field
/// as that Trace saw it — `state` (the engine's once the step's entrants are in) with the
/// earlier lines' copies made, since a Trace that copied an ability is a foe the next Trace may
/// copy (DESIGN.md R251). A line an earlier step already settled draws nothing here.
/// Optional: the step may not be the one Trace looks in.
fn trace_tape(
    compiled: &Compiled,
    before: &BattleState,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    trace_draws(compiled, before, state, entrants, oracle_frame)
        .into_iter()
        .map(|(_, draw)| draw)
        .collect()
}

/// `trace_tape`'s draws, each with its Trace holder.
fn trace_draws(
    compiled: &Compiled,
    before: &BattleState,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<(PokemonId, RandomOverride)> {
    let id = |text: &str| {
        who_from_text(&Value::from(text))
            .and_then(|who| label_to_id(compiled, &who).or_else(|| who_to_id(state, &who)))
    };
    let mut seen = state.clone();
    let mut tape = Vec::new();
    for event in oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["kind"] == "ability")
    {
        let args: Vec<&str> = event["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !args.contains(&"[from] ability: Trace") {
            continue;
        }
        let (Some(holder), Some(foe)) = (
            args.first().and_then(|text| id(text)),
            args.iter()
                .find_map(|arg| arg.strip_prefix("[of] "))
                .and_then(id),
        ) else {
            continue;
        };
        let looks = entrants.contains(&holder)
            || before
                .pokemon(holder)
                .is_some_and(|pokemon| pokemon.ability_state.trace_seek);
        let candidates =
            pokemon_battle_engine::engine::trace_candidates(&seen, &compiled.rules, holder);
        // The field as an entrant's Trace first looked at it: a foe that comes in later in the
        // step is not there yet, the Pokémon it replaces still is.
        let mut at_entry = seen.clone();
        if let Some(at) = entrants.iter().position(|entrant| *entrant == holder) {
            for later in &entrants[at + 1..] {
                let Some(side) = at_entry.pokemon(*later).map(|pokemon| pokemon.side) else {
                    continue;
                };
                let Some(slot) = at_entry
                    .side(side)
                    .active
                    .iter()
                    .position(|occupant| *occupant == Some(*later))
                else {
                    continue;
                };
                let previous = before.side(side).active.get(slot).copied().flatten();
                at_entry.side_mut(side).active[slot] = previous;
            }
        }
        let first_look =
            pokemon_battle_engine::engine::trace_candidates(&at_entry, &compiled.rules, holder);
        // The line names a foe not there at that first look: Trace drew a foe whose ability
        // `setAbility` refuses (Gen 9 Gulp Missile: `cantsuppress` without `notrace`) and kept
        // looking, to copy the named foe at an `Update` once it had come in. It looked twice
        // before: as it started, and at the `Update` that closes its switch — the same foe
        // each time.
        if looks && entrants.contains(&holder) && !first_look.contains(&foe) {
            if first_look.len() >= 2
                && let Some(index) = first_look.iter().position(|candidate| {
                    at_entry.pokemon(*candidate).is_some_and(|pokemon| {
                        compiled
                            .rules
                            .conditions()
                            .cant_suppress
                            .contains(&pokemon_battle_engine::engine::raw_ability(pokemon))
                    })
                })
            {
                for _ in 0..2 {
                    tape.push((
                        holder,
                        RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
                    ));
                }
            }
            // The foe came in later in the step: the copy is this step's too, from the field
            // the step leaves (otherwise a later step's, where the holder still seeks).
            if !candidates.contains(&foe) {
                continue;
            }
        }
        if looks
            && candidates.len() >= 2
            && let Some(index) = candidates.iter().position(|c| *c == foe)
        {
            tape.push((
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ));
        }
        if let Some(copied) = args
            .get(1)
            .and_then(|name| compiled.dex.ability_by_name(name))
            && let Some(pokemon) = seen.pokemon_mut(holder)
        {
            pokemon.change_ability(copied.id);
        }
    }
    tape
}

/// The berry draws of the residual's closing `Update` (no berry handler of its own: Gen 4+), after
/// every residual handler — Moody's included (DESIGN.md R263) — in the active order.
fn residual_end_berry_tape(
    state: &BattleState,
    rules: &ResolvedRules,
    berry_confusions: &[(Who, RandomOverride, BerryConfusionAt)],
) -> Vec<RandomOverride> {
    if rules.residual().key(ResidualKind::Berry).is_some() {
        return Vec::new();
    }
    [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().copied().flatten())
        .flat_map(|id| {
            berry_confusions
                .iter()
                .filter(move |(who, _, at)| {
                    *at == BerryConfusionAt::ResidualEnd && who_to_id(state, who) == Some(id)
                })
                .map(|(_, decision, _)| *decision)
        })
        .collect()
}

/// The switch-ins' berry draws, optional (whichever switch-in asks takes one).
fn entry_berry_durations(
    berry_confusions: &[(Who, RandomOverride, BerryConfusionAt)],
) -> Vec<RandomOverride> {
    berry_confusions
        .iter()
        .filter(|(_, _, at)| *at == BerryConfusionAt::Entry)
        .map(|(_, decision, _)| decision.optional())
        .collect()
}

/// The end-of-turn decisions in schedule order (order, faster first, sub-order, position — as
/// the scheduler orders them): Shed Skin for every statused holder on the field, and Gen 2's
/// thaw roll for every frozen one.
/// The end of the turn's `SpeedTie` decisions (DESIGN.md §12.4): for every group of tied
/// residual handlers, then of Pokémon the weather chips at the same speed, the one the closing
/// events show first gets the larger draw. A handler that shows nothing (a Taunt still
/// running) goes last; its place changes nothing observable.
/// The leads in the order `run_lead_switch_ins` starts them: fastest first, the ties in the
/// order Showdown first names them (as `lead_tie_tape` plans).
fn lead_order(compiled: &Compiled, initial: Option<&Value>) -> Vec<PokemonId> {
    let shown_at = first_named_at(compiled, initial);
    let mut leads = compiled.engine.lead_speeds();
    leads.sort_by_key(|(id, speed)| (std::cmp::Reverse(*speed), shown_at(*id)));
    leads.into_iter().map(|(id, _)| id).collect()
}

/// Where each Pokémon is first named among a frame's events (switches aside): the leads in the
/// initial frame, simultaneous entrants in their replacements' frame.
fn first_named_at(compiled: &Compiled, initial: Option<&Value>) -> impl Fn(PokemonId) -> usize {
    let events = initial
        .and_then(|frame| frame["events"].as_array())
        .cloned()
        .unwrap_or_default();
    let state = &compiled.engine.state();
    let mut named: Vec<PokemonId> = Vec::new();
    for event in events.iter().filter(|event| event["kind"] != "switch") {
        let args = event["args"].as_array().cloned().unwrap_or_default();
        let whos = std::iter::once(serde_json::from_value::<Who>(event["pokemon"].clone()).ok())
            .chain(args.iter().map(|arg| {
                let text = arg.as_str()?;
                who_from_text(&Value::from(text.strip_prefix("[of] ").unwrap_or(text)))
            }))
            .flatten();
        for who in whos {
            if let Some(id) = label_to_id(compiled, &who).or_else(|| who_to_id(state, &who))
                && !named.contains(&id)
            {
                named.push(id);
            }
        }
    }
    move |id: PokemonId| named.iter().position(|n| *n == id).unwrap_or(usize::MAX)
}

/// The SpeedTie decisions of the leads' switch-in batch (`BattleEngine::lead_speed_ties`): tied
/// leads go in the order the initial frame first names them (the source of a weather, the
/// Intimidate user …); one it never names comes last.
fn lead_tie_tape(compiled: &Compiled, initial: Option<&Value>) -> Vec<RandomOverride> {
    let ties = compiled.engine.lead_speed_ties();
    if ties.is_empty() {
        return Vec::new();
    }
    let shown_at = first_named_at(compiled, initial);
    ties.iter()
        .map(|(lead, speed)| {
            let mut members: Vec<PokemonId> = ties
                .iter()
                .filter(|(_, s)| s == speed)
                .map(|(id, _)| *id)
                .collect();
            members.sort_by_key(|id| shown_at(*id));
            let rank = members.iter().position(|id| id == lead).unwrap_or(0);
            RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
        })
        .collect()
}

/// The SpeedTie decisions of simultaneous replacements (a probe's `SwitchInTiesDrawn`): tied
/// entrants go in the order the frame first names them, as the leads do. Unplanned, the
/// decisions fell to the PRNG — and a Trace entrant's optional foe draw, planned next, was
/// passed over by the first of them (R-M8-140).
fn entry_tie_tape(
    compiled: &Compiled,
    events: &[TraceEvent],
    frame: &Value,
) -> Vec<RandomOverride> {
    let shown_at = first_named_at(compiled, Some(frame));
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::SwitchInTiesDrawn { tied } => Some(tied),
            _ => None,
        })
        .flat_map(|tied| {
            tied.iter().map(|(entrant, speed)| {
                let mut members: Vec<PokemonId> = tied
                    .iter()
                    .filter(|(_, s)| s == speed)
                    .map(|(id, _)| *id)
                    .collect();
                members.sort_by_key(|id| shown_at(*id));
                let rank = members.iter().position(|id| id == entrant).unwrap_or(0);
                RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
            })
        })
        .collect()
}

/// The residual's `SpeedTie` decisions: the tapes to try, the first the plan. A tied handler
/// the log shows goes where it shows; one it does not — a duration-only handler, counting down
/// in silence — keeps its collection order in the plan, and the other tapes put it everywhere
/// else among its group. No frame shows that order unless the battle ends between them (the
/// residual stops there): the replay then takes the tape whose frame is the oracle's.
fn residual_tie_tapes(
    compiled: &Compiled,
    state: &BattleState,
    oracle_frame: &Value,
) -> Vec<Vec<RandomOverride>> {
    let ties = compiled.engine.pending_residual_ties();
    if ties.handlers.is_empty() && ties.durations.is_empty() && ties.weather.is_empty() {
        return vec![Vec::new()];
    }
    // What each closing event stands for: a residual kind on a Pokémon, or (`None`) the
    // weather's chip on it.
    let events = oracle_frame["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let start = events
        .iter()
        .position(|event| event["kind"] == "residual")
        .map_or(events.len(), |marker| marker + 1);
    let id_of = |who: &Who| label_to_id(compiled, who).or_else(|| who_to_id(state, who));
    let mut shown: Vec<(PokemonId, Option<ResidualKind>)> = Vec::new();
    for event in &events[start..] {
        let from = event["from"].as_str().unwrap_or("");
        let args = event["args"].as_array().cloned().unwrap_or_default();
        let arg = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or("");
        let pokemon = serde_json::from_value::<Who>(event["pokemon"].clone()).ok();
        let (who, what) = match event["kind"].as_str().unwrap_or("") {
            "damage" | "heal" => {
                let what = if from.contains("Leech Seed") {
                    Some(ResidualKind::LeechSeed)
                } else if from == "[from] psn" {
                    Some(ResidualKind::Poison)
                } else if from == "[from] brn" {
                    Some(ResidualKind::Burn)
                } else if from == "[from] item: Leftovers" {
                    Some(ResidualKind::Leftovers)
                } else if from == "[from] item: Black Sludge" {
                    Some(ResidualKind::BlackSludge)
                } else if from == "[from] move: Ingrain" {
                    Some(ResidualKind::Ingrain)
                } else if from == "[from] Grassy Terrain" {
                    Some(ResidualKind::GrassyTerrain)
                } else if from == "[from] Salt Cure" {
                    Some(ResidualKind::SaltCure)
                } else if event["of"].as_str() == Some("[partiallytrapped]") {
                    Some(ResidualKind::PartiallyTrapped)
                } else if from == "[from] Sandstorm" || from == "[from] Hail" {
                    None
                } else {
                    continue;
                };
                (pokemon, what)
            }
            "status" if from.contains("item: Toxic Orb") || from.contains("item: Flame Orb") => {
                (pokemon, Some(ResidualKind::StatusOrb))
            }
            "boost" if event["stat"] == "spe" => (pokemon, Some(ResidualKind::SpeedBoost)),
            "unboost" if event["stat"] == "spe" => (pokemon, Some(ResidualKind::SyrupBomb)),
            "curestatus" if event["status"] == "frz" => (pokemon, Some(ResidualKind::FreezeThaw)),
            "activate" if arg(1) == "ability: Shed Skin" => {
                (who_from_text(&args[0]), Some(ResidualKind::ShedSkin))
            }
            "activate" if arg(1) == "ability: Cud Chew" => {
                (who_from_text(&args[0]), Some(ResidualKind::CudChew))
            }
            // Moody (`-ability|p|Moody|boost`, then its `-boost` / `-unboost`).
            "ability" if arg(1) == "Moody" && arg(2) == "boost" => (
                args.first().and_then(who_from_text),
                Some(ResidualKind::Moody),
            ),
            "item" if arg(2) == "[from] ability: Harvest" => (
                args.first().and_then(who_from_text),
                Some(ResidualKind::Harvest),
            ),
            // Pickup's find (`-item|p|X|[from] ability: Pickup`): two holders that tie pick up in
            // the order shown — the first's find may be the second's to pick up (R-M8-84).
            "item" if arg(2) == "[from] ability: Pickup" => (
                args.first().and_then(who_from_text),
                Some(ResidualKind::Pickup),
            ),
            // A lock's fatigue (Thrash and its kind) ending in the residual: its `-start … confusion
            // [fatigue]` orders the tied handlers — two locks may end on the same turn
            // (DESIGN.md R-M8-44).
            "start"
                if arg(1) == "confusion"
                    && args.iter().any(|a| matches!(a.as_str(), Some("[fatigue]"))) =>
            {
                // A `-start` names its Pokémon in its first argument (R-M8-79).
                (
                    args.first().and_then(who_from_text),
                    Some(ResidualKind::Rampage),
                )
            }
            // Perish Song's count (`-start|p|perishN`), its end at 0 included.
            "start" if arg(1).starts_with("perish") => (
                args.first().and_then(who_from_text),
                Some(ResidualKind::PerishSong),
            ),
            "end" => {
                let what = match arg(1) {
                    // A foreseen attack striking (DESIGN.md R277).
                    "move: Future Sight" | "move: Doom Desire" => ResidualKind::FutureAttack,
                    "move: Yawn" => ResidualKind::Yawn,
                    "move: Taunt" => ResidualKind::Taunt,
                    "Encore" => ResidualKind::Encore,
                    "Disable" => ResidualKind::Disable,
                    _ => continue,
                };
                (args.first().and_then(who_from_text), Some(what))
            }
            _ => continue,
        };
        if let Some(id) = who.as_ref().and_then(id_of)
            && !shown.contains(&(id, what))
        {
            shown.push((id, what));
        }
    }
    let same = |kind: ResidualKind| match kind {
        ResidualKind::Toxic => ResidualKind::Poison,
        other => other,
    };
    let position = |id: PokemonId, what: Option<ResidualKind>| {
        shown
            .iter()
            .position(|(who, w)| *who == id && w.map(same) == what.map(same))
            .unwrap_or(usize::MAX)
    };
    let ranked = |members: Vec<(usize, usize)>, of: usize| {
        let mut ordered = members;
        ordered.sort_by_key(|(_, shown_at)| *shown_at);
        let rank = ordered.iter().position(|(i, _)| *i == of).unwrap_or(0);
        RandomOverride::raw(RandomRequestKind::SpeedTie, (ordered.len() - rank) as u64)
    };
    // The handlers' decisions in draw order — the ones with a callback, then the duration-only
    // ones — each with its group and where the log shows it (`usize::MAX`: nowhere).
    let drawn: Vec<(usize, usize)> = ties
        .handlers
        .iter()
        .map(|(holder, kind, group)| (*group, position(*holder, Some(*kind))))
        .chain(ties.durations.iter().map(|(_, group)| (*group, usize::MAX)))
        .collect();
    let mut weather = Vec::new();
    for (index, (_, group)) in ties.weather.iter().enumerate() {
        let members = ties
            .weather
            .iter()
            .enumerate()
            .filter(|(_, (_, g))| g == group)
            .map(|(i, (pokemon, _))| (i, position(*pokemon, None)))
            .collect();
        weather.push(ranked(members, index));
    }
    // Each group's orders: the plan's (shown first, the rest in draw order), then — for a
    // group with a handler the log does not show — every other order that keeps the shown ones
    // as shown. An order lists the group's members (indices into `drawn`) first to last.
    let mut groups: Vec<usize> = drawn.iter().map(|(group, _)| *group).collect();
    groups.dedup();
    groups.sort_unstable();
    groups.dedup();
    let orders: Vec<Vec<Vec<usize>>> = groups
        .iter()
        .map(|group| {
            let mut members: Vec<usize> =
                (0..drawn.len()).filter(|i| drawn[*i].0 == *group).collect();
            members.sort_by_key(|i| (drawn[*i].1, *i));
            let mut orders = vec![members.clone()];
            if members.iter().any(|i| drawn[*i].1 == usize::MAX) && members.len() <= 5 {
                let shown: Vec<usize> = members
                    .iter()
                    .copied()
                    .filter(|i| drawn[*i].1 != usize::MAX)
                    .collect();
                for order in permutations(&members) {
                    let keeps_shown = order
                        .iter()
                        .copied()
                        .filter(|i| drawn[*i].1 != usize::MAX)
                        .eq(shown.iter().copied());
                    if keeps_shown && order != members {
                        orders.push(order);
                    }
                }
            }
            orders
        })
        .collect();
    let tape_for = |choice: &[usize]| {
        let mut values = vec![0u64; drawn.len()];
        for (group, &pick) in choice.iter().enumerate() {
            let order = &orders[group][pick];
            for (rank, member) in order.iter().enumerate() {
                values[*member] = (order.len() - rank) as u64;
            }
        }
        let mut tape: Vec<RandomOverride> = values
            .into_iter()
            .map(|value| RandomOverride::raw(RandomRequestKind::SpeedTie, value))
            .collect();
        tape.extend(weather.iter().copied());
        tape
    };
    // The plan, then each group's other orders one group at a time, then their combinations
    // (a handful at most: tied silent handlers come in twos and threes).
    let mut choice = vec![0usize; orders.len()];
    let mut tapes = vec![tape_for(&choice)];
    'combinations: loop {
        let mut digit = 0;
        loop {
            if digit == orders.len() {
                break 'combinations;
            }
            choice[digit] += 1;
            if choice[digit] < orders[digit].len() {
                break;
            }
            choice[digit] = 0;
            digit += 1;
        }
        tapes.push(tape_for(&choice));
        if tapes.len() >= 64 {
            break;
        }
    }
    tapes
}

/// Every order of `items`.
fn permutations(items: &[usize]) -> Vec<Vec<usize>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut all = Vec::new();
    for (index, first) in items.iter().enumerate() {
        let mut rest = items.to_vec();
        rest.remove(index);
        for mut tail in permutations(&rest) {
            tail.insert(0, *first);
            all.push(tail);
        }
    }
    all
}

#[allow(clippy::too_many_arguments)]
fn residual_tape(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    cures: &[(Who, ResidualKind)],
    faints: &[(Who, ResidualKind, Who)],
    yawn_sleeps: &[(Who, u16)],
    berry_confusions: &[(Who, RandomOverride, BerryConfusionAt)],
    fatigues: &[(Who, u16)],
    strikes: &[FutureStrikeOracle],
    rolls: &[Option<u16>],
    moody: &[(PokemonId, ResidualKind, RandomOverride)],
) -> Vec<RandomOverride> {
    let mut decisions: Vec<(ResidualScheduleKey, RandomOverride)> = Vec::new();
    // Moody's and Pickup's draws, by their handler's place: the log's order breaks a tie in speed
    // (`residual_tie_tape` planned it), and one holder's draws keep their own order.
    for (rank, (holder, kind, draw)) in moody.iter().enumerate() {
        let (order, sub_order) = rules.residual().key(*kind).unwrap_or((u8::MAX, u8::MAX));
        let speed = pokemon_battle_engine::engine::residual_speed(state, rules, *holder);
        decisions.push((
            (order, std::cmp::Reverse(speed), sub_order, rank, usize::MAX),
            *draw,
        ));
    }
    // Handlers tied in order, speed and sub-order run in the order Showdown's shuffle gave them
    // (the tie decisions planned from the log): the order the log shows their outcomes in.
    let shown_at = |id: PokemonId| {
        cures
            .iter()
            .map(|(who, _)| who)
            .chain(yawn_sleeps.iter().map(|(who, _)| who))
            .position(|who| who_to_id(state, who) == Some(id))
            .unwrap_or(usize::MAX)
    };
    // A berry the residual fed its holder draws (a confusion's duration, Starf Berry's stat) at
    // the `Update` after the weather (Gen 7+) or at the residual's closing one — or, where the
    // berries are residual handlers of their own (Gen 2-3 `ResidualKind::Berry`), at the
    // holder's berry.
    let weather = rules
        .residual()
        .key(ResidualKind::Weather)
        .map_or(0, |(order, _)| order);
    let berry = rules.residual().key(ResidualKind::Berry);
    let active = [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().copied().flatten());
    for (index, id) in active.clone().enumerate() {
        for (who, decision, at) in berry_confusions {
            if who_to_id(state, who) != Some(id) {
                continue;
            }
            let speed = pokemon_battle_engine::engine::residual_speed(state, rules, id);
            let key = match (at, berry) {
                (BerryConfusionAt::AfterWeather, _) => (
                    weather,
                    std::cmp::Reverse(0),
                    u8::MAX,
                    usize::MAX,
                    usize::MAX,
                ),
                (BerryConfusionAt::ResidualEnd, Some((order, sub_order))) => (
                    order,
                    std::cmp::Reverse(speed),
                    sub_order,
                    shown_at(id),
                    index,
                ),
                // At the residual's closing `Update`: after every handler, Moody's too
                // (`residual_end_berry_tape`).
                (BerryConfusionAt::ResidualEnd, None) | (BerryConfusionAt::Entry, _) => continue,
            };
            decisions.push((key, *decision));
        }
    }
    // Where in the residual a Shed Skin or a Healer cured a Pokémon (the log): a status roll for it
    // that comes later never comes — the status is gone (Showdown's Healer asks
    // `allyActive.status`, Shed Skin `pokemon.status`; R-M9-48).
    let actives: Vec<PokemonId> = active.clone().collect();
    let key_of = |id: PokemonId, kind: ResidualKind| -> Option<ResidualScheduleKey> {
        let (order, sub_order) = rules.residual().key(kind)?;
        let index = actives.iter().position(|active| *active == id)?;
        let speed = pokemon_battle_engine::engine::residual_speed(state, rules, id);
        Some((
            order,
            std::cmp::Reverse(speed),
            sub_order,
            shown_at(id),
            index,
        ))
    };
    let healer_of = |id: PokemonId| {
        state.pokemon(id).is_some_and(|pokemon| {
            pokemon_battle_engine::engine::ability_handlers(state, rules, pokemon)
                .residual
                .is_some_and(|participant| participant.kind == ResidualKind::Healer)
        })
    };
    let cured_at = |target: PokemonId| -> Option<ResidualScheduleKey> {
        cures
            .iter()
            .filter(|(who, _)| who_to_id(state, who) == Some(target))
            .filter_map(|(_, kind)| match kind {
                ResidualKind::ShedSkin => key_of(target, ResidualKind::ShedSkin),
                ResidualKind::Healer => {
                    let position = state.active_position(target, rules.topology())?;
                    rules
                        .topology()
                        .adjacent_allies(position)
                        .into_iter()
                        .filter_map(|p| state.pokemon_at_position(p, rules.topology()))
                        .filter(|ally| healer_of(*ally))
                        .filter_map(|ally| key_of(ally, ResidualKind::Healer))
                        .min()
                }
                _ => None,
            })
            .min()
    };
    let cured_before = |target: PokemonId, key: Option<ResidualScheduleKey>| matches!((cured_at(target), key), (Some(at), Some(key)) if at < key);
    // Where in the residual a Pokémon fell (the log): a roll of a handler of its own, or one
    // aimed at it, that comes later never comes — Showdown's `fieldEvent` skips a fainted
    // holder's handlers, and Healer's `adjacentAllies()` leaves a fainted ally out (R-M9-51).
    let fell_before = |target: PokemonId, key: Option<ResidualScheduleKey>| {
        let fell_at = faints
            .iter()
            .filter(|(who, _, _)| who_to_id(state, who) == Some(target))
            .filter_map(|(_, kind, holder)| key_of(who_to_id(state, holder)?, *kind))
            .min();
        matches!((fell_at, key), (Some(at), Some(key)) if at < key)
    };
    for (index, id) in active.enumerate() {
        let Some(pokemon) = state.pokemon(id) else {
            continue;
        };
        if pokemon.is_fainted() {
            continue;
        }
        let speed = pokemon_battle_engine::engine::residual_speed(state, rules, id);
        let cured = |kind: ResidualKind| {
            cures
                .iter()
                .any(|(who, k)| *k == kind && who_to_id(state, who) == Some(id))
        };
        let mut push = |kind: ResidualKind, request: RandomRequestKind| {
            if let Some((order, sub_order)) = rules.residual().key(kind) {
                decisions.push((
                    (
                        order,
                        std::cmp::Reverse(speed),
                        sub_order,
                        shown_at(id),
                        index,
                    ),
                    RandomOverride::bool(request, cured(kind)),
                ));
            }
        };
        let sheds = pokemon_battle_engine::engine::ability_handlers(state, rules, pokemon)
            .residual
            .is_some_and(|participant| participant.kind == ResidualKind::ShedSkin);
        if sheds
            && pokemon.status.is_some()
            && !cured_before(id, key_of(id, ResidualKind::ShedSkin))
            && !fell_before(id, key_of(id, ResidualKind::ShedSkin))
        {
            push(ResidualKind::ShedSkin, RandomRequestKind::ShedSkin);
        }
        if pokemon.has_status(NonVolatileStatus::Freeze)
            && !fell_before(id, key_of(id, ResidualKind::FreezeThaw))
        {
            push(ResidualKind::FreezeThaw, RandomRequestKind::Thaw);
        }
        // Harvest rolls at every turn's end out of the sun, with a berry to bring back or not:
        // it succeeded when the berry came back. Optional: in the sun it rolls nothing, and a
        // sun that runs out in this residual leaves it rolling after all.
        let harvests = pokemon_battle_engine::engine::ability_handlers(state, rules, pokemon)
            .residual
            .is_some_and(|participant| participant.kind == ResidualKind::Harvest);
        if harvests
            && !fell_before(id, key_of(id, ResidualKind::Harvest))
            && let Some((order, sub_order)) = rules.residual().key(ResidualKind::Harvest)
        {
            decisions.push((
                (
                    order,
                    std::cmp::Reverse(speed),
                    sub_order,
                    shown_at(id),
                    index,
                ),
                RandomOverride::bool(RandomRequestKind::Harvest, cured(ResidualKind::Harvest))
                    .optional(),
            ));
        }
        // Healer: one roll per adjacent ally with a status, in adjacency order.
        let healer = pokemon_battle_engine::engine::ability_handlers(state, rules, pokemon)
            .residual
            .is_some_and(|participant| participant.kind == ResidualKind::Healer);
        if healer
            && !fell_before(id, key_of(id, ResidualKind::Healer))
            && let Some((order, sub_order)) = rules.residual().key(ResidualKind::Healer)
            && let Some(position) = state.active_position(id, rules.topology())
        {
            for ally in rules
                .topology()
                .adjacent_allies(position)
                .into_iter()
                .filter_map(|p| state.pokemon_at_position(p, rules.topology()))
                .filter_map(|ally| state.pokemon(ally).map(|p| (ally, p)))
                .filter(|(_, p)| !p.is_fainted() && p.status.is_some())
                .map(|(ally, _)| ally)
                .filter(|ally| !cured_before(*ally, key_of(id, ResidualKind::Healer)))
                .filter(|ally| !fell_before(*ally, key_of(id, ResidualKind::Healer)))
            {
                let cured = cures.iter().any(|(who, k)| {
                    *k == ResidualKind::Healer && who_to_id(state, who) == Some(ally)
                });
                decisions.push((
                    (
                        order,
                        std::cmp::Reverse(speed),
                        sub_order,
                        shown_at(id),
                        index,
                    ),
                    RandomOverride::bool(RandomRequestKind::Healer, cured),
                ));
            }
        }
        // A Yawn that ran out and put the holder to sleep draws the sleep's duration.
        if pokemon.volatiles.contains(VolatileConditionKind::Yawn)
            && let Some((_, time)) = yawn_sleeps
                .iter()
                .find(|(who, _)| who_to_id(state, who) == Some(id))
            && let Some((order, sub_order)) = rules.residual().key(ResidualKind::Yawn)
        {
            decisions.push((
                (
                    order,
                    std::cmp::Reverse(speed),
                    sub_order,
                    shown_at(id),
                    index,
                ),
                RandomOverride::range(RandomRequestKind::Duration, *time),
            ));
        }
    }
    // A lock (Thrash and its kind, §15.12) that ended in the residual with its fatigue draws the
    // confusion's duration at the lock's place: Gen 3-4 the cartridge's (`ResidualKind::Rampage`),
    // Gen 5+ a duration with no order, at the holder's speed.
    for (index, id) in [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().copied().flatten())
        .enumerate()
    {
        let Some((_, time)) = fatigues
            .iter()
            .find(|(who, _)| who_to_id(state, who) == Some(id))
        else {
            continue;
        };
        let speed = pokemon_battle_engine::engine::residual_speed(state, rules, id);
        let (order, sub_order) = rules.residual().key(ResidualKind::Rampage).unwrap_or((
            u8::MAX,
            pokemon_battle_engine::rules::UNORDERED_VOLATILE_SUB_ORDER,
        ));
        // Two locks ending in one residual at a speed tie: in the order the log shows their
        // fatigue (R-M8-79).
        decisions.push((
            (
                order,
                std::cmp::Reverse(speed),
                sub_order,
                shown_at(id),
                index,
            ),
            RandomOverride::range(RandomRequestKind::Duration, *time),
        ));
    }
    // The foreseen attacks' strikes (§15.22), at their place in the residual and the target's
    // speed: the hit steps' accuracy, then (Gen 5+) the critical hit and the damage roll, or
    // (Gen 2-3 on the cartridge) the random factor of the damage computed at use.
    if let Some((order, sub_order)) = rules.residual().key(ResidualKind::FutureAttack) {
        for (index, strike) in strikes.iter().enumerate() {
            let Some(target) = who_to_id(state, &strike.target) else {
                continue;
            };
            let Some(position) = state.active_position(target, rules.topology()) else {
                continue;
            };
            let Some(spec) = rules.topology().spec(position) else {
                continue;
            };
            let Some(attack) = state
                .side(spec.side)
                .future_attacks
                .iter()
                .find(|attack| attack.slot == usize::from(spec.slot))
            else {
                continue;
            };
            let Some(strike_def) = pokemon_battle_engine::engine::future_strike(rules, dex, attack)
            else {
                continue;
            };
            let source = attack.source;
            // Gen 3 on the cartridge (issues.md I111): by battler, not by speed.
            let speed = if rules.conditions().future_attack.battler_order {
                u32::MAX - (u32::from(spec.slot) * 2 + u32::from(spec.side == SideId::P2))
            } else {
                pokemon_battle_engine::engine::residual_speed(state, rules, target)
            };
            let key = (
                order,
                std::cmp::Reverse(speed),
                sub_order,
                usize::MAX,
                index,
            );
            let roll = rolls
                .get(index)
                .copied()
                .flatten()
                .unwrap_or(rules.damage().random_max);
            // Gen 2 on the cartridge: the random factor first, hit, miss or dodge (none for 1).
            if rules.conditions().future_attack.damage_at_use
                == Some(pokemon_battle_engine::rules::ForeseenRoll::BeforeAccuracy)
                && attack.damage.is_some_and(|damage| damage > 1)
            {
                decisions.push((
                    key,
                    RandomOverride::range(RandomRequestKind::DamageRoll, roll),
                ));
            }
            if strike.immune
                || pokemon_battle_engine::engine::two_turn::dodged_by(
                    state,
                    rules,
                    source,
                    target,
                    &strike_def,
                )
                .is_some()
            {
                continue;
            }
            // Optional when it hit: the check is the strike's own, on the field the earlier
            // strikes left (a user they knocked out has lost its Lock-On, DESIGN.md R277).
            let accuracy = RandomOverride::bool(RandomRequestKind::Accuracy, !strike.missed);
            decisions.push((
                key,
                if strike.missed {
                    accuracy
                } else {
                    accuracy.optional()
                },
            ));
            if strike.missed {
                continue;
            }
            if strike_def.fixed_damage.is_none()
                && matches!(
                    pokemon_battle_engine::engine::critical_roll(
                        state,
                        rules,
                        dex,
                        source,
                        target,
                        &strike_def
                    ),
                    pokemon_battle_engine::engine::CriticalRoll::Chance(..)
                )
            {
                decisions.push((
                    key,
                    RandomOverride::bool(RandomRequestKind::Critical, strike.critical),
                ));
            }
            match strike_def.fixed_damage {
                None => decisions.push((
                    key,
                    RandomOverride::range(RandomRequestKind::DamageRoll, roll),
                )),
                Some(pokemon_battle_engine::dex::FixedDamage::Foreseen(_)) => decisions.push((
                    key,
                    RandomOverride::range(RandomRequestKind::DamageRoll, roll).optional(),
                )),
                Some(_) => {}
            }
            // The target's Focus Band, as any hit's (DESIGN.md R277): Gen 3-4 on every hit,
            // later only on one that would knock it out — optional, as the damage search tries
            // rolls that would or would not.
            if pokemon_battle_engine::engine::held_item_id(state, rules, target)
                == Some(pokemon_battle_engine::ids::ItemId::FOCUS_BAND)
            {
                let decision =
                    RandomOverride::bool(RandomRequestKind::FocusBand, strike.focus_band_held);
                decisions.push((
                    key,
                    if matches!(rules.generation(), Generation::Gen3 | Generation::Gen4) {
                        decision
                    } else {
                        decision.optional()
                    },
                ));
            }
            // A Starf Berry the strike's `Update` fed the target: its stat.
            if let Some(draw @ BerryDraw::Stat(_)) = strike.berry_draw
                && let Some(decision) = berry_decision(draw, || None)
            {
                decisions.push((key, decision));
            }
        }
    }
    decisions.sort_by_key(|(key, _)| *key);
    decisions
        .into_iter()
        .map(|(_, decision)| decision)
        .collect()
}

/// A replacement answer in Showdown choice syntax (`switch N`); a side that was not asked
/// sends nothing.
/// The slot of the Pokémon whose Revival Blessing waits for an answer (its side's
/// `reviving`), in its side's slot order.
fn reviving_slot(compiled: &Compiled, user: PokemonId) -> Option<usize> {
    let position = compiled
        .engine
        .state()
        .active_position(user, compiled.rules.topology())?;
    compiled
        .rules
        .topology()
        .spec(position)
        .map(|spec| usize::from(spec.slot))
}

/// The Pokémon a side's entry names for its Revival Blessing (`switch N`): the part for the
/// reviving user's slot — Showdown's request gives one part per slot, `pass` for one it does
/// not ask (R-M8-171) — else the first `switch`.
fn revive_answer(compiled: &Compiled, side: SideId, text: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = text?.split(',').map(str::trim).collect();
    let slot = compiled
        .engine
        .state()
        .side(side)
        .reviving
        .and_then(|user| reviving_slot(compiled, user));
    slot.and_then(|slot| parts.get(slot))
        .and_then(|part| part.strip_prefix("switch "))
        .or_else(|| parts.iter().find_map(|part| part.strip_prefix("switch ")))
        .map(|named| named.trim().to_string())
}

/// Answers a side's Revival Blessing with the Pokémon `named` (`switch N`'s target): its party
/// index names it, no swap follows. A revived Pokémon that goes back into its slot at once (Gen
/// 8+'s re-sort) runs its switch-in there: a Trace holder draws its foe (probed: who came in;
/// DESIGN.md R263).
fn answer_revive(
    compiled: &mut Compiled,
    side: SideId,
    named: &str,
    oracle_frame: &Value,
) -> Result<pokemon_battle_engine::engine::TurnResult, pokemon_battle_engine::engine::BattleError> {
    let n = switch_index(compiled, side, named.trim());
    let pokemon = compiled.party[side.index()][n - 1];
    if traces(oracle_frame) {
        flush_trace(compiled);
        let snapshot = compiled.engine.snapshot();
        let trace_len = compiled.engine.trace().events().len();
        let mut probe = restore_with_tape(compiled, &snapshot, Vec::new());
        if probe.resolve_revive(ReviveChoice { side, pokemon }).is_ok() {
            let came_in = entrants(&probe.trace().events()[trace_len..]);
            let tape = trace_tape(
                compiled,
                compiled.engine.state(),
                probe.state(),
                &came_in,
                oracle_frame,
            );
            if !tape.is_empty() {
                compiled.engine = restore_with_tape(compiled, &snapshot, tape);
            }
        }
    }
    compiled
        .engine
        .resolve_revive(ReviveChoice { side, pokemon })
}

fn parse_replacement(
    compiled: &mut Compiled,
    requirements: &[ReplacementRequirement],
    side: SideId,
    text: Option<&str>,
) -> ForcedSwitchChoice {
    // A Pokémon ordered out mid-turn (an Eject Pack's) whose own switch is still queued: the
    // replacement's `switchIn` cancels that switch (`queue.cancelAction(oldActive)`), so its swap
    // never happens. Undo it before `switch N` counts in Showdown's party (DESIGN.md R-M9-23).
    if let Some(requirement) = requirements.iter().find(|r| r.side == side) {
        for slot in &requirement.positions {
            let slot_index = compiled
                .rules
                .topology()
                .spec(slot.position)
                .map_or(0, |spec| usize::from(spec.slot));
            let leaving_alive = compiled
                .engine
                .state()
                .side(side)
                .active
                .get(slot_index)
                .copied()
                .flatten()
                .and_then(|id| compiled.engine.state().pokemon(id))
                .is_some_and(|pokemon| !pokemon.is_fainted());
            if !leaving_alive {
                continue;
            }
            while let Some(at) = compiled
                .pending_swaps
                .iter()
                .rposition(|&(s, a, _, _)| s == side.index() && a == slot_index)
            {
                let (_, a, b, _) = compiled.pending_swaps.remove(at);
                compiled.party[side.index()].swap(a, b);
            }
        }
    }
    // `switch N[, switch M]`: one entry per slot that has to be filled, in slot order; `pass`
    // entries (Showdown's placeholder for slots that are not asked) are skipped.
    let switches: Vec<usize> = text
        .map(|t| {
            t.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty() && *part != "pass")
                .map(|part| {
                    let target = part
                        .strip_prefix("switch ")
                        .unwrap_or_else(|| panic!("unsupported replacement syntax: {part}"));
                    switch_index(compiled, side, target.trim())
                })
                .collect()
        })
        .unwrap_or_default();
    let mut replacements = Vec::new();
    if !switches.is_empty() {
        let requirement = requirements
            .iter()
            .find(|requirement| requirement.side == side)
            .expect("replacement requirement for the answering side");
        for (slot, n) in requirement.positions.iter().zip(switches) {
            let position = slot.position;
            let slot_index = compiled
                .rules
                .topology()
                .spec(position)
                .map_or(0, |spec| usize::from(spec.slot));
            replacements.push(ForcedReplacement {
                position,
                pokemon: take_switch_target(compiled, side, slot_index, n),
            });
        }
    }
    ForcedSwitchChoice { side, replacements }
}

// ---------------------------------------------------------------------------------------------
// Failure bundle (TESTING.md §9)
// ---------------------------------------------------------------------------------------------

pub const FAILURE_BUNDLE_FILES: [&str; 8] = [
    "scenario.json",
    "rust.frames.json",
    "oracle.frames.json",
    "diff.json",
    "rust.trace.txt",
    "showdown.log.txt",
    "provenance.json",
    "reproduce.txt",
];

/// Writes the reproducible artefact for a diverging scenario. A failure must be inspectable
/// and re-runnable without the original test run.
pub fn write_failure_bundle(
    dir: &std::path::Path,
    scenario_path: &std::path::Path,
    oracle: &Value,
    report: &ReplayReport,
    reproduce: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let pretty = |value: &Value| serde_json::to_string_pretty(value).expect("json") + "\n";
    std::fs::copy(scenario_path, dir.join("scenario.json"))?;
    std::fs::write(
        dir.join("rust.frames.json"),
        pretty(&Value::Array(report.rust_frames.clone())),
    )?;
    std::fs::write(dir.join("oracle.frames.json"), pretty(&oracle["frames"]))?;
    std::fs::write(
        dir.join("diff.json"),
        pretty(&serde_json::json!({
            "first_divergence": report.divergences.first(),
            "divergences": report.divergences,
            "frames_compared": report.frames_compared,
            "actions_replayed": report.actions_replayed,
        })),
    )?;
    std::fs::write(dir.join("rust.trace.txt"), report.trace.join("\n") + "\n")?;
    std::fs::write(
        dir.join("showdown.log.txt"),
        oracle["debug_log"].as_str().unwrap_or(""),
    )?;
    std::fs::write(
        dir.join("provenance.json"),
        pretty(&serde_json::json!({"oracle": oracle["provenance"], "rust": report.provenance})),
    )?;
    std::fs::write(dir.join("reproduce.txt"), format!("{reproduce}\n"))?;
    Ok(())
}

/// The value a move drew as it started, as the log shows it: Magnitude's announced magnitude;
/// Present's heal (0) when its target was healed or refused the heal (`-fail|target|heal`, Gen
/// 2's bare `-fail|target`; a Heal Block's refusal fails the move, `-fail|user`: no damage band
/// fails so) or, behind a Substitute before Gen 6, when nothing struck the doll; else the first
/// 40-power value — its 80 and 120 bands (`present_bands`) are tried when that one does not
/// fit.
fn drawn_value(
    random: pokemon_battle_engine::effects::moves::RandomPower,
    oracle: &ActionOracle,
    behind_substitute: bool,
) -> u16 {
    use pokemon_battle_engine::effects::moves::RandomPower;
    match random {
        RandomPower::Magnitude => oracle.magnitude.unwrap_or(random.table().0[0]),
        RandomPower::Present => {
            // Before Gen 6 a heal fails against a Substitute (Gen 3-5 `-fail|user`, Gen 2 with
            // no word): a doll nothing struck means the heal was drawn.
            let healed = oracle
                .applied
                .iter()
                .any(|(_, kind)| matches!(kind.as_str(), "heal" | "fail:heal" | "fail"))
                || oracle.failed
                || (behind_substitute
                    && oracle.substitute_struck.is_empty()
                    && oracle.missed.is_empty());
            if healed { 0 } else { 2 }
        }
    }
}

/// Whether the move's first target is behind a Substitute as the action starts.
fn behind_substitute(state: &BattleState, fates: &[(Option<PokemonId>, TargetFate)]) -> bool {
    fates
        .first()
        .and_then(|(target, _)| *target)
        .is_some_and(|target| substitute_hp(state, target).is_some())
}

/// Present's other power bands (80, 120), for a plan whose 40-power draw fits no damage roll:
/// the template with its draw replaced. The draw is the last `Sample` before the hit's damage
/// roll: a move that called Present (Sleep Talk, Metronome) drew its pick before it, which may be
/// a 2 as well (R-M8-166), and what the hit sets off draws after it — a Starf Berry's stat
/// (R-M8-173).
fn present_bands(
    template: &[Option<RandomOverride>],
    move_def: Option<&ResolvedMove>,
) -> Vec<Vec<Option<RandomOverride>>> {
    let present = move_def.is_some_and(|def| {
        def.behavior.random_power
            == Some(pokemon_battle_engine::effects::moves::RandomPower::Present)
    });
    let Some(damage_roll) = template
        .iter()
        .position(|entry| entry.is_none() || is_open_optional_roll(entry))
    else {
        return Vec::new();
    };
    let Some(index) = template[..damage_roll].iter().rposition(|entry| {
        entry.is_some_and(|decision| decision.kind == RandomRequestKind::Sample)
    }) else {
        return Vec::new();
    };
    if !present
        || template[index].is_none_or(|decision| {
            decision.value != pokemon_battle_engine::state::RandomOverrideValue::Range(2)
        })
    {
        return Vec::new();
    }
    [6, 9]
        .into_iter()
        .map(|value| {
            let mut variant = template.to_vec();
            variant[index] = Some(RandomOverride::range(RandomRequestKind::Sample, value));
            variant
        })
        .collect()
}

/// The Psywave roll that deals what Showdown's log shows: the HP the target lost (nothing: a
/// Gen 1 roll of 0), read back through the generation's formula; a knock-out takes the smallest
/// roll that reaches it.
fn psywave_roll(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    target: PokemonId,
    dealt: u16,
    after: Option<u16>,
    substitutes: &[(PokemonId, Option<u16>)],
) -> u16 {
    let level = state.pokemon(actor).map_or(1, |p| p.level);
    let (min, max) = pokemon_battle_engine::engine::psywave_range(rules, level);
    let damage = |roll: u16| pokemon_battle_engine::engine::psywave_damage(rules, level, roll);
    // A hit on the target's Substitute commits no HP: the boundary frame's Substitute settles
    // it — broken, the smallest roll that breaks it; left standing, the roll it lost.
    if let Some(doll) = substitute_hp(state, target)
        && let Some((_, after)) = substitutes.iter().find(|(id, _)| *id == target)
    {
        return (min..=max)
            .find(|roll| match after {
                None => damage(*roll) >= doll,
                Some(left) => damage(*roll) == doll.saturating_sub(*left),
            })
            .unwrap_or(max);
    }
    // Each hit's own roll, from that hit's HP (a Parental Bond second strike rolls again).
    let Some(after) = after else {
        return min;
    };
    let before = state.pokemon(target).map_or(0, |p| p.hp);
    (min..=max)
        .find(|roll| damage(*roll) == dealt || (after == 0 && damage(*roll) >= before))
        .unwrap_or(max)
}

// ---------------------------------------------------------------------------------------------
// Random stat picks (R166): Acupressure's and Moody's `sample`, read from the stats they named
// ---------------------------------------------------------------------------------------------

/// The stat a `boost:` / `unboost:` label names.
fn labelled_stat(label: &str) -> Option<pokemon_battle_engine::state::Stat> {
    pokemon_battle_engine::engine::STAGED_STATS
        .into_iter()
        .find(|stat| stat_changed(label, *stat))
}

/// Acupressure's `sample` over the target's stats below +6 (drawn only with a choice): the
/// index of the stat its boost named.
fn acupressure_tape(
    state: &BattleState,
    victim: PokemonId,
    oracle: &ActionOracle,
) -> Option<Option<RandomOverride>> {
    let pokemon = state.pokemon(victim)?;
    let candidates: Vec<_> = pokemon_battle_engine::engine::STAGED_STATS
        .into_iter()
        .filter(|stat| pokemon.stages.get(*stat) < 6)
        .collect();
    if candidates.len() < 2 {
        return None;
    }
    let picked = oracle.applied.iter().find_map(|(who, kind)| {
        (who_to_id(state, who) == Some(victim))
            .then(|| labelled_stat(kind))
            .flatten()
    })?;
    let index = candidates.iter().position(|stat| *stat == picked)?;
    Some(Some(RandomOverride::range(
        RandomRequestKind::Sample,
        index as u16,
    )))
}

/// Conversion's (Gen 2-5) or Conversion 2's `sample` over `engine::conversion_candidates` /
/// `conversion2_candidates` (drawn only with a choice): the place of the type the user's
/// `-start|user|typechange|T` names — the first slot with it, which draws the same type.
fn conversion_tape(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    user: PokemonId,
    target: Option<PokemonId>,
    move_id: MoveId,
    oracle: &ActionOracle,
) -> Option<Option<RandomOverride>> {
    let candidates = if move_id == MoveId::CONVERSION {
        pokemon_battle_engine::engine::conversion_candidates(state, rules, dex, user)
    } else {
        pokemon_battle_engine::engine::conversion2_candidates(state, rules, dex, user, target?)?
    };
    if candidates.len() < 2 {
        return None;
    }
    let shown = oracle
        .typechanges
        .iter()
        .find(|(who, _)| who_to_id(state, who) == Some(user))
        .and_then(|(_, name)| pokemon_battle_engine::dex::PokemonType::from_name(name));
    let index = shown
        .and_then(|type_| candidates.iter().position(|c| *c == type_))
        .unwrap_or(0);
    Some(Some(RandomOverride::range(
        RandomRequestKind::Sample,
        index as u16,
    )))
}

/// Moody and Pickup in the end-of-turn residual (both order 28.2, after every other residual
/// that draws), in the log's order. Moody: each holder that showed `-ability|p|Moody|boost`,
/// fastest first, draws the stat it raised among those below +6, then the one it lowered among
/// those above -6 but that one — read from the two stat changes that follow, in order
/// (Contrary turns them around, the stats stay). Pickup: a holder that showed `-item|p|X|[from]
/// ability: Pickup` drew, among the adjacent Pokémon that used an item up this turn (in
/// `getAllActive` order), the first whose last item is X — when there were several; the one it
/// took no longer counts for a later Pickup.
/// Moody's and Pickup's draws in the log's order, each with its holder and its residual handler:
/// `residual_tape` puts them among the other residual decisions by the handlers' order and
/// speed (a faster Moody holder draws before a slower Harvest holder's roll, DESIGN.md R-M8-92).
fn moody_tape(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &pokemon_battle_engine::dex::ResolvedDex,
    oracle_frame: &Value,
) -> Vec<(PokemonId, ResidualKind, RandomOverride)> {
    let events = oracle_frame["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // Each holder's stat changes in the log's order, with whether each was a rise (`boost`);
    // a Pickup holder's with the item it picked up instead. Moody's candidate lists are built
    // from the holder's stages **as its handler runs** (§20.2), so the walk below keeps the
    // residual's own `boost`/`unboost` events applied on top of the frame's state.
    type MoodyPick = (
        PokemonId,
        Vec<(bool, pokemon_battle_engine::state::Stat)>,
        pokemon_battle_engine::state::StatStages,
    );
    let mut live: Vec<(PokemonId, pokemon_battle_engine::state::StatStages)> = state
        .all_pokemon()
        .iter()
        .map(|pokemon| (pokemon.id(), pokemon.stages))
        .collect();
    let mut picks: Vec<MoodyPick> = Vec::new();
    let mut pickups: Vec<(usize, PokemonId, String)> = Vec::new();
    let mut in_residual = false;
    let mut reading: Option<usize> = None;
    for event in &events {
        if event["kind"] == "residual" {
            in_residual = true;
            continue;
        }
        if !in_residual {
            continue;
        }
        // The residual's own stat changes, in the order they happened, for every Pokémon (not
        // only the one being read): Moody's filters see them.
        let kind = event["kind"].as_str().unwrap_or("");
        if (kind == "boost" || kind == "unboost")
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
            && let Some(id) = who_to_id(state, &who)
            && let Some(stat) =
                labelled_stat(&format!("{kind}:{}", event["stat"].as_str().unwrap_or("")))
            && let Some(amount) = event["amount"].as_i64()
            && let Some(entry) = live.iter_mut().find(|(who, _)| *who == id)
        {
            let delta = amount as i8 * if kind == "boost" { 1 } else { -1 };
            entry.1.set(stat, (entry.1.get(stat) + delta).clamp(-6, 6));
        }
        if event["kind"] == "item"
            && let Some(args) = event["args"].as_array()
            && args.get(2).and_then(Value::as_str) == Some("[from] ability: Pickup")
            && let (Some(id), Some(item)) = (
                args.first()
                    .and_then(who_from_text)
                    .and_then(|who| who_to_id(state, &who)),
                args.get(1).and_then(Value::as_str),
            )
        {
            pickups.push((picks.len(), id, item.to_string()));
            reading = None;
            continue;
        }
        if event["kind"] == "ability"
            && let Some(args) = event["args"].as_array()
            && args.get(1).and_then(Value::as_str) == Some("Moody")
            && let Some(id) = args
                .first()
                .and_then(who_from_text)
                .and_then(|who| who_to_id(state, &who))
        {
            let stages = live
                .iter()
                .find(|(who, _)| *who == id)
                .map(|(_, stages)| *stages)
                .unwrap_or_default();
            picks.push((id, Vec::new(), stages));
            reading = Some(picks.len() - 1);
            continue;
        }
        let Some(current) = reading else {
            continue;
        };
        let kind = event["kind"].as_str().unwrap_or("");
        if (kind == "boost" || kind == "unboost")
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
            && who_to_id(state, &who) == Some(picks[current].0)
            && let Some(stat) =
                labelled_stat(&format!("{kind}:{}", event["stat"].as_str().unwrap_or("")))
        {
            picks[current].1.push((kind == "boost", stat));
        } else {
            reading = None;
        }
    }
    // Pickup's draws, each placed before the Moody pick that followed it in the log.
    let mut taken: Vec<PokemonId> = Vec::new();
    let mut pickup_draws: Vec<(usize, PokemonId, RandomOverride)> = Vec::new();
    for (before_pick, holder, item) in pickups {
        let Some(position) = state.active_position(holder, rules.topology()) else {
            continue;
        };
        let topology = rules.topology();
        let adjacent: Vec<PokemonId> = topology
            .adjacent_allies(position)
            .into_iter()
            .chain(topology.adjacent_foes(position))
            .filter_map(|position| state.pokemon_at_position(position, topology))
            .collect();
        let candidates: Vec<PokemonId> = [SideId::P1, SideId::P2]
            .into_iter()
            .flat_map(|side| state.side(side).active.iter().copied().flatten())
            .filter(|id| {
                adjacent.contains(id)
                    && !taken.contains(id)
                    && state.pokemon(*id).is_some_and(|p| {
                        !p.is_fainted()
                            && p.history.last_item.is_some()
                            && p.history.used_item_this_turn
                    })
            })
            .collect();
        let Some(index) = candidates.iter().position(|id| {
            state
                .pokemon(*id)
                .and_then(|p| p.history.last_item)
                .and_then(|last| dex.item_by_id(last))
                .is_some_and(|last| last.name == item)
        }) else {
            continue;
        };
        taken.push(candidates[index]);
        if candidates.len() > 1 {
            pickup_draws.push((
                before_pick,
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16),
            ));
        }
    }
    let accuracy_evasion = rules.conditions().moody_draws_accuracy_evasion;
    let stats = pokemon_battle_engine::effects::abilities::moody_stats(accuracy_evasion);
    // The picks in the log's order: the order the holders' residuals ran in (by their speed at
    // the time, ties already planned by `residual_tie_tape`).
    let mut tape = Vec::new();
    let pick_count = picks.len();
    for (pick, (id, named, stages)) in picks.into_iter().enumerate() {
        tape.extend(
            pickup_draws
                .iter()
                .filter(|(before_pick, _, _)| *before_pick == pick)
                .map(|(_, holder, draw)| (*holder, ResidualKind::Pickup, *draw)),
        );
        let Some(pokemon) = state.pokemon(id) else {
            continue;
        };
        let _ = stats;
        let raisable = pokemon_battle_engine::effects::abilities::raisable(
            |stat| stages.get(stat),
            accuracy_evasion,
        );
        // The rise and the drop, told apart by their direction (Contrary turns both around):
        // with nothing left to raise, the only change shown is the drop.
        let contrary = pokemon_battle_engine::engine::ability_active(state, rules, pokemon)
            == Some(pokemon_battle_engine::ids::AbilityId::CONTRARY);
        let raised = named
            .iter()
            .find(|(up, _)| *up != contrary)
            .map(|(_, stat)| *stat);
        let lowered = named
            .iter()
            .find(|(up, _)| *up == contrary)
            .map(|(_, stat)| *stat);
        if raisable.len() > 1
            && let Some(index) = raised.and_then(|r| raisable.iter().position(|s| *s == r))
        {
            tape.push((
                id,
                ResidualKind::Moody,
                RandomOverride::range(RandomRequestKind::Sample, index as u16),
            ));
        }
        let lowerable = pokemon_battle_engine::effects::abilities::lowerable(
            |stat| stages.get(stat),
            accuracy_evasion,
            raised,
        );
        if lowerable.len() > 1
            && let Some(index) = lowered.and_then(|l| lowerable.iter().position(|s| *s == l))
        {
            tape.push((
                id,
                ResidualKind::Moody,
                RandomOverride::range(RandomRequestKind::Sample, index as u16),
            ));
        }
    }
    // The Pickups after the last Moody pick.
    tape.extend(
        pickup_draws
            .iter()
            .filter(|(before_pick, _, _)| *before_pick == pick_count)
            .map(|(_, holder, draw)| (*holder, ResidualKind::Pickup, *draw)),
    );
    tape
}
