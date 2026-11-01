//! Uproar in the Move Shell (Showdown `uproar`, DESIGN.md §15.12): where it starts, who its hit
//! wakes, and what the turn's move does to it.

use crate::dex::ResolvedMove;
use crate::engine::{BattleContext, BeforeMoveBlockReason, HitBlockReason};
use crate::ids::{MoveId, PokemonId};
use crate::rules::UproarWake;
use crate::state::VolatileConditionKind;

use super::{HitOutcome, MoveExecution, MovePipeline};

impl<'a> MovePipeline<'a> {
    /// Showdown `selfDrops` on a hit (body or Substitute): Uproar starts.
    pub(super) fn uproar_self_drop(&mut self, actor: PokemonId, move_def: &ResolvedMove) {
        if move_def.behavior.uproar {
            BattleContext::new(self.state, self.rules, self.trace).start_uproar(actor);
        }
    }

    /// Gen 5+ Uproar's own `onTryHit`, as it reaches `target` past the hit steps (before a
    /// Substitute): every active Pokémon asleep wakes.
    pub(super) fn uproar_wakes_on_hit(&mut self, target: PokemonId, move_def: &ResolvedMove) {
        if move_def.behavior.uproar && self.rules.conditions.uproar.wake == UproarWake::OnHit {
            BattleContext::new(self.state, self.rules, self.trace).uproar_wakes_on_hit(target);
        }
    }

    /// A holder's move stopped by BeforeMove (Gen 3-4, the cartridge): Gen 3's flinch, love,
    /// Disable, Taunt end the uproar at once (`UproarRules::cancel_blocks`); the generation's
    /// failing blocks have the turn-end step end it (`failing_blocks`).
    pub(super) fn uproar_blocked(&mut self, actor: PokemonId, reason: BeforeMoveBlockReason) {
        let rules = self.rules.conditions.uproar;
        if rules.cancel_blocks.contains(&reason) {
            BattleContext::new(self.state, self.rules, self.trace).cancel_uproar(actor);
        } else if rules.failing_blocks.contains(&reason)
            && let Some(uproar) = self
                .state
                .pokemon_mut(actor)
                .and_then(|p| p.volatiles.uproar_mut())
        {
            uproar.failed = true;
        }
    }

    /// The holder's Uproar is over (Gen 3-4, the cartridge): a type immunity fails the turn;
    /// Gen 3's Protect ends the uproar at once.
    pub(super) fn uproar_after_move(&mut self, actor: PokemonId, execution: &MoveExecution) {
        let rules = self.rules.conditions.uproar;
        if execution.stopped || execution.move_id != MoveId::UPROAR {
            return;
        }
        if self
            .state
            .pokemon(actor)
            .is_none_or(|p| p.volatiles.uproar().is_none())
        {
            return;
        }
        if rules.protect_cancels && self.met_protect(execution) {
            BattleContext::new(self.state, self.rules, self.trace).cancel_uproar(actor);
        } else if rules.immunity_fails
            && (self.type_immune_target_guarded
                || execution.hits.iter().any(|hit| {
                    matches!(
                        hit,
                        HitOutcome::Blocked {
                            reason: HitBlockReason::TypeImmunity,
                            ..
                        }
                    )
                }))
            && let Some(uproar) = self
                .state
                .pokemon_mut(actor)
                .and_then(|p| p.volatiles.uproar_mut())
        {
            uproar.failed = true;
        }
    }

