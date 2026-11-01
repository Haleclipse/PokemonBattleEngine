//! The abilities that change their holder's type (DESIGN.md §41.4, K.6b): Protean and Libero
//! before the holder's move hits, Color Change after a move hits the holder
//! (`BattleContext::set_type`).

use crate::dex::{MoveCategory, PokemonType};
use crate::engine::{BattleContext, DamagingHit, EffectSource};
use crate::ids::PokemonId;
use crate::state::PokemonTypes;

/// Color Change: the holder, still standing, takes a damaging move's type unless it is `???`
/// or one of the holder's already. Gen 5+ (`onAfterMoveSecondary`) also points a second-slot
/// holder's queued Curse at its ally (the "Curse glitch").
pub fn color_change(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    move_type: PokemonType,
    category: MoveCategory,
) {
    if retype(ctx, holder, move_type, category) {
        ctx.curse_glitch(holder);
    }
}

/// Gen 3-4 Color Change (`onDamagingHit`): on each hit that took HP from the holder.
pub fn color_change_on_hit(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if hit.damage > 0 {
        retype(ctx, hit.holder, hit.move_type, hit.category);
    }
}

fn retype(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    move_type: PokemonType,
    category: MoveCategory,
) -> bool {
    if ctx.is_fainted(holder)
        || category == MoveCategory::Status
        || move_type == PokemonType::Typeless
        || ctx.has_type(holder, move_type)
    {
        return false;
    }
    let ability = ctx
        .raw_ability_of(holder)
        .unwrap_or(crate::ids::AbilityId::COLOR_CHANGE);
    ctx.set_type(
        holder,
        PokemonTypes::mono(move_type),
        EffectSource::Ability {
            pokemon: holder,
            ability,
        },
    )
}
