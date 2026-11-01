//! The team validator against Showdown's (DESIGN.md §45.3), ruling by ruling, under every target
//! format and a list of custom ones: `tools/oracle-node/src/export-validator.mjs` asked
//! Showdown's `TeamValidator` about every species, item, move, ability and nature of each
//! format's dex (`tests/fixtures/formats/checks.json`).

use std::sync::OnceLock;

use pokemon_battle_engine::format::{
    FormatLibrary, LegalityData, Problem, ProblemKind, SetHas, SpeciesFacts, SpeciesSet,
    TeamValidator, ThingFacts,
};
use serde_json::Value;

fn library() -> &'static FormatLibrary {
    static LIBRARY: OnceLock<FormatLibrary> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        FormatLibrary::open(concat!(env!("CARGO_MANIFEST_DIR"), "/data"))
            .unwrap_or_else(|error| panic!("{error}"))
    })
}

fn checks() -> &'static Value {
    static CHECKS: OnceLock<Value> = OnceLock::new();
    CHECKS.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/formats/checks.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    })
}

fn species<'a>(legality: &'a LegalityData, id: &str) -> &'a SpeciesFacts {
    legality
        .species(id)
        .unwrap_or_else(|| panic!("{} has no species {id}", legality.mod_name))
}

/// Every ruling of one validator against Showdown's: `own` holds the answers where they differ
/// from the mod's (`common`); `set_has` the sampled records. The mismatches, and the count.
fn rule_on_everything(
    validator: &TeamValidator<'_>,
    name: &str,
    own: &Value,
    common: &Value,
    set_name: &str,
) -> (Vec<String>, usize) {
    let mut wrong = Vec::new();
    let mut count = 0;
    let mut compare = |table: &str, key: &str, actual: Option<Problem>, set_has: &SetHas| {
        count += 1;
        let expected = match own[table].get(key) {
            Some(value) => value.as_str(),
            None => common[table].get(key).and_then(Value::as_str),
        };
        let actual = actual.map(|problem| problem.message);
        if actual.as_deref() != expected {
            wrong.push(format!(
                "{name} {table} {key}:\n  engine   {actual:?}\n  showdown {expected:?}"
            ));
        }
        if let Some(expected) = own["set_has"][format!("{table}:{key}")].as_array() {
            let expected: Vec<&str> = expected.iter().map(|v| v.as_str().unwrap()).collect();
            let actual: Vec<&str> = set_has.iter().map(String::as_str).collect();
            if actual != expected {
                wrong.push(format!(
                    "{name} {table} {key} set_has:\n  engine   {actual:?}\n  showdown {expected:?}"
                ));
            }
        }
    };
    let legality = validator.legality;
    for row in &legality.species {
        let mut set_has = SetHas::new();
        let problem = validator.check_species(&SpeciesSet::default(), row, row, &mut set_has);
        compare("species", &row.id, problem, &set_has);
    }
    for key in common["pair_keys"].as_array().unwrap() {
        let key = key.as_str().unwrap();
        let parts: Vec<&str> = key.split('|').collect();
        let set = SpeciesSet {
            ability: parts[2],
            gigantamax: parts[3] == "gmax",
        };
        let mut set_has = SetHas::new();
        let problem = validator.check_species(
            &set,
            species(legality, parts[0]),
            species(legality, parts[1]),
            &mut set_has,
        );
        compare("pairs", key, problem, &set_has);
    }
    type Ruling = fn(&TeamValidator<'_>, &str, Option<&ThingFacts>, &mut SetHas) -> Option<Problem>;
    let rulings: [(&str, &[ThingFacts], Ruling); 4] = [
        ("items", &legality.items, |v, n, t, s| v.check_item(n, t, s)),
        ("moves", &legality.moves, |v, n, t, s| {
            v.check_move(n, t.unwrap(), s)
        }),
        ("abilities", &legality.abilities, |v, n, t, s| {
            v.check_ability(n, t, s)
        }),
        ("natures", &legality.natures, |v, n, t, s| {
            v.check_nature(n, t, s)
        }),
    ];
    for (table, rows, ruling) in rulings {
        // Nothing first (no item, no ability, no nature); a move is never empty.
        if table != "moves" {
            let mut set_has = SetHas::new();
            let problem = ruling(validator, set_name, None, &mut set_has);
            compare(table, "", problem, &set_has);
        }
        for row in rows {
            let mut set_has = SetHas::new();
            let problem = ruling(validator, set_name, Some(row), &mut set_has);
            compare(table, &row.key, problem, &set_has);
        }
    }
    (wrong, count)
}

fn report(wrong: Vec<String>, total: usize, at_least: usize) {
    let shown: Vec<_> = wrong.iter().take(30).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong of {total}:\n{}",
        wrong.len(),
        shown.join("\n")
    );
    assert!(total > at_least, "{total} rulings");
}

