//! A resolved rule table (Showdown `RuleTable`): every rule and ban in effect, with where it came
//! from, in the order Showdown's `Map` keeps them, plus the values and numbers resolved from them.

/// A ban on a combination (Showdown `complexBans` / `complexTeamBans`): `bans` together on one
/// Pokémon (or across the team), more than `limit` of them. `None` is no limit (an unban, `+`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexBan {
    pub rule: String,
    pub source: String,
    pub limit: Option<u32>,
    pub bans: Vec<String>,
}

/// The numbers Showdown resolves from the rules (`resolveNumbers`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuleNumbers {
    pub min_team_size: u32,
    pub max_team_size: u32,
    pub picked_team_size: Option<u32>,
    pub max_total_level: Option<u32>,
    pub max_move_count: u32,
    pub min_source_gen: u32,
    pub min_level: u32,
    pub max_level: u32,
    pub default_level: u32,
    pub adjust_level: Option<u32>,
    pub adjust_level_down: Option<u32>,
    pub ev_limit: Option<u32>,
}

/// A map that keeps insertion order the way a JavaScript `Map` does: setting an existing key
/// keeps its place, deleting it and setting it again puts it last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderedMap {
    entries: Vec<(String, String)>,
}

impl OrderedMap {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    pub fn has(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }
    pub fn set(&mut self, key: &str, value: &str) {
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value.to_string(),
            None => self.entries.push((key.to_string(), value.to_string())),
        }
    }
    pub fn delete(&mut self, key: &str) {
        self.entries.retain(|(k, _)| k != key);
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A format's resolved rule table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleTable {
    /// Every rule id and ban spec in effect (`-pokemon:greninjabond`), with the rule that brought
    /// it (empty: the format itself).
    pub entries: OrderedMap,
    /// The values the value rules took (`adjustlevel` → `50`).
    pub value_rules: OrderedMap,
    pub complex_bans: Vec<ComplexBan>,
    pub complex_team_bans: Vec<ComplexBan>,
    /// The `+` / `*` / `-` tag rules, last added first (Showdown `getTagRules`).
    pub tag_rules: Vec<String>,
    pub numbers: RuleNumbers,
}

impl RuleTable {
    pub fn has(&self, key: &str) -> bool {
        self.entries.has(key)
    }

    /// The table's identity (DESIGN.md §45.2): two tables with one fingerprint are the same
    /// rules. It covers what the rules are, not where they came from (the sources), and keeps the
    /// tag rules' order, which decides between them (the later rule wins). A host keeps it across
    /// builds (a session's snapshot, §45.5), so it is the SHA-256 of a canonical text written out
    /// field by field, never of a `Debug` rendering.
    pub fn fingerprint(&self) -> String {
        let mut lines = Vec::new();
        let mut keys: Vec<&str> = self.entries.iter().map(|(key, _)| key).collect();
        keys.sort_unstable();
        lines.extend(keys.into_iter().map(|key| format!("rule {key}")));
        let mut values: Vec<String> = self
            .value_rules
            .iter()
            .map(|(key, value)| format!("value {key}={value}"))
            .collect();
        values.sort_unstable();
        lines.extend(values);
        lines.extend(self.tag_rules.iter().map(|tag| format!("tag {tag}")));
        for (kind, bans) in [
            ("ban", &self.complex_bans),
            ("teamban", &self.complex_team_bans),
        ] {
            let mut out: Vec<String> = bans
                .iter()
                .map(|ban| {
                    let limit = ban.limit.map_or("-".to_string(), |limit| limit.to_string());
                    format!(
                        "{kind} {}>{limit}:{}",
                        super::data::to_id(&ban.rule),
                        ban.bans.join("+")
                    )
                })
                .collect();
            out.sort_unstable();
            lines.extend(out);
        }
        // Every number by name: a new one cannot be left out (the pattern names them all).
        let RuleNumbers {
            min_team_size,
            max_team_size,
            picked_team_size,
            max_total_level,
            max_move_count,
            min_source_gen,
            min_level,
            max_level,
            default_level,
            adjust_level,
            adjust_level_down,
            ev_limit,
        } = self.numbers;
        let given = |value: Option<u32>| value.map_or("-".to_string(), |value| value.to_string());
        lines.push(format!(
            "numbers min_team_size={min_team_size} max_team_size={max_team_size} \
             picked_team_size={} max_total_level={} max_move_count={max_move_count} \
             min_source_gen={min_source_gen} min_level={min_level} max_level={max_level} \
             default_level={default_level} adjust_level={} adjust_level_down={} ev_limit={}",
            given(picked_team_size),
            given(max_total_level),
            given(adjust_level),
            given(adjust_level_down),
            given(ev_limit),
        ));
        format!(
            "format:{}",
            crate::digest::sha256_hex(lines.join("\n").as_bytes())
        )
    }

