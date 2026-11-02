//! `run_move`'s end (DESIGN.md §48.3): what follows the hits — a combined Pledge's side
//! condition (Showdown `runMoveEffects`), a called move handed on, the user's own effects
//! (defrost, Explosion's faint, self boosts and drops, Mind Blown), the hit loop's faints,
//! Sky Drop's and the crash moves' failure, the in-move Update — and the move's result.

use super::*;

impl MovePipeline<'_> {
    /// The move's end, once its hits are over: the combined Pledge's side condition; a move
    /// this one called goes on as the action's (the end is the called move's); Gen 2's defrost
    /// after a landed hit; Explosion's faint (Gen 1, or Gen 2 and 4 after a target); the user's
    /// own effects; Mind Blown; the knocked-out Pokémon's faints; Sky Drop's failure; crash
    /// damage; Gen 4+'s Update after a move that went through; a burn cured; the result.
    pub(super) fn finish_move(&mut self, end: MoveEnd) -> Result<MoveExecution, BattleError> {
        let MoveEnd {
            actor,
            move_id,
            move_def,
            targets,
            releasing,
            defrost_after,
            always,
            pledge_condition,
            batch:
                HitBatch {
                    hits,
                    any_landed_hit,
                    move_failed,
                    custom_hit_pokemon,
                },
        } = end;
        // Explosion's timing (`ConditionRules::self_faint`).
        let self_faint = self.rules.conditions.self_faint;
        // The combined Pledge's side condition (Showdown `runMoveEffects` before the
        // secondaries): it lands once a hit went through — on the target's side only through
        // its body (`runMoveEffects` skips the `null` a Substitute's hit leaves, `if (target)`);
        // the user's own side (the rainbow is the move's `self`) through a Substitute too
        // (R-M8-139).
        let pledge_lands = |side: crate::engine::pledges::PledgeSide| match side {
            crate::engine::pledges::PledgeSide::User => any_landed_hit,
            crate::engine::pledges::PledgeSide::Target => hits
                .iter()
                .any(|hit| matches!(hit, HitOutcome::Damaged { .. })),
        };
        if let Some(combined) = pledge_condition
            && pledge_lands(combined.side)
        {
            let side = match combined.side {
                crate::engine::pledges::PledgeSide::User => {
                    self.state.pokemon(actor).map(|p| p.side)
                }
                crate::engine::pledges::PledgeSide::Target => targets
                    .pokemon_ids()
                    .next()
                    .and_then(|target| self.state.pokemon(target).map(|p| p.side)),
            };
            if let Some(side) = side {
                BattleContext::new(self.state, self.rules, self.trace)
                    .set_pledge_condition(side, combined.condition);
            }
        }

        // A move that called another (Metronome, Sleep Talk, Copycat …): Showdown's
        // `useMoveInner` runs the called move right here, inside the same `useMove`, and the
        // action reports it — everything after this point belongs to the called move, which the
        // pipeline runs next (DESIGN.md §42.1).
        // A reflection is the reflector's own `useMove`: the move it took is the actor's, as its
        // hits had it (a move every target bounced back failed).
        if let Some(call) = self.state.pending_call.as_ref() {
            let result = if call.user == actor {
                MoveResult::Succeeded
            } else {
                move_result(self.rules, &move_def, &hits, any_landed_hit, move_failed)
            };
            return Ok(MoveExecution {
                actor,
                move_id,
                hits,
                stopped: false,
                aborted: false,
                result,
            });
        }

        if defrost_after && any_landed_hit {
            self.thaw(actor, crate::engine::CureReason::Move(move_id));
        }
        // Gen 1 (`moveHit`'s `doSelf`): a hit that breaks a Substitute skips the move's own
        // `self` effects — Hyper Beam then owes no recharge.
        let broke_gen1_substitute = self.rules.conditions.substitute_gen1
            && hits
                .iter()
                .any(|hit| matches!(hit, HitOutcome::SubstituteDamaged { broken: true, .. }));
        // Gen 1 Explosion: the user faints once the move is over — hit, missed or not
        // affecting — unless it broke a Substitute.
        if always
            && self_faint.timing == crate::rules::SelfDestructTiming::AfterMove
            && !broke_gen1_substitute
        {
            BattleContext::new(self.state, self.rules, self.trace)
                .faint_by_own_move(actor, move_id);
        }
        // A multi-hit move's lands on every hit instead (`hit_sequence::multi_hit_self`).
        if any_landed_hit && !broke_gen1_substitute && move_def.multi_hit == MultiHit::Single {
            let move_target = targets.pokemon_ids().next();
            self.apply_secondary_effects(
                actor,
                actor,
                move_target,
                &move_def,
                SecondaryTrigger::OnceAfterMove,
                false,
            );
        }
        // Showdown `hitStepMoveHitLoop` → `mindBlownRecoil`: right after a hit that struck
        // something, before the struck targets' item reactions and the user's Life Orb.
        if struck_something(&hits) {
            self.after_move_self_damage(actor, move_id);
        }

        // The hit loop's `faintMessages` (Showdown `hitStepMoveHitLoop`) has reached the faints
        // so far: no longer Pokémon left for `foePokemonLeft`, no longer on the field for the
        // `Update` below. Announced only here, after the single-hit move's own `self`, which
        // Showdown lands inside the hit step (`selfDrops`) — Close Combat lowers its user's stats
        // even as it knocks the last foe out; nothing in between changes a stage — and after
        // Mind Blown's self damage, inside the loop too. The recoil the engine took with the
        // hits comes after it: a user that recoil knocked out is still pending (R-M8-166).
        // Gen 4+ runs the knocked-out Pokémon's `Faint` listeners and the batch's `AfterFaint`
        // there too (R-M10-52).
        BattleContext::new(self.state, self.rules, self.trace)
            .announce_knocked_out(self.recoil_knocked_out.then_some(actor));

        // Sky Drop's `onMoveFail` on the dropping turn (§15.23): a drop that came to nothing lets
        // the carried Pokémon down all the same (a miss, before its `onTryHit`) and ends the
        // charge (`twoturnmove`, at 1 turn left) — after its `onTryHit` let go too (a Flying
        // target's `-immune`, R-M8-157).
        if move_def.behavior.sky_drop
            && releasing
            && !any_landed_hit
            && self
                .state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::TwoTurnMove))
        {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            ctx.end_sky_drop(actor);
            ctx.end_volatile(actor, VolatileConditionKind::TwoTurnMove);
        }

        // Showdown `useMoveInner` → `singleEvent('MoveFail')`: a move that reached its targets
        // but hit none (a miss, an immunity, Protect — a Substitute counts as hit) crashes.
        if move_def.behavior.crash_damage && !hits.is_empty() && !struck_something(&hits) {
            self.crash(actor, targets.pokemon_ids().next(), &move_def);
        }

        // Gen 4+ (`hitStepMoveHitLoop`): once a hit went through on some target — a damaging
        // hit, a Substitute's, a status move's effect — the faints it caused are processed and
        // an `Update` runs, before the `AfterMoveSecondary` events. A status move's per-hit
        // `Update` comes just before this one with nothing between but those faints, so this
        // one stands for both.
        let went_through = custom_hit_pokemon
            || hits.iter().any(|hit| {
                matches!(
                    hit,
                    HitOutcome::Damaged { .. }
                        | HitOutcome::SubstituteDamaged { .. }
                        | HitOutcome::Effect { .. }
                )
            });
        if self.rules.update.in_move.after_move() && went_through {
            BattleContext::new(self.state, self.rules, self.trace)
                .run_update(crate::engine::PendingFaints::Announced);
        }
        self.binding_after_move(actor, &move_def, targets.pokemon_ids().next(), &hits);
        self.after_move(actor, &move_def, &hits, any_landed_hit);
        if move_def.behavior.cures_burn_after_move {
            self.sparkling_aria(actor, &move_def, &hits);
        }

        let result = move_result(self.rules, &move_def, &hits, any_landed_hit, move_failed);
        Ok(MoveExecution {
            actor,
            move_id,
            hits,
            stopped: false,
            aborted: false,
            result,
        })
    }
}
