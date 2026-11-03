//! `BattleEngine`'s turn (DESIGN.md §11): the choices submitted, the engine stepping to its next
//! request — each queued action run (`execute_queued_action`) — Dynamax's end, the faint check,
//! the next turn.

use crate::engine::{
    ActionChoice, ActionStep, BattleError, BattleRequest, EngineStep, MechanicChoice, MovePipeline,
    PendingExecution, PendingFaints, QueuedAction, SideChoice, TraceEvent,
};
use crate::ids::{PokemonId, SideId};
use crate::state::{BattlePhase, VolatileConditionKind};

use super::*;

impl BattleEngine {
    pub(super) fn submit_turn_choices_inner(
        &mut self,
        choices: [SideChoice; 2],
    ) -> Result<(), BattleError> {
        self.ensure_phase(BattlePhase::TurnChoice)?;
        self.validate_choice(SideId::P1, &choices[0])?;
        self.validate_choice(SideId::P2, &choices[1])?;

        self.trace.push(TraceEvent::TurnStarted {
            turn: self.state.turn,
        });
        self.set_phase(BattlePhase::TurnResolution);
        self.state.turn_actions_done = false;
        let queue = self.build_action_queue(&choices)?;
        self.record_selected_moves(&choices);
        let mut actions = queue.into_actions();
        // Gen 1 (`ActionOrderRules::recharge_acts_with_last_selected`): a recharging Pokémon
        // acts with its side's last selected move, which the recharge turn left as it was; so
        // does a frozen or sleeping one (`sleeping_frozen_act_with_last_selected`: its menu is
        // `Fight` alone, `commitChoices` sets `choice.moveid = lastSelectedMove`) — the move a
        // Pokémon thawed by a Fire move uses later in the turn (R-M8-145).
        let order = self.rules.action_order;
        self.state.fight_actors.clear();
        if order.recharge_acts_with_last_selected || order.sleeping_frozen_act_with_last_selected {
            for action in &mut actions {
                let fight = order.sleeping_frozen_act_with_last_selected
                    && self.state.pokemon(action.actor).is_some_and(|pokemon| {
                        pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
                            || pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                    });
                if let ActionChoice::Move { slot, .. } = &mut action.choice
                    && (fight
                        || (order.recharge_acts_with_last_selected
                            && crate::engine::choice::recharging(&self.state, action.actor)))
                    && let Some(pokemon) = self.state.pokemon(action.actor)
                    && let Some(last) = crate::ids::MoveSlot::new(
                        self.state.side(pokemon.side).last_selected_move_slot as u8,
                    )
                {
                    *slot = last;
                    if fight {
                        self.state.fight_actors.push(action.actor);
                    }
                }
            }
        }
        // Showdown `beforeTurnMove` (`priorityChargeCallback`): a chosen Chilly Reception puts
        // its announcement on its user as the turn begins, in action order.
        self.state.pending_movers = actions
            .iter()
            .filter(|action| matches!(action.choice, ActionChoice::Move { .. }))
            .map(|action| action.actor)
            .collect();
        self.state.struggling = self
            .state
            .pending_movers
            .iter()
            .copied()
            .filter(|actor| {
                crate::engine::choice::must_struggle(&self.state, &self.rules, &self.dex, *actor)
            })
            .collect();
        self.state.cannot_move = self.bound_movers();
        self.state.curse_glitched.clear();
        // The actors whose Dynamax is a step of this turn (`ActionStep::Dynamax`: their move
        // action carries no mechanic any more).
        let dynamaxing: Vec<PokemonId> = actions
            .iter()
            .filter(|action| action.step == ActionStep::Dynamax)
            .map(|action| action.actor)
            .collect();
        // What each move action names, for Sucker Punch / Upper Hand (`queue.willMove`): the
        // chosen move (Struggle when nothing was left), its category, its ordered priority.
        self.state.queued_moves = actions
            .iter()
            // The move's own step: a Mega Evolution's, a Dynamax's or a Terastallization's step
            // is no move (`queue.willMove` finds the `move` action, R-M9-32).
            .filter(|action| action.step == ActionStep::Act)
            .filter_map(|action| {
                let ActionChoice::Move { slot, mechanic, .. } = action.choice else {
                    return None;
                };
                // A locked Pokémon's action is its lock: Me First copies a rampage's next turn
                // (R-M8-66).
                let move_id = if self.state.struggling.contains(&action.actor) {
                    crate::ids::MoveId::STRUGGLE
                } else {
                    self.action_move(action.actor, slot)?
                };
                let category = self.dex.move_by_id(move_id)?.category;
                Some(crate::state::QueuedMove {
                    actor: action.actor,
                    move_id,
                    category,
                    priority: action.order.move_priority,
                    z_powered: mechanic == Some(MechanicChoice::ZMove),
                    max_move: mechanic == Some(MechanicChoice::Dynamax)
                        || dynamaxing.contains(&action.actor)
                        || self.state.pokemon(action.actor).is_some_and(|pokemon| {
                            pokemon.volatiles.contains(VolatileConditionKind::Dynamax)
                        }),
                    recharge: self.state.pokemon(action.actor).is_some_and(|pokemon| {
                        pokemon
                            .volatiles
                            .contains(VolatileConditionKind::MustRecharge)
                    }),
                })
            })
            .collect();
        // Showdown `beforeTurnMove` / `priorityChargeMove` (Chilly Reception, Focus Punch, Beak
        // Blast, Shell Trap, Counter, Mirror Coat): the move a Pokémon will use puts its volatile
        // on it as the turn begins, in action order (a Pokémon left to Struggle announces
        // nothing, nor does a recharging one: its action is the `recharge` pseudo-move,
        // R-M8-81). A Max Move or a Z-Move announces nothing either (`!action.maxMove &&
        // !action.zmove`, R-M9-30).
        let announced: Vec<(PokemonId, VolatileConditionKind)> = self
            .state
            .queued_moves
            .iter()
            .filter(|queued| queued.walked_move().is_some())
            .filter_map(|queued| {
                let kind = self
                    .dex
                    .move_by_id(queued.move_id)?
                    .behavior
                    .turn_start_volatile?;
                Some((queued.actor, kind))
            })
            .collect();
        for (actor, kind) in announced {
            if self
                .state
                .pokemon_mut(actor)
                .is_some_and(|pokemon| pokemon.volatiles.set_turn_start(kind))
            {
                self.trace.push(TraceEvent::VolatileStarted {
                    target: actor,
                    kind,
                });
            }
        }
        // Rage as the turn starts (§15.19): Gen 2-3 a raging Pokémon that did not choose Rage
        // stops raging; Gen 4 it loses what the cartridge's bug wipes instead.
        self.rage_turn_start();
        // Pursuit's `beforeTurnCallback` (§15.11).
        self.mark_pursuit_targets(&actions);
        self.pending = PendingExecution::Turn {
            actions,
            strategy: self.rules.action_order.strategy,
            residual: true,
            resort_ties: false,
        };
        Ok(())
    }

