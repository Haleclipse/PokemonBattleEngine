//! Defog and Court Change (DESIGN.md §14.2): entry hazards, screens and the like cleared or
//! swapped; the work is `BattleContext::defog` / `swap_side_conditions`.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn defog(
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
    ctx.defog(user, target)
}

pub fn court_change(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    if ctx.swap_side_conditions() {
        Ok(())
    } else {
        Err(ConditionApplyError::NothingToDo)
    }
}
