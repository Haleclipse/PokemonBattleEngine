mod support;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::data;
use pokemon_battle_engine::dex::{DexResolver, PokemonType, ResolvedDex, StatusMoveEffect};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{AbilityId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleState, BattleStats, MoveSlotState, PokemonState, SideState, WeatherKind,
};
use support::{
    CoverageLedger, CoverageStatus, FixtureManifest, assert_semantic_eq, capture, load_json,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Collects every `fn name(` declared in the integration test sources. The coverage ledger
/// points at these names, so a renamed test must not silently orphan a ledger entry.
fn declared_test_functions() -> HashSet<String> {
    let mut names = HashSet::new();
    let entries = std::fs::read_dir(root().join("tests")).expect("tests directory");
    for entry in entries {
        let path = entry.expect("tests directory entry").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for line in source.lines() {
            let Some(rest) = line.trim_start().strip_prefix("fn ") else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                names.insert(name);
            }
        }
    }
    names
}

#[test]
fn oracle_manifest_is_parseable_and_pinned() {
    let manifest: FixtureManifest = load_json(&root().join("tests/fixtures/manifest.json"));
    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.oracle_schema, 1);
    assert_eq!(
        manifest.provenance.showdown.as_ref().unwrap().version,
        "0.10.11"
    );
    assert_eq!(
        manifest.provenance.damage.as_ref().unwrap().version,
        "0.11.0"
    );
}

#[test]
fn coverage_ledger_has_unique_mechanics_and_verified_entries_have_tests() {
    let ledger: CoverageLedger = load_json(&root().join("tests/coverage/mechanics.json"));
    let mut ids = HashSet::new();
    for mechanic in ledger.mechanics {
        assert!(
            ids.insert(mechanic.id.clone()),
            "duplicate mechanic coverage id: {}",
            mechanic.id
        );
        if mechanic.status == CoverageStatus::Verified {
            assert!(
                !mechanic.local_tests.is_empty(),
                "verified mechanic must name at least one local test: {}",
                mechanic.id
            );
        }
    }
}

#[test]
fn coverage_ledger_local_tests_reference_real_test_functions() {
    let ledger: CoverageLedger = load_json(&root().join("tests/coverage/mechanics.json"));
    let declared = declared_test_functions();
    let mut missing = Vec::new();
    for mechanic in &ledger.mechanics {
        for test in &mechanic.local_tests {
            if !declared.contains(test) {
                missing.push(format!("{} -> {test}", mechanic.id));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "coverage ledger references tests that do not exist:\n{}",
        missing.join("\n")
    );
}

#[test]
fn semantic_comparator_reports_first_field_path() {
    #[derive(serde::Serialize)]
    struct Example {
        hp: u16,
        status: &'static str,
    }

    let expected = Example {
        hp: 100,
        status: "none",
    };
    let actual = Example {
        hp: 99,
        status: "none",
    };
    let diffs = support::compare_json(&expected, &actual, &[]);
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].path, "$.hp");
}

#[test]
fn coverage_pool_prioritizes_unexercised_mechanics() {
    let mut pool = support::CoveragePool::new(["tackle", "protect", "tailwind"]);
    pool.mark_used("tackle");
    assert_eq!(pool.count("tackle"), Some(1));
    assert_eq!(pool.count("unknown"), None);
    assert_eq!(pool.least_covered(), vec!["protect", "tailwind"]);
    pool.mark_used("protect");
    pool.mark_used("tailwind");
    assert_eq!(pool.cycles_completed(), 1);
}

