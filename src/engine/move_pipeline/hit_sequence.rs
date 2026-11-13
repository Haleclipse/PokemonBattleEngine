//! The hit sequence of a damaging move (DESIGN.md §15.3, ROADMAP §6.4 / §6.5): hit count,
//! per-hit damage, Substitute routing, per-hit secondaries and contact reactions, drain and
//! recoil. Split out of `move_pipeline/mod.rs`; every method here is still `MovePipeline`'s.

use crate::dex::{MultiHit, PrimaryEffect, Recoil, ResolvedMove, SecondaryTrigger};
use crate::ids::PokemonId;
use crate::rules::{Fraction, RecoilTiming, Rounding, ShellBellCount};
use crate::state::RandomRequestKind;

use super::rage::RageHit;
use super::{ArmedFocusItem, HitOutcome, MovePipeline};
use crate::engine::damage::{DamageAmount, DamageApplication};
use crate::engine::{BattleContext, EffectSource, PendingFaints, TraceEvent};

/// The move strikes once (Showdown `move.multihit` unset: not a multi-hit move, Beat Up or
/// Parental Bond's) and keeps its `AfterMoveSecondary` events (no Sheer Force boost):
/// `DamageFlags::single_strike`.
fn strikes_once(move_def: &ResolvedMove) -> bool {
    !move_def.multi_strike() && !move_def.flags.sheer_force
}

impl<'a> MovePipeline<'a> {
    /// The move's own `self` (Close Combat's drops, Diamond Storm's boost) as a hit goes
    /// through, on a body or a Substitute (Showdown `selfDrops`): before the hit's secondaries
    /// and the target's `DamagingHit` reactions — a Mummy that takes the user's Simple comes
    /// after the drops it doubled (R-M11-102). A multi-hit move's (Parental Bond's Close
    /// Combat) lands on every hit, each with its own roll; a single-hit move's once
    /// (`selfDropped`), at its first such hit — a spread move's before any target's
    /// secondaries.
    fn move_self(&mut self, actor: PokemonId, target: PokemonId, move_def: &ResolvedMove) {
        if !move_def.multi_strike() && std::mem::replace(&mut self.self_dropped, true) {
            return;
        }
        self.apply_secondary_effects(
            actor,
            actor,
            Some(target),
            move_def,
            SecondaryTrigger::OnceAfterMove,
            false,
        );
    }

