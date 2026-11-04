//! `run_move`'s second stage (DESIGN.md §48.3): the use paid and recorded — Showdown
//! `runMove`'s `deductPP`, the `move` line, `moveUsed`.

use super::*;

impl MovePipeline<'_> {
    /// The use paid and recorded: the PP (none for a call, a lock's later turn, a two-turn
    /// move's attack), the `move` line, the last move, the Choice and Gorilla Tactics locks, a
    /// Rollout chain started. Gen 1's Desync Clause Mod ends the move here. On: whether a hard
    /// lock held the user to the move (no PP paid).
    pub(super) fn commit_use(&mut self, run: &MoveRun) -> MoveStage<bool> {
        let MoveRun {
            actor,
            slot,
            move_id,
            ref move_def,
            declared_target,
            action_target,
            ref resolved,
            struggling,
            continuing_bind,
            releasing,
            ..
        } = *run;
        // Showdown `runMove`: no `deductPP` while a hard lock holds the user to the move
        // (Rollout's later turns, §15.12) — `LockMove` asks the lock, not the move, so an Encore
        // that turned the locked action into its own move pays nothing either (R-M8-137).
        let continuing_lock = self.state.pokemon(actor).is_some_and(|p| {
            p.hard_lock().is_some_and(|lock| {
                (lock.move_id == move_id || p.volatiles.encored_move() == Some(move_id))
                    // Gen 1's charged attack pays (below).
                    && !(self.rules.two_turn.gen1() && lock.by == VolatileConditionKind::TwoTurnMove)
            })
        });
        self.continuing_lock = continuing_lock;
        // The charging turn pays (the attack runs locked, without `deductPP`) — Gen 1 the other
        // way round (`ChargeModel::Gen1`: pokered decrements the PP as the charged move goes
        // out, `PlayerCanExecuteChargingMove`).
        let gen1_charge = self.rules.two_turn.gen1();
        let charging_now = !releasing
            && crate::engine::two_turn::will_charge(self.state, self.rules, actor, move_def);
        // A called move spends no PP (Showdown `useMove` has no `deductPP`; the caller paid).
        let pays_pp = self.call.is_none()
            && if gen1_charge {
                !charging_now
            } else {
                !releasing
            };
        // Gen 1: a charging move a caller (Metronome, Mirror Move) called gives the caller's PP
        // back (the gen1 `useMove`'s `deductPP(-1)` on `lastSelectedMoveSlot`); the attack pays
        // it as it goes out, like a charging move picked from the menu (DESIGN.md R-M8-54).
        if gen1_charge && self.call.is_some() && charging_now {
            let side = self.state.pokemon(actor).map(|p| p.side);
            let last = side.map(|side| self.state.side(side).last_selected_move_slot);
            if let (Some(index), Some(pokemon)) = (last, self.state.pokemon_mut(actor)) {
                if let Some(caller) = pokemon.move_slots_mut().get_mut(index) {
                    caller.pp = (caller.pp + 1) % 64;
                }
                if self.rules.conditions.calls.mimic_keeps_the_slot_pp {
                    pokemon.sync_mimicked_pp(index);
                }
            }
        }
        // Gen 1's Transform + Mirror Move / Metronome PP error (the gen1 `useMove`, next to the
        // refund above): a transformed user's own last-picked slot gains one PP.
        if self
            .rules
            .conditions
            .calls
            .transformed_caller_restores_own_pp
            && let Some(frame) = self.call
            && matches!(
                frame.source,
                crate::state::CallSource::Move(MoveId::METRONOME | MoveId::MIRROR_MOVE)
            )
        {
            let side = self.state.pokemon(actor).map(|p| p.side);
            let last = side.map(|side| self.state.side(side).last_selected_move_slot);
            if let (Some(index), Some(pokemon)) = (last, self.state.pokemon_mut(actor))
                && pokemon.transform.is_some()
                && let Some(own) = pokemon.base_slot_mut(index)
            {
                own.pp = (own.pp + 1) % 64;
            }
        }
        // The slot the PP comes off (`PpRules::moves_slot`): the action's, Gen 1's last pick from
        // the menu, or Gen 4+'s first slot holding the action's move (issues.md I170).
        let pp_slot = match self.rules.conditions.pp.moves_slot {
            MovesSlot::TheLastSelected => self
                .state
                .pokemon(actor)
                .map(|p| self.state.side(p.side).last_selected_move_slot)
                .and_then(|index| MoveSlot::new(index as u8))
                .unwrap_or(slot),
            MovesSlot::TheChosen => slot,
            MovesSlot::TheFirstHoldingIt => self
                .state
                .pokemon(actor)
                .and_then(|p| {
                    let id = p.move_slots().get(slot.index())?.move_id;
                    p.move_slots().iter().position(|other| other.move_id == id)
                })
                .and_then(|index| MoveSlot::new(index as u8))
                .unwrap_or(slot),
        };
        let remaining_pp = {
            let pokemon = self
                .state
                .pokemon_mut(actor)
                .ok_or(BattleError::UnknownPokemon(actor))?;
            let move_slot = pokemon
                .move_slot_mut(pp_slot)
                .ok_or(BattleError::InvalidMoveSlot { actor, slot })?;
            if !pays_pp || struggling || continuing_bind || continuing_lock {
                None
            } else {
                if move_slot.pp == 0 {
                    return Err(BattleError::NoPp { actor, move_id });
                }
                // Showdown `deductPP`: the slot counts as used (Last Resort).
                move_slot.used = true;
                move_slot.pp -= 1;
                let remaining = move_slot.pp;
                if self.rules.conditions.calls.mimic_keeps_the_slot_pp {
                    pokemon.sync_mimicked_pp(pp_slot.index());
                }
                Some(remaining)
            }
        };

        // Gen 1's Desync Clause Mod (the gen1 `runMove`, after the PP): an action chosen as
        // `Fight` (`BattleState::fight_actors`) is the side's last selected move; when the slot
        // that paid holds another (a frozen Pokémon that came in after a teammate's pick, the
        // slot back at the first, thawed by a Fire move), the move ends here — no `move` line,
        // the after-move residual as for a stopped action (R-M8-150). Any other action is
        // stopped the same way when nothing locks it any more (`!lockedMove`: no hard lock, no
        // charge's release, no Bide, no binding still on the Pokémon it caught) and its move is
        // not the paying slot's — a binding move Mirror Move or Metronome called, whose target
        // switched out, ends its lock at BeforeMove (`partialtrappinglock`'s `onBeforeMove`)
        // with the action still on that move (R-M9-47).
        let unlocked = !(struggling || continuing_bind || continuing_lock || releasing)
            && self
                .state
                .pokemon(actor)
                .is_none_or(|pokemon| pokemon.semi_locked_move() != Some(move_id));
        if self.rules.conditions.pp.desync_clause
            && self.call.is_none()
            && unlocked
            && let Some(pokemon) = self.state.pokemon(actor)
            && let Some(paying) = pokemon.move_slot(pp_slot).map(|slot| slot.move_id)
            && if self.state.fight_actors.contains(&actor) {
                matches!(
                    self.state.side(pokemon.side).last_selected_move,
                    Some(crate::state::SelectedMove::Move(selected)) if paying != selected
                )
            } else {
                paying != move_id
            }
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::Desync,
            });
            return Ok(ControlFlow::Break(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: true,
                aborted: false,
                result: MoveResult::Failed,
            }));
        }
        // A use of its own (DESIGN.md §47.0 item 13): no call, nor a lock's later turn (the
        // `[from] lockedmove` lines).
        let own_move = (self.call.is_none()
            && !releasing
            && !continuing_lock
            && !continuing_bind
            && !struggling)
            .then(|| self.shown_own_move(move_id))
            .flatten();
        self.trace.push(TraceEvent::MoveUsed {
            actor,
            move_id,
            declared_target,
            called_by: self.call.map(|frame| frame.source),
            own_move,
        });
        if let Some(z) = self.z_power.take() {
            self.spend_z_power(actor, &z);
        }
        // The move is decided: it becomes the last move (Encore / Disable lock onto it) and
        // a Choice item locks the holder onto it (Showdown `onModifyMove`).
        // Gen 1's charging turn is not the user's last move (the gen1 `useMove`).
        let leaves_no_last_move = self
            .rules
            .conditions
            .moves_leaving_no_last_move
            .contains(&move_id);
        // A called move is nobody's last move after Gen 1 (`useMove` sets `lastMoveUsed` only,
        // §42.2): Encore and Disable keep reading the move the action chose. Gen 2's `moveUsed`
        // clears the last move outright when the chosen move is one of the callers
        // (`ConditionRules::moves_leaving_no_last_move`).
        let writes_last_move =
            self.call.is_none() || self.rules.conditions.calls.called_sets_last_move;
        // Showdown `lastMoveTargetLoc` (Instruct repeats the move where it aimed, §42.9): the
        // action's own move writes it (`moveUsed`; a called one does not), none — `0` — when no
        // position was chosen (an Acupressure in singles aims at its user again, R-M8-111).
        if self.call.is_none()
            && let Some(pokemon) = self.state.pokemon_mut(actor)
        {
            pokemon.last_move_target = declared_target.map(|target| target.0);
        }
        if let Some(pokemon) = self.state.pokemon_mut(actor)
            && !(gen1_charge && charging_now)
        {
            if leaves_no_last_move && self.call.is_none() {
                pokemon.last_move = None;
                pokemon.last_move_encore = None;
                pokemon.last_move_base = None;
            } else if writes_last_move {
                pokemon.last_move = Some(move_id);
                pokemon.last_move_pattern = Some((move_id, move_def.target));
                pokemon.last_move_encore = Some(move_id);
                // A Max Move's `baseMove` (DESIGN.md §44.5): the move it was made from.
                pokemon.last_move_base = self.max_base;
                // `lastMove.isZ`: this use was a Z-Move (a status one keeps its own id).
                pokemon.last_move_z = self.z_powered && self.max_base.is_none();
            }
            // `lastMoveUsed` is the move object `ModifyType` edits: the type as used.
            pokemon.last_move_used = Some((move_id, move_def.move_type));
        }
        // Struggle too (Showdown `choicelock.onStart` takes any move with an id): the lock onto
        // a move the holder does not know keeps nothing off the request and is dropped as the
        // next turn starts (`next_turn`). Not a move out of a reflection (`hasBounced`: a Magic
        // Bounce holder's first move of the turn is still its own to choose) nor a snatched one
        // (`sourceEffect === 'snatch'`).
        let snatched = matches!(
            self.call.map(|frame| frame.source),
            Some(crate::state::CallSource::Move(MoveId::SNATCH))
        );
        if !self.rules.conditions.items.choice_locks_after_the_move
            && !self.bounced
            && !snatched
            && let Some((_, item)) = crate::engine::held_item(self.state, self.rules, actor)
            && let Some(handler) = item.move_decided
        {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            handler(&mut ctx, actor, move_id);
        }
        // Gorilla Tactics (`onModifyMove`): the first move since coming in, Struggle aside — a
        // Z-powered move neither takes the lock nor answers it (DESIGN.md §44.4).
        let gorilla = self.state.pokemon(actor).is_some_and(|pokemon| {
            crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.choice_lock)
        });
        if !struggling
            && !self.z_powered
            && gorilla
            && let Some(pokemon) = self.state.pokemon_mut(actor)
            && pokemon.ability_state.choice_lock.is_none()
        {
            pokemon.ability_state.choice_lock = Some(move_id);
        }
        if let Some(remaining) = remaining_pp {
            self.trace.push(TraceEvent::PpSpent {
                actor,
                move_id,
                remaining,
            });
        }
        self.metronome_starts(actor, move_id, releasing);
        // Rollout / Ice Ball's `onModifyMove`: a first use with a target starts the chain, aimed
        // at the action's position — the declared one even when an Encore turned the action into
        // this move (`lastMoveTargetLoc`, as a two-turn move's charge, R-M8-135); not while asleep
        // (§15.12). A called one (`move.sourceEffect`: Metronome's) aims at where its target
        // stands (`lastMoveTargetLoc = getLocOf(target)`, R-M8-73).
        let first_target = resolved
            .as_ref()
            .and_then(|targets| targets.pokemon_ids().next());
        let chain_target = match self.call {
            Some(_) => first_target
                .and_then(|target| self.state.active_position(target, &self.rules.topology)),
            None => action_target.or(declared_target).map(|target| target.0),
        };
        // On a Gen 7 Rollout storage (issues.md I212) the move starts no chain — its user is not
        // locked in — and a use that fails clears the storage (`rolling_after_move`).
        if move_def.behavior.rolling
            && first_target.is_some()
            && let Some(pokemon) = self.state.pokemon_mut(actor)
            && !pokemon.has_status(NonVolatileStatus::Sleep)
        {
            if pokemon.volatiles.rollout_storage().is_some() {
                pokemon.volatiles.arm_rollout_storage();
            } else if pokemon.volatiles.start_rolling(move_id, chain_target) {
                self.trace.push(TraceEvent::VolatileStarted {
                    target: actor,
                    kind: VolatileConditionKind::Rolling,
                });
            }
        }
        Ok(ControlFlow::Continue(continuing_lock))
    }
}
