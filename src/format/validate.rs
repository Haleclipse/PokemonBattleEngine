//! The team validator (M9.2, DESIGN.md §45.3), Showdown's `TeamValidator` ported a layer at a
//! time. M9.2b: the rulings on one thing — `checkSpecies`, `checkItem`, `checkMove`,
//! `checkAbility`, `checkNature` — each against the format's rule table, with the problem worded
//! as Showdown words it (the differential compares the text) and classified for a caller.

use std::borrow::Cow;
use std::collections::BTreeSet;

use super::data::{FormatEntry, to_id};
use super::legality::{
    Aliases, BattleOnly, LegalityData, MaxMove, Named, SpeciesFacts, TAGS, ThingFacts,
};
use super::library::{BASE_MOD, FormatLibrary};
use super::resolve::{FormatError, FormatResolver};
use super::set::{Stats, TeamSet};
use super::table::RuleTable;

/// What a set has, as the bans name it (`pokemon:…`, `item:…`, `pokemontag:…`): Showdown's
/// `setHas`, read by the combination bans and the team's clauses.
pub type SetHas = BTreeSet<String>;

/// Showdown `RuleTable.check`: a thing whitelisted (`''`), banned (the reason), or neither
/// (`null`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Banned(String),
    Neutral,
}

impl RuleTable {
    /// Showdown `check(thing, setHas)`: an unban first; then, the set marked as having the
    /// thing, its ban's reason.
    pub fn check(&self, thing: &str, set_has: Option<&mut SetHas>) -> Verdict {
        if self.has(&format!("+{thing}")) {
            return Verdict::Allowed;
        }
        if let Some(set_has) = set_has {
            set_has.insert(thing.to_string());
        }
        match self.reason(&format!("-{thing}")) {
            Some(reason) => Verdict::Banned(reason),
            None => Verdict::Neutral,
        }
    }

    /// Showdown `getReason`: `banned by <rule>`, `banned`, or for Nonexistent `not obtainable`.
    /// (Showdown also tests `key.startsWith('obtainable')`, which a `-` key never does.)
    pub fn reason(&self, key: &str) -> Option<String> {
        let source = self.entries.get(key)?;
        Some(if key == "-nonexistent" || key.starts_with("obtainable") {
            "not obtainable".into()
        } else if source.is_empty() {
            "banned".into()
        } else {
            format!("banned by {source}")
        })
    }
}

/// What kind of problem a ruling found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum ProblemKind {
    /// Banned by name (or its Mega Evolution, its Gigantamax).
    Banned,
    /// Banned by a tag (`is tagged Uber, which is banned`).
    Tagged,
    /// Not in this game (another generation's, Let's Go's, a CAP).
    Nonexistent,
    /// In the game, but not to be had without hacking.
    Unobtainable,
    /// Not on the format's whitelist (`-All Pokemon` …).
    NotAllowed,
    /// The Gigantamax flag where it cannot be (or a G-Max Move without it).
    Gigantamax,
    /// A forme the set cannot start the battle in, or without what it needs.
    Forme,
    /// EVs, IVs (DVs), Hidden Power, and what Gen 2's DVs fix.
    Stats,
    /// A name that names nothing, or a value that is none (a species, an item, an ability, a
    /// move, a nature, a Hidden Power or Terastal type, the happiness).
    Invalid,
    /// Below the minimum level, above the maximum, or below its evolution's.
    Level,
    /// No ability, one the species cannot have, a Hidden Ability it cannot have so.
    Ability,
    /// No moves, too many, the same twice, or moves its origin rules out.
    Moves,
    /// A combination banned on one Pokémon.
    Combination,
    /// A rule's own check of a set (OHKO Clause, Sleep Moves Clause …).
    Clause,
    /// A nickname too long, or another species' name.
    Nickname,
    /// A gender the set cannot have.
    Gender,
    /// An item the species cannot hold (Gen 4's Griseous Orb).
    Item,
    /// A Terastal type the species cannot have.
    Tera,
    /// The team's size, or a team rule (one Gen 3 Deoxys forme, one Let's Go starter).
    Team,
    /// A move the set cannot learn, a moveset it cannot have together, an event it does not
    /// match.
    Learnset,
}

/// A problem with a team, worded as Showdown words it — and, for a program to act on it
/// (DESIGN.md §45.6), where it is and which rule it breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
    pub message: String,
    pub location: ProblemLocation,
    /// The rule broken, where one is: the rule table's key of a ban (`-pokemon:mewtwo`,
    /// `-pokemontag:uber`, `-nonexistent`) or a team rule's id (`speciesclause`,
    /// `itemclause`); `None` for an obtainability ruling of the validator's own.
    pub rule: Option<String>,
}

/// Where in the team a problem is: the registered set (none for the team as a whole) and the
/// part of it (none where Showdown's ruling spans several).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProblemLocation {
    pub set: Option<usize>,
    pub field: Option<SetField>,
}

