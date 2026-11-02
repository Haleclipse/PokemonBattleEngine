//! `run_move`'s `onTry` (DESIGN.md §48.3): the moves' own `Try` handlers after TryMove — Focus
//! Punch's lost focus, Fake Out's first turn, Future Sight foreseen.

use super::*;

impl MovePipeline<'_> {
    /// The moves' own `onTry`: Gen 3-4 Focus Punch stops when its user lost its focus; Fake Out
    /// and its kin fail after their user's first turn; Future Sight and Doom Desire are
    /// foreseen, and the move stops there.
    pub(super) fn on_try(
        &mut self,
        run: &MoveRun,
        targets: &crate::rules::ResolvedTargets,
    ) -> MoveStage<()> {
        let MoveRun {
            actor,
            move_id,
            ref move_def,
            declared_target,
            ..
        } = *run;
        // Gen 3-4 Focus Punch's `onTry` (§15.8): a user that lost its focus stops (`cant`), the
        // PP spent, neither succeeding nor failing.
        if move_def.behavior.lost_focus_fails
            && self
                .state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.lost_focus())
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::NoEffect,
            });
            return Ok(ControlFlow::Break(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Neither,
            }));
        }
        // Showdown `onTry` (Fake Out): after targeting, before any hit step.
        if let Some(try_use) = move_def.behavior.try_use
            && !crate::engine::try_use_holds(
                self.state,
                self.rules,
                actor,
                targets.pokemon_ids().next(),
                try_use,
            )
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::NoEffect,
            });
            return Ok(ControlFlow::Break(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Failed,
            }));
        }

        // Future Sight, Doom Desire (`onTry`): the attack is foreseen and the move stops.
        if move_def.behavior.future_attack {
            let spot = self.foreseen_spot(actor, declared_target, targets);
            return Ok(ControlFlow::Break(self.foresee(actor, move_def, spot)));
        }
        Ok(ControlFlow::Continue(()))
    }
}
