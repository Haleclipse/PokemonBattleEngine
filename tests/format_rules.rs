//! The format resolver against Showdown (DESIGN.md §45.2): every target format's rule table,
//! item by item and in Showdown's order, as `tools/oracle-node/src/export-formats.mjs` exported
//! it from the pinned @pkmn/sim (`tests/fixtures/formats/ruletables.json`).

use pokemon_battle_engine::format::{
    ComplexBan, DataError, FORMATS_SCHEMA, FormatError, FormatLibrary, FormatResolver, RuleTable,
};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/formats/ruletables.json");

/// The committed data directory, read at run time as a host's would be (DESIGN.md §46).
const DATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data");

fn library() -> &'static FormatLibrary {
    static LIBRARY: std::sync::OnceLock<FormatLibrary> = std::sync::OnceLock::new();
    LIBRARY.get_or_init(|| FormatLibrary::open(DATA_DIR).unwrap_or_else(|error| panic!("{error}")))
}

fn resolver() -> FormatResolver<'static> {
    FormatResolver::new(library())
}

fn pairs(map: &pokemon_battle_engine::format::OrderedMap) -> Value {
    Value::Array(map.iter().map(|(key, value)| json!([key, value])).collect())
}

fn bans(bans: &[ComplexBan]) -> Value {
    Value::Array(
        bans.iter()
            .map(|ban| json!([ban.rule, ban.source, ban.limit, ban.bans]))
            .collect(),
    )
}

fn as_fixture(table: &RuleTable) -> Value {
    let numbers = &table.numbers;
    json!({
        "entries": pairs(&table.entries),
        "value_rules": pairs(&table.value_rules),
        "tag_rules": table.tag_rules,
        "complex_bans": bans(&table.complex_bans),
        "complex_team_bans": bans(&table.complex_team_bans),
        "numbers": {
            "min_team_size": numbers.min_team_size,
            "max_team_size": numbers.max_team_size,
            "picked_team_size": numbers.picked_team_size,
            "max_total_level": numbers.max_total_level,
            "max_move_count": numbers.max_move_count,
            "min_source_gen": numbers.min_source_gen,
            "min_level": numbers.min_level,
            "max_level": numbers.max_level,
            "default_level": numbers.default_level,
            "adjust_level": numbers.adjust_level,
            "adjust_level_down": numbers.adjust_level_down,
            "ev_limit": numbers.ev_limit,
        },
    })
}

