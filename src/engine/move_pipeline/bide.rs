//! Bide's turns after the first (DESIGN.md §15.10): its volatile's `onBeforeMove` and the
//! move's `beforeMoveCallback` take the holder's action — storing, or releasing twice what was
//! stored at the Pokémon that dealt the last of it.

use crate::dex::{FixedDamage, PrimaryEffect};
use crate::ids::{MoveId, PokemonId};
use crate::rules::{Fraction, ResolvedTarget, TargetPattern};
use crate::state::{MoveResult, VolatileConditionKind};

use crate::engine::{BattleContext, MoveFailReason, TraceEvent};

use super::{HitOutcome, MoveExecution, MovePipeline};

impl<'a> MovePipeline<'a> {
    /// The Bide holder's action once BeforeMove let it through, or `None` without Bide. Gen 2+:
    /// storing until its `duration` reads 1 (`-activate`); Gen 1: the battle's last damage is
    /// stored and `time` counts down, the release at 0. The release (`-end`) fails with nothing
    /// stored; it strikes the last Pokémon that damaged the holder — gone, a random foe; Gen 1
    /// always a random foe — with a Normal physical contact hit of twice the store, through the
    /// hit steps only in Gen 2-3 (`BideRules`). Either way the move counts as failed (Showdown
    /// `moveThisTurnResult = false`), no PP is spent and the turn leaves no last move.
    pub(super) fn bide_turn(&mut self, actor: PokemonId, move_id: MoveId) -> Option<MoveExecution> {
        let rules = self.rules.conditions.retaliation.bide;
        let last_damage = self.state.last_damage;
        let bide = self.state.pokemon_mut(actor)?.volatiles.bide_mut()?;
        let release = if rules.gen1 {
            bide.damage = bide.damage.saturating_add(u32::from(last_damage));
            bide.time = bide.time.saturating_sub(1);
            bide.time == 0
        } else {
            bide.turns == Some(1)
        };
        let bide = *bide;
        // Storing, the volatile's `onBeforeMove` lets the action through and the move's
        // `beforeMoveCallback` stops it; the release is the volatile's `onBeforeMove` refusing it
        // (`MoveAborted` runs).
        let stopped = |hits| MoveExecution {
            actor,
            move_id,
            hits,
            stopped: true,
            aborted: release,
            result: MoveResult::Failed,
        };
        self.trace.push(TraceEvent::MoveFailed {
            actor,
            move_id,
            reason: MoveFailReason::Bide { released: release },
        });
        // Gen 3-4 on the cartridge: the release, stopped before the move as Showdown's
        // `MoveAborted` has it, leaves no last move (issues.md I67) — Gen 3 a storing turn too
        // (I114). A tormented holder may choose Bide again.
        if self.rules.conditions.blocked_move_clears_last_move
            && (release || rules.storing_clears_last_move)
            && let Some(pokemon) = self.state.pokemon_mut(actor)
        {
            pokemon.last_move = None;
            pokemon.last_move_encore = None;
            pokemon.last_move_base = None;
        }
        if !release {
            return Some(stopped(Vec::new()));
        }
        let hits = self.release_bide(actor, bide);
        self.phase2_item_after_release(actor, move_id, &hits);
        BattleContext::new(self.state, self.rules, self.trace)
            .end_volatile(actor, VolatileConditionKind::Bide);
        Some(stopped(hits))
    }

    fn release_bide(&mut self, actor: PokemonId, bide: crate::state::BideState) -> Vec<HitOutcome> {
        let rules = self.rules.conditions.retaliation.bide;
        if bide.damage == 0 || (!rules.gen1 && bide.source.is_none()) {
            return Vec::new();
        }
        let standing = |pokemon: PokemonId| {
            self.state
                .active_position(pokemon, &self.rules.topology)
                .is_some()
                && self.state.pokemon(pokemon).is_some_and(|p| !p.is_fainted())
        };
        let Some(mut release) = self.dex.move_by_id(MoveId::BIDE).cloned() else {
            return Vec::new();
        };
        // `getRandomTarget(pokemon, 'Pound')` when the source is gone (Gen 1: always): one of
        // the adjacent foes standing, drawn when there are several (`RandomTarget`).
        release.target = TargetPattern::SelectedAdjacent;
        let target = match bide
            .source
            .filter(|source| !rules.gen1 && standing(*source))
        {
            Some(source) => Some(source),
            None => self
                .fallback_target(
                    actor,
                    &release,
                    None,
                    crate::rules::TargetError::TargetRequired,
                )
                .and_then(|targets| match targets.targets.first() {
                    Some(ResolvedTarget::Pokemon(target)) => Some(*target),
                    _ => None,
                }),
        };
        let Some(target) = target else {
            return Vec::new();
        };
        release.primary_effect = PrimaryEffect::Damage;
        release.fixed_damage = Some(FixedDamage::Bide);
        release.flags.type_immunity = rules.release_type_immunity;
        let gate = if rules.release_hit_steps {
            self.resolve_hit_gate(actor, target, &release)
        } else if rules.release_type_immunity {
            self.type_immunity_step(actor, target, &release)
        } else {
            Ok(Fraction::ONE)
        };
        let outcomes = match gate {
            Ok(effectiveness) => {
                self.apply_damage_sequence(actor, target, &release, 1, effectiveness)
            }
            Err(outcome) => vec![outcome],
        };
        // Gen 2-4 the release is an attack its target records; Gen 5+ it is no attack
        // (`BideRules::release_records_attack`, DESIGN.md R-M8-90) but still a physical damaging
        // hit, which Counter's `onDamagingHit` answers and Metal Burst and Revenge do not
        // (R-M9-28).
        if rules.release_records_attack {
            self.record_attack(actor, target, &release, &outcomes);
        } else if !rules.gen1
            && let Some(HitOutcome::Damaged { amount, .. }) = outcomes.last()
        {
            let record = crate::state::AttackRecord {
                source: actor,
                damage: Some(amount.0),
                move_id: MoveId::BIDE,
                position: self.state.active_position(actor, &self.rules.topology),
                category: crate::dex::MoveCategory::Physical,
                damaging_hit_only: true,
            };
            if let Some(pokemon) = self.state.pokemon_mut(target) {
                pokemon.history.attacked_this_turn.push(record);
            }
        }
        outcomes
    }
}