/// The SemanticFrame projection is the Rust side of the cross-engine contract. This test pins
/// its observable shape at request boundaries and checks that snapshot → restore yields an
/// identical frame, which is the L0 "snapshot/restore identity" property from TESTING.md.
#[test]
fn semantic_frame_projection_is_stable_across_request_boundaries_and_restore() {
    let (rules, dex, mut engine, choices) = singles_fixture(7);

    let step = engine.step().unwrap();
    let initial = capture(&engine, "initial", Some(&step));
    assert_eq!(initial.turn, 1);
    assert_eq!(initial.request.as_deref(), Some("turn_choices"));
    assert_eq!(initial.outcome, None);
    assert_eq!(initial.sides[0].active, vec![Some(0)]);
    assert_eq!(initial.sides[1].active, vec![Some(0)]);
    assert_eq!(
        initial.sides[1].party[0].hp,
        initial.sides[1].party[0].max_hp
    );

    engine.submit_turn_choices(choices).unwrap();
    let boundary = loop {
        match engine.step().unwrap() {
            EngineStep::Progressed { .. } => continue,
            other => break other,
        }
    };
    let end_of_turn = capture(&engine, "boundary_1", Some(&boundary));
    assert_eq!(end_of_turn.turn, 2);
    assert_eq!(end_of_turn.request.as_deref(), Some("turn_choices"));
    assert!(end_of_turn.sides[1].party[0].hp < end_of_turn.sides[1].party[0].max_hp);
    assert!(end_of_turn.sides[0].party[0].hp < end_of_turn.sides[0].party[0].max_hp);

    let restored = BattleEngine::restore(rules, dex, engine.snapshot()).unwrap();
    let restored_frame = capture(&restored, "boundary_1", Some(&boundary));
    assert_semantic_eq(&end_of_turn, &restored_frame, &[]);
}

const GENERATIONS: [Generation; 9] = [
    Generation::Gen1,
    Generation::Gen2,
    Generation::Gen3,
    Generation::Gen4,
    Generation::Gen5,
    Generation::Gen6,
    Generation::Gen7,
    Generation::Gen8,
    Generation::Gen9,
];

/// The committed `@pkmn/data` snapshots under `data/dex` are the canonical data source. The
/// resolver must preserve them without loss for every generation: every snapshot move, ability
/// and item with a representable id resolves, typed fields equal the snapshot's, and name lookup
/// round-trips through Showdown's `toID` normalisation.
#[test]
fn resolved_dex_preserves_snapshot_data_for_every_generation() {
    let mut report = Vec::new();
    for generation in GENERATIONS {
        let number = generation.number();
        let snapshot = data::snapshot(generation);
        assert_eq!(snapshot.generation, number);
        assert_eq!(snapshot.provenance.engine, "@pkmn/data");
        let dex = DexResolver::resolve(generation);

        for row in &snapshot.moves {
            let num = row.eid;
            let Some(resolved) = dex.move_by_id(MoveId(num)) else {
                if PokemonType::from_name(&row.type_name).is_some() {
                    report.push(format!("gen{number} {}: not resolved", row.name));
                }
                continue;
            };
            let mut check = |field: &str, oracle: String, rust: String| {
                if oracle != rust {
                    report.push(format!(
                        "gen{number} {} {field}: snapshot={oracle} rust={rust}",
                        row.name
                    ));
                }
            };
            check("name", row.name.clone(), resolved.name.clone());
            // Struggle's `onModifyMove` makes it typeless (`MoveBehavior::typeless`, Gen 2+).
            let expected_type = if resolved.behavior.typeless {
                "???".to_string()
            } else {
                row.type_name.clone()
            };
            check("type", expected_type, resolved.move_type.name().to_string());
            check(
                "category",
                row.category.clone(),
                format!("{:?}", resolved.category),
            );
            check("bp", row.bp.to_string(), resolved.base_power.to_string());
            check(
                "acc",
                format!("{:?}", row.acc),
                format!("{:?}", resolved.accuracy),
            );
            check("pp", row.pp.to_string(), resolved.max_pp.to_string());
            check("pri", row.pri.to_string(), resolved.priority.to_string());
            if dex.move_by_name(&row.name).map(|m| m.id) != Some(MoveId(num)) {
                report.push(format!(
                    "gen{number} {}: name lookup does not round-trip",
                    row.name
                ));
            }
        }
        for row in &snapshot.abilities {
            match dex.ability_by_id(AbilityId(row.eid)) {
                Some(resolved) if resolved.name == row.name => {}
                other => report.push(format!("gen{number} ability {}: {other:?}", row.name)),
            }
        }
        for row in &snapshot.items {
            match dex.item_by_id(pokemon_battle_engine::ids::ItemId(row.eid)) {
                Some(resolved) if resolved.name == row.name => {}
                other => report.push(format!("gen{number} item {}: {other:?}", row.name)),
            }
        }
        if dex.species().len() != snapshot.species.len() {
            report.push(format!(
                "gen{number} species: snapshot={} rust={}",
                snapshot.species.len(),
                dex.species().len()
            ));
        }
    }
    assert!(
        report.is_empty(),
        "ResolvedDex loses snapshot data in {} place(s):\n{}",
        report.len(),
        report.join("\n")
    );
}

