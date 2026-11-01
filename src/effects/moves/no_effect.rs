//! Moves whose effect is a message (DESIGN.md §42): Splash's `onTryHit` (`-nothing`; its
//! `onTry` is Gravity's check for a Z-Splash — a plain one Gravity's `onBeforeMove` stops
//! before it), Celebrate's `onTryHit` (`-activate|user|move: Celebrate`) and Hold Hands, which
//! declares nothing at all.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn nothing(
    _ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    Ok(())
}