/// A part of a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetField {
    Name,
    Species,
    Item,
    Ability,
    Nature,
    /// The moveset as a whole (none, too many, a copy, a combination).
    Moves,
    /// One move, by its index in the set.
    Move(usize),
    Evs,
    Ivs,
    Level,
    Gender,
    Happiness,
    HpType,
    TeraType,
    /// Where the set comes from: an event, Pokémon GO, an egg's fathers.
    Origin,
}

impl Problem {
    pub fn new(kind: ProblemKind, message: String) -> Self {
        Self {
            kind,
            message,
            location: ProblemLocation::default(),
            rule: None,
        }
    }

    /// The problem at `field`.
    pub fn at(mut self, field: SetField) -> Self {
        self.location.field = Some(field);
        self
    }

    /// The problem breaking `rule`.
    pub fn by(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }
}

/// Every problem from `from` on without a field of its own at `field`: a stretch of the
/// validation that rules on one part of the set.
pub(super) fn locate(problems: &mut [Problem], from: usize, field: SetField) {
    for problem in problems.iter_mut().skip(from) {
        problem.location.field.get_or_insert(field);
    }
}

fn problem(kind: ProblemKind, message: String) -> Option<Problem> {
    Some(Problem::new(kind, message))
}

/// The set a species ruling reads: its Gigantamax flag and its ability.
#[derive(Debug, Clone, Default)]
pub struct SpeciesSet<'a> {
    pub ability: &'a str,
    pub gigantamax: bool,
}

/// The tags whose ban means a thing is not in the game (`checkSpecies`'s `EXISTENCE_TAG`).
const EXISTENCE_TAGS: [&str; 7] = [
    "past",
    "future",
    "lgpe",
    "unobtainable",
    "cap",
    "custom",
    "nonexistent",
];

/// Nothing (`dex.items.get('')`): no item, no ability, no nature.
fn nothing() -> ThingFacts {
    ThingFacts::default()
}

/// A format's team validator.
pub struct TeamValidator<'d> {
    pub format: &'d FormatEntry,
    pub library: &'d FormatLibrary,
    /// The learnsets; none on the learnset seam's empty side (`without_learnsets`).
    pub learnsets: Option<super::learnset::Learnsets<'d>>,
    /// Pokémon GO's data; none on the seam's empty side.
    pub pokemon_go: Option<&'d super::learnset::PokemonGoTable>,
    /// The rules as the format's mod defines them.
    pub formats: &'d super::data::FormatData,
    pub mod_name: &'d str,
    pub table: RuleTable,
    pub legality: &'d LegalityData,
    pub aliases: &'d Aliases,
    pub generation: u32,
}

impl<'d> TeamValidator<'d> {
    /// A format's validator, with its learnsets: move legality needs them (they are optional
    /// data, DESIGN.md §46).
    pub fn new(library: &'d FormatLibrary, format_id: &str) -> Result<Self, FormatError> {
        Self::with_custom(library, format_id, &[])
    }

    /// A format with custom rules added (`format@@@rule, rule`).
    pub fn with_custom(
        library: &'d FormatLibrary,
        format_id: &str,
        custom_rules: &[String],
    ) -> Result<Self, FormatError> {
        let mut validator = Self::build(library, format_id, custom_rules)?;
        validator.learnsets = Some(
            library
                .learnsets(validator.mod_name)
                .map_err(FormatError::Data)?,
        );
        validator.pokemon_go = Some(library.pokemon_go().map_err(FormatError::Data)?);
        Ok(validator)
    }

    /// The learnset seam's empty side (§45.3): every move learnable, no species event-only, no
    /// Gen 1 encounter — the validator without the learnsets.
    pub fn without_learnsets(
        library: &'d FormatLibrary,
        format_id: &str,
        custom_rules: &[String],
    ) -> Result<Self, FormatError> {
        Self::build(library, format_id, custom_rules)
    }

    fn build(
        library: &'d FormatLibrary,
        format_id: &str,
        custom_rules: &[String],
    ) -> Result<Self, FormatError> {
        let format = library
            .formats()
            .format(format_id)
            .ok_or_else(|| FormatError::UnknownFormat(format_id.to_string()))?;
        let table = FormatResolver::new(library).resolve_custom(format_id, custom_rules)?;
        let legality = library
            .legality(format.mod_name.as_deref().unwrap_or(BASE_MOD))
            .map_err(FormatError::Data)?;
        let aliases = library.aliases().map_err(FormatError::Data)?;
        Ok(Self {
            format,
            library,
            learnsets: None,
            pokemon_go: None,
            formats: library.formats(),
            mod_name: format.mod_name.as_deref().unwrap_or(BASE_MOD),
            table,
            legality,
            aliases,
            generation: u32::from(format.generation.unwrap_or(9)),
        })
    }

    /// Whether the moves are checked against the learnsets: false on the seam's empty side
    /// (`without_learnsets`), whose verdicts can never say a team is legal, only that nothing
    /// else is wrong with it (DESIGN.md §45.6).
    pub fn checks_learnsets(&self) -> bool {
        self.learnsets.is_some()
    }

