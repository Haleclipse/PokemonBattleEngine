//! The format layer's data directory (DESIGN.md §46): what a host configures rather than what
//! the engine is. Format rules, bans, tiers and tags change by the season and are read from
//! files at run time, never embedded in the binary; the dex is a generation's fixed facts,
//! validated together with the code, and stays compiled in (`crate::data`).
//!
//! ```text
//! <root>/formats/formats.json    the formats and their rules (M9.1)
//! <root>/legality/<mod>.json     each generation mod's names, tiers, tags … (M9.2)
//! <root>/legality/aliases.json   the alias table
//! <root>/legality/learnsets*.json the learnsets (M9.3; optional: only team validation with
//!                                 move legality reads them)
//! ```
//!
//! The format data is read when the directory is opened; a mod's legality data and the
//! aliases on first use, once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::data::FormatData;
use super::learnset::{LearnsetTable, Learnsets, PokemonGoTable};
use super::legality::{Aliases, LegalityData};

/// Why the data directory cannot give the format layer its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    /// The directory, or a file in it, cannot be read.
    Unreadable { path: PathBuf, reason: String },
    /// A file is not what this engine reads (not JSON of its schema, another mod's).
    Malformed { path: PathBuf, reason: String },
    /// No format of the data is of this generation mod.
    UnknownMod(String),
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable { path, reason } => {
                write!(f, "{}: cannot be read: {reason}", path.display())
            }
            Self::Malformed { path, reason } => write!(f, "{}: {reason}", path.display()),
            Self::UnknownMod(name) => write!(f, "no format of the data is of mod {name}"),
        }
    }
}

impl std::error::Error for DataError {}

/// A format data directory, opened.
#[derive(Debug)]
pub struct FormatLibrary {
    root: PathBuf,
    formats: FormatData,
    legality: BTreeMap<String, OnceLock<Result<LegalityData, DataError>>>,
    aliases: OnceLock<Result<Aliases, DataError>>,
    /// `learnsets.json`, then `learnsets-<mod>.json` per mod and the two external games'.
    learnsets: BTreeMap<String, OnceLock<Result<LearnsetTable, DataError>>>,
    pokemon_go: OnceLock<Result<PokemonGoTable, DataError>>,
    digest: OnceLock<String>,
}

/// Showdown's base mod (`DEFAULT_MOD`): its dex checks a custom rule's names first.
pub(super) const BASE_MOD: &str = "gen9";

