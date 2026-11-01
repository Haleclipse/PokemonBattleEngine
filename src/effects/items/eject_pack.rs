//! Eject Pack (DESIGN.md R247): a stage of the holder's that goes down (Showdown
//! `onAfterBoost`) marks it, and at the next `onAnySwitchIn` (−4), `onAnyAfterMove` or
//! `onResidual` (29) the holder uses the pack up and is ordered out for a replacement its trainer
//! chooses (`onUseItem`, `onUse`). Its `onAnyAfterMega` waits for Mega Evolution (family O), its
//! `commanding` / `commanded` refusal for Commander.

use crate::effects::{ResidualHandler, ResidualParticipant};
use crate::engine::{BattleContext, StatChangeRequest, StatChangeResult};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResidualKind;

use super::{ItemAfterBoostHandler, ItemBehavior};

pub(super) fn eject_pack() -> ItemBehavior {
    ItemBehavior {
        after_boost: Some(mark_a_drop as ItemAfterBoostHandler),
        any_switch_in: Some((-4, eject)),
        any_after_move: Some(eject),
        // Showdown `onAnyAfterMega` (DESIGN.md §44.3): the same check.
        any_after_mega: Some(eject),
        residual: Some(ResidualParticipant {
            kind: ResidualKind::EjectPack,
            handler: eject as ResidualHandler,
        }),
        ejects_after_a_drop: true,
        ..ItemBehavior::default()
    }
}

/// `onAfterBoost`: a stage the change took down (Showdown's capped `boost[i] < 0`) marks the
/// holder — not while Parting Shot is the move under way (`activeMove`), the drops Mirror Armor
/// sends back to its user included.
fn mark_a_drop(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    _request: StatChangeRequest,
    result: StatChangeResult,
) {
    let StatChangeResult::Applied { from, to } = result else {
        return;
    };
    if to >= from || ctx.active_move() == Some(MoveId::PARTING_SHOT) {
        return;
    }
    if let Some(state) = ctx.item_state_mut(holder) {
        state.eject = true;
    }
}

/// The pack's use: a marked holder up and on the field uses it up and is ordered out — unless
/// nobody can come in, or a Pokémon on the field is already ordered out by an item or an ability
/// (Showdown `switchFlag === true`; a U-turn's order, which holds the move's id, does not stop
/// it, and the pack's order takes its place). Refused, the mark stays for the next chance.
pub fn eject(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some((_, behavior)) = ctx.held_item(holder) else {
        return;
    };
    let Some(side) = ctx.side_of(holder) else {
        return;
    };
    if !behavior.ejects_after_a_drop
        || !ctx.item_state(holder).is_some_and(|state| state.eject)
        || ctx.is_fainted(holder)
        || !ctx.is_active(holder)
        || !ctx.can_switch(side)
        || ctx.chosen_switch_pending()
    {
        return;
    }
    // The order goes out before the pack is used up (Showdown's `useItem`: the item's `onUse`
    // sets `switchFlag`, then `AfterUseItem`): a partner's Symbiosis sees the holder leaving
    // (R-M8-135). Both succeed: the guards above cover the order's refusals, and the holder holds
    // the pack.
    ctx.request_switch_out(holder);
    ctx.consume_item(holder);
}
