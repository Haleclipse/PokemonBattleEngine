//! Speed Boost (Showdown residual order 28.2, Gen 4: 10.3): +1 Speed at the end of every
//! turn the holder was on the field when the turn started (`active_turns > 0`: not the turn
//! it switched in).

use crate::engine::{BattleContext, EffectSource, Stat, StatChangeCause, StatChangeRequest};
use crate::ids::{AbilityId, PokemonId};

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.active_turns(holder) == 0 {
        return;
    }
    ctx.try_change_stat_stage(StatChangeRequest {
        target: holder,
        stat: Stat::Speed,
        stages: 1,
        source: EffectSource::Ability {
            pokemon: holder,
            ability: AbilityId::SPEED_BOOST,
        },
        cause: StatChangeCause::Ability,
    });
}