#[test]
fn every_thing_is_ruled_on_as_showdown_rules_on_it() {
    let fixture = checks();
    let set_name = fixture["set_name"].as_str().unwrap();
    let (mut wrong, mut total) = (Vec::new(), 0);
    for (format, own) in fixture["formats"].as_object().unwrap() {
        let validator = TeamValidator::without_learnsets(library(), format, &[]).unwrap();
        let common = &fixture["mods"][own["mod"].as_str().unwrap()];
        let (w, n) = rule_on_everything(&validator, format, own, common, set_name);
        wrong.extend(w);
        total += n;
    }
    report(wrong, total, 120_000);
}

/// The branches no target format reaches: whitelists (with Showdown's `allnatures`, which
/// `-All Natures` never sets), a tag's unban against Nonexistent, `+CAP`, `+Past`, an unbanned
/// base species, Megas banned by their tag.
#[test]
fn custom_formats_rule_as_showdown_rules() {
    let fixture = checks();
    let set_name = fixture["set_name"].as_str().unwrap();
    let (mut wrong, mut total) = (Vec::new(), 0);
    for case in fixture["custom"].as_array().unwrap() {
        let format = case["format"].as_str().unwrap();
        let rules: Vec<String> = case["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|rule| rule.as_str().unwrap().to_string())
            .collect();
        let validator = TeamValidator::without_learnsets(library(), format, &rules).unwrap();
        let common = &fixture["mods"][case["mod"].as_str().unwrap()];
        let name = format!("{format}@@@{}", rules.join(","));
        let (w, n) = rule_on_everything(&validator, &name, case, common, set_name);
        wrong.extend(w);
        total += n;
    }
    report(wrong, total, 40_000);
}

/// Showdown `getReason`: a Nonexistent ban reads `not obtainable` — no ruling prints it (the
/// rulings word a Nonexistent thing themselves), so it is checked here.
#[test]
fn a_nonexistent_ban_reads_not_obtainable() {
    let validator = TeamValidator::without_learnsets(library(), "gen9ou", &[]).unwrap();
    assert!(validator.table.has("-nonexistent"));
    assert_eq!(
        validator.table.reason("-nonexistent").as_deref(),
        Some("not obtainable")
    );
}

/// A species that can Gigantamax whose Gigantamax Factor cannot be had (`gmaxUnreleased`):
/// no species of the pinned data is one, so a made-up one, worded as Showdown's
/// `checkSpecies` words it.
#[test]
fn an_unreleased_gigantamax_factor_is_unobtainable() {
    let validator = TeamValidator::without_learnsets(library(), "gen8ubers", &[]).unwrap();
    let mut melmetal = species(validator.legality, "melmetal").clone();
    assert!(melmetal.can_gigantamax.is_some());
    melmetal.gmax_unreleased = true;
    let set = SpeciesSet {
        ability: "",
        gigantamax: true,
    };
    let problem = validator
        .check_species(&set, &melmetal, &melmetal, &mut SetHas::new())
        .unwrap();
    assert_eq!(problem.kind, ProblemKind::Unobtainable);
    assert_eq!(
        problem.message,
        "Melmetal is flagged as gigantamax, but it cannot gigantamax without hacking or glitches."
    );
}

/// Name resolution (`dex.species.get` and the other five getters) for every key, alias,
/// cosmetic forme, guessed regional or Mega forme and a few names that are nothing
/// (`tests/fixtures/formats/names.json`, export-legality.mjs): [id, name, exists].
#[test]
fn every_name_resolves_as_showdown_resolves_it() {
    use pokemon_battle_engine::format::Named;
    use serde_json::json;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/names.json"
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let aliases = library().aliases().unwrap();
    fn answer<T>(named: Named<'_, T>, id: impl Fn(&T) -> (String, String)) -> Value {
        match named {
            Named::Empty => json!(["", "", false]),
            Named::Found(thing) => {
                let (id, name) = id(thing);
                json!([id, name, true])
            }
            Named::Missing(missing) => json!([missing, missing, false]),
        }
    }
    let mut wrong = Vec::new();
    let mut count = 0;
    for (mod_name, kinds) in fixture["names"].as_object().unwrap() {
        let legality = library().legality(mod_name).unwrap();
        for (kind, inputs) in kinds.as_object().unwrap() {
            for (input, expected) in inputs.as_object().unwrap() {
                count += 1;
                let thing = |t: &ThingFacts| (t.id.clone(), t.name.clone());
                let actual = match kind.as_str() {
                    "species" => answer(legality.species_named(aliases, input), |s| {
                        (s.id.clone(), s.name.clone())
                    }),
                    "items" => answer(legality.item_named(aliases, input), thing),
                    "moves" => answer(legality.move_named(aliases, input), thing),
                    "abilities" => answer(legality.ability_named(aliases, input), thing),
                    "natures" => answer(legality.nature_named(aliases, input), thing),
                    "types" => match legality.type_named(input) {
                        // A type that does not exist is still named, its id capitalised.
                        Named::Missing(id) => {
                            let mut name = id.clone();
                            if let Some(first) = name.get_mut(0..1) {
                                first.make_ascii_uppercase();
                            }
                            json!([id, name, false])
                        }
                        named => answer(named, |t| (t.id.clone(), t.name.clone())),
                    },
                    other => panic!("{other}"),
                };
                if &actual != expected {
                    wrong.push(format!(
                        "{mod_name} {kind} {input:?}: engine {actual}, showdown {expected}"
                    ));
                }
            }
        }
    }
    let shown: Vec<_> = wrong.iter().take(30).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong of {count}:\n{}",
        wrong.len(),
        shown.join("\n")
    );
    assert!(count > 30_000, "{count} names");
}

