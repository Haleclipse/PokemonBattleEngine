use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let side = BattleContext::target_side(targets).ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.set_stealth_rock(side, user)
}
