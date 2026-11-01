//! Disable: seals the target's last move (Showdown `onTryHit` fails without one; the
//! condition's `onStart` sets the duration) — in Gen 1 a random move with PP instead
//! (`ConditionRules::disable_gen1`). Everything else — the hit gate, Substitute — is the
//! shared Move Shell's.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = targets
        .iter()
        .find_map(|target| match target {
            ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
            _ => None,
        })
        .ok_or(ConditionApplyError::InvalidTarget)?;
    let result = ctx.set_disable(target);
    // Aroma Veil's `-block|target|ability: Aroma Veil|[of] holder` (R-M10-40).
    if result == Err(ConditionApplyError::Immune) {
        ctx.announce_volatile_refusal(target, crate::state::VolatileConditionKind::Disable);
    }
    result
}
