//! Flash Fire (Showdown `onTryHit` and its `flashfire` condition; DESIGN.md §20.2, R172): a
//! Fire move from someone else is taken — the holder's Fire moves then stronger
//! (`VolatileConditionKind::FlashFire`, where `ConditionRules::flash_fire` puts it), `-immune`
//! when it already is — except, in Gen 3-4, by a frozen holder, and in Gen 3, a Will-O-Wisp
//! that would fail against it anyway.

use crate::dex::PokemonType;
use crate::engine::{BattleContext, HitBlockReason, TryHitContext, TryHitResult};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::state::{NonVolatileStatus, VolatileConditionKind};

pub fn flash_fire(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    let rules = ctx.rules().conditions().flash_fire;
    let passes = (rules.frozen_holder_passes
        && ctx.target_status() == Some(NonVolatileStatus::Freeze))
        || (rules.passes_failing_will_o_wisp
            && ctx.move_id() == MoveId::WILL_O_WISP
            && (ctx.target_has_type(PokemonType::Fire)
                || ctx.target_status().is_some()
                || ctx.target_behind_substitute()));
    if ctx.user() != ctx.target() && ctx.move_type() == PokemonType::Fire && !passes {
        TryHitResult::Blocked(HitBlockReason::Ability(AbilityId::FLASH_FIRE))
    } else {
        TryHitResult::Allow
    }
}

/// Flash Fire's take: its boost starts (nothing more when it is already on).
pub fn flash_fire_absorb(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let _ = ctx.set_marker_volatile(holder, VolatileConditionKind::FlashFire);
}
