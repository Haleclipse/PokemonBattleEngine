//! Red Card: when a damaging move hits the holder, the attacker is dragged out for a random
//! bench Pokémon and the card is used up (Showdown `onAfterMoveSecondary`). Nothing happens
//! when the attacker is down, off the field, has no bench, or is already being dragged.

use crate::engine::{AfterHit, BattleContext, EffectSource};
use crate::ids::ItemId;

pub fn after_hit(ctx: &mut BattleContext<'_>, hit: AfterHit) {
    let Some(attacker_side) = ctx.side_of(hit.attacker) else {
        return;
    };
    if hit.attacker == hit.holder
        || ctx.is_fainted(hit.attacker)
        || ctx.is_fainted(hit.holder)
        || !ctx.is_active(hit.attacker)
        || !ctx.can_switch(attacker_side)
        || ctx.is_drag_pending(hit.attacker)
        || ctx.is_drag_pending(hit.holder)
    {
        return;
    }
    if ctx.consume_item(hit.holder).is_some() {
        ctx.item_activated(hit.holder, ItemId::RED_CARD);
        ctx.request_drag_out(
            hit.attacker,
            EffectSource::Item {
                pokemon: hit.holder,
                item: ItemId::RED_CARD,
            },
        );
    }
}