    /// Damage primary for one target that passed the hit gate, as a hit sequence (ROADMAP §6.4):
    /// the hit count is decided once, every hit resolves its own damage (critical, roll), goes
    /// through DamageApplication (Substitute routing), fires its per-hit secondaries and drain,
    /// and the sequence stops as soon as either side faints. Recoil follows the generation's
    /// timing (§6.5). No move gets to re-implement any part of this.
    pub(super) fn apply_damage_sequence(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        target_count: usize,
        type_effectiveness: Fraction,
    ) -> Vec<HitOutcome> {
        // A power of `null` (Gen 2-4 Beat Up with no member, Gen 2 Return / Frustration at 0):
        // nothing, not even a roll.
        if crate::engine::zero_power_does_nothing(
            self.state, self.rules, self.dex, actor, target, move_def,
        ) {
            return vec![HitOutcome::NoEffect { target }];
        }
        // Showdown `TryPrimaryHit`: the user's gem goes before the hit is resolved — Present's
        // heal, Pollen Puff on an ally and a Substitute's hit included.
        self.use_gem(actor, target, move_def);
        self.source_try_primary_hit(actor, target, move_def);
        // Present's heal, Pollen Puff on an ally: no power, the target is healed instead — once
        // per hit: Parental Bond's two hits heal twice (Showdown's hit loop runs `moveData.heal`
        // each time), and a hit that did nothing (a full-HP target) ends the loop (R-M8-161).
        if let Some(heal) = crate::engine::heals_instead(
            self.state, self.rules, actor, target, move_def, self.drawn,
        ) {
            let count = match move_def.multi_hit {
                MultiHit::Fixed(count) if move_def.flags.parental_bond => count,
                _ => 1,
            };
            if count > 1 {
                self.trace.push(TraceEvent::MultiHitResolved {
                    actor,
                    target,
                    move_id: move_def.id,
                    count,
                });
            }
            let mut outcomes = Vec::new();
            for _ in 0..count {
                let outcome = self.heal_instead(actor, target, move_def, heal);
                let nothing = matches!(outcome, HitOutcome::NoEffect { .. });
                outcomes.push(outcome);
                if nothing {
                    break;
                }
            }
            return outcomes;
        }
        let hit_rules = self.rules.hits;
        self.arm_focus_item(actor, target);
        let count = match move_def.multi_hit {
            _ if move_def.behavior.beat_up => self.beat_up_members.len() as u8,
            _ if self.smart_hit => 1,
            MultiHit::Single => 1,
            MultiHit::Fixed(count) => count,
            MultiHit::Range { min: 2, max: 5 } => self.state.rng.sample_weighted_for(
                RandomRequestKind::MultiHitCount,
                &[2, 3, 4, 5],
                &hit_rules.multi_hit_weights,
            ) as u8,
            MultiHit::Range { min, max } => self.state.rng.range_inclusive_for(
                RandomRequestKind::MultiHitCount,
                u16::from(min),
                u16::from(max),
            ) as u8,
        };
        if move_def.multi_strike() {
            self.trace.push(TraceEvent::MultiHitResolved {
                actor,
                target,
                move_id: move_def.id,
                count,
            });
        }
        let source = EffectSource::Move {
            pokemon: actor,
            move_id: move_def.id,
        };

        let multi_hit = move_def.multi_strike();
        let mut outcomes = Vec::with_capacity(usize::from(count));
        let mut first_damage: Option<DamageAmount> = None;
        let mut recoil_total: u32 = 0;
        self.hit_count = count;
        for hit in 1..=count {
            // A user put to sleep during the sequence (Effect Spore) stops (Showdown
            // `hitStepMoveHitLoop`'s `pokemon.status === 'slp'`, the gen2 / gen3 loops alike;
            // DESIGN.md R263) — unless Sleep Talk called the move (R-M8-81).
            if hit > 1
                && self
                    .state
                    .pokemon(actor)
                    .is_some_and(|p| p.has_status(crate::state::NonVolatileStatus::Sleep))
                && !(self.rules.hits.sleep_callers_hit_on && self.called_by_sleep_usable())
            {
                break;
            }
            // Showdown `multiaccuracy` (Triple Kick): every hit after the first checks accuracy
            // again; a miss ends the sequence quietly.
            if hit > 1
                && move_def.multi_accuracy
                && self.accuracy_step(actor, target, move_def).is_err()
            {
                break;
            }
            self.hit = hit;
            // Brick Break's `onTryHit` (Showdown `spreadMoveHit`, every hit): the screens come
            // down as the hit lands, before a Substitute can take it and before the damage.
            if move_def.behavior.breaks_screens {
                BattleContext::new(self.state, self.rules, self.trace).break_screens(actor, target);
            }
            // Gen 1 deals the first hit's damage on every hit; later generations roll each hit.
            // A Gen 1 binding lock repeats its first hit's damage (§30.9).
            let binding =
                crate::engine::binding_damage(self.state, self.rules, actor, target, move_def);
            // Whether this hit was a critical hit (Showdown `getMoveHitData(move).crit`): a
            // repeated damage (Gen 1) rolls none.
            let mut critical = false;
            // The hit's own type effectiveness (Showdown `modifyDamage` writes
            // `getMoveHitData(move).typeMod` on each hit it computes): what Enigma Berry and
            // Weakness Policy read — a hit after Ice Face broke is super effective again.
            let mut hit_effectiveness = type_effectiveness;
            let final_damage = match (binding, first_damage) {
                (Some(amount), _) => amount,
                (None, Some(amount)) if hit_rules.same_damage_each_hit => amount,
                _ => {
                    // A hit after the first asks the target's types again: Color Change (a Gen 3
                    // hit that rewrote them) leaves the later hits of a multi-hit move resisted
                    // (Showdown recomputes `getDamage` per hit, DESIGN.md R-M8-47). The question
                    // is the gate's own (`type_immunity`: Foresight's and Scrappy's negation, a
                    // Ring Target, the Ground special cases), not the bare chart — the first hit
                    // keeps the value the gate already gave it.
                    let effectiveness = if hit > 1 {
                        crate::engine::type_immunity(self.state, self.rules, target, move_def).0
                    } else {
                        type_effectiveness
                    };
                    hit_effectiveness = effectiveness;
                    let breakdown = self.resolve_standard_damage(
                        actor,
                        target,
                        move_def,
                        target_count,
                        effectiveness,
                    );
                    first_damage.get_or_insert(breakdown.final_damage);
                    critical = breakdown.critical.0;
                    breakdown.final_damage
                }
            };

            // Gen 1 (pokered `AdjustDamageForMoveType` sets `wMoveMissed`): a damage the type chart
            // took to 0 misses — nothing after it, a crash move's crash aside (issues.md I221).
            if hit_rules.zero_damage_misses
                && final_damage.0 == 0
                && binding.is_none()
                && move_def.fixed_damage.is_none()
            {
                if self.rules.conditions.retaliation.gen1_last_damage {
                    self.state.last_damage = 0;
                }
                self.trace.push(TraceEvent::MoveFailed {
                    actor,
                    move_id: move_def.id,
                    reason: crate::engine::MoveFailReason::ZeroDamage,
                });
                outcomes.push(HitOutcome::Missed { target });
                break;
            }
            // Gen 1 Psywave rolling 0 fails (the Desync Clause Mod: `damageCallback` → `false`);
            // so does a Gen 2 Counter with nothing to strike back at (its `damageCallback`).
            if final_damage.0 == 0
                && matches!(
                    move_def.fixed_damage,
                    Some(
                        crate::dex::FixedDamage::Psywave | crate::dex::FixedDamage::Retaliation(_)
                    )
                )
            {
                outcomes.push(HitOutcome::NoEffect { target });
                break;
            }
            self.roll_focus_band(target);
            let final_damage = self.focus_item_holds(target, final_damage, move_def);
            let application = {
                let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                ctx.apply_damage(crate::engine::DamageRequest {
                    target,
                    amount: final_damage,
                    kind: crate::engine::DamageKind::Move,
                    source,
                    flags: crate::engine::DamageFlags {
                        bypass_substitute: move_def.flags.bypass_substitute,
                        spare_one_hp: move_def.behavior.spares_target,
                        instafaint: false,
                        single_strike: strikes_once(move_def),
                        future_move: move_def.flags.future_move,
                    },
                })
            };

            let (outcome, dealt, hit_substitute) = match application {
                DamageApplication::Hp { amount, .. } => {
                    // Showdown's `spreadDamage`: Gen 1's recoil and the drain follow the
                    // damage at once, before the move's effects and the target's reactions.
                    if hit_rules.recoil_timing == RecoilTiming::PerHit && amount.0 > 0 {
                        self.apply_recoil(actor, move_def, u32::from(amount.0));
                    }
                    self.apply_drain(actor, target, move_def, amount.0, false);
                    // The move's own `onHit` (`runMoveEffects`, before the secondaries): Wake-Up
                    // Slap wakes, Smelling Salts cures paralysis.
                    if let Some(status) = move_def.behavior.cures_target_status
                        && self
                            .state
                            .pokemon(target)
                            .is_some_and(|p| !p.is_fainted() && p.has_status(status))
                    {
                        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                        ctx.cure_status(target, crate::engine::CureReason::Move(move_def.id));
                    }
                    // A flung herb's `fling.effect`, Fling's `onHit` (DESIGN.md R-M8-86).
                    if let Some(effect) = move_def.behavior.flung_on_hit {
                        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                        match effect {
                            crate::effects::items::FlingEffect::ClearLoweredStages => {
                                ctx.clear_lowered_stages(target, source);
                            }
                            crate::effects::items::FlingEffect::CureMentalHerbVolatiles => {
                                ctx.cure_mental_herb_volatiles(target);
                            }
                            crate::effects::items::FlingEffect::Status(_)
                            | crate::effects::items::FlingEffect::Flinch => {}
                        }
                    }
                    // Clear Smog: the target's stages back to 0.
                    if move_def.behavior.clears_target_stages {
                        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                        ctx.clear_stages(target, source);
                    }
                    // Plasma Fists' `pseudoWeather`, Steel Roller's `onHit` (`runMoveEffects`).
                    if move_def.behavior.starts_ion_deluge {
                        BattleContext::new(self.state, self.rules, self.trace).start_ion_deluge();
                    }
                    self.hit_or_sub(actor, target, move_def);
                    self.hit_traps(actor, target, move_def);
                    self.hit_takes_item(actor, target, move_def);
                    self.struck_volatile_reactions(actor, target, move_def);
                    self.target_hit_event(actor, target, critical, false);
                    self.item_hit_event(actor, target, move_def, amount.0, hit_effectiveness);
                    self.self_clears_hazards(actor, move_def);
                    self.self_loses_type(actor, move_def);
                    self.move_self(actor, target, move_def);
                    self.rampage_self_drop(actor, move_def);
                    self.uproar_self_drop(actor, move_def);
                    self.rage_self_drop(actor, move_def, false);
                    self.glaive_rush_self_drop(actor, move_def);
                    self.max_move_effect(actor, move_def);
                    let picked = self.apply_secondary_effects(
                        actor,
                        target,
                        Some(target),
                        move_def,
                        SecondaryTrigger::PerTargetHit,
                        false,
                    );
                    self.z_move_terrain(actor, move_def);
                    self.force_switch_step(actor, target, move_def);
                    // The user's Emergency Exit once the hit's `DamagingHit` reactions (Rough
                    // Skin, Rocky Helmet) and the move's `onAfterHit` are over.
                    let user_hp = self.state.pokemon(actor).map_or(0, |p| p.hp);
                    // Gen 1-4 the move's `onAfterHit` comes before the `DamagingHit` reactions
                    // — and before the thaw among them: none of its moves that has one is a
                    // Fire move. Gen 3-4 Rage builds between the two (§15.19).
                    let after_hit_first = self.rules.hits.after_hit_before_reactions;
                    if after_hit_first {
                        self.after_hit(actor, target, move_def, true);
                    }
                    self.rage_stage(actor, target);
                    // A frozen target thaws after the secondaries (DESIGN.md §22.6): a damaging
                    // Fire hit in `frz`'s `onDamagingHit` (Gen 3+), a `thawsTarget` move or Gen
                    // 1-2's burn secondary in `onAfterMoveSecondary`. A Fire move's burn therefore
                    // never lands on the frozen target it thaws.
                    if crate::engine::thaws_target_on_hit(self.rules, move_def, picked) {
                        self.thaw(target, crate::engine::CureReason::Thawed);
                    }
                    let earlier = outcomes
                        .iter()
                        .map(|outcome| match outcome {
                            HitOutcome::Damaged { amount, .. } => amount.0,
                            _ => 0,
                        })
                        .fold(0u16, u16::saturating_add);
                    self.run_damaging_hit_reactions(
                        actor,
                        target,
                        move_def,
                        (amount.0, earlier),
                        hit_effectiveness,
                    );
                    if !after_hit_first {
                        self.after_hit(actor, target, move_def, true);
                    }
                    self.user_emergency_exit(actor, user_hp);
                    (HitOutcome::Damaged { target, amount }, amount.0, false)
                }
                // Target-directed secondaries do not reach a body behind a Substitute; the
                // user-directed ones do, and so do the move's own `self` effects.
                // Showdown's Substitute condition recoils and drains off the doll at once.
                DamageApplication::Substitute { amount, broken, .. } => {
                    // Gen 1: a hit that breaks the doll neither recoils nor drains.
                    if !(broken && self.rules.conditions.substitute_gen1) {
                        self.apply_substitute_recoil(actor, move_def, u32::from(amount.0));
                        self.apply_drain(actor, target, move_def, amount.0, true);
                    }
                    // `AfterSubDamage`: the holder's Rage (Gen 2), then its ability.
                    self.rage_struck(actor, target, move_def, RageHit::Substitute);
                    self.target_hit_event(actor, target, critical, true);
                    // The move's `onAfterSubDamage` (none needs its user standing), then its self
                    // effects' (Gen 2-4 Rapid Spin); the holder's Air Balloon pops.
                    self.hit_or_sub(actor, target, move_def);
                    self.after_hit(actor, target, move_def, false);
                    self.substitute_pops_balloon(target);
                    self.self_clears_hazards(actor, move_def);
                    self.self_loses_type(actor, move_def);
                    // Gen 1 (`moveHit`'s `doSelf`; pokered's `.nullifyEffect` zeroes the move's
                    // effect as the doll breaks): a hit that breaks it leaves the move's `self`
                    // undone — Hyper Beam owes no recharge — and its user no rage (R-M8-106).
                    let does_self = !(broken && self.rules.conditions.substitute_gen1);
                    if does_self {
                        self.move_self(actor, target, move_def);
                    }
                    self.rampage_self_drop(actor, move_def);
                    self.uproar_self_drop(actor, move_def);
                    if does_self {
                        self.rage_self_drop(actor, move_def, true);
                    }
                    self.glaive_rush_self_drop(actor, move_def);
                    // A Max Move's `self.onHit` too: `selfDrops` skips only a `false` target, not
                    // the `null` a Substitute's hit leaves (DESIGN.md R-M9-20).
                    self.max_move_effect(actor, move_def);
                    self.apply_secondary_effects(
                        actor,
                        target,
                        Some(target),
                        move_def,
                        SecondaryTrigger::PerTargetHit,
                        true,
                    );
                    self.z_move_terrain(actor, move_def);
                    (
                        HitOutcome::SubstituteDamaged {
                            target,
                            amount,
                            broken,
                        },
                        amount.0,
                        true,
                    )
                }
                // Move damage is never refused by the resolution behavior (Magic Guard lets moves
                // through); kept exhaustive so a new handler cannot fall through silently.
                DamageApplication::Prevented { .. } => (HitOutcome::NoEffect { target }, 0, false),
            };
            outcomes.push(outcome);
            // Showdown `hitStepMoveHitLoop`'s `mindBlownRecoil`: after the first hit that struck,
            // before the hit's `Update` (a user it knocks out ends the loop below).
            if matches!(
                outcomes.last(),
                Some(HitOutcome::Damaged { .. } | HitOutcome::SubstituteDamaged { .. })
            ) {
                self.after_move_self_damage(actor, move_def.id);
            }
            // The hit's `Update` (Gen 4+ `hitStepMoveHitLoop` after every hit; Gen 2-3 inside
            // a multi-hit move's loop), its faints still pending.
            if self.rules.update.in_move.after_hit(multi_hit) {
                BattleContext::new(self.state, self.rules, self.trace)
                    .run_update(PendingFaints::Pending);
            }
            // Gen 3 Shell Bell: a multi-hit move's hit heals at its own move end (I47).
            if multi_hit && self.rules.conditions.shell_bell == ShellBellCount::EachHit {
                self.shell_bell(actor, &[dealt]);
            }

            // Hits on HP recoil once the sequence is over (Gen 2+); the rest recoiled above.
            // Struggle's own recoil does not come from the damage (below).
            if dealt > 0
                && !hit_substitute
                && move_def
                    .recoil
                    .is_some_and(|recoil| recoil != Recoil::Struggle)
                && hit_rules.recoil_timing != RecoilTiming::PerHit
            {
                recoil_total += u32::from(dealt);
            }

            let target_down = self.state.pokemon(target).is_none_or(|p| p.is_fainted());
            let actor_down = self.state.pokemon(actor).is_none_or(|p| p.is_fainted());
            // Gen 1 (`subFainted`): breaking the Substitute ends a multi-hit move.
            let broke_gen1_substitute = self.rules.conditions.substitute_gen1
                && matches!(
                    outcomes.last(),
                    Some(HitOutcome::SubstituteDamaged { broken: true, .. })
                );
            if target_down || actor_down || broke_gen1_substitute {
                break;
            }
        }
        self.hit = 1;
        self.hit_count = 1;
        // `faintMessages` at the hit loop's end: the knocked-out Pokémon's Destiny Bond and
        // Grudge (Gen 4+), then a knocked-out gas holder's gas, end here.
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.knocked_out_bonds();
        ctx.knocked_out_gas_ends();
        // Struggle from Gen 4 (`struggleRecoil`, after the hit loop): a quarter of the user's max
        // HP once a hit landed — on the body or a Substitute, whatever it dealt (an Endure at 1
        // HP takes nothing).
        let struggle_landed = move_def.recoil == Some(Recoil::Struggle)
            && outcomes.iter().any(|outcome| {
                matches!(
                    outcome,
                    HitOutcome::Damaged { .. } | HitOutcome::SubstituteDamaged { .. }
                )
            });
        if recoil_total > 0 || struggle_landed {
            let user_hp = self.state.pokemon(actor).map_or(0, |p| p.hp);
            self.apply_recoil(actor, move_def, recoil_total);
            self.note_recoil_knock_out(actor, user_hp);
            self.user_emergency_exit(actor, user_hp);
        }
        // Gen 3 (the gen3 mod's `tryMoveHit`): the target's hit, recoil included, ends with an
        // `Update` — its faints still pending — when the hit returned damage: a single hit on
        // HP, or any hit of a multi-hit move (a single hit on a Substitute returns none).
        if self.rules.update.in_move.after_target()
            && outcomes.iter().any(|outcome| match outcome {
                HitOutcome::Damaged { .. } => true,
                HitOutcome::SubstituteDamaged { .. } => multi_hit,
                _ => false,
            })
        {
            BattleContext::new(self.state, self.rules, self.trace)
                .run_update(PendingFaints::Pending);
        }
        outcomes
    }

