//! The legality data (DESIGN.md §45.3): per generation mod, every species, move, item, ability
//! and nature the mod's dex knows, with the facts the format layer reads of them — tiers, tags,
//! `isNonstandard` …; the alias table; Showdown's tags (`data/tags.ts`) as predicates over those
//! facts; and the name resolution of a ban (`validateBanRule`). Exported from Showdown by
//! `tools/oracle-node/src/export-legality.mjs` and read at run time (`FormatLibrary`, §46).

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use super::data::to_id;

/// The legality data schema this engine reads (`schema` in `legality/<mod>.json`).
pub const LEGALITY_SCHEMA: u32 = 5;

/// A species as Showdown's `dex.species.get` resolves it. The data leaves out what is null,
/// false or empty; it reads as the default.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SpeciesFacts {
    pub id: String,
    pub name: String,
    pub base_species: String,
    pub forme: Option<String>,
    pub num: i32,
    /// The generation the species first appears in.
    #[serde(rename = "gen")]
    pub generation: u8,
    pub tier: Option<String>,
    pub doubles_tier: Option<String>,
    pub nat_dex_tier: Option<String>,
    /// Showdown `isNonstandard` (`Past`, `Future`, `Unobtainable`, `CAP` …).
    pub nonstandard: Option<String>,
    pub tags: Vec<String>,
    pub mega: bool,
    pub primal: bool,
    pub can_gigantamax: Option<String>,
    pub gmax_unreleased: bool,
    /// The raw entry has other or cosmetic formes: named without one, a ban means them all.
    pub has_formes: bool,
    pub exists: bool,
    /// Slot (`0`, `1`, `H`, `S`) → ability name.
    pub abilities: BTreeMap<String, String>,
    pub types: Vec<String>,
    /// A fixed gender (`M`, `F`, `N`).
    pub gender: Option<String>,
    pub gender_ratio: GenderRatio,
    pub prevo: Option<String>,
    pub evo_level: Option<u32>,
    pub nfe: bool,
    pub egg_groups: Vec<String>,
    pub base_forme: Option<String>,
    /// The out-of-battle forme (or formes) of a battle-only one.
    pub battle_only: Option<BattleOnly>,
    pub changes_from: Option<String>,
    pub required_ability: Option<String>,
    pub required_item: Option<String>,
    pub required_items: Option<Vec<String>>,
    pub required_move: Option<String>,
    pub required_tera_type: Option<String>,
    /// The Hidden Ability is unreleased (`true`), or only in a past generation (`"Past"`).
    pub unreleased_hidden: Unreleased,
    pub male_only_hidden: bool,
    pub cosmetic_formes: Option<Vec<String>>,
    pub is_cosmetic_forme: bool,
    pub evos: Vec<String>,
    /// How it evolves (`levelMove`: knowing `evo_move`).
    pub evo_type: Option<String>,
    pub evo_move: Option<String>,
    /// Where it evolves (`Alola`: an Alolan evolution of a prevo from elsewhere).
    pub evo_region: Option<String>,
    /// The species a male-only one hatches from (Gallade's Kirlia line from Gardevoir's).
    pub mother: Option<String>,
    pub can_hatch: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct GenderRatio {
    #[serde(rename = "M")]
    pub male: f64,
    #[serde(rename = "F")]
    pub female: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BattleOnly {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Unreleased {
    #[default]
    No,
    Flag(bool),
    Since(String),
}

/// A move's `isMax`: a Max Move (`true`) or a G-Max Move (the species' name).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MaxMove {
    Max(bool),
    Gmax(String),
}

/// An item, move, ability or nature as Showdown resolves it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ThingFacts {
    /// The dex table's key, what a ban's name is looked up by.
    pub key: String,
    /// The thing's own id, what a ruling records (Hidden Power Dark's is `hiddenpower`).
    pub id: String,
    pub name: String,
    pub nonstandard: Option<String>,
    /// The generation the thing first appears in.
    #[serde(rename = "gen")]
    pub generation: u8,
    /// Moves: the move's type (Hidden Power Fire's is Fire).
    #[serde(rename = "type")]
    pub move_type: Option<String>,
    /// Moves: a one-hit KO move (OHKO Clause).
    pub ohko: bool,
    /// Moves: the status it inflicts (`slp` for Sleep Moves Clause).
    pub status: Option<String>,
    /// Moves: the accuracy, `true` for a move that never misses.
    pub accuracy: Option<serde_json::Value>,
    /// Moves: lowers the target's accuracy for sure (Accuracy Trap Clause's own filter).
    pub lowers_accuracy: bool,
    /// Moves: Sketch cannot copy it.
    pub nosketch: bool,
    /// Items: a Gem (Gems Clause).
    pub is_gem: bool,
    /// Moves: a Z-Move.
    pub z: bool,
    /// Moves: a Max or G-Max Move.
    pub max: Option<MaxMove>,
    /// Items: the species a Mega Stone evolves, by name, into its Mega's name.
    pub mega_stone: Option<BTreeMap<String, String>>,
    /// Items: the forme the holder takes (a Plate's Arceus, a Drive's Genesect).
    pub forced_forme: Option<String>,
}

