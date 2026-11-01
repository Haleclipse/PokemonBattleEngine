//! Eject Button: when a damaging move hits the holder, the holder is ordered out for a
//! replacement its trainer chooses (the same mid-turn request as U-turn) and the button is
//! used up. It pre-empts the attacker's own self-switch (a U-turn into an Eject Button does
//! not switch the U-turn user). Showdown `onAfterMoveSecondary`, priority 2 (before Red Card).

use crate::engine::{AfterHit, BattleContext};
use crate::ids::{ItemId, MoveId};

pub fn after_hit(ctx: &mut BattleContext<'_>, hit: AfterHit) {
    let Some(side) = ctx.side_of(hit.holder) else {
        return;
    };
    // Not for a foreseen strike (Showdown `!move.flags['futuremove']`, R-M8-128).
    if hit.attacker == hit.holder
        || matches!(hit.move_id, MoveId::FUTURE_SIGHT | MoveId::DOOM_DESIRE)
        || ctx.is_fainted(hit.holder)
        || !ctx.can_switch(side)
        || ctx.is_drag_pending(hit.holder)
        // Leaving already, Pursuit striking it as it goes (`beingCalledBack`).
        || ctx.is_called_back(hit.holder)
        // Sky Drop holds the holder in the air (`isSkyDropped`).
        || ctx.is_sky_dropped(hit.holder)
    {
        return;
    }
    // Showdown: refused while any active Pokémon already carries a chosen-switch flag other
    // than the attacker's own (which the button then cancels).
    ctx.cancel_switch_out(hit.attacker);
    if ctx.any_switch_out_pending() {
        return;
    }
    // The order goes out before the button is used (Showdown sets `switchFlag` first): a
    // partner's Symbiosis then sees the holder leaving.
    // Both always succeed here: the guards above cover `request_switch_out`'s refusals, and
    // the holder holds the button.
    ctx.request_switch_out(hit.holder);
    ctx.consume_item(hit.holder);
    ctx.item_activated(hit.holder, ItemId::EJECT_BUTTON);
}
