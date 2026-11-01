//! Instruct (DESIGN.md §42.9): the target is made to use its last move again, at once — a fresh
//! queued action of its own (Showdown `queue.resolveAction` + `prioritizeAction`), so the target
//! still takes its own turn later. It fails when the target has no last move, when that move is
//! a Z-Move or a Max Move, carries `failinstruct`, charges or recharges, when the target is
//! charging one of the moves
//! with a turn-start volatile (Beak Blast, Focus Punch, Shell Trap) or has no PP left for it.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::VolatileConditionKind;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let dex = ctx.dex().ok_or(ConditionApplyError::NothingToDo)?;
    let Some(target_state) = ctx.battle_state().pokemon(target) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    // `instruct.onHit`: a Dynamaxed target is not instructed (`target.volatiles['dynamax']`,
    // and a Max Move's `lastMove` is refused below; DESIGN.md §44.5).
    if target_state
        .volatiles
        .contains(VolatileConditionKind::Dynamax)
    {
        return Err(ConditionApplyError::NothingToDo);
    }
    let Some(last) = target_state.last_move else {
        return Err(ConditionApplyError::NothingToDo);
    };
    let Some(move_def) = dex.move_by_id(last) else {
        return Err(ConditionApplyError::NothingToDo);
    };
    let charging = [
        VolatileConditionKind::BeakBlast,
        VolatileConditionKind::FocusPunch,
        VolatileConditionKind::ShellTrap,
    ]
    .into_iter()
    .any(|kind| target_state.volatiles.contains(kind));
    // Showdown's `lastMove.isZ || lastMove.isMax`: a Max Move a custom game put in a slot keeps
    // its PP (R-M8-80), and so may a Z-Move, its one PP given back by a Leppa Berry (R-M10-20).
    let refused = charging
        // A status move used as a Z-Move keeps its slot and PP; `lastMove.isZ` refuses it
        // (R-M9-36).
        || target_state.last_move_z
        || move_def.z_crystal.is_some()
        || move_def.is_max
        || move_def.flags.fail_instruct
        || move_def.flags.charge
        || move_def.flags.recharge;
    if refused {
        return Err(ConditionApplyError::NothingToDo);
    }
    // The move must be one of the target's own slots with PP left (Showdown's `moveSlot &&
    // moveSlot.pp <= 0`; a last move no slot holds — a called or copied one — is not
    // instructed here).
    let Some(slot) = target_state
        .move_slots()
        .iter()
        .position(|slot| slot.move_id == last && slot.pp > 0)
    else {
        return Err(ConditionApplyError::NothingToDo);
    };
    ctx.instruct(target, user, last, slot);
    Ok(())
}
