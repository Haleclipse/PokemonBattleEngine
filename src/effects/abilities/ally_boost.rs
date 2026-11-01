//! Power Spot / Battery (Showdown `onAllyBasePower`, priority 22): a partner's move gets
//! [5325, 4096] (≈ ×1.3) in its BasePower chain — any move for Power Spot, special moves for
//! Battery. The holder's own moves are not boosted (the pipeline only asks the user's allies).

use crate::dex::MoveCategory;
use crate::effects::moves::BasePower;
use crate::engine::BasePowerContext;

pub fn power_spot(_ctx: &BasePowerContext<'_>, _power: BasePower, _chain: u32) -> Option<u32> {
    Some(5325)
}

pub fn battery(ctx: &BasePowerContext<'_>, _power: BasePower, _chain: u32) -> Option<u32> {
    (ctx.move_category() == MoveCategory::Special).then_some(5325)
}