/// The id contract (DESIGN.md §27.1, data/dex/README.md), checked over all nine generations:
/// moves and abilities carry the in-game index (PokéAPI primary key) or sit in the reserved range;
/// items are generation-ordered (`gen * 1000 + n`); species are dex number + forme index with the
/// base forme at 0. Guards the registry against ever inheriting a data-source bug or drifting.
#[test]
fn engine_ids_follow_the_registry_contract() {
    use pokemon_battle_engine::ids::{ItemId, SpeciesId};
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(serde::Deserialize)]
    struct OfficialIndex {
        abilities: BTreeMap<String, u16>,
        moves: BTreeMap<String, u16>,
    }
    const RESERVED: u16 = 60_000;
    let index: OfficialIndex = load_json(&root().join("data/dex/official-index.json"));
    assert!(index.abilities.len() > 300 && index.moves.len() > 900);

    let mut report = Vec::new();
    let mut first_item_gen: BTreeMap<String, u8> = BTreeMap::new();
    for generation in GENERATIONS {
        let number = generation.number();
        let snapshot = data::snapshot(generation);

        for row in &snapshot.moves {
            match index.moves.get(&row.id) {
                Some(official) if *official != row.eid => report.push(format!(
                    "gen{number} move {}: eid={} official={official}",
                    row.id, row.eid
                )),
                None if row.eid < RESERVED => report.push(format!(
                    "gen{number} move {}: no official index but eid={} is outside the reserved range",
                    row.id, row.eid
                )),
                _ => {}
            }
        }
        for row in &snapshot.abilities {
            match index.abilities.get(&row.id) {
                Some(official) if *official != row.eid => report.push(format!(
                    "gen{number} ability {}: eid={} official={official}",
                    row.id, row.eid
                )),
                None if row.eid < RESERVED => report.push(format!(
                    "gen{number} ability {}: no official index but eid={} is outside the reserved range",
                    row.id, row.eid
                )),
                _ => {}
            }
        }
        for row in &snapshot.items {
            let first = *first_item_gen.entry(row.id.clone()).or_insert(number);
            if row.generation != first || row.eid / 1000 != u16::from(first) || row.eid % 1000 == 0
            {
                report.push(format!(
                    "gen{number} item {}: eid={} gen={} but first appears in gen{first}",
                    row.id, row.eid, row.generation
                ));
            }
        }

        let dex = DexResolver::resolve(generation);
        let mut seen: BTreeSet<SpeciesId> = BTreeSet::new();
        for (row, resolved) in snapshot.species.iter().zip(dex.species()) {
            let is_base = row.forme.is_none();
            if resolved.id.dex != u16::try_from(row.num).unwrap_or(0) {
                report.push(format!("gen{number} species {}: dex mismatch", row.id));
            }
            if is_base != resolved.id.is_base() {
                report.push(format!(
                    "gen{number} species {}: forme index {} but base={is_base}",
                    row.id, resolved.id.form
                ));
            }
            if !seen.insert(resolved.id) {
                report.push(format!(
                    "gen{number} species {}: duplicate id {}",
                    row.id, resolved.id
                ));
            }
            if dex.species_by_id(resolved.id).map(|s| &s.key) != Some(&row.id) {
                report.push(format!(
                    "gen{number} species {}: id lookup does not round-trip",
                    row.id
                ));
            }
        }
    }
    assert!(
        report.is_empty(),
        "engine ids break the registry contract in {} place(s):\n{}",
        report.len(),
        report.join("\n")
    );

    // Concrete anchors.
    let gen9 = DexResolver::resolve(Generation::Gen9);
    for (name, official) in [
        ("Vessel of Ruin", 284),
        ("Sword of Ruin", 285),
        ("Tablets of Ruin", 286),
        ("Beads of Ruin", 287),
    ] {
        assert_eq!(
            gen9.ability_by_name(name).unwrap().id,
            AbilityId(official),
            "{name}"
        );
    }
    let band = gen9.item_by_name("Choice Band").unwrap().id;
    assert_eq!(band, ItemId::CHOICE_BAND);
    assert_eq!(band.0 / 1000, 3, "Choice Band was introduced in Gen 3");

    let gen6 = DexResolver::resolve(Generation::Gen6);
    let charizard = gen6.species_by_name("Charizard").unwrap();
    let mega_x = gen6.species_by_name("Charizard-Mega-X").unwrap();
    let mega_y = gen6.species_by_name("Charizard-Mega-Y").unwrap();
    assert_eq!(charizard.id, SpeciesId::base(6));
    assert_eq!(mega_x.id, SpeciesId { dex: 6, form: 1 });
    assert_eq!(mega_y.id, SpeciesId { dex: 6, form: 2 });
    assert_eq!(
        mega_x.forme_kind,
        pokemon_battle_engine::dex::FormeKind::Mega
    );
    assert!(mega_x.id.same_species(charizard.id));
    assert_eq!(
        gen9.species_by_name("Raichu-Alola").unwrap().forme_kind,
        pokemon_battle_engine::dex::FormeKind::Regional
    );
}

