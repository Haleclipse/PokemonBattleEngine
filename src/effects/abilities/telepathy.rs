//! Telepathy (Showdown `onTryHit`): a partner's damaging move does not touch the holder
//! (`-activate … ability: Telepathy`, no accuracy check, no damage).

use crate::dex::MoveCategory;
use crate::engine::{HitBlockReason, TryHitContext, TryHitResult};
use crate::ids::AbilityId;

pub fn try_hit(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    if ctx.user_is_ally() && ctx.move_category() != MoveCategory::Status {
        TryHitResult::Blocked(HitBlockReason::Ability(AbilityId::TELEPATHY))
    } else {
        TryHitResult::Allow
    }
}
