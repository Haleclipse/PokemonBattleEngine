//! The hit gate every foreign target passes (DESIGN.md §15.1): semi-invulnerability; type
//! immunity, TryHit (Protect, abilities, items, powder), one-hit KO immunity and accuracy in the
//! generation's order; a Substitute against a status move.

use crate::dex::{MoveCategory, PrimaryEffect, ResolvedMove, StatusMoveEffect};
use crate::effects;
use crate::ids::{PokemonId, SideId};
use crate::rules::{Fraction, HitStepOrder};
use crate::state::{
    NonVolatileStatus, RandomRequestKind, SideConditionKind, VolatileConditionKind,
};

use crate::engine::damage::HitBlockReason;
use crate::engine::{BattleContext, EffectSource, TraceEvent};

use super::{HitOutcome, MovePipeline};

/// Where a target stands once its hit gate passed TryHit (`MovePipeline::hit_gate_through_try_hit`):
/// what the generation's order leaves for the rest of the gate.
#[derive(Debug, Clone, Copy)]
pub(super) enum PastTryHit {
    /// Gen 7+ (TryHit first): type immunity, the move's immunities and accuracy ahead.
    TypeAhead,
    /// Gen 1-2, 5-6 (type immunity first): the type effectiveness known; accuracy ahead.
    AccuracyAhead(Fraction),
    /// Gen 3-4 (TryHit after the accuracy roll): every step taken.
    Done(Fraction),
}

