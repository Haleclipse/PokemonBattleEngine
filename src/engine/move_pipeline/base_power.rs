//! The BasePower stage (DESIGN.md §22.5, §22.7): the move's own formula, then every chain
//! participant, highest priority first.

use crate::dex::ResolvedMove;
use crate::effects;
use crate::effects::moves::BasePower;
use crate::ids::PokemonId;
use crate::rules::{DamageBonusStage, Fraction, TypeItemStage};
use crate::state::VolatileConditionKind;

use crate::engine::damage::{chain_modifiers, fraction_4096, showdown_modify};
use crate::engine::environment;
use crate::engine::{BasePowerContext, ModifierSource, ModifierStage, TraceEvent};

use super::MovePipeline;

impl<'a> MovePipeline<'a> {
    /// The BasePower stage (DESIGN.md §22.5), in Showdown's shape: the move's own
    /// `basePowerCallback` sets the starting value; then every `onBasePower` participant — the
    /// user's ability, its partners' abilities (`onAllyBasePower`), any active aura
    /// (`onAnyBasePower`), its Helping Hand, its item,
    /// the terrain, the move itself, a hiding target (Gen 2) — contributes a 4096-based
    /// modifier, highest Showdown priority first, chained with `chainModify` and applied once
    /// with `modify`. Each contribution is traced with the power it leads to.
    pub(super) fn resolve_base_power(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        actor_item: Option<crate::ids::ItemId>,
    ) -> BasePower {
        let value = self.formula_base_power(actor, target, move_def);
        let participants = self.base_power_participants(actor, target, move_def, actor_item);
        let mut chain: u32 = 4096;
        let mut applications = Vec::new();
        {
            let ctx = BasePowerContext::new(self.state, self.rules, actor, Some(target), move_def);
            for (priority, source, participant) in participants {
                let modifier = match participant {
                    PowerParticipant::Chain(handler) => handler(&ctx, value, chain),
                    PowerParticipant::Fixed(modifier) => Some(modifier),
                };
                let Some(modifier) = modifier else { continue };
                let from = showdown_modify(u32::from(value.0), chain);
                chain = chain_modifiers(chain, modifier);
                let to = showdown_modify(u32::from(value.0), chain);
                applications.push((priority, source, from, to));
            }
        }
        for (priority, source, from, to) in applications {
            self.trace.push(TraceEvent::ModifierApplied {
                stage: ModifierStage::BasePower,
                source,
                priority,
                from,
                to,
            });
        }
        // A power the modifiers bring to 0 (Mud Sport on a power-1 move) stays 0: the move deals
        // nothing (`if (!basePower) return 0`, DESIGN.md R-M8-84). The power it starts from is
        // at least 1.
        let floor = if value.0 == 0 { 1 } else { 0 };
        BasePower(
            showdown_modify(u32::from(value.0), chain).clamp(floor, u32::from(u16::MAX)) as u16,
        )
    }

