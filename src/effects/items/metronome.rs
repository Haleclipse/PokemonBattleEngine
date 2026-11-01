//! Metronome (the item, DESIGN.md R126): its `onStart` gives the holder the `metronome`
//! condition, which counts the moves it uses in a row; the counting and the boost follow the
//! generation's `DamageRules::metronome` (the engine's move pipeline).

use crate::engine::BattleContext;
use crate::ids::PokemonId;

/// Showdown `onStart`: `addVolatile('metronome')` as the holder enters.
pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.start_metronome(holder);
}
