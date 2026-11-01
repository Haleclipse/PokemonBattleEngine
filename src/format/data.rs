//! The format layer's data: `formats/formats.json` in the data directory (read at run time by
//! `FormatLibrary`, never embedded, DESIGN.md §46), exported from Showdown by
//! `tools/oracle-node/src/export-formats.mjs` (DESIGN.md §45.2). It holds the target formats
//! and every rule they reach under each generation's dex (a generation can redefine a rule:
//! Gen 3's Standard brings Switch Priority Clause Mod), with their rule and ban strings as
//! Showdown writes them; the names in a ban are resolved with the legality data (§45.3).

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;

/// Showdown `toID`, shared with the dex.
pub use crate::data::to_id;

/// The format data schema this engine reads (`schema` in `formats.json`).
pub const FORMATS_SCHEMA: u32 = 4;

/// Whether a rule takes a value (`Rule = value`), and of what kind (Showdown `hasValue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasValue {
    /// Any text (`hasValue: true`).
    Text,
    /// An integer (`'integer'`).
    Integer,
    /// A positive integer (`'positive-integer'`).
    PositiveInteger,
}

/// A format, or a rule (Showdown keeps both in one table: a format can be another's rule).
#[derive(Debug, Clone, Deserialize)]
pub struct FormatEntry {
    pub id: String,
    pub name: String,
    pub effect_type: String,
    pub ruleset: Vec<String>,
    pub banlist: Vec<String>,
    pub unbanlist: Vec<String>,
    pub restricted: Vec<String>,
    #[serde(deserialize_with = "has_value")]
    pub has_value: Option<HasValue>,
    pub mutually_exclusive_with: Option<String>,
    /// A format's generation mod (`gen9`); none for a rule.
    #[serde(rename = "mod", default)]
    pub mod_name: Option<String>,
    /// A format's generation number.
    #[serde(rename = "gen", default)]
    pub generation: Option<u8>,
    /// A format's game type (`singles`, `doubles`).
    #[serde(default)]
    pub game_type: Option<String>,
    /// The hooks the rule has in its mod (`onValidateSet`, `onValidateTeam` …): a mod can
    /// drop one, and the engine runs a rule's hook only where Showdown's has it.
    #[serde(default)]
    pub hooks: Vec<String>,
}

fn has_value<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<HasValue>, D::Error> {
    Ok(match Value::deserialize(de)? {
        Value::Bool(true) => Some(HasValue::Text),
        Value::String(kind) if kind == "integer" => Some(HasValue::Integer),
        Value::String(kind) if kind == "positive-integer" => Some(HasValue::PositiveInteger),
        _ => None,
    })
}

#[derive(Debug, Deserialize)]
struct RawData {
    schema: u32,
    formats: Vec<FormatEntry>,
    rules: BTreeMap<String, Vec<FormatEntry>>,
}

/// The parsed format data, indexed.
#[derive(Debug)]
pub struct FormatData {
    pub formats: Vec<FormatEntry>,
    /// Each generation mod's rules (`gen3` → its Standard, its Obtainable…).
    pub rules: BTreeMap<String, Vec<FormatEntry>>,
    by_id: HashMap<(String, String), usize>,
}

impl FormatData {
    /// The format data from its text (`FormatLibrary` reads it from the data directory).
    pub fn parse(raw: &str) -> Result<Self, String> {
        let data: RawData = serde_json::from_str(raw).map_err(|error| error.to_string())?;
        if data.schema != FORMATS_SCHEMA {
            return Err(format!(
                "schema {}, this engine reads {FORMATS_SCHEMA}",
                data.schema
            ));
        }
        let mut by_id = HashMap::new();
        for (mod_name, rules) in &data.rules {
            for (index, entry) in rules.iter().enumerate() {
                by_id.insert((mod_name.clone(), entry.id.clone()), index);
            }
        }
        Ok(Self {
            formats: data.formats,
            rules: data.rules,
            by_id,
        })
    }

    /// The rule (or format used as a rule) with this id under a generation mod's dex (Showdown
    /// `Dex.mod(mod).formats.get`).
    pub fn entry(&self, mod_name: &str, id: &str) -> Option<&FormatEntry> {
        let index = *self.by_id.get(&(mod_name.to_string(), id.to_string()))?;
        Some(&self.rules[mod_name][index])
    }

    /// A target format by id.
    pub fn format(&self, id: &str) -> Option<&FormatEntry> {
        self.formats.iter().find(|format| format.id == id)
    }
}
