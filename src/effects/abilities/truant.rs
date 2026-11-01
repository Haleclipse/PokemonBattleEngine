//! Truant (DESIGN.md §15.21): every other turn the holder loafs instead of moving (BeforeMove,
//! `BattleContext::truant_loafs`). Gen 5+ `onStart` clears the `truant` volatile and may set it
//! again; Gen 3's `onSwitchIn` is the entry's flag (`BattleContext::enter_truant`, which every
//! Pokémon coming in goes through) and its `onResidual` flips that flag at every turn end.

use crate::engine::BattleContext;
use crate::ids::PokemonId;

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.start_truant(holder);
}

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.flip_truant_turn(holder);
}
