//! Abilities: behavior coverage and the abilities Showdown hard-codes elsewhere.

use crate::data::{self, DexSnapshot};
use crate::effects;
use crate::ids::AbilityId;

use super::support::missing_callbacks;
use crate::dex::ResolvedAbility;

/// Every ability of the generation, classified against its behavior components (DESIGN.md
/// §27.2).
pub(super) fn resolve_abilities(snapshot: &DexSnapshot) -> Vec<ResolvedAbility> {
    snapshot
        .abilities
        .iter()
        .map(|ability| {
            let id = AbilityId(ability.eid);
            let mut missing_callbacks =
                missing_callbacks(&ability.callbacks, &ability_covered_callbacks(id));
            // An ability whose effect Showdown hard-codes elsewhere (`hasAbility`) is only
            // as implemented as that code.
            if let Some((_, reference, _)) =
                UNMODELLED_ABILITY_REFERENCES
                    .iter()
                    .find(|(name, _, generations)| {
                        *name == data::to_id(&ability.name)
                            && generations.contains(&snapshot.generation)
                    })
            {
                missing_callbacks.push(format!("referenced-by:{reference}"));
            }
            // An ability with no callbacks has nothing to compare: it passes only once reviewed
            // (`CALLBACKLESS_ABILITIES`), so one whose effect Showdown hard-codes elsewhere
            // cannot pass as implemented unmodelled (Tera Shell, R-M9-45).
            if ability.callbacks.is_empty()
                && !CALLBACKLESS_ABILITIES
                    .iter()
                    .any(|(name, _)| *name == data::to_id(&ability.name))
            {
                missing_callbacks.push("callbackless:unreviewed".to_string());
            }
            ResolvedAbility {
                id,
                name: ability.name.clone(),
                implemented: missing_callbacks.is_empty(),
                missing_callbacks,
            }
        })
        .collect()
}

/// Abilities with no callbacks of their own whose effect Showdown hard-codes in the simulator or
/// in a condition (`hasAbility('…')`, found by grepping `@pkmn/sim`). Levitate is modelled where
/// Showdown has it (`isGrounded`, the hit gate's type-immunity step) and Multitype has no
/// battle effect the engine models, so neither is listed.
const UNMODELLED_ABILITY_REFERENCES: &[(&str, &str, std::ops::RangeInclusive<u8>)] = &[];

/// The abilities with no callbacks in some generation's data, each with where the engine models
/// what Showdown hard-codes for it (`hasAbility`, the event order, a non-function flag) or why it
/// has nothing to model (R-M9-45).
const CALLBACKLESS_ABILITIES: &[(&str, &str)] = &[
    ("noability", "no ability"),
    ("ballfetch", "outside battle"),
    (
        "battlearmor",
        "AbilityBehavior::blocks_critical_hit (`onCriticalHit: false`)",
    ),
    (
        "shellarmor",
        "AbilityBehavior::blocks_critical_hit (`onCriticalHit: false`)",
    ),
    (
        "corrosion",
        "engine::status (`setStatus`'s poison immunity)",
    ),
    ("dancer", "the Shell's dance (`runMove` after a dance move)"),
    ("earlybird", "engine::before_move (the sleep counter)"),
    ("honeygather", "outside battle"),
    ("illuminate", "outside battle (before Gen 9)"),
    (
        "levitate",
        "engine::environment::grounded, the hit gate's Ground immunity",
    ),
    (
        "multitype",
        "no battle effect modelled (Arceus's forme is its Plate's)",
    ),
    ("pickup", "outside battle (Gen 3-4)"),
    ("stench", "outside battle (Gen 3-4)"),
    (
        "rkssystem",
        "engine::typing (Silvally's type is its Memory's)",
    ),
    ("runaway", "wild battles only"),
    ("stall", "the action order (`subOrder` 9)"),
    (
        "terashell",
        "move_pipeline::hit_gate::tera_shell_holds (`runEffectiveness`)",
    ),
];

/// The Showdown callbacks each behavior component stands for (the component tables, DESIGN.md
/// §25.1). `implemented` means every callback the snapshot declares is covered, so a behavior that
/// covers one of an ability's two callbacks (Sturdy's `onDamage` without `onTryHit`) leaves it
/// unsupported instead of a false positive.
fn ability_covered_callbacks(id: AbilityId) -> Vec<&'static str> {
    effects::abilities::definition(id).covered_callbacks()
}
