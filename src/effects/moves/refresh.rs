//! Refresh (Showdown `onHit`): cures the user's own burn, paralysis or poison; fails otherwise.

use crate::engine::{BattleContext, ConditionApplyError, CureReason};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::NonVolatileStatus;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    match ctx.status_of(user) {
        Some(
            NonVolatileStatus::Burn
            | NonVolatileStatus::Paralysis
            | NonVolatileStatus::Poison
            | NonVolatileStatus::Toxic,
        ) => {
            ctx.cure_status(user, CureReason::Move(MoveId::REFRESH));
            Ok(())
        }
        _ => Err(ConditionApplyError::AlreadyActive),
    }
}
