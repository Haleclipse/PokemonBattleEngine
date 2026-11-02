//! The failure bundle a diverging replay leaves for a reproduction.

use std::collections::BTreeMap;
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, FollowUp, MoveCategory, MultiHit, Nature, PokemonSet, PrimaryEffect, ResolvedDex,
    ResolvedMove, SecondaryEffect, SecondaryTarget, SecondaryTrigger, StatSpread, StatusMoveEffect,
    build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, BattleSnapshot, EngineStep, ForcedReplacement, ForcedSwitchChoice,
    MechanicChoice, PriorityChance, ReplacementRequirement, ReviveChoice, SideChoice, SlotChoice,
    TraceEvent, two_turn,
};
use pokemon_battle_engine::format::{FormatLibrary, Stats, TeamSet};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, HitStepOrder, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::session::{OpenTeamSheets, SessionFormat, TeamChoice, TeamSlot};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    ActiveMove, BattleRng, BattleState, Gender, NonVolatileStatus, RandomOverride,
    RandomRequestKind, SideState, VolatileCondition, VolatileConditionKind,
};
use serde::Deserialize;
use serde_json::Value;

use crate::support::{capture, compare_json};

use super::*;

// ---------------------------------------------------------------------------------------------
// Failure bundle (TESTING.md §9)
// ---------------------------------------------------------------------------------------------

pub const FAILURE_BUNDLE_FILES: [&str; 8] = [
    "scenario.json",
    "rust.frames.json",
    "oracle.frames.json",
    "diff.json",
    "rust.trace.txt",
    "showdown.log.txt",
    "provenance.json",
    "reproduce.txt",
];

/// Writes the reproducible artefact for a diverging scenario. A failure must be inspectable
/// and re-runnable without the original test run.
pub fn write_failure_bundle(
    dir: &std::path::Path,
    scenario_path: &std::path::Path,
    oracle: &Value,
    report: &ReplayReport,
    reproduce: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let pretty = |value: &Value| serde_json::to_string_pretty(value).expect("json") + "\n";
    std::fs::copy(scenario_path, dir.join("scenario.json"))?;
    std::fs::write(
        dir.join("rust.frames.json"),
        pretty(&Value::Array(report.rust_frames.clone())),
    )?;
    std::fs::write(dir.join("oracle.frames.json"), pretty(&oracle["frames"]))?;
    std::fs::write(
        dir.join("diff.json"),
        pretty(&serde_json::json!({
            "first_divergence": report.divergences.first(),
            "divergences": report.divergences,
            "frames_compared": report.frames_compared,
            "actions_replayed": report.actions_replayed,
        })),
    )?;
    std::fs::write(dir.join("rust.trace.txt"), report.trace.join("\n") + "\n")?;
    std::fs::write(
        dir.join("showdown.log.txt"),
        oracle["debug_log"].as_str().unwrap_or(""),
    )?;
    std::fs::write(
        dir.join("provenance.json"),
        pretty(&serde_json::json!({"oracle": oracle["provenance"], "rust": report.provenance})),
    )?;
    std::fs::write(dir.join("reproduce.txt"), format!("{reproduce}\n"))?;
    Ok(())
}