    /// The move came from a sleep-usable caller (Showdown `move.sourceEffect`'s `sleepUsable`:
    /// Sleep Talk) — Gen 2-3: the action's own move, at the head of the chain (Sleep Talk's
    /// Metronome's Bone Rush, `HitRules::sleep_check_reads_the_chosen_move`, issues.md I203).
    fn called_by_sleep_usable(&self) -> bool {
        let sleep_usable = |caller: crate::ids::MoveId| {
            self.dex
                .move_by_id(caller)
                .is_some_and(|caller| caller.flags.sleep_usable)
        };
        self.call.is_some_and(|call| {
            if self.rules.hits.sleep_check_reads_the_chosen_move
                && let Some(action_move) = call.action_move
            {
                return sleep_usable(action_move);
            }
            match call.source {
                crate::state::CallSource::Move(caller) => sleep_usable(caller),
                crate::state::CallSource::Ability(_) => false,
            }
        })
    }

    /// Showdown `onSourceTryPrimaryHit` of the user's ability (Gulp Missile's Surf, priority 0:
    /// before the gem's 1): a Cramorant that surfs comes up with its catch.
    fn source_try_primary_hit(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) {
        let Some(handler) = self.state.pokemon(actor).and_then(|pokemon| {
            crate::engine::ability_handler(self.state, self.rules, pokemon, |b| {
                b.source_try_primary_hit
            })
        }) else {
            return;
        };
        handler(
            &mut BattleContext::new(self.state, self.rules, self.trace),
            actor,
            target,
            move_def,
        );
    }

    /// Showdown's gem (`onSourceTryPrimaryHit`, `useItem`): a damaging move of the gem's type
    /// about to strike another Pokémon uses the user's gem up (`-enditem … [from] gem`); the
    /// move keeps its boost (`gem`). A gem may go even at 0 HP (`useItem`), but the user must be
    /// on the field. The Pledges never use one (`move.flags['pledgecombo']`, DESIGN.md R-M8-62).
    fn use_gem(&mut self, actor: PokemonId, target: PokemonId, move_def: &ResolvedMove) {
        let Some(item) = crate::engine::held_item_id(self.state, self.rules, actor) else {
            return;
        };
        if target == actor
            || move_def.category == crate::dex::MoveCategory::Status
            || move_def.flags.pledge_combo
            || crate::engine::item_behavior(self.rules, item).gem != Some(move_def.move_type)
            || self
                .state
                .active_position(actor, &self.rules.topology)
                .is_none()
        {
            return;
        }
        if BattleContext::new(self.state, self.rules, self.trace)
            .consume_item(actor)
            .is_some()
        {
            self.gem = Some(item);
        }
    }

    /// The target's focus item for the move (`ItemBehavior::holds_for_the_move`), when it has one
    /// and the move has not armed it yet.
    fn unarmed_focus_item(
        &self,
        target: PokemonId,
    ) -> Option<(crate::ids::ItemId, crate::effects::items::HoldsForTheMove)> {
        if self.focus_items.iter().any(|armed| armed.holder == target) {
            return None;
        }
        let (item, behavior) = crate::engine::held_item(self.state, self.rules, target)?;
        Some((item, behavior.holds_for_the_move?))
    }

    fn arm(
        &mut self,
        holder: PokemonId,
        item: crate::ids::ItemId,
        how: crate::effects::items::HoldsForTheMove,
    ) {
        self.focus_items.push(ArmedFocusItem {
            holder,
            item,
            how,
            held: false,
        });
    }

    /// Gen 4 Focus Sash (`onTryHit`): the move reached another Pokémon holding one at full HP —
    /// the sash holds for the rest of the move.
    fn arm_focus_item(&mut self, actor: PokemonId, target: PokemonId) {
        if target != actor
            && self
                .state
                .pokemon(target)
                .is_some_and(|pokemon| pokemon.hp == pokemon.max_hp)
            && let Some((item, how @ crate::effects::items::HoldsForTheMove::FromFullHp)) =
                self.unarmed_focus_item(target)
        {
            self.arm(target, item, how);
        }
    }