/// A type (`dex.types.get`): its name and the IVs (DVs) Hidden Power of it takes.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TypeFacts {
    pub id: String,
    pub name: String,
    pub nonstandard: Option<String>,
    pub hp_ivs: Option<BTreeMap<String, u8>>,
    pub hp_dvs: Option<BTreeMap<String, u8>>,
}

#[derive(Debug, Deserialize)]
struct RawLegality {
    schema: u32,
    #[serde(rename = "mod")]
    mod_name: String,
    #[serde(rename = "gen")]
    generation: u8,
    species: Vec<SpeciesFacts>,
    moves: Vec<ThingFacts>,
    items: Vec<ThingFacts>,
    abilities: Vec<ThingFacts>,
    natures: Vec<ThingFacts>,
    types: Vec<TypeFacts>,
    /// The aliases that, as a species name, make a species of their own (a cosmetic forme, a
    /// special event species).
    extra_species: BTreeMap<String, SpeciesFacts>,
    species_order: Vec<String>,
}

/// What `validateBanRule` searches: one kind of thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BanKind {
    Pokemon,
    Move,
    Ability,
    Item,
    Nature,
    PokemonTag,
}

impl BanKind {
    /// Showdown's order of search.
    pub const ALL: [Self; 6] = [
        Self::Pokemon,
        Self::Move,
        Self::Ability,
        Self::Item,
        Self::Nature,
        Self::PokemonTag,
    ];

    pub fn prefix(self) -> &'static str {
        match self {
            Self::Pokemon => "pokemon",
            Self::Move => "move",
            Self::Ability => "ability",
            Self::Item => "item",
            Self::Nature => "nature",
            Self::PokemonTag => "pokemontag",
        }
    }
}

/// Why a ban names nothing, or too much (Showdown throws on each).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanError {
    Nothing,
    Ambiguous(Vec<String>),
}

/// One generation mod's legality data.
#[derive(Debug)]
pub struct LegalityData {
    pub mod_name: String,
    pub generation: u8,
    pub species: Vec<SpeciesFacts>,
    pub moves: Vec<ThingFacts>,
    pub items: Vec<ThingFacts>,
    pub abilities: Vec<ThingFacts>,
    pub natures: Vec<ThingFacts>,
    pub types: Vec<TypeFacts>,
    extra_species: BTreeMap<String, SpeciesFacts>,
    /// The species in `dex.species.all()`'s order (the Pokédex's own).
    pub species_order: Vec<String>,
    index: [HashMap<String, usize>; 5],
}

/// What a name resolves to (`dex.species.get(name)` …): nothing (an empty name), a thing of the
/// data, or a thing that does not exist, by the id Showdown gives it (its name too).
#[derive(Debug, Clone, PartialEq)]
pub enum Named<'a, T> {
    Empty,
    Found(&'a T),
    Missing(String),
}

impl<T> Named<'_, T> {
    pub fn found(&self) -> Option<&T> {
        match self {
            Named::Found(thing) => Some(thing),
            _ => None,
        }
    }
}

/// `getByID`'s guesses for a species name the table lacks: a regional or Mega forme written
/// before or after the species (`alolanraichu`, `raichua`, `megacharizardx` …).
const FORME_NAMES: [(&str, &[&str]); 6] = [
    ("alola", &["a", "alola", "alolan"]),
    ("galar", &["g", "galar", "galarian"]),
    ("hisui", &["h", "hisui", "hisuian"]),
    ("paldea", &["p", "paldea", "paldean"]),
    ("mega", &["m", "mega"]),
    ("primal", &["p", "primal"]),
];

