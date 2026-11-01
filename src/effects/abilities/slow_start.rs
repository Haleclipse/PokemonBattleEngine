//! Slow Start (Showdown `onStart`, `onResidual` 28.2): coming in, the holder starts a countdown
//! of 5 (`AbilityState::slow_start`) that each turn end it was on the field for ticks down; while
//! it runs, its physical moves' Attack and its Speed are halved (`offense_boost`, `speed_boost`;
//! Gen 4-7 by the move's data category, `DamageRules::slow_start_reads_data_category`). Its
//! `onEnd` only announces the end.

use crate::engine::BattleContext;
use crate::ids::{AbilityId, PokemonId};

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.start_slow_start(holder);
    ctx.ability_activated(holder, AbilityId::SLOW_START);
}

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.active_turns(holder) == 0 {
        return;
    }
    ctx.tick_slow_start(holder);
}
