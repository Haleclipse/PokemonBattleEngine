//! Forced replacements (DESIGN.md §30): what each side must send in, the choices checked and
//! resolved.

use crate::engine::{
    BattleError, ChoiceError, ForcedSwitchChoice, PendingExecution, PendingFaints,
};
use crate::ids::SideId;
use crate::state::BattlePhase;

use super::*;

impl BattleEngine {
    internal!(
        pub fn replacement_requirements(&self) -> [ReplacementRequirement; 2] {
            [
                self.replacement_requirement(SideId::P1),
                self.replacement_requirement(SideId::P2),
            ]
        }
    );

    pub(super) fn resolve_forced_switches_inner(
        &mut self,
        choices: [ForcedSwitchChoice; 2],
    ) -> Result<TurnResult, BattleError> {
        self.ensure_phase(BattlePhase::ForcedSwitch)?;
        let requirements = self.replacement_requirements();

        for (index, expected_side) in [SideId::P1, SideId::P2].into_iter().enumerate() {
            let choice = &choices[index];
            if choice.side != expected_side {
                return Err(ChoiceError::WrongSide {
                    expected: expected_side,
                    actual: choice.side,
                }
                .into());
            }
            self.validate_replacements(choice, &requirements[index])?;
        }

        let mut entrants = Vec::new();
        for side in [SideId::P1, SideId::P2] {
            self.state.side_mut(side).refill_mid_turn = None;
        }
        self.state.faint_refills_held = false;
        for (index, choice) in choices.into_iter().enumerate() {
            for replacement in choice.replacements {
                let slot = requirements[index]
                    .positions
                    .iter()
                    .find(|slot| slot.position == replacement.position)
                    .copied()
                    .ok_or(BattleError::InvalidReplacementPosition(
                        replacement.position,
                    ))?;
                // One placement path for both kinds of slot: an ordered-out Pokémon is cleaned
                // up and replaced, an empty slot is filled. Every entrant then runs its
                // switch-in below in one speed-ordered batch, as Showdown's runSwitch does.
                // Gen 4 (Showdown `switchIn` with the U-turn / Baton Pass as source effect): the
                // replacement takes over the user's last move.
                let passed_move = slot
                    .outgoing
                    .filter(|_| self.rules.conditions.self_switch_passes_last_move)
                    .and_then(|outgoing| self.state.pokemon(outgoing))
                    .and_then(|outgoing| outgoing.last_move);
                let mut handover = None;
                if let Some(outgoing) = slot.outgoing {
                    self.state.side_mut(choice.side).switch_out = None;
                    // Showdown `switchIn` → `queue.cancelAction`: a Pokémon Eject Button sent out
                    // before it acted loses its action, even if a drag brings it back this turn.
                    if let PendingExecution::Turn { actions, .. } = &mut self.pending {
                        actions.retain(|action| action.actor != outgoing);
                    }
                    self.run_switch_out(outgoing);
                    handover = self.take_handover(choice.side, outgoing, replacement.pokemon);
                    self.clear_on_switch(outgoing);
                }
                self.place(replacement.position, slot.outgoing, replacement.pokemon)?;
                if let (Some(outgoing), Some(handover)) = (slot.outgoing, handover) {
                    self.hand_over(outgoing, replacement.pokemon, handover);
                }
                if let Some(move_id) = passed_move
                    && let Some(incoming) = self.state.pokemon_mut(replacement.pokemon)
                {
                    incoming.last_move = Some(move_id);
                    incoming.last_move_encore = Some(move_id);
                    incoming.last_move_base = None;
                }
                entrants.push(replacement.pokemon);
            }
        }
        // The replacements' `switch` actions end with their `Update`.
        self.run_update(PendingFaints::Processed);

        // Simultaneous entries run their switch-in together, fastest first; only genuine speed
        // ties draw a SpeedTie decision (`run_switch_ins`: Gen 1-4 one after another, an
        // entrant the hazards knock out fainting before the next one enters; Gen 5+ one sorted
        // `SwitchIn` event). When a faint decides the battle nobody else enters.
        let entrants = self.speed_ordered(entrants);
        self.run_switch_ins(&entrants);
        self.resolve_faints();

        if self.state.outcome.is_none() {
            // The switch-ins' `runSwitch` action ends with its `Update`: Gen 5+ before any
            // replacement request, Gen 1-4 only when none follows (below).
            if self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }
            // Showdown's `runAction` ends the switch with `checkFainted` when nothing is queued
            // after it (an end-of-turn replacement) or, in Gen 1-3, when a move or the residual
            // comes next: a newcomer knocked out by the hazards loses its status to the faint.
            let mid_turn = matches!(&self.pending, PendingExecution::Turn { .. });
            if !mid_turn
                || (self.rules.faint.replace_after_each_move && !self.next_pending_is_switch())
            {
                self.check_fainted();
            }
            let requirements = self.replacement_requirements();
            if requirements
                .iter()
                .any(|requirement| requirement.required_count > 0)
            {
                self.feed_status_berries_before_request();
                self.set_phase(BattlePhase::ForcedSwitch);
            } else {
                if !self.rules.update.before_replacement {
                    self.run_update(PendingFaints::Processed);
                }
                // A turn suspended mid-way still owns its pending execution (even with no
                // action left, the end-of-turn residual has not run); a turn that ended with a
                // faint replacement has already released it.
                let resume_turn = matches!(&self.pending, PendingExecution::Turn { .. });
                if resume_turn {
                    // A Revival Blessing owed with the switch comes next (R-M8-171).
                    let phase = if self.revive_request().is_some() {
                        BattlePhase::Revive
                    } else {
                        BattlePhase::TurnResolution
                    };
                    self.set_phase(phase);
                } else if matches!(self.pending, PendingExecution::Start) {
                    // A lead ordered out as the battle started (Eject Pack): the start-of-battle
                    // `endTurn` comes now (`run_lead_switch_ins`), and the replacement starts the
                    // first turn as the leads do.
                    self.pending = PendingExecution::Idle;
                    self.state.turn = 1;
                    for pokemon in self.active_pokemon() {
                        if let Some(pokemon) = self.state.pokemon_mut(pokemon) {
                            pokemon.active_turns = 1;
                            pokemon.history.newly_switched = false;
                        }
                    }
                    self.set_phase(BattlePhase::TurnChoice);
                    self.end_turn_rules();
                } else {
                    self.next_turn();
                    if self.state.outcome.is_none() {
                        self.set_phase(BattlePhase::TurnChoice);
                    }
                }
            }
        }
        Ok(self.turn_result())
    }

    fn replacement_requirement(&self, side: SideId) -> ReplacementRequirement {
        let side_state = self.state.side(side);
        // A turn suspended mid-way (U-turn, Eject Button) asks only for the slots ordered out;
        // a slot emptied by a faint is filled once the turn is over (Showdown `checkFainted`
        // runs only when the action queue is empty), and after a residual Emergency Exit's
        // replacement (`BattleState::faint_refills_held`).
        let mid_turn = (matches!(self.pending, PendingExecution::Turn { .. })
            && !self.rules.faint.replace_after_each_move)
            || self.state.faint_refills_held;
        let positions = self
            .rules
            .topology
            .positions()
            .iter()
            .filter(|position| position.side == side)
            .filter_map(|position| match side_state.active[position.slot as usize] {
                // Gen 4 Healing Wish / Lunar Dance: the user's slot is filled at once.
                None if mid_turn && side_state.refill_mid_turn != Some(position.slot as usize) => {
                    None
                }
                None => Some(ReplacementSlot {
                    position: position.id,
                    outgoing: None,
                }),
                Some(occupant) if side_state.switch_out == Some(occupant) => {
                    Some(ReplacementSlot {
                        position: position.id,
                        outgoing: Some(occupant),
                    })
                }
                Some(_) => None,
            })
            .collect::<Vec<_>>();
        let available_pokemon = self.state.healthy_bench(side);
        let required_count = positions.len().min(available_pokemon.len());

        ReplacementRequirement {
            side,
            positions,
            available_pokemon,
            required_count,
        }
    }

    /// Some side has an empty slot and a healthy Pokémon to fill it.
    pub(super) fn fainted_slot_to_fill(&self) -> bool {
        [SideId::P1, SideId::P2].into_iter().any(|side| {
            let side_state = self.state.side(side);
            side_state.active.iter().any(Option::is_none)
                && !self.state.healthy_bench(side).is_empty()
        })
    }

    /// An ordered-out Pokémon with a bench to come from suspends the turn; without a bench the
    /// order lapses (Showdown: the switch flag is dropped when nothing can switch in).
    pub(super) fn mid_turn_replacement_needed(&mut self) -> bool {
        let mut needed = false;
        for side in [SideId::P1, SideId::P2] {
            if let Some(slot) = self.state.side(side).refill_mid_turn {
                if self
                    .state
                    .side(side)
                    .active
                    .get(slot)
                    .is_some_and(Option::is_none)
                    && !self.state.healthy_bench(side).is_empty()
                {
                    needed = true;
                } else {
                    self.state.side_mut(side).refill_mid_turn = None;
                }
            }
            if self.state.side(side).switch_out.is_none() {
                continue;
            }
            if self.state.healthy_bench(side).is_empty() {
                self.state.side_mut(side).switch_out = None;
            } else {
                needed = true;
            }
        }
        needed
    }

    fn validate_replacements(
        &self,
        choice: &ForcedSwitchChoice,
        requirement: &ReplacementRequirement,
    ) -> Result<(), BattleError> {
        if choice.replacements.len() != requirement.required_count {
            return Err(ChoiceError::WrongReplacementCount {
                expected: requirement.required_count,
                actual: choice.replacements.len(),
            }
            .into());
        }

        let mut positions = Vec::new();
        let mut pokemon = Vec::new();
        for replacement in &choice.replacements {
            if !requirement
                .positions
                .iter()
                .any(|slot| slot.position == replacement.position)
            {
                return Err(ChoiceError::InvalidReplacementPosition(replacement.position).into());
            }
            if positions.contains(&replacement.position) {
                return Err(ChoiceError::DuplicateReplacementPosition(replacement.position).into());
            }
            positions.push(replacement.position);

            if !requirement.available_pokemon.contains(&replacement.pokemon) {
                return Err(ChoiceError::SwitchTargetNotOnTeam(replacement.pokemon).into());
            }
            if pokemon.contains(&replacement.pokemon) {
                return Err(ChoiceError::DuplicateSwitchTarget(replacement.pokemon).into());
            }
            pokemon.push(replacement.pokemon);
        }

        Ok(())
    }
}