impl LegalityData {
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawLegality = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if raw.schema != LEGALITY_SCHEMA {
            return Err(format!(
                "schema {}, this engine reads {LEGALITY_SCHEMA}",
                raw.schema
            ));
        }
        let ids = |rows: &[ThingFacts]| -> HashMap<String, usize> {
            rows.iter()
                .enumerate()
                .map(|(index, row)| (row.key.clone(), index))
                .collect()
        };
        let species = raw
            .species
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.clone(), index))
            .collect();
        let index = [
            species,
            ids(&raw.moves),
            ids(&raw.abilities),
            ids(&raw.items),
            ids(&raw.natures),
        ];
        Ok(Self {
            mod_name: raw.mod_name,
            generation: raw.generation,
            species: raw.species,
            moves: raw.moves,
            items: raw.items,
            abilities: raw.abilities,
            natures: raw.natures,
            types: raw.types,
            extra_species: raw.extra_species,
            species_order: raw.species_order,
            index,
        })
    }

    fn row<'a>(
        &'a self,
        rows: &'a [ThingFacts],
        table: usize,
        key: &str,
    ) -> Option<&'a ThingFacts> {
        self.index[table].get(key).map(|&index| &rows[index])
    }

    /// Showdown `dex.species.get(name)`: the name trimmed and made an id (`Nidoran♀` is
    /// `nidoranf`).
    pub fn species_named(&self, aliases: &Aliases, name: &str) -> Named<'_, SpeciesFacts> {
        let name = name.trim();
        let mut id = to_id(name);
        if id == "nidoran" && name.ends_with('♀') {
            id = "nidoranf".into();
        } else if id == "nidoran" && name.ends_with('♂') {
            id = "nidoranm".into();
        }
        self.species_by_id(aliases, &id)
    }

    /// Showdown `dex.species.getByID(id)`: an alias first (its own species, if it makes one — a
    /// cosmetic forme, a special event species), then a forme guessed from the name, then the
    /// table.
    pub fn species_by_id(&self, aliases: &Aliases, id: &str) -> Named<'_, SpeciesFacts> {
        if id.is_empty() || id == "constructor" {
            return Named::Empty;
        }
        if let Some(target) = aliases.get(id) {
            if let Some(own) = self.extra_species.get(id) {
                return Named::Found(own);
            }
            return self.species_named(aliases, target);
        }
        if self.species(id).is_none() {
            for (forme, names) in FORME_NAMES {
                let mut poke = String::new();
                for prefix in names {
                    if let Some(rest) = id.strip_prefix(prefix) {
                        poke = rest.to_string();
                    } else if let Some(rest) = id.strip_suffix(prefix) {
                        poke = rest.to_string();
                    }
                }
                if let Some(target) = aliases.get(&poke) {
                    poke = target.to_string();
                }
                let guess = format!("{poke}{forme}");
                if self.species(&guess).is_some() {
                    if let Named::Found(species) = self.species_named(aliases, &guess) {
                        return Named::Found(species);
                    }
                    break;
                }
            }
        }
        match self.species(id) {
            Some(species) => Named::Found(species),
            None => Named::Missing(id.to_string()),
        }
    }

    /// Showdown `dex.items.get(name)`: an alias, then a Berry named without `Berry`.
    pub fn item_named(&self, aliases: &Aliases, name: &str) -> Named<'_, ThingFacts> {
        self.item_by_id(aliases, &to_id(name.trim()))
    }

    fn item_by_id(&self, aliases: &Aliases, id: &str) -> Named<'_, ThingFacts> {
        if id.is_empty() || id == "constructor" {
            return Named::Empty;
        }
        if let Some(target) = aliases.get(id) {
            return self.item_named(aliases, target);
        }
        let berry = format!("{id}berry");
        if self.row(&self.items, 3, id).is_none() && self.row(&self.items, 3, &berry).is_some() {
            return self.item_by_id(aliases, &berry);
        }
        match self.row(&self.items, 3, id) {
            Some(item) => Named::Found(item),
            None => Named::Missing(id.to_string()),
        }
    }

    /// Showdown `dex.moves.get(name)`: an alias, then Hidden Power without its power
    /// (`hiddenpowerfire60` is `hiddenpowerfire`).
    pub fn move_named(&self, aliases: &Aliases, name: &str) -> Named<'_, ThingFacts> {
        let mut id = to_id(name.trim());
        if id.is_empty() || id == "constructor" {
            return Named::Empty;
        }
        if let Some(target) = aliases.get(&id) {
            return self.move_named(aliases, target);
        }
        if id.starts_with("hiddenpower") {
            id = id.chars().take_while(char::is_ascii_lowercase).collect();
        }
        match self.row(&self.moves, 1, &id) {
            Some(the_move) => Named::Found(the_move),
            None => Named::Missing(id),
        }
    }

    /// Showdown `dex.abilities.get(name)`.
    pub fn ability_named(&self, aliases: &Aliases, name: &str) -> Named<'_, ThingFacts> {
        self.simple_named(aliases, name, 2)
    }

    /// Showdown `dex.natures.get(name)`.
    pub fn nature_named(&self, aliases: &Aliases, name: &str) -> Named<'_, ThingFacts> {
        self.simple_named(aliases, name, 4)
    }

    fn simple_named(&self, aliases: &Aliases, name: &str, table: usize) -> Named<'_, ThingFacts> {
        let id = to_id(name);
        if id.is_empty() || id == "constructor" {
            return Named::Empty;
        }
        if let Some(target) = aliases.get(&id) {
            return self.simple_named(aliases, target, table);
        }
        let rows = if table == 2 {
            &self.abilities
        } else {
            &self.natures
        };
        match self.row(rows, table, &id) {
            Some(thing) => Named::Found(thing),
            None => Named::Missing(id),
        }
    }

    /// Showdown `dex.types.get(name)`: no alias, the type's name its id capitalised.
    pub fn type_named(&self, name: &str) -> Named<'_, TypeFacts> {
        let id = to_id(name);
        if id.is_empty() || id == "constructor" {
            return Named::Empty;
        }
        match self.types.iter().find(|row| row.id == id) {
            Some(row) => Named::Found(row),
            None => Named::Missing(id),
        }
    }

    fn has(&self, kind: BanKind, id: &str) -> bool {
        let table = match kind {
            BanKind::Pokemon => 0,
            BanKind::Move => 1,
            BanKind::Ability => 2,
            BanKind::Item => 3,
            BanKind::Nature => 4,
            BanKind::PokemonTag => return false,
        };
        self.index[table].contains_key(id)
    }

    /// The species with this id (`dex.species.get(id)` for an id of the mod's table).
    pub fn species(&self, id: &str) -> Option<&SpeciesFacts> {
        self.index[0].get(id).map(|&index| &self.species[index])
    }

    /// Showdown `validateBanRule`: what a ban's name names, as a ban spec (`pokemon:salamence`,
    /// `basepokemon:pikachu`, `pokemontag:mythical`, `item:noitem` …).
    pub fn ban_spec(&self, aliases: &Aliases, rule: &str) -> Result<String, BanError> {
        let mut id = to_id(rule);
        if id == "unreleased" || id == "nonexistent" {
            return Ok(id);
        }
        let mut kinds: &[BanKind] = &BanKind::ALL;
        let only;
        for kind in BanKind::ALL {
            if rule.starts_with(&format!("{}:", kind.prefix())) {
                only = [kind];
                kinds = &only;
                id = id[kind.prefix().len()..].to_string();
                break;
            }
        }
        let rule_id = id.clone();
        if let Some(target) = aliases.get(&id) {
            id = target.to_string();
        }
        let mut matches = Vec::new();
        for &kind in kinds {
            if kind == BanKind::Item && rule_id == "noitem" {
                return Ok("item:noitem".into());
            }
            if kind == BanKind::PokemonTag {
                if ALL_TAGS.contains(&rule_id.as_str()) || Tag::from_id(&rule_id).is_some() {
                    matches.push(format!("pokemontag:{rule_id}"));
                }
                continue;
            }
            if self.has(kind, &id) {
                if kind == BanKind::Pokemon
                    && self.species(&id).is_some_and(|species| species.has_formes)
                    && rule_id == id
                {
                    matches.push(format!("basepokemon:{id}"));
                    continue;
                }
                matches.push(format!("{}:{id}", kind.prefix()));
            } else if kind == BanKind::Pokemon && id.ends_with("base") {
                // Showdown strips `base` from the id itself: the kinds searched after it see
                // the stripped id too.
                id.truncate(id.len() - 4);
                if self.has(BanKind::Pokemon, &id) {
                    matches.push(format!("pokemon:{id}"));
                }
            }
        }
        match matches.len() {
            0 => Err(BanError::Nothing),
            1 => Ok(matches.remove(0)),
            _ => Err(BanError::Ambiguous(matches)),
        }
    }
}

