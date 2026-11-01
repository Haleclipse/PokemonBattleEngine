//! Trace (DESIGN.md §41.3, K.4b): it copies a foe's ability — at once as its holder comes in,
//! and again at every `Update` while it found nothing to copy (`BattleContext::start_trace`).

use crate::engine::BattleContext;
use crate::ids::PokemonId;

/// Trace's `onStart`.
pub fn start(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.start_trace(holder);
}
