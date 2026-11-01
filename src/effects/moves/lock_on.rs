//! Lock-On and Mind Reader (DESIGN.md §15.14): the user takes aim at the target — Gen 3+ a lock
//! on the user (`LockOnRules::model`), Gen 2 a flag on the target. The hit gate, Protect and a
//! Substitute are the shared Move Shell's.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = targets
        .iter()
        .find_map(|target| match target {
            ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
            _ => None,
        })
        .ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.lock_on(user, target)
}
