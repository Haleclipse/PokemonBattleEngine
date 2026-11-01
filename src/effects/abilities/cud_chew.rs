//! Cud Chew (Gen 9; Showdown `onEatItem`, `onResidual` 28.2): a berry the holder eats it eats
//! again at the second turn's end after — the first, when it was eaten with the turn's queue
//! empty (`BattleContext::remember_for_cud_chew`, `BattleContext::chew_the_cud`).

use crate::engine::BattleContext;
use crate::ids::PokemonId;

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.chew_the_cud(holder);
}
