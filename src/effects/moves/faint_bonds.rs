//! Destiny Bond and Grudge (DESIGN.md §15.9): volatiles on the user that act as it faints
//! (`BattleContext::faint_bonds`) and go at its next move attempt (`BattleContext::
//! drop_faint_bonds`).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::VolatileConditionKind;

/// Destiny Bond: its `onPrepareHit` (before Protean's, `CustomPrimary::tries`) takes a bond
/// still up away — Gen 7+ failing the move with it (`FaintBondRules::
/// destiny_bond_fails_in_a_row`), the gen6 mod to set it anew.
pub fn destiny_bond_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    let was_up = ctx.end_volatile(user, VolatileConditionKind::DestinyBond);
    if was_up
        && ctx
            .rules()
            .conditions()
            .faint_bonds
            .destiny_bond_fails_in_a_row
    {
        return Err(ConditionApplyError::AlreadyActive);
    }
    Ok(())
}

/// Destiny Bond past its `onPrepareHit`: the bond.
pub fn destiny_bond(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_marker_volatile(user, VolatileConditionKind::DestinyBond)
}

/// Grudge: the volatile on the user (it cannot be up still: every move attempt ends it first).
pub fn grudge(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_marker_volatile(user, VolatileConditionKind::Grudge)
}
