//! Supreme Overlord (Showdown `onStart`): coming in, the holder counts its side's fainted
//! Pokémon, at most 5 (`AbilityState::fallen`). Its `base_power_boost`
//! (`BoostModifier::FallenAllies`) reads that count, which later faints leave alone.

use crate::engine::BattleContext;
use crate::ids::{AbilityId, PokemonId};

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.count_fallen_allies(holder) > 0 {
        ctx.ability_activated(holder, AbilityId::SUPREME_OVERLORD);
    }
}
