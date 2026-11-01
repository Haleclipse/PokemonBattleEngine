//! Canonical per-generation dex snapshots.
//!
//! `data/dex/gen{1..9}.json` are exported from `@pkmn/data` (the Showdown Dex) by
//! `tools/oracle-node/src/export-dex.mjs` and committed to the repository. They are the single
//! source of static data: `ResolvedDex` and `TypeChart` are built from them, and every
//! `MoveId` / `AbilityId` / `ItemId` / `SpeciesId` comes from `data/dex/ids.json` (rules in
//! data/dex/README.md; assigned once and never renumbered). Regenerating snapshots is an
//! explicit, reviewable change tied to a provenance bump (TESTING.md §8).
//!
//! The snapshots are embedded at compile time so the engine has no runtime file dependency;
//! each generation is parsed lazily, once per process, on first use.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

use crate::spec::Generation;

const RAW: [&str; 9] = [
    include_str!("../../data/dex/gen1.json"),
    include_str!("../../data/dex/gen2.json"),
    include_str!("../../data/dex/gen3.json"),
    include_str!("../../data/dex/gen4.json"),
    include_str!("../../data/dex/gen5.json"),
    include_str!("../../data/dex/gen6.json"),
    include_str!("../../data/dex/gen7.json"),
    include_str!("../../data/dex/gen8.json"),
    include_str!("../../data/dex/gen9.json"),
];

fn index(generation: Generation) -> usize {
    usize::from(generation.number() - 1)
}

/// Raw snapshot text for a generation; used for fingerprinting.
pub fn raw(generation: Generation) -> &'static str {
    RAW[index(generation)]
}