    /// Gen 2-4 on the cartridges (pokecrystal `ParsePlayerAction`, pokeemerald
    /// `TryClearRageStatuses`, pokeplatinum `PRE_MOVE_ACTION_STATE_CHECK_RAGE_FLAG`; §15.19):
    /// every raging Pokémon whose action this turn is not Rage — a switch, Struggle, any other
    /// move — stops raging (`RageEnd::TurnStart`) or loses the volatiles Gen 4's bug wipes
    /// (`RageEnd::Gen4`).
    fn rage_turn_start(&mut self) {
        let end = self.rules.conditions.rage.end;
        if end == crate::rules::RageEnd::BeforeMove {
            return;
        }
        let raging: Vec<PokemonId> =
            [SideId::P1, SideId::P2]
                .into_iter()
                .flat_map(|side| self.state.side(side).active.iter().flatten().copied())
                .filter(|id| {
                    self.state
                        .pokemon(*id)
                        .is_some_and(|p| !p.is_fainted() && p.volatiles.rage().is_some())
                })
                .filter(|id| {
                    !self.state.queued_moves.iter().any(|queued| {
                        queued.actor == *id && queued.move_id == crate::ids::MoveId::RAGE
                    })
                })
                .collect();
        let mut ctx =
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        for pokemon in raging {
            if end == crate::rules::RageEnd::TurnStart {
                ctx.end_rage(pokemon);
            } else {
                ctx.rage_wipe(pokemon);
            }
        }
    }