/// The `pokemontag:all…` names: not tags, the whitelist switches.
const ALL_TAGS: [&str; 5] = [
    "allpokemon",
    "allitems",
    "allmoves",
    "allabilities",
    "allnatures",
];

/// Showdown `getAlias`'s table: alias id → target id, the same for every mod.
#[derive(Debug, Default)]
pub struct Aliases(BTreeMap<String, String>);

impl Aliases {
    pub fn parse(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Raw {
            schema: u32,
            aliases: BTreeMap<String, String>,
        }
        let raw: Raw = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if raw.schema != LEGALITY_SCHEMA {
            return Err(format!(
                "schema {}, this engine reads {LEGALITY_SCHEMA}",
                raw.schema
            ));
        }
        Ok(Self(raw.aliases))
    }

    pub fn get(&self, id: &str) -> Option<&str> {
        self.0.get(id).map(String::as_str)
    }
}

/// What a tag tests (Showdown's `speciesFilter` / `genericFilter`).
#[derive(Debug, Clone, Copy)]
enum Filter {
    Mega,
    /// `species.tags.includes(…)`.
    Tagged(&'static str),
    Tier(&'static [&'static str]),
    DoublesTier(&'static [&'static str]),
    NatDexTier(&'static [&'static str]),
    /// `isNonstandard === …`.
    Nonstandard(&'static str),
    /// `!!isNonstandard && isNonstandard !== 'Unobtainable'`.
    Nonexistent,
}

