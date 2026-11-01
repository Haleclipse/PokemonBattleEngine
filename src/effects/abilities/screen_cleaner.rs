//! Screen Cleaner (Showdown `onStart`): Reflect, Light Screen and Aurora Veil end on both sides,
//! the holder's first; the ability is announced once, only when there was a screen to clean.

use crate::engine::BattleContext;
use crate::ids::{AbilityId, PokemonId};

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(side) = ctx.side_of(holder) else {
        return;
    };
    if !ctx.any_screen() {
        return;
    }
    ctx.ability_activated(holder, AbilityId::SCREEN_CLEANER);
    ctx.clear_screens(side);
}
