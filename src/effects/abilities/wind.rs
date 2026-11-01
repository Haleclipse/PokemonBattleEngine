//! Wind Power and Wind Rider (DESIGN.md §20.2, R167): wind moves (Showdown's `wind` flag) and
//! a Tailwind starting on the holder's side (`onSideConditionStart`).

use crate::engine::{BattleContext, DamagingHit, HitBlockReason, TryHitContext, TryHitResult};
use crate::ids::{AbilityId, PokemonId};
use crate::state::{SideConditionKind, Stat};

use super::hit_boosts::boost;

/// Wind Power (`onDamagingHit`, order 1): a wind move's hit charges the holder.
pub fn wind_power(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if hit.wind {
        charge(ctx, hit.holder);
    }
}

/// Charge's `-start|p|Charge|move|[from] ability: Wind Power` names it (R-M10-46).
fn charge(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.is_fainted(holder) {
        return;
    }
    ctx.ability_activated(holder, AbilityId::WIND_POWER);
    ctx.charge(holder);
}

/// Wind Power: a Tailwind starting on its side charges the holder.
pub fn wind_power_tailwind(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    kind: SideConditionKind,
) {
    if kind == SideConditionKind::Tailwind {
        charge(ctx, holder);
    }
}

/// Wind Rider (`onStart`): entering with a Tailwind up on its side, +1 Attack.
pub fn wind_rider_entry(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let tailwind = ctx
        .side_of(holder)
        .is_some_and(|side| ctx.side_has_condition(side, SideConditionKind::Tailwind));
    if tailwind {
        boost(ctx, holder, &[(Stat::Attack, 1)]);
    }
}

/// Wind Rider (`onTryHit`): a wind move from someone else is refused.
pub fn wind_rider(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    if ctx.user() != ctx.target() && ctx.move_is_wind() {
        TryHitResult::Blocked(HitBlockReason::Ability(AbilityId::WIND_RIDER))
    } else {
        TryHitResult::Allow
    }
}

/// Wind Rider's refusal: +1 Attack (`-immune` when it cannot rise).
pub fn wind_rider_absorb(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    boost(ctx, holder, &[(Stat::Attack, 1)]);
}

/// Wind Rider: a Tailwind starting on its side, +1 Attack.
pub fn wind_rider_tailwind(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    kind: SideConditionKind,
) {
    if kind == SideConditionKind::Tailwind {
        boost(ctx, holder, &[(Stat::Attack, 1)]);
    }
}
