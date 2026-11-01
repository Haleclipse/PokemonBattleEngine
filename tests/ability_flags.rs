//! The engine's ability flags against Showdown's, generation by generation (DESIGN.md §41.4,
//! F8): the rule values (`ConditionRules::cant_suppress`, …) against
//! `tests/fixtures/oracle/ability-flags.json`, exported by
//! `tools/oracle-node/src/ability-flags.mjs`. A flag is checked once the engine reads it.

use std::path::PathBuf;

use pokemon_battle_engine::dex::DexResolver;
use pokemon_battle_engine::ids::AbilityId;
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use serde_json::Value;

fn showdown_id(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The ids of the engine's list against the fixture's, for every generation from Gen 3.
fn check(
    flag: &str,
    list: impl Fn(&pokemon_battle_engine::rules::ConditionRules) -> Vec<AbilityId>,
) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("tests/fixtures/oracle/ability-flags.json"))
            .expect("fixture"),
    )
    .expect("json");
    for generation in [
        Generation::Gen3,
        Generation::Gen4,
        Generation::Gen5,
        Generation::Gen6,
        Generation::Gen7,
        Generation::Gen8,
        Generation::Gen9,
    ] {
        let expected: Vec<String> = serde_json::from_value(
            fixture["flags"][flag][format!("gen{}", generation.number())].clone(),
        )
        .expect("ids");
        let dex = DexResolver::resolve(generation);
        let rules =
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .expect("rules");
        let mut engine: Vec<String> = list(&rules.conditions())
            .iter()
            .map(|id| showdown_id(&dex.ability_by_id(*id).expect("in the dex").name))
            .collect();
        engine.sort();
        assert_eq!(engine, expected, "{generation:?} {flag}");
    }
}

#[test]
fn cant_suppress_matches_showdown() {
    check("cantsuppress", |rules| rules.cant_suppress.to_vec());
}

#[test]
fn breakable_matches_showdown() {
    check("breakable", |rules| rules.breakable.to_vec());
}

#[test]
fn fail_role_play_matches_showdown() {
    check("failroleplay", |rules| rules.fail_role_play.to_vec());
}

#[test]
fn fail_skill_swap_matches_showdown() {
    check("failskillswap", |rules| rules.fail_skill_swap.to_vec());
}

#[test]
fn no_entrain_matches_showdown() {
    check("noentrain", |rules| rules.no_entrain.to_vec());
}

#[test]
fn no_receiver_matches_showdown() {
    check("noreceiver", |rules| rules.no_receiver.to_vec());
}

#[test]
fn no_trace_matches_showdown() {
    check("notrace", |rules| rules.no_trace.to_vec());
}

#[test]
fn no_transform_matches_showdown() {
    check("notransform", |rules| rules.no_transform.to_vec());
}
