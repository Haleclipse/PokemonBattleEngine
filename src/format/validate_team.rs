//! A team's validation (M9.2d, DESIGN.md §45.3): Showdown's `baseValidateTeam` — the team's
//! size, each set (`validate_set`), the two hard-coded team rules (one Gen 3 Deoxys forme, one
//! Let's Go starter), the team's combination bans and the rules' `onValidateTeam` — on the
//! learnset seam's empty side, like the sets.

use super::data::to_id;
use super::legality::{Named, SpeciesFacts, TAGS};
use super::set::TeamSet;
use super::table::RuleTable;
use super::validate::{Problem, ProblemKind, TeamValidator};
use super::validate_set::TeamHas;

impl RuleTable {
    /// Showdown `isRestrictedSpecies`: the species' own unban or restriction first, then its base
    /// species', then a tag's restriction, a tag's unban, and All Pokemon restricted.
    pub fn is_restricted_species(&self, species: &SpeciesFacts) -> bool {
        if self.has(&format!("+pokemon:{}", species.id)) {
            return false;
        }
        if self.has(&format!("*pokemon:{}", species.id)) {
            return true;
        }
        let base = to_id(&species.base_species);
        if self.has(&format!("+basepokemon:{base}")) {
            return false;
        }
        if self.has(&format!("*basepokemon:{base}")) {
            return true;
        }
        let tagged = |sign: char| {
            TAGS.iter().any(|tag| {
                self.has(&format!("{sign}pokemontag:{}", tag.id)) && tag.matches(species)
            })
        };
        if tagged('*') {
            return true;
        }
        if tagged('+') {
            return false;
        }
        self.has("*pokemontag:allpokemon")
    }
}

/// Baton Pass Stat Clause's ways to boost.
const BATON_PASS_STAT: [&str; 68] = [
    "absorbbulb",
    "acidarmor",
    "acupressure",
    "agility",
    "amnesia",
    "ancientpower",
    "angerpoint",
    "apicotberry",
    "autotomize",
    "barrier",
    "bellydrum",
    "bulkup",
    "calmmind",
    "cellbattery",
    "charge",
    "chargebeam",
    "coil",
    "cosmicpower",
    "cottonguard",
    "curse",
    "defensecurl",
    "defendorder",
    "defiant",
    "download",
    "dragondance",
    "fierydance",
    "flamecharge",
    "focusenergy",
    "ganlonberry",
    "growth",
    "harden",
    "honeclaws",
    "howl",
    "irondefense",
    "justified",
    "lansatberry",
    "liechiberry",
    "lightningrod",
    "meditate",
    "metalclaw",
    "meteormash",
    "motordrive",
    "moxie",
    "nastyplot",
    "ominouswind",
    "petayaberry",
    "quiverdance",
    "rage",
    "rattled",
    "rockpolish",
    "salacberry",
    "sapsipper",
    "sharpen",
    "shellsmash",
    "shiftgear",
    "silverwind",
    "skullbash",
    "speedboost",
    "starfberry",
    "steadfast",
    "steelwing",
    "stockpile",
    "stormdrain",
    "swordsdance",
    "tailglow",
    "weakarmor",
    "withdraw",
    "workup",
];

/// Baton Pass Stat Trap Clause's ways to boost or to trap (by name; Showdown makes them ids).
const BATON_PASS_STAT_TRAP: [&str; 60] = [
    "Acid Armor",
    "Acupressure",
    "Agility",
    "Amnesia",
    "Ancient Power",
    "Assist",
    "Barrier",
    "Belly Drum",
    "Block",
    "Bulk Up",
    "Calm Mind",
    "Charge",
    "Charge Beam",
    "Cosmic Power",
    "Curse",
    "Defend Order",
    "Defense Curl",
    "Dragon Dance",
    "Growth",
    "Guard Swap",
    "Harden",
    "Heart Swap",
    "Howl",
    "Iron Defense",
    "Ingrain",
    "Mean Look",
    "Meteor Mash",
    "Meditate",
    "Metal Claw",
    "Nasty Plot",
    "Ominous Wind",
    "Power Trick",
    "Psych Up",
    "Rage",
    "Rock Polish",
    "Sharpen",
    "Silver Wind",
    "Skull Bash",
    "Spider Web",
    "Steel Wing",
    "Stockpile",
    "Swords Dance",
    "Tail Glow",
    "Withdraw",
    "Speed Boost",
    "Apicot Berry",
    "Ganlon Berry",
    "Liechi Berry",
    "Petaya Berry",
    "Salac Berry",
    "Starf Berry",
    "Kee Berry",
    "Maranga Berry",
    "Weakness Policy",
    "Blunder Policy",
    "Luminiscent Moss",
    "Snowball",
    "Throat Spray",
    "Mirror Herb",
    "Adrenaline Orb",
];