    /// Showdown `isBannedSpecies`: the species' own ban or unban first, then its base
    /// species', then a tag's ban, a tag's unban, and All Pokemon.
    pub fn is_banned_species(&self, species: &super::legality::SpeciesFacts) -> bool {
        if self.has(&format!("+pokemon:{}", species.id)) {
            return false;
        }
        if self.has(&format!("-pokemon:{}", species.id)) {
            return true;
        }
        let base = super::data::to_id(&species.base_species);
        if self.has(&format!("+basepokemon:{base}")) {
            return false;
        }
        if self.has(&format!("-basepokemon:{base}")) {
            return true;
        }
        let tagged = |sign: char| {
            super::legality::TAGS.iter().any(|tag| {
                self.has(&format!("{sign}pokemontag:{}", tag.id)) && tag.matches(species)
            })
        };
        if tagged('-') {
            return true;
        }
        if tagged('+') {
            return false;
        }
        self.has("-pokemontag:allpokemon")
    }

    pub(super) fn add_complex_ban(&mut self, team: bool, ban: ComplexBan) {
        let list = if team {
            &mut self.complex_team_bans
        } else {
            &mut self.complex_bans
        };
        let id = super::data::to_id(&ban.rule);
        match list
            .iter_mut()
            .find(|existing| super::data::to_id(&existing.rule) == id)
        {
            // An unban already in place (no limit) stays.
            Some(existing) if existing.limit.is_none() => {}
            Some(existing) => *existing = ban,
            None => list.push(ban),
        }
    }

    pub(super) fn compute_tag_rules(&mut self) {
        let mut tags = Vec::new();
        for (key, _) in self.entries.iter() {
            let first = key.chars().next().unwrap_or(' ');
            if !"+*-".contains(first) {
                continue;
            }
            if let Some(tag) = key[1..].strip_prefix("pokemontag:") {
                if !matches!(
                    tag,
                    "allpokemon" | "allitems" | "allmoves" | "allabilities" | "allnatures"
                ) {
                    tags.push(key.to_string());
                }
            } else if &key[1..] == "nonexistent" {
                tags.push(format!("{first}pokemontag:nonexistent"));
            }
        }
        tags.reverse();
        self.tag_rules = tags;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ban(rule: &str, source: &str, limit: Option<u32>) -> ComplexBan {
        ComplexBan {
            rule: rule.into(),
            source: source.into(),
            limit,
            bans: vec![],
        }
    }

    // No target format unbans a combination, so the fixture cannot show this: a combination's
    // unban (`+A + B`, no limit) is not replaced by a ban of it met later, a ban is (Showdown
    // `addComplexBan`), and the rule is matched by id.
    #[test]
    fn a_combination_unban_stays_and_a_ban_is_replaced() {
        let mut table = RuleTable::default();
        table.add_complex_ban(false, ban("Baton Pass + Speed Boost", "", None));
        table.add_complex_ban(false, ban("baton pass+speed boost", "Later", Some(0)));
        assert_eq!(
            table.complex_bans,
            [ban("Baton Pass + Speed Boost", "", None)]
        );
        table.add_complex_ban(true, ban("Drizzle ++ Swift Swim", "", Some(0)));
        table.add_complex_ban(true, ban("Drizzle ++ Swift Swim", "Later", Some(1)));
        assert_eq!(
            table.complex_team_bans,
            [ban("Drizzle ++ Swift Swim", "Later", Some(1))]
        );
    }
}
