//! Wide Guard / Quick Guard (priority +3, target `allySide`): the user's side is guarded for
//! the turn (a side condition checked at TryHit: spread moves / moves with positive priority,
//! only those with the `protect` flag). From Gen 6 they do not roll the Protect counter (Gen 5
//! does, `ConditionRules::guards_roll_stall`) but they do advance it, even when the side was
//! already guarded; they fail when nobody else still acts this turn (Showdown `willAct`). A
//! side already guarded is no failure: the `onHitSide` that advances the counter is what the
//! move did (R-M8-146).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::SideConditionKind;

/// Wide Guard / Quick Guard's `onTry` (`BattleContext::guard_try`).
pub fn guard_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    ctx.guard_try(user)
}

/// Crafty Shield's `onTry`: somebody still acts (`willAct`).
pub fn crafty_shield_tries(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
) -> Result<(), ConditionApplyError> {
    if ctx.anyone_will_act() {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}

/// Mat Block's `onTry`: the user's first move action since it came in (`activeMoveActions`),
/// and somebody still acts.
pub fn mat_block_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    if !ctx.first_move_action(user) {
        return Err(ConditionApplyError::NothingToDo);
    }
    crafty_shield_tries(ctx, user)
}

pub fn wide_guard(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    guard(ctx, user, targets, SideConditionKind::WideGuard)
}

pub fn quick_guard(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    guard(ctx, user, targets, SideConditionKind::QuickGuard)
}

fn guard(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
    kind: SideConditionKind,
) -> Result<(), ConditionApplyError> {
    let side = BattleContext::target_side(targets).ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.set_guard(side, kind, user)
}

/// Crafty Shield (Gen 6-8, priority +3): status moves aimed at the user's side are stopped for
/// the turn (`HitBlockReason::CraftyShield`); it fails when nobody else still acts (`onTry`:
/// `willAct`) or the side already is. It does not touch the Protect counter.
pub fn crafty_shield(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let side = BattleContext::target_side(targets).ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.set_single_turn_guard(side, SideConditionKind::CraftyShield, user)
}

/// Mat Block (Gen 6-8): every damaging move Protect would stop, aimed at the user's side, is
/// stopped for the turn (`HitBlockReason::MatBlock`) — only on the user's first move action since it
/// came in (`onTry`: `activeMoveActions`) and while someone still acts. Its `stallingMove` rolls
/// and advances nothing.
pub fn mat_block(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let side = BattleContext::target_side(targets).ok_or(ConditionApplyError::InvalidTarget)?;
    ctx.set_single_turn_guard(side, SideConditionKind::MatBlock, user)
}
