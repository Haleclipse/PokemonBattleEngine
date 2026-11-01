//! Unnerve and As One (Showdown `onStart`, `onFoeTryEatItem`, `onEnd`): the first start since the
//! holder came in announces it and sets `AbilityState::unnerved`; while that holds and the
//! holder is on the field, no foe eats a berry (`BattleContext::eat_item`). The state goes with
//! the holder leaving the field (`onEnd`). As One announces itself, then Unnerve.

use crate::engine::BattleContext;
use crate::ids::{AbilityId, PokemonId};

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.start_unnerve(holder) {
        ctx.ability_activated(holder, AbilityId::UNNERVE);
    }
}

pub fn as_one(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if let Some(ability) = ctx.ability_of(holder)
        && ctx.start_unnerve(holder)
    {
        ctx.ability_activated(holder, ability);
        // The line names the effect, not the ability.
        ctx.ability_at_work(holder, AbilityId::UNNERVE);
    }
}
