//! A set's validation (M9.2c-3, DESIGN.md §45.3): Showdown's `validateSet`, assembled from the
//! rulings (M9.2b), the forme (M9.2c-1) and the stats (M9.2c-2), on the learnset seam's empty
//! side — every move learnable, no species event-only, no Pokémon GO origin, no Gen 1
//! encounters. The set comes back normalised as Showdown's does: the one a battle is built from.

use std::collections::BTreeMap;

use super::data::to_id;
use super::legality::Named;
use super::set::{self, TeamSet};
use super::table::RuleTable;
use super::validate::{Problem, ProblemKind, SetField, SetHas, SpeciesSet, TeamValidator, locate};

/// What the team has, counted (Showdown `teamHas`): the team's combination bans and clauses
/// read it.
pub type TeamHas = BTreeMap<String, u32>;

/// The trapping abilities and moves of Accuracy Trap Clause.
const TRAPPING: [&str; 11] = [
    "arenatrap",
    "magnetpull",
    "shadowtag",
    "block",
    "meanlook",
    "spiderweb",
    "anchorshot",
    "jawlock",
    "octolock",
    "spiritshackle",
    "thousandwaves",
];

impl RuleTable {
    /// Showdown `blame(key)`: ` from <rule>`, where the key came from a rule.
    pub fn blame(&self, key: &str) -> String {
        match self.entries.get(key) {
            Some(source) if !source.is_empty() => format!(" from {source}"),
            _ => String::new(),
        }
    }

    /// Showdown `isBanned(thing)`: not unbanned, and banned.
    pub fn is_banned(&self, thing: &str) -> bool {
        !self.has(&format!("+{thing}")) && self.has(&format!("-{thing}"))
    }
}

