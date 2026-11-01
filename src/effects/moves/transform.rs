//! Transform (DESIGN.md §41.7): `onHit` is `pokemon.transformInto(target)`.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

/// Transform: the user battles as a copy of its target (`BattleContext::transform_into`); the
/// move fails when it cannot.
pub fn transform(
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
    if ctx.transform_into(user, target, None) {
        Ok(())
    } else {
        Err(ConditionApplyError::Immune)
    }
}