    /// Another generation's legality data (`dex.forGen(n)`).
    pub fn legality_of(&self, generation: u32) -> Option<&'d LegalityData> {
        self.library.legality(&format!("gen{generation}")).ok()
    }

    /// Whether the rule has the hook in the format's mod (and is in the table).
    pub fn runs(&self, rule: &str, hook: &str) -> bool {
        self.table.has(rule)
            && self
                .formats
                .entry(self.mod_name, rule)
                .is_some_and(|entry| entry.hooks.iter().any(|h| h == hook))
    }

    /// Showdown `dex.species.get(name)`: a species of the data, or the one Showdown makes of a
    /// name that is none (`exists: false`, Illegal, Custom).
    pub fn species(&self, name: &str) -> Cow<'d, SpeciesFacts> {
        match self.legality.species_named(self.aliases, name) {
            Named::Found(species) => Cow::Borrowed(species),
            Named::Empty => Cow::Owned(missing_species(String::new())),
            Named::Missing(id) => Cow::Owned(missing_species(id)),
        }
    }

    /// Showdown `dex.items.get(name)` (a missing one named by its id).
    pub fn item(&self, name: &str) -> Cow<'d, ThingFacts> {
        thing(self.legality.item_named(self.aliases, name))
    }

    /// Showdown `dex.abilities.get(name)`.
    pub fn ability(&self, name: &str) -> Cow<'d, ThingFacts> {
        thing(self.legality.ability_named(self.aliases, name))
    }

    /// Showdown `getValidationSpecies(set)`: the species the set is out of battle (Battle Bond
    /// Greninja, Own Tempo Rockruff) and the one its tier is judged by (its Mega, its Primal,
    /// its Crowned forme, Mega Rayquaza where Dragon Ascent makes it).
    pub fn validation_species(
        &self,
        set: &TeamSet,
    ) -> (Cow<'d, SpeciesFacts>, Cow<'d, SpeciesFacts>) {
        let species = self.species(&set.species);
        let item = self.item(&set.item);
        let ability = self.ability(&set.ability);
        let obtainable_formes = self.table.has("obtainableformes");
        let mut out_of_battle = species.clone();
        let mut tier = species.clone();
        let base = to_id(&species.base_species);
        if ability.id == "battlebond" && base == "greninja" {
            out_of_battle = self.species("greninjabond");
            if obtainable_formes {
                tier = out_of_battle.clone();
            }
        }
        if ability.id == "owntempo" && base == "rockruff" {
            out_of_battle = self.species("rockruffdusk");
            if obtainable_formes {
                tier = out_of_battle.clone();
            }
        }
        if obtainable_formes {
            let can_mega_evolve = self.generation <= 7 || self.table.has("+pokemontag:past");
            let mega = item
                .mega_stone
                .as_ref()
                .and_then(|stones| stones.get(&species.name));
            let moves: Vec<String> = set.moves.iter().map(|m| to_id(m)).collect();
            if let Some(mega) = mega {
                tier = self.species(mega);
            } else if item.id == "redorb" && species.id == "groudon" {
                tier = self.species("Groudon-Primal");
            } else if item.id == "blueorb" && species.id == "kyogre" {
                tier = self.species("Kyogre-Primal");
            } else if can_mega_evolve
                && species.id == "rayquaza"
                && moves.iter().any(|m| m == "dragonascent")
                && !self.table.has("megarayquazaclause")
            {
                tier = self.species("Rayquaza-Mega");
            } else if item.id == "rustedsword" && species.id == "zacian" {
                tier = self.species("Zacian-Crowned");
            } else if item.id == "rustedshield" && species.id == "zamazenta" {
                tier = self.species("Zamazenta-Crowned");
            }
        }
        (out_of_battle, tier)
    }

    /// Showdown `validateForme(set)`: the forme a set starts the battle in. A battle-only forme
    /// (a Mega, Darmanitan-Zen, Meloetta-Pirouette) becomes its out-of-battle one; a forme an
    /// item or a move makes (a Plate's Arceus, Keldeo-Resolute) needs it; a few formes take the
    /// set's moves or ability with them. The set comes back normalised.
    pub fn validate_forme(&self, set: &mut TeamSet) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut forme = |message: String| {
            problems.push(Problem::new(ProblemKind::Forme, message));
        };
        let name = if set.name.is_empty() {
            set.species.clone()
        } else {
            set.name.clone()
        };
        let item = self.item(&set.item);
        let species = self.species(&set.species);
        let move_ids =
            |set: &TeamSet| -> Vec<String> { set.moves.iter().map(|m| to_id(m)).collect() };
        let knows = |set: &TeamSet, id: &str| move_ids(set).iter().any(|m| m == id);
        if species.name == "Necrozma-Ultra" {
            let which =
                u8::from(knows(set, "sunsteelstrike")) + 2 * u8::from(knows(set, "moongeistbeam"));
            if item.name != "Ultranecrozium Z" {
                // Necrozma-Ultra transforms from one of two formes, neither the base one.
                forme("Necrozma-Ultra must start the battle holding Ultranecrozium Z.".into());
            } else if which == 1 {
                set.species = "Necrozma-Dusk-Mane".into();
                set.ability = "Prism Armor".into();
            } else if which == 2 {
                set.species = "Necrozma-Dawn-Wings".into();
                set.ability = "Prism Armor".into();
            } else {
                forme(
                    "Necrozma-Ultra must start the battle as Necrozma-Dusk-Mane or \
                     Necrozma-Dawn-Wings holding Ultranecrozium Z. Please specify which Necrozma \
                     it should start as."
                        .into(),
                );
            }
        } else if species.base_species == "Zygarde" {
            if species.name == "Zygarde-Complete" || species.name == "Zygarde-Mega" {
                forme(format!(
                    "{} must start the battle as Zygarde or Zygarde-10% with Power Construct. \
                     Please specify which Zygarde it should start as.",
                    species.name
                ));
            }
            if item.id == "zygardite" && set.ability != "Power Construct" {
                forme(
                    "Zygarde holding Zygardite can only Mega Evolve with the Power Construct \
                     ability."
                        .into(),
                );
            }
        } else if species.base_species == "Terapagos" {
            set.species = "Terapagos".into();
            set.ability = "Tera Shift".into();
        } else if let Some(battle_only) = &species.battle_only {
            if let Some(required) = &species.required_ability
                && set.ability != *required
            {
                // Darmanitan-Zen
                forme(format!(
                    "{} transforms in-battle with {required}, please fix its ability.",
                    species.name
                ));
            }
            if let Some(items) = &species.required_items
                && !items.contains(&item.name)
            {
                // A Mega or a Primal
                forme(format!(
                    "{} transforms in-battle with {}, please fix its item.",
                    species.name,
                    species.required_item.as_deref().unwrap_or("undefined")
                ));
            }
            if let Some(required) = &species.required_move
                && !knows(set, &to_id(required))
            {
                // Meloetta-Pirouette, Rayquaza-Mega
                forme(format!(
                    "{} transforms in-battle with {required}, please fix its moves.",
                    species.name
                ));
            }
            // The two battle-only formes of several (Ultra Necrozma, Complete Zygarde) were
            // handled above.
            if let BattleOnly::One(out_of_battle) = battle_only {
                set.species = out_of_battle.clone();
            }
        } else {
            if let Some(items) = &species.required_items
                && !items.contains(&item.name)
            {
                let wanted = items.join(" or ");
                if self.generation >= 8
                    && (species.base_species == "Arceus" || species.base_species == "Silvally")
                {
                    forme(format!("{name} needs to hold {wanted}."));
                    forme(
                        "(It will revert to its Normal forme if you remove the item or give it a \
                         different item.)"
                            .into(),
                    );
                } else {
                    // A Memory, a Drive, the Griseous Orb, a Plate, a Z-Crystal: forme mismatch.
                    let base = self.species(species.changes_from.as_deref().unwrap_or_default());
                    forme(format!(
                        "{name} needs to hold {wanted} to be in its {} forme.",
                        species.forme.as_deref().unwrap_or_default()
                    ));
                    forme(format!(
                        "(It will revert to its {} forme if you remove the item or give it a \
                         different item.)",
                        base.base_forme.as_deref().unwrap_or("base")
                    ));
                }
            }
            if let Some(required) = &species.required_move
                && !knows(set, &to_id(required))
            {
                let base = self.species(species.changes_from.as_deref().unwrap_or_default());
                forme(format!(
                    "{name} needs to know the move {required} to be in its {} forme.",
                    species.forme.as_deref().unwrap_or_default()
                ));
                forme(format!(
                    "(It will revert to its {} forme if it forgets the move.)",
                    // A species' `baseForme` is `''` when it has none.
                    base.base_forme.as_deref().unwrap_or_default()
                ));
            }
            // A set of the base species takes its item's forme (Arceus, Genesect, Giratina,
            // Silvally).
            if let Some(forced) = &item.forced_forme
                && species.name == self.species(forced).base_species
            {
                set.species = forced.clone();
            }
        }
        if species.name == "Pikachu-Cosplay" {
            // The move as written, not its id.
            const COSPLAY: [(&str, &str); 5] = [
                ("meteormash", "Pikachu-Rock-Star"),
                ("iciclecrash", "Pikachu-Belle"),
                ("drainingkiss", "Pikachu-Pop-Star"),
                ("electricterrain", "Pikachu-PhD"),
                ("flyingpress", "Pikachu-Libre"),
            ];
            if let Some((_, cosplay)) = set
                .moves
                .iter()
                .find_map(|m| COSPLAY.iter().find(|(id, _)| id == m))
            {
                set.species = (*cosplay).into();
            }
        }
        if species.name == "Keldeo" && knows(set, "secretsword") && self.generation >= 8 {
            set.species = "Keldeo-Resolute".into();
        }
        let behemoth = match species.name.as_str() {
            "Zacian-Crowned" => Some("behemothblade"),
            "Zamazenta-Crowned" => Some("behemothbash"),
            _ => None,
        };
        if let Some(behemoth) = behemoth
            && let Some(index) = move_ids(set).iter().position(|m| m == behemoth)
        {
            set.moves[index] = "ironhead".into();
        }
        if species.base_species == "Hoopa" && self.generation >= 9 {
            if species.name == "Hoopa" && knows(set, "hyperspacefury") {
                forme(
                    "In Generation 9, Hoopa cannot run Hyperspace Fury because it gets replaced \
                     with Hyperspace Hole upon changing forme."
                        .into(),
                );
            } else if species.name == "Hoopa-Unbound" && knows(set, "hyperspacehole") {
                forme(
                    "In Generation 9, Hoopa-Unbound cannot run Hyperspace Hole because it gets \
                     replaced with Hyperspace Fury upon changing forme."
                        .into(),
                );
            }
        }
        if species.base_species == "Greninja" && to_id(&set.ability) == "battlebond" {
            set.species = "Greninja-Bond".into();
        }
        if species.base_species == "Rockruff" && to_id(&set.ability) == "owntempo" {
            set.species = "Rockruff-Dusk".into();
        }
        if species.base_species == "Unown" && self.generation == 2 {
            // The letter the DVs give (Atk, Def, Spe, Spc: two middle bits each).
            let ivs = set.ivs.unwrap_or(Stats::filled(31));
            let bits: String = [ivs.atk, ivs.def, ivs.spe, ivs.spa]
                .iter()
                .map(|iv| {
                    let binary = js_binary(*iv);
                    let padded = format!("{binary:0>5}");
                    padded.chars().skip(1).take(2).collect::<String>()
                })
                .collect();
            let expected = js_parse_binary(&bits)
                .map(|value| char::from(u8::try_from(value / 10 + 65).unwrap_or(0)))
                .unwrap_or('\0');
            let letter = species.forme.as_deref().unwrap_or("A");
            if letter != expected.to_string() {
                forme(format!(
                    "Unown has forme {letter}, but its DVs give it the forme {expected}."
                ));
            }
        }
        problems
    }

    /// Showdown `checkSpecies(set, species, tierSpecies, setHas)`: `tier_species` is what the
    /// set is validated as (its Mega, its Primal, its Crowned forme …).
    pub fn check_species(
        &self,
        set: &SpeciesSet<'_>,
        species: &SpeciesFacts,
        tier_species: &SpeciesFacts,
        set_has: &mut SetHas,
    ) -> Option<Problem> {
        use ProblemKind::*;
        let table = &self.table;
        let mut species = species;
        // https://www.smogon.com/forums/posts/8659168
        if (tier_species.id == "zamazentacrowned" && species.id == "zamazenta")
            || (tier_species.id == "zaciancrowned" && species.id == "zacian")
        {
            species = tier_species;
        }
        let distinct = tier_species.id != species.id;
        set_has.insert(format!("pokemon:{}", species.id));
        set_has.insert(format!("basepokemon:{}", to_id(&species.base_species)));
        let mut is_mega = false;
        if distinct {
            set_has.insert(format!("pokemon:{}", tier_species.id));
            if tier_species.mega || tier_species.primal {
                set_has.insert("pokemontag:mega".into());
                is_mega = true;
            }
        }
        let mut is_gmax = false;
        if tier_species.can_gigantamax.is_some() && set.gigantamax {
            set_has.insert(format!("pokemon:{}gmax", tier_species.id));
            is_gmax = true;
        }
        let ability = to_id(set.ability);
        if tier_species.base_species == "Greninja" && ability == "battlebond" {
            set_has.insert("pokemon:greninjabond".into());
        }
        if tier_species.base_species == "Rockruff" && ability == "owntempo" {
            set_has.insert("pokemon:rockruffdusk".into());
        }
        let tier_id = |tier: &Option<String>| to_id(tier.as_deref().unwrap_or_default());
        set_has.insert(format!("pokemontag:{}", tier_id(&tier_species.tier)));
        let doubles = match tier_species.doubles_tier.as_deref() {
            Some("(DUU)") => "DNU".to_string(),
            other => other.unwrap_or_default().to_string(),
        };
        set_has.insert(format!("pokemontag:{}", to_id(&doubles)));
        set_has.insert(format!(
            "pokemontag:nd{}",
            tier_id(&tier_species.nat_dex_tier)
        ));

        if tier_species.can_gigantamax.is_none() && set.gigantamax {
            return problem(
                Gigantamax,
                format!(
                    "{} cannot Gigantamax but is flagged as being able to.",
                    tier_species.name
                ),
            );
        }
        match table.check(&format!("pokemon:{}", species.id), None) {
            Verdict::Banned(reason) => {
                return problem(Banned, format!("{} is {reason}.", species.name))
                    .map(|p| p.by(format!("-pokemon:{}", species.id)));
            }
            Verdict::Allowed => return None,
            Verdict::Neutral => {}
        }
        if distinct {
            match table.check(&format!("pokemon:{}", tier_species.id), None) {
                Verdict::Banned(reason) => {
                    return problem(Banned, format!("{} is {reason}.", tier_species.name))
                        .map(|p| p.by(format!("-pokemon:{}", tier_species.id)));
                }
                Verdict::Allowed => return None,
                Verdict::Neutral => {}
            }
        }
        if is_mega && let Verdict::Banned(reason) = table.check("pokemontag:mega", Some(set_has)) {
            return problem(Banned, format!("Mega evolutions are {reason}."))
                .map(|p| p.by("-pokemontag:mega"));
        }
        if is_gmax
            && let Verdict::Banned(reason) =
                table.check(&format!("pokemon:{}gmax", tier_species.id), None)
        {
            return problem(
                Banned,
                format!("Gigantamaxing {} is {reason}.", species.name),
            )
            .map(|p| p.by(format!("-pokemon:{}gmax", tier_species.id)));
        }
        let base = to_id(&species.base_species);
        match table.check(&format!("basepokemon:{base}"), None) {
            Verdict::Banned(reason) => {
                return problem(Banned, format!("{} is {reason}.", species.name))
                    .map(|p| p.by(format!("-basepokemon:{base}")));
            }
            // An unbanned base species is not its nonstandard formes (unbanning Pichu does not
            // allow Pichu-Spiky-Eared outside Gen 4).
            Verdict::Allowed => {
                if self
                    .legality
                    .species(&base)
                    .is_some_and(|base| base.nonstandard == species.nonstandard)
                {
                    return None;
                }
            }
            Verdict::Neutral => {}
        }
        // `-Nonexistent` is complicated in what trumps it: `+Mythical` should not unban
        // Shaymin in Gen 1.
        let mut nonexistent = if nonexistent_tag(tier_species.nonstandard.as_deref()) {
            table.check("nonexistent", None)
        } else {
            Verdict::Neutral
        };
        for rule in &table.tag_rules {
            if rule.starts_with('*') {
                continue;
            }
            let tag_id = &rule[12..];
            let Some(tag) = TAGS.iter().find(|tag| tag.id == tag_id) else {
                continue;
            };
            if !tag.matches(tier_species) {
                continue;
            }
            let existence = EXISTENCE_TAGS.contains(&tag_id);
            if rule.starts_with('+') {
                // `+CAP` trumps `-Nonexistent`, most tags do not.
                if !existence && matches!(nonexistent, Verdict::Banned(_)) {
                    continue;
                }
                return None;
            }
            if existence {
                nonexistent = Verdict::Banned("banned".into());
                break;
            }
            let reason = match table.check(&rule[1..], None) {
                Verdict::Banned(reason) => reason,
                _ => "banned".into(),
            };
            return problem(
                Tagged,
                format!(
                    "{} is tagged {}, which is {reason}.",
                    species.name, tag.name
                ),
            )
            .map(|p| p.by(rule.clone()));
        }
        match nonexistent {
            Verdict::Banned(_) => {
                let name = &tier_species.name;
                let generation = self.generation;
                return (match tier_species.nonstandard.as_deref() {
                    Some("Past" | "Future") => problem(
                        Nonexistent,
                        format!("{name} does not exist in Gen {generation}."),
                    ),
                    Some("LGPE") => problem(
                        Nonexistent,
                        format!(
                            "{name} does not exist in this game, only in Let's Go Pikachu/Eevee."
                        ),
                    ),
                    Some("CAP") => problem(
                        Nonexistent,
                        format!("{name} is a CAP and does not exist in this game."),
                    ),
                    Some("Unobtainable") => problem(
                        Unobtainable,
                        format!("{name} is not possible to obtain in this game."),
                    ),
                    Some("Gigantamax") => problem(
                        Nonexistent,
                        format!(
                            "{name} is a placeholder for a Gigantamax sprite, not a real Pokémon. \
                             (This message is likely to be a validator bug.)"
                        ),
                    ),
                    _ => problem(Nonexistent, format!("{name} does not exist in this game.")),
                })
                .map(|p| p.by("-nonexistent"));
            }
            Verdict::Allowed => return None,
            Verdict::Neutral => {}
        }
        // A species that can Gigantamax whose Gigantamax Factor cannot be had.
        if tier_species.gmax_unreleased && set.gigantamax {
            match table.check("pokemontag:unobtainable", None) {
                Verdict::Banned(_) => {
                    return problem(
                        Unobtainable,
                        format!(
                            "{} is flagged as gigantamax, but it cannot gigantamax without \
                             hacking or glitches.",
                            tier_species.name
                        ),
                    )
                    .map(|p| p.by("-pokemontag:unobtainable"));
                }
                Verdict::Allowed => return None,
                Verdict::Neutral => {}
            }
        }
        if let Verdict::Banned(_) = table.check("pokemontag:allpokemon", None) {
            return problem(
                NotAllowed,
                format!("{} is not in the list of allowed pokemon.", species.name),
            )
            .map(|p| p.by("-pokemontag:allpokemon"));
        }
        None
    }

    /// Showdown `checkItem(set, item, setHas)`; `None` is no item.
    pub fn check_item(
        &self,
        set_name: &str,
        item: Option<&ThingFacts>,
        set_has: &mut SetHas,
    ) -> Option<Problem> {
        use ProblemKind::*;
        let nothing = nothing();
        let item = item.unwrap_or(&nothing);
        let table = &self.table;
        set_has.insert(format!("item:{}", item.id));
        let id = if item.id.is_empty() {
            "noitem"
        } else {
            &item.id
        };
        match table.check(&format!("item:{id}"), None) {
            Verdict::Banned(reason) => {
                return problem(
                    Banned,
                    if item.id.is_empty() {
                        format!("{set_name} not holding an item is {reason}.")
                    } else {
                        format!("{set_name}'s item {} is {reason}.", item.name)
                    },
                )
                .map(|p| p.by(format!("-item:{id}")));
            }
            Verdict::Allowed => return None,
            Verdict::Neutral => {}
        }
        if item.id.is_empty() {
            return None;
        }
        if let Verdict::Banned(_) = table.check("pokemontag:allitems", None) {
            return problem(
                NotAllowed,
                format!(
                    "{set_name}'s item {} is not in the list of allowed items.",
                    item.name
                ),
            )
            .map(|p| p.by("-pokemontag:allitems"));
        }
        if let Some(nonstandard) = item.nonstandard.as_deref() {
            match table.check(&format!("pokemontag:{}", to_id(nonstandard)), None) {
                Verdict::Banned(reason) => {
                    return if nonstandard == "Unobtainable" {
                        problem(
                            Unobtainable,
                            format!(
                                "{} is not obtainable without hacking or glitches.",
                                item.name
                            ),
                        )
                        .map(|p| p.by(format!("-pokemontag:{}", to_id(nonstandard))))
                    } else {
                        problem(
                            Tagged,
                            format!(
                                "{set_name}'s item {} is tagged {nonstandard}, which is {reason}.",
                                item.name
                            ),
                        )
                        .map(|p| p.by(format!("-pokemontag:{}", to_id(nonstandard))))
                    };
                }
                Verdict::Allowed => return None,
                Verdict::Neutral => {}
            }
            if nonstandard != "Unobtainable" {
                return self.nonexistent_thing(set_name, "item", item, set_has);
            }
        }
        None
    }

    /// Showdown `checkMove(set, move, setHas)`.
    pub fn check_move(
        &self,
        set_name: &str,
        the_move: &ThingFacts,
        set_has: &mut SetHas,
    ) -> Option<Problem> {
        use ProblemKind::*;
        let table = &self.table;
        set_has.insert(format!("move:{}", the_move.id));
        match table.check(&format!("move:{}", the_move.id), None) {
            Verdict::Banned(reason) => {
                return problem(
                    Banned,
                    format!("{set_name}'s move {} is {reason}.", the_move.name),
                )
                .map(|p| p.by(format!("-move:{}", the_move.id)));
            }
            Verdict::Allowed => return None,
            Verdict::Neutral => {}
        }
        if let Verdict::Banned(_) = table.check("pokemontag:allmoves", None) {
            return problem(
                NotAllowed,
                format!(
                    "{set_name}'s move {} is not in the list of allowed moves.",
                    the_move.name
                ),
            )
            .map(|p| p.by("-pokemontag:allmoves"));
        }
        if let Some(nonstandard) = the_move.nonstandard.as_deref() {
            match table.check(&format!("pokemontag:{}", to_id(nonstandard)), None) {
                Verdict::Banned(reason) => {
                    return match nonstandard {
                        "Unobtainable" => {
                            let later = self.generation >= 9
                                && u32::from(the_move.generation) < self.generation;
                            problem(
                                Unobtainable,
                                format!(
                                    "{} is not obtainable without hacking or glitches{}.",
                                    the_move.name,
                                    if later {
                                        format!(" in Gen {}", self.generation)
                                    } else {
                                        String::new()
                                    }
                                ),
                            )
                            .map(|p| p.by(format!("-pokemontag:{}", to_id(nonstandard))))
                        }
                        "Gigantamax" => {
                            let user = match &the_move.max {
                                Some(MaxMove::Gmax(user)) => user.clone(),
                                Some(MaxMove::Max(true)) => "true".into(),
                                _ => "undefined".into(),
                            };
                            problem(
                                Gigantamax,
                                format!(
                                    "{} is not usable without Gigantamaxing its user, {user}.",
                                    the_move.name
                                ),
                            )
                            .map(|p| p.by(format!("-pokemontag:{}", to_id(nonstandard))))
                        }
                        _ => problem(
                            Tagged,
                            format!(
                                "{set_name}'s move {} is tagged {nonstandard}, which is {reason}.",
                                the_move.name
                            ),
                        )
                        .map(|p| p.by(format!("-pokemontag:{}", to_id(nonstandard)))),
                    };
                }
                Verdict::Allowed => return None,
                Verdict::Neutral => {}
            }
            if nonstandard != "Unobtainable" {
                return self.nonexistent_thing(set_name, "move", the_move, set_has);
            }
        }
        None
    }

    /// Showdown `checkAbility(set, ability, setHas)`; `None` is no ability.
    pub fn check_ability(
        &self,
        set_name: &str,
        ability: Option<&ThingFacts>,
        set_has: &mut SetHas,
    ) -> Option<Problem> {
        self.check_kind(
            set_name,
            ability,
            set_has,
            "ability",
            "pokemontag:allabilities",
            "abilities",
        )
    }

    /// Showdown `checkNature(set, nature, setHas)`; `None` is no nature. (Showdown checks its
    /// whitelist as `allnatures`, without the `pokemontag:` the other four have.)
    pub fn check_nature(
        &self,
        set_name: &str,
        nature: Option<&ThingFacts>,
        set_has: &mut SetHas,
    ) -> Option<Problem> {
        self.check_kind(set_name, nature, set_has, "nature", "allnatures", "natures")
    }

    /// `checkAbility` and `checkNature`, which differ only in their names.
    fn check_kind(
        &self,
        set_name: &str,
        thing: Option<&ThingFacts>,
        set_has: &mut SetHas,
        kind: &str,
        whitelist: &str,
        plural: &str,
    ) -> Option<Problem> {
        use ProblemKind::*;
        let nothing = nothing();
        let thing = thing.unwrap_or(&nothing);
        let table = &self.table;
        set_has.insert(format!("{kind}:{}", thing.id));
        match table.check(&format!("{kind}:{}", thing.id), None) {
            Verdict::Banned(reason) => {
                return problem(
                    Banned,
                    format!("{set_name}'s {kind} {} is {reason}.", thing.name),
                )
                .map(|p| p.by(format!("-{kind}:{}", thing.id)));
            }
            Verdict::Allowed => return None,
            Verdict::Neutral => {}
        }
        if let Verdict::Banned(_) = table.check(whitelist, None) {
            return problem(
                NotAllowed,
                format!(
                    "{set_name}'s {kind} {} is not in the list of allowed {plural}.",
                    thing.name
                ),
            )
            .map(|p| p.by(format!("-{whitelist}")));
        }
        if let Some(nonstandard) = thing.nonstandard.as_deref() {
            match table.check(&format!("pokemontag:{}", to_id(nonstandard)), None) {
                Verdict::Banned(reason) => {
                    return problem(
                        Tagged,
                        format!(
                            "{set_name}'s {kind} {} is tagged {nonstandard}, which is {reason}.",
                            thing.name
                        ),
                    )
                    .map(|p| p.by(format!("-pokemontag:{}", to_id(nonstandard))));
                }
                Verdict::Allowed => return None,
                Verdict::Neutral => {}
            }
            return self.nonexistent_thing(set_name, kind, thing, set_has);
        }
        None
    }

    /// The last check of an item, move, ability or nature: `-Nonexistent`.
    fn nonexistent_thing(
        &self,
        set_name: &str,
        kind: &str,
        thing: &ThingFacts,
        set_has: &mut SetHas,
    ) -> Option<Problem> {
        match self.table.check("nonexistent", Some(set_has)) {
            Verdict::Banned(_) => {
                let past = matches!(thing.nonstandard.as_deref(), Some("Past" | "Future"));
                Some(
                    Problem::new(
                        ProblemKind::Nonexistent,
                        if past {
                            format!(
                                "{set_name}'s {kind} {} does not exist in Gen {}.",
                                thing.name, self.generation
                            )
                        } else {
                            format!(
                                "{set_name}'s {kind} {} does not exist in this game.",
                                thing.name
                            )
                        },
                    )
                    .by("-nonexistent"),
                )
            }
            _ => None,
        }
    }
}