/// Showdown `dex.getName(name)`: separators and whitespace runs made one space, trimmed, cut to
/// 18 UTF-16 units, runs of three or more combining marks and the bracket pieces
/// (U+239B-U+23B9) removed.
pub fn clean_name(name: &str) -> String {
    let separator = |c: char| matches!(c, '|' | '[' | ']' | ',' | '\u{202e}') || js_space(c);
    let mut out = String::new();
    let mut in_run = false;
    for c in name.chars() {
        if separator(c) {
            if !in_run {
                out.push(' ');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    let mut name = js_trim(&out).to_string();
    if name.encode_utf16().count() > 18 {
        let mut units = 0;
        let cut: String = name
            .chars()
            .take_while(|c| {
                units += c.len_utf16();
                units <= 18
            })
            .collect();
        name = js_trim(&cut).to_string();
    }
    // Zalgo: three or more combining marks in a row.
    let combining = |c: char| {
        matches!(c,
            '\u{0300}'..='\u{036f}' | '\u{0483}'..='\u{0489}' | '\u{0610}'..='\u{0615}'
            | '\u{064b}'..='\u{065f}' | '\u{0670}' | '\u{06d6}'..='\u{06dc}'
            | '\u{06df}'..='\u{06ed}' | '\u{0e31}' | '\u{0e34}'..='\u{0e3a}'
            | '\u{0e47}'..='\u{0e4e}')
    };
    let chars: Vec<char> = name.chars().collect();
    let mut kept = String::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && combining(chars[i]) {
            i += 1;
        }
        if i - start >= 3 {
            continue;
        }
        if i > start {
            kept.extend(&chars[start..i]);
            continue;
        }
        kept.push(chars[i]);
        i += 1;
    }
    kept.chars()
        .filter(|c| !('\u{239b}'..='\u{23b9}').contains(c))
        .collect()
}

/// JavaScript's `\s`: its WhiteSpace and LineTerminator characters.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000b}' | '\u{000c}' | '\r' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn js_trim(text: &str) -> &str {
    text.trim_matches(js_space)
}

fn problem(kind: ProblemKind, message: String) -> Problem {
    Problem::new(kind, message)
}

impl TeamValidator<'_> {
    /// Showdown `validateSet(set, teamHas)` on the learnset seam's empty side. The set comes
    /// back normalised (its species', item's, ability's and nature's names, its out-of-battle
    /// forme, its EVs and IVs, its level adjusted).
    pub fn validate_set(&self, set: &mut TeamSet, team_has: Option<&mut TeamHas>) -> Vec<Problem> {
        use ProblemKind::*;
        let table = &self.table;
        let numbers = table.numbers;
        let generation = self.generation;
        let mut problems: Vec<Problem> = Vec::new();

        let mut species = self.species(&set.species);
        set.species = species.name.clone();
        // Backwards compatibility with the old Gigantamax names.
        if set.species.to_lowercase().ends_with("-gmax") && self.format.id != "gen8megamax" {
            let base: String = set
                .species
                .chars()
                .take(set.species.chars().count() - 5)
                .collect();
            set.species = base;
            species = self.species(&set.species);
            if set.name.ends_with("-Gmax") {
                set.name = species.base_species.clone();
            }
            set.gigantamax = true;
        }
        if !set.name.is_empty() && set.name.encode_utf16().count() > 18 {
            if set.name == set.species {
                set.name = species.base_species.clone();
            } else {
                problems.push(
                    problem(
                        Nickname,
                        format!(
                            "Nickname \"{}\" too long (should be 18 characters or fewer)",
                            set.name
                        ),
                    )
                    .at(SetField::Name),
                );
            }
        }
        set.name = clean_name(&set.name);
        let mut item = self.item(&set.item);
        set.item = item.name.clone();
        let mut ability = self.ability(&set.ability);
        set.ability = ability.name.clone();
        let nature_named = self.legality.nature_named(self.aliases, &set.nature);
        let (mut nature_id, mut nature_exists, nature_name) = named_parts(&nature_named);
        set.nature = nature_name;
        if set.name.is_empty() {
            set.name = species.base_species.clone();
        }
        let mut name = set.species.clone();
        if set.species != set.name && species.base_species != set.name {
            name = format!("{} ({})", set.name, set.species);
        }

        if set.level == 0 {
            set.level = numbers.default_level;
        }
        let mut adjust_level = numbers.adjust_level;
        if let Some(down) = numbers.adjust_level_down
            && set.level >= down
        {
            adjust_level = Some(down);
        }
        if Some(set.level) == adjust_level || (set.level == 100 && numbers.max_level < 100) {
            // A level 50 set of VGC is validated at 100 (a level 50 Hydreigon though it
            // evolves at 64): levelling up never makes a Pokémon unobtainable.
            set.level = numbers.max_level;
        }
        let from = problems.len();
        if set.level < numbers.min_level {
            problems.push(problem(
                Level,
                format!(
                    "{name} (level {}) is below the minimum level of {}{}",
                    set.level,
                    numbers.min_level,
                    table.blame("minlevel")
                ),
            ));
        }
        if set.level > numbers.max_level {
            problems.push(problem(
                Level,
                format!(
                    "{name} (level {}) is above the maximum level of {}{}",
                    set.level,
                    numbers.max_level,
                    table.blame("maxlevel")
                ),
            ));
        }
        locate(&mut problems, from, SetField::Level);
        let mut set_has = SetHas::new();
        if set.evs.is_none() {
            set.evs = Some(set::Stats::filled(if numbers.ev_limit.is_none() {
                252
            } else {
                0
            }));
        }
        if set.ivs.is_none() {
            set.ivs = Some(set::Stats::filled(31));
        }
        if table.has("obtainableformes") {
            let from = problems.len();
            problems.extend(self.validate_forme(set));
            locate(&mut problems, from, SetField::Species);
            species = self.species(&set.species);
        }
        let mut sources = self.all_sources(Some(&species));
        // Obtainable Misc's `onChangeSet`: the gender a species fixes, one copy of each move.
        if self.runs("obtainablemisc", "onChangeSet") {
            let species = self.species(&set.species);
            if let Some(gender) = &species.gender {
                if set.gender != *gender {
                    set.gender = gender.clone();
                }
            } else if set.gender != "M" && set.gender != "F" {
                set.gender = String::new();
            }
            let mut seen = Vec::new();
            for move_name in &set.moves {
                let named = self.legality.move_named(self.aliases, move_name);
                let (id, _, move_display) = named_parts(&named);
                if seen.contains(&id) {
                    problems.push(
                        problem(
                            Moves,
                            format!(
                                "{} has multiple copies of {move_display}.",
                                species.base_species
                            ),
                        )
                        .at(SetField::Moves),
                    );
                    break;
                }
                seen.push(id);
            }
        }
        species = self.species(&set.species);
        item = self.item(&set.item);
        ability = self.ability(&set.ability);
        if set.gender != "M" && set.gender != "F" {
            set.gender = String::new();
        }
        if (generation <= 5 || table.has("obtainablemisc"))
            && let Some(gender) = &species.gender
        {
            set.gender = gender.clone();
        }
        let (out_of_battle, tier_species) = self.validation_species(set);
        if ability.id == "battlebond"
            && to_id(&species.base_species) == "greninja"
            && table.has("obtainablemisc")
        {
            if !set.gender.is_empty() && set.gender != "M" {
                problems.push(
                    problem(Gender, "Battle Bond Greninja must be male.".into())
                        .at(SetField::Gender),
                );
            }
            set.gender = "M".into();
        }
        // Gigantamax Melmetal does not come from Max Soup: its event alone.
        if species.id == "melmetal"
            && set.gigantamax
            && self.learnset_data_or_none(&species.id).event_data.is_some()
        {
            sources.sources_before = 0;
            sources.sources = vec!["8S0 melmetal".into()];
        }
        if !species.exists {
            return vec![
                problem(
                    Invalid,
                    format!("The Pokemon \"{}\" does not exist.", set.species),
                )
                .at(SetField::Species),
            ];
        }
        let item_exists = matches!(
            self.legality.item_named(self.aliases, &set.item),
            Named::Found(_)
        );
        if !item.id.is_empty() && !item_exists {
            return vec![
                problem(Invalid, format!("\"{}\" is an invalid item.", set.item))
                    .at(SetField::Item),
            ];
        }
        let ability_exists = matches!(
            self.legality.ability_named(self.aliases, &set.ability),
            Named::Found(_)
        );
        if !ability.id.is_empty() && !ability_exists {
            if generation < 3 {
                // Gen 1-2 have no abilities: removed, silently.
                ability = self.ability("");
                set.ability = String::new();
            } else {
                return vec![
                    problem(
                        Invalid,
                        format!("\"{}\" is an invalid ability.", set.ability),
                    )
                    .at(SetField::Ability),
                ];
            }
        }
        if !nature_id.is_empty() && !nature_exists {
            if generation < 3 {
                nature_id = String::new();
                nature_exists = false;
                set.nature = String::new();
            } else {
                problems.push(
                    problem(Invalid, format!("\"{}\" is an invalid nature.", set.nature))
                        .at(SetField::Nature),
                );
            }
        }
        let _ = (nature_id, nature_exists);
        if set.happiness.is_some_and(f64::is_nan) {
            problems.push(
                problem(Invalid, format!("{name} has an invalid happiness value."))
                    .at(SetField::Happiness),
            );
        }
        if !set.hp_type.is_empty() {
            match self.legality.type_named(&set.hp_type) {
                Named::Found(hp_type)
                    if !["normal", "fairy", "stellar"].contains(&hp_type.id.as_str()) =>
                {
                    set.hp_type = hp_type.name.clone();
                }
                _ => problems.push(
                    problem(
                        Invalid,
                        format!("{name}'s Hidden Power type ({}) is invalid.", set.hp_type),
                    )
                    .at(SetField::HpType),
                ),
            }
        }
        let from = problems.len();
        if generation == 9 && !table.has("terastalclause") || table.has("bonustypemod") {
            let wanted = if !set.tera_type.is_empty() {
                set.tera_type.clone()
            } else if let Some(required) = &species.required_tera_type {
                required.clone()
            } else {
                species.types.first().cloned().unwrap_or_default()
            };
            match self.legality.type_named(&wanted) {
                Named::Found(tera) if tera.nonstandard.is_none() => {
                    if let Some(required) = &species.required_tera_type
                        && *required != tera.name
                        && table.has("obtainablemisc")
                    {
                        problems.push(problem(
                            Tera,
                            format!("{}'s Terastal type needs to be {required}.", species.name),
                        ));
                    }
                    set.tera_type = tera.name.clone();
                }
                other => {
                    let shown = if set.tera_type.is_empty() {
                        "undefined".to_string()
                    } else {
                        set.tera_type.clone()
                    };
                    problems.push(problem(
                        Invalid,
                        format!("{name}'s Terastal type ({shown}) is invalid."),
                    ));
                    // `set.teraType = type.name`: a type that does not exist is named still.
                    set.tera_type = match other {
                        Named::Missing(id) => capitalise(&id),
                        Named::Found(tera) => tera.name.clone(),
                        Named::Empty => String::new(),
                    };
                }
            }
        } else {
            set.tera_type = String::new();
        }
        locate(&mut problems, from, SetField::TeraType);

        let species_set = SpeciesSet {
            ability: &set.ability.clone(),
            gigantamax: set.gigantamax,
        };
        if let Some(found) = self.check_species(&species_set, &species, &tier_species, &mut set_has)
        {
            problems.push(found.at(SetField::Species));
        }
        let item_ruling = (!item.id.is_empty()).then_some(item.as_ref());
        let from = problems.len();
        if let Some(found) = self.check_item(&set.name, item_ruling, &mut set_has) {
            problems.push(found);
        }
        if table.has("obtainablemisc") {
            if generation == 4 && item.id == "griseousorb" && species.num != 487 {
                problems.push(problem(
                    Item,
                    format!("{} cannot hold the Griseous Orb.", set.name),
                ));
                problems.push(problem(
                    Item,
                    "(In Gen 4, only Giratina could hold the Griseous Orb).".into(),
                ));
            }
            if generation <= 1 && !item.id.is_empty() {
                // No items in Gen 1.
                set.item = String::new();
            }
        }
        locate(&mut problems, from, SetField::Item);
        if set.ability.is_empty() {
            set.ability = "No Ability".into();
        }
        let from = problems.len();
        if table.has("obtainableabilities") {
            if generation <= 2 {
                set.ability = "No Ability".into();
            } else {
                let held = ability.name.clone();
                if held.is_empty() || held == "No Ability" {
                    problems.push(problem(
                        Ability,
                        format!("{name} needs to have an ability."),
                    ));
                } else if !species.abilities.values().any(|a| *a == held) {
                    if tier_species.abilities.get("0") == Some(&held) {
                        set.ability = species.abilities.get("0").cloned().unwrap_or_default();
                    } else {
                        problems.push(problem(
                            Ability,
                            format!("{name} can't have {}.", set.ability),
                        ));
                    }
                }
                if species.abilities.get("H") == Some(&held) {
                    sources.is_hidden = Some(true);
                    let mut unreleased = species.unreleased_hidden.clone();
                    if unreleased == super::legality::Unreleased::Since("Past".into())
                        && numbers.min_source_gen < generation
                    {
                        unreleased = super::legality::Unreleased::No;
                    }
                    let unreleased = !matches!(
                        unreleased,
                        super::legality::Unreleased::No | super::legality::Unreleased::Flag(false)
                    );
                    if unreleased && table.has("-unreleased") {
                        problems.push(problem(
                            Ability,
                            format!("{name}'s Hidden Ability is unreleased."),
                        ));
                    } else if generation == 7
                        && ["entei", "suicune", "raikou"].contains(&species.id.as_str())
                        && numbers.min_source_gen > 1
                    {
                        problems.push(problem(
                            Ability,
                            format!(
                                "{name}'s Hidden Ability is only available from Virtual Console, \
                                 which is not allowed in this format."
                            ),
                        ));
                    } else if generation == 6
                        && held == "Symbiosis"
                        && (set.species.ends_with("Orange") || set.species.ends_with("White"))
                    {
                        problems.push(problem(
                            Ability,
                            format!(
                                "{name}'s Hidden Ability is unreleased for the Orange and White forms."
                            ),
                        ));
                    } else if generation == 5
                        && set.level < 10
                        && (species.male_only_hidden || species.gender.as_deref() == Some("N"))
                    {
                        problems.push(problem(
                            Ability,
                            format!("{name} must be at least level 10 to have a Hidden Ability."),
                        ));
                    }
                    if species.male_only_hidden {
                        if !set.gender.is_empty() && set.gender != "M" {
                            problems.push(
                                problem(
                                    Gender,
                                    format!("{name} must be male to have a Hidden Ability."),
                                )
                                .at(SetField::Gender),
                            );
                        }
                        set.gender = "M".into();
                        sources.sources = vec!["5D".into()];
                    }
                } else {
                    sources.is_hidden = Some(false);
                }
            }
        }
        let ability = self.ability(&set.ability);
        let ability_ruling = (!ability.id.is_empty()).then_some(ability.as_ref());
        if let Some(found) = self.check_ability(&set.name, ability_ruling, &mut set_has) {
            problems.push(found);
        }
        locate(&mut problems, from, SetField::Ability);
        if generation <= 2 {
            set.nature = String::new();
        } else if set.nature.is_empty() {
            set.nature = "Serious".into();
        }
        let nature = self.legality.nature_named(self.aliases, &set.nature);
        let nature_thing = match &nature {
            Named::Found(nature) => Some((*nature).clone()),
            Named::Missing(id) => Some(super::legality::ThingFacts {
                key: id.clone(),
                id: id.clone(),
                name: id.clone(),
                ..Default::default()
            }),
            Named::Empty => None,
        };
        if let Some(found) = self.check_nature(&set.name, nature_thing.as_ref(), &mut set_has) {
            problems.push(found.at(SetField::Nature));
        }
        if set.shiny && generation == 1 {
            set.shiny = false;
        }
        set.moves.retain(|m| !m.is_empty());
        let from = problems.len();
        if set.moves.is_empty() {
            problems.push(problem(
                Moves,
                format!("{name} has no moves (it must have at least one to be usable)."),
            ));
        }
        if set.moves.len() as u32 > numbers.max_move_count {
            problems.push(problem(
                Moves,
                format!(
                    "{name} has {} moves, which is more than the limit of {}.",
                    set.moves.len(),
                    numbers.max_move_count
                ),
            ));
            locate(&mut problems, from, SetField::Moves);
            return problems;
        }
        locate(&mut problems, from, SetField::Moves);
        // Pokémon GO as an origin: `None` when it explains nothing.
        let mut go_problems = self.validate_pokemon_go(&out_of_battle, set, &mut sources, None);
        if table.is_banned("nonexistent") {
            problems.extend(self.validate_stats(
                set,
                &species,
                &mut sources,
                go_problems.as_deref(),
            ));
        }
        for (index, move_name) in set.moves.clone().iter().enumerate() {
            match self.legality.move_named(self.aliases, move_name) {
                Named::Found(the_move) => {
                    if let Some(found) = self.check_move(&set.name, the_move, &mut set_has) {
                        problems.push(found.at(SetField::Move(index)));
                    }
                }
                Named::Missing(id) => {
                    return vec![
                        problem(Invalid, format!("\"{id}\" is an invalid move."))
                            .at(SetField::Move(index)),
                    ];
                }
                Named::Empty => {}
            }
        }
        // The learnsets (M9.3). Without them (the seam's empty side) a species has no
        // learnset data: no Gen 1 encounter, no event; and no move is checked (`validateMoves`)
        // nor any species event-only (`getEventOnlyData`).
        let with_learnsets = self.learnsets.is_some();
        let learnset = self.learnset_data_or_none(&out_of_battle.id);
        let mut from_rby_encounter = false;
        if generation == 1 && table.has("obtainablemisc") && !table.has("allowtradeback") {
            let mut lowest: Option<u32> = None;
            for encounter in learnset.encounters.iter().flatten() {
                let Some(level) = encounter.level.filter(|l| *l != 0) else {
                    continue;
                };
                if encounter.generation != 1 || lowest.is_some_and(|low| level > low) {
                    continue;
                }
                lowest = Some(level);
            }
            if let Some(lowest) = lowest {
                if set.level < lowest {
                    problems.push(
                        problem(
                            Level,
                            format!("{name} is not obtainable at levels below {lowest} in Gen 1."),
                        )
                        .at(SetField::Level),
                    );
                }
                from_rby_encounter = true;
            }
        }
        let mut underleveled: Option<(String, u32)> = None;
        if !from_rby_encounter && table.has("obtainablemisc") {
            let mut evo = species.clone().into_owned();
            while let Some(prevo) = evo.prevo.clone() {
                if let Some(level) = evo.evo_level
                    && set.level < level
                {
                    underleveled = Some((evo.name.clone(), level));
                    break;
                }
                evo = self.species(&prevo).into_owned();
            }
        }
        if table.has("obtainablemoves") {
            if with_learnsets {
                let moves = set.moves.clone();
                let from = problems.len();
                problems.extend(self.validate_moves(
                    &out_of_battle,
                    &moves,
                    &mut sources,
                    set,
                    &name,
                ));
                locate(&mut problems, from, SetField::Moves);
            }
            if let (Some(go), Some(incompatible)) =
                (&mut go_problems, &sources.go_incompatible_moves)
            {
                go.push(if incompatible.is_empty() {
                    format!("{name} has moves which are incompatible with a Pokemon GO origin.")
                } else {
                    let one = incompatible.len() == 1;
                    format!(
                        "{name}'s move{} {} {} incompatible with a Pokemon GO origin.",
                        if one { "" } else { "s" },
                        incompatible.join(", "),
                        if one { "is" } else { "are" }
                    )
                });
            }
        }
        let go_failures = |problems: &mut Vec<Problem>, go: &Option<Vec<String>>, intro: &str| {
            if let Some(go) = go.as_ref().filter(|go| !go.is_empty()) {
                problems.push(problem(Learnset, intro.to_string()));
                problems.extend(go.iter().map(|m| problem(Learnset, m.clone())));
            }
        };
        let from = problems.len();
        if (sources.sources_before == 0 && !sources.sources.is_empty()) || underleveled.is_some() {
            let mut skipped_egg_source = true;
            let mut legal_sources: Vec<String> = Vec::new();
            let check_go_legality = !(underleveled.is_some() && sources.sources.is_empty());
            if let Some((underleveled_name, _)) = &underleveled
                && sources.sources.is_empty()
            {
                let mut evo = species.clone().into_owned();
                while let Some(prevo) = evo.prevo.clone() {
                    if let Some(events) = &self.learnset_data_or_none(&evo.id).event_data {
                        for (index, event) in events.iter().enumerate() {
                            if let Some(level) = event.level.filter(|l| *l != 0)
                                && set.level >= level
                            {
                                sources
                                    .sources
                                    .push(format!("{}S{index} {}", event.generation, evo.id));
                            }
                        }
                    }
                    if evo.name == *underleveled_name {
                        break;
                    }
                    evo = self.species(&prevo).into_owned();
                }
            }
            for source in sources.sources.clone() {
                if let Some((underleveled_name, _)) = &underleveled {
                    if source.chars().nth(1) == Some('S') {
                        // Showdown reads the species from `source.substr(3)`.
                        let event_species = self.species(source.get(3..).unwrap_or_default());
                        let underleveled_species = self.species(underleveled_name);
                        if event_species.base_species != species.base_species
                            && event_species.base_species != underleveled_species.base_species
                        {
                            continue;
                        }
                    } else if source != "8V" {
                        continue;
                    }
                }
                if (source.starts_with("2E") || source.starts_with("3E")) && set.level < 5 {
                    continue;
                }
                skipped_egg_source = false;
                if self
                    .validate_source(set, &source, &sources, &out_of_battle, None)
                    .is_some()
                {
                    continue;
                }
                legal_sources.push(source);
            }
            if check_go_legality && !legal_sources.iter().any(|s| s == "8V") {
                sources.is_from_pokemon_go = Some(false);
            }
            if sources.is_from_pokemon_go != Some(false)
                && go_problems.as_ref().is_some_and(Vec::is_empty)
            {
                if legal_sources.is_empty() {
                    sources.is_from_pokemon_go = Some(true);
                }
                if !legal_sources.iter().any(|s| s == "8V") {
                    legal_sources.push("8V".into());
                }
            }
            if !legal_sources.is_empty() {
                sources.sources = legal_sources;
            } else if let Some((_, required)) = &underleveled {
                problems.push(
                    problem(
                        Level,
                        format!("{name} must be at least level {required} to be evolved."),
                    )
                    .at(SetField::Level),
                );
                if let Some(first) = sources
                    .sources
                    .iter()
                    .find(|s| s.chars().nth(1) == Some('S'))
                    .cloned()
                    && let Some(found) = self.validate_source(
                        set,
                        &first,
                        &sources,
                        &out_of_battle,
                        Some(" to be underleveled"),
                    )
                {
                    problems.extend(found.into_iter().map(|m| problem(Learnset, m)));
                }
                go_failures(
                    &mut problems,
                    &go_problems,
                    "It failed to validate as a Pokemon from Pokemon GO because:",
                );
            } else {
                match sources
                    .sources
                    .iter()
                    .find(|s| s.chars().nth(1) != Some('E'))
                    .cloned()
                {
                    None => {
                        if skipped_egg_source {
                            problems.push(problem(
                                Learnset,
                                format!(
                                    "{name} is from a Gen 2 or 3 egg, which cannot be obtained at \
                                     levels below 5."
                                ),
                            ));
                        } else {
                            let limited = sources
                                .limited_egg_moves
                                .value()
                                .cloned()
                                .unwrap_or_default();
                            problems.push(problem(
                                Learnset,
                                format!(
                                    "{name} can't get its egg move combination ({}) from any \
                                     possible father.",
                                    limited.join(", ")
                                ),
                            ));
                            problems.push(problem(
                                Learnset,
                                "(Is this incorrect? If so, post the chainbreeding instructions in \
                                 Bug Reports)"
                                    .into(),
                            ));
                        }
                    }
                    Some(_)
                        if species.id == "mew"
                            && go_problems.as_ref().is_some_and(Vec::is_empty) =>
                    {
                        // Pokémon GO's Mew, which cannot be sent to Let's Go.
                        sources.is_from_pokemon_go = Some(true);
                    }
                    Some(non_egg) => {
                        if sources.sources.len() > 1 {
                            problems.push(problem(
                                Learnset,
                                format!(
                                    "{name} has an event-exclusive move that it doesn't qualify for \
                                     (only one of several ways to get the move will be listed):"
                                ),
                            ));
                        }
                        if let Some(found) = self.validate_source(
                            set,
                            &non_egg,
                            &sources,
                            &out_of_battle,
                            Some(" because it has a move only available"),
                        ) {
                            problems.extend(found.into_iter().map(|m| problem(Learnset, m)));
                        }
                        if species.id == "mew" {
                            go_failures(
                                &mut problems,
                                &go_problems,
                                "Additionally, it failed to validate as a Pokemon from Pokemon GO because:",
                            );
                        }
                    }
                }
            }
        } else if table.has("obtainablemisc")
            && with_learnsets
            && let Some((event_species, events)) = self.event_only_data(&out_of_battle, false)
        {
            let mut events: Vec<super::learnset::EventInfo> = events.clone();
            if table.has("fullarceusclause") && event_species.base_species == "Arceus" {
                // Hall of Origin Arceus.
                events.push(super::learnset::EventInfo {
                    generation: 4,
                    level: Some(80),
                    moves: Some(vec![
                        "refresh".into(),
                        "futuresight".into(),
                        "recover".into(),
                        "hyperbeam".into(),
                    ]),
                    ..Default::default()
                });
            }
            let mut legal = false;
            for event in &events {
                if self
                    .validate_event(set, &sources, event, &event_species, None, "from an event")
                    .is_some()
                {
                    continue;
                }
                sources.event_only_min_source_gen = Some(event.generation);
                legal = true;
                break;
            }
            if !legal
                && species.generation <= 2
                && generation >= 7
                && self
                    .validate_source(set, "7V", &sources, &species, None)
                    .is_none()
            {
                legal = true;
            }
            if !legal && go_problems.as_ref().is_some_and(Vec::is_empty) {
                // A Pokémon GO origin explains it.
                sources.is_from_pokemon_go = Some(true);
            } else if !legal {
                problems.push(problem(
                    Learnset,
                    if events.len() == 1 {
                        format!(
                            "{} is only obtainable from an event - it needs to match its event:",
                            species.name
                        )
                    } else {
                        format!(
                            "{} is only obtainable from events - it needs to match one of its events:",
                            species.name
                        )
                    },
                ));
                let min_source_gen = numbers.min_source_gen;
                for (index, event) in events.iter().enumerate() {
                    if event.generation <= generation
                        && (event.generation >= min_source_gen || generation > 8)
                    {
                        let event_name = if events.len() > 1 {
                            format!(" #{}", index + 1)
                        } else {
                            String::new()
                        };
                        if let Some(found) = self.validate_event(
                            set,
                            &sources,
                            event,
                            &event_species,
                            Some(" to be"),
                            &format!("from its event{event_name}"),
                        ) {
                            problems.extend(found.into_iter().map(|m| problem(Learnset, m)));
                        }
                    }
                }
                go_failures(
                    &mut problems,
                    &go_problems,
                    "Additionally, it failed to validate as a Pokemon from Pokemon GO because:",
                );
            }
        }
        // The species only Pokémon GO has.
        if table.has("obtainablemisc")
            && ["meltan", "melmetal", "gimmighoulroaming"].contains(&species.id.as_str())
        {
            go_failures(
                &mut problems,
                &go_problems,
                &format!(
                    "{name} is only obtainable from Pokemon GO, and failed to validate because:"
                ),
            );
        }
        locate(&mut problems, from, SetField::Origin);
        let from = problems.len();
        if table.has("obtainablemoves") {
            if species.id == "keldeo"
                && set.moves.iter().any(|m| m == "secretsword")
                && numbers.min_source_gen > 5
                && generation <= 7
            {
                problems.push(problem(
                    Moves,
                    format!(
                        "{name} has Secret Sword, which is only compatible with Keldeo-Ordinary \
                         obtained from Gen 5."
                    ),
                ));
            }
            let requires_gen3_source = sources.max_source_gen() <= 3;
            if requires_gen3_source
                && self.ability(&set.ability).generation == 4
                && species.prevo.is_none()
                && generation <= 5
            {
                problems.push(problem(
                    Moves,
                    format!(
                        "{name} has a Gen 4 ability and isn't evolved - it can't use moves from Gen 3."
                    ),
                ));
            }
            let can_use_ability_patch = generation >= 8 && self.mod_name != "gen8dlc1";
            if sources.is_hidden == Some(true)
                && !can_use_ability_patch
                && sources.max_source_gen() < 5
            {
                problems.push(problem(
                    Moves,
                    format!("{name} has a Hidden Ability - it can't use moves from before Gen 5."),
                ));
            }
            if species.male_only_hidden
                && sources.is_hidden == Some(true)
                && sources.sources_before < 5
                && sources
                    .sources
                    .iter()
                    .all(|s| s.chars().nth(1) == Some('E'))
            {
                problems.push(problem(
                    Moves,
                    format!("{name} has an unbreedable Hidden Ability - it can't use egg moves."),
                ));
            }
        }
        locate(&mut problems, from, SetField::Moves);
        if let Some(team_has) = team_has {
            for thing in &set_has {
                *team_has.entry(thing.clone()).or_insert(0) += 1;
            }
        }
        for ban in &table.complex_bans {
            let count = ban.bans.iter().filter(|b| set_has.contains(*b)).count() as u32;
            let clause = if ban.source.is_empty() {
                String::new()
            } else {
                format!(" by {}", ban.source)
            };
            match ban.limit {
                Some(limit) if limit > 0 && count > limit => problems.push(
                    problem(
                        Combination,
                        format!("{name} is limited to {limit} of {}{clause}.", ban.rule),
                    )
                    .by(ban.rule.clone()),
                ),
                Some(0) if count as usize >= ban.bans.len() => problems.push(
                    problem(
                        Combination,
                        if ban.source == "Obtainable Moves" {
                            format!(
                                "{name} has the combination of {}, which is impossible to obtain \
                             legitimately.",
                                ban.rule
                            )
                        } else {
                            format!(
                                "{name} has the combination of {}, which is banned{clause}.",
                                ban.rule
                            )
                        },
                    )
                    .by(ban.rule.clone()),
                ),
                _ => {}
            }
        }
        // The rules' `onValidateSet`, in the table's order.
        for (rule, _) in table.entries.iter() {
            if !self.runs(rule, "onValidateSet") {
                continue;
            }
            match rule {
                "ohkoclause" | "sleepmovesclause" => {
                    let (clause, test): (&str, fn(&super::legality::ThingFacts) -> bool) =
                        if rule == "ohkoclause" {
                            ("OHKO Clause", |m| m.ohko)
                        } else {
                            ("Sleep Moves Clause", |m| m.status.as_deref() == Some("slp"))
                        };
                    for (index, move_name) in set.moves.iter().enumerate() {
                        if let Named::Found(the_move) =
                            self.legality.move_named(self.aliases, move_name)
                            && test(the_move)
                        {
                            problems.push(
                                problem(
                                    Clause,
                                    format!("{} is banned by {clause}.", the_move.name),
                                )
                                .at(SetField::Move(index))
                                .by(rule),
                            );
                        }
                    }
                }
                "dynamaxclause" if set.gigantamax => {
                    problems.push(problem(
                        Clause,
                        format!(
                            "Your set for {} is flagged as Gigantamax, but Gigantamaxing is disallowed",
                            set.species
                        ),
                    ).by(rule));
                    problems.push(
                        problem(
                            Clause,
                            "(If this was a mistake, disable Gigantamaxing on the set.)".into(),
                        )
                        .by(rule),
                    );
                }
                "gemsclause" if !set.item.is_empty() => {
                    if let Named::Found(gem) = self.legality.item_named(self.aliases, &set.item)
                        && gem.is_gem
                        && !table.has(&format!("+item:{}", gem.id))
                    {
                        problems.push(
                            problem(
                                Clause,
                                format!("{} is banned due to Gems Clause.", gem.name),
                            )
                            .at(SetField::Item)
                            .by(rule),
                        );
                    }
                }
                "accuracytrapclause" => {
                    let move_ids: Vec<String> = set.moves.iter().map(|m| to_id(m)).collect();
                    let lowers = move_ids.iter().any(|id| {
                        self.legality
                            .moves
                            .iter()
                            .any(|m| m.lowers_accuracy && m.id == *id)
                    });
                    let traps = TRAPPING.contains(&to_id(&set.ability).as_str())
                        || move_ids.iter().any(|id| TRAPPING.contains(&id.as_str()));
                    if lowers && traps {
                        problems.push(
                            problem(
                                Clause,
                                format!(
                                    "{} has the combination of a trapping move/ability and a \
                                     guaranteed accuracy-lowering move, which is banned.",
                                    set.species
                                ),
                            )
                            .at(SetField::Moves)
                            .by(rule),
                        );
                    }
                }
                _ => {}
            }
        }
        // A nickname that is a species' name.
        if let Named::Found(named) = self.legality.species_named(self.aliases, &set.name)
            && named.name.to_lowercase() == set.name.to_lowercase()
        {
            if named.base_species == species.base_species {
                set.name = species.base_species.clone();
            } else if named.name != species.name
                && named.name != species.base_species
                && table.has("nicknameclause")
            {
                problems.push(
                    problem(
                        Nickname,
                        format!(
                            "{name} must not be nicknamed a different Pokémon species than what it \
                             actually is."
                        ),
                    )
                    .at(SetField::Name)
                    .by("nicknameclause"),
                );
            }
        }
        if problems.is_empty() {
            if generation > 5 && !table.has("obtainablemisc") && set.gender.is_empty() {
                set.gender = "N".into();
            }
            if let Some(level) = adjust_level {
                set.level = level;
            }
        }
        problems
    }
}

/// A name's resolution as Showdown's object would give it: its id, whether it exists, its name.
fn named_parts(named: &Named<'_, super::legality::ThingFacts>) -> (String, bool, String) {
    match named {
        Named::Found(thing) => (thing.id.clone(), true, thing.name.clone()),
        Named::Missing(id) => (id.clone(), false, id.clone()),
        Named::Empty => (String::new(), false, String::new()),
    }
}

/// `id.charAt(0).toUpperCase() + id.substr(1)`.
fn capitalise(id: &str) -> String {
    let mut name = id.to_string();
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    name
}
