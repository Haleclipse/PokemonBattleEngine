//! Clear Amulet (Showdown `onTryBoost`, `onTryBoostPriority` 1): refuses every stat drop
//! someone else causes, as Clear Body. Its priority puts it before the holder's ability at the
//! TryBoost stage: a Mirror Armor holder with it has nothing left to reflect.

use crate::engine::{StatChangeBlockReason, StatChangeContext, TryStatChangeResult};
use crate::ids::ItemId;

pub fn try_stat_change(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    if ctx.stages() < 0 && !ctx.is_self_inflicted() {
        // `-fail|holder|unboost|[from] item: Clear Amulet|[of] holder`, as Clear Body's.
        TryStatChangeResult::Blocked {
            reason: StatChangeBlockReason::Item(ItemId::CLEAR_AMULET),
            shown: !ctx.is_secondary() && !ctx.is_octolock(),
        }
    } else {
        TryStatChangeResult::Allow
    }
}
