//! Helping Hand (priority +5, target `adjacentAlly`): the ally's move this turn gets ×1.5 in
//! the BasePower chain (Showdown `helpinghand` condition, `onBasePowerPriority` 10); a second
//! Helping Hand on the same ally multiplies again. Fails when the ally has already moved this
//! turn and did not just switch in (Showdown `onTryHit`: `newlySwitched || willMove`).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(ally)) = targets.first() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    ctx.set_helping_hand(*ally)
}
