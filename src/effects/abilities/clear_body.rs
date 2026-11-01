//! Clear Body: refuses stat drops caused by someone else (Intimidate, Growl, a foe's Crunch);
//! the holder's own drops (Close Combat) go through. Showdown `onTryBoost`.

use crate::engine::{StatChangeBlockReason, StatChangeContext, TryStatChangeResult};
use crate::ids::AbilityId;

pub fn try_stat_change(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    if ctx.stages() < 0 && !ctx.is_self_inflicted() {
        TryStatChangeResult::Blocked {
            reason: StatChangeBlockReason::Ability(AbilityId::CLEAR_BODY),
            shown: super::drop_guard::announced_unless_octolock(ctx),
        }
    } else {
        TryStatChangeResult::Allow
    }
}
