//! The learnsets (M9.3, DESIGN.md §45.3): optional data — a host that does not validate teams
//! need not ship them — read from the data directory (`legality/learnsets*.json`, §46). Each
//! species' moves with their sources (`9M`, `8L36`, `7E`, `6S0` …), its events, whether it is
//! event-only and its Gen 1-2 encounters; per mod, the base learnsets with the mod's own
//! entries over them; and Brilliant Diamond / Shining Pearl's and Legends: Arceus' learnsets for
//! Gen 9's HOME relearner.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;

/// The learnset data schema this engine reads.
pub const LEARNSET_SCHEMA: u32 = 1;

/// A move's sources, by move id.
pub type MoveSources = BTreeMap<String, Vec<String>>;

/// An event's shininess: never (absent), may be (`1`), must be (`true`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EventShiny {
    #[default]
    Never,
    May,
    Must,
}

impl<'de> Deserialize<'de> for EventShiny {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        Ok(match Value::deserialize(de)? {
            Value::Bool(true) => Self::Must,
            Value::Bool(false) | Value::Null => Self::Never,
            _ => Self::May,
        })
    }
}

/// One event a species was given at (Showdown `EventInfo`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EventInfo {
    pub generation: u32,
    pub level: Option<u32>,
    pub shiny: EventShiny,
    pub gender: Option<String>,
    pub nature: Option<String>,
    /// The IVs the event fixes, in the data's order (Showdown reads them in that order).
    pub ivs: Option<OrderedStats>,
    #[serde(rename = "perfectIVs")]
    pub perfect_ivs: Option<u32>,
    pub is_hidden: bool,
    pub abilities: Option<Vec<String>>,
    pub moves: Option<Vec<String>>,
    pub japan: bool,
    pub emerald_event_egg: bool,
    pub from: Option<String>,
}

/// Numbers by name in the order the data writes them (an event's IVs, Let's Go's restrictive
/// moves): Showdown iterates them in that order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderedStats(pub Vec<(String, i32)>);

impl OrderedStats {
    pub fn get(&self, stat: &str) -> Option<i32> {
        self.0
            .iter()
            .find(|(name, _)| name == stat)
            .map(|(_, v)| *v)
    }
}

impl<'de> Deserialize<'de> for OrderedStats {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = OrderedStats;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("numbers by name")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut stats = Vec::new();
                while let Some((name, value)) = map.next_entry::<String, i32>()? {
                    stats.push((name, value));
                }
                Ok(OrderedStats(stats))
            }
        }
        de.deserialize_map(Visitor)
    }
}

/// A Gen 1-2 encounter: where and at which level a species is met.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Encounter {
    pub generation: u32,
    pub level: Option<u32>,
}

/// A species' learnset data (Showdown `Learnset`); a species without one has none of it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Learnset {
    pub learnset: Option<MoveSources>,
    pub event_data: Option<Vec<EventInfo>>,
    pub event_only: bool,
    pub encounters: Option<Vec<Encounter>>,
}

#[derive(Debug, Deserialize)]
struct RawLearnsets {
    schema: u32,
    learnsets: HashMap<String, Learnset>,
}

/// One learnset file, parsed.
#[derive(Debug, Default)]
pub struct LearnsetTable(HashMap<String, Learnset>);

impl LearnsetTable {
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawLearnsets = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if raw.schema != LEARNSET_SCHEMA {
            return Err(format!(
                "schema {}, this engine reads {LEARNSET_SCHEMA}",
                raw.schema
            ));
        }
        Ok(Self(raw.learnsets))
    }

    pub fn get(&self, id: &str) -> Option<&Learnset> {
        self.0.get(id)
    }
}

/// A species' Pokémon GO data: how it is met there, and (Mew, Mewtwo …) the Let's Go level-up
/// moves a Pokémon sent through Let's Go is limited to, by the level it learns them at.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PokemonGo {
    pub encounters: Option<Vec<String>>,
    pub lgpe_restrictive_moves: Option<OrderedStats>,
}

#[derive(Debug, Deserialize)]
struct RawPokemonGo {
    schema: u32,
    pokemongo: HashMap<String, PokemonGo>,
}

/// `legality/pokemongo.json`, parsed.
#[derive(Debug, Default)]
pub struct PokemonGoTable(HashMap<String, PokemonGo>);

impl PokemonGoTable {
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawPokemonGo = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if raw.schema != LEARNSET_SCHEMA {
            return Err(format!(
                "schema {}, this engine reads {LEARNSET_SCHEMA}",
                raw.schema
            ));
        }
        Ok(Self(raw.pokemongo))
    }

    /// Showdown `getPokemonGoData(id)`.
    pub fn get(&self, id: &str) -> Option<&PokemonGo> {
        self.0.get(id)
    }
}

/// A mod's learnsets: its own entries over the base ones, and the two external games' for the
/// HOME relearner.
#[derive(Debug, Clone, Copy)]
pub struct Learnsets<'a> {
    pub base: &'a LearnsetTable,
    pub own: &'a LearnsetTable,
    pub bdsp: &'a LearnsetTable,
    pub legends: &'a LearnsetTable,
}

static NO_LEARNSET: Learnset = Learnset {
    learnset: None,
    event_data: None,
    event_only: false,
    encounters: None,
};

impl Learnset {
    /// No learnset data (`exists: false`).
    pub fn none() -> &'static Learnset {
        &NO_LEARNSET
    }
}

impl<'a> Learnsets<'a> {
    /// Showdown `dex.species.getLearnsetData(id)`: the species' data, or none (`exists: false`).
    pub fn data(&self, id: &str) -> &'a Learnset {
        self.own
            .get(id)
            .or_else(|| self.base.get(id))
            .unwrap_or(&NO_LEARNSET)
    }

    /// Showdown `getExternalLearnsetData(id, mod)` for `gen8bdsp` / `gen8legends`: the move's
    /// sources there, for a species standard there.
    pub fn external(&self, id: &str, legends: bool, move_id: &str) -> Option<&'a Vec<String>> {
        let table = if legends { self.legends } else { self.bdsp };
        table.get(id)?.learnset.as_ref()?.get(move_id)
    }
}
