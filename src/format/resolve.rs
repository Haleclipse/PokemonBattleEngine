//! Format resolution: a format's rules composed into its rule table the way Showdown's
//! `Dex.formats.getRuleTable` does (DESIGN.md §45.2): sub-rules expanded recursively, `!Rule`
//! repealing, `+` / `*` / `-` unbanning / restricting / banning, value rules (`Rule = value`,
//! `!!` replacing), mutually exclusive rules, the numbers (`resolveNumbers`) and the rules' own
//! checks (`onValidateRule`). The result is compared item by item with Showdown's
//! (`tests/format_rules.rs`).

use std::collections::HashMap;

use super::data::{FormatEntry, HasValue, to_id};
use super::legality::{Aliases, BanError, LegalityData};
use super::library::{BASE_MOD, DataError, FormatLibrary};
use super::table::{ComplexBan, RuleNumbers, RuleTable};

/// Why a format could not be resolved (Showdown throws on each).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum FormatError {
    UnknownFormat(String),
    UnknownRule(String),
    /// A ban names nothing (`Nothing matches`).
    UnresolvedBan(String),
    /// A ban names more than one thing (`More than one thing matches`): the specs it could be.
    AmbiguousBan(String, Vec<String>),
    /// A rule string with spaces around it.
    Untrimmed(String),
    ConfusingRule(String),
    /// A custom rule bans or unbans a Pokémon before the All Pokemon rule.
    PokemonRuleOrder(String),
    /// None of the custom rules changes the format.
    NoChange,
    /// The data directory could not give the format's data.
    Data(DataError),
    MultipleRepeals(String),
    RepealDidNothing(String),
    AlreadyExists(String),
    PokemonRuleBeforeAllPokemon(String),
    MissingValue(String),
    UnexpectedValue(String),
    NotAnInteger(String),
    NotPositive(String),
    Redundant(String),
    NotReplacing(String),
    DoubleBangUnsupported(String),
    Conflicts(String),
    MutuallyExclusive(String),
    Recursion(String),
    AllPokemonWithoutBans,
    /// A rule's own check failed (`onValidateRule`), or a number is out of range
    /// (`resolveNumbers`).
    Invalid(String),
}

/// A rule string as `validateRule` leaves it: a rule id or ban spec (`-pokemon:salamence`,
/// `!!adjustlevel=60`), or a combination's ban.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleSpec {
    Text(String),
    Complex {
        team: bool,
        rule: String,
        /// `None`: no limit (an unban, `+`).
        limit: Option<u32>,
        bans: Vec<String>,
    },
}

use RuleSpec as Spec;

/// The repeals in force (`!Rule`): rule id → the depth it was written at, negative once it
/// repealed something (Showdown's `repeals` map, shared down the recursion).
type Repeals = Option<HashMap<String, i32>>;

pub struct FormatResolver<'d> {
    library: &'d FormatLibrary,
}

impl<'d> FormatResolver<'d> {
    pub fn new(library: &'d FormatLibrary) -> Self {
        Self { library }
    }