fn formes() -> &'static Value {
    static FORMES: OnceLock<Value> = OnceLock::new();
    FORMES.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/formats/formes.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    })
}

/// A set from the fixture's key: [species, item, ability, moves, ivs?].
fn set_of(key: &str) -> pokemon_battle_engine::format::TeamSet {
    use pokemon_battle_engine::format::{Stats, TeamSet};
    let parts: Value = serde_json::from_str(key).unwrap();
    let text = |i: usize| parts[i].as_str().unwrap().to_string();
    let ivs = parts[4].as_object().map(|ivs| {
        let stat = |name: &str| ivs[name].as_i64().unwrap() as i32;
        Stats {
            hp: stat("hp"),
            atk: stat("atk"),
            def: stat("def"),
            spa: stat("spa"),
            spd: stat("spd"),
            spe: stat("spe"),
        }
    });
    TeamSet {
        species: text(0),
        item: text(1),
        ability: text(2),
        moves: parts[3]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap().to_string())
            .collect(),
        ivs: Some(ivs.unwrap_or(Stats::filled(31))),
        ..TeamSet::default()
    }
}

/// `validateForme` on sets made to reach each of its branches, per mod
/// (`tests/fixtures/formats/formes.json`, export-validator.mjs): the problems, and the set as
/// it comes back (species, ability, moves).
#[test]
fn every_forme_is_validated_as_showdown_validates_it() {
    use serde_json::json;
    let fixture = formes();
    let formats = &library().formats().formats;
    let mut wrong = Vec::new();
    let mut count = 0;
    for (mod_name, answers) in fixture["forme"].as_object().unwrap() {
        // The first target format of the mod, as the export used.
        let format = formats
            .iter()
            .filter(|f| f.mod_name.as_deref() == Some(mod_name.as_str()))
            .map(|f| f.id.as_str())
            .min()
            .unwrap();
        let validator = TeamValidator::without_learnsets(library(), format, &[]).unwrap();
        for (key, expected) in answers.as_object().unwrap() {
            count += 1;
            let mut set = set_of(key);
            let before = set.clone();
            let problems: Vec<String> = validator
                .validate_forme(&mut set)
                .into_iter()
                .map(|p| p.message)
                .collect();
            let changed = set.species != before.species
                || set.ability != before.ability
                || set.moves != before.moves;
            let actual = if problems.is_empty() && !changed {
                json!(0)
            } else {
                json!([problems, set.species, set.ability, set.moves])
            };
            if &actual != expected {
                wrong.push(format!(
                    "{mod_name} {key}:\n  engine   {actual}\n  showdown {expected}"
                ));
            }
        }
    }
    let shown: Vec<_> = wrong.iter().take(20).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong of {count}:\n{}",
        wrong.len(),
        shown.join("\n")
    );
    assert!(count > 15_000, "{count} sets");
}

