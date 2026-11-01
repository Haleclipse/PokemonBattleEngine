//! Corrosive Gas (DESIGN.md R213): `onHit` on each Pokémon around the user.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;

/// Each target's item is taken by the user and lost (`-enditem … [from] move: Corrosive Gas`);
/// one that keeps it shows `-fail`. The move does not fail either way (`onHit` returns nothing).
pub fn corrosive_gas(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    for target in targets {
        let ResolvedTarget::Pokemon(target) = *target else {
            continue;
        };
        let _ = ctx.take_item(target, user, Some(MoveId::CORROSIVE_GAS));
    }
    Ok(())
}