    /// Whether a target of the move holds Protect (Gen 3's check is the target's Protect itself,
    /// ahead of the hit test: a move that missed a protected target met it too).
    pub(super) fn met_protect(&self, execution: &MoveExecution) -> bool {
        execution.hits.iter().any(|hit| {
            let target = match hit {
                HitOutcome::Missed { target }
                | HitOutcome::Blocked { target, .. }
                | HitOutcome::Damaged { target, .. }
                | HitOutcome::Effect { target }
                | HitOutcome::SubstituteDamaged { target, .. }
                | HitOutcome::NoEffect { target }
                | HitOutcome::Reflected { target } => *target,
            };
            self.state
                .pokemon(target)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Protect))
        })
    }

    /// Gen 3 (pokeemerald `BattleScript_PrintMoveMissed`'s `ppreduce`, issues.md I65): a locked
    /// turn of Thrash or Uproar that misses or meets Protect spends a PP — the scripts only skip
    /// `ppreduce` on the way to a hit.
    pub(super) fn locked_miss_spends_pp(&mut self, actor: PokemonId, execution: &MoveExecution) {
        let move_def = self.dex.move_by_id(execution.move_id);
        let spends = move_def.is_some_and(|m| {
            (m.behavior.rampage && self.rules.conditions.rampage.miss_spends_pp)
                || (m.behavior.uproar && self.rules.conditions.uproar.miss_spends_pp)
        });
        if !spends || !self.continuing_lock || execution.stopped {
            return;
        }
        let missed = execution
            .hits
            .iter()
            .any(|hit| matches!(hit, HitOutcome::Missed { .. }));
        if !missed && !self.met_protect(execution) {
            return;
        }
        let Some(remaining) = self.state.pokemon_mut(actor).and_then(|p| {
            let slot = p
                .move_slots_mut()
                .iter_mut()
                .find(|slot| slot.move_id == execution.move_id)?;
            slot.pp = slot.pp.checked_sub(1)?;
            Some(slot.pp)
        }) else {
            return;
        };
        self.trace.push(crate::engine::TraceEvent::PpSpent {
            actor,
            move_id: execution.move_id,
            remaining,
        });
    }

    /// Gen 2-4 (the cartridges, §15.18): a use of Fury Cutter that landed on nothing — a miss,
    /// Protect, an immunity — ends its count (a BeforeMove block has its own rule).
    pub(super) fn fury_cutter_after_move(&mut self, actor: PokemonId, execution: &MoveExecution) {
        if execution.move_id != MoveId::FURY_CUTTER
            || !matches!(
                self.rules.conditions.fury_cutter.model,
                crate::rules::FuryCutterModel::Count { .. }
            )
            || execution.aborted
        {
            return;
        }
        let landed = execution.hits.iter().any(|hit| {
            matches!(
                hit,
                HitOutcome::Damaged { .. } | HitOutcome::SubstituteDamaged { .. }
            )
        });
        if !landed {
            BattleContext::new(self.state, self.rules, self.trace).end_fury_cutter(actor);
        }
    }

    /// Rollout / Ice Ball's `onAfterMove` in Gen 7 (the Rollout storage glitch, issues.md
    /// I212): a chain that ran its five hits short of doublings — Disguise took some — stores
    /// the rest; a use on the storage that computed no power (it failed: a miss, Protect, an
    /// immunity) clears it.
    pub(super) fn rolling_after_move(&mut self, actor: PokemonId, execution: &MoveExecution) {
        if execution.stopped
            || !self
                .dex
                .move_by_id(execution.move_id)
                .is_some_and(|move_def| move_def.behavior.rolling)
        {
            return;
        }
        let Some(pokemon) = self.state.pokemon_mut(actor) else {
            return;
        };
        if let Some((_, hits, _)) = pokemon.volatiles.rolling()
            && hits == crate::effects::moves::ROLLING_MAX_HITS
            && let Some(doublings) = pokemon.volatiles.rolling_doublings()
            && doublings < hits
        {
            pokemon.volatiles.store_rollout(doublings);
            self.trace.push(crate::engine::TraceEvent::VolatileStarted {
                target: actor,
                kind: VolatileConditionKind::RolloutStorage,
            });
        } else if pokemon
            .volatiles
            .rollout_storage()
            .is_some_and(|(_, armed)| armed)
        {
            BattleContext::new(self.state, self.rules, self.trace)
                .end_volatile(actor, VolatileConditionKind::RolloutStorage);
        }
    }
}