/// A Showdown tag with a species or generic filter: the tags a `pokemontag:` ban can name.
#[derive(Debug, Clone, Copy)]
pub struct Tag {
    pub id: &'static str,
    pub name: &'static str,
    filter: Filter,
    /// A `genericFilter` (it tests items, moves, abilities and natures too).
    pub generic: bool,
}

const fn species_tag(id: &'static str, name: &'static str, filter: Filter) -> Tag {
    Tag {
        id,
        name,
        filter,
        generic: false,
    }
}

const fn generic_tag(id: &'static str, name: &'static str, filter: Filter) -> Tag {
    Tag {
        id,
        name,
        filter,
        generic: true,
    }
}

/// `data/tags.ts`, the tags with a species or generic filter, in its order.
pub const TAGS: [Tag; 45] = [
    species_tag("mega", "Mega", Filter::Mega),
    species_tag("mythical", "Mythical", Filter::Tagged("Mythical")),
    species_tag(
        "sublegendary",
        "Sub-Legendary",
        Filter::Tagged("Sub-Legendary"),
    ),
    species_tag(
        "restrictedlegendary",
        "Restricted Legendary",
        Filter::Tagged("Restricted Legendary"),
    ),
    species_tag("ultrabeast", "Ultra Beast", Filter::Tagged("Ultra Beast")),
    species_tag("paradox", "Paradox", Filter::Tagged("Paradox")),
    species_tag("uber", "Uber", Filter::Tier(&["Uber", "AG", "(AG)"])),
    species_tag("ou", "OU", Filter::Tier(&["OU", "(OU)"])),
    species_tag("uubl", "UUBL", Filter::Tier(&["UUBL"])),
    species_tag("uu", "UU", Filter::Tier(&["UU"])),
    species_tag("rubl", "RUBL", Filter::Tier(&["RUBL"])),
    species_tag("ru", "RU", Filter::Tier(&["RU"])),
    species_tag("nubl", "NUBL", Filter::Tier(&["NUBL"])),
    species_tag("nu", "NU", Filter::Tier(&["NU"])),
    species_tag("publ", "PUBL", Filter::Tier(&["PUBL"])),
    species_tag("pu", "PU", Filter::Tier(&["PU"])),
    species_tag("zubl", "ZUBL", Filter::Tier(&["ZUBL"])),
    species_tag("zu", "ZU", Filter::Tier(&["ZU"])),
    species_tag("nfe", "NFE", Filter::Tier(&["NFE"])),
    species_tag("lc", "LC", Filter::DoublesTier(&["LC"])),
    species_tag("captier", "CAP Tier", Filter::Nonstandard("CAP")),
    species_tag("caplc", "CAP LC", Filter::Tier(&["CAP LC"])),
    species_tag("capnfe", "CAP NFE", Filter::Tier(&["CAP NFE"])),
    species_tag("ag", "AG", Filter::Tier(&["AG", "(AG)"])),
    species_tag("duber", "DUber", Filter::DoublesTier(&["DUber", "(DUber)"])),
    species_tag("dou", "DOU", Filter::DoublesTier(&["DOU", "(DOU)"])),
    species_tag("dbl", "DBL", Filter::DoublesTier(&["DBL"])),
    species_tag("duu", "DUU", Filter::DoublesTier(&["DUU"])),
    species_tag("dnu", "DNU", Filter::DoublesTier(&["(DUU)"])),
    species_tag("ndag", "ND AG", Filter::NatDexTier(&["AG", "(AG)"])),
    species_tag("nduber", "ND Uber", Filter::NatDexTier(&["Uber"])),
    species_tag("ndou", "ND OU", Filter::NatDexTier(&["OU", "(OU)"])),
    species_tag("nduubl", "ND UUBL", Filter::NatDexTier(&["UUBL"])),
    species_tag("nduu", "ND UU", Filter::NatDexTier(&["UU"])),
    species_tag("ndrubl", "ND RUBL", Filter::NatDexTier(&["RUBL"])),
    species_tag("ndru", "ND RU", Filter::NatDexTier(&["RU"])),
    species_tag("ndnfe", "ND NFE", Filter::NatDexTier(&["NFE"])),
    species_tag("ndlc", "ND LC", Filter::NatDexTier(&["LC"])),
    generic_tag("past", "Past", Filter::Nonstandard("Past")),
    generic_tag("future", "Future", Filter::Nonstandard("Future")),
    generic_tag("lgpe", "LGPE", Filter::Nonstandard("LGPE")),
    generic_tag(
        "unobtainable",
        "Unobtainable",
        Filter::Nonstandard("Unobtainable"),
    ),
    species_tag("cap", "CAP", Filter::Nonstandard("CAP")),
    generic_tag("custom", "Custom", Filter::Nonstandard("Custom")),
    generic_tag("nonexistent", "Nonexistent", Filter::Nonexistent),
];

