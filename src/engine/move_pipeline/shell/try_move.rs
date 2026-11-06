//! `run_move`'s TryMove (DESIGN.md §15.5, §48.3): Showdown `useMoveInner`'s `TryMove` event and
//! the moves' own `onTryMove` — a two-turn move's charge, Damp, Shell Trap, Powder, a strong
//! weather, Dazzling and its kin, Pollen Puff under Heal Block — and the no-target failure
//! that comes after them.

use super::*;

impl MovePipeline<'_> {
    /// TryMove: a two-turn move charges (the action ends) or releases; Gen 1 binding moves end
    /// the target's recharge; Burn Up and Double Shock need their type; Echoed Voice counts;
    /// Damp, Shell Trap, Powder, a strong weather and Dazzling stop the move; Explosion's user
    /// faints as the move starts (Gen 3, 5+); a move left with no target fails; Gen 2 and 4
    /// charge a target's self-destruct; Pollen Puff will not heal an ally under Heal Block.
    pub(super) fn try_move(
        &mut self,
        run: &MoveRun,
        targets: &crate::rules::ResolvedTargets,
    ) -> MoveStage<()> {
        let MoveRun {
            actor,
            move_id,
            ref move_def,
            action_target,
            overridden,
            ref resolved,
            releasing,
            ..
        } = *run;
        // Gen 1's charge model (`ChargeModel::Gen1`).
        let gen1_charge = self.rules.two_turn.gen1();
        // TryMove (DESIGN.md §15.5): a two-turn move either starts its charge here and ends
        // the action (Showdown `-prepare`, `onTryMove` → null; it was not redirected above),
        // releases the charge started last turn, or attacks at once when the weather says so.
        // It comes before the no-target failure (`useMoveInner` reports `-notarget` only after
        // TryMove): Dig aimed at a foe that just fainted still goes underground.
        if let Some(spec) = move_def.behavior.two_turn {
            if releasing {
                if gen1_charge {
                    // Gen 1: the attack clears the charge and the hiding at once.
                    let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                    ctx.end_volatile(actor, VolatileConditionKind::TwoTurnMove);
                    ctx.end_volatile(actor, VolatileConditionKind::Invulnerability);
                } else if let Some(pokemon) = self.state.pokemon_mut(actor) {
                    pokemon.volatiles.release_two_turn_move();
                }
                self.trace
                    .push(TraceEvent::ChargeReleased { actor, move_id });
            } else {
                self.trace
                    .push(TraceEvent::ChargeStarted { actor, move_id });
                // The user's own stat change as it charges (`this.boost` after `-prepare`),
                // before the weather can send the attack out at once (Electro Shot). Gen 1's
                // Skull Bash raises nothing.
                if let Some((stat, stages)) = spec.charge_boost.filter(|_| !gen1_charge) {
                    BattleContext::new(self.state, self.rules, self.trace).try_change_stat_stage(
                        crate::engine::StatChangeRequest {
                            target: actor,
                            stat,
                            stages,
                            source: EffectSource::Move {
                                pokemon: actor,
                                move_id,
                            },
                            cause: crate::engine::StatChangeCause::Move,
                        },
                    );
                }
                match crate::engine::two_turn::charge_skip(self.state, self.rules, actor, spec) {
                    Some(cause) => {
                        // Power Herb (`useItem`): `-enditem`, the move goes ahead this turn.
                        if let crate::engine::two_turn::ChargeSkipCause::Item(_) = cause {
                            BattleContext::new(self.state, self.rules, self.trace)
                                .consume_item(actor);
                        }
                        self.trace.push(TraceEvent::ChargeSkipped {
                            actor,
                            move_id,
                            cause,
                        });
                    }
                    None => {
                        let turns = self.rules.conditions.two_turn_move_turns;
                        // The attack aims at the action's slot (Showdown `lastMoveTargetLoc`),
                        // before any redirection: the declared one, even when an Encore turned
                        // the action into this move and drew this turn's target anew. An action
                        // chosen without one got its `targetLoc` as it was queued
                        // (`resolveAction`'s `getRandomTarget` on the chosen move), not from this
                        // turn's draw: the user's own slot for a move for itself or its side
                        // (Iron Defense), which the release does not take (`getTarget` →
                        // `getRandomTarget`, R-M9-26); none for one for an ally in a single
                        // battle; for any other (Stealth Rock, Razor Wind) a foe drawn among
                        // those standing then — the release's draw among the foes standing in
                        // those slots stands for it, the attack going at one of them evenly
                        // (R-M10-20; it would not where a slot empty then was filled since, by a
                        // Revival Blessing). So an overridden action's release draws. Otherwise
                        // the action's own move: a called one at the target drawn for it
                        // (`twoturnmove.onStart`'s `effect.sourceEffect`), a single battle's at
                        // the foe.
                        let aimed = action_target.map(|d| d.0).or_else(|| {
                            if overridden {
                                return None;
                            }
                            resolved
                                .as_ref()
                                .and_then(|targets| match targets.targets[..] {
                                    [ResolvedTarget::Pokemon(target)] if target != actor => {
                                        self.state.active_position(target, &self.rules.topology)
                                    }
                                    _ => None,
                                })
                        });
                        if let Some(pokemon) = self.state.pokemon_mut(actor)
                            && pokemon.volatiles.set_two_turn_move(
                                move_id,
                                turns,
                                aimed,
                                spec.invulnerable,
                            )
                        {
                            self.trace.push(TraceEvent::VolatileStarted {
                                target: actor,
                                kind: VolatileConditionKind::TwoTurnMove,
                            });
                        }
                        // The charge's `onStart` runs the `PrepareHit` event's side effects
                        // (Showdown: "e.g., Protean, Libero") on the charging turn.
                        BattleContext::new(self.state, self.rules, self.trace)
                            .protean(actor, move_def.move_type);
                        // Dive's `onTryMove` (DESIGN.md §43.4): a Cramorant comes up with its
                        // catch as it dives.
                        if move_def.id == crate::ids::MoveId::DIVE
                            && self.state.pokemon(actor).is_some_and(|pokemon| {
                                crate::engine::ability_handler(
                                    self.state,
                                    self.rules,
                                    pokemon,
                                    |b| b.gulp_missile,
                                )
                            })
                        {
                            crate::effects::abilities::forme_ability::gulp_missile_dive(
                                &mut BattleContext::new(self.state, self.rules, self.trace),
                                actor,
                            );
                        }
                        if gen1_charge {
                            // Gen 1 Fly / Dig hide the user with a flag of its own.
                            if let Some(kind) = spec.invulnerable
                                && let Some(pokemon) = self.state.pokemon_mut(actor)
                                && pokemon.volatiles.set_invulnerability(kind)
                            {
                                self.trace.push(TraceEvent::VolatileStarted {
                                    target: actor,
                                    kind: VolatileConditionKind::Invulnerability,
                                });
                            }
                            // The gen1 `useMove`: the charging turn is the side's last move.
                            if let Some(side) = self.state.pokemon(actor).map(|p| p.side) {
                                self.state.side_mut(side).last_move = Some(move_id);
                            }
                        }
                        // Showdown `onTryMove` → `null`: charging is neither.
                        return Ok(ControlFlow::Break(MoveExecution {
                            actor,
                            move_id,
                            hits: Vec::new(),
                            stopped: false,
                            aborted: false,
                            result: MoveResult::Neither,
                        }));
                    }
                }
            }
        }

        // Gen 1 binding moves' own `onTryMove`: the target's recharge turn ends.
        self.binding_ends_recharge(
            move_def,
            resolved.as_ref().and_then(|t| t.pokemon_ids().next()),
        );

        // Burn Up, Double Shock (the move's own `onTryMove`, before the `TryMove` event's
        // handlers): a user without the type stops (`-fail`), neither failing nor succeeding.
        if let Some(type_) = move_def.behavior.burns_up
            && !BattleContext::new(self.state, self.rules, self.trace).has_type(actor, type_)
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
        self.metronome_tries(actor, move_id, releasing);
        // Echoed Voice's own `onTryMove` (`singleEvent('TryMove', move)`), its PP spent: before
        // the `TryMove` event's handlers, so a strong weather that then stops the move (a Liquid
        // Voice made it Water under Desolate Land) leaves the condition started (R-M8-181); and
        // with the target `useMoveInner` picked — a fainted foe still in its slot too
        // (`getRandomTarget`) — before the move finds no target standing (`[notarget]`).
        if move_def.behavior.echoes && self.state.field.echo() {
            self.trace.push(TraceEvent::FieldConditionStarted {
                kind: crate::state::FieldConditionKind::EchoedVoice,
            });
        }
        // Shell Trap's `onTryMove` (§15.8): not armed by a foe's physical hit, it stops (`cant`),
        // the PP spent, neither succeeding nor failing.
        if move_def.behavior.needs_armed_trap
            && !self
                .state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.shell_trap_armed())
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
        // Powder's `onTryMove` in Gen 6 (priority 1, ahead of the field's strong weather).
        if self.rules.conditions.powder_before_strong_weather
            && let Some(execution) = self.powder_stops(actor, move_def)
        {
            return Ok(ControlFlow::Break(execution));
        }
        // A strong weather's `onTryMove` (§23.5): the PP spent, nothing else happens. It returns
        // `null`, so the move neither succeeded nor failed (Stomping Tantrum is not doubled).
        if let Some(weather) =
            crate::engine::weather::strong_weather_stops(self.state, self.rules, move_def)
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::Weather(weather),
            });
            self.self_damage_cancelled = true;
            return Ok(ControlFlow::Break(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Neither,
            }));
        }
        // Showdown `runEvent('TryMove')`: an active Damp stops Explosion and its kin
        // (`onAnyTryMove`, priority 0: after a strong weather's, so Desolate Land's or Primordial
        // Sea's stops a Mind Blown first, Damp unseen, R-M11-76) — the PP spent, the user
        // standing.
        if move_def.behavior.stopped_by_damp
            && let Some(holder) = crate::engine::context::damp_holder(self.state, self.rules)
        {
            let ability = self
                .state
                .pokemon(holder)
                .map_or(crate::ids::AbilityId::DAMP, crate::engine::raw_ability);
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::Ability { holder, ability },
            });
            self.self_damage_cancelled = true;
            return Ok(ControlFlow::Break(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Failed,
            }));
        }
        // Dazzling, Queenly Majesty, Armor Tail (`onFoeTryMove`, priority 0): a foe's move of
        // raised priority aimed at their side fails (`cant|holder|ability: …|move|[of] user`).
        // TryMove's target is the last of the move's targets (`targets[targets.length - 1]`, "in
        // case of redirection"): a move an ally's Storm Drain drew is aimed at the user's own
        // side (R-M10-11); with none, the one it was given.
        if let Some((holder, ability)) = crate::engine::priority_guard(
            self.state,
            self.rules,
            actor,
            targets.pokemon_ids().last().or_else(|| {
                resolved
                    .as_ref()
                    .and_then(|targets| targets.pokemon_ids().next())
            }),
            move_def,
            self.move_priority(move_def),
        ) {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::Ability { holder, ability },
            });
            self.self_damage_cancelled = true;
            return Ok(ControlFlow::Break(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Failed,
            }));
        }
        // Powder's `onTryMove` from Gen 7 (priority −1, last).
        if !self.rules.conditions.powder_before_strong_weather
            && let Some(execution) = self.powder_stops(actor, move_def)
        {
            return Ok(ControlFlow::Break(execution));
        }
        // Gen 1 `useMoveInner`: the move is going ahead — this side's last move, and the last
        // damage starts over (`SKIP_LASTDAMAGE` aside).
        let retaliation = self.rules.conditions.retaliation;
        if retaliation.gen1_last_damage {
            if let Some(side) = self.state.pokemon(actor).map(|p| p.side) {
                self.state.side_mut(side).last_move = Some(move_id);
            }
            if !retaliation.keeps_last_damage.contains(&move_id) {
                self.state.last_damage = 0;
            }
        }
        // Explosion and its kin (`selfdestruct: 'always'`), Gen 3 and 5+: the user faints as
        // the move starts, target or not (§30.4).
        let self_faint = self.rules.conditions.self_faint;
        let always = move_def.self_destruct == Some(crate::dex::SelfDestruct::Always);
        if always && self_faint.timing == crate::rules::SelfDestructTiming::BeforeTargets {
            BattleContext::new(self.state, self.rules, self.trace)
                .faint_by_own_move(actor, move_id);
        }

        let pokemon_target_count = targets.pokemon_count();
        let no_usable_target = targets.targets.is_empty()
            || (move_def.primary_effect == PrimaryEffect::Damage && pokemon_target_count == 0);
        // A future move keeps a fainted target (`getMoveTargets`): it is foreseen on its slot.
        if no_usable_target && !move_def.behavior.future_attack {
            return Ok(ControlFlow::Break(self.fail_no_target(actor, move_id)));
        }
        // Gen 2 and 4: only a move with a target costs its user, before its hit steps.
        if always && self_faint.timing == crate::rules::SelfDestructTiming::WithTargets {
            BattleContext::new(self.state, self.rules, self.trace)
                .faint_by_own_move(actor, move_id);
        }

        // Pollen Puff's `onTryMove`: a user under Heal Block cannot aim it at an ally
        // (`cant|user|move: Heal Block|Pollen Puff`); the move fails.
        if self
            .state
            .pokemon(actor)
            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::HealBlock))
            && targets.pokemon_ids().any(|target| {
                crate::engine::heals_instead(self.state, self.rules, actor, target, move_def, None)
                    .is_some()
            })
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::BeforeMove(crate::engine::BeforeMoveBlockReason::HealBlock),
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
        Ok(ControlFlow::Continue(()))
    }
}