/// Showdown's Nonexistent tag on a nonstandard value.
fn nonexistent_tag(nonstandard: Option<&str>) -> bool {
    nonstandard.is_some_and(|value| !value.is_empty() && value != "Unobtainable")
}

/// The species Showdown makes of a name that is none.
fn missing_species(id: String) -> SpeciesFacts {
    SpeciesFacts {
        name: id.clone(),
        base_species: id.clone(),
        id,
        exists: false,
        tier: Some("Illegal".into()),
        doubles_tier: Some("Illegal".into()),
        nat_dex_tier: Some("Illegal".into()),
        nonstandard: Some("Custom".into()),
        ..SpeciesFacts::default()
    }
}

/// A thing of the data, or the one Showdown makes of a name that is none (named by its id).
fn thing(named: Named<'_, ThingFacts>) -> Cow<'_, ThingFacts> {
    match named {
        Named::Found(thing) => Cow::Borrowed(thing),
        Named::Empty => Cow::Owned(ThingFacts::default()),
        Named::Missing(id) => Cow::Owned(ThingFacts {
            key: id.clone(),
            name: id.clone(),
            id,
            ..ThingFacts::default()
        }),
    }
}

/// JavaScript `n.toString(2)` of an integer.
fn js_binary(value: i32) -> String {
    if value < 0 {
        format!("-{:b}", value.unsigned_abs())
    } else {
        format!("{value:b}")
    }
}

/// JavaScript `parseInt(text, 2)`: an optional sign, then the leading binary digits; none is
/// NaN.
fn js_parse_binary(text: &str) -> Option<i64> {
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let digits: String = digits
        .chars()
        .take_while(|c| *c == '0' || *c == '1')
        .collect();
    if digits.is_empty() {
        return None;
    }
    i64::from_str_radix(&digits, 2)
        .ok()
        .map(|value| sign * value)
}
