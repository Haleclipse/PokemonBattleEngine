//! Stat-formula micro-oracle: `calculate_stats` against pinned `@smogon/calc` results.

mod support;

use std::path::PathBuf;

use pokemon_battle_engine::dex::{DexResolver, Nature, StatSpread, calculate_stats};
use pokemon_battle_engine::spec::Generation;
use support::{StatsExpectations, StatsRow, load_json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn generation(number: u8) -> Generation {
    match number {
        1 => Generation::Gen1,
        2 => Generation::Gen2,
        3 => Generation::Gen3,
        4 => Generation::Gen4,
        5 => Generation::Gen5,
        6 => Generation::Gen6,
        7 => Generation::Gen7,
        8 => Generation::Gen8,
        9 => Generation::Gen9,
        other => panic!("unsupported generation {other}"),
    }
}

fn spread(row: StatsRow) -> StatSpread {
    let clamp = |v: u16| u8::try_from(v).expect("EV/IV fits in u8");
    StatSpread {
        hp: clamp(row.hp),
        atk: clamp(row.atk),
        def: clamp(row.def),
        spa: clamp(row.spa),
        spd: clamp(row.spd),
        spe: clamp(row.spe),
    }
}

#[test]
fn stat_formula_agrees_with_smogon_calc() {
    let expectations: StatsExpectations =
        load_json(&root().join("tests/fixtures/oracle/stats-expected.json"));
    assert_eq!(expectations.provenance.engine, "@smogon/calc");

    let mut report = Vec::new();
    for case in &expectations.cases {
        let dex = DexResolver::resolve(generation(case.generation));
        let species = dex.species_by_name(&case.species).unwrap_or_else(|| {
            panic!(
                "{}: {} missing from gen{}",
                case.id, case.species, case.generation
            )
        });
        let nature = Nature::from_name(&case.calc_inputs.nature)
            .unwrap_or_else(|| panic!("{}: unknown nature {}", case.id, case.calc_inputs.nature));
        // Feed calc's effective EVs / IVs, but leave the HP IV at 31 in Gen 1–2 so the engine's
        // own DV derivation (not calc's) produces the value under test.
        let mut ivs = spread(case.calc_inputs.ivs);
        if case.generation <= 2 {
            ivs.hp = 31;
        }
        let (max_hp, stats) = calculate_stats(
            generation(case.generation),
            species.base_stats,
            case.level,
            nature,
            spread(case.calc_inputs.evs),
            ivs,
        );
        let rust = StatsRow {
            hp: max_hp,
            atk: stats.attack,
            def: stats.defense,
            spa: stats.special_attack,
            spd: stats.special_defense,
            spe: stats.speed,
        };
        if rust != case.expected.raw_stats || max_hp != case.expected.max_hp {
            report.push(format!(
                "{}: rust={rust:?} calc={:?}",
                case.id, case.expected.raw_stats
            ));
        }
    }
    assert!(
        report.is_empty(),
        "stat formula disagrees with @smogon/calc in {} of {} case(s):\n{}",
        report.len(),
        expectations.cases.len(),
        report.join("\n")
    );
}
