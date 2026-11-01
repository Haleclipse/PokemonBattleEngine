//! Rage in the Move Shell (DESIGN.md §15.19): where its user's flag starts and ends with the
//! user's own moves, and where the hits a raging Pokémon takes build its rage.

use crate::dex::ResolvedMove;
use crate::engine::{BattleContext, HitBlockReason};
use crate::ids::{MoveId, PokemonId};
use crate::rules::{RageBuild, RageEnd};

use super::{HitOutcome, MoveExecution, MovePipeline};

/// What reached the raging Pokémon (`MovePipeline::rage_struck`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RageHit {
    /// The move's hit on its body (Showdown's `Hit` event).
    Body,
    /// The move's hit on its Substitute (Showdown's `AfterSubDamage`).
    Substitute,
    /// A heal in place of the hit (Present, Pollen Puff on an ally: the `Hit` event too).
    Heal,
}

impl<'a> MovePipeline<'a> {
    fn raging(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|p| !p.is_fainted() && p.volatiles.rage().is_some())
    }

    /// Showdown `selfDrops` on a hit, body or Substitute: Rage leaves its user raging — Gen 3
    /// (the cartridge's flag is the accuracy check's, `RageRules::accuracy_sets`) not against a
    /// Substitute.
    pub(super) fn rage_self_drop(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        on_substitute: bool,
    ) {
        if move_def.behavior.rage && !(on_substitute && self.rules.conditions.rage.accuracy_sets) {
            BattleContext::new(self.state, self.rules, self.trace).start_rage(actor);
        }
    }

    /// Showdown `selfDrops` on a hit, body or Substitute: Glaive Rush leaves its user open
    /// (§15.20).
    pub(super) fn glaive_rush_self_drop(&mut self, actor: PokemonId, move_def: &ResolvedMove) {
        if move_def.behavior.glaive_rush {
            let _ = BattleContext::new(self.state, self.rules, self.trace)
                .set_marker_volatile(actor, crate::state::VolatileConditionKind::GlaiveRush);
        }
    }

    /// The struck Pokémon's rage, in a damaging move's `Hit` event (its body, or a heal in
    /// place of the hit) or `AfterSubDamage` (its Substitute) — no status move gets here
    /// (Showdown's `move.category !== 'Status'`): Gen 5+ Showdown's `rage.onHit` raises its
    /// Attack for another Pokémon's hit on its body; Gen 2 the cartridge counts a foe's hit,
    /// body or Substitute; Gen 1 raises it for a hit on its body — on the cartridge its
    /// Substitute's too — but not a self-destructing move's, which builds once whatever it did
    /// (`rage_after_move`).
    pub(super) fn rage_struck(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        hit: RageHit,
    ) {
        if target == actor || !self.raging(target) {
            return;
        }
        match (self.rules.conditions.rage.build, hit) {
            (RageBuild::Boost, RageHit::Body | RageHit::Heal) => {
                BattleContext::new(self.state, self.rules, self.trace).raise_rage(target);
            }
            (RageBuild::Counter, RageHit::Body | RageHit::Substitute) => {
                BattleContext::new(self.state, self.rules, self.trace).count_rage(target);
            }
            (
                RageBuild::Gen1 {
                    substitute_builds, ..
                },
                RageHit::Body | RageHit::Substitute,
            ) if move_def.self_destruct.is_none()
                && (hit == RageHit::Body || substitute_builds) =>
            {
                self.gen1_rage_build(target);
            }
            _ => {}
        }
    }

    /// Gen 1: the raging Pokémon's Attack goes up below +6 (Showdown gen1 `boost`, pokered
    /// `StatModifierUpEffect`); on the cartridge a stage that moved cuts its foe's paralysed
    /// Speed / burned Attack again, as every stat-up effect's end does
    /// (`RageBuild::Gen1::cuts_foe_status_stats`).
    fn gen1_rage_build(&mut self, holder: PokemonId) {
        let RageBuild::Gen1 {
            cuts_foe_status_stats,
            ..
        } = self.rules.conditions.rage.build
        else {
            return;
        };
        if self
            .state
            .pokemon(holder)
            .is_none_or(|p| p.stages.attack >= 6)
        {
            return;
        }
        let raised = BattleContext::new(self.state, self.rules, self.trace).raise_rage(holder);
        if raised && cuts_foe_status_stats {
            self.cut_foe_status_stats_again(holder);
        }
    }

    /// Gen 3-4 on the cartridges (pokeemerald `MOVEEND_RAGE`, pokeplatinum `ONE_HIT_RAGE`):
    /// a hit that took `target`'s HP raises the standing raging target's Attack after the
    /// move's secondaries and before the reactions — Gen 3 a foe's hit only. At +6 the stage
    /// cannot go higher: nothing happens (the cartridges' `< MAX_STAT_STAGE`).
    pub(super) fn rage_stage(&mut self, actor: PokemonId, target: PokemonId) {
        let RageBuild::Stage { foes_only } = self.rules.conditions.rage.build else {
            return;
        };
        let foe =
            self.state.pokemon(actor).map(|p| p.side) != self.state.pokemon(target).map(|p| p.side);
        if target == actor || (foes_only && !foe) || !self.raging(target) {
            return;
        }
        BattleContext::new(self.state, self.rules, self.trace).raise_rage(target);
    }

    /// Gen 5+ Rage's `onBeforeMove` (priority 100, ahead of every check): any move attempt
    /// ends it.
    pub(super) fn rage_before_move(&mut self, actor: PokemonId) {
        if self.rules.conditions.rage.end == RageEnd::BeforeMove {
            BattleContext::new(self.state, self.rules, self.trace).end_rage(actor);
        }
    }

    /// The user's own move is over. Gen 4 (pokeplatinum `AFTER_MOVE_HIT_STATE_RAGE`): a move
    /// other than Rage that got past BeforeMove ends the flag — not a Pursuit that struck a
    /// Pokémon switching out (Showdown's `useMove` runs no `AfterMove`). Gen 3 (`RageRules::
    /// accuracy_sets`): a Rage that missed — or met Protect — ends it; one whose accuracy
    /// check passed starts it even as a type immunity (Ghost) or Wonder Guard stops it, unless
    /// the target is behind a Substitute.
    pub(super) fn rage_after_move(&mut self, actor: PokemonId, execution: &MoveExecution) {
        if execution.stopped {
            return;
        }
        let rules = self.rules.conditions.rage;
        // Gen 1 (Showdown gen1 `useMove` after `tryMoveHit`; pokered: no hit test before
        // `HandleBuildingRage` for Disable, and Explosion's `.notDone`): Disable and a
        // self-destructing move build their target's rage, hit or miss.
        if matches!(rules.build, RageBuild::Gen1 { .. })
            && (execution.move_id == MoveId::DISABLE
                || self
                    .dex
                    .move_by_id(execution.move_id)
                    .is_some_and(|m| m.self_destruct.is_some()))
        {
            let mut struck: Vec<PokemonId> = Vec::new();
            for hit in &execution.hits {
                let target = match *hit {
                    HitOutcome::Missed { target }
                    | HitOutcome::Blocked { target, .. }
                    | HitOutcome::Damaged { target, .. }
                    | HitOutcome::Effect { target }
                    | HitOutcome::SubstituteDamaged { target, .. }
                    | HitOutcome::NoEffect { target }
                    | HitOutcome::Reflected { target } => target,
                };
                if target != actor && !struck.contains(&target) {
                    struck.push(target);
                }
            }
            // A move that settled its target without an outcome of its own (Disable's custom
            // primary) aimed at the foe: Gen 1 is singles only.
            if struck.is_empty()
                && let Some(side) = self.state.pokemon(actor).map(|p| p.side)
                && let Some(foe) = self.state.side(side.other()).active[0]
            {
                struck.push(foe);
            }
            for target in struck {
                if self.raging(target) {
                    self.gen1_rage_build(target);
                }
            }
        }
        if matches!(rules.end, RageEnd::Gen4 { .. })
            && execution.move_id != MoveId::RAGE
            && !self.skips_before_move
        {
            BattleContext::new(self.state, self.rules, self.trace).end_rage(actor);
        }
        if !rules.accuracy_sets || execution.move_id != MoveId::RAGE {
            return;
        }
        for hit in &execution.hits {
            let ends = match *hit {
                HitOutcome::Missed { .. }
                | HitOutcome::Blocked {
                    reason: HitBlockReason::Protect,
                    ..
                } => true,
                HitOutcome::Blocked {
                    reason: HitBlockReason::TypeImmunity,
                    ..
                } if self.immune_target_missed => true,
                HitOutcome::Blocked {
                    target,
                    reason:
                        HitBlockReason::TypeImmunity
                        | HitBlockReason::Airborne
                        | HitBlockReason::Ability(_),
                } => {
                    if self.state.pokemon(target).is_some_and(|p| {
                        !p.volatiles
                            .contains(crate::state::VolatileConditionKind::Substitute)
                    }) {
                        BattleContext::new(self.state, self.rules, self.trace).start_rage(actor);
                    }
                    false
                }
                _ => false,
            };
            if ends {
                BattleContext::new(self.state, self.rules, self.trace).end_rage(actor);
            }
        }
    }
}
