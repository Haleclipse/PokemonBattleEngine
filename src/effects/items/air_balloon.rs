//! Air Balloon (Gen 5+, DESIGN.md §41.5): its holder is off the ground
//! (`engine::environment::grounded`) until a move's hit pops it — on the holder (`onDamagingHit`)
//! or on its Substitute (`onAfterSubDamage`). Its `onStart` announces it, for all to see.

use crate::engine::{BattleContext, DamagingHit};
use crate::ids::{ItemId, PokemonId};
use crate::state::FieldConditionKind;

/// `onStart` (its holder comes in, or receives it): `-item|holder|Air Balloon`, but under
/// Gravity, which keeps it on the ground. A holder that ignores its item runs no `onStart`
/// (`singleEvent` asks `ignoringItem()`), as the engine runs no item handler of it.
pub fn start(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if !ctx.field_has(FieldConditionKind::Gravity) {
        ctx.announce_item(holder, ItemId::AIR_BALLOON);
    }
}

/// `onDamagingHit` (no order: after the ordered reactions).
pub fn damaging_hit_reaction(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    ctx.pop_air_balloon(hit.holder);
}
