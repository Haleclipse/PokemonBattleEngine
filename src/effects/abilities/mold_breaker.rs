//! Mold Breaker, Teravolt, Turboblaze (DESIGN.md §41.2): `onStart` announces the ability
//! (`-ability|holder|Mold Breaker`); `onModifyMove` makes every move its holder uses ignore
//! the others' breakable abilities (`MoveEdit::ignores_abilities`).

use crate::engine::BattleContext;
use crate::ids::PokemonId;

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if let Some(ability) = ctx.ability_of(holder) {
        ctx.ability_activated(holder, ability);
    }
}
