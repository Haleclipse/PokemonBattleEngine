//! Trick, Switcheroo, Bestow (DESIGN.md R214): their `onHit` on the target. Recycle
//! (DESIGN.md R239): its `onHit` on the user.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;

fn swap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
    move_id: MoveId,
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    if ctx.swap_items(user, target, move_id) {
        Ok(())
    } else {
        Err(ConditionApplyError::NothingToDo)
    }
}

/// Trick: the user and the target swap items (`BattleContext::swap_items`).
pub fn trick(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    swap(ctx, user, targets, MoveId::TRICK)
}

/// Switcheroo: Trick under another name.
pub fn switcheroo(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    swap(ctx, user, targets, MoveId::SWITCHEROO)
}

/// Bestow: a target holding nothing receives the user's item (`BattleContext::bestow`).
pub fn bestow(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    if ctx.bestow(user, target) {
        Ok(())
    } else {
        Err(ConditionApplyError::NothingToDo)
    }
}

/// Recycle: the user takes back the item it last used up or ate (`BattleContext::recycle`).
pub fn recycle(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    if ctx.recycle(user) {
        Ok(())
    } else {
        Err(ConditionApplyError::NothingToDo)
    }
}
