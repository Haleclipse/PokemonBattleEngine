//! Me First (DESIGN.md §42.8): the user takes the move its target has queued this turn and uses
//! it itself, ×1.5 (the `mefirst` volatile marks the boost). It fails when the target has
//! nothing queued, is recharging, chose a status move or a move carrying `failmefirst`, or a
//! Z-Move — one its Z-Power makes (Showdown `action.zmove`) or one in its slot (`move.isZ`,
//! DESIGN.md R-M8-71).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::{CallSource, CallTarget, VolatileConditionKind};

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let Some(queued) = ctx.battle_state().will_move_with(target) else {
        return Err(ConditionApplyError::NothingToDo);
    };
    if queued.category == crate::dex::MoveCategory::Status || queued.z_powered {
        return Err(ConditionApplyError::NothingToDo);
    }
    let recharging = ctx.battle_state().pokemon(target).is_some_and(|pokemon| {
        pokemon
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
    });
    // A Z-Move the target has in a slot is refused as one its Z-Power makes (`move.isZ`).
    let refused = recharging
        || ctx
            .dex()
            .and_then(|dex| dex.move_by_id(queued.move_id))
            .is_none_or(|move_def| move_def.flags.fail_me_first || move_def.z_crystal.is_some());
    if refused {
        return Err(ConditionApplyError::NothingToDo);
    }
    if ctx.set_me_first(user) {
        ctx.trace(crate::engine::TraceEvent::VolatileStarted {
            target: user,
            kind: VolatileConditionKind::MeFirst,
        });
    }
    ctx.call_move(
        user,
        queued.move_id,
        CallTarget::Pokemon(target),
        CallSource::Move(MoveId::ME_FIRST),
    );
    Ok(())
}
