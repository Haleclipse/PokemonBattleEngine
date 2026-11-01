//! What a side keeps on its slots across a faint (DESIGN.md §30.5): Revival Blessing's Revive
//! request, Healing Wish and Lunar Dance; and what Baton Pass and Shed Tail hand on to the
//! Pokémon taking the slot (§30.7).

use crate::ids::{PokemonId, SideId};
use crate::state::{BattlePhase, PassKind, StatStages, VolatileCondition, VolatileConditionKind};

/// What a Pokémon leaving by Baton Pass or Shed Tail hands on to its replacement.
pub(super) struct Handover {
    stages: Option<StatStages>,
    volatiles: Vec<VolatileCondition>,
    /// The kinds it lets through, had the Pokémon them.
    lets_through: crate::state::VolatileSet,
}

use super::{BattleEngine, TurnResult};
use crate::engine::{
    BattleContext, BattleError, BattleRequest, ChoiceError, ReviveChoice, TraceEvent,
};

impl BattleEngine {
    /// The Revive request a side owes after Revival Blessing (its fainted Pokémon, in party
    /// order), if any; a side with none left to revive drops the request.
    pub(super) fn revive_request(&mut self) -> Option<BattleRequest> {
        for side in [SideId::P1, SideId::P2] {
            if self.state.side(side).reviving.is_none() {
                continue;
            }
            let candidates = self.revive_candidates(side);
            if candidates.is_empty() {
                self.state.side_mut(side).reviving = None;
                continue;
            }
            return Some(BattleRequest::Revive { side, candidates });
        }
        None
    }

    fn revive_candidates(&self, side: SideId) -> Vec<PokemonId> {
        self.state
            .side(side)
            .battle_order
            .iter()
            .copied()
            .filter(|pokemon| self.state.pokemon(*pokemon).is_some_and(|p| p.is_fainted()))
            .collect()
    }

    /// The side's answer to `BattleRequest::Revive` (Showdown's `revivalblessing` action): the
    /// Pokémon comes back with half its max HP (at least 1) and no status. One still holding an
    /// active slot's party index — a partner knocked out earlier this turn, not yet replaced —
    /// goes back into its slot once the turn's residual is over (`SideState::returning`,
    /// Showdown's `instaswitch` queued behind the residual); any other stays on the bench. The
    /// turn then goes on.
    pub fn resolve_revive(&mut self, choice: ReviveChoice) -> Result<TurnResult, BattleError> {
        let result = self.resolve_revive_inner(choice);
        if result.is_ok() {
            self.handed_back("resolve_revive");
        }
        result
    }

    fn resolve_revive_inner(&mut self, choice: ReviveChoice) -> Result<TurnResult, BattleError> {
        self.ensure_phase(BattlePhase::Revive)?;
        let Some(BattleRequest::Revive { side, candidates }) = self.revive_request() else {
            return Err(BattleError::WrongPhase {
                expected: BattlePhase::Revive,
                actual: self.state.phase,
            });
        };
        if choice.side != side {
            return Err(ChoiceError::WrongSide {
                expected: side,
                actual: choice.side,
            }
            .into());
        }
        if !candidates.contains(&choice.pokemon) {
            return Err(ChoiceError::NotRevivable(choice.pokemon).into());
        }
        self.state.side_mut(side).reviving = None;
        if let Some(pokemon) = self.state.pokemon_mut(choice.pokemon) {
            pokemon.hp = (pokemon.max_hp / 2).max(1);
            pokemon.status = None;
            let hp = pokemon.hp;
            self.trace.push(TraceEvent::Revived {
                pokemon: choice.pokemon,
                hp,
            });
        }
        let slots = usize::from(self.rules.topology.active_slots_per_side());
        let index = self
            .state
            .side(side)
            .battle_order
            .iter()
            .position(|p| *p == choice.pokemon);
        if index.is_some_and(|slot| slot < slots && self.state.side(side).active[slot].is_none()) {
            self.state.side_mut(side).returning = Some(choice.pokemon);
            // Showdown queues its `instaswitch` behind the rest of the turn; the Gen 8+ re-sort
            // after this action puts it ahead of a move that comes next.
            if self.resorts_before_next() {
                self.return_revived();
            }
        }
        self.set_phase(if self.state.outcome.is_some() {
            BattlePhase::Finished
        } else {
            BattlePhase::TurnResolution
        });
        Ok(self.turn_result())
    }

