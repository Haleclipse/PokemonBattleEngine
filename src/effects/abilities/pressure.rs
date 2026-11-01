//! Pressure (DESIGN.md §15.15): `onStart` announces it (Gen 3: only on the holder's side's
//! screen, `[silent]` to the other); `onDeductPP` makes a move that meets the holder cost one
//! more PP (`AbilityBehavior::pressure`, `engine::pressure_holders`).

use crate::engine::BattleContext;
use crate::ids::PokemonId;

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if let Some(ability) = ctx.ability_of(holder) {
        if ctx.rules().conditions().pressure_shown_to_foes {
            ctx.ability_activated(holder, ability);
        } else {
            ctx.ability_at_work(holder, ability);
        }
    }
}
