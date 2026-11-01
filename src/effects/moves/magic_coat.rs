//! Magic Coat (DESIGN.md §42.8): the user `-singleturn`s a coat that reflects a reflectable move
//! aimed at it back at its user, for the turn. The reflection itself lives in the hit gate
//! (`MovePipeline::reflect_move`), which reads the volatile this sets.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    if ctx.set_magic_coat(user) {
        ctx.singleturn(user, crate::state::VolatileConditionKind::MagicCoat);
    }
    Ok(())
}
