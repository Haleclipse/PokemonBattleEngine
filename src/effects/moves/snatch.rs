//! Snatch (DESIGN.md §42.8): the user `-singleturn`s a steal that takes over a move carrying the
//! `snatch` flag this turn (the PrepareHit step reads the volatile) and uses it itself.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    if ctx.set_snatch(user) {
        ctx.singleturn(user, crate::state::VolatileConditionKind::Snatch);
    }
    Ok(())
}