impl<'a> MovePipeline<'a> {
    /// The shared per-target hit gate: semi-invulnerability, then type immunity (damaging
    /// primaries only, because a custom primary's own pipeline such as status application owns
    /// its immunity rules), TryHit (Protect / ability / item / powder), one-hit KO immunity and
    /// accuracy in the generation's order (`HitRules::step_order`), and last a Substitute
    /// against a status move. On success the resolved type effectiveness is returned so the
    /// damage primary does not recompute it.
    pub(super) fn resolve_hit_gate(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<Fraction, HitOutcome> {
        let past = self.hit_gate_through_try_hit(actor, target, move_def)?;
        self.hit_gate_after_try_hit(actor, target, move_def, past)
    }

    /// Semi-invulnerability (Showdown `hitStepInvulnerabilityEvent`): a target in the air /
    /// underground is missed outright unless something reaches it — the user itself too, from
    /// Gen 2 (`TwoTurnRules::hides_from_itself`).
    pub(super) fn invulnerability_step(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<(), HitOutcome> {
        let Some(kind) =
            crate::engine::two_turn::dodged_by(self.state, self.rules, actor, target, move_def)
        else {
            return Ok(());
        };
        // Gen 2 on the cartridge: the flag is read (and cleared) before the target's flight
        // makes Earthquake, Fissure or Magnitude miss.
        if crate::engine::context::locked_on(self.state, self.rules, actor, target) {
            BattleContext::new(self.state, self.rules, self.trace).consume_locked_on(target);
        }
        self.trace.push(TraceEvent::SemiInvulnerableDodged {
            actor,
            target,
            move_id: move_def.id,
            kind,
        });
        Err(HitOutcome::Missed { target })
    }

    /// The hit gate up to TryHit, Magic Coat's and Magic Bounce's reflection included: where a
    /// move with several targets has every target pass this stage before any goes on (Gen 4+,
    /// Showdown's `trySpreadMoveHit` running each step over all targets, `HitRules::
    /// staged_spread_hits`, DESIGN.md R-M8-77).
    pub(super) fn hit_gate_through_try_hit(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<PastTryHit, HitOutcome> {
        let move_id = move_def.id;

        // 0. Semi-invulnerability.
        self.invulnerability_step(actor, target, move_def)?;

        match self.rules.hits.step_order {
            HitStepOrder::ImmunityFirst => {
                let effectiveness = self.type_immunity_step(actor, target, move_def)?;
                if !self.rules.hits.move_immunity_after_try_hit {
                    self.move_immunity_step(actor, target, move_def)?;
                }
                self.try_hit_step(actor, target, move_def)?;
                Ok(PastTryHit::AccuracyAhead(effectiveness))
            }
            HitStepOrder::TryHitFirst => {
                self.try_hit_step(actor, target, move_def)?;
                Ok(PastTryHit::TypeAhead)
            }
            HitStepOrder::AccuracyBeforeTryHit => {
                let effectiveness = self.type_immunity_step(actor, target, move_def)?;
                self.ohko_step(actor, target, move_def)?;
                self.gate_accuracy(actor, target, move_def)?;
                self.move_immunity_step(actor, target, move_def)?;
                self.try_hit_step(actor, target, move_def)?;
                Ok(PastTryHit::Done(effectiveness))
            }
            // Gen 3 `tryMoveHit`: the immunity is known first but only reported after the
            // accuracy roll (a miss against an immune target is `-immune` too) and TryHit.
            HitStepOrder::AccuracyFirst => {
                let (effectiveness, immunity) = self.type_immunity(target, move_def);
                // Gen 3's `tryMoveHit` treats the move's own immunity like the type's.
                let immunity = immunity.or_else(|| {
                    crate::engine::move_immune(self.state, self.rules, actor, target, move_def)
                        .then_some(HitBlockReason::MoveRule)
                });
                self.ohko_step(actor, target, move_def)?;
                let accuracy = self.gate_accuracy(actor, target, move_def);
                if let Some(reason) = immunity
                    && accuracy.is_err()
                {
                    self.immune_target_missed = true;
                    return Err(self.block(actor, target, move_id, reason));
                }
                accuracy?;
                if let Err(outcome) = self.try_hit_step(actor, target, move_def) {
                    if immunity == Some(HitBlockReason::TypeImmunity)
                        && matches!(
                            outcome,
                            HitOutcome::Blocked {
                                reason: HitBlockReason::Ability(
                                    crate::ids::AbilityId::WONDER_GUARD
                                ),
                                ..
                            }
                        )
                    {
                        self.type_immune_target_guarded = true;
                    }
                    return Err(outcome);
                }
                if let Some(reason) = immunity {
                    return Err(self.block(actor, target, move_id, reason));
                }
                Ok(PastTryHit::Done(effectiveness))
            }
        }
    }

    /// The rest of the hit gate once TryHit passed: the steps the generation's order puts after
    /// it, then the protection-breaking, boost-stealing and move-specific `TryHit`s and a
    /// Substitute against a status move.
    pub(super) fn hit_gate_after_try_hit(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        past: PastTryHit,
    ) -> Result<Fraction, HitOutcome> {
        let move_id = move_def.id;
        let type_effectiveness = match past {
            PastTryHit::AccuracyAhead(effectiveness) => {
                if self.rules.hits.move_immunity_after_try_hit {
                    self.move_immunity_step(actor, target, move_def)?;
                }
                self.powder_immunity_step(actor, target, move_def)?;
                self.ohko_step(actor, target, move_def)?;
                self.gate_accuracy(actor, target, move_def)?;
                effectiveness
            }
            PastTryHit::TypeAhead => {
                let effectiveness = self.type_immunity_step(actor, target, move_def)?;
                self.move_immunity_step(actor, target, move_def)?;
                self.powder_immunity_step(actor, target, move_def)?;
                self.prankster_immunity_step(actor, target, move_id)?;
                self.ohko_step(actor, target, move_def)?;
                self.gate_accuracy(actor, target, move_def)?;
                effectiveness
            }
            PastTryHit::Done(effectiveness) => effectiveness,
        };

        // Showdown `hitStepBreakProtect`, once the hit steps passed (Feint is not protectable
        // itself): the target's protection and its side's guards come down.
        if move_def.breaks_protect {
            self.break_protection(actor, target);
        }
        // Showdown `hitStepStealBoosts` (Spectral Thief), before any hit.
        if move_def.steals_boosts {
            BattleContext::new(self.state, self.rules, self.trace).steal_boosts(
                actor,
                target,
                move_def.id,
            );
        }

        // Sky Drop's `onTryHit` (§15.23, after the accuracy check): the carrying up, or the drop.
        if move_def.behavior.sky_drop {
            self.sky_drop_try_hit(actor, target)?;
        }

        // Gen 2 Swagger's `onTryHit` (in `moveHit`, after the accuracy roll): a target whose
        // Attack is maxed out — stage +6, or 999 with its stages — makes it miss (`-miss|user`).
        if move_def.behavior.misses_maxed_attack
            && self.state.pokemon(target).is_some_and(|p| {
                p.stages.attack >= 6
                    || crate::engine::stats::apply_stage(
                        self.rules,
                        p.battle_stats().attack,
                        p.stages.attack,
                    ) >= 999
            })
        {
            return Err(self.block(actor, target, move_id, HitBlockReason::MoveRule));
        }

        // Low Kick, Grass Knot, Heavy Slam, Heat Crash (their own `onTryHit`, in `spreadMoveHit`
        // after the accuracy check): the move fails against a Dynamaxed target — its weight
        // cannot be read (DESIGN.md §44.5) — once its accuracy was drawn (R-M9-33).
        if move_def.behavior.dynamax_only_try_hit
            && self
                .state
                .pokemon(target)
                .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::Dynamax))
        {
            return Err(self.block(actor, target, move_id, HitBlockReason::MoveRule));
        }

        // The move's own `onTryHit` (`spreadMoveHit`, after the accuracy check): Yawn needs a
        // target without a status (`-fail|user`); Gen 4 Embargo one neither Multitype nor
        // holding a Griseous Orb — the ability and the item as they are, suppressed or not;
        // Gastro Acid an ability that can be suppressed.
        if let Some(rule) = move_def.behavior.target_rule
            && self.state.pokemon(target).is_some_and(|p| match rule {
                // Gen 3-4 (the cartridge, issues.md I63): not while an uproar keeps the target
                // awake.
                crate::effects::moves::TargetRule::TargetUnstatused => {
                    p.status.is_some()
                        || (self.rules.conditions.uproar.yawn_fails
                            && crate::engine::context::kept_awake(self.state, self.rules, target))
                }
                crate::effects::moves::TargetRule::ItemBoundTarget => {
                    crate::engine::raw_ability(p) == crate::ids::AbilityId::MULTITYPE
                        || p.held_item == Some(crate::ids::ItemId::GRISEOUS_ORB)
                }
                crate::effects::moves::TargetRule::SuppressibleAbility => self
                    .rules
                    .conditions
                    .cant_suppress
                    .contains(&crate::engine::raw_ability(p)),
                crate::effects::moves::TargetRule::UserMovesLast => self
                    .state
                    .pending_movers
                    .iter()
                    .any(|mover| *mover != actor),
            })
        {
            return Err(self.block(actor, target, move_id, HitBlockReason::MoveRule));
        }
        // Gastro Acid's `onTryHit`, then: Ability Shield (`hasItem`: its effects must apply)
        // blocks it (`-block`, no `-fail`).
        if move_def.behavior.target_rule
            == Some(crate::effects::moves::TargetRule::SuppressibleAbility)
            && crate::engine::held_item_id(self.state, self.rules, target)
                == Some(crate::ids::ItemId::ABILITY_SHIELD)
        {
            return Err(self.block(
                actor,
                target,
                move_id,
                HitBlockReason::Item(crate::ids::ItemId::ABILITY_SHIELD),
            ));
        }
        // Poltergeist's `onTryHit`: `-activate|target|move: Poltergeist|<item>` names the
        // target's item (R-M10-40).
        if move_def.behavior.announcement == Some(crate::effects::moves::Announcement::TargetItem)
            && let Some(item) = self.state.pokemon(target).and_then(|p| p.held_item)
        {
            BattleContext::new(self.state, self.rules, self.trace).announce_item(target, item);
        }
        // Gen 5+ Uproar's `onTryHit`: the sleepers wake.
        self.uproar_wakes_on_hit(target, move_def);

        // A Substitute absorbs damaging moves (DamageApplication) but makes status moves fail,
        // once they have passed the other steps (Showdown `onTryPrimaryHit`, in the hit loop):
        // declarative ones and, from Gen 2, custom ones aimed at a foe (Leech Seed) alike. Gen
        // 1's only stops poison, stat drops and confusion (gen1 `substitute` `onTryHit`): sleep
        // and paralysis go through.
        let status_move = match &move_def.primary_effect {
            PrimaryEffect::Status(effect) if self.rules.conditions.substitute_gen1 => matches!(
                effect,
                StatusMoveEffect::SetStatus(NonVolatileStatus::Poison | NonVolatileStatus::Toxic)
                    | StatusMoveEffect::StatStages(_)
                    | StatusMoveEffect::Volatile(VolatileConditionKind::Confusion)
            ),
            // Gen 2: Swagger's stat stages go through (its confusion is dropped, `primary`).
            PrimaryEffect::Status(_)
                if self.rules.conditions.substitute_gen2_quirks
                    && move_def.behavior.boosts_through_gen2_substitute =>
            {
                false
            }
            PrimaryEffect::Status(_) => true,
            // A move that calls another from its `onTryHit` (Nature Power, Mirror Move, Me First:
            // `callsMove`, aimed at a foe) is over by then: the move it calls meets the Substitute
            // itself (DESIGN.md R-M8-81).
            // Gen 2's Mean Look and Spider Web go through (`MoveBehavior::through_gen2_substitute`).
            PrimaryEffect::Custom => {
                move_def.category == MoveCategory::Status
                    && !self.rules.conditions.substitute_gen1
                    && !move_def.calls_move
                    && !(self.rules.conditions.substitute_gen2_quirks
                        && move_def.behavior.through_gen2_substitute)
            }
            _ => false,
        };
        let behind_substitute = target != actor
            && self
                .state
                .pokemon(target)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Substitute));
        if status_move && !move_def.flags.bypass_substitute && behind_substitute {
            return Err(self.block(actor, target, move_id, HitBlockReason::Substitute));
        }
        // Gen 2: a draining move misses a Pokémon behind a Substitute (`-miss`, after the
        // accuracy roll).
        if self.rules.conditions.substitute_gen2_quirks
            && move_def.drain.is_some()
            && behind_substitute
        {
            return Err(self.block(actor, target, move_id, HitBlockReason::Substitute));
        }