    /// Gen 3-4 Focus Band (`ConditionRules::focus_band_lasts_the_move`, issues.md I38): every hit
    /// of the move on the holder rolls — a Substitute's included — until one succeeds; from then
    /// on the band holds for the rest of the move.
    fn roll_focus_band(&mut self, target: PokemonId) {
        if !self.rules.conditions.focus_band_lasts_the_move {
            return;
        }
        let Some((item, how @ crate::effects::items::HoldsForTheMove::OnChance)) =
            self.unarmed_focus_item(target)
        else {
            return;
        };
        let chance = self.rules.conditions.focus_band_chance;
        if self.state.rng.roll_ratio_for(
            RandomRequestKind::FocusBand,
            chance.numerator,
            chance.denominator,
        ) {
            self.arm(target, item, how);
        }
    }

    /// An armed focus item's hold (Gen 4 Focus Sash's condition `onDamage`, priority 0: ahead of
    /// Endure and False Swipe; Gen 3-4 Focus Band's `focusBanded`): a hit that would knock its
    /// holder out leaves it at 1 — not a hit a Substitute takes. `amount` as it now lands.
    fn focus_item_holds(
        &mut self,
        target: PokemonId,
        amount: DamageAmount,
        move_def: &ResolvedMove,
    ) -> DamageAmount {
        let Some(index) = self
            .focus_items
            .iter()
            .position(|armed| armed.holder == target)
        else {
            return amount;
        };
        let Some(pokemon) = self.state.pokemon(target) else {
            return amount;
        };
        let behind_substitute = !move_def.flags.bypass_substitute
            && pokemon
                .volatiles
                .contains(crate::state::VolatileConditionKind::Substitute);
        if behind_substitute || pokemon.hp == 0 || amount.0 < pokemon.hp {
            return amount;
        }
        let held = DamageAmount(pokemon.hp - 1);
        let armed = &mut self.focus_items[index];
        armed.held = true;
        let item = armed.item;
        if armed.how == crate::effects::items::HoldsForTheMove::OnChance {
            self.trace.push(TraceEvent::ItemActivated {
                holder: target,
                item,
            });
        }
        self.trace.push(TraceEvent::DamageReduced {
            target,
            requested: amount.0,
            applied: held.0,
            by: EffectSource::Item {
                pokemon: target,
                item,
            },
        });
        held
    }

    /// The user's Emergency Exit check against its HP before a step that may have hurt it.
    fn user_emergency_exit(&mut self, actor: PokemonId, hp_before: u16) {
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.emergency_exit(actor, hp_before);
    }

    /// A spread move's hit on every target that passed the gate, stage by stage as Showdown's
    /// `spreadMoveHit` runs it (DESIGN.md §15.3): Substitutes first (their damage, recoil and
    /// drain), then every other target's damage (critical, roll), then the damage lands target
    /// by target with its drain, then each target's secondaries (behind a Substitute only the
    /// user-directed ones), then each damaged target's thaw and reactions; recoil from the HP
    /// hits comes once at the end (Gen 2+). Outcomes are returned per target, in target order.
    pub(super) fn apply_spread_hit(
        &mut self,
        actor: PokemonId,
        targets: &[(PokemonId, Fraction)],
        move_def: &ResolvedMove,
        target_count: usize,
    ) -> Vec<(PokemonId, HitOutcome)> {
        let hit_rules = self.rules.hits;
        let source = EffectSource::Move {
            pokemon: actor,
            move_id: move_def.id,
        };
        let apply = |pipeline: &mut Self, target: PokemonId, amount: DamageAmount| {
            pipeline.roll_focus_band(target);
            let amount = pipeline.focus_item_holds(target, amount, move_def);
            let mut ctx = BattleContext::new(pipeline.state, pipeline.rules, pipeline.trace);
            ctx.apply_damage(crate::engine::DamageRequest {
                target,
                amount,
                kind: crate::engine::DamageKind::Move,
                source,
                flags: crate::engine::DamageFlags {
                    bypass_substitute: move_def.flags.bypass_substitute,
                    spare_one_hp: move_def.behavior.spares_target,
                    instafaint: false,
                    single_strike: strikes_once(move_def),
                    future_move: move_def.flags.future_move,
                },
            })
        };
        let behind_substitute = |pipeline: &Self, target: PokemonId| {
            !move_def.flags.bypass_substitute
                && pipeline.state.pokemon(target).is_some_and(|p| {
                    p.volatiles
                        .contains(crate::state::VolatileConditionKind::Substitute)
                })
        };
        let mut outcomes: Vec<Option<HitOutcome>> = vec![None; targets.len()];

        // Showdown `tryPrimaryHitEvent`: the user's gem goes at the first target, before any
        // Substitute takes its hit.
        for &(target, _) in targets {
            self.use_gem(actor, target, move_def);
            self.source_try_primary_hit(actor, target, move_def);
            self.arm_focus_item(actor, target);
        }
        // 0. Substitutes (Showdown `tryPrimaryHitEvent`): each takes its hit, recoil and drain.
        for (index, &(target, effectiveness)) in targets.iter().enumerate() {
            if !behind_substitute(self, target) {
                continue;
            }
            let breakdown =
                self.resolve_standard_damage(actor, target, move_def, target_count, effectiveness);
            outcomes[index] = Some(match apply(self, target, breakdown.final_damage) {
                DamageApplication::Substitute { amount, broken, .. } => {
                    self.apply_substitute_recoil(actor, move_def, u32::from(amount.0));
                    self.apply_drain(actor, target, move_def, amount.0, true);
                    self.rage_struck(actor, target, move_def, RageHit::Substitute);
                    self.target_hit_event(actor, target, breakdown.critical.0, true);
                    // The move's `onAfterSubDamage` (Core Enforcer, Flame Burst); the holder's
                    // Air Balloon pops.
                    self.hit_or_sub(actor, target, move_def);
                    self.after_hit(actor, target, move_def, false);
                    self.substitute_pops_balloon(target);
                    HitOutcome::SubstituteDamaged {
                        target,
                        amount,
                        broken,
                    }
                }
                DamageApplication::Hp { amount, .. } => HitOutcome::Damaged { target, amount },
                DamageApplication::Prevented { .. } => HitOutcome::NoEffect { target },
            });
        }

        // 1. Every other target's damage (`getSpreadDamage`), before any of it lands.
        let damages: Vec<Option<crate::engine::DamageBreakdown>> = targets
            .iter()
            .enumerate()
            .map(|(index, &(target, effectiveness))| {
                outcomes[index].is_none().then(|| {
                    self.resolve_standard_damage(
                        actor,
                        target,
                        move_def,
                        target_count,
                        effectiveness,
                    )
                })
            })
            .collect();

        // 2. The damage lands target by target (`spreadDamage`), each with its drain.
        let mut dealt_total: u32 = 0;
        for (index, &(target, _)) in targets.iter().enumerate() {
            let Some(breakdown) = damages[index] else {
                continue;
            };
            outcomes[index] = Some(match apply(self, target, breakdown.final_damage) {
                DamageApplication::Hp { amount, .. } => {
                    if hit_rules.recoil_timing == RecoilTiming::PerHit && amount.0 > 0 {
                        self.apply_recoil(actor, move_def, u32::from(amount.0));
                    } else {
                        dealt_total += u32::from(amount.0);
                    }
                    self.apply_drain(actor, target, move_def, amount.0, false);
                    HitOutcome::Damaged { target, amount }
                }
                DamageApplication::Substitute { amount, broken, .. } => {
                    HitOutcome::SubstituteDamaged {
                        target,
                        amount,
                        broken,
                    }
                }
                DamageApplication::Prevented { .. } => HitOutcome::NoEffect { target },
            });
        }

        // 3. The move's own `onHit` on each body it struck (`runMoveEffects`): Core Enforcer,
        //    Thousand Waves; then the body's (`runEvent('Hit')`: Anger Point).
        for (index, &(target, effectiveness)) in targets.iter().enumerate() {
            if let Some(HitOutcome::Damaged { amount, .. }) = outcomes[index] {
                self.hit_or_sub(actor, target, move_def);
                self.hit_traps(actor, target, move_def);
                self.hit_takes_item(actor, target, move_def);
                self.struck_volatile_reactions(actor, target, move_def);
                let critical = damages[index].is_some_and(|breakdown| breakdown.critical.0);
                self.target_hit_event(actor, target, critical, false);
                self.item_hit_event(actor, target, move_def, amount.0, effectiveness);
            }
        }

        // 4. The move's `self` effects (`selfDrops`) for each target it reached, a Substitute's
        // too (only a `false` target is skipped): its own `self` once, a Max Move's for each.
        for (index, outcome) in outcomes.iter().enumerate() {
            if matches!(
                outcome,
                Some(HitOutcome::Damaged { .. } | HitOutcome::SubstituteDamaged { .. })
            ) {
                self.move_self(actor, targets[index].0, move_def);
                self.max_move_effect(actor, move_def);
            }
        }

        // 5. Secondaries, target by target.
        let mut picked = vec![None; targets.len()];
        for (index, &(target, _)) in targets.iter().enumerate() {
            let behind = match outcomes[index] {
                Some(HitOutcome::Damaged { .. }) => false,
                Some(HitOutcome::SubstituteDamaged { .. }) => true,
                _ => continue,
            };
            picked[index] = self.apply_secondary_effects(
                actor,
                target,
                Some(target),
                move_def,
                SecondaryTrigger::PerTargetHit,
                behind,
            );
            self.z_move_terrain(actor, move_def);
        }

        // 6b. `forceSwitch`, then 7. `DamagingHit`: a frozen target thaws, then its reactions
        //     (Rough Skin …).
        for (index, &(target, _)) in targets.iter().enumerate() {
            if matches!(outcomes[index], Some(HitOutcome::Damaged { .. })) {
                self.force_switch_step(actor, target, move_def);
            }
        }
        // Gen 3-4 Rage builds on every damaged target first (Showdown's `DamagingHit` runs the
        // whole event left to right, the cartridge's step at its head, §15.19).
        for (index, &(target, _)) in targets.iter().enumerate() {
            if matches!(outcomes[index], Some(HitOutcome::Damaged { .. })) {
                self.rage_stage(actor, target);
            }
        }
        let user_hp = self.state.pokemon(actor).map_or(0, |p| p.hp);
        for (index, &(target, effectiveness)) in targets.iter().enumerate() {
            let Some(HitOutcome::Damaged { amount, .. }) = outcomes[index] else {
                continue;
            };
            if crate::engine::thaws_target_on_hit(self.rules, move_def, picked[index]) {
                self.thaw(target, crate::engine::CureReason::Thawed);
            }
            self.run_damaging_hit_reactions(actor, target, move_def, (amount.0, 0), effectiveness);
        }
        // 8. The move's `onAfterHit` on every damaged target, once all have reacted (no spread
        //    move of Gen 1-4, where it would come first, has one).
        for (index, &(target, _)) in targets.iter().enumerate() {
            if matches!(outcomes[index], Some(HitOutcome::Damaged { .. })) {
                self.after_hit(actor, target, move_def, true);
            }
        }
        self.user_emergency_exit(actor, user_hp);
        // `hitStepMoveHitLoop`'s `mindBlownRecoil`, before the hit's `Update` as for a single
        // target: a berry the spread hit brought into reach is eaten after it (R-M8-112).
        if outcomes.iter().any(|outcome| {
            matches!(
                outcome,
                Some(HitOutcome::Damaged { .. } | HitOutcome::SubstituteDamaged { .. })
            )
        }) {
            self.after_move_self_damage(actor, move_def.id);
        }
        // The hit's `Update` (`hitStepMoveHitLoop`: a spread move hits once, every target).
        if self.rules.update.in_move.after_hit(false) {
            BattleContext::new(self.state, self.rules, self.trace)
                .run_update(PendingFaints::Pending);
        }
        // `faintMessages` at the hit loop's end: the knocked-out Pokémon's Destiny Bond and
        // Grudge (Gen 4+), then a knocked-out gas holder's gas, end here.
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.knocked_out_bonds();
        ctx.knocked_out_gas_ends();

        if dealt_total > 0 && move_def.recoil.is_some() {
            let user_hp = self.state.pokemon(actor).map_or(0, |p| p.hp);
            self.apply_recoil(actor, move_def, dealt_total);
            self.note_recoil_knock_out(actor, user_hp);
            self.user_emergency_exit(actor, user_hp);
        }
        targets
            .iter()
            .zip(outcomes)
            .filter_map(|(&(target, _), outcome)| outcome.map(|outcome| (target, outcome)))
            .collect()
    }