/// The id constants are generated from the registry (build.rs): each names the entry whose
/// English name it spells, including the awkward ones.
#[test]
fn generated_id_constants_name_their_registry_entries() {
    use pokemon_battle_engine::ids::ItemId;

    let gen9 = DexResolver::resolve(Generation::Gen9);
    let gen7 = DexResolver::resolve(Generation::Gen7);
    for (id, name) in [
        (MoveId::TACKLE, "Tackle"),
        (MoveId::U_TURN, "U-turn"),
        (MoveId::WILL_O_WISP, "Will-O-Wisp"),
        (MoveId::STOMPING_TANTRUM, "Stomping Tantrum"),
    ] {
        assert_eq!(gen9.move_by_name(name).map(|m| m.id), Some(id), "{name}");
    }
    assert_eq!(
        gen7.move_by_name("10,000,000 Volt Thunderbolt")
            .map(|m| m.id),
        Some(MoveId::N10_000_000_VOLT_THUNDERBOLT)
    );
    assert_eq!(
        gen7.move_by_name("King's Shield").map(|m| m.id),
        Some(MoveId::KINGS_SHIELD)
    );
    for (id, name) in [
        (AbilityId::INTIMIDATE, "Intimidate"),
        (AbilityId::AS_ONE_GLASTRIER, "As One (Glastrier)"),
    ] {
        assert_eq!(gen9.ability_by_name(name).map(|a| a.id), Some(id), "{name}");
    }
    for (id, name) in [
        (ItemId::CHOICE_BAND, "Choice Band"),
        (ItemId::NEVER_MELT_ICE, "Never-Melt Ice"),
        (ItemId::KINGS_ROCK, "King's Rock"),
    ] {
        assert_eq!(gen9.item_by_name(name).map(|i| i.id), Some(id), "{name}");
    }
}