        Ok(type_effectiveness)
    }

    /// Sky Drop's `onTryHit` (§15.23). The drop (the user carries someone): it lets go of them
    /// first (`removeVolatile('skydrop')`); a target other than the one carried fails, a Flying
    /// type is immune (`-immune`, `null`). The carrying up: a target behind a Substitute or on the
    /// user's side fails; Gen 6+ one of 200 kg or more is too heavy (`-fail … [heavy]`, `null`);
    /// otherwise the user carries it up (`BattleContext::lift`) and the move stops (`null`).
    fn sky_drop_try_hit(&mut self, actor: PokemonId, target: PokemonId) -> Result<(), HitOutcome> {
        let move_id = crate::ids::MoveId::SKY_DROP;
        let carried = self
            .state
            .pokemon(actor)
            .and_then(|pokemon| pokemon.volatiles.sky_drop_target());
        if let Some(carried) = carried {
            BattleContext::new(self.state, self.rules, self.trace).end_sky_drop(actor);
            if target != carried {
                return Err(self.block(actor, target, move_id, HitBlockReason::MoveRule));
            }
            if BattleContext::new(self.state, self.rules, self.trace)
                .has_type(target, crate::dex::PokemonType::Flying)
            {
                return Err(self.block(actor, target, move_id, HitBlockReason::TypeImmunity));
            }
            return Ok(());
        }
        let same_side =
            self.state.pokemon(actor).map(|p| p.side) == self.state.pokemon(target).map(|p| p.side);
        let behind_substitute = self
            .state
            .pokemon(target)
            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Substitute));
        if behind_substitute || same_side {
            return Err(self.block(actor, target, move_id, HitBlockReason::MoveRule));
        }
        if self.rules.conditions.sky_drop_heavy_fails
            && crate::engine::weight_hg(self.state, self.rules, self.dex, target) >= 2000
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: crate::engine::MoveFailReason::NoEffect,
            });
            return Err(HitOutcome::NoEffect { target });
        }
        let aimed = self.state.active_position(target, &self.rules.topology);
        BattleContext::new(self.state, self.rules, self.trace).lift(actor, target, aimed);
        Err(HitOutcome::NoEffect { target })
    }

    /// Showdown `hitStepTryImmunity`: the move's own immunity check (`onTryImmunity`).
    fn move_immunity_step(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<(), HitOutcome> {
        if crate::engine::move_immune(self.state, self.rules, actor, target, move_def) {
            return Err(self.block(actor, target, move_def.id, HitBlockReason::MoveRule));
        }
        Ok(())
    }

    /// Showdown `hitStepTryImmunity`: a Grass-type target (Gen 6+) shrugs a powder move off
    /// (`-immune`, before the accuracy check) — after the abilities, so Sap Sipper and its kin
    /// absorb it first.
    fn powder_immunity_step(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<(), HitOutcome> {
        let grass = move_def.flags.powder
            && target != actor
            && self.rules.conditions.powder_immunity
            && self.state.pokemon(target).is_some_and(|pokemon| {
                crate::engine::types_of(self.state, self.rules, pokemon)
                    .contains(crate::dex::PokemonType::Grass)
            });
        if grass {
            return Err(self.block(actor, target, move_def.id, HitBlockReason::Powder));
        }
        Ok(())
    }

    /// Gen 7+ `hitStepTryImmunity`: a Dark-type foe is immune to a move Prankster boosted, while
    /// its user still has Prankster (`-immune`, before the accuracy check).
    fn prankster_immunity_step(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_id: crate::ids::MoveId,
    ) -> Result<(), HitOutcome> {
        if crate::engine::prankster_immune(
            self.state,
            self.rules,
            actor,
            target,
            self.prankster_boosted,
        ) {
            return Err(self.block(actor, target, move_id, HitBlockReason::TypeImmunity));
        }
        Ok(())
    }

    /// Records a block and returns its outcome.
    fn block(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_id: crate::ids::MoveId,
        reason: HitBlockReason,
    ) -> HitOutcome {
        self.trace.push(TraceEvent::HitBlocked {
            actor,
            target,
            move_id,
            reason,
        });
        HitOutcome::Blocked { target, reason }
    }

    /// The type chart against the target and whether it makes the target immune
    /// (`type_immunity`), traced.
    fn type_immunity(
        &mut self,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> (Fraction, Option<HitBlockReason>) {
        let (type_effectiveness, reason) = type_immunity(self.state, self.rules, target, move_def);
        if move_def.flags.type_immunity {
            self.trace.push(TraceEvent::TypeEffectivenessResolved {
                target,
                move_id: move_def.id,
                move_type: move_def.move_type,
                modifier: type_effectiveness,
            });
        }
        (type_effectiveness, reason)
    }

    pub(super) fn type_immunity_step(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<Fraction, HitOutcome> {
        match self.type_immunity(target, move_def) {
            (_, Some(reason)) => Err(self.block(actor, target, move_def.id, reason)),
            (effectiveness, None) => Ok(effectiveness),
        }
    }

    /// TryHit: Protect and its kin, Magic Coat and Magic Bounce, the target's ability and item,
    /// powder immunity.
    fn try_hit_step(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<(), HitOutcome> {
        // The `TryHit` handlers by priority: the guards first (3-4), then Magic Coat (2) and
        // Magic Bounce (1), which take the move over and use it again with the reflector as its
        // user (DESIGN.md §42.8) — a Magic Bounce holder's Protect stops the move before its
        // ability sees it (R-M8-90) — then the rest.
        let reason = match self.guard_block(actor, target, move_def) {
            Some(reason) => reason,
            None => {
                if self.reflect_move(actor, target, move_def) {
                    return Err(HitOutcome::Reflected { target });
                }
                let Some(reason) = self.try_hit_block(actor, target, move_def) else {
                    return Ok(());
                };
                reason
            }
        };
        // A protection that stops a contact move costs the attacker something (King's Shield,
        // Spiky Shield …), after its `-activate`.
        if reason == HitBlockReason::Protect
            && crate::engine::move_makes_contact(self.state, self.rules, actor, move_def)
        {
            let outcome = self.block(actor, target, move_def.id, reason);
            if let Some(kind) = self
                .state
                .pokemon(target)
                .and_then(|pokemon| pokemon.volatiles.protect_kind())
            {
                let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                crate::effects::moves::punish_contact(&mut ctx, kind, actor, target);
            }
            return Err(outcome);
        }
        // An absorbing ability's refusal has a side effect (Lightning Rod's +1 SpA).
        if let HitBlockReason::Ability(ability) = reason
            && let Some(absorb) =
                crate::engine::ability_behavior(self.rules, ability).absorb_reaction
        {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            absorb(&mut ctx, target);
        }
        Err(self.block(actor, target, move_def.id, reason))
    }

    /// Magic Coat and Magic Bounce (`onTryHit`, priorities 2 and 1): a reflectable move aimed at
    /// a Pokémon wearing the coat — or holding the ability — is used again against its user.
    /// Only the target's own: a partner reflects a move aimed at its side alone
    /// (`reflect_side_move`, DESIGN.md R-M8-77). True when the move was taken over (the caller's
    /// hit loop ends and the called move runs, §42.8).
    fn reflect_move(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> bool {
        if actor == target || !self.reflectable(move_def) {
            return false;
        }
        let Some(coat) = self.reflection_of(target) else {
            return false;
        };
        self.reflect(actor, target, coat, true, move_def);
        true
    }

    /// Magic Coat's and Magic Bounce's `onAllyTryHitSide`: a reflectable move aimed at the foes'
    /// side is used again against its user by the first of that side's Pokémon, fastest first,
    /// wearing the coat or holding the ability. From Gen 6 a hiding Pokémon in the side's first
    /// position — the event's target — stops it (`CallRules::hiding_first_stops_side_reflection`).
    /// The reflector, when the move was taken over.
    pub(super) fn reflect_side_move(
        &mut self,
        actor: PokemonId,
        side: SideId,
        move_def: &ResolvedMove,
    ) -> Option<PokemonId> {
        if !self.reflectable(move_def) || self.state.pokemon(actor)?.side == side {
            return None;
        }
        if self
            .rules
            .conditions
            .calls
            .hiding_first_stops_side_reflection
            && self
                .rules
                .topology
                .position(side, 0)
                .and_then(|first| self.state.pokemon_at_position(first, &self.rules.topology))
                .and_then(|first| self.state.pokemon(first))
                .is_some_and(|first| crate::engine::two_turn::hiding(first).is_some())
        {
            return None;
        }
        let mut holders: Vec<(PokemonId, bool, u32)> = self
            .state
            .all_pokemon()
            .iter()
            .filter(|pokemon| {
                pokemon.side == side
                    && !pokemon.is_fainted()
                    && self
                        .state
                        .active_position(pokemon.id(), &self.rules.topology)
                        .is_some()
            })
            .filter_map(|pokemon| {
                let coat = self.reflection_of(pokemon.id())?;
                Some((
                    pokemon.id(),
                    coat,
                    crate::engine::stats::handler_speed(self.state, pokemon.id()),
                ))
            })
            .collect();
        holders.sort_by_key(|(_, _, speed)| std::cmp::Reverse(*speed));
        let (reflector, coat, _) = holders.first().copied()?;
        self.reflect(actor, reflector, coat, false, move_def);
        Some(reflector)
    }

    /// A move a reflection may take: `reflectable`, and not out of a reflection already
    /// (`move.hasBounced`).
    fn reflectable(&self, move_def: &ResolvedMove) -> bool {
        move_def.flags.reflectable && !self.bounced
    }

    /// How `pokemon` reflects: `Some(true)` wearing Magic Coat, `Some(false)` holding Magic
    /// Bounce (a breakable ability: `ability_handlers` leaves a suppressed or ignored one out).
    fn reflection_of(&self, pokemon: PokemonId) -> Option<bool> {
        let holder = self.state.pokemon(pokemon)?;
        if holder.volatiles.contains(VolatileConditionKind::MagicCoat) {
            Some(true)
        } else {
            crate::engine::ability_handler(self.state, self.rules, holder, |b| b.magic_bounce)
                .then_some(false)
        }
    }

    /// `reflector` takes `actor`'s move over and uses it back at `actor`. A coat reflecting a
    /// move aimed at its holder (`onTryHit`, not `onAllyTryHitSide`) hands its own Prankster
    /// boost on (DESIGN.md R-M8-88).
    fn reflect(
        &mut self,
        actor: PokemonId,
        reflector: PokemonId,
        coat: bool,
        at_holder: bool,
        move_def: &ResolvedMove,
    ) {
        let coat_boost = coat
            && at_holder
            && self
                .state
                .pokemon(reflector)
                .is_some_and(|holder| holder.volatiles.magic_coat_prankster());
        let ability = crate::engine::raw_ability(
            self.state.pokemon(reflector).expect("the reflector stands"),
        );
        // Gen 3-4 (the gen4 mod's `onTryHit`): reflecting ends the coat at once; Gen 5+ leave it
        // until the end of the turn.
        if coat && self.rules.conditions.calls.magic_coat_ends_on_reflection {
            BattleContext::new(self.state, self.rules, self.trace)
                .end_volatile(reflector, VolatileConditionKind::MagicCoat);
        }
        if coat {
            crate::engine::context::takes_over_under_pressure(
                self.state,
                self.rules,
                self.trace,
                reflector,
                actor,
                crate::ids::MoveId::MAGIC_COAT,
            );
        }
        self.trace.push(TraceEvent::MoveReflected {
            reflector,
            by: if coat {
                EffectSource::Move {
                    pokemon: reflector,
                    move_id: move_def.id,
                }
            } else {
                EffectSource::Ability {
                    pokemon: reflector,
                    ability,
                }
            },
        });
        let source = if coat {
            crate::state::CallSource::Move(crate::ids::MoveId::MAGIC_COAT)
        } else {
            crate::state::CallSource::Ability(crate::ids::AbilityId::MAGIC_BOUNCE)
        };
        BattleContext::new(self.state, self.rules, self.trace).call_move(
            reflector,
            move_def.id,
            crate::state::CallTarget::Pokemon(actor),
            source,
        );
        if let Some(call) = &mut self.state.pending_call {
            call.reflected_prankster = coat_boost;
        }
    }

    /// Showdown `hitStepBreakProtect`: the target's protection (any `ProtectKind` but Max Guard,
    /// which is not on its list — Feint only goes through it, R-M8-159) and its side's
    /// Crafty Shield / Mat Block / Quick Guard / Wide Guard — a guard on the user's own side only
    /// from Gen 6 — end; from Gen 6
    /// so does the target's stall counter, when something broke.
    fn break_protection(&mut self, actor: PokemonId, target: PokemonId) {
        let gen6 = self.rules.conditions.broken_protection_gen6;
        let max_guard = self
            .state
            .pokemon(target)
            .and_then(|pokemon| pokemon.volatiles.protect_kind())
            == Some(crate::state::ProtectKind::MaxGuard);
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        let mut broke = !max_guard && ctx.end_volatile(target, VolatileConditionKind::Protect);
        let target_side = ctx.side_of(target);
        if let Some(side) = target_side
            && (gen6 || ctx.side_of(actor) != target_side)
        {
            for kind in [
                SideConditionKind::CraftyShield,
                SideConditionKind::MatBlock,
                SideConditionKind::QuickGuard,
                SideConditionKind::WideGuard,
            ] {
                broke |= ctx.remove_side_condition(side, kind);
            }
        }
        if broke && gen6 {
            ctx.end_volatile(target, VolatileConditionKind::Stall);
        }
    }

    /// One-hit KO moves fail outright against a target they cannot affect (Showdown
    /// `-immune|target|[ohko]`, at the accuracy step).
    fn ohko_step(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<(), HitOutcome> {
        if crate::engine::ohko_blocked(self.state, self.rules, actor, target, move_def) {
            return Err(self.block(actor, target, move_def.id, HitBlockReason::Ohko));
        }
        Ok(())
    }

    /// The hit gate's accuracy check (Showdown `hitStepAccuracy`): a miss on the roll — not a
    /// one-hit KO move's — sets off the user's Blunder Policy (`hit_items::use_blunder_policy`).
    /// A later hit's own check (`multiaccuracy`) calls `accuracy_step` alone.
    fn gate_accuracy(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<(), HitOutcome> {
        let checked = self.accuracy_step(actor, target, move_def);
        if checked.is_err() && move_def.ohko.is_none() {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            crate::effects::items::hit_items::use_blunder_policy(&mut ctx, actor, move_def.id);
        }
        checked
    }

    /// What the `Accuracy` event uses up: Micle Berry's boost, a Lock-On on the target.
    pub(super) fn accuracy_event(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) {
        let spent =
            crate::engine::spend_accuracy_check(self.state, self.rules, actor, target, move_def);
        if spent.micle_berry {
            self.trace.push(TraceEvent::VolatileEnded {
                target: actor,
                kind: crate::state::VolatileConditionKind::MicleBerry,
                seen: true,
            });
        }
        if spent.locked_on {
            self.trace.push(TraceEvent::VolatileEnded {
                target,
                kind: crate::state::VolatileConditionKind::LockedOn,
                seen: true,
            });
        }
    }

    pub(super) fn accuracy_step(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Result<(), HitOutcome> {
        let move_id = move_def.id;
        let roll = crate::engine::accuracy_roll(self.state, self.rules, actor, target, move_def);
        self.accuracy_event(actor, target, move_def);
        let final_basis_points = roll.map_or(10_000, |(numerator, denominator)| {
            (u32::from(numerator) * 10_000 / u32::from(denominator)) as u16
        });
        self.trace.push(TraceEvent::AccuracyResolved {
            actor,
            target,
            move_id,
            base_percent: move_def.accuracy,
            final_basis_points,
        });
        // Gen 1 Thrash's lock keeps the byte this check used (`lockedmove.accuracy`, §15.12),
        // and so does Rage's (`rage.accuracy`, §15.19).
        if self.rules.hits.chance_scale == crate::rules::ChanceScale::ByteAlwaysRolls
            && let Some((numerator, 256)) = roll
            && let Some(pokemon) = self.state.pokemon_mut(actor)
        {
            let byte = numerator.min(255) as u8;
            if let Some(rampage) = pokemon.volatiles.rampage_mut() {
                rampage.accuracy = byte;
            }
            pokemon.volatiles.set_rage_lock(byte);
        }
        let hit = match roll {
            None => true,
            Some((numerator, denominator)) => {
                self.state
                    .rng
                    .roll_ratio_for(RandomRequestKind::Accuracy, numerator, denominator)
            }
        };
        self.trace.push(TraceEvent::AccuracyChecked {
            actor,
            target,
            move_id,
            chance: (final_basis_points / 100).min(100) as u8,
            hit,
        });
        if hit {
            Ok(())
        } else {
            // Gen 1 `tryMoveHit`: a miss leaves no last damage.
            if self.rules.conditions.retaliation.gen1_last_damage {
                self.state.last_damage = 0;
            }
            Err(HitOutcome::Missed { target })
        }
    }

    /// The move's priority as its effects see it (Showdown `move.priority`): Gen 6+ the one the
    /// action was ordered with (Prankster, Gale Wings, Triage), before that the move's own.
    pub(super) fn move_priority(&self, move_def: &ResolvedMove) -> i8 {
        if self.rules.action_order.boosted_priority_at_hit {
            self.action_priority.unwrap_or(move_def.priority)
        } else {
            move_def.priority
        }
    }

    /// A move on its user meets its own side's Quick Guard: Showdown runs `TryHit` on the user
    /// too, where of the guards only Quick Guard asks for no other target (Mat Block and Crafty
    /// Shield skip `move.target === 'self'`, Wide Guard wants a spread move). Bide (+1, stopped
    /// by Protect) does not start under its partner's Quick Guard (R-M8-119).
    pub(super) fn own_quick_guard(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
    ) -> Option<HitOutcome> {
        let side = self.state.pokemon(actor)?.side;
        (move_def.flags.protectable
            && self.move_priority(move_def) > 0
            && self
                .state
                .side(side)
                .conditions
                .contains(SideConditionKind::QuickGuard))
        .then(|| self.block(actor, actor, move_def.id, HitBlockReason::QuickGuard))
    }

    /// The guards' `TryHit` (`onTryHitPriority` 3-4, ahead of Magic Coat's and Magic Bounce's), in
    /// Showdown's handler order (`comparePriority`): priority 4 first — Wide Guard and Quick Guard
    /// (side conditions, `subOrder` 4), then Psychic Terrain (a field condition, 5) — then
    /// priority 3 — Protect and its variants (the target's volatile, sorted by its Speed ahead of
    /// the speedless side conditions), then Crafty Shield and Mat Block.
    fn guard_block(
        &self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Option<HitBlockReason> {
        // Wide Guard / Quick Guard on the target's side (Showdown `onTryHit` priority 4): spread
        // moves / moves with positive priority, only those Protect would stop.
        if move_def.flags.protectable
            && let Some(side) = self.state.pokemon(target).map(|p| p.side)
        {
            let conditions = &self.state.side(side).conditions;
            let spread = move_def.is_spread
                && (move_def.category != MoveCategory::Status
                    || self.rules.conditions.wide_guard_blocks_status);
            if spread && conditions.contains(SideConditionKind::WideGuard) {
                return Some(HitBlockReason::WideGuard);
            }
            if self.move_priority(move_def) > 0
                && conditions.contains(SideConditionKind::QuickGuard)
            {
                return Some(HitBlockReason::QuickGuard);
            }
        }
        // Psychic Terrain (`onTryHit`, priority 4): a move with raised priority does not reach a
        // grounded foe that is not semi-invulnerable (Gen 6+ priority: the one the action was
        // ordered with).
        if self.state.field.terrain.map(|t| t.kind) == Some(crate::state::TerrainKind::Psychic)
            && let (Some(user), Some(defender)) =
                (self.state.pokemon(actor), self.state.pokemon(target))
            && user.side != defender.side
            && crate::engine::environment::grounded(self.state, self.rules, defender)
            && crate::engine::two_turn::hiding(defender).is_none()
            && self.move_priority(move_def) > 0
        {
            return Some(HitBlockReason::PsychicTerrain);
        }
        // Protect and its variants (`ProtectKind`): King's Shield, Obstruct, Silk Trap and Burning
        // Bulwark let status moves through.
        if let Some(kind) = self
            .state
            .pokemon(target)
            .and_then(|pokemon| pokemon.volatiles.protect_kind())
        {
            // Max Guard (DESIGN.md §44.5): every move but its own list — Z-Moves and Max Moves
            // included, whatever their flags.
            if kind == crate::state::ProtectKind::MaxGuard {
                if !max_guard_bypasses(move_def.id) {
                    return Some(HitBlockReason::Protect);
                }
            } else if move_def.flags.protectable
                && (kind.blocks_status() || move_def.category != MoveCategory::Status)
            {
                return Some(HitBlockReason::Protect);
            }
        }

        // Crafty Shield / Mat Block on the target's side (`onTryHit` priority 3; the moves aimed
        // at the user itself or the whole field never come here): a status move / a damaging
        // move Protect would stop (`checkMoveBypassesProtect(…, blockStatus = false)`). Not a
        // move on the user's whole party (Howl, Jungle Healing, Life Dew: Bulbapedia, issues.md
        // I186), which Showdown's handler stops on every member.
        if let Some(side) = self.state.pokemon(target).map(|p| p.side) {
            let conditions = &self.state.side(side).conditions;
            if move_def.category == MoveCategory::Status
                && move_def.target != crate::rules::TargetPattern::UserAndAllies
                && conditions.contains(SideConditionKind::CraftyShield)
            {
                return Some(HitBlockReason::CraftyShield);
            }
            if move_def.flags.protectable
                && move_def.category != MoveCategory::Status
                && conditions.contains(SideConditionKind::MatBlock)
            {
                return Some(HitBlockReason::MatBlock);
            }
        }
        None
    }

    /// The `TryHit` handlers after Magic Bounce's: the target's ability and item. (The move's own
    /// runs in `spreadMoveHit`, after the accuracy check.)
    fn try_hit_block(
        &self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Option<HitBlockReason> {
        // Sturdy (Showdown `onTryHit`): one-hit KO moves do not affect the holder.
        if move_def.ohko.is_some()
            && let Some(ability) = self
                .state
                .pokemon(target)
                .and_then(|p| crate::engine::ability_running(self.state, self.rules, p))
            && crate::engine::ability_behavior(self.rules, ability).ohko_immune
        {
            return Some(HitBlockReason::Ability(ability));
        }

        // Overcoat and Safety Goggles (`onTryHit`): no accuracy check follows. The Grass
        // type's powder immunity is the TryImmunity step's, after the abilities
        // (`powder_immunity_step`).
        if move_def.flags.powder
            && target != actor
            && let Some(by) = crate::engine::powder_holds(self.state, self.rules, target)
        {
            return Some(match by {
                EffectSource::Ability { ability, .. } => HitBlockReason::Ability(ability),
                EffectSource::Item { item, .. } => HitBlockReason::Item(item),
                _ => HitBlockReason::Powder,
            });
        }

        crate::engine::holders_try_hit(self.state, self.rules, actor, target, move_def)
    }

    pub(super) fn type_effectiveness(
        &self,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Fraction {
        self.state.pokemon(target).map_or(Fraction::ONE, |pokemon| {
            if stellar_against_tera(pokemon, move_def) {
                return Fraction::new(2, 1);
            }
            crate::engine::types_of(self.state, self.rules, pokemon)
                .iter()
                .fold(Fraction::ONE, |current, defender| {
                    current.multiply(effectiveness_against(
                        self.state, self.rules, move_def, defender,
                    ))
                })
                .multiply(tar_shot_factor(pokemon, move_def))
        })
    }
}

/// Whether `target` is a Terapagos in its Terastal forme whose Tera Shell runs (Showdown
/// `Pokemon#runEffectiveness`: `species.name === 'Terapagos-Terastal' && hasAbility('Tera Shell')
/// && !suppressingAbility(this)` — the ability has no handler of its own, R-M9-45).
pub(crate) fn tera_shell_holds(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: PokemonId,
) -> bool {
    state.pokemon(target).is_some_and(|pokemon| {
        crate::engine::species_of(pokemon) == Some(crate::ids::SpeciesId::TERAPAGOS_TERASTAL)
            && crate::engine::ability_running(state, rules, pokemon)
                == Some(crate::ids::AbilityId::TERA_SHELL)
    })
}

/// Tera Shell takes the move as not very effective (`runEffectiveness` returns -1): a damaging
/// move but Struggle, one the holder is not immune to and that is not already not very
/// effective, while the holder is at full HP (R-M9-45).
pub(crate) fn tera_shell_resists(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: PokemonId,
    move_def: &ResolvedMove,
    effectiveness: Fraction,
) -> bool {
    tera_shell_holds(state, rules, target)
        && move_def.category != MoveCategory::Status
        && move_def.id != crate::ids::MoveId::STRUGGLE
        && !effectiveness.is_zero()
        && effectiveness.numerator >= effectiveness.denominator
        && state
            .pokemon(target)
            .is_some_and(|pokemon| pokemon.hp >= pokemon.max_hp)
}

/// Showdown `runEffectiveness(move)` as a factor: the move against each of the target's types, a
/// type the chart makes immune counting as neutral (`getEffectiveness`), Tar Shot's step
/// included. With `type_immunity`'s block it is Wonder Guard's whole question.
pub(crate) fn chart_effectiveness(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: PokemonId,
    move_def: &ResolvedMove,
) -> Fraction {
    state.pokemon(target).map_or(Fraction::ONE, |pokemon| {
        if stellar_against_tera(pokemon, move_def) {
            return Fraction::new(2, 1);
        }
        crate::engine::types_of(state, rules, pokemon)
            .iter()
            .fold(Fraction::ONE, |current, defender| {
                let modifier = effectiveness_against(state, rules, move_def, defender);
                current.multiply(if modifier.is_zero() {
                    Fraction::ONE
                } else {
                    modifier
                })
            })
            .multiply(tar_shot_factor(pokemon, move_def))
    })
}

/// Showdown `runEffectiveness`'s first step (DESIGN.md §44.1): a Stellar-type move against a
/// Terastallized target is super effective whatever its types are.
fn stellar_against_tera(pokemon: &crate::state::PokemonState, move_def: &ResolvedMove) -> bool {
    pokemon.terastallized.is_some() && move_def.move_type == crate::dex::PokemonType::Stellar
}

/// Tar Shot's `onEffectiveness` (priority -2, after the move's own): a Fire move is one step
/// more effective against the holder's first type — twice the whole.
fn tar_shot_factor(pokemon: &crate::state::PokemonState, move_def: &ResolvedMove) -> Fraction {
    if move_def.move_type == crate::dex::PokemonType::Fire
        && pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::TarShot)
    {
        Fraction::new(2, 1)
    } else {
        Fraction::ONE
    }
}

/// The move's modifier against each of the target's types that it is not neutral against,
/// type by type — an immunity the hit got past counts as neutral (Gen 4's cartridge meets them
/// one at a time, `BattleSystem_ApplyTypeChart`).
pub(super) fn type_factors(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: PokemonId,
    move_def: &ResolvedMove,
) -> Vec<(crate::dex::PokemonType, Fraction)> {
    state.pokemon(target).map_or_else(Vec::new, |pokemon| {
        crate::engine::types_of(state, rules, pokemon)
            .iter()
            .map(|defending| {
                (
                    defending,
                    effectiveness_against(state, rules, move_def, defending),
                )
            })
            .filter(|(_, factor)| !factor.is_zero() && factor.numerator != factor.denominator)
            .collect()
    })
}

/// The move's modifier against one defending type (Showdown `runEffectiveness`, per type): the
/// chart's, then the move's own `onEffectiveness`, then Delta Stream's (priority −1: a damaging
/// move is only neutral against a Flying type it would hit super effectively).
fn effectiveness_against(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    move_def: &ResolvedMove,
    defender: crate::dex::PokemonType,
) -> Fraction {
    let factor = own_effectiveness_against(rules, move_def, defender);
    if defender == crate::dex::PokemonType::Flying
        && move_def.category != crate::dex::MoveCategory::Status
        && factor.numerator > factor.denominator
        && crate::engine::weather::strong_winds_blow(state, rules)
    {
        return Fraction::ONE;
    }
    factor
}

fn own_effectiveness_against(
    rules: &crate::rules::ResolvedRules,
    move_def: &ResolvedMove,
    defender: crate::dex::PokemonType,
) -> Fraction {
    let chart = rules.type_chart.effectiveness(move_def.move_type, defender);
    let chart = if chart.is_zero() && move_def.flags.immunity_ignored {
        Fraction::ONE
    } else {
        chart
    };
    match move_def.behavior.effectiveness {
        Some(effects::moves::TypeEffectiveness::AlsoAs(extra)) => {
            chart.multiply(rules.type_chart.effectiveness(extra, defender))
        }
        Some(effects::moves::TypeEffectiveness::SuperEffectiveOn(weak)) if weak == defender => {
            Fraction::new(2, 1)
        }
        _ => chart,
    }
}

/// Foresight / Odor Sleuth's and Miracle Eye's `onNegateImmunity` (DESIGN.md §41.5): the
/// identified Ghost holder is not immune to Normal and Fighting moves, the identified Dark one to
/// Psychic moves.
fn identified_negates(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: &crate::state::PokemonState,
    move_type: crate::dex::PokemonType,
) -> bool {
    use crate::dex::PokemonType;
    use crate::state::VolatileConditionKind;
    let types = crate::engine::types_of(state, rules, pokemon);
    (pokemon.volatiles.contains(VolatileConditionKind::Foresight)
        && matches!(move_type, PokemonType::Normal | PokemonType::Fighting)
        && types.contains(PokemonType::Ghost))
        || (pokemon
            .volatiles
            .contains(VolatileConditionKind::MiracleEye)
            && move_type == PokemonType::Psychic
            && types.contains(PokemonType::Dark))
}

/// The hit gate's type-immunity step for a move that checks the type chart: the effectiveness,
/// and what makes the target immune — a zero effectiveness, or a Ground move against a target
/// that is not grounded (Showdown `runImmunity` → `isGrounded`: Levitate lifts its holder;
/// Showdown hard-codes it there rather than in a callback of the ability, so a status move,
/// which skips this step, is not stopped by it). A Ring Target holder
/// (`negates_type_immunity`, Showdown `NegateImmunity`) is immune to nothing but a Ground move
/// through Levitate; an identified one (`identified_negates`) to nothing that move's type would
/// otherwise be. Shared with the replay driver.
pub fn type_immunity(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: PokemonId,
    move_def: &ResolvedMove,
) -> (Fraction, Option<HitBlockReason>) {
    if !move_def.flags.type_immunity {
        return (Fraction::ONE, None);
    }
    let Some(pokemon) = state.pokemon(target) else {
        return (Fraction::ONE, None);
    };
    // A Stellar-type move against a Terastallized target is super effective whatever its types
    // (Showdown `runEffectiveness`, DESIGN.md §44.6).
    if stellar_against_tera(pokemon, move_def) {
        return (Fraction::new(2, 1), None);
    }
    // Disguise and Ice Face (`onEffectiveness`) make the move neutral on the whole forme, but
    // only where the chart is not already an immunity (`if (!target.runImmunity(move)) return;`)
    // and only for the moves the forme takes whole: any damaging one for Disguise, a physical
    // one for Ice Face (`move.category !== 'Physical'`: a special Max Flare stays super
    // effective on Eiscue) — not one that strikes a Substitute in front of it (`hitSub`,
    // R-M10-78: a Rock move on Eiscue's Substitute is super effective).
    let undisguised = crate::engine::ability_handler(state, rules, pokemon, |b| b.absorbing)
        .is_some_and(|absorbing| absorbing.answers_before_the_hit(state, rules, target, move_def));
    // The `Effectiveness` event after the chart, on every way past it: Disguise's or Ice Face's
    // 0 ends it (`runEvent` stops at a falsy value), else Tar Shot adds its step (priority -2).
    let effectiveness_event = |chart: Fraction| {
        if undisguised {
            Fraction::ONE
        } else {
            chart.multiply(tar_shot_factor(pokemon, move_def))
        }
    };
    let chart = crate::engine::types_of(state, rules, pokemon).iter().fold(
        Fraction::ONE,
        |current, defender| {
            current.multiply(effectiveness_against(state, rules, move_def, defender))
        },
    );
    let type_effectiveness = if chart.is_zero() {
        chart
    } else {
        effectiveness_event(chart)
    };
    // The chart's effectiveness ignores immunities (Showdown `getEffectiveness`): a type the
    // move cannot affect counts as neutral once nothing stops the move — a Ring Target's
    // holder's, Tar Shot's step included.
    let past_immunities = || {
        effectiveness_event(crate::engine::types_of(state, rules, pokemon).iter().fold(
            Fraction::ONE,
            |current, defender| {
                let modifier = effectiveness_against(state, rules, move_def, defender);
                current.multiply(if modifier.is_zero() {
                    Fraction::ONE
                } else {
                    modifier
                })
            },
        ))
    };
    let negates = crate::engine::held_item(state, rules, target)
        .is_some_and(|(_, item)| item.negates_type_immunity)
        || identified_negates(state, rules, pokemon, move_def.move_type);
    if move_def.move_type == crate::dex::PokemonType::Ground {
        // A Ground move's immunity is groundedness alone, so a Flying type rooted by Ingrain
        // or held down by an Iron Ball takes it neutrally — the whole hit, from Gen 5, for the
        // Iron Ball holder (its `onEffectiveness`). Thousand Arrows reaches one off the ground
        // as well: a Flying type there takes it neutrally whatever its other type (the move's
        // `onEffectiveness` returns 0 for each of its types).
        let reaches_the_airborne = move_def.behavior.effectiveness
            == Some(effects::moves::TypeEffectiveness::GroundReachesTheAirborne);
        if !crate::engine::environment::grounded_negating(state, rules, pokemon, negates)
            && reaches_the_airborne
        {
            let flying = crate::engine::types_of(state, rules, pokemon)
                .contains(crate::dex::PokemonType::Flying);
            return (
                if flying {
                    Fraction::ONE
                } else {
                    past_immunities()
                },
                None,
            );
        }
        if crate::engine::environment::grounded_negating(state, rules, pokemon, negates) {
            // Iron Ball's `onEffectiveness` steps aside under Ingrain and Gravity.
            let neutral = crate::engine::types_of(state, rules, pokemon)
                .contains(crate::dex::PokemonType::Flying)
                && !pokemon
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::Ingrain)
                && !state
                    .field
                    .contains(crate::state::FieldConditionKind::Gravity)
                && crate::engine::held_item(state, rules, target)
                    .is_some_and(|(_, item)| item.flying_holder_takes_ground_neutrally);
            return (
                if neutral {
                    Fraction::ONE
                } else {
                    past_immunities()
                },
                None,
            );
        }
        // Off the ground with a chart that lets the move through: by Levitate, which the
        // `-immune` names, or by an Air Balloon, Magnet Rise or Telekinesis, which it does not.
        let reason = if type_effectiveness.is_zero() && !negates {
            HitBlockReason::TypeImmunity
        } else if rules.hits.levitate_immunity_named
            && crate::engine::environment::grounding(state, rules, pokemon, negates)
                == crate::engine::environment::Grounding::Levitating
        {
            HitBlockReason::Ability(crate::ids::AbilityId::LEVITATE)
        } else {
            HitBlockReason::Airborne
        };
        return (type_effectiveness, Some(reason));
    }
    if negates {
        return (past_immunities(), None);
    }
    let reason = type_effectiveness
        .is_zero()
        .then_some(HitBlockReason::TypeImmunity);
    (type_effectiveness, reason)
}

/// The moves Max Guard lets through (Showdown `bypassesMaxGuard`, DESIGN.md §44.5). The two
/// G-Max moves it names are not in the snapshots.
fn max_guard_bypasses(move_id: crate::ids::MoveId) -> bool {
    use crate::ids::MoveId as M;
    [
        M::ACUPRESSURE,
        M::AFTER_YOU,
        M::ALLY_SWITCH,
        M::AROMATHERAPY,
        M::AROMATIC_MIST,
        M::COACHING,
        M::CONFIDE,
        M::COPYCAT,
        M::CURSE,
        M::DECORATE,
        M::DOOM_DESIRE,
        M::FEINT,
        M::FUTURE_SIGHT,
        M::HEAL_BELL,
        M::HOLD_HANDS,
        M::HOWL,
        M::JUNGLE_HEALING,
        M::LIFE_DEW,
        M::MEAN_LOOK,
        M::PERISH_SONG,
        M::PLAY_NICE,
        M::POWER_TRICK,
        M::ROAR,
        M::ROLE_PLAY,
        M::TEARFUL_LOOK,
    ]
    .contains(&move_id)
}