    /// The dex a format of `mod_name` resolves under: its legality data and the aliases.
    fn dex<'m>(&self, mod_name: &'m str, generation: u32) -> Result<Dex<'m>, FormatError>
    where
        'd: 'm,
    {
        Ok(Dex {
            mod_name,
            generation,
            legality: self.library.legality(mod_name).map_err(FormatError::Data)?,
            aliases: self.library.aliases().map_err(FormatError::Data)?,
        })
    }

    /// A target format's rule table.
    pub fn resolve(&self, format_id: &str) -> Result<RuleTable, FormatError> {
        self.resolve_custom(format_id, &[])
    }

    /// A target format with custom rules added (Showdown `format@@@rule, rule`).
    pub fn resolve_custom(
        &self,
        format_id: &str,
        custom_rules: &[String],
    ) -> Result<RuleTable, FormatError> {
        let format = self
            .library
            .formats()
            .format(format_id)
            .ok_or_else(|| FormatError::UnknownFormat(format_id.to_string()))?;
        let dex = self.dex(mod_of(format), u32::from(format.generation.unwrap_or(9)))?;
        // Showdown starts at depth 1 and, for a format of another generation's dex, starts over
        // at depth 2 under that dex (only the recursion limit reads the depth itself).
        let depth = if dex.mod_name == BASE_MOD { 1 } else { 2 };
        if custom_rules.is_empty() {
            return self.rule_table(&dex, format, depth, &mut None, None);
        }
        let custom = self.validate_custom(format, custom_rules)?;
        self.rule_table(&dex, format, depth, &mut None, Some(&custom))
    }

    /// Showdown `Dex.formats.validate` for `format@@@rules`: each rule cleaned of line breaks
    /// and `|` and trimmed, checked under the base dex, a Pokémon's ban not before All Pokemon,
    /// and one of them at least changing the format.
    fn validate_custom(
        &self,
        format: &FormatEntry,
        custom_rules: &[String],
    ) -> Result<Vec<String>, FormatError> {
        let base = self.dex(BASE_MOD, 9)?;
        let table = self.resolve(&format.id)?;
        let mut changes = false;
        let mut pokemon_rule = false;
        let mut cleaned = Vec::new();
        for rule in custom_rules {
            let rule: String = rule
                .chars()
                .filter(|c| !matches!(c, '\r' | '\n' | '|'))
                .collect::<String>()
                .trim()
                .to_string();
            let spec = self.validate_rule_under(&base, &rule)?;
            if let Spec::Text(text) = &spec {
                if (text == "-pokemontag:allpokemon" || text == "+pokemontag:allpokemon")
                    && pokemon_rule
                {
                    return Err(FormatError::PokemonRuleOrder(rule));
                }
                if is_pokemon_rule(text) {
                    pokemon_rule = true;
                }
            }
            if !matches!(&spec, Spec::Text(text) if table.has(text)) {
                changes = true;
            }
            cleaned.push(rule);
        }
        if !changes {
            return Err(FormatError::NoChange);
        }
        Ok(cleaned)
    }

    fn entry(&self, dex: &Dex<'_>, name: &str) -> Result<&'d FormatEntry, FormatError> {
        self.library
            .formats()
            .entry(dex.mod_name, &to_id(name))
            .ok_or_else(|| FormatError::UnknownRule(name.to_string()))
    }

    /// Showdown `validateRule` under a generation mod's dex.
    pub fn validate_rule(&self, mod_name: &str, raw: &str) -> Result<RuleSpec, FormatError> {
        let generation = self
            .library
            .formats()
            .formats
            .iter()
            .find(|format| mod_of(format) == mod_name)
            .and_then(|format| format.generation)
            .map_or(9, u32::from);
        self.validate_rule_under(&self.dex(mod_name, generation)?, raw)
    }

    fn validate_rule_under(&self, dex: &Dex<'_>, raw: &str) -> Result<Spec, FormatError> {
        if raw != raw.trim() {
            return Err(FormatError::Untrimmed(raw.to_string()));
        }
        let first = raw.chars().next().unwrap_or(' ');
        if "+*-".contains(first) {
            let body = &raw[1..];
            if body.contains('>') || body.contains('+') {
                return self.complex_spec(dex, first, body, raw);
            }
            return Ok(Spec::Text(format!("{first}{}", ban(dex, body)?)));
        }
        // `rule.split('=')`: the name, and the value up to a second `=` if any.
        let mut parts = raw.split('=');
        let name = parts.next().unwrap_or_default();
        let value = parts.next().map(str::trim);
        let mut id = self.entry(dex, name)?.id.clone();
        if let Some(value) = value {
            id = format!("{id}={value}");
        }
        let prefix = if raw.starts_with("^!") {
            "^!"
        } else if raw.starts_with('^') {
            "^"
        } else if raw.starts_with("!!") {
            "!!"
        } else if raw.starts_with('!') {
            "!"
        } else {
            ""
        };
        Ok(Spec::Text(format!("{prefix}{id}")))
    }

    fn complex_spec(
        &self,
        dex: &Dex<'_>,
        first: char,
        body: &str,
        raw: &str,
    ) -> Result<Spec, FormatError> {
        let mut buf = body.to_string();
        let mut limit = if first == '+' { None } else { Some(0) };
        if let Some(at) = buf.rfind('>') {
            let count = buf[at + 1..].trim();
            if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()) {
                if limit == Some(0) {
                    limit = Some(count.parse::<u32>().unwrap_or(u32::MAX));
                }
                buf.truncate(at);
            }
        }
        let mut team = buf.contains("++");
        let names: Vec<String> = buf
            .split(if team { "++" } else { "+" })
            .map(|name| name.trim().to_string())
            .collect();
        if names.len() == 1 && limit != Some(0) {
            team = true;
        }
        let rule = names.join(if team { " ++ " } else { " + " });
        let bans = names
            .iter()
            .map(|name| ban(dex, name))
            .collect::<Result<Vec<_>, _>>()?;
        if team || bans.len() > 1 || limit != Some(0) {
            return Ok(Spec::Complex {
                team,
                rule,
                limit,
                bans,
            });
        }
        Err(FormatError::ConfusingRule(raw.to_string()))
    }

    /// Showdown `getRuleTable(format, depth, repeals)`.
    fn rule_table(
        &self,
        dex: &Dex<'_>,
        entry: &FormatEntry,
        depth: i32,
        repeals: &mut Repeals,
        custom: Option<&[String]>,
    ) -> Result<RuleTable, FormatError> {
        let mut table = RuleTable::default();
        let mut ruleset: Vec<String> = entry.ruleset.clone();
        ruleset.extend(entry.banlist.iter().map(|ban| format!("-{ban}")));
        ruleset.extend(entry.restricted.iter().map(|ban| format!("*{ban}")));
        ruleset.extend(entry.unbanlist.iter().map(|ban| format!("+{ban}")));
        if let Some(custom) = custom {
            ruleset.extend(custom.iter().cloned());
        }
        let specs = ruleset
            .iter()
            .map(|rule| self.validate_rule_under(dex, rule))
            .collect::<Result<Vec<_>, _>>()?;
        // Repeals apply before the other rules.
        for spec in &specs {
            if let Spec::Text(text) = spec {
                let text = text.strip_prefix('^').unwrap_or(text);
                if text.starts_with('!') && !text.starts_with("!!") {
                    repeals
                        .get_or_insert_with(HashMap::new)
                        .insert(text[1..].to_string(), depth);
                }
            }
        }
        let count = |wanted: &str| {
            specs
                .iter()
                .filter(|spec| matches!(spec, Spec::Text(text) if text == wanted))
                .count()
        };
        let mut skip_pokemon_bans = count("+pokemontag:allpokemon");
        let mut has_pokemon_bans = false;
        let warn_for_no_pokemon_bans = skip_pokemon_bans > 0 && custom.is_none();
        skip_pokemon_bans += count("-pokemontag:allpokemon");

        for spec in specs {
            let text = match spec {
                Spec::Complex {
                    team,
                    rule,
                    limit,
                    bans,
                } => {
                    table.add_complex_ban(
                        team,
                        ComplexBan {
                            rule,
                            source: String::new(),
                            limit,
                            bans,
                        },
                    );
                    continue;
                }
                Spec::Text(text) => text,
            };
            let no_warn = text.starts_with('^');
            let text = text.strip_prefix('^').unwrap_or(&text).to_string();
            // A repeal.
            if text.starts_with('!') && !text.starts_with("!!") {
                let key = &text[1..];
                let at = repeals.as_ref().and_then(|map| map.get(key)).copied();
                match at {
                    None => return Err(FormatError::MultipleRepeals(text)),
                    Some(at) if at == depth && !no_warn => {
                        return Err(FormatError::RepealDidNothing(text));
                    }
                    Some(at) if at == -depth => {
                        if let Some(map) = repeals.as_mut() {
                            map.remove(key);
                        }
                    }
                    Some(_) => {}
                }
                continue;
            }
            // A ban, unban or restriction.
            let first = text.chars().next().unwrap_or(' ');
            if "+*-".contains(first) {
                if table.has(&text) {
                    return Err(FormatError::AlreadyExists(text));
                }
                if skip_pokemon_bans > 0 {
                    if text == "-pokemontag:allpokemon" || text == "+pokemontag:allpokemon" {
                        skip_pokemon_bans -= 1;
                    } else if is_pokemon_rule(&text) {
                        if custom.is_none() {
                            return Err(FormatError::PokemonRuleBeforeAllPokemon(text));
                        }
                        continue;
                    }
                }
                for prefix in ['+', '*', '-'] {
                    table.entries.delete(&format!("{prefix}{}", &text[1..]));
                }
                table.entries.set(&text, "");
                continue;
            }
            // A rule.
            let mut parts = text.split('=');
            let format_id = parts.next().unwrap_or_default();
            let mut value = parts.next().map(str::to_string);
            let sub = self.entry(dex, format_id)?;
            let repeal_and_replace = text.starts_with("!!");
            if let Some(map) = repeals.as_mut()
                && let Some(at) = map.get_mut(&sub.id)
            {
                *at = -at.abs();
                continue;
            }
            if let Some(kind) = sub.has_value {
                let Some(given) = value.as_mut() else {
                    return Err(FormatError::MissingValue(text));
                };
                if given == "Current Gen" {
                    *given = dex.generation.to_string();
                }
                let auto = (sub.id == "pickedteamsize" || sub.id == "evlimit") && given == "Auto";
                if !auto && matches!(kind, HasValue::Integer | HasValue::PositiveInteger) {
                    match given.parse::<i64>() {
                        Ok(number) if number.to_string() == *given => {
                            if kind == HasValue::PositiveInteger && number <= 0 {
                                return Err(FormatError::NotPositive(text));
                            }
                        }
                        _ => return Err(FormatError::NotAnInteger(text)),
                    }
                }
                let given = given.clone();
                let old = table.value_rules.get(&sub.id).map(str::to_string);
                if old.as_deref() == Some(given.as_str()) {
                    if !no_warn {
                        return Err(FormatError::Redundant(text));
                    }
                } else if repeal_and_replace {
                    if old.is_none() {
                        match sub.mutually_exclusive_with.as_deref() {
                            Some(other) if table.value_rules.has(other) => {
                                if !self.entry(dex, other)?.ruleset.is_empty() {
                                    return Err(FormatError::DoubleBangUnsupported(text));
                                }
                                table.value_rules.delete(other);
                                table.entries.delete(other);
                            }
                            _ => return Err(FormatError::NotReplacing(text)),
                        }
                    }
                } else {
                    if old.is_some() {
                        return Err(FormatError::Conflicts(text));
                    }
                    if let Some(other) = sub.mutually_exclusive_with.as_deref()
                        && table.value_rules.has(other)
                    {
                        return Err(FormatError::MutuallyExclusive(text));
                    }
                }
                table.value_rules.set(&sub.id, &given);
            } else {
                if value.is_some() {
                    return Err(FormatError::UnexpectedValue(text));
                }
                if repeal_and_replace {
                    return Err(FormatError::DoubleBangUnsupported(text));
                }
                if table.has(&sub.id) && !no_warn {
                    return Err(FormatError::AlreadyExists(text));
                }
            }
            table.entries.set(&sub.id, "");
            if depth > 16 {
                return Err(FormatError::Recursion(text));
            }
            // The sub-rule's table: it shares the repeals in force here, or starts its own.
            let sub_table = if repeals.is_some() {
                self.rule_table(dex, sub, depth + 1, repeals, None)?
            } else {
                let mut own = None;
                self.rule_table(dex, sub, depth + 1, &mut own, None)?
            };
            for (rule_id, source) in sub_table.entries.iter() {
                if repeals
                    .as_ref()
                    .is_some_and(|map| map.contains_key(rule_id))
                {
                    continue;
                }
                let first = rule_id.chars().next().unwrap_or(' ');
                if skip_pokemon_bans > 0 && "+*-".contains(first) && is_pokemon_rule(rule_id) {
                    has_pokemon_bans = true;
                    continue;
                }
                if let Some(new_value) = sub_table.value_rules.get(rule_id) {
                    let rule = self.entry(dex, rule_id)?;
                    if let Some(other) = rule.mutually_exclusive_with.as_deref()
                        && table.value_rules.has(other)
                    {
                        return Err(FormatError::MutuallyExclusive(rule_id.to_string()));
                    }
                    let old = table.value_rules.get(rule_id);
                    if old != Some(new_value) {
                        if old.is_some() {
                            return Err(FormatError::Conflicts(rule_id.to_string()));
                        }
                        table.value_rules.set(rule_id, new_value);
                    }
                }
                let source = if source.is_empty() { &sub.name } else { source };
                table.entries.set(rule_id, source);
            }
            for (team, bans) in [
                (false, &sub_table.complex_bans),
                (true, &sub_table.complex_team_bans),
            ] {
                for ban in bans {
                    let source = if ban.source.is_empty() {
                        sub.name.clone()
                    } else {
                        ban.source.clone()
                    };
                    table.add_complex_ban(
                        team,
                        ComplexBan {
                            source,
                            ..ban.clone()
                        },
                    );
                }
            }
        }
        if !has_pokemon_bans && warn_for_no_pokemon_bans {
            return Err(FormatError::AllPokemonWithoutBans);
        }
        // Every level ends the same way, a rule's own table as much as the format's.
        table.compute_tag_rules();
        let level = Level {
            dex,
            mod_name: mod_of(entry),
            game_type: entry.game_type.as_deref().unwrap_or("singles"),
            name: &entry.name,
        };
        self.resolve_numbers(&level, &mut table)?;
        // Banning Rayquaza-Mega and not Rayquaza brings the Mega Rayquaza Clause, where Mega
        // Evolution is (Gen 1-7, or Past unbanned).
        let can_mega_evolve = dex.generation <= 7 || table.has("+pokemontag:past");
        let banned = |id: &str| {
            dex.legality
                .species(id)
                .is_some_and(|species| table.is_banned_species(species))
        };
        if table.has("obtainableformes")
            && can_mega_evolve
            && banned("rayquazamega")
            && !banned("rayquaza")
        {
            table.entries.set("megarayquazaclause", "");
        }
        self.validate_rules(&level, &table)?;
        Ok(table)
    }

    /// Showdown `RuleTable.resolveNumbers`, run for every level's table. The timer's numbers
    /// decide no battle and are only checked (DESIGN.md §45.2).
    fn resolve_numbers(&self, level: &Level<'_>, table: &mut RuleTable) -> Result<(), FormatError> {
        let game_type_min: i64 = match level.game_type {
            "triples" | "rotation" => 3,
            "doubles" => 2,
            _ => 1,
        };
        let values = &table.value_rules;
        // JavaScript `Number(value)`: `None` stands for NaN (a missing or non-numeric value).
        let number = |id: &str| {
            values
                .get(id)
                .and_then(|value| value.trim().parse::<i64>().ok())
        };
        // `Number(value) || fallback`: zero falls back as NaN does.
        let or = |id: &str| number(id).filter(|value| *value != 0);
        let mut min_team_size = or("minteamsize").unwrap_or(0);
        let max_team_size = or("maxteamsize").unwrap_or(6);
        let mut picked_team_size = or("pickedteamsize");
        let max_total_level = or("maxtotallevel");
        let max_move_count = or("maxmovecount").unwrap_or(4);
        let min_source_gen = or("minsourcegen").unwrap_or(1);
        let min_level = or("minlevel").unwrap_or(1);
        let max_level = or("maxlevel").unwrap_or(100);
        let mut default_level = or("defaultlevel").unwrap_or(0);
        let adjust_level = or("adjustlevel");
        let adjust_level_down = or("adjustleveldown");
        let mut ev_limit = number("evlimit");
        if values.get("pickedteamsize") == Some("Auto") {
            picked_team_size = Some(match level.game_type {
                "doubles" | "rotation" => 4,
                "triples" => 6,
                _ => 3,
            });
        }
        if values.get("evlimit") == Some("Auto") {
            ev_limit = (level.dex.generation > 2).then_some(510);
            if level.mod_name == "gen7letsgo" {
                ev_limit = table.has("lgpenormalrules").then_some(0);
            }
            if level.mod_name.starts_with("champions") {
                ev_limit = Some(66);
            }
        }
        let invalid = |what: &str| Err(FormatError::Invalid(format!("{what} in {}", level.name)));
        if max_team_size > 24 {
            return invalid("max team size above 24");
        }
        if max_level > 99999 {
            return invalid("max level above 99999");
        }
        if max_move_count > 24 {
            return invalid("max move count above 24");
        }
        if default_level == 0 {
            let team_size = picked_team_size.unwrap_or(max_team_size);
            default_level = match max_total_level {
                Some(total) if max_level > 100 && max_level * team_size > total => 100,
                _ => max_level,
            };
        }
        if min_team_size != 0 && min_team_size < game_type_min {
            return invalid("min team size below the game type's");
        }
        if picked_team_size.is_some_and(|picked| picked < game_type_min) {
            return invalid("picked team size below the game type's");
        }
        if min_team_size != 0 && picked_team_size.is_some_and(|picked| min_team_size < picked) {
            return invalid("min team size below picked team size");
        }
        if min_team_size == 0 {
            min_team_size = game_type_min.max(picked_team_size.unwrap_or(0));
        }
        if max_team_size < game_type_min {
            return invalid("max team size below the game type's");
        }
        if max_team_size < min_team_size {
            return invalid("max team size below min team size");
        }
        if min_level > max_level {
            return invalid("min level above max level");
        }
        if default_level > max_level {
            return invalid("default level above max level");
        }
        if default_level < min_level {
            return invalid("default level below min level");
        }
        if adjust_level_down.is_some_and(|down| down >= max_level) {
            return invalid("adjust level down not below max level");
        }
        if adjust_level.is_some() && values.has("minlevel") {
            return invalid("min level with adjust level");
        }
        if ev_limit.is_some_and(|limit| limit >= 1512) {
            return invalid("EV limit of 1512 or more");
        }
        if ev_limit.is_some_and(|limit| limit < 0) {
            return invalid("EV limit below 0");
        }
        let timer = |id: &str| values.has(id).then(|| number(id));
        let outside = |value: Option<Option<i64>>| {
            // A NaN compares false both ways, as in JavaScript.
            matches!(value, Some(Some(value)) if !(10..=1200).contains(&value))
        };
        if outside(timer("timerstarting")) {
            return invalid("timer starting value outside 10..=1200");
        }
        if timer("timergrace")
            .flatten()
            .is_some_and(|grace| grace > 300)
        {
            return invalid("timer grace above 300");
        }
        if timer("timeraddperturn")
            .flatten()
            .is_some_and(|add| add > 30)
        {
            return invalid("timer add per turn above 30");
        }
        if outside(timer("timermaxperturn")) {
            return invalid("timer max per turn outside 10..=1200");
        }
        if outside(timer("timermaxfirstturn")) {
            return invalid("timer max first turn outside 10..=1200");
        }
        // Every number left is non-negative and bounded by the checks above.
        let narrow = |value: i64| value.clamp(0, i64::from(u32::MAX)) as u32;
        table.numbers = RuleNumbers {
            min_team_size: narrow(min_team_size),
            max_team_size: narrow(max_team_size),
            picked_team_size: picked_team_size.map(narrow),
            max_total_level: max_total_level.map(narrow),
            max_move_count: narrow(max_move_count),
            min_source_gen: narrow(min_source_gen),
            min_level: narrow(min_level),
            max_level: narrow(max_level),
            default_level: narrow(default_level),
            adjust_level: adjust_level.map(narrow),
            adjust_level_down: adjust_level_down.map(narrow),
            ev_limit: ev_limit.map(narrow),
        };
        Ok(())
    }

    /// The rules' own `onValidateRule` checks, for the four rules the target formats reach
    /// that have one (none of them changes its value: Item Clause returns it as it was).
    fn validate_rules(&self, level: &Level<'_>, table: &RuleTable) -> Result<(), FormatError> {
        let previewed = table.has("teampreview") || table.has("teamtypepreview");
        let invalid = |what: String| Err(FormatError::Invalid(format!("{what} in {}", level.name)));
        for (rule, _) in table.entries.iter() {
            // A rule's own check, where its mod's rule has one.
            if "+*-!".contains(rule.chars().next().unwrap_or(' '))
                || !self
                    .library
                    .formats()
                    .entry(level.dex.mod_name, rule)
                    .is_some_and(|entry| entry.hooks.iter().any(|h| h == "onValidateRule"))
            {
                continue;
            }
            let value = table.value_rules.get(rule);
            // `Number(value)` / `parseInt(value)`; `None` is NaN, which fails no comparison.
            let number = value.and_then(|value| value.trim().parse::<i64>().ok());
            match rule {
                "itemclause" => {
                    let max = i64::from(table.numbers.max_team_size);
                    if number.is_some_and(|count| count < 1 || count > max) {
                        return invalid(format!("item clause outside 1..={max}"));
                    }
                }
                "minsourcegen" => {
                    if number.is_some_and(|source| source > i64::from(level.dex.generation)) {
                        return invalid("min source gen above the generation".into());
                    }
                }
                "openteamsheets" | "pickedteamsize" if !previewed => {
                    return invalid(format!("{rule} without Team Preview"));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// The dex a format resolves under: its generation's, with the names it knows.
struct Dex<'a> {
    mod_name: &'a str,
    generation: u32,
    legality: &'a LegalityData,
    aliases: &'a Aliases,
}

/// One level of the recursion: the format, or a rule, whose table is being built.
struct Level<'a> {
    dex: &'a Dex<'a>,
    /// The entry's own mod (a rule's is the base mod).
    mod_name: &'a str,
    game_type: &'a str,
    name: &'a str,
}

fn mod_of(entry: &FormatEntry) -> &str {
    entry.mod_name.as_deref().unwrap_or(BASE_MOD)
}

/// Showdown `isPokemonRule`.
fn is_pokemon_rule(spec: &str) -> bool {
    let body = &spec[1..];
    body.starts_with("pokemontag:")
        || body.starts_with("pokemon:")
        || body.starts_with("basepokemon:")
}

/// Showdown `validateBanRule` under the dex.
fn ban(dex: &Dex<'_>, name: &str) -> Result<String, FormatError> {
    dex.legality
        .ban_spec(dex.aliases, name)
        .map_err(|error| match error {
            BanError::Nothing => FormatError::UnresolvedBan(name.to_string()),
            BanError::Ambiguous(specs) => FormatError::AmbiguousBan(name.to_string(), specs),
        })
}
