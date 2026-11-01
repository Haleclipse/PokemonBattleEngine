//! The gimmicks' move data (DESIGN.md §44.1): each damaging move's Z-Move power (Gen 7) and Max
//! Move power (Gen 8) against Showdown's own (`dex-moves.mjs` defaults and the move data's
//! overrides), exported by `tools/oracle-node/src/gimmick-powers.mjs` into
//! `tests/fixtures/oracle/gimmick-powers.json`; and what a status move's Z version adds.

use std::collections::BTreeMap;
use std::path::PathBuf;

use pokemon_battle_engine::data::to_id;
use pokemon_battle_engine::dex::{
    DexResolver, MoveCategory, MultiHit, PokemonType, ZStatusEffect, max_move_power, z_move_power,
};
use pokemon_battle_engine::spec::Generation;
use pokemon_battle_engine::state::Stat;
use serde_json::Value;

fn fixture(key: &str) -> BTreeMap<String, u16> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let json: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("tests/fixtures/oracle/gimmick-powers.json"))
            .expect("fixture"),
    )
    .expect("json");
    serde_json::from_value(json[key].clone()).expect(key)
}

/// Every move of the generation with a power of `power(move)`, by Showdown id.
fn engine_powers(
    generation: Generation,
    power: impl Fn(&pokemon_battle_engine::dex::ResolvedMove) -> Option<u16>,
) -> BTreeMap<String, u16> {
    DexResolver::resolve(generation)
        .moves()
        .iter()
        .filter_map(|m| power(m).map(|p| (to_id(&m.name), p)))
        .collect()
}

#[test]
fn z_move_powers_are_showdowns() {
    assert_eq!(engine_powers(Generation::Gen7, |m| m.z_power), fixture("z"));
}

#[test]
fn max_move_powers_are_showdowns() {
    assert_eq!(
        engine_powers(Generation::Gen8, |m| m.max_power),
        fixture("max")
    );
}

#[test]
fn only_the_gimmicks_generations_have_their_data() {
    for generation in [Generation::Gen6, Generation::Gen8, Generation::Gen9] {
        let dex = DexResolver::resolve(generation);
        assert!(
            dex.moves()
                .iter()
                .all(|m| m.z_power.is_none() && m.z_status.is_none()),
            "{generation:?} has no Z-Moves"
        );
    }
    for generation in [Generation::Gen7, Generation::Gen9] {
        let dex = DexResolver::resolve(generation);
        assert!(
            dex.moves().iter().all(|m| m.max_power.is_none()),
            "{generation:?} has no Max Moves"
        );
    }
}

#[test]
fn power_tables_by_step() {
    // Z: each step's lower bound, a multi-hit range counted three times, no power at all.
    for (base, power) in [
        (140, 200),
        (130, 195),
        (120, 190),
        (110, 185),
        (100, 180),
        (90, 175),
        (80, 160),
        (70, 140),
        (60, 120),
        (59, 100),
        (0, 100),
    ] {
        assert_eq!(z_move_power(base, MultiHit::Single), power, "Z {base}");
    }
    assert_eq!(z_move_power(25, MultiHit::Range { min: 2, max: 5 }), 140);
    assert_eq!(z_move_power(50, MultiHit::Fixed(2)), 100);
    // Max: the ordinary table, and Fighting / Poison's.
    for (base, power, low) in [
        (150, 150, 100),
        (110, 140, 95),
        (75, 130, 90),
        (65, 120, 85),
        (55, 110, 80),
        (45, 100, 75),
        (44, 90, 70),
    ] {
        assert_eq!(max_move_power(PokemonType::Fire, base), power, "Max {base}");
        assert_eq!(
            max_move_power(PokemonType::Fighting, base),
            low,
            "Max Knuckle {base}"
        );
        assert_eq!(
            max_move_power(PokemonType::Poison, base),
            low,
            "Max Ooze {base}"
        );
    }
    assert_eq!(max_move_power(PokemonType::Fighting, 0), 100);
}

#[test]
fn status_z_effects_come_from_the_move_data() {
    let dex = DexResolver::resolve(Generation::Gen7);
    let z = |name: &str| dex.move_by_name(name).expect(name).z_status.clone();
    assert_eq!(
        z("Splash"),
        Some(ZStatusEffect::Boosts(vec![(Stat::Attack, 3)]))
    );
    assert_eq!(z("Swords Dance"), Some(ZStatusEffect::ClearNegativeBoost));
    assert_eq!(z("Recover"), Some(ZStatusEffect::ClearNegativeBoost));
    assert_eq!(z("Memento"), Some(ZStatusEffect::HealReplacement));
    assert_eq!(z("Curse"), Some(ZStatusEffect::Curse));
    assert_eq!(z("Destiny Bond"), Some(ZStatusEffect::Redirect));
    assert_eq!(z("Foresight"), Some(ZStatusEffect::Crit2));
    assert_eq!(z("Belly Drum"), Some(ZStatusEffect::Heal));
    // Every status move whose data names a Z effect has one the engine reads.
    assert!(
        dex.moves()
            .iter()
            .filter(|m| m.category == MoveCategory::Status)
            .all(|m| !m
                .unsupported
                .iter()
                .any(|reason| reason.starts_with("zMove:")))
    );
}