impl Tag {
    /// Showdown `validPokemonTag`: a tag with a species or generic filter.
    pub fn from_id(id: &str) -> Option<&'static Tag> {
        TAGS.iter().find(|tag| tag.id == id)
    }

    /// The tag's filter on a species.
    pub fn matches(&self, species: &SpeciesFacts) -> bool {
        let is = |value: &Option<String>, wanted: &[&str]| {
            value
                .as_deref()
                .is_some_and(|value| wanted.contains(&value))
        };
        match self.filter {
            Filter::Mega => species.mega,
            Filter::Tagged(tag) => species.tags.iter().any(|t| t == tag),
            Filter::Tier(tiers) => is(&species.tier, tiers),
            Filter::DoublesTier(tiers) => is(&species.doubles_tier, tiers),
            Filter::NatDexTier(tiers) => is(&species.nat_dex_tier, tiers),
            Filter::Nonstandard(value) => species.nonstandard.as_deref() == Some(value),
            Filter::Nonexistent => nonexistent(species.nonstandard.as_deref()),
        }
    }

    /// A generic tag's filter on an item, move, ability or nature (`None` for a species tag).
    pub fn matches_thing(&self, thing: &ThingFacts) -> Option<bool> {
        if !self.generic {
            return None;
        }
        Some(match self.filter {
            Filter::Nonstandard(value) => thing.nonstandard.as_deref() == Some(value),
            Filter::Nonexistent => nonexistent(thing.nonstandard.as_deref()),
            _ => false,
        })
    }
}

fn nonexistent(nonstandard: Option<&str>) -> bool {
    nonstandard.is_some_and(|value| !value.is_empty() && value != "Unobtainable")
}
