//! Bide (DESIGN.md §15.10): the move puts the volatile on its user; the turns after are the
//! volatile's (`MovePipeline::bide_turn`: its `onBeforeMove` and the move's
//! `beforeMoveCallback`).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn start(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.start_bide(user)
}