    /// The power the move starts from: its data, a drawn power (Magnitude, Present), or its
    /// own formula (Showdown `basePowerCallback`, §22.7), traced.
    pub(super) fn formula_base_power(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> BasePower {
        let mut value = BasePower(match self.drawn {
            Some(crate::effects::moves::DrawnPower::Power(power)) => power,
            _ => move_def.base_power,
        });
        // The move's own formula (Showdown `basePowerCallback`, §22.7) sets the starting value.
        if let Some(formula) = move_def.behavior.power_formula {
            // Fury Cutter counts this use first (§15.18) — its first hit (Showdown `move.hit ===
            // 1`): Parental Bond's second hit reads the chain as the first left it.
            if formula == crate::effects::moves::PowerFormula::FuryCutter
                && (self.hit == 1
                    || self
                        .state
                        .pokemon(actor)
                        .is_none_or(|p| p.volatiles.fury_cutter().is_none()))
            {
                crate::engine::BattleContext::new(self.state, self.rules, self.trace)
                    .advance_fury_cutter(actor);
            }
            let before = value;
            value = BasePower(crate::engine::formula_power(
                self.state,
                self.rules,
                self.dex,
                actor,
                target,
                move_def,
                formula,
                crate::engine::MoveUse {
                    hit: self.hit,
                    prioritized_by: self.prioritized_by,
                    member: self.hit_member(),
                    called_by: self.call.map(|call| call.source),
                },
            ));
            self.trace.push(TraceEvent::ModifierApplied {
                stage: ModifierStage::BasePower,
                source: ModifierSource::Move(move_def.id),
                priority: 0,
                from: u32::from(before.0),
                to: u32::from(value.0),
            });
            // Rollout / Ice Ball's `basePowerCallback` counts the hit as it computes the power
            // (§15.12): only a hit that reached the damage calculation — past the hit gate, on
            // the target or its Substitute — keeps the chain going; not while asleep.
            if formula == crate::effects::moves::PowerFormula::Rolling
                && let Some(pokemon) = self.state.pokemon_mut(actor)
                && !pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
            {
                pokemon
                    .volatiles
                    .advance_rolling(crate::effects::moves::ROLLING_MAX_HITS);
            }
        }
        // Gen 7's Rollout storage (issues.md I212; the oracle's `rolloutstorage` `onBasePower`,
        // ahead of every other participant): whatever the move, the stored doubling takes its
        // power over — the move's own as the data has it (at least 1) doubled that many times,
        // once more after Defense Curl — and the next stored one waits for the next power.
        if let Some(pokemon) = self.state.pokemon_mut(actor)
            && let Some((doublings, spent)) = pokemon.volatiles.take_rollout_storage()
        {
            let curled = pokemon
                .volatiles
                .contains(VolatileConditionKind::DefenseCurl);
            let raw = match self.drawn {
                Some(crate::effects::moves::DrawnPower::Power(power)) => power,
                _ => move_def.base_power,
            };
            let before = value;
            let stored = (u32::from(raw.max(1)) << doublings) * if curled { 2 } else { 1 };
            value = BasePower(stored.min(u32::from(u16::MAX)) as u16);
            self.trace.push(TraceEvent::ModifierApplied {
                stage: ModifierStage::BasePower,
                source: ModifierSource::Volatile(VolatileConditionKind::RolloutStorage),
                priority: 0,
                from: u32::from(before.0),
                to: u32::from(value.0),
            });
            if spent {
                crate::engine::BattleContext::new(self.state, self.rules, self.trace)
                    .end_volatile(actor, VolatileConditionKind::RolloutStorage);
            }
        }
        value
    }

    /// Every `onBasePower` participant of the hit, highest Showdown priority first (equal
    /// priorities keep collection order): the user's ability, its partners' abilities
    /// (`onAllyBasePower`), any active aura (`onAnyBasePower`), its Helping Hand, its item, the
    /// gem it used, the terrain, the move itself, a hiding target (Gen 2, Gen 4) — each with its
    /// Showdown priority and source.
    pub(super) fn base_power_participants(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        actor_item: Option<crate::ids::ItemId>,
    ) -> Vec<(i16, ModifierSource, PowerParticipant)> {
        use PowerParticipant as Participant;
        let mut participants: Vec<(i16, ModifierSource, Participant)> = Vec::new();
        let user_ability = self
            .state
            .pokemon(actor)
            .and_then(|p| crate::engine::ability_running(self.state, self.rules, p));
        // The target's ability as data (`onSourceBasePower`: Gen 3-4 Thick Fat).
        if let Some(ability) = self
            .state
            .pokemon(target)
            .and_then(|p| crate::engine::ability_running(self.state, self.rules, p))
            && let Some(boost) = crate::engine::ability_behavior(self.rules, ability).foe_base_power
            && let Some(modifier) =
                boost.modifier_for(self.state, self.rules, target, actor, move_def)
        {
            participants.push((
                boost.priority,
                ModifierSource::AbilityInstance {
                    holder: target,
                    ability,
                },
                Participant::Fixed(modifier),
            ));
        }
        // The user's ability as data (Iron Fist, Tough Claws, Gen 3-4 Blaze).
        if let Some(ability) = user_ability
            && let Some(boost) =
                crate::engine::ability_behavior(self.rules, ability).base_power_boost
            && let Some(modifier) =
                boost.modifier_for(self.state, self.rules, actor, target, move_def)
        {
            participants.push((
                boost.priority,
                ModifierSource::AbilityInstance {
                    holder: actor,
                    ability,
                },
                Participant::Fixed(modifier),
            ));
        }
        if let Some(ability) = user_ability
            && let Some(ordered) = crate::engine::ability_behavior(self.rules, ability).base_power
        {
            participants.push((
                if ability == crate::ids::AbilityId::TECHNICIAN {
                    self.rules.damage.technician_priority
                } else {
                    ordered.priority
                },
                ModifierSource::AbilityInstance {
                    holder: actor,
                    ability,
                },
                Participant::Chain(ordered.handler),
            ));
        }
        for ally in self.active_allies(actor) {
            if let Some(ability) = self
                .state
                .pokemon(ally)
                .and_then(|p| crate::engine::ability_running(self.state, self.rules, p))
                && let Some(ordered) =
                    crate::engine::ability_behavior(self.rules, ability).ally_base_power
            {
                participants.push((
                    ordered.priority,
                    ModifierSource::AbilityInstance {
                        holder: ally,
                        ability,
                    },
                    Participant::Chain(ordered.handler),
                ));
            }
        }
        // Steely Spirit (`onAllyBasePower` as data): the holder's own moves and its partners'.
        for holder in std::iter::once(actor).chain(self.active_allies(actor)) {
            if let Some(ability) = self
                .state
                .pokemon(holder)
                .and_then(|p| crate::engine::ability_running(self.state, self.rules, p))
                && let Some(boost) =
                    crate::engine::ability_behavior(self.rules, ability).team_base_power
                && let Some(modifier) =
                    boost.modifier_for(self.state, self.rules, holder, target, move_def)
            {
                participants.push((
                    boost.priority,
                    ModifierSource::AbilityInstance { holder, ability },
                    Participant::Fixed(modifier),
                ));
            }
        }
        // Dark Aura / Fairy Aura (`onAnyBasePower`, priority 20): one active holder of the
        // move's aura, whoever uses the move; weakened while an Aura Break is active (its
        // `onAnyTryPrimaryHit` marks the move before it hits). Showdown also skips a move aimed
        // at its user, which no damaging move is.
        if let Some((holder, ability)) = crate::engine::active_holder(self.state, self.rules, |b| {
            b.aura == Some(move_def.move_type)
        }) {
            let broken =
                crate::engine::active_holder(self.state, self.rules, |b| b.aura_break).is_some();
            participants.push((
                20,
                ModifierSource::AbilityInstance { holder, ability },
                Participant::Fixed(if broken {
                    effects::abilities::AURA_BROKEN
                } else {
                    effects::abilities::AURA_BOOST
                }),
            ));
        }
        // Charge (`onBasePowerPriority` 9): an Electric move's power doubled.
        if move_def.move_type == crate::dex::PokemonType::Electric
            && self
                .state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Charge))
        {
            participants.push((
                9,
                ModifierSource::Volatile(VolatileConditionKind::Charge),
                Participant::Fixed(8192),
            ));
        }
        // Me First (`onBasePowerPriority` 12, Gen 5+): the move the caller took over is ×1.5.
        if self.rules.conditions.calls.me_first_boost == Some(crate::rules::MeFirstBoost::BasePower)
            && self
                .state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::MeFirst))
        {
            participants.push((
                12,
                ModifierSource::Volatile(VolatileConditionKind::MeFirst),
                Participant::Fixed(6144),
            ));
        }
        if let Some(multiplier) = self
            .state
            .pokemon(actor)
            .and_then(|p| p.volatiles.helping_hand_multiplier())
        {
            participants.push((
                10,
                ModifierSource::Volatile(VolatileConditionKind::HelpingHand),
                Participant::Fixed(multiplier),
            ));
        }
        if let Some(item) = actor_item
            && (self.rules.damage.type_item_stage == TypeItemStage::BasePower
                || !matches!(
                    crate::engine::item_behavior(self.rules, item).base_power_boost,
                    Some(effects::items::BasePowerBoost {
                        filter: effects::items::BoostFilter::MoveType(_),
                        ..
                    })
                ))
            && let Some(boost) = crate::engine::item_behavior(self.rules, item).base_power_boost
        {
            let species = self
                .state
                .pokemon(actor)
                .and_then(|p| crate::engine::item_species(self.rules, p));
            if boost.filter.admits(species, move_def) {
                participants.push((
                    boost.priority,
                    ModifierSource::ItemInstance {
                        holder: actor,
                        item,
                    },
                    Participant::Fixed(boost.numerator),
                ));
            }
        }
        // The gem this move used up (Showdown's `gem` condition, `onBasePower`).
        if let Some(item) = self.gem {
            participants.push((
                self.rules.damage.gem_boost_priority,
                ModifierSource::ItemInstance {
                    holder: actor,
                    item,
                },
                Participant::Fixed(self.rules.damage.gem_boost),
            ));
        }
        if let Some(terrain) = self.state.field.terrain
            && let Some(modifier) =
                environment::terrain_power_modifier(self.state, self.rules, actor, target, move_def)
        {
            participants.push((
                6,
                ModifierSource::Terrain(terrain.kind),
                Participant::Fixed(modifier),
            ));
        }
        // Mud Sport / Water Sport (§23.4): Gen 6+ the pseudo-weather's `onBasePower`, Gen 3-5
        // every holder's volatile (`onAnyBasePower`), on Electric / Fire moves.
        let field_rules = self.rules.conditions.field;
        for (field, volatile, move_type) in [
            (
                crate::state::FieldConditionKind::MudSport,
                VolatileConditionKind::MudSport,
                crate::dex::PokemonType::Electric,
            ),
            (
                crate::state::FieldConditionKind::WaterSport,
                VolatileConditionKind::WaterSport,
                crate::dex::PokemonType::Fire,
            ),
        ] {
            if move_def.move_type != move_type {
                continue;
            }
            let modifier = Participant::Fixed(field_rules.sport_modifier);
            if self.state.field.contains(field) {
                participants.push((
                    field_rules.sport_priority,
                    ModifierSource::Field(field),
                    modifier,
                ));
                continue;
            }
            let holders = [crate::ids::SideId::P1, crate::ids::SideId::P2]
                .into_iter()
                .flat_map(|side| self.state.side(side).active.iter().flatten().copied())
                .filter(|holder| {
                    self.state
                        .pokemon(*holder)
                        .is_some_and(|p| !p.is_fainted() && p.volatiles.contains(volatile))
                })
                .count();
            for _ in 0..holders {
                participants.push((
                    field_rules.sport_priority,
                    ModifierSource::Volatile(volatile),
                    Participant::Fixed(field_rules.sport_modifier),
                ));
            }
        }
        // The move's own conditional participant (Showdown `onBasePower`, §22.7).
        if let Some(boost) = move_def.behavior.power_boost {
            // Collision Course and Electro Drift ask `runEffectiveness`, Tera Shell included
            // (R-M9-45).
            let super_effective = {
                let effectiveness = self.type_effectiveness(target, move_def);
                effectiveness.numerator > effectiveness.denominator
                    && !super::hit_gate::tera_shell_resists(
                        self.state,
                        self.rules,
                        target,
                        move_def,
                        effectiveness,
                    )
            };
            if crate::engine::power_condition_holds(
                self.state,
                self.rules,
                actor,
                target,
                boost.condition,
                super_effective,
            ) {
                participants.push((
                    0,
                    ModifierSource::Move(move_def.id),
                    Participant::Fixed(boost.modifier),
                ));
            }
        }
        // The ability that retyped the move (Aerilate …, Gen 7+ Normalize), priority 23: its own
        // `onBasePower` (`typeChangerBoosted === this.effect`), so only while the user still has
        // it working — a Mummy the first hit touched leaves the later hits retyped but unboosted
        // (R-M8-166).
        if let Some(ability) = self.type_changer
            && self.state.pokemon(actor).is_some_and(|pokemon| {
                crate::engine::has_ability(self.state, self.rules, pokemon, ability)
            })
        {
            participants.push((
                23,
                ModifierSource::AbilityInstance {
                    holder: actor,
                    ability,
                },
                Participant::Fixed(self.rules.damage.type_change_boost),
            ));
        }
        // The move's own chance (Fickle Beam), rolled with the BasePower event — after the
        // critical-hit check (Showdown `getDamage`).
        if let Some(chance) = move_def.behavior.power_chance
            && self.state.rng.roll_ratio_for(
                crate::state::RandomRequestKind::PowerChance,
                chance.numerator,
                chance.denominator,
            )
        {
            participants.push((
                0,
                ModifierSource::Move(move_def.id),
                Participant::Fixed(chance.modifier),
            ));
        }
        // Gen 2 and Gen 4 double Gust / Earthquake against a hiding target at this stage.
        let hiding_bonus = self.state.pokemon(target).map_or(Fraction::ONE, |t| {
            crate::engine::two_turn::damage_bonus(
                self.rules,
                self.dex,
                t,
                move_def,
                DamageBonusStage::BasePower,
            )
        });
        if hiding_bonus != Fraction::ONE {
            participants.push((
                0,
                ModifierSource::Rule("SemiInvulnerable"),
                Participant::Fixed(fraction_4096(
                    u32::from(hiding_bonus.numerator),
                    u32::from(hiding_bonus.denominator),
                )),
            ));
        }
        // Gen 4 doubles a `minimize` move's power against a Minimized target (the cartridge).
        if crate::engine::minimize_bonus(self.state, self.rules, target, move_def)
            == Some(DamageBonusStage::BasePower)
        {
            participants.push((
                0,
                ModifierSource::Rule("Minimize"),
                Participant::Fixed(8192),
            ));
        }
        // Showdown runs higher priorities first; equal priorities keep collection order.
        participants.sort_by_key(|(priority, _, _)| std::cmp::Reverse(*priority));
        participants
    }
}

/// One BasePower participant: a handler that reads the power and the chain so far
/// (Technician), or a fixed 4096-based modifier.
pub(super) enum PowerParticipant {
    Chain(effects::moves::BasePowerChainHandler),
    Fixed(u32),
}
