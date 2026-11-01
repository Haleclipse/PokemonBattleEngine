//! Ingrain: the user is rooted — healed 1/16 every turn (residual order 7, Gen 4: 10.1),
//! unable to switch, and immune to being dragged out (Showdown `onTrapPokemon` /
//! `onDragOut`). Fails when already rooted.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_ingrain(user)
}
