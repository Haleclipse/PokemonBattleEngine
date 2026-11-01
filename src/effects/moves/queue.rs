//! After You and Quash (DESIGN.md §42.4): the target's pending action goes first or last.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn after_you(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    ctx.after_you(*target)
}

pub fn quash(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    ctx.quash(*target)
}
