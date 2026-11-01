//! Focus Sash (Gen 5+): a move's damage that would knock the holder out from full HP leaves it at
//! 1 and the sash is consumed (Showdown `onDamage`, priority -40 → after Sturdy, so Sturdy's 1 HP
//! keeps the sash) — the confusion self-hit's pseudo-move included. Gen 4's lasts the whole move
//! (`ItemBehavior::holds_for_the_move`, `MovePipeline`), and its confusion self-hit checks the sash
//! as well (`confusion_self_hit`, issues.md I39).

use crate::engine::{BattleContext, DamageAmount, DamageDecision, DamageRequest};

pub fn damage_resolution(ctx: &mut BattleContext<'_>, request: &DamageRequest) -> DamageDecision {
    let (Some(hp), Some(max_hp)) = (ctx.hp(request.target), ctx.max_hp(request.target)) else {
        return DamageDecision::Allow;
    };
    if request.from_a_move(&ctx.rules())
        && hp == max_hp
        && request.amount.0 >= hp
        && ctx.consume_item(request.target).is_some()
    {
        DamageDecision::Reduce(DamageAmount(hp - 1))
    } else {
        DamageDecision::Allow
    }
}
