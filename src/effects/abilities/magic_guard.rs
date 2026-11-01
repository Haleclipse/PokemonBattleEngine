//! Magic Guard: only move damage reaches the holder (Showdown `onDamage`: every effect that is
//! not a Move is refused — residual status, weather, hazards, recoil, Rough Skin / Rocky
//! Helmet). A confusion self-hit is a Move in Showdown (`{id: 'confused', effectType: 'Move'}`)
//! and goes through; HP paid as a cost (Substitute, Belly Drum) is direct damage and is not
//! refused either. Gen 4 also keeps Toxic Spikes' poison off (`onSetStatus`, R129) and full
//! paralysis away (`StatusLifecycleRules::magic_guard_skips_full_paralysis`).

use crate::engine::{
    BattleContext, DamageDecision, DamageKind, DamageRequest, EffectSource, StatusBlockReason,
    TrySetStatusContext, TrySetStatusResult,
};
use crate::ids::AbilityId;
use crate::state::SideConditionKind;

pub fn damage_resolution(ctx: &mut BattleContext<'_>, request: &DamageRequest) -> DamageDecision {
    if matches!(request.kind, DamageKind::Move | DamageKind::SelfHit) || request.kind.is_direct() {
        return DamageDecision::Allow;
    }
    // Showdown says nothing of the holder's; from Gen 7 an ability's damage it keeps off names
    // that ability on its source (`-activate|source|ability: Rough Skin`; the gen6 mod's says
    // nothing, R-M10-42).
    if let EffectSource::Ability { pokemon, ability } = request.source
        && ctx.rules().hits().magic_guard_names_ability
    {
        ctx.ability_activated(pokemon, ability);
    }
    ctx.ability_at_work(request.target, AbilityId::MAGIC_GUARD);
    DamageDecision::Prevent
}

/// Gen 4 Magic Guard (`onSetStatus`): Toxic Spikes poison nothing — the gen4 hazard skips a
/// Magic Guard holder by name too.
pub fn try_set_status(ctx: &TrySetStatusContext<'_>) -> TrySetStatusResult {
    if ctx.source() == EffectSource::SideCondition(SideConditionKind::ToxicSpikes) {
        TrySetStatusResult::Blocked {
            reason: StatusBlockReason::Ability(AbilityId::MAGIC_GUARD),
            shown: false,
        }
    } else {
        TrySetStatusResult::Allow
    }
}
