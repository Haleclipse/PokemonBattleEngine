//! `run_move`'s targeting (DESIGN.md §48.3): Showdown `useMoveInner` after the `move` line —
//! no target at all, `getMoveTargets` (a retaliation's target, the redirectors, Dragon Darts'
//! split), then the PP each Pressure the move meets takes.

use super::*;

impl MovePipeline<'_> {
    /// The move's targets as it is used: none at all fails it here; a retaliation goes where
    /// the attacker it answers stood; a redirector (Follow Me, Lightning Rod / Storm Drain)
    /// draws a single-target move not charging; Dragon Darts splits between a target and its
    /// partner. On: the targets, and whether Dragon Darts split.
    pub(super) fn move_targets(
        &mut self,
        run: &MoveRun,
    ) -> MoveStage<(crate::rules::ResolvedTargets, bool)> {
        let MoveRun {
            actor,
            move_id,
            ref move_def,
            declared_target,
            ref resolved,
            releasing,
            ..
        } = *run;
        // Showdown `useMoveInner`'s `if (!target)`, right after the `move|` line: a move handed
        // no target at all fails here, before the redirection, the PP Pressure takes and
        // TryMove (the Metronome item's count, R-M10-18); one whose target is gone fails after
        // TryMove (below).
        if self.aims_at_nothing(actor, move_def, declared_target, resolved.as_ref()) {
            return Ok(ControlFlow::Break(self.fail_no_target(actor, move_id)));
        }
        // Showdown `getMoveTargets`, after the `move|` line and before TryMove (§15.5; the M
        // family's redirection moved ahead, R218): the targets drawn before BeforeMove; in
        // doubles a single-target move can still be pulled elsewhere (Follow Me, Lightning Rod /
        // Storm Drain) below — even one left with no target at all (Showdown `getMoveTargets`
        // then aims at a fainted slot "for any possible redirection": a partner's Lightning Rod
        // takes it). The TryMove checks below (a charge, Damp, Shell Trap, a strong weather) do
        // not depend on them; the no-target failure comes after those.
        let mut targets = resolved.clone().unwrap_or(crate::rules::ResolvedTargets {
            targets: Vec::new(),
        });
        // Counter / Mirror Coat (their volatile's `onRedirectTarget`), Metal Burst / Comeuppance
        // (`onModifyTarget`): the move goes where the attacker it answers stood — whoever
        // stands there now (§15.8). With nobody standing there, Metal Burst keeps the target
        // drawn for it (`onModifyTarget` comes before `getMoveTargets`, which turns from a
        // fainted foe); Counter and Mirror Coat in doubles are redirected to the fainted slot
        // after that turn and have no target (`[notarget]`, R190) — unless a redirector
        // of higher priority (Follow Me) takes them first, below.
        if let Some(kind) = move_def.behavior.strikes_back_at
            && let Some((record, _)) =
                crate::engine::retaliation_record(self.state, self.rules, actor, kind)
            && let Some(position) = record.position
        {
            let answered = self
                .state
                .pokemon_at_position(position, &self.rules.topology)
                .filter(|answered| {
                    self.state
                        .pokemon(*answered)
                        .is_some_and(|p| !p.is_fainted())
                });
            match answered {
                Some(answered) => targets.targets = vec![ResolvedTarget::Pokemon(answered)],
                None if kind != crate::dex::Retaliation::MetalBurst
                    && self.rules.topology.active_slots_per_side() > 1 =>
                {
                    targets.targets.clear();
                }
                None => {}
            }
        }
        // A charging turn is not redirected (Showdown `getMoveTargets`'s `isCharging`): the
        // attack aims at its slot when it is released.
        // Sky Drop's carrying turn too, Power Herb or not (`isCharging`, §15.23).
        let charging = move_def.behavior.two_turn.is_some_and(|spec| {
            !releasing
                && crate::engine::two_turn::charge_skip(self.state, self.rules, actor, spec)
                    .is_none()
        }) || (move_def.behavior.sky_drop && !releasing);
        let held_by_redirector =
            !charging && self.redirect_target(actor, move_def, declared_target, &mut targets);
        // Dragon Darts (`smartTarget`) in doubles: the target and the partner beside it
        // (Showdown `getSmartTargets`, after the redirection) — both darts on the center of
        // attention, or the Pokémon whose ability drew the move (R-M8-169).
        let mut smart = false;
        if move_def.smart_target
            && !held_by_redirector
            && let [ResolvedTarget::Pokemon(target)] = targets.targets.as_slice()
        {
            let split = crate::engine::smart_targets(self.state, self.rules, actor, *target);
            smart = split.len() > 1;
            targets.targets = split.into_iter().map(ResolvedTarget::Pokemon).collect();
        }

        self.trace
            .push_with(crate::engine::TraceCategory::Calculation, || {
                TraceEvent::TargetsResolved {
                    actor,
                    move_id,
                    targets: targets.targets.clone(),
                }
            });
        Ok(ControlFlow::Continue((targets, smart)))
    }

    /// The PP each Pressure the move meets takes, at once, before TryMove (`continuing_lock`:
    /// a hard lock's later turn pays none).
    pub(super) fn pressure_pp(
        &mut self,
        run: &MoveRun,
        continuing_lock: bool,
        targets: &crate::rules::ResolvedTargets,
    ) {
        let MoveRun {
            actor,
            slot,
            move_id,
            ref move_def,
            struggling,
            continuing_bind,
            ..
        } = *run;
        // Showdown `useMoveInner` (§15.15): each Pressure the move meets costs one more PP, taken
        // at once (`deductPP`) before TryMove — a charging turn too; not a locked turn (a
        // two-turn move's attack and a hard lock's later turns, `sourceEffect` lockedmove: the
        // hard lock covers both).
        // A move another move called charges it to the caller (Gen 4+, `callerMoveForPressure`:
        // the caller's `sourceEffect.pp`) or to the action's move (Gen 3's cartridge, but not
        // after Sleep Talk); a reflection or a snatch — whose source is a condition, not a move —
        // charges nothing.
        // A Max Move or a Z-Move charges the slot's move it was made from (Showdown deducts
        // `moveOrMoveName`, the base move `runMove` hands `useMove`, R-M8-159).
        // An action another move moved to the front (Round's, a combined Pledge's) charges the
        // move it uses: the Pokémon that strikes pays for its own Pledge — the user's decision
        // (issues.md I201, R-M8-170), where Showdown charges the ally's Pledge that moved it up
        // (its `sourceEffect`), which the oracle patches.
        let charged = match self.call {
            None if move_def.is_max || move_def.z_crystal.is_some() => Some(
                self.state
                    .pokemon(actor)
                    .and_then(|pokemon| pokemon.move_slot(slot))
                    .map_or(move_id, |move_slot| move_slot.move_id),
            ),
            None => Some(move_id),
            Some(frame) => match frame.source {
                crate::state::CallSource::Move(caller)
                    if self.dex.move_by_id(caller).is_some_and(|m| m.calls_move) =>
                {
                    match self.rules.conditions.pp.called_move_pressure {
                        crate::rules::CalledMovePressure::TheCaller => Some(caller),
                        crate::rules::CalledMovePressure::TheActionsMove { spared_after } => {
                            frame.action_move.filter(|_| caller != spared_after)
                        }
                    }
                }
                _ => None,
            },
        };
        if let Some(charged) = charged
            && !continuing_lock
            && !continuing_bind
            && !struggling
        {
            let holders = crate::engine::context::pressure_holders(
                self.state, self.rules, actor, move_def, targets,
            );
            if let Some(first) = holders.first() {
                BattleContext::new(self.state, self.rules, self.trace).deduct_pp(
                    actor,
                    charged,
                    holders.len() as u8,
                    EffectSource::Ability {
                        pokemon: *first,
                        ability: crate::ids::AbilityId::PRESSURE,
                    },
                );
            }
        }
    }
}