    /// The move's `onAfterHit` (`MoveBehavior::after_hit`, `takes_item`) after a hit on
    /// `target`'s HP (`on_body`) and the `DamagingHit` reactions — Gen 1-4 before them
    /// (`HitRules::after_hit_before_reactions`) —, or its `onAfterSubDamage` as `target`'s
    /// Substitute took the hit (`after_hit` alone): only while the user stands and unless Sheer
    /// Force dropped the move's secondaries. A hazard is laid with the target as its source
    /// (Showdown's `event.target`).
    fn after_hit(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        on_body: bool,
    ) {
        if move_def.flags.sheer_force || self.state.pokemon(actor).is_none_or(|p| p.is_fainted()) {
            return;
        }
        if on_body && let Some(take) = move_def.behavior.takes_item {
            let used_gem = self.gem.is_some();
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            match take {
                crate::effects::moves::ItemTake::KnockOff => ctx.knock_off(actor, target),
                crate::effects::moves::ItemTake::Steal => {
                    ctx.steal_item(actor, target, move_def.id, used_gem);
                }
            }
        }
        let Some(effect) = move_def.behavior.after_hit else {
            return;
        };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        match effect {
            crate::effects::moves::AfterHitEffect::ClearHazards => ctx.spin_away(actor),
            crate::effects::moves::AfterHitEffect::LayHazard(kind) => {
                if let Some(side) = ctx.side_of(actor) {
                    ctx.lay_hazard(side.other(), kind, target);
                }
            }
            crate::effects::moves::AfterHitEffect::ClearTerrain => {
                ctx.clear_terrain();
            }
        }
    }

