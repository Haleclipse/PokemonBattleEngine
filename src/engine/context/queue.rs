//! Moves that change the action queue (DESIGN.md §42.4): After You puts a Pokémon's pending
//! action first, Quash puts it last. The move asks (`BattleState::queue_requests`); the engine
//! changes the queue once the action is over.

use crate::ids::{MoveId, PokemonId};
use crate::state::{CallSource, CallTarget, MoveCall, QueueRequest};

use super::{BattleContext, ConditionApplyError, TraceEvent};

impl BattleContext<'_> {
    /// A move this effect makes `user` use next (Showdown `useMove(move, user, {target,
    /// sourceEffect})`, DESIGN.md §42.1): the Shell runs it once the effect in progress is over.
    /// The caller shows the move to the replay as `[from] <source>`.
    pub fn call_move(
        &mut self,
        user: PokemonId,
        move_id: MoveId,
        target: CallTarget,
        source: CallSource,
    ) {
        self.state.pending_call = Some(MoveCall {
            user,
            move_id,
            target,
            source,
            reflected_prankster: false,
        });
    }

    /// After You's `onHit`: in a battle with one Pokémon a side it fails (`activePerHalf ===
    /// 1`); otherwise the target's pending move action goes to the front of the queue
    /// (`queue.willMove`, `prioritizeAction`; `-activate|target|move: After You`), and a target
    /// that has moved already makes it fail.
    pub fn after_you(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        self.queue_request(target, QueueRequest::Prioritize(target))
    }

    /// Quash's `onHit`: as After You, but the target's action goes last (`-activate|target|move:
    /// Quash`; where, `ActionOrderRules::quash`).
    pub fn quash(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        self.queue_request(target, QueueRequest::Quash(target))
    }

    /// Instruct (Showdown `onHit`): the target uses its last move again, at once, from its own
    /// slot — a fresh queued action (`-singleturn|target|move: Instruct|[of] user`), while its
    /// own action for the turn stays. What may be instructed is the caller's check.
    pub fn instruct(
        &mut self,
        target: PokemonId,
        instructor: PokemonId,
        move_id: MoveId,
        slot: usize,
    ) {
        let target_position = self
            .state
            .pokemon(target)
            .and_then(|pokemon| pokemon.last_move_target);
        self.state.queue_requests.push(QueueRequest::Instruct {
            actor: target,
            move_id,
            slot,
            target_position,
        });
        self.trace.push(TraceEvent::Instructed {
            target,
            instructor,
            move_id,
        });
        // An instructed Counter or Mirror Coat without this turn's volatile gets it as Instruct
        // lands, as the chosen move would have had it since the turn began (issues.md I180, the
        // oracle patch's `addVolatile`; DESIGN.md R-M9-32). It lasts to the turn's end.
        let kind = match move_id {
            MoveId::COUNTER => Some(crate::state::VolatileConditionKind::Counter),
            MoveId::MIRROR_COAT => Some(crate::state::VolatileConditionKind::MirrorCoat),
            _ => None,
        };
        if let Some(kind) = kind
            && self
                .state
                .pokemon_mut(target)
                .is_some_and(|pokemon| pokemon.volatiles.set_turn_start(kind))
        {
            self.trace
                .push(TraceEvent::VolatileStarted { target, kind });
        }
    }

    fn queue_request(
        &mut self,
        target: PokemonId,
        request: QueueRequest,
    ) -> Result<(), ConditionApplyError> {
        if self.rules.topology.active_slots_per_side() == 1 || !self.state.will_move(target) {
            return Err(ConditionApplyError::NothingToDo);
        }
        self.state.queue_requests.push(request);
        self.trace
            .push(TraceEvent::QueueChangeRequested { request });
        Ok(())
    }
}
