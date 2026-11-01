//! Mean Look, Block, Spider Web, Octolock (DESIGN.md §30.8): the target cannot switch while
//! the user stays on the field. (Thousand Waves, Jaw Lock, Anchor Shot and Spirit Shackle trap through
//! data: `MoveBehavior::hit_traps`, a `secondary_on_hit`.)

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

/// Mean Look / Block / Spider Web (`onHit`: `return target.addVolatile('trapped', source, move,
/// 'trapper')`): fails on a target already trapped or, from Gen 6, a Ghost type.
pub fn mean_look(
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
    ctx.trap(target, user)
}

/// Octolock (`onTryImmunity`, `volatileStatus: 'octolock'`): see `BattleContext::octolock`.
pub fn octolock(
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
    ctx.octolock(target, user)
}
