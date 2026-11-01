//! Foresight, Odor Sleuth and Miracle Eye (DESIGN.md §41.5, K.7a): the target is identified —
//! `foresight` (Normal and Fighting moves hit it through a Ghost type) or `miracleeye` (Psychic
//! moves through a Dark type), and its evasion counts as `IdentifyRules::evasion` says
//! (`engine::type_immunity`, the accuracy check). The hit gate, Protect, Magic Coat and the
//! Substitute it bypasses are the shared Move Shell's.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::VolatileConditionKind;

/// Foresight, Odor Sleuth.
pub fn foresight(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    identify(ctx, targets, VolatileConditionKind::Foresight)
}

/// Miracle Eye.
pub fn miracle_eye(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    identify(ctx, targets, VolatileConditionKind::MiracleEye)
}

/// Showdown: the move's `onTryHit` fails on a target the other move identified, `addVolatile`
/// on one this move did (Gen 2's own `onTryHit`). Gen 3-4 on the cartridge
/// (`IdentifyRules::again_succeeds`, issues.md I89) the flag is set whatever is there.
fn identify(
    ctx: &mut BattleContext<'_>,
    targets: &[ResolvedTarget],
    kind: VolatileConditionKind,
) -> Result<(), ConditionApplyError> {
    let target = targets
        .iter()
        .find_map(|target| match target {
            ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
            _ => None,
        })
        .ok_or(ConditionApplyError::InvalidTarget)?;
    let other = match kind {
        VolatileConditionKind::Foresight => VolatileConditionKind::MiracleEye,
        _ => VolatileConditionKind::Foresight,
    };
    if ctx.rules().conditions().identify.again_succeeds {
        if ctx.has_volatile(target, kind) {
            return Ok(());
        }
    } else if ctx.has_volatile(target, other) {
        return Err(ConditionApplyError::Immune);
    }
    ctx.set_marker_volatile(target, kind)
}
