//! Follow Me: this turn, foes' single-target moves are redirected to the user (Showdown
//! `onFoeRedirectTarget`). Fails in singles (`onTry`: more than one active per side). Spotlight
//! does the same for its target.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::VolatileConditionKind;

/// Follow Me / Rage Powder's `onTry`: more than one active Pokémon per side.
pub fn tries(ctx: &mut BattleContext<'_>, _user: PokemonId) -> Result<(), ConditionApplyError> {
    if ctx.rules().topology().active_slots_per_side() < 2 {
        return Err(ConditionApplyError::InvalidTarget);
    }
    Ok(())
}

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_follow_me(user)
}

/// Rage Powder: Follow Me that a powder-immune attacker ignores (Showdown `ragepowder`,
/// priority +2 from Gen 6). Fails in singles.
pub fn rage_powder(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_rage_powder(user)
}

/// Spotlight (Gen 7, priority +3): the target draws its foes' single-target moves this turn
/// (`onFoeRedirectTarget`, priority 2: ahead of Follow Me). Its `onTryHit` fails it in singles.
pub fn spotlight(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    if ctx.rules().topology().active_slots_per_side() < 2 {
        return Err(ConditionApplyError::InvalidTarget);
    }
    let Some(ResolvedTarget::Pokemon(target)) = targets.first() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    ctx.set_marker_volatile(*target, VolatileConditionKind::Spotlight)
}