    /// Showdown `copyVolatileFrom`, taken before the ordered-out Pokémon is cleaned up: Baton
    /// Pass hands on the stat stages and the volatiles the generation lets through
    /// (`SwitchingRules::not_passed`), Shed Tail its Substitute alone.
    ///
    /// Gen 2-4 hand Mean Look's links on too (`trapped` / `trapper` without `noCopy`): they move
    /// to `incoming` — those the outgoing Pokémon trapped are now trapped by it — so the
    /// outgoing Pokémon's cleanup releases nothing.
    pub(super) fn take_handover(
        &mut self,
        side: SideId,
        outgoing: PokemonId,
        incoming: PokemonId,
    ) -> Option<Handover> {
        let kind = self.state.side_mut(side).passing.take()?;
        let pokemon = self.state.pokemon(outgoing)?;
        let not_passed = self.rules.conditions.switching.not_passed;
        let handover = match kind {
            PassKind::BatonPass => Handover {
                stages: Some(pokemon.stages),
                volatiles: pokemon
                    .volatiles
                    .passed_on(|volatile| !not_passed.contains(&volatile)),
                lets_through: {
                    let mut kinds = crate::state::VolatileSet::ALL;
                    for kind in not_passed {
                        kinds.remove(*kind);
                    }
                    kinds
                },
            },
            PassKind::ShedTail => Handover {
                stages: None,
                volatiles: pokemon
                    .volatiles
                    .passed_on(|volatile| volatile == VolatileConditionKind::Substitute),
                lets_through: [VolatileConditionKind::Substitute].into_iter().collect(),
            },
        };
        // Gen 3-4 on the cartridge (issues.md I68): the user's lock goes on with its 2 turns
        // afresh, and a lock on the outgoing Pokémon follows it to the replacement.
        let mut handover = handover;
        if kind == PassKind::BatonPass && self.rules.conditions.lock_on.baton_pass_follows {
            for volatile in &mut handover.volatiles {
                if let VolatileCondition::LockOn { turns, .. } = volatile {
                    *turns = crate::engine::context::LOCK_ON_TURNS;
                }
            }
            let ids: Vec<PokemonId> = self.state.pokemon_ids().collect();
            for other in ids {
                if let Some(pokemon) = self.state.pokemon_mut(other) {
                    pokemon.volatiles.retarget_lock_on(outgoing, incoming, None);
                }
            }
        }
        let passes = |kind| handover.volatiles.iter().any(|v| v.kind() == kind);
        if passes(VolatileConditionKind::Trapper) {
            let ids: Vec<PokemonId> = self.state.pokemon_ids().collect();
            for other in ids {
                if let Some(pokemon) = self.state.pokemon_mut(other) {
                    pokemon.volatiles.retarget_trap(outgoing, incoming);
                }
            }
        }
        for link in [
            VolatileConditionKind::Trapped,
            VolatileConditionKind::Trapper,
        ] {
            if passes(link)
                && let Some(pokemon) = self.state.pokemon_mut(outgoing)
            {
                pokemon.volatiles.remove(link);
            }
        }
        Some(handover)
    }

    /// The replacement takes over what `from` handed on, before its switch-in runs (entry
    /// hazards, abilities).
    pub(super) fn hand_over(&mut self, from: PokemonId, to: PokemonId, handover: Handover) {
        let Some(pokemon) = self.state.pokemon_mut(to) else {
            return;
        };
        if let Some(stages) = handover.stages {
            pokemon.stages = stages;
        }
        pokemon.volatiles.take_over(&handover.volatiles);
        // Gastro Acid's `onCopy`: a replacement whose ability cannot be suppressed drops it.
        if self
            .rules
            .conditions
            .cant_suppress
            .contains(&crate::engine::raw_ability(pokemon))
        {
            pokemon.volatiles.remove(VolatileConditionKind::GastroAcid);
        }
        // Power Trick's `onCopy`: the replacement's own Attack and Defense trade places.
        if handover
            .volatiles
            .iter()
            .any(|v| v.kind() == VolatileConditionKind::PowerTrick)
        {
            let (attack, defense) = (
                pokemon.battle_stats().attack,
                pokemon.battle_stats().defense,
            );
            pokemon.set_stored_stat(crate::state::Stat::Attack, defense);
            pokemon.set_stored_stat(crate::state::Stat::Defense, attack);
        }
        self.trace.push(TraceEvent::PassedOn {
            from,
            to,
            stages: handover.stages.is_some(),
            volatiles: handover.volatiles.iter().map(|v| v.kind()).collect(),
            lets_through: handover.lets_through,
        });
    }

    /// The partners Revival Blessing brought back into their own slots come in: before the next
    /// move when one follows (Gen 8+'s re-sort), otherwise after the residual, before the
    /// end-of-turn replacements.
    pub(super) fn return_revived(&mut self) {
        for side in [SideId::P1, SideId::P2] {
            let Some(pokemon) = self.state.side_mut(side).returning.take() else {
                continue;
            };
            let slot = self
                .state
                .side(side)
                .battle_order
                .iter()
                .position(|p| *p == pokemon);
            let position = self
                .rules
                .topology
                .positions()
                .iter()
                .find(|position| position.side == side && Some(usize::from(position.slot)) == slot)
                .map(|position| position.id);
            if let Some(position) = position
                && let Some(slot) = slot
                && self.state.side(side).active[slot].is_none()
                && self.state.outcome.is_none()
                && self.place(position, None, pokemon).is_ok()
            {
                // Showdown's `switchIn` cancels the action of the Pokémon it replaces — here
                // the revived Pokémon itself, still in its slot: the move it chose this turn is
                // not used.
                BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                    .cancel_move(pokemon);
                self.run_switch_in(pokemon);
                self.resolve_faints();
            }
        }
    }

    /// Healing Wish / Lunar Dance on `pokemon`'s slot as it comes in (Showdown `onSwitchIn`:
    /// Gen 4 after the entry hazards, Gen 5+ before them — `SwitchInOrder`;
    /// `BattleContext::apply_slot_heals`).
    pub(super) fn apply_slot_heals(&mut self, pokemon: PokemonId) {
        BattleContext::new(&mut self.state, &self.rules, &mut self.trace).apply_slot_heals(pokemon);
    }
}