/// `getValidationSpecies`, per format: [out-of-battle species, tier species].
#[test]
fn every_validation_species_is_showdowns() {
    use serde_json::json;
    let fixture = formes();
    let mut wrong = Vec::new();
    for (format, answers) in fixture["validation_species"].as_object().unwrap() {
        let validator = TeamValidator::without_learnsets(library(), format, &[]).unwrap();
        for (key, expected) in answers.as_object().unwrap() {
            let (out_of_battle, tier) = validator.validation_species(&set_of(key));
            let actual = json!([out_of_battle.id, tier.id]);
            if &actual != expected {
                wrong.push(format!(
                    "{format} {key}: engine {actual}, showdown {expected}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// `validateStats` on sets made to reach each branch, per format, with the sources as no
/// learnset narrows them and no Pokémon GO origin (`tests/fixtures/formats/stats.json`,
/// export-validator.mjs): the problems, and the set's EVs, IVs, Hidden Power type, gender and
/// nature as it comes back.
#[test]
fn every_set_s_stats_are_validated_as_showdown_validates_them() {
    use pokemon_battle_engine::format::{Stats, TeamSet};
    use serde_json::json;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/stats.json"
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let stats_of = |value: &Value| -> Option<Stats> {
        let stat = |name: &str| value[name].as_i64().unwrap() as i32;
        value.is_object().then(|| Stats {
            hp: stat("hp"),
            atk: stat("atk"),
            def: stat("def"),
            spa: stat("spa"),
            spd: stat("spd"),
            spe: stat("spe"),
        })
    };
    let json_of = |stats: &Option<Stats>| {
        stats.map(|s| {
            json!({"hp": s.hp, "atk": s.atk, "def": s.def, "spa": s.spa, "spd": s.spd, "spe": s.spe})
        })
    };
    let mut wrong = Vec::new();
    let mut count = 0;
    for (format, own) in fixture["formats"].as_object().unwrap() {
        let validator = TeamValidator::without_learnsets(library(), format, &[]).unwrap();
        let common = &fixture["mods"][own["mod"].as_str().unwrap()];
        for (index, key) in common["cases"].as_array().unwrap().iter().enumerate() {
            count += 1;
            let key = key.as_str().unwrap();
            let expected = own["diff"]
                .get(index.to_string())
                .unwrap_or(&common["common"][index]);
            let case: Value = serde_json::from_str(key).unwrap();
            let name = case[0].as_str().unwrap();
            let c = &case[1];
            let text = |field: &str| c[field].as_str().unwrap_or_default().to_string();
            let mut set = TeamSet {
                species: validator.species(name).name.clone(),
                item: text("item"),
                moves: c["moves"]
                    .as_array()
                    .map(|m| m.iter().map(|m| m.as_str().unwrap().to_string()).collect())
                    .unwrap_or_default(),
                level: c["level"].as_u64().unwrap_or(100) as u32,
                gender: text("gender"),
                shiny: c["shiny"].as_bool().unwrap_or(false),
                hp_type: text("hpType"),
                evs: stats_of(&c["evs"]),
                ivs: stats_of(&c["ivs"]),
                ..TeamSet::default()
            };
            let species = validator.species(name).into_owned();
            let mut sources = validator.all_sources(Some(&species));
            let problems: Vec<String> = validator
                .validate_stats(&mut set, &species, &mut sources, None)
                .into_iter()
                .map(|p| p.message)
                .collect();
            let actual = json!([
                problems,
                json_of(&set.evs),
                json_of(&set.ivs),
                set.hp_type,
                set.gender,
                set.nature
            ]);
            if &actual != expected {
                wrong.push(format!(
                    "{format} {key}:\n  engine   {actual}\n  showdown {expected}"
                ));
            }
        }
    }
    let shown: Vec<_> = wrong.iter().take(15).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong of {count}:\n{}",
        wrong.len(),
        shown.join("\n")
    );
    assert!(count > 9_000, "{count} sets");
}

/// One set of `sets.json` through a validator: the problems and the set as it comes back,
/// against Showdown's.
fn run_set(validator: &TeamValidator<'_>, input: &Value) -> Value {
    use pokemon_battle_engine::format::{Stats, TeamHas, TeamSet};
    use serde_json::json;
    let stats_of = |value: &Value| -> Option<Stats> {
        let stat = |name: &str| value[name].as_i64().unwrap() as i32;
        value.is_object().then(|| Stats {
            hp: stat("hp"),
            atk: stat("atk"),
            def: stat("def"),
            spa: stat("spa"),
            spd: stat("spd"),
            spe: stat("spe"),
        })
    };
    let json_of = |stats: &Option<Stats>| {
        stats.map(|s| {
            json!({"hp": s.hp, "atk": s.atk, "def": s.def, "spa": s.spa, "spd": s.spd, "spe": s.spe})
        })
    };
    let text = |field: &str| input[field].as_str().unwrap_or_default().to_string();
    let mut set = TeamSet {
        species: text("species"),
        name: text("name"),
        item: text("item"),
        ability: text("ability"),
        moves: input["moves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap().to_string())
            .collect(),
        nature: text("nature"),
        gender: text("gender"),
        level: input["level"].as_u64().unwrap() as u32,
        evs: stats_of(&input["evs"]),
        ivs: stats_of(&input["ivs"]),
        shiny: input["shiny"].as_bool().unwrap(),
        happiness: match &input["happiness"] {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => Some(s.parse().unwrap_or(f64::NAN)),
            _ => None,
        },
        hp_type: text("hpType"),
        tera_type: text("teraType"),
        gigantamax: input["gigantamax"].as_bool().unwrap(),
    };
    let mut team_has = TeamHas::new();
    let problems: Vec<String> = validator
        .validate_set(&mut set, Some(&mut team_has))
        .into_iter()
        .map(|p| p.message)
        .collect();
    json!([problems, {
        "species": set.species, "name": set.name, "item": set.item,
        "ability": set.ability, "moves": set.moves, "nature": set.nature,
        "gender": set.gender, "level": set.level, "evs": json_of(&set.evs),
        "ivs": json_of(&set.ivs), "hpType": set.hp_type, "teraType": set.tera_type,
        "shiny": set.shiny,
    }])
}

/// `validateSet` on the learnset seam's empty side, per format and under three custom ones, on
/// sets drawn from a seeded generator and a few made to reach rare branches
/// (`tests/fixtures/formats/sets.json`, export-validator.mjs): the problems, and the set as it
/// comes back.
#[test]
fn every_set_is_validated_as_showdown_validates_it() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/sets.json"
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut wrong = Vec::new();
    let mut count = 0;
    let mut cases: Vec<(String, TeamValidator<'_>, &Value)> = Vec::new();
    for (format, own) in fixture["formats"].as_object().unwrap() {
        cases.push((
            format.clone(),
            TeamValidator::without_learnsets(library(), format, &[]).unwrap(),
            own,
        ));
    }
    for own in fixture["custom"].as_array().unwrap() {
        let format = own["format"].as_str().unwrap();
        let rules: Vec<String> = own["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect();
        let validator = TeamValidator::without_learnsets(library(), format, &rules).unwrap();
        cases.push((format!("{format}@@@{}", rules.join(",")), validator, own));
    }
    for (name, validator, own) in &cases {
        let common = &fixture["mods"][own["mod"].as_str().unwrap()];
        for (index, input) in common["sets"].as_array().unwrap().iter().enumerate() {
            count += 1;
            let expected = own["diff"]
                .get(index.to_string())
                .unwrap_or(&common["common"][index]);
            let actual = run_set(validator, input);
            if &actual != expected {
                wrong.push(format!(
                    "{name} #{index} {input}:\n  engine   {actual}\n  showdown {expected}"
                ));
            }
        }
    }
    let shown: Vec<_> = wrong.iter().take(12).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong of {count}:\n{}",
        wrong.len(),
        shown.join("\n")
    );
    assert!(count > 10_000, "{count} sets");
}

/// `validateTeam` on the learnset seam's empty side, per format, on teams drawn from the mod's
/// sets (`sets.json`'s, then `teams.json`'s extras made for the team rules) and teams made for
/// each rule (`tests/fixtures/formats/teams.json`, export-validator.mjs): the problems.
#[test]
fn every_team_is_validated_as_showdown_validates_it() {
    use pokemon_battle_engine::format::TeamSet;
    let read = |name: &str| -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/formats")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    let sets = read("sets.json");
    let teams = read("teams.json");
    let to_set = |input: &Value| -> TeamSet {
        // The same reading `run_set` makes of a set.
        let text = |field: &str| input[field].as_str().unwrap_or_default().to_string();
        let stats = |value: &Value| {
            value.is_object().then(|| {
                let stat = |name: &str| value[name].as_i64().unwrap() as i32;
                pokemon_battle_engine::format::Stats {
                    hp: stat("hp"),
                    atk: stat("atk"),
                    def: stat("def"),
                    spa: stat("spa"),
                    spd: stat("spd"),
                    spe: stat("spe"),
                }
            })
        };
        TeamSet {
            species: text("species"),
            name: text("name"),
            item: text("item"),
            ability: text("ability"),
            moves: input["moves"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m.as_str().unwrap().to_string())
                .collect(),
            nature: text("nature"),
            gender: text("gender"),
            level: input["level"].as_u64().unwrap() as u32,
            evs: stats(&input["evs"]),
            ivs: stats(&input["ivs"]),
            shiny: input["shiny"].as_bool().unwrap(),
            happiness: match &input["happiness"] {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => Some(s.parse().unwrap_or(f64::NAN)),
                _ => None,
            },
            hp_type: text("hpType"),
            tera_type: text("teraType"),
            gigantamax: input["gigantamax"].as_bool().unwrap(),
        }
    };
    let mut wrong = Vec::new();
    let mut count = 0;
    for (format, own) in teams["formats"].as_object().unwrap() {
        let validator = TeamValidator::without_learnsets(library(), format, &[]).unwrap();
        let mod_name = own["mod"].as_str().unwrap();
        let pool: Vec<&Value> = sets["mods"][mod_name]["sets"]
            .as_array()
            .unwrap()
            .iter()
            .chain(teams["mods"][mod_name]["extras"].as_array().unwrap())
            .collect();
        let common = &teams["mods"][mod_name];
        for (index, team) in common["teams"].as_array().unwrap().iter().enumerate() {
            count += 1;
            let expected = own["diff"]
                .get(index.to_string())
                .unwrap_or(&common["common"][index]);
            let mut team: Vec<TeamSet> = team
                .as_array()
                .unwrap()
                .iter()
                .map(|i| to_set(pool[i.as_u64().unwrap() as usize]))
                .collect();
            let problems: Vec<String> = validator
                .validate_team(&mut team)
                .into_iter()
                .map(|p| p.message)
                .collect();
            let actual = serde_json::json!(problems);
            if &actual != expected {
                wrong.push(format!(
                    "{format} team #{index}:\n  engine   {actual}\n  showdown {expected}"
                ));
            }
        }
    }
    let shown: Vec<_> = wrong.iter().take(10).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong of {count}:\n{}",
        wrong.len(),
        shown.join("\n")
    );
    assert!(count > 5_000, "{count} teams");
}

/// M9.3 first phase: `validateSet` with the learnsets (the seams only an egg move combination's
/// father, always found, and Pokémon GO, none), per format, on sets whose moves come mostly
/// from the species' family learnset (`tests/fixtures/formats/learnset-sets.json`,
/// export-validator.mjs): the problems, and the set as it comes back.
#[test]
fn every_set_s_moves_are_validated_as_showdown_validates_them() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/learnset-sets.json"
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut wrong = Vec::new();
    let mut count = 0;
    for (format, own) in fixture["formats"].as_object().unwrap() {
        let validator = TeamValidator::new(library(), format).unwrap();
        let common = &fixture["mods"][own["mod"].as_str().unwrap()];
        for (index, input) in common["sets"].as_array().unwrap().iter().enumerate() {
            count += 1;
            let expected = own["diff"]
                .get(index.to_string())
                .unwrap_or(&common["common"][index]);
            let actual = run_set(&validator, input);
            if &actual != expected {
                wrong.push(format!(
                    "{format} #{index} {input}:\n  engine   {actual}\n  showdown {expected}"
                ));
            }
        }
    }
    let shown: Vec<_> = wrong.iter().take(10).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong of {count}:\n{}",
        wrong.len(),
        shown.join("\n")
    );
    assert!(count > 8_000, "{count} sets");
}

/// Each problem says where it is and which rule it breaks (DESIGN.md §45.6): the set (none for
/// the team's), the part of the set (a move by its index), the rule table's key of a ban or the
/// team rule's id — the words staying Showdown's.
#[test]
fn every_problem_says_where_it_is() {
    use pokemon_battle_engine::format::{ProblemLocation, SetField, Stats, TeamSet};
    let validator = TeamValidator::new(library(), "gen9ou").unwrap();
    let set = |species: &str, ability: &str, moves: &[&str]| TeamSet {
        species: species.into(),
        ability: ability.into(),
        moves: moves.iter().map(|m| m.to_string()).collect(),
        evs: Some(Stats {
            hp: 4,
            ..Stats::default()
        }),
        ..TeamSet::default()
    };
    let mut over = set("Snorlax", "Thick Fat", &["Body Slam"]);
    over.evs = Some(Stats::filled(252));
    let mut team = vec![
        set("Pikachu", "Static", &["Thunderbolt", "Spore"]),
        set("Koraidon", "Orichalcum Pulse", &["Flare Blitz"]),
        over,
        set("Pikachu", "Static", &["Thunderbolt"]),
    ];
    let problems = validator.validate_team(&mut team);
    let found: Vec<String> = problems
        .iter()
        .map(|p| format!("{:?} {:?} {}", p.location, p.rule, p.message))
        .collect();
    let at = |set, field| ProblemLocation {
        set: Some(set),
        field: Some(field),
    };
    let find = |text: &str| {
        problems
            .iter()
            .find(|p| p.message.contains(text))
            .unwrap_or_else(|| panic!("{text}: {found:?}"))
    };
    assert_eq!(find("Spore").location, at(0, SetField::Move(1)));
    let uber = find("Koraidon");
    assert_eq!(uber.location, at(1, SetField::Species));
    assert_eq!(uber.rule.as_deref(), Some("-pokemontag:uber"));
    assert_eq!(find("total EVs").location, at(2, SetField::Evs));
    let clause = find("Species Clause");
    assert_eq!(clause.location.set, None);
    assert_eq!(clause.rule.as_deref(), Some("speciesclause"));
}
