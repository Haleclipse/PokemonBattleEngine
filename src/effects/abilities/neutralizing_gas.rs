//! Neutralizing Gas (Gen 8+, DESIGN.md §41.2): `onSwitchIn` (priority 2, before every other
//! switch-in handler) announces it and clears the field of what it suppresses
//! (`BattleContext::neutralizing_gas_started`); from then on `engine::ignoring_ability` sets every
//! other ability aside. Its `onEnd` is `BattleContext::end_ability`'s.

use crate::engine::BattleContext;
use crate::ids::{AbilityId, PokemonId};

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.ability_activated(holder, AbilityId::NEUTRALIZING_GAS);
    ctx.neutralizing_gas_started(holder);
}