impl FormatLibrary {
    /// Opens a data directory and reads its format data.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, DataError> {
        let root = root.into();
        let path = root.join("formats").join("formats.json");
        let formats = FormatData::parse(&read(&path)?)
            .map_err(|reason| DataError::Malformed { path, reason })?;
        let mut legality = BTreeMap::new();
        legality.insert(BASE_MOD.to_string(), OnceLock::new());
        for format in &formats.formats {
            let mod_name = format.mod_name.clone().unwrap_or_else(|| BASE_MOD.into());
            legality.entry(mod_name).or_insert_with(OnceLock::new);
        }
        let mut learnsets = BTreeMap::new();
        for name in ["", "gen8bdsp", "gen8legends"]
            .into_iter()
            .map(str::to_string)
            .chain(legality.keys().cloned())
        {
            learnsets.insert(name, OnceLock::new());
        }
        Ok(Self {
            root,
            formats,
            legality,
            aliases: OnceLock::new(),
            learnsets,
            pokemon_go: OnceLock::new(),
            digest: OnceLock::new(),
        })
    }

    internal!(
        pub fn root(&self) -> &Path {
            &self.root
        }
    );

    internal!(
        /// The formats and their rules.
        pub fn formats(&self) -> &FormatData {
            &self.formats
        }
    );

    /// Reads and checks every file now, rather than on first use.
    pub fn preload(&self) -> Result<(), DataError> {
        for mod_name in self.legality.keys() {
            self.legality(mod_name)?;
        }
        self.aliases()?;
        Ok(())
    }

    internal!(
        /// A generation mod's legality data (`legality/<mod>.json`).
        pub fn legality(&self, mod_name: &str) -> Result<&LegalityData, DataError> {
            let slot = self
                .legality
                .get(mod_name)
                .ok_or_else(|| DataError::UnknownMod(mod_name.to_string()))?;
            slot.get_or_init(|| {
                let path = self.root.join("legality").join(format!("{mod_name}.json"));
                let data =
                    LegalityData::parse(&read(&path)?).map_err(|reason| DataError::Malformed {
                        path: path.clone(),
                        reason,
                    })?;
                if data.mod_name != mod_name {
                    return Err(DataError::Malformed {
                        path,
                        reason: format!("the data of mod {}", data.mod_name),
                    });
                }
                Ok(data)
            })
            .as_ref()
            .map_err(Clone::clone)
        }
    );

    internal!(
        /// A generation mod's learnsets (`legality/learnsets.json` and the mod's own entries), with
        /// the external games' for the HOME relearner.
        pub fn learnsets(&self, mod_name: &str) -> Result<Learnsets<'_>, DataError> {
            Ok(Learnsets {
                base: self.learnset_table("")?,
                own: self.learnset_table(mod_name)?,
                bdsp: self.learnset_table("gen8bdsp")?,
                legends: self.learnset_table("gen8legends")?,
            })
        }
    );

    internal!(
        /// Pokémon GO's data (`legality/pokemongo.json`, optional like the learnsets).
        pub fn pokemon_go(&self) -> Result<&PokemonGoTable, DataError> {
            self.pokemon_go
                .get_or_init(|| {
                    let path = self.root.join("legality").join("pokemongo.json");
                    PokemonGoTable::parse(&read(&path)?)
                        .map_err(|reason| DataError::Malformed { path, reason })
                })
                .as_ref()
                .map_err(Clone::clone)
        }
    );

    fn learnset_table(&self, name: &str) -> Result<&LearnsetTable, DataError> {
        let slot = self
            .learnsets
            .get(name)
            .ok_or_else(|| DataError::UnknownMod(name.to_string()))?;
        slot.get_or_init(|| {
            let file = if name.is_empty() {
                "learnsets.json".to_string()
            } else {
                format!("learnsets-{name}.json")
            };
            let path = self.root.join("legality").join(file);
            LearnsetTable::parse(&read(&path)?)
                .map_err(|reason| DataError::Malformed { path, reason })
        })
        .as_ref()
        .map_err(Clone::clone)
    }

    internal!(
        /// The data directory's identity (DESIGN.md §45.5): the SHA-256 of every file this library
        /// may read — the format data, each mod's legality data, the aliases, the learnsets,
        /// Pokémon GO's — each by its path and its own digest (`-` for an optional file that is
        /// absent). A team validated against other data is another registration.
        pub fn data_digest(&self) -> &str {
            self.digest.get_or_init(|| {
                let mut files = vec![
                    "formats/formats.json".to_string(),
                    "legality/aliases.json".to_string(),
                    "legality/pokemongo.json".to_string(),
                ];
                files.extend(
                    self.legality
                        .keys()
                        .map(|mod_name| format!("legality/{mod_name}.json")),
                );
                files.extend(self.learnsets.keys().map(|name| {
                    if name.is_empty() {
                        "legality/learnsets.json".to_string()
                    } else {
                        format!("legality/learnsets-{name}.json")
                    }
                }));
                files.sort();
                let lines: Vec<String> = files
                    .iter()
                    .map(|file| {
                        let digest = std::fs::read(self.root.join(file))
                            .map_or("-".to_string(), |bytes| crate::digest::sha256_hex(&bytes));
                        format!("{file} {digest}")
                    })
                    .collect();
                format!(
                    "data:{}",
                    crate::digest::sha256_hex(lines.join("\n").as_bytes())
                )
            })
        }
    );

    internal!(
        /// The alias table (`legality/aliases.json`).
        pub fn aliases(&self) -> Result<&Aliases, DataError> {
            self.aliases
                .get_or_init(|| {
                    let path = self.root.join("legality").join("aliases.json");
                    Aliases::parse(&read(&path)?)
                        .map_err(|reason| DataError::Malformed { path, reason })
                })
                .as_ref()
                .map_err(Clone::clone)
        }
    );
}

fn read(path: &Path) -> Result<String, DataError> {
    std::fs::read_to_string(path).map_err(|error| DataError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}