    /// A Max Move's own `self.onHit` (DESIGN.md §44.5), which runs only while its user is
    /// Dynamaxed: the side's boost, the foes' drop, a weather or a terrain — on a body it struck
    /// or a Substitute it hit (R-M9-20), among the move's `self` effects (`selfDrops`), before
    /// the secondaries and the hit's `DamagingHit` (R-M8-96).
    fn max_move_effect(&mut self, actor: PokemonId, move_def: &ResolvedMove) {
        use crate::effects::moves::MaxMoveEffect;
        let Some(effect) = move_def.behavior.max_move_effect else {
            return;
        };
        let dynamaxed = self.state.pokemon(actor).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .contains(crate::state::VolatileConditionKind::Dynamax)
        });
        if !dynamaxed {
            return;
        }
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        match effect {
            MaxMoveEffect::TeamBoost(stages) => {
                for pokemon in ctx.allies_and_self(actor) {
                    for (stat, delta) in stages {
                        ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
                            target: pokemon,
                            stat: *stat,
                            stages: *delta,
                            source: crate::engine::EffectSource::Move {
                                pokemon: actor,
                                move_id: move_def.id,
                            },
                            cause: crate::engine::StatChangeCause::Move,
                        });
                    }
                }
            }
            MaxMoveEffect::FoeDrop(stages) => {
                for pokemon in ctx.active_foes(actor) {
                    for (stat, delta) in stages {
                        ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
                            target: pokemon,
                            stat: *stat,
                            stages: *delta,
                            source: crate::engine::EffectSource::Move {
                                pokemon: actor,
                                move_id: move_def.id,
                            },
                            cause: crate::engine::StatChangeCause::Move,
                        });
                    }
                }
            }
            MaxMoveEffect::Weather(kind) => {
                ctx.set_weather_by_move(kind, actor);
            }
            MaxMoveEffect::Terrain(kind) => {
                ctx.set_terrain(kind, actor);
            }
        }
    }

    /// A Z-Move's own terrain (Genesis Supernova's `secondary.self.onHit` at 100 %): the move
    /// lays it on the field among its secondaries, before the hit's `DamagingHit` — an Innards
    /// Out that knocks the user out comes too late to stop it (DESIGN.md §44.4, R-M8-96). A
    /// Substitute's hit too: Showdown's `secondaries` walks the `null` it leaves (only `false`
    /// is skipped), and a `self` secondary needs no target (R-M8-137). Sheer Force drops it with
    /// the secondaries.
    fn z_move_terrain(&mut self, actor: PokemonId, move_def: &ResolvedMove) {
        let Some(terrain) = move_def.behavior.z_terrain else {
            return;
        };
        if move_def.flags.sheer_force {
            return;
        }
        let kind = match terrain {
            crate::effects::moves::ZTerrain::Psychic => crate::state::TerrainKind::Psychic,
        };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.set_terrain(kind, actor);
    }

    /// Incinerate's, Bug Bite / Pluck's and Fling's `onHit` on a body it struck
    /// (`MoveBehavior::burns_item`, `steals_berry`, `fling`).
    fn hit_takes_item(&mut self, actor: PokemonId, target: PokemonId, move_def: &ResolvedMove) {
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        if move_def.behavior.burns_item {
            ctx.burn_item(actor, target);
        }
        if move_def.behavior.steals_berry {
            ctx.steal_and_eat_berry(actor, target, move_def.id);
        }
        if move_def.behavior.fling {
            ctx.fling_hits(actor, target);
        }
    }

    /// Gen 2-4 Rapid Spin's `self.onHit`, as the move's self effects land on a hit (Showdown
    /// `selfDrops`, before the secondaries and the target's reactions; a Substitute's hit too).
    /// Thousand Waves / Jaw Lock's `onHit` on the target's body (`MoveBehavior::hit_traps`):
    /// Thousand Waves traps the target, Jaw Lock the user by the target and then the target by
    /// the user — each only if both are still standing (`addVolatile` with a linked status).
    fn hit_traps(&mut self, actor: PokemonId, target: PokemonId, move_def: &ResolvedMove) {
        let Some(trap) = move_def.behavior.hit_traps else {
            return;
        };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        if trap == crate::effects::moves::HitTrap::Mutual {
            let _ = ctx.trap(actor, target);
        }
        let _ = ctx.trap(target, actor);
    }

    /// The struck body's turn-start volatiles (their `onHit`, the `Hit` event of
    /// `runMoveEffects`; DESIGN.md §15.8): a damaging hit costs Focus Punch its focus, a contact
    /// hit burns the attacker of a Beak Blast user, a foe's physical hit arms Shell Trap and
    /// sends its user's action to the front, another Pokémon's hit builds Rage (§15.19).
    fn struck_volatile_reactions(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) {
        use crate::dex::MoveCategory;
        use crate::state::VolatileConditionKind;
        let Some(holder) = self.state.pokemon(target) else {
            return;
        };
        let focusing = holder.volatiles.contains(VolatileConditionKind::FocusPunch);
        let beak = holder.volatiles.contains(VolatileConditionKind::BeakBlast);
        let trap = holder.volatiles.contains(VolatileConditionKind::ShellTrap);
        // Only a damaging move's hit on the body gets here (`move.category !== 'Status'`).
        if focusing && let Some(holder) = self.state.pokemon_mut(target) {
            holder.volatiles.lose_focus();
        }
        if beak && crate::engine::move_makes_contact(self.state, self.rules, actor, move_def) {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            ctx.try_set_status(
                actor,
                crate::state::NonVolatileStatus::Burn,
                crate::engine::EffectSource::Move {
                    pokemon: target,
                    move_id: crate::ids::MoveId::BEAK_BLAST,
                },
            );
        }
        let foe =
            self.state.pokemon(actor).map(|p| p.side) != self.state.pokemon(target).map(|p| p.side);
        if trap
            && foe
            && move_def.category == MoveCategory::Physical
            && self
                .state
                .pokemon_mut(target)
                .is_some_and(|holder| holder.volatiles.arm_shell_trap())
            && self.state.will_move_with(target).is_some()
        {
            self.effects.arm(target);
        }
        self.rage_struck(actor, target, move_def, RageHit::Body);
    }

    /// The target's ability on a hit (`runEvent('Hit')`, `AbilityBehavior::on_hit`) or on its
    /// Substitute's (`runEvent('AfterSubDamage')`, `AbilityBehavior::after_sub_damage`).
    fn target_hit_event(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        critical: bool,
        substitute: bool,
    ) {
        let Some(ability) = self
            .state
            .pokemon(target)
            .and_then(|p| crate::engine::ability_running(self.state, self.rules, p))
        else {
            return;
        };
        let behavior = crate::engine::ability_behavior(self.rules, ability);
        let handler = if substitute {
            behavior.after_sub_damage
        } else {
            behavior.on_hit
        };
        if let Some(handler) = handler {
            handler(
                &mut BattleContext::new(self.state, self.rules, self.trace),
                target,
                actor,
                critical,
            );
        }
    }

    /// `MoveBehavior::hit_or_sub`: the move's `onHit` on the target's body, or its
    /// `onAfterSubDamage` when a Substitute took the hit.
    fn hit_or_sub(&mut self, actor: PokemonId, target: PokemonId, move_def: &ResolvedMove) {
        let Some(effect) = move_def.behavior.hit_or_sub else {
            return;
        };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        match effect {
            crate::effects::moves::HitOrSubEffect::ClearTerrain => {
                ctx.clear_terrain();
            }
            crate::effects::moves::HitOrSubEffect::SuppressAbilityOfMoved => {
                ctx.core_enforcer(target)
            }
            crate::effects::moves::HitOrSubEffect::SplashAdjacentAllies => {
                ctx.flame_burst(actor, target)
            }
        }
    }

    /// The holder's Air Balloon on a move's hit its Substitute took (`onAfterSubDamage`): it
    /// pops — on one target or on each of a spread move's (R-M10-50).
    fn substitute_pops_balloon(&mut self, target: PokemonId) {
        if crate::engine::held_item(self.state, self.rules, target)
            .is_some_and(|(_, item)| item.pops_on_substitute_hit)
        {
            BattleContext::new(self.state, self.rules, self.trace).pop_air_balloon(target);
        }
    }

    /// The holder's item in the same `runEvent('Hit')`, after its ability's (Showdown orders an
    /// ability's handler before an item's): Enigma Berry's `onHit`, Sticky Barb's (Gen 5+:
    /// `ItemTransferRules::sticky_barb_after_contact_abilities`).
    fn item_hit_event(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        damage: u16,
        type_effectiveness: Fraction,
    ) {
        if !self.holder_item_answers(move_def) {
            return;
        }
        let Some((_, behavior)) = crate::engine::held_item(self.state, self.rules, target) else {
            return;
        };
        let sticky = behavior.sticks_to_contact
            && !self
                .rules
                .conditions
                .items
                .sticky_barb_after_contact_abilities;
        let handler = behavior.on_hit;
        if handler.is_none() && !sticky {
            return;
        }
        let hit = crate::engine::DamagingHit {
            holder: target,
            attacker: actor,
            move_id: move_def.id,
            move_type: move_def.move_type,
            category: move_def.category,
            contact: crate::engine::move_makes_contact(self.state, self.rules, actor, move_def),
            padded: move_def.flags.contact
                && !crate::engine::move_makes_contact(self.state, self.rules, actor, move_def),
            wind: move_def.flags.wind,
            damage,
            earlier_damage: 0,
            super_effective: {
                let effectiveness = self.hit_effectiveness(target, type_effectiveness);
                effectiveness.numerator > effectiveness.denominator
            },
            fixed_damage: move_def.fixed_damage.is_some(),
        };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        if let Some(handler) = handler {
            handler(&mut ctx, hit);
        }
        if sticky {
            ctx.sticky_barb_transfer(hit);
        }
    }

    /// Shell Bell's heal off the damage `dealt` (one entry per heal: Gen 3's hits are healed one
    /// by one), each an eighth, at least 1 (Showdown `heal`, pokeplatinum `BattleSystem_Divide`),
    /// while the holder stands below its max HP.
    pub(super) fn shell_bell(&mut self, actor: PokemonId, dealt: &[u16]) {
        let Some((item, share)) = crate::engine::held_item(self.state, self.rules, actor)
            .and_then(|(item, behavior)| behavior.heals_off_damage_dealt.map(|s| (item, s)))
        else {
            return;
        };
        for &damage in dealt {
            if damage == 0 || self.state.pokemon(actor).is_none_or(|p| p.is_fainted()) {
                continue;
            }
            let amount = share.apply_u16(damage).max(1);
            BattleContext::new(self.state, self.rules, self.trace).apply_heal(
                crate::engine::HealRequest {
                    target: actor,
                    amount,
                    source: crate::engine::EffectSource::Item {
                        pokemon: actor,
                        item,
                    },
                    direct: false,
                },
            );
        }
    }

    fn self_clears_hazards(&mut self, actor: PokemonId, move_def: &ResolvedMove) {
        if move_def.behavior.self_clears_hazards {
            BattleContext::new(self.state, self.rules, self.trace).spin_away(actor);
        }
    }

    /// Burn Up, Double Shock's `self.onHit`: the user's types without the added one
    /// (`getTypes(true)`, Roost's view included) with this type turned into `???` (`setType`,
    /// which Arceus refuses).
    fn self_loses_type(&mut self, actor: PokemonId, move_def: &ResolvedMove) {
        let Some(lost) = move_def.behavior.burns_up else {
            return;
        };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        let Some(types) = ctx.types(actor) else {
            return;
        };
        let mut left = types.without_added().map(|t| {
            if t == lost {
                crate::dex::PokemonType::Typeless
            } else {
                t
            }
        });
        let types = match (left.next(), left.next()) {
            (Some(primary), Some(secondary)) => {
                crate::state::PokemonTypes::dual(primary, secondary)
            }
            (Some(primary), None) => crate::state::PokemonTypes::mono(primary),
            (None, _) => return,
        };
        ctx.set_type(
            actor,
            types,
            crate::engine::EffectSource::Move {
                pokemon: actor,
                move_id: move_def.id,
            },
        );
    }

    /// The target's ability and item react to a hit that reached its HP (Rough Skin, Rocky
    /// Helmet …), ability first (Showdown `onDamagingHitOrder`). Runs even on the hit that
    /// fainted the holder; each reaction checks the attacker is still standing.
    /// Showdown `forceSwitch` (Dragon Tail, Circle Throw): after the secondaries and before
    /// `DamagingHit`, a target that took the hit is ordered out if it and the user still stand
    /// — so a user its target's Rough Skin then knocks out still drags it.
    fn force_switch_step(&mut self, actor: PokemonId, target: PokemonId, move_def: &ResolvedMove) {
        if !move_def.force_switch
            || move_def.primary_effect != PrimaryEffect::Damage
            || self.state.pokemon(actor).is_none_or(|p| p.is_fainted())
        {
            return;
        }
        let source = EffectSource::Move {
            pokemon: actor,
            move_id: move_def.id,
        };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.request_drag_out(target, source);
    }

    /// Whether the hit target's item answers the hit (its `Hit` / `DamagingHit` handlers):
    /// not for a Gen 4 foreseen strike (`ItemTransferRules::foreseen_strike_skips_hit_items`).
    fn holder_item_answers(&self, move_def: &ResolvedMove) -> bool {
        !(move_def.flags.future_move && self.rules.conditions.items.foreseen_strike_skips_hit_items)
    }

    /// `damage`: this hit's and the move's earlier hits' on the holder.
    pub(super) fn run_damaging_hit_reactions(
        &mut self,
        attacker: PokemonId,
        holder: PokemonId,
        move_def: &ResolvedMove,
        (damage, earlier_damage): (u16, u16),
        type_effectiveness: Fraction,
    ) {
        let (ability, handlers, held_item) = match self.state.pokemon(holder) {
            Some(pokemon) => (
                crate::engine::raw_ability(pokemon),
                crate::engine::ability_handlers(self.state, self.rules, pokemon),
                crate::engine::held_item_id(self.state, self.rules, holder),
            ),
            None => return,
        };
        let hit = crate::engine::DamagingHit {
            holder,
            attacker,
            move_id: move_def.id,
            move_type: move_def.move_type,
            category: move_def.category,
            contact: crate::engine::move_makes_contact(self.state, self.rules, attacker, move_def),
            padded: move_def.flags.contact
                && !crate::engine::move_makes_contact(self.state, self.rules, attacker, move_def),
            wind: move_def.flags.wind,
            damage,
            earlier_damage,
            super_effective: {
                let effectiveness = self.hit_effectiveness(holder, type_effectiveness);
                effectiveness.numerator > effectiveness.denominator
            },
            fixed_damage: move_def.fixed_damage.is_some(),
        };
        let reactors = crate::engine::chance_reactors(self.state, self.rules, attacker, holder);
        // Static / Flame Body / Poison Point (ability order 1, so before any item): the chance
        // is rolled whenever the hit qualifies, whether or not the status can then take
        // (Showdown `randomChance` before `trySetStatus`).
        if let Some(status) = handlers.contact_status {
            let needs_damage = self.rules.hits.contact_reaction_needs_damage;
            let attacker_down = self.rules.hits.contact_status_needs_living_attacker
                && self.state.pokemon(attacker).is_none_or(|p| p.is_fainted());
            if hit.contact && !(needs_damage && damage == 0) && !attacker_down {
                let chance = self.rules.status.lifecycle.contact_status_chance;
                let triggered = self.state.rng.roll_ratio_for(
                    RandomRequestKind::ContactReaction,
                    chance.numerator,
                    chance.denominator,
                );
                self.trace.push(TraceEvent::ContactReactionChecked {
                    holder,
                    attacker,
                    ability,
                    triggered,
                });
                if triggered {
                    BattleContext::new(self.state, self.rules, self.trace)
                        .inflict_status_by_ability(holder, attacker, status, ability);
                }
            }
        }
        // `(order, from the item, handler)`.
        let mut reactions: Vec<(i16, bool, crate::engine::DamagingHitReactionHandler)> = Vec::new();
        if let Some(ordered) = handlers.damaging_hit_reaction {
            reactions.push((ordered.priority, false, ordered.handler));
        }
        if let Some(item) = held_item
            && self.holder_item_answers(move_def)
            && let Some(ordered) =
                crate::engine::item_behavior(self.rules, item).damaging_hit_reaction
        {
            reactions.push((ordered.priority, true, ordered.handler));
        }
        reactions.sort_by_key(|(priority, _, _)| *priority);
        // The unordered ones as Showdown collects them: the holder's ability's chance reactions
        // before its item's (Cursed Body disables before a Jaboca Berry knocks the attacker
        // out, R-M11-89), the attacker's own last.
        let mut holder_chances = Some(reactors);
        for (priority, from_item, handler) in reactions {
            if from_item
                && priority == crate::effects::abilities::UNORDERED_REACTION
                && let Some(reactors) = holder_chances.take()
            {
                self.run_holder_chance_reactions(attacker, holder, move_def, damage, reactors);
            }
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            handler(&mut ctx, hit);
        }
        if let Some(reactors) = holder_chances {
            self.run_holder_chance_reactions(attacker, holder, move_def, damage, reactors);
        }
        self.run_attacker_chance_reaction(attacker, holder, move_def, reactors);
        // Gen 4 Sticky Barb, after the contact abilities (issues.md I100).
        if self
            .rules
            .conditions
            .items
            .sticky_barb_after_contact_abilities
            && self.holder_item_answers(move_def)
            && crate::engine::held_item(self.state, self.rules, holder)
                .is_some_and(|(_, behavior)| behavior.sticks_to_contact)
        {
            BattleContext::new(self.state, self.rules, self.trace).sticky_barb_transfer(hit);
        }
    }

    /// The drain heal off `dealt` damage: Showdown heals off a Substitute with ceil, off HP with
    /// the generation's rounding.
    /// A hit that heals its target instead (`HealInstead`): Showdown `moveData.heal` for
    /// Present — a full-HP target fails the move (`-fail|target|heal`) — and Pollen Puff's
    /// `onHit` heal of an ally.
    fn heal_instead(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        heal: crate::engine::HealInstead,
    ) -> HitOutcome {
        // Before Gen 6 a Substitute takes no heal: it asks for damage, and there is none.
        let behind_substitute = self
            .state
            .pokemon(target)
            .is_some_and(|p| p.volatiles.substitute_hp().is_some());
        if behind_substitute && !self.rules.conditions.substitute_honors_infiltrates {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id: move_def.id,
                reason: crate::engine::MoveFailReason::NoEffect,
            });
            return HitOutcome::NoEffect { target };
        }
        // Showdown `moveData.heal` / Pollen Puff: a share of the target's `baseMaxhp`.
        let max_hp = self.state.pokemon(target).map_or(0, |p| p.base_max_hp());
        let (fraction, round_half_up) = match heal {
            crate::engine::HealInstead::Drawn(fraction) => {
                (fraction, self.rules.conditions.heal_rounds_half_up)
            }
            crate::engine::HealInstead::Ally(fraction) => (fraction, false),
        };
        let numerator = u32::from(max_hp) * u32::from(fraction.numerator);
        let denominator = u32::from(fraction.denominator.max(1));
        let amount = if round_half_up {
            (numerator + denominator / 2) / denominator
        } else {
            numerator / denominator
        };
        let result = {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            ctx.apply_heal(crate::engine::HealRequest {
                target,
                amount: amount.clamp(1, u32::from(u16::MAX)) as u16,
                source: EffectSource::Move {
                    pokemon: actor,
                    move_id: move_def.id,
                },
                direct: false,
            })
        };
        if matches!(result, crate::engine::HealResult::Applied { .. }) {
            // The heal is the hit's: the `Hit` event follows (Gen 5+ Rage).
            self.rage_struck(actor, target, move_def, RageHit::Heal);
            return HitOutcome::Effect { target };
        }
        let fails = match heal {
            crate::engine::HealInstead::Drawn(_) => true,
            crate::engine::HealInstead::Ally(_) => self.rules.hits.ally_heal_fails_at_full_hp,
        };
        if fails {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id: move_def.id,
                reason: crate::engine::MoveFailReason::NoEffect,
            });
            HitOutcome::NoEffect { target }
        } else {
            HitOutcome::Effect { target }
        }
    }

    fn apply_drain(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        dealt: u16,
        off_substitute: bool,
    ) {
        let Some(drain) = move_def.drain else {
            return;
        };
        if dealt == 0 {
            return;
        }
        let hit_rules = self.rules.hits;
        // Showdown's `substitute` drains `ceil(damage × drain)` off the doll; Gen 1's rounds
        // like any drain (floor, at least 1).
        let amount = if off_substitute && !self.rules.conditions.substitute_gen1 {
            (u32::from(dealt) * u32::from(drain.numerator))
                .div_ceil(u32::from(drain.denominator.max(1)))
        } else {
            hit_rules
                .drain_rounding
                .apply(u32::from(dealt), drain)
                .max(u32::from(hit_rules.drain_minimum))
        };
        if amount > 0 {
            // Gen 1: a drain can be countered (`battle.lastDamage = amount`).
            if self.rules.conditions.retaliation.gen1_last_damage {
                self.state.last_damage = amount.min(u32::from(u16::MAX)) as u16;
            }
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            ctx.drain_heal(
                crate::engine::HealRequest {
                    target: actor,
                    amount: amount.min(u32::from(u16::MAX)) as u16,
                    source: EffectSource::Move {
                        pokemon: actor,
                        move_id: move_def.id,
                    },
                    direct: false,
                },
                target,
                crate::engine::HealDrain::Move(move_def.id),
            );
        }
    }

    /// Recoil from `dealt` damage (the generation's rounding, never below the minimum) or from
    /// the user's max HP (Chloroblast), straight to the user's HP (never through its own
    /// Substitute), skipped once the user is down.
    /// The recoil after the hits took the user from `hp_before` to 0 (`recoil_knocked_out`).
    fn note_recoil_knock_out(&mut self, actor: PokemonId, hp_before: u16) {
        if hp_before > 0 && self.state.pokemon(actor).is_some_and(|p| p.is_fainted()) {
            self.recoil_knocked_out = true;
        }
    }

    pub(super) fn apply_recoil(&mut self, actor: PokemonId, move_def: &ResolvedMove, dealt: u32) {
        let Some(recoil) = move_def.recoil else {
            return;
        };
        if self.state.pokemon(actor).is_none_or(|p| p.is_fainted()) {
            return;
        }
        let hit_rules = self.rules.hits;
        let max_hp = self.state.pokemon(actor).map_or(0, |p| p.max_hp);
        let amount = match recoil {
            Recoil::Dealt(fraction) => hit_rules
                .recoil_rounding
                .apply(dealt, fraction)
                .max(u32::from(hit_rules.recoil_minimum)),
            Recoil::UserMaxHp(fraction) => Rounding::HalfUp.apply(u32::from(max_hp), fraction),
            // Gen 5+ `round(baseMaxhp / 4)` (before Gen 5 no Dynamax: the same HP).
            Recoil::Struggle => hit_rules
                .struggle_recoil_rounding
                .apply(
                    u32::from(self.state.pokemon(actor).map_or(0, |p| p.base_max_hp())),
                    Fraction::new(1, 4),
                )
                .max(1),
        }
        .min(u32::from(u16::MAX)) as u16;
        self.recoil_hp(actor, move_def, recoil, amount);
    }

    /// Recoil off a Substitute (Showdown's `substitute` condition recoils at once): from the
    /// damage the doll took — or, in Gen 2, exactly 1 HP.
    fn apply_substitute_recoil(&mut self, actor: PokemonId, move_def: &ResolvedMove, dealt: u32) {
        match move_def.recoil {
            // Struggle's recoil is its own, once the hits are over.
            Some(Recoil::Struggle) => {}
            Some(recoil) if self.rules.conditions.substitute_recoil_one => {
                if self.state.pokemon(actor).is_some_and(|p| !p.is_fainted()) {
                    self.recoil_hp(actor, move_def, recoil, 1);
                }
            }
            _ => self.apply_recoil(actor, move_def, dealt),
        }
    }

    fn recoil_hp(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        recoil: Recoil,
        amount: u16,
    ) {
        // Struggle's recoil is Showdown `directDamage`: an HP cost no ability or item refuses.
        let kind = if recoil == Recoil::Struggle {
            crate::engine::DamageKind::Direct
        } else {
            crate::engine::DamageKind::Recoil
        };
        // Rock Head (`onDamage`): no recoil from the holder's move — but Struggle's (Gen 3's
        // `recoil: [1, 4]`; pokeemerald `BattleScript_MoveEffectRecoil` checks Struggle first).
        if kind == crate::engine::DamageKind::Recoil
            && move_def.id != crate::ids::MoveId::STRUGGLE
            && self.state.pokemon(actor).is_some_and(|p| {
                crate::engine::ability_handler(self.state, self.rules, p, |b| b.blocks_recoil)
            })
        {
            return;
        }
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: actor,
            amount: DamageAmount(amount),
            kind,
            source: EffectSource::Move {
                pokemon: actor,
                move_id: move_def.id,
            },
            flags: crate::engine::DamageFlags {
                bypass_substitute: true,
                spare_one_hp: false,
                instafaint: false,
                single_strike: false,
                future_move: false,
            },
        });
    }

    /// The crash of a move with crash damage that hit no target (Showdown `onMoveFail`), as the
    /// generation's `HitRules::crash` says. Gen 2-4 compute what the move would have dealt the
    /// target (a fresh critical-hit check and damage roll) and hurt the user by a share of it.
    pub(super) fn crash(
        &mut self,
        actor: PokemonId,
        target: Option<PokemonId>,
        move_def: &ResolvedMove,
    ) {
        if self.state.pokemon(actor).is_none_or(|p| p.is_fainted()) {
            return;
        }
        let max_hp = self.state.pokemon(actor).map_or(0, |p| p.base_max_hp());
        let (amount, kind) = match self.rules.hits.crash {
            crate::rules::CrashDamage::OneHp => (1, crate::engine::DamageKind::Direct),
            crate::rules::CrashDamage::ShareOfDamage {
                divisor,
                immune_spared,
                capped_at_half_target_max_hp,
            } => {
                let Some(target) = target else {
                    return;
                };
                let target_max_hp = self.state.pokemon(target).map_or(0, |p| p.max_hp);
                // The target's immunity as the move meets it (Showdown `runImmunity('Fighting')`,
                // `getDamage`): a Ghost Foresight identified is no longer immune (R-M11-126).
                let (effectiveness, _) =
                    crate::engine::type_immunity(self.state, self.rules, target, move_def);
                let damage = if effectiveness.is_zero() {
                    if immune_spared {
                        return;
                    }
                    target_max_hp
                } else {
                    match self
                        .resolve_standard_damage(actor, target, move_def, 1, effectiveness)
                        .final_damage
                        .0
                    {
                        0 => target_max_hp,
                        damage => damage,
                    }
                };
                let mut amount = (damage / divisor.max(1)).max(1);
                if capped_at_half_target_max_hp {
                    amount = amount.min(target_max_hp / 2);
                }
                (amount, crate::engine::DamageKind::SelfHit)
            }
            // Showdown `baseMaxhp / 2`.
            crate::rules::CrashDamage::HalfUserMaxHp => {
                (max_hp / 2, crate::engine::DamageKind::Other)
            }
        };
        // Gen 1: behind the user's own Substitute the crash strikes the foe's, or nothing.
        let (recipient, kind, bypass_substitute) =
            match crate::engine::self_damage_recipient(self.state, self.rules, actor) {
                crate::engine::SelfDamageRecipient::Own => (actor, kind, true),
                crate::engine::SelfDamageRecipient::FoeSubstitute(foe) => {
                    (foe, crate::engine::DamageKind::Move, false)
                }
                crate::engine::SelfDamageRecipient::Nothing => return,
            };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: recipient,
            amount: DamageAmount(amount),
            kind,
            source: EffectSource::Move {
                pokemon: actor,
                move_id: move_def.id,
            },
            flags: crate::engine::DamageFlags {
                bypass_substitute,
                spare_one_hp: false,
                instafaint: false,
                single_strike: false,
                future_move: false,
            },
        });
    }
}