/// One Boost Passer Clause's ways to boost.
const ONE_BOOST_PASSER: [&str; 27] = [
    "acidarmor",
    "agility",
    "amnesia",
    "apicotberry",
    "barrier",
    "bellydrum",
    "bulkup",
    "calmmind",
    "cosmicpower",
    "curse",
    "defensecurl",
    "dragondance",
    "ganlonberry",
    "growth",
    "harden",
    "howl",
    "irondefense",
    "liechiberry",
    "meditate",
    "petayaberry",
    "salacberry",
    "sharpen",
    "speedboost",
    "starfberry",
    "swordsdance",
    "tailglow",
    "withdraw",
];

/// Speed Pass Clause's ways to boost Speed.
const SPEED_PASS: [&str; 7] = [
    "agility",
    "dragondance",
    "ancientpower",
    "silverwind",
    "salacberry",
    "speedboost",
    "starfberry",
];

fn problem(kind: ProblemKind, message: String) -> Problem {
    Problem::new(kind, message)
}

fn display(set: &TeamSet) -> &str {
    if set.name.is_empty() {
        &set.species
    } else {
        &set.name
    }
}

impl TeamValidator<'_> {
    /// Showdown `baseValidateTeam(team)` on the learnset seam's empty side. Each set comes back
    /// normalised (`validate_set`); the problems are the sets', then the team's.
    pub fn validate_team(&self, team: &mut [TeamSet]) -> Vec<Problem> {
        use ProblemKind::*;
        let table = &self.table;
        let numbers = table.numbers;
        let mut problems = Vec::new();
        let size = team.len() as u32;
        if size < numbers.min_team_size {
            problems.push(
                problem(
                    Team,
                    format!(
                        "You must bring at least {} Pokémon (your team has {size}).",
                        numbers.min_team_size
                    ),
                )
                .by("minteamsize"),
            );
        }
        if size > numbers.max_team_size {
            return vec![
                problem(
                    Team,
                    format!(
                        "You may only bring up to {} Pokémon (your team has {size}).",
                        numbers.max_team_size
                    ),
                )
                .by("maxteamsize"),
            ];
        }
        let mut team_has = TeamHas::new();
        let mut starters = 0;
        let mut deoxys: Option<String> = None;
        for (index, set) in team.iter_mut().enumerate() {
            let mut set_problems = self.validate_set(set, Some(&mut team_has));
            for problem in &mut set_problems {
                problem.location.set = Some(index);
            }
            if set.species == "Pikachu-Starter" || set.species == "Eevee-Starter" {
                starters += 1;
                if starters == 2 && table.is_banned("nonexistent") {
                    problems.push(
                        problem(
                            Team,
                            "You can only have one of Pikachu-Starter or Eevee-Starter on a team."
                                .into(),
                        )
                        .by("-nonexistent"),
                    );
                }
            }
            if self.generation == 3 && set.species.starts_with("Deoxys") {
                match &deoxys {
                    None => deoxys = Some(set.species.clone()),
                    Some(first) if *first != set.species && table.is_banned("nonexistent") => {
                        return vec![
                            problem(
                                Team,
                                "You cannot have more than one type of Deoxys forme.".into(),
                            )
                            .by("-nonexistent"),
                            problem(
                                Team,
                                "(Each game in Gen 3 supports only one forme of Deoxys.)".into(),
                            )
                            .by("-nonexistent"),
                        ];
                    }
                    Some(_) => {}
                }
            }
            problems.extend(set_problems);
        }
        for ban in &table.complex_team_bans {
            let mut count = 0u32;
            for thing in &ban.bans {
                if let Some(&has) = team_has.get(thing).filter(|has| **has > 0) {
                    count += if ban.limit == Some(0) { 1 } else { has };
                }
            }
            let clause = if ban.source.is_empty() {
                String::new()
            } else {
                format!(" by {}", ban.source)
            };
            match ban.limit {
                Some(limit) if limit > 0 && count > limit => problems.push(
                    problem(
                        Combination,
                        format!("You are limited to {limit} of {}{clause}.", ban.rule),
                    )
                    .by(ban.rule.clone()),
                ),
                Some(0) if count as usize >= ban.bans.len() => problems.push(
                    problem(
                        Combination,
                        format!(
                            "Your team has the combination of {}, which is banned{clause}.",
                            ban.rule
                        ),
                    )
                    .by(ban.rule.clone()),
                ),
                _ => {}
            }
        }
        // The rules' `onValidateTeam`, in the table's order.
        for (rule, _) in table.entries.iter() {
            if self.runs(rule, "onValidateTeam") {
                problems.extend(
                    self.team_rule(rule, team)
                        .into_iter()
                        .map(|problem| problem.by(rule)),
                );
            }
        }
        problems
    }

    /// One rule's `onValidateTeam` (none for a rule without one).
    fn team_rule(&self, rule: &str, team: &[TeamSet]) -> Vec<Problem> {
        use ProblemKind::Clause;
        let clause = |messages: &[&str]| -> Vec<Problem> {
            messages
                .iter()
                .map(|m| problem(Clause, (*m).to_string()))
                .collect()
        };
        match rule {
            "obtainable" => {
                // One of each forme a key item makes of one Pokémon.
                const FUSED: [(&[&str], &str, &str); 4] = [
                    (
                        &["Kyurem-White", "Kyurem-Black"],
                        "You cannot have more than one Kyurem-Black/Kyurem-White.",
                        "(It's untradeable and you can only make one with the DNA Splicers.)",
                    ),
                    (
                        &["Necrozma-Dusk-Mane"],
                        "You cannot have more than one Necrozma-Dusk-Mane",
                        "(It's untradeable and you can only make one with the N-Solarizer.)",
                    ),
                    (
                        &["Necrozma-Dawn-Wings"],
                        "You cannot have more than one Necrozma-Dawn-Wings",
                        "(It's untradeable and you can only make one with the N-Lunarizer.)",
                    ),
                    (
                        &["Calyrex-Ice", "Calyrex-Shadow"],
                        "You cannot have more than one Calyrex-Ice/Calyrex-Shadow.",
                        "(It's untradeable and you can only make one with the Reins of Unity.)",
                    ),
                ];
                let mut counts = [0; 4];
                for set in team {
                    for (index, (formes, first, second)) in FUSED.iter().enumerate() {
                        if formes.contains(&set.species.as_str()) {
                            if counts[index] > 0 {
                                return clause(&[first, second]);
                            }
                            counts[index] += 1;
                        }
                    }
                }
                Vec::new()
            }
            "speciesclause" => {
                let mut seen = Vec::new();
                for set in team {
                    let species = self.species(&set.species);
                    if seen.contains(&species.num) {
                        return vec![
                            problem(
                                Clause,
                                "You are limited to one of each Pokémon by Species Clause.".into(),
                            ),
                            problem(
                                Clause,
                                format!("(You have more than one {})", species.base_species),
                            ),
                        ];
                    }
                    seen.push(species.num);
                }
                Vec::new()
            }
            "nicknameclause" => {
                let mut seen: Vec<&str> = Vec::new();
                for set in team {
                    let name = set.name.as_str();
                    if name.is_empty() || name == self.species(&set.species).base_species {
                        continue;
                    }
                    if seen.contains(&name) {
                        return vec![
                            problem(Clause, "Your Pokémon must have different nicknames.".into()),
                            problem(Clause, format!("(You have more than one {name})")),
                        ];
                    }
                    seen.push(name);
                }
                Vec::new()
            }
            "gravitysleepclause" => {
                let mut orbeetle = false;
                let mut sleep = false;
                for set in team {
                    let species = self.species(&set.species);
                    if species.name == "Orbeetle" && set.gigantamax {
                        orbeetle = true;
                    }
                    if !orbeetle && species.name == "Orbeetle-Gmax" {
                        orbeetle = true;
                    }
                    for move_name in &set.moves {
                        if let Named::Found(the_move) =
                            self.legality.move_named(self.aliases, move_name)
                        {
                            // Showdown compares `true < 100`, which is true: a move that never
                            // misses counts, one of 100 % does not.
                            let misses = match &the_move.accuracy {
                                Some(serde_json::Value::Bool(true)) => true,
                                Some(serde_json::Value::Number(n)) => {
                                    n.as_f64().is_some_and(|a| a < 100.0)
                                }
                                _ => false,
                            };
                            if the_move.status.as_deref() == Some("slp") && misses {
                                sleep = true;
                            }
                        }
                    }
                }
                if orbeetle && sleep {
                    return clause(&[
                        "The combination of Gravity and Gigantamax Orbeetle on the same team is \
                         banned.",
                    ]);
                }
                Vec::new()
            }
            "itemclause" => {
                // A multiset in the order the items come.
                let mut items: Vec<(String, u32)> = Vec::new();
                for set in team {
                    let item = to_id(&set.item);
                    if item.is_empty() {
                        continue;
                    }
                    match items.iter_mut().find(|(id, _)| *id == item) {
                        Some((_, count)) => *count += 1,
                        None => items.push((item, 1)),
                    }
                }
                let limit: u32 = self
                    .table
                    .value_rules
                    .get("itemclause")
                    .filter(|v| !v.is_empty())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1);
                for (id, count) in items {
                    if count <= limit {
                        continue;
                    }
                    let name = self.item(&id).name.clone();
                    return vec![
                        problem(
                            Clause,
                            format!("You are limited to {limit} of each item by Item Clause."),
                        ),
                        problem(Clause, format!("(You have more than {limit} {name})")),
                    ];
                }
                Vec::new()
            }
            "limitonerestricted" | "limittworestricted" => {
                let restricted: Vec<String> = team
                    .iter()
                    .map(|set| self.species(&set.species))
                    .filter(|species| self.table.is_restricted_species(species))
                    .map(|species| species.name.clone())
                    .collect();
                let (limit, message) = if rule == "limitonerestricted" {
                    (1, "You can only use one restricted Pok\u{e9}mon")
                } else {
                    (2, "You can only use up to two restricted Pok\u{e9}mon")
                };
                if restricted.len() > limit {
                    return vec![problem(
                        Clause,
                        format!("{message} (you have: {})", restricted.join(", ")),
                    )];
                }
                Vec::new()
            }
            "batonpassstatclause" | "speedpassclause" => {
                let (effects, banned): (&[&str], &str) = if rule == "batonpassstatclause" {
                    (
                        &BATON_PASS_STAT,
                        "a way to boost its stats, which is banned by Baton Pass Stat Clause",
                    )
                } else {
                    (
                        &SPEED_PASS,
                        "a way to boost its Speed, which is banned by Speed Pass Clause",
                    )
                };
                for set in team {
                    let moves: Vec<String> = set.moves.iter().map(|m| to_id(m)).collect();
                    if !moves.iter().any(|m| m == "batonpass") {
                        continue;
                    }
                    let item = to_id(&set.item);
                    let ability = to_id(&set.ability);
                    if moves.iter().any(|m| effects.contains(&m.as_str()))
                        || effects.contains(&item.as_str())
                        || effects.contains(&ability.as_str())
                    {
                        return vec![problem(
                            Clause,
                            format!("{} has Baton Pass and {banned}.", display(set)),
                        )];
                    }
                }
                Vec::new()
            }
            "batonpassstattrapclause" => {
                let effects: Vec<String> = BATON_PASS_STAT_TRAP
                    .iter()
                    .map(|name| to_id(name))
                    .collect();
                for set in team {
                    if !set.moves.iter().any(|m| to_id(m) == "batonpass") {
                        continue;
                    }
                    let item = to_id(&set.item);
                    let ability = to_id(&set.ability);
                    let passable = set.moves.iter().any(|m| effects.contains(&to_id(m)))
                        || effects.contains(&item)
                        || effects.contains(&ability);
                    if passable {
                        return vec![problem(
                            Clause,
                            format!(
                                "{} has Baton Pass and a way to boost its stats or pass trapping, \
                                 which is banned by Baton Pass Stat Trap Clause.",
                                display(set)
                            ),
                        )];
                    }
                }
                Vec::new()
            }
            "oneboostpasserclause" => {
                let mut passers = 0;
                for set in team {
                    // The move as written: `batonpass` is not `Baton Pass` here.
                    if !set.moves.iter().any(|m| m == "Baton Pass") {
                        continue;
                    }
                    let item = to_id(&set.item);
                    let ability = to_id(&set.ability);
                    let mut boosts = set
                        .moves
                        .iter()
                        .filter(|m| ONE_BOOST_PASSER.contains(&to_id(m).as_str()))
                        .count();
                    if ONE_BOOST_PASSER.contains(&item.as_str()) {
                        boosts += 1;
                    }
                    if ONE_BOOST_PASSER.contains(&ability.as_str()) {
                        boosts += 1;
                    }
                    if boosts == 1 {
                        passers += 1;
                    }
                    if boosts > 1 {
                        return vec![problem(
                            Clause,
                            format!(
                                "{} has Baton Pass and multiple ways to boost its stats, which is \
                                 banned by One Boost Passer Clause.",
                                display(set)
                            ),
                        )];
                    }
                    if passers > 1 {
                        return clause(&[
                            "Multiple Pokemon have Baton Pass and a way to boost their stats, \
                             which is banned by One Boost Passer Clause.",
                        ]);
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}
