//! `run_move`'s PrepareHit (DESIGN.md §48.3): a custom primary's own `onTry` / `onPrepareHit`,
//! Snatch's take, Round, Fling's and Natural Gift's item, a combined Pledge, the `PrepareHit`
//! event's handlers (Protean, Libero) — the move as it will hit.

use super::*;

impl MovePipeline<'_> {
    /// PrepareHit: a custom primary's own checks; Snatch takes the move (before `onTry` in Gen
    /// 3, at the event's end later); Round moves the next Round up; Fling and Natural Gift take
    /// their power from the item, or fail without one; Parental Bond; Toxic's sure hit from a
    /// Poison type (Gen 6-7); a Pledge waits for its ally's or combines with it; Protean and
    /// Libero retype the user. On: the move as it will hit, and a combined Pledge's side
    /// condition.
    pub(super) fn prepare_hit(
        &mut self,
        actor: PokemonId,
        move_id: MoveId,
        move_def: ResolvedMove,
        targets: &crate::rules::ResolvedTargets,
        pokemon_target_count: usize,
    ) -> MoveStage<(ResolvedMove, Option<crate::engine::pledges::PledgeCombo>)> {
        // A custom primary's own `onTry` / `onPrepareHit` (Teleport, Rest, a stalling move's
        // roll …), before the `PrepareHit` handlers: a move it fails never meets Protean. Gen 5+
        // Swallow's lets a snatched Swallow through.
        let snatched = matches!(
            self.call.map(|frame| frame.source),
            Some(crate::state::CallSource::Move(MoveId::SNATCH))
        );
        let passes_snatched = snatched
            && self.rules.conditions().snatched_swallow_skips_stockpile
            && effects::moves::tries_pass_snatched(move_id);
        // Gen 3 runs the `PrepareHit` event, Snatch's take among it, before the move's own
        // `onTry` (`CallRules::snatch_before_try`): a Swallow with nothing stockpiled is taken
        // all the same, and fails for its taker (R-M8-102).
        let snatch_before_try = self.rules.conditions.calls.snatch_before_try;
        // A move Snatch took failed for its user: the take's `PrepareHit` returns `null`, and
        // `trySpreadMoveHit` answers `null === NOT_FAIL`, `false` (a Stomping Tantrum next turn is
        // doubled, R-M8-138).
        if snatch_before_try && self.snatch_takes(actor, &move_def, snatched) {
            return Ok(ControlFlow::Break(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Failed,
            }));
        }
        if let Some(tries) = effects::moves::custom(move_id)
            .and_then(|primary| primary.tries)
            .filter(|_| !passes_snatched)
        {
            let result = {
                let mut ctx =
                    BattleContext::new(self.state, self.rules, self.trace).with_dex(self.dex);
                tries(&mut ctx, actor)
            };
            if let Err(error) = result {
                self.trace.push(TraceEvent::MoveFailed {
                    actor,
                    move_id,
                    reason: MoveFailReason::Condition(error),
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
        }

        // Round's `onTry`: the next Round in the queue goes to the front once this move is over.
        if move_def.behavior.prioritizes_same_move {
            self.prioritizes = Some(move_id);
        }

        // `onPrepareHit` (Showdown: the move's own, then Parental Bond's): Fling's and Natural
        // Gift's power from the item, then twice, unless the move hits several targets.
        let mut move_def = crate::engine::prepared_move(
            self.state,
            self.rules,
            actor,
            move_def,
            pokemon_target_count,
            targets.pokemon_ids().next(),
        );
        // Fling, Natural Gift (`onPrepareHit`): without an item to throw or a berry to use, the
        // move fails (`-fail`); Fling marks its item to go at the next `Update`, Natural Gift
        // uses its berry up at once — not eaten. Gen 1-2 charging moves (`onPrepareHit`:
        // `source.status !== 'slp'`) fail as a sleeping user releases them (through Sleep Talk).
        let asleep_release = move_def.behavior.charge_fails_asleep
            && self
                .state
                .pokemon(actor)
                .is_some_and(|p| p.has_status(NonVolatileStatus::Sleep));
        if asleep_release
            || ((move_def.behavior.fling || move_def.behavior.natural_gift)
                && !self.prepare_item_move(actor, &move_def))
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

        // Showdown `toxic` `onPrepareHit` (the gen7 mod: Gen 6-7), once the move has targets and
        // passed `onTry`: a Poison-type user takes the `toxic` volatile that makes its Toxic
        // unmissable. Gen 8+ check it in the hit steps and leave nothing on the user.
        if self.rules.conditions.typed_sure_hit == Some(crate::rules::TypedSureHit::Volatile)
            && crate::engine::sure_hit(self.state, self.rules, actor, &move_def)
            && self
                .state
                .pokemon_mut(actor)
                .is_some_and(|p| p.volatiles.set_sure_hit())
        {
            self.trace.push(TraceEvent::VolatileStarted {
                target: actor,
                kind: VolatileConditionKind::SureHit,
            });
        }

        // A Pledge's own `onPrepareHit` (DESIGN.md §42.9): an ally still holding another Pledge
        // this turn takes this one's place — that action goes to the front with this move as its
        // `sourceEffect`, and this move stops (`useMoveInner` returns `null`). A called Pledge
        // waits too: the handler is the move's own (R-M8-65).
        if move_def.behavior.pledge_combo
            && self.rules.conditions.calls.pledge_combines
            && let Some(ally_move) = self.pledge_partner(actor, move_id)
        {
            self.pledge_promotes = Some(ally_move);
            self.trace.push(TraceEvent::PledgeWaiting {
                waiting: actor,
                move_id,
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
        // The combined move (Showdown's `basePowerCallback` and `onModifyMove` with a
        // `sourceEffect` of another Pledge): 150 power and the pair's side condition — its type
        // and `forceSTAB` were set as the move started.
        let mut pledge_condition: Option<crate::engine::pledges::PledgeCombo> = None;
        if move_def.behavior.pledge_combo
            && let Some(by) = self.prioritized_by
            && let Some(combined) = crate::engine::pledges::combo(by, move_id)
        {
            move_def.base_power = 150;
            pledge_condition = Some(combined);
            self.trace.push(TraceEvent::PledgeCombined {
                users: (actor, by),
                move_id,
            });
        }

        // The `PrepareHit` event's handlers, after the move's own: Protean, Libero — which leave
        // alone a move out of a reflection (`hasBounced`), a snatched move and one that calls
        // another (`callsMove`: Mirror Move, Copycat …; the move it calls retypes the user),
        // DESIGN.md R-M8-80. (Their `futuremove` line has nothing to do here: Future Sight ends
        // at its `onTry`, before this event.)
        if !(self.bounced || snatched || move_def.calls_move) {
            BattleContext::new(self.state, self.rules, self.trace)
                .protean(actor, move_def.move_type);
        }
        // Snatch's `onAnyPrepareHit` (priority −1, last of the event): a Snatch holder takes a
        // move carrying the `snatch` flag over and uses it itself (DESIGN.md §42.8). The outer
        // move stops where it stands (`useMoveInner` returns `null`). A called move too
        // (Metronome's Swords Dance, Dancer's); a move Snatch itself called only where the
        // generation takes it again (R-M8-68). Gen 3 took it before `onTry`.
        if !snatch_before_try && self.snatch_takes(actor, &move_def, snatched) {
            return Ok(ControlFlow::Break(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Failed,
            }));
        }
        Ok(ControlFlow::Continue((move_def, pledge_condition)))
    }
}
