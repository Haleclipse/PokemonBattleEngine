//! Spikes / Toxic Spikes / Sticky Web: side-targeted entry hazards. Laying a layer on a full
//! side fails; what they do on entry lives in the switch-in stage (`apply_entry_hazards`).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn spikes(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let side = BattleContext::target_side(targets).ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.add_spikes(side, user).map(|_| ())
}

pub fn toxic_spikes(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let side = BattleContext::target_side(targets).ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.add_toxic_spikes(side, user).map(|_| ())
}

pub fn sticky_web(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let side = BattleContext::target_side(targets).ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.set_sticky_web(side, user)
}
