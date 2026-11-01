//! Ally Switch (DESIGN.md §42.4): the user trades places with its partner. Gen 9's
//! `onPrepareHit` rolls the consecutive-use chance first (`allyswitch`'s counter 3 / 9 … 729,
//! like Protect's stall); a failed roll ends the move.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

/// Gen 9's `onPrepareHit`: `randomChance(1, counter)` while a counter stands; the counter
/// advances (×3) as the move goes ahead.
pub fn try_switch(ctx: &mut BattleContext<'_>, user: PokemonId) -> Result<(), ConditionApplyError> {
    if !ctx
        .battle_rules()
        .conditions
        .calls
        .ally_switch_consecutive_chance
    {
        return Ok(());
    }
    let Some(counter) = ctx
        .battle_state()
        .pokemon(user)
        .and_then(|pokemon| pokemon.volatiles.ally_switch_counter())
    else {
        ctx.bump_ally_switch(user);
        return Ok(());
    };
    let success = ctx.chance_for(crate::state::RandomRequestKind::ProtectSuccess, 1, counter);
    if !success {
        ctx.drop_ally_switch(user);
        return Err(ConditionApplyError::NothingToDo);
    }
    ctx.bump_ally_switch(user);
    Ok(())
}

/// The swap itself (`onHit`): in singles (and a triples middle position), or with the partner
/// gone, nothing moves, yet the move has not failed (`-fail`, then `NOT_FAIL`): a Temper Flare
/// or Stomping Tantrum next turn is not doubled (R-M8-130).
pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    if ctx.battle_rules().topology.active_slots_per_side() == 1 {
        return Ok(());
    }
    match ctx.swap_with_partner(user) {
        Err(ConditionApplyError::NothingToDo) => Ok(()),
        result => result,
    }
}