#[test]
fn every_target_format_resolves_as_showdown_does() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let tables = fixture["tables"].as_object().unwrap();
    let data = library().formats();
    assert_eq!(
        tables.len(),
        data.formats.len(),
        "a table per target format"
    );
    let resolver = resolver();
    let mut wrong = Vec::new();
    for (id, expected) in tables {
        let table = resolver
            .resolve(id)
            .unwrap_or_else(|error| panic!("{id}: {error:?}"));
        let actual = as_fixture(&table);
        for key in expected.as_object().unwrap().keys() {
            if actual[key] != expected[key] {
                wrong.push(format!(
                    "{id}.{key}:\n  engine   {}\n  showdown {}",
                    actual[key], expected[key]
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_fixture_and_the_data_come_from_one_showdown() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let text = std::fs::read_to_string(library().root().join("formats/formats.json")).unwrap();
    let data: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(fixture["provenance"], data["provenance"]);
    assert_eq!(data["provenance"]["sim"], "0.10.11");
}

#[test]
fn a_fingerprint_names_the_rules() {
    let resolver = resolver();
    let data = library().formats();
    let mut seen = std::collections::HashMap::new();
    for format in &data.formats {
        let table = resolver.resolve(&format.id).unwrap();
        assert_eq!(
            table.fingerprint(),
            resolver.resolve(&format.id).unwrap().fingerprint()
        );
        if let Some(other) = seen.insert(table.fingerprint(), format.id.clone()) {
            panic!("{} and {other} share a fingerprint", format.id);
        }
    }
    // Where a rule came from is not part of it.
    let mut table = resolver.resolve("gen9ou").unwrap();
    let before = table.fingerprint();
    table.entries.set("speciesclause", "Somewhere Else");
    assert_eq!(table.fingerprint(), before);
    // A value is.
    let table = custom("gen9vgc2025regi", &["!! Adjust Level = 60"]).unwrap();
    assert_ne!(
        table.fingerprint(),
        resolver.resolve("gen9vgc2025regi").unwrap().fingerprint()
    );
}

fn custom(format: &str, rules: &[&str]) -> Result<RuleTable, FormatError> {
    let rules: Vec<String> = rules.iter().map(|rule| rule.to_string()).collect();
    resolver().resolve_custom(format, &rules)
}

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/formats")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Showdown's error, by its message, as the resolver's variant.
fn showdown_error(message: &str) -> &'static str {
    [
        ("Multiple \"", "MultipleRepeals"),
        ("is redundant", "Redundant"),
        ("conflicts with", "Conflicts"),
        ("requires Team Preview", "Invalid"),
        ("More than one thing matches", "AmbiguousBan"),
        ("Nothing matches", "UnresolvedBan"),
        ("None of your custom rules change anything", "NoChange"),
        ("before banning/unbanning all Pokemon", "PokemonRuleOrder"),
    ]
    .iter()
    .find(|(text, _)| message.contains(text))
    .map(|(_, variant)| *variant)
    .unwrap_or_else(|| panic!("an unclassified Showdown error: {message}"))
}

fn variant(error: &FormatError) -> String {
    format!("{error:?}")
        .split(['(', ' '])
        .next()
        .unwrap()
        .to_string()
}

/// Custom formats (`format@@@rules`), each against Showdown: the rule table item by item, or
/// the error (`tests/fixtures/formats/custom.json`, export-legality.mjs).
#[test]
fn custom_formats_resolve_as_showdown_does() {
    let mut wrong = Vec::new();
    for case in fixture("custom.json")["custom"].as_array().unwrap() {
        let format = case["format"].as_str().unwrap();
        let rules: Vec<&str> = case["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|rule| rule.as_str().unwrap())
            .collect();
        let actual = custom(format, &rules);
        match (&actual, case.get("error")) {
            (Ok(table), None) => {
                let actual = as_fixture(table);
                for key in case["table"].as_object().unwrap().keys() {
                    if actual[key] != case["table"][key] {
                        wrong.push(format!(
                            "{format} {rules:?} {key}:\n  engine   {}\n  showdown {}",
                            actual[key], case["table"][key]
                        ));
                    }
                }
            }
            (Err(error), Some(message)) => {
                let expected = showdown_error(message.as_str().unwrap());
                if variant(error) != expected {
                    wrong.push(format!("{format} {rules:?}: {error:?}, Showdown {message}"));
                }
            }
            (actual, expected) => {
                wrong.push(format!(
                    "{format} {rules:?}: {actual:?}, Showdown {expected:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// `validateBanRule` for every name each generation's dex knows, bare and with its kind, the
/// aliases and the tags (`tests/fixtures/formats/banrules.json`, export-legality.mjs).
#[test]
fn ban_names_resolve_as_showdown_does() {
    use pokemon_battle_engine::format::BanError;
    let aliases = library().aliases().unwrap();
    let mut wrong = Vec::new();
    let mut count = 0;
    for (mod_name, answers) in fixture("banrules.json")["answers"].as_object().unwrap() {
        let legality = library().legality(mod_name).unwrap();
        for (input, expected) in answers.as_object().unwrap() {
            count += 1;
            let actual = match legality.ban_spec(aliases, input) {
                Ok(spec) => json!(spec),
                Err(BanError::Nothing) => json!({"nothing": true}),
                Err(BanError::Ambiguous(specs)) => json!({"ambiguous": specs}),
            };
            if &actual != expected {
                wrong.push(format!(
                    "{mod_name} {input:?}: engine {actual}, showdown {expected}"
                ));
            }
        }
    }
    assert!(count > 80_000, "{count} names");
    let shown: Vec<_> = wrong.iter().take(40).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        shown.join("\n")
    );
}

/// `validateRule` for every rule or ban string the target formats use, under each generation's
/// dex (`tests/fixtures/formats/canonical.json`, export-formats.mjs).
#[test]
fn rule_strings_validate_as_showdown_does() {
    use pokemon_battle_engine::format::RuleSpec;
    let resolver = resolver();
    let mut wrong = Vec::new();
    for (mod_name, answers) in fixture("canonical.json")["canonical"].as_object().unwrap() {
        for (raw, expected) in answers.as_object().unwrap() {
            let actual = match resolver.validate_rule(mod_name, raw) {
                Ok(RuleSpec::Text(text)) => json!(text),
                Ok(RuleSpec::Complex {
                    team,
                    rule,
                    limit,
                    bans,
                }) => json!([
                    if team { "complexTeamBan" } else { "complexBan" },
                    rule,
                    "",
                    limit,
                    bans
                ]),
                Err(error) => json!(format!("{error:?}")),
            };
            if &actual != expected {
                wrong.push(format!(
                    "{mod_name} {raw:?}: engine {actual}, showdown {expected}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_format_data_is_read_from_the_directory_given() {
    let scratch = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("format-library");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(scratch.join("formats")).unwrap();
    // No file: unreadable, named.
    assert!(matches!(
        FormatLibrary::open(&scratch),
        Err(DataError::Unreadable { path, .. }) if path.ends_with("formats/formats.json")
    ));
    // Another schema: refused rather than misread.
    let text = std::fs::read_to_string(library().root().join("formats/formats.json")).unwrap();
    let mut other: Value = serde_json::from_str(&text).unwrap();
    other["schema"] = json!(FORMATS_SCHEMA + 1);
    std::fs::write(scratch.join("formats/formats.json"), other.to_string()).unwrap();
    assert!(matches!(
        FormatLibrary::open(&scratch),
        Err(DataError::Malformed { reason, .. }) if reason.contains("schema")
    ));
    // A host's own copy, edited: what the resolver reads is that copy. Gen 9 OU with Sleep
    // Clause Mod no longer repealed has it again.
    let mut edited: Value = serde_json::from_str(&text).unwrap();
    let gen9ou = edited["formats"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|format| format["id"] == "gen9ou")
        .unwrap();
    let ruleset = gen9ou["ruleset"].as_array_mut().unwrap();
    ruleset.retain(|rule| rule != "!Sleep Clause Mod");
    std::fs::write(scratch.join("formats/formats.json"), edited.to_string()).unwrap();
    let copy = FormatLibrary::open(&scratch).unwrap();
    // The names in a format's bans come with the legality data: without it, no table.
    assert!(matches!(
        FormatResolver::new(&copy).resolve("gen9ou"),
        Err(FormatError::Data(DataError::Unreadable { path, .. })) if path.ends_with("legality/gen9.json")
    ));
    std::fs::create_dir_all(scratch.join("legality")).unwrap();
    for file in ["gen9.json", "aliases.json"] {
        std::fs::copy(
            library().root().join("legality").join(file),
            scratch.join("legality").join(file),
        )
        .unwrap();
    }
    let copy = FormatLibrary::open(&scratch).unwrap();
    assert!(
        FormatResolver::new(&copy)
            .resolve("gen9ou")
            .unwrap()
            .has("sleepclausemod")
    );
    assert!(!resolver().resolve("gen9ou").unwrap().has("sleepclausemod"));
}

/// `isBannedSpecies` of every species each target format's dex knows: its own ban or unban,
/// its base species', the tags' (`tests/fixtures/formats/banned-species.json`).
#[test]
fn every_species_is_banned_as_showdown_bans_it() {
    let resolver = resolver();
    let mut wrong = Vec::new();
    for (format_id, banned) in fixture("banned-species.json")["banned"]
        .as_object()
        .unwrap()
    {
        let format = library().formats().format(format_id).unwrap();
        let table = resolver.resolve(format_id).unwrap();
        let legality = library()
            .legality(format.mod_name.as_deref().unwrap())
            .unwrap();
        let expected: std::collections::BTreeSet<&str> = banned
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect();
        for species in &legality.species {
            if table.is_banned_species(species) != expected.contains(species.id.as_str()) {
                wrong.push(format!("{format_id} {}", species.id));
            }
        }
    }
    let shown: Vec<_> = wrong.iter().take(40).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        shown.join("\n")
    );
}