/// Advisory drift check between the two structured sources (TESTING.md §1): PokéAPI's CSV view
/// of Gen 9 species and moves against the @pkmn/data snapshot the engine runs on. Entries only
/// one source names are skipped; known, classified disagreements are listed here so a new one
/// fails loudly.
#[test]
fn gen9_snapshot_agrees_with_pokeapi_crosscheck() {
    use support::PokeapiCrosscheck;

    // species: Showdown's "Minior" is the Core forme, PokéAPI's default is Meteor (stats differ).
    const KNOWN_SPECIES: &[&str] = &["minior"];
    // moves: Take Heart PP — Showdown 15, PokéAPI 10. Unadjudicated; Bulbapedia to decide.
    const KNOWN_MOVES: &[&str] = &["takeheart"];

    let cross: PokeapiCrosscheck = load_json(&root().join("data/dex/crosscheck-pokeapi.json"));
    let snapshot = data::snapshot(Generation::Gen9);
    let mut report = Vec::new();
    let mut compared = 0usize;

    for row in &snapshot.species {
        let Some(theirs) = cross.species.get(&row.id) else {
            continue;
        };
        compared += 1;
        if KNOWN_SPECIES.contains(&row.id.as_str()) {
            continue;
        }
        let mine: std::collections::BTreeMap<String, u16> = [
            ("hp", row.base_stats.hp),
            ("atk", row.base_stats.atk),
            ("def", row.base_stats.def),
            ("spa", row.base_stats.spa),
            ("spd", row.base_stats.spd),
            ("spe", row.base_stats.spe),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        if mine != theirs.base_stats {
            report.push(format!(
                "species {}: base stats {mine:?} vs pokeapi {:?}",
                row.id, theirs.base_stats
            ));
        }
        if row.types != theirs.types {
            report.push(format!(
                "species {}: types {:?} vs pokeapi {:?}",
                row.id, row.types, theirs.types
            ));
        }
    }
    for row in &snapshot.moves {
        let Some(theirs) = cross.moves.get(&row.id) else {
            continue;
        };
        compared += 1;
        if KNOWN_MOVES.contains(&row.id.as_str()) {
            continue;
        }
        let mine = (
            row.type_name.as_str(),
            row.category.as_str(),
            row.bp,
            row.acc,
            row.pp,
            row.pri,
        );
        let other = (
            theirs.type_name.as_str(),
            theirs.category.as_str(),
            theirs.bp,
            theirs.acc,
            theirs.pp,
            theirs.pri,
        );
        if mine != other {
            report.push(format!("move {}: {mine:?} vs pokeapi {other:?}", row.id));
        }
    }
    assert!(
        compared > 1400,
        "cross-check compared only {compared} entries"
    );
    assert!(
        report.is_empty(),
        "unclassified drift between @pkmn/data and PokéAPI in {} place(s):\n{}",
        report.len(),
        report.join("\n")
    );
}

/// Every id the behavior tables register must be a real snapshot entry with the expected
/// name, and must classify as Custom / Damage / implemented. Guards against a behavior silently
/// pointing at the wrong Showdown `num`.
#[test]
fn behavior_tables_reference_real_snapshot_entries() {
    use pokemon_battle_engine::dex::PrimaryEffect;
    use pokemon_battle_engine::ids::ItemId;

    let gen9 = DexResolver::resolve(Generation::Gen9);
    for (id, name, expected) in [
        (MoveId::TACKLE, "Tackle", PrimaryEffect::Damage),
        (MoveId::FLAMETHROWER, "Flamethrower", PrimaryEffect::Damage),
        (MoveId::THUNDERBOLT, "Thunderbolt", PrimaryEffect::Damage),
        (MoveId::EARTHQUAKE, "Earthquake", PrimaryEffect::Damage),
        (MoveId::ACROBATICS, "Acrobatics", PrimaryEffect::Damage),
        (MoveId::NUZZLE, "Nuzzle", PrimaryEffect::Damage),
        (MoveId::PROTECT, "Protect", PrimaryEffect::Custom),
        (MoveId::SUBSTITUTE, "Substitute", PrimaryEffect::Custom),
        (MoveId::STEALTH_ROCK, "Stealth Rock", PrimaryEffect::Custom),
        (MoveId::TAILWIND, "Tailwind", PrimaryEffect::Custom),
        (MoveId::TRICK_ROOM, "Trick Room", PrimaryEffect::Custom),
        (
            MoveId::SUNNY_DAY,
            "Sunny Day",
            PrimaryEffect::Status(StatusMoveEffect::Weather(WeatherKind::Sun)),
        ),
        (
            MoveId::RAIN_DANCE,
            "Rain Dance",
            PrimaryEffect::Status(StatusMoveEffect::Weather(WeatherKind::Rain)),
        ),
        (
            MoveId::SANDSTORM,
            "Sandstorm",
            PrimaryEffect::Status(StatusMoveEffect::Weather(WeatherKind::Sandstorm)),
        ),
        (
            MoveId::SNOWSCAPE,
            "Snowscape",
            PrimaryEffect::Status(StatusMoveEffect::Weather(WeatherKind::Snow)),
        ),
        (
            MoveId::ELECTRIC_TERRAIN,
            "Electric Terrain",
            PrimaryEffect::Custom,
        ),
    ] {
        let resolved = gen9
            .move_by_id(id)
            .unwrap_or_else(|| panic!("{name} ({id:?}) missing from the gen9 snapshot"));
        assert_eq!(resolved.name, name);
        assert_eq!(
            resolved.primary_effect, expected,
            "{name}: {:?}",
            resolved.unsupported
        );
    }

    // Power-Up Punch was removed from the cartridge in Gen 9; Gen 8 is its last generation.
    assert!(gen9.move_by_id(MoveId::POWER_UP_PUNCH).is_none());
    let gen8 = DexResolver::resolve(Generation::Gen8);
    let punch = gen8.move_by_id(MoveId::POWER_UP_PUNCH).unwrap();
    assert_eq!(punch.name, "Power-Up Punch");
    assert_eq!(
        punch.primary_effect,
        PrimaryEffect::Damage,
        "{:?}",
        punch.unsupported
    );

    for (id, name) in [
        (AbilityId::INTIMIDATE, "Intimidate"),
        (AbilityId::LEVITATE, "Levitate"),
        (AbilityId::HUGE_POWER, "Huge Power"),
        (AbilityId::TECHNICIAN, "Technician"),
        (AbilityId::DROUGHT, "Drought"),
        (AbilityId::DRIZZLE, "Drizzle"),
        (AbilityId::SAND_STREAM, "Sand Stream"),
        (AbilityId::SNOW_WARNING, "Snow Warning"),
    ] {
        let resolved = gen9
            .ability_by_id(id)
            .unwrap_or_else(|| panic!("{name} ({id:?}) missing from the gen9 snapshot"));
        assert_eq!(resolved.name, name);
        assert!(
            resolved.implemented,
            "{name}: {:?}",
            resolved.missing_callbacks
        );
    }
    // White Herb is complete since O.3 (its `onAnyAfterMega` arrived with Mega Evolution);
    // a partially covered one still names its remaining callbacks.
    let herb = gen9.item_by_name("White Herb").unwrap();
    assert!(herb.implemented, "{:?}", herb.missing_callbacks);
    let mail = gen9
        .item_by_name("Mail")
        .or_else(|| gen9.item_by_name("Grass Mail"));
    if let Some(mail) = mail {
        assert_eq!(mail.missing_callbacks.len(), mail.missing_callbacks.len());
    }
    // Sturdy's `onTryHit` (one-hit KO immunity) completes it (M7); Battle Armor's value-form
    // `onCriticalHit: false` is a declared callback like any other.
    let sturdy = gen9.ability_by_id(AbilityId::STURDY).unwrap();
    assert!(sturdy.implemented, "{:?}", sturdy.missing_callbacks);
    let armor = gen9.ability_by_name("Battle Armor").unwrap();
    assert!(armor.implemented, "{:?}", armor.missing_callbacks);
    // Choice Band covers all three of its callbacks (onModifyAtk, onModifyMove = the lock,
    // onStart = clearing it on entry).
    let band = gen9.item_by_id(ItemId::CHOICE_BAND).unwrap();
    assert_eq!(band.name, "Choice Band");
    assert!(band.implemented, "{:?}", band.missing_callbacks);
    for name in ["Leftovers", "Rocky Helmet", "Focus Sash"] {
        assert!(gen9.item_by_name(name).unwrap().implemented, "{name}");
    }
}

/// A move the engine has not implemented is refused at choice validation instead of reaching
/// the pipeline and doing nothing.
#[test]
fn unsupported_moves_are_rejected_at_choice_validation() {
    use pokemon_battle_engine::dex::PrimaryEffect;
    use pokemon_battle_engine::engine::ChoiceError;

    // The queue shrinks batch by batch — Gen 9 has no unsupported move left at all — so the
    // test walks the generations backwards for the first damaging one it finds. With the M7
    // inventory empty (R-N.7) there is no unsupported move anywhere: the refusal has no input
    // left to check, and the test stands down.
    let Some((generation, rules, dex, unsupported)) = [
        Generation::Gen9,
        Generation::Gen8,
        Generation::Gen7,
        Generation::Gen6,
        Generation::Gen5,
        Generation::Gen4,
        Generation::Gen3,
    ]
    .into_iter()
    .find_map(|generation| {
        let rules = Arc::new(
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .unwrap(),
        );
        let dex = Arc::new(DexResolver::resolve(generation));
        let unsupported = dex
            .moves()
            .iter()
            .find(|m| {
                m.primary_effect == PrimaryEffect::Unsupported
                    && m.category != pokemon_battle_engine::dex::MoveCategory::Status
            })
            .cloned()?;
        Some((generation, rules, dex, unsupported))
    }) else {
        return;
    };
    let _ = generation;
    assert!(!unsupported.unsupported.is_empty());

    let a = PokemonId(1);
    let b = PokemonId(2);
    let mut attacker = pokemon(a, SideId::P1, 100);
    attacker.moves = vec![MoveSlotState::new(unsupported.id, 5)];
    let state = BattleState::new(
        vec![attacker, pokemon(b, SideId::P2, 90)],
        [side(a), side(b)],
        1,
    );
    let engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let target = DeclaredTarget(rules.topology().position(SideId::P2, 0).unwrap());
    assert_eq!(
        engine.validate_choice(SideId::P1, &choice(SideId::P1, a, target)),
        Err(ChoiceError::UnsupportedMove(unsupported.id))
    );
}

fn singles_fixture(
    seed: u64,
) -> (
    Arc<ResolvedRules>,
    Arc<ResolvedDex>,
    BattleEngine,
    [SideChoice; 2],
) {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let a = PokemonId(1);
    let b = PokemonId(2);
    let state = BattleState::new(
        vec![pokemon(a, SideId::P1, 100), pokemon(b, SideId::P2, 90)],
        [side(a), side(b)],
        seed,
    );
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let a_target = DeclaredTarget(rules.topology().position(SideId::P2, 0).unwrap());
    let b_target = DeclaredTarget(rules.topology().position(SideId::P1, 0).unwrap());
    let choices = [
        choice(SideId::P1, a, a_target),
        choice(SideId::P2, b, b_target),
    ];
    (rules, dex, engine, choices)
}

fn pokemon(id: PokemonId, side: SideId, speed: u16) -> PokemonState {
    let mut pokemon = PokemonState::new(id, side);
    pokemon.level = 50;
    pokemon.hp = 300;
    pokemon.max_hp = 300;
    pokemon.stats = BattleStats {
        speed,
        ..BattleStats::default()
    };
    pokemon.moves = vec![MoveSlotState::new(MoveId::TACKLE, 35)];
    pokemon
}

fn side(active: PokemonId) -> SideState {
    let mut side = SideState::new(vec![active], 1);
    side.active[0] = Some(active);
    side
}

fn choice(side: SideId, actor: PokemonId, target: DeclaredTarget) -> SideChoice {
    SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: Some(target),
                mechanic: None,
            },
        }],
    }
}

#[test]
fn an_optional_semantic_decision_is_passed_over_when_the_engine_does_not_ask() {
    use pokemon_battle_engine::state::{BattleRng, RandomOverride, RandomRequestKind};
    // Gen 1-2 roll no damage variance for a damage of 1: the replay plans the roll as optional.
    let mut rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::range(RandomRequestKind::DamageRoll, 230).optional(),
            RandomOverride::bool(RandomRequestKind::Secondary, true),
        ],
    );
    assert!(rng.roll_percent_for(RandomRequestKind::Secondary, 30));
    assert!(rng.semantic_tape_exhausted());
    // Asked for, it is drawn like any other; left over at the end, the tape still counts as
    // used up.
    let mut rng = BattleRng::with_semantic_tape(
        1,
        vec![RandomOverride::range(RandomRequestKind::DamageRoll, 230).optional()],
    );
    assert!(rng.semantic_tape_exhausted());
    assert_eq!(
        rng.range_inclusive_for(RandomRequestKind::DamageRoll, 217, 255),
        230
    );
}
