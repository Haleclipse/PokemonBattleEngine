//! Magnet Rise and Telekinesis (DESIGN.md §41.5, K.7b): a volatile that keeps its holder off the
//! ground (`engine::environment::grounded`). Smack Down and Thousand Arrows ground their target
//! through their `volatileStatus` (`BattleContext::smack_down`). The hit gate, Protect and Magic
//! Coat are the shared Move Shell's.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

/// Magnet Rise's `onTry` (`BattleContext::magnet_rise_try`).
pub fn magnet_rise_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    ctx.magnet_rise_try(user)
}

/// Magnet Rise: the user off the ground for five turns.
pub fn magnet_rise(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_magnet_rise(user)
}

/// Telekinesis: the target off the ground for three turns, sure to be hit.
pub fn telekinesis(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = targets
        .iter()
        .find_map(|target| match target {
            ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
            _ => None,
        })
        .ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.set_telekinesis(target)
}