/// The parsed snapshot for a generation.
pub fn snapshot(generation: Generation) -> &'static DexSnapshot {
    static PARSED: [OnceLock<DexSnapshot>; 9] = [const { OnceLock::new() }; 9];
    PARSED[index(generation)].get_or_init(|| {
        serde_json::from_str(RAW[index(generation)]).unwrap_or_else(|error| {
            panic!(
                "data/dex/gen{}.json is not a valid dex snapshot: {error}",
                generation.number()
            )
        })
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct DexSnapshot {
    pub schema: u32,
    pub provenance: SnapshotProvenance,
    pub generation: u8,
    pub types: TypeSnapshot,
    pub moves: Vec<MoveSnapshot>,
    pub species: Vec<SpeciesSnapshot>,
    pub abilities: Vec<AbilitySnapshot>,
    pub items: Vec<ItemSnapshot>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotProvenance {
    pub engine: String,
    pub version: String,
    #[serde(default)]
    pub sim: Option<String>,
}

/// Attacking type → defending type → multiplier. Only non-neutral entries are present.
#[derive(Debug, Clone, Deserialize)]
pub struct TypeSnapshot {
    pub names: Vec<String>,
    pub chart: BTreeMap<String, BTreeMap<String, f64>>,
}

/// A move as Showdown declares it. Fields the engine consumes are typed; every other
/// declarative key lands in `other` so the resolver can classify the move as unsupported instead
/// of silently ignoring a mechanic.
#[derive(Debug, Clone, Deserialize)]
pub struct MoveSnapshot {
    pub id: String,
    /// Engine id from `data/dex/ids.json` (Showdown `num` unless that collided).
    pub eid: u16,
    pub num: i32,
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub category: String,
    pub bp: u16,
    #[serde(default)]
    pub acc: Option<u8>,
    pub pp: u8,
    pub pri: i8,
    pub target: String,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub secondaries: Vec<EffectSnapshot>,
    #[serde(default, rename = "self")]
    pub self_effect: Option<EffectSnapshot>,
    #[serde(default)]
    pub callbacks: Vec<String>,
    /// Gen 7: the upstream `zMove` the move's data declares (DESIGN.md §44.1).
    #[serde(default, rename = "zMove")]
    pub z_move: Option<ZMoveSnapshot>,
    /// Gen 8: the upstream `maxMove` the move's data declares (DESIGN.md §44.1).
    #[serde(default, rename = "maxMove")]
    pub max_move: Option<MaxMoveSnapshot>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// A move's own `zMove` entry: its Z-Move's power, or what its status Z version adds (stat
/// stages or a named effect).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ZMoveSnapshot {
    #[serde(default, rename = "basePower")]
    pub base_power: Option<u16>,
    #[serde(default)]
    pub boost: Option<BTreeMap<String, i8>>,
    #[serde(default)]
    pub effect: Option<String>,
}

/// A move's own `maxMove` entry: its Max Move's power.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct MaxMoveSnapshot {
    #[serde(rename = "basePower")]
    pub base_power: u16,
}

impl MoveSnapshot {
    /// The callbacks a behavior component may cover: the move's own, each secondary's as
    /// `secondary:<name>` and its `self` table's as `self:<name>`.
    pub fn declared_callbacks(&self) -> Vec<String> {
        self.callbacks
            .iter()
            .cloned()
            .chain(self.secondaries.iter().flat_map(|secondary| {
                secondary
                    .callbacks
                    .iter()
                    .map(|callback| format!("secondary:{callback}"))
                    // A secondary's own `self` (Genesis Supernova's `self.onHit`): the same
                    // `self:<name>` a behavior component spells out for the move's own self.
                    .chain(
                        secondary
                            .self_effect
                            .iter()
                            .flat_map(|inner| inner.callbacks.iter())
                            .map(|callback| format!("self:{callback}")),
                    )
            }))
            .chain(self.self_effect.iter().flat_map(|self_effect| {
                self_effect
                    .callbacks
                    .iter()
                    .map(|callback| format!("self:{callback}"))
            }))
            .collect()
    }
}

/// A secondary or self effect table.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EffectSnapshot {
    #[serde(default)]
    pub chance: Option<u8>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub boosts: Option<BTreeMap<String, i8>>,
    #[serde(default, rename = "volatileStatus")]
    pub volatile_status: Option<String>,
    #[serde(default, rename = "self")]
    pub self_effect: Option<Box<EffectSnapshot>>,
    #[serde(default)]
    pub callbacks: Vec<String>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpeciesSnapshot {
    pub id: String,
    pub num: i32,
    /// Forme index from `data/dex/ids.json`; 0 is the base forme.
    #[serde(default)]
    pub form: u8,
    pub name: String,
    pub types: Vec<String>,
    pub base_stats: BaseStatsSnapshot,
    /// Slot ("0", "1", "H", "S") → ability name. Empty before Gen 3.
    #[serde(default)]
    pub abilities: BTreeMap<String, String>,
    #[serde(default)]
    pub weight_kg: f64,
    #[serde(default)]
    pub base_species: Option<String>,
    #[serde(default)]
    pub forme: Option<String>,
    /// mega | primal | regional | battle_only | other; absent on base formes.
    #[serde(default)]
    pub forme_kind: Option<String>,
    #[serde(default)]
    pub battle_only: bool,
    #[serde(default)]
    pub changes_from: Option<String>,
    #[serde(default)]
    pub required_item: Option<Value>,
    #[serde(default)]
    pub required_ability: Option<String>,
    /// The move a forme needs to change into (Showdown `requiredMove`; Mega Rayquaza's Dragon
    /// Ascent, DESIGN.md §44.3).
    #[serde(default)]
    pub required_move: Option<String>,
    /// Not fully evolved: an evolution of this species / forme exists in the same generation's
    /// dex (derived by the exporter, data/dex/README.md).
    #[serde(default)]
    pub nfe: bool,
    /// The species / forme's fixed gender (Showdown `species.gender`); absent when it has a
    /// gender ratio (derived by the exporter, data/dex/README.md).
    #[serde(default)]
    pub gender: Option<GenderSnapshot>,
    /// Gen 8: the species cannot Dynamax (DESIGN.md §44.1). Absent on every other generation
    /// and on the species that can.
    #[serde(default)]
    pub cannot_dynamax: bool,
}

/// A fixed gender as the snapshot writes it: male, female, or `N` for genderless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum GenderSnapshot {
    M,
    F,
    N,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct BaseStatsSnapshot {
    pub hp: u16,
    pub atk: u16,
    pub def: u16,
    pub spa: u16,
    pub spd: u16,
    pub spe: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AbilitySnapshot {
    pub id: String,
    pub eid: u16,
    pub num: i32,
    pub name: String,
    #[serde(default)]
    pub callbacks: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ItemSnapshot {
    pub id: String,
    pub eid: u16,
    /// Generation the item first appears in; `eid / 1000`.
    #[serde(default, rename = "gen")]
    pub generation: u8,
    pub num: i32,
    pub name: String,
    #[serde(default)]
    pub callbacks: Vec<String>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// Showdown's `toID`: lowercase alphanumerics only. Every name lookup goes through it so
/// "Power-Up Punch", "powerUpPunch" and "Power Up Punch" resolve identically.
pub fn to_id(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