    pub(super) fn step_inner(&mut self) -> Result<EngineStep, BattleError> {
        if let Some(outcome) = self.state.outcome {
            return Ok(EngineStep::Finished(outcome));
        }

        match self.state.phase {
            BattlePhase::TurnChoice => {
                return Ok(EngineStep::Awaiting(BattleRequest::TurnChoices {
                    sides: [SideId::P1, SideId::P2],
                }));
            }
            BattlePhase::ForcedSwitch => {
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: self.replacement_requirements(),
                }));
            }
            BattlePhase::Revive => {
                if let Some(request) = self.revive_request() {
                    return Ok(EngineStep::Awaiting(request));
                }
                self.set_phase(BattlePhase::TurnResolution);
            }
            _ => {}
        }

        // A U-turn user, Eject Button holder … marked by Pursuit or Dynamaxed: its
        // `BeforeSwitchOut` before the replacement request (Showdown `runAction`), then the
        // faints (§15.11, R-M9-21).
        if self.state.phase == BattlePhase::TurnResolution
            && let Some(outgoing) = self.before_switch_out_due()
        {
            self.state.skip_before_switch_out.push(outgoing);
            self.before_switch_out(outgoing)?;
            self.state.clear_active_move();
            self.resolve_faints();
            if let Some(outcome) = self.state.outcome {
                self.pending = PendingExecution::Idle;
                return Ok(EngineStep::Finished(outcome));
            }
            if self.mid_turn_replacement_needed() {
                self.set_phase(BattlePhase::ForcedSwitch);
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: self.replacement_requirements(),
                }));
            }
            return Ok(EngineStep::Progressed {
                phase: self.state.phase,
            });
        }

        let action = self.take_next_pending_action();
        if let Some(action) = action {
            self.execute_queued_action(action)?;
            self.process_drag_queue();
            // Showdown `runAction`: `clearActiveMove` after the drags, before the faints.
            self.state.clear_active_move();
            self.resolve_faints();
            if let Some(outcome) = self.state.outcome {
                self.pending = PendingExecution::Idle;
                return Ok(EngineStep::Finished(outcome));
            }
            // Gen 1-3: Showdown's `checkFainted` runs after every action followed by a move or
            // the residual (a switch still queued goes first): a Pokémon knocked out by the
            // action loses its status to the faint whether or not a replacement exists, and is
            // replaced right away.
            if self.rules.faint.replace_after_each_move && !self.next_pending_is_switch() {
                self.check_fainted();
            }
            if self.rules.faint.replace_after_each_move
                && self.fainted_slot_to_fill()
                && !self.next_pending_is_switch()
            {
                self.feed_status_berries_before_request();
                self.set_phase(BattlePhase::ForcedSwitch);
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: self.replacement_requirements(),
                }));
            }
            // The action's `Update` (Showdown `runAction`): Gen 5+ before any replacement
            // request; Gen 1-4 only when none is asked for — a Gen 1-3 replacement requested
            // above returned already, a mid-turn one below returns before it.
            if self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }
            // Mid-turn replacement (U-turn / Volt Switch; Gen 4 Healing Wish's slot): the turn is
            // suspended with the remaining actions still pending; `resolve_forced_switches`
            // resumes it. A leaving Pokémon a pursuer marked, or a Dynamaxed one, meets its
            // `BeforeSwitchOut` first, at the next step (§15.11, R-M9-21).
            if self.before_switch_out_due().is_some() {
                return Ok(EngineStep::Progressed {
                    phase: self.state.phase,
                });
            }
            // Asked before a Revival Blessing owed at the same time: Showdown asks both in one
            // request and runs the switch first (`instaswitch` before `revivalblessing`), then
            // the revive (`resolve_forced_switches` resumes the turn on it, R-M8-171).
            if self.mid_turn_replacement_needed() {
                self.feed_status_berries_before_request();
                self.set_phase(BattlePhase::ForcedSwitch);
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: self.replacement_requirements(),
                }));
            }
            // Revival Blessing: the side names the fainted Pokémon that comes back, the turn
            // suspended until `resolve_revive`.
            if let Some(request) = self.revive_request() {
                self.set_phase(BattlePhase::Revive);
                return Ok(EngineStep::Awaiting(request));
            }
            if !self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }
            return Ok(EngineStep::Progressed {
                phase: self.state.phase,
            });
        }

        if self.state.phase == BattlePhase::TurnResolution {
            if !matches!(
                self.pending,
                PendingExecution::Turn {
                    residual: false,
                    ..
                }
            ) {
                self.run_end_turn_residual();
                // The residual's drags (a Red Card a future attack's hit set off) — Showdown's
                // `runAction` drags in after the residual action as after any other, before the
                // faints (R-M8-141).
                self.process_drag_queue();
            }
            self.resolve_faints();
            // Revival Blessing's partner back into its slot (queued behind the residual).
            self.return_revived();
            self.pending = PendingExecution::Idle;

            // A battle decided in the residual ends without its `Update`.
            if let Some(outcome) = self.state.outcome {
                return Ok(EngineStep::Finished(outcome));
            }
            if self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }
            self.check_fainted();

            let requirements = self.replacement_requirements();
            if requirements
                .iter()
                .any(|requirement| requirement.required_count > 0)
            {
                self.feed_status_berries_before_request();
                self.set_phase(BattlePhase::ForcedSwitch);
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: requirements,
                }));
            }
            // No replacement asked for: the residual's `Update` (before Gen 5 it comes only now).
            if !self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }

            self.next_turn();
            // A rule at the turn's end may have ended the battle (DESIGN.md §45.7).
            if let Some(outcome) = self.state.outcome {
                return Ok(EngineStep::Finished(outcome));
            }
            self.set_phase(BattlePhase::TurnChoice);
            return Ok(EngineStep::Awaiting(BattleRequest::TurnChoices {
                sides: [SideId::P1, SideId::P2],
            }));
        }

        Ok(EngineStep::Progressed {
            phase: self.state.phase,
        })
    }

    fn execute_queued_action(&mut self, action: QueuedAction) -> Result<(), BattleError> {
        // A move cancelled while queued (Showdown `queue.cancelMove`: Gravity grounding a
        // flyer, `BattleContext::cancel_move`) is no longer pending: it does not happen. Its
        // gimmick's step — queued separately — still runs (Showdown cancels the move action
        // alone).
        // An injected action (Instruct) is the target's extra use, not the turn's own: it
        // neither checks nor consumes the turn's pending action (DESIGN.md §42.9).
        if action.step == ActionStep::Act && !action.instructed {
            let cancelled = matches!(action.choice, ActionChoice::Move { .. })
                && !self.state.pending_movers.contains(&action.actor);
            self.state
                .pending_movers
                .retain(|actor| *actor != action.actor);
            if cancelled {
                return Ok(());
            }
        }
        let actor_can_act = self
            .state
            .pokemon(action.actor)
            .is_some_and(|pokemon| !pokemon.is_fainted())
            && self
                .state
                .active_position(action.actor, &self.rules.topology)
                .is_some();
        if !actor_can_act {
            return Ok(());
        }

        // A gimmick's own step (DESIGN.md §44.2): it happens before every move of the turn,
        // whatever the two Pokémon's speeds, and the move action that follows carries no
        // mechanic.
        if action.step != ActionStep::Act {
            if let ActionChoice::Move {
                mechanic: Some(mechanic),
                ..
            } = action.choice
            {
                self.apply_mechanic(action.actor, mechanic)?;
            }
            return Ok(());
        }
        let _ = &action.step;
        match action.choice {
            ActionChoice::Move {
                slot,
                target,
                mechanic,
            } => {
                // Gen 1 `cannotmove` (§30.9): bound as the choices were made, the Pokémon's
                // action is lost — only its after-action residual runs (`abortMove`).
                if let Some(index) = self
                    .state
                    .cannot_move
                    .iter()
                    .position(|pokemon| *pokemon == action.actor)
                {
                    self.state.cannot_move.remove(index);
                    self.run_after_own_action(action.actor, true, false);
                    return Ok(());
                }
                // Gen 4 Custap Berry (`onCustap`): the holder eats it as its move begins, if it
                // still holds it.
                if let Some(index) = self
                    .state
                    .custap_ready
                    .iter()
                    .position(|pokemon| *pokemon == action.actor)
                {
                    self.state.custap_ready.remove(index);
                    if crate::engine::held_item(&self.state, &self.rules, action.actor)
                        .is_some_and(|(_, behavior)| behavior.pinch_priority)
                    {
                        let mut ctx = crate::engine::BattleContext::new(
                            &mut self.state,
                            &self.rules,
                            &mut self.trace,
                        );
                        if let Some(item) = ctx.eat_item(action.actor) {
                            ctx.item_activated(action.actor, item);
                        }
                    }
                }
                // A queue built without the split (a hand-built queue, the preview) still
                // carries its mechanic on the move action; a Z-Move was spent above, as it
                // actually ran.
                if let Some(mechanic) = mechanic
                    && mechanic != MechanicChoice::ZMove
                {
                    self.apply_mechanic(action.actor, mechanic)?;
                }
                // The Curse glitch (`BattleContext::curse_glitch`): the action aims at the ally's
                // slot, the first of the side.
                let target = match self
                    .state
                    .curse_glitched
                    .iter()
                    .position(|pokemon| *pokemon == action.actor)
                {
                    Some(index) => {
                        self.state.curse_glitched.remove(index);
                        self.state
                            .pokemon(action.actor)
                            .and_then(|pokemon| self.rules.topology.position(pokemon.side, 0))
                            .map(crate::rules::DeclaredTarget)
                            .or(target)
                    }
                    None => target,
                };
                // A Z-Move: the slot's move is what the crystal converts, and the Z power is
                // spent as the move is used — once BeforeMove and the PP let it through (Showdown
                // `runMove`, DESIGN.md §44.4, R-M9-29; `MovePipeline::spend_z_power`).
                let z_move = match mechanic {
                    Some(MechanicChoice::ZMove) => {
                        let base = self
                            .state
                            .pokemon(action.actor)
                            .and_then(|pokemon| pokemon.move_slot(slot))
                            .and_then(|slot_state| {
                                self.dex.move_by_id(slot_state.move_id).cloned()
                            });
                        base.and_then(|base| {
                            crate::engine::z_move_execution(
                                &self.state,
                                &self.rules,
                                &self.dex,
                                action.actor,
                                &base,
                            )
                        })
                    }
                    _ => None,
                };
                // A Dynamaxed Pokémon's move runs as its Max Move (Showdown `chooseMove` +
                // `getActiveMaxMove`, DESIGN.md §44.5): the type as it will be used, the base
                // move's category, priority and power.
                let max_move = {
                    let base = self
                        .state
                        .pokemon(action.actor)
                        .and_then(|pokemon| pokemon.move_slot(slot))
                        .and_then(|slot_state| self.dex.move_by_id(slot_state.move_id));
                    base.and_then(|base| {
                        let modified = crate::engine::modified_move_cow(
                            &self.state,
                            &self.rules,
                            action.actor,
                            base,
                            false,
                        )
                        .0;
                        crate::engine::max_move_execution(
                            &self.state,
                            &self.dex,
                            action.actor,
                            &modified,
                        )
                    })
                };
                let mut pipeline =
                    MovePipeline::new(&mut self.state, &self.rules, &self.dex, &mut self.trace)
                        .with_action_priority(
                            action.order.move_priority,
                            action.order.prankster_boosted,
                        )
                        .with_prioritized_by(action.prioritized_by)
                        .with_original_target(action.original_target);
                if let Some(z) = z_move {
                    pipeline = pipeline.with_z_move(z);
                } else if let Some(max) = max_move {
                    pipeline = pipeline.with_max_move(max);
                }
                let execution = pipeline.execute(action.actor, slot, target)?;
                // What the action's runs asked of the queue (DESIGN.md §48.3).
                let crate::engine::move_pipeline::ActionEffects {
                    promote_pledge,
                    prioritize_move,
                    prioritize_actors: armed,
                } = pipeline.take_effects();
                // Gen 4 (Showdown restores `activeMove = oldActiveMove` at the end of the
                // outermost `runMove`): the battle's last move — what Copycat reads — is the
                // outermost of a chain, not the move it called (DESIGN.md §42.2).
                let outermost = pipeline.outermost();
                drop(pipeline);
                // A Pledge that waited for its ally's: that action goes to the front with this
                // move as its `sourceEffect` (DESIGN.md §42.9).
                if let Some(ally_move) = promote_pledge {
                    self.prioritize_ally_move(execution.move_id, execution.actor, ally_move);
                }
                if let Some(move_id) = prioritize_move {
                    self.prioritize_same_move(move_id);
                }
                if self.rules.conditions.calls.copycat_reads_outermost
                    && let Some(outermost) = outermost
                    && let Some(active) = &mut self.state.active_move
                {
                    active.move_id = outermost;
                }
                for actor in armed {
                    self.prioritize_actor(actor);
                }
                for request in std::mem::take(&mut self.state.queue_requests) {
                    match request {
                        crate::state::QueueRequest::Prioritize(actor) => {
                            self.prioritize_actor(actor)
                        }
                        crate::state::QueueRequest::Quash(actor) => self.quash_actor(actor),
                        crate::state::QueueRequest::Instruct {
                            actor,
                            slot,
                            target_position,
                            ..
                        } => self.instruct_action(actor, slot, target_position),
                    }
                }
                // Gen 2: a Roar's drag is over by then (`SwitchingRules::drag_within_the_move`).
                if self.rules.conditions.switching.drag_within_the_move {
                    self.process_drag_queue();
                }
                // Gen 2: the user's own after-action residual (`onAfterMoveSelf`), skipped
                // after a self-switching move — the action's own (the gen2 `runMove`'s
                // `move.selfSwitch`): a Baton Pass Metronome called leaves it in (R-M11-44).
                let used = self.dex.move_by_id(execution.move_id);
                let self_switch = self
                    .dex
                    .move_by_id(outermost.unwrap_or(execution.move_id))
                    .is_some_and(|m| m.self_switch.is_some());
                let aimed_at_self =
                    used.is_some_and(|m| m.target == crate::rules::TargetPattern::SelfOnly);
                if execution.stopped || !self_switch {
                    self.run_after_own_action(action.actor, execution.stopped, aimed_at_self);
                }
            }
            ActionChoice::Switch { pokemon } => self.execute_switch(action.actor, pokemon)?,
            ActionChoice::Pass => {}
        }
        Ok(())
    }

    /// Showdown `checkFainted`, run once the turn's queue is empty and the battle goes on: a
    /// fainted Pokémon's status gives way to the faint (`fnt`). A battle decided mid-turn never
    /// reaches it, so the last Pokémon to faint keeps its status in the final state, as in
    /// Showdown.
    /// Showdown `endTurn`'s Dynamax sweep (DESIGN.md §44.5): every Pokémon whose counter reached
    /// 3 ends its Dynamax, fastest first (its own `speedSort`).
    fn expire_dynamax(&mut self) {
        let mut ending: Vec<(PokemonId, u32)> = self
            .active_pokemon()
            .into_iter()
            .filter(|pokemon| {
                self.state
                    .pokemon(*pokemon)
                    .is_some_and(|pokemon| pokemon.volatiles.dynamax_turns() == Some(3))
            })
            .map(|pokemon| {
                (
                    pokemon,
                    crate::engine::stats::action_speed(&self.state, &self.rules, pokemon, None),
                )
            })
            .collect();
        ending.sort_by_key(|(_, speed)| std::cmp::Reverse(*speed));
        for (pokemon, _) in ending {
            self.end_dynamax(pokemon);
        }
    }

    pub(super) fn check_fainted(&mut self) {
        let fainted: Vec<(PokemonId, crate::state::NonVolatileStatus)> = self
            .state
            .all_pokemon()
            .iter()
            .filter(|p| p.is_fainted())
            .filter_map(|p| p.status.map(|status| (p.id(), status.kind)))
            .collect();
        for (pokemon, status) in fainted {
            if let Some(state) = self.state.pokemon_mut(pokemon) {
                state.status = None;
            }
            self.trace.push(TraceEvent::StatusCured {
                target: pokemon,
                status,
                reason: crate::engine::CureReason::Fainted,
            });
        }
    }

    /// The turn boundary (Showdown `endTurn`): every Pokémon on the field starts the coming
    /// turn one turn older (`activeTurns++`) and its event history ages (DESIGN.md §22.8);
    /// each side's faints of the turn become last turn's; the turn's last successful move is
    /// forgotten. Then the rules that end a battle at a turn's end (`end_turn_rules`).
    pub(super) fn next_turn(&mut self) {
        // Showdown `endTurn`'s Dynamax sweep: once the turn is over — after the residual's
        // `Update` and after the replacements a faint asked for (a Pokémon Dynamaxed on its
        // third turn is still so while they come in).
        self.expire_dynamax();
        self.clean_up_binding();
        let active = self.active_pokemon();
        for &pokemon_id in &active {
            if let Some(pokemon) = self.state.pokemon_mut(pokemon_id) {
                pokemon.active_turns = pokemon.active_turns.saturating_add(1);
                pokemon.history.next_turn(&active);
            }
        }
        for side in [SideId::P1, SideId::P2] {
            let side = self.state.side_mut(side);
            side.fainted_last_turn = std::mem::take(&mut side.fainted_this_turn);
        }
        self.state.last_successful_move = None;
        self.state.skip_before_switch_out.clear();
        self.state.turn = self.state.turn.saturating_add(1);
        // Showdown `endTurn` → `DisableMove`: `choicelock.onDisableMove` drops a lock onto a move
        // the holder does not know (a lock onto Struggle), or held by a Pokémon that no longer
        // holds a Choice item (`getItem().isChoice`, set aside or not: Knock Off, Thief). Gen 4
        // drops it while the item is set aside as well (issues.md I55).
        let set_aside_ends_it = self.rules.conditions.items.choice_lock_ends_while_set_aside;
        for pokemon_id in self.active_pokemon() {
            let stale = self.state.pokemon(pokemon_id).is_some_and(|pokemon| {
                pokemon
                    .volatiles
                    .choice_locked_move()
                    .is_some_and(|locked| {
                        pokemon.move_slot_of(locked).is_none()
                            || !crate::engine::holds_choice_item(&self.rules, pokemon)
                            || (set_aside_ends_it
                                && crate::engine::held_item_of(&self.state, &self.rules, pokemon)
                                    .is_none())
                    })
            });
            if stale
                && let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
                && pokemon.volatiles.remove(VolatileConditionKind::ChoiceLock)
            {
                self.trace.push(TraceEvent::VolatileEnded {
                    target: pokemon_id,
                    kind: VolatileConditionKind::ChoiceLock,
                    seen: true,
                });
            }
        }
        self.end_turn_rules();
    }
}
