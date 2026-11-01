//! Gen 4's damage formula as the cartridge computes it (DESIGN.md R125; issues.md I14-I20):
//! pokeplatinum's `BattleSystem_CalcMoveDamage`, `BattleScript_CalcMoveDamage`,
//! `BattleSystem_CalcDamageVariance`, `BattleSystem_ApplyTypeChart` and the resist berry of its HP
//! update, step for step. The participants are the ones the Showdown-shaped path collects —
//! their conditions and values stay with the abilities, items and moves — but every one
//! multiplies in turn and is floored at once (`× 120 / 100`, not a 4096-based chain rounded
//! once), in the cartridge's order (`CartridgeProfile`), and the stat modifiers multiply the
//! stored stat before its stage.

use crate::dex::{MoveCategory, PokemonType, ResolvedMove};
use crate::effects;
use crate::effects::moves::{BasePower, PowerCondition, PowerFormula};
use crate::ids::PokemonId;
use crate::rules::{CartridgeProfile, CartridgeStep, DamageModifierKind, Fraction, MetronomeRules};
use crate::state::{NonVolatileStatus, RandomRequestKind, VolatileConditionKind};

use crate::engine::damage::{
    CriticalHit, DamageAmount, DamageBreakdown, base_damage, exact_fraction,
};
use crate::engine::environment;
use crate::engine::stats::{
    StatParts, defensive_parts, offensive_parts, stage_value, weather_defense_boost,
};
use crate::engine::{
    BasePowerContext, DefensiveStat, ModifierDescriptor, ModifierSource, ModifierStage,
    OffensiveStat, TraceEvent,
};

use super::MovePipeline;
use super::base_power::PowerParticipant;

/// `value × modifier`, the modifier as the exact fraction its 4096-based value stands for,
/// floored.
fn times(value: u32, modifier: u32) -> u32 {
    let (numerator, denominator) = exact_fraction(modifier);
    (u64::from(value) * u64::from(numerator) / u64::from(denominator)).min(u64::from(u32::MAX))
        as u32
}

fn times_fraction(value: u32, fraction: Fraction) -> u32 {
    fraction.apply_u32(value)
}

/// `BattleSystem_Divide`: a quotient of 0 from a value that was not is 1.
fn divide(value: u32, numerator: u32, denominator: u32) -> u32 {
    if value == 0 {
        return 0;
    }
    (value * numerator / denominator).max(1)
}

/// Which of the profile's steps a participant is.
fn step_of(source: ModifierSource) -> Option<CartridgeStep> {
    match source {
        ModifierSource::Move(_) | ModifierSource::Rule(_) => Some(CartridgeStep::Move),
        ModifierSource::AbilityInstance { ability, .. } => Some(CartridgeStep::Ability(ability)),
        ModifierSource::ItemInstance { .. } => Some(CartridgeStep::Item),
        ModifierSource::Volatile(kind) => Some(CartridgeStep::Volatile(kind)),
        ModifierSource::Weather(_) => Some(CartridgeStep::Weather),
        ModifierSource::Status(_) | ModifierSource::Terrain(_) | ModifierSource::Field(_) => None,
    }
}

/// Where a participant stands in `sequence`; one the profile does not list comes last.
fn rank(sequence: &[CartridgeStep], source: ModifierSource) -> usize {
    step_of(source)
        .and_then(|step| CartridgeProfile::position(sequence, step))
        .unwrap_or(usize::MAX)
}

impl<'a> MovePipeline<'a> {
    /// The Gen 4 damage of one hit (`DamageAlgorithm::Gen4`). `self_hit`: the confusion
    /// self-hit, which borrows Struggle (`CALC_SELF_HIT(MOVE_STRUGGLE, 40)`): no screen, no
    /// weather (nor the sun's Flower Gift), no second phase, no type chart.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_cartridge_damage(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        target_count: usize,
        type_effectiveness: Fraction,
        profile: &CartridgeProfile,
        self_hit: bool,
    ) -> DamageBreakdown {
        let actor_item = crate::engine::held_item_id(self.state, self.rules, actor);
        let critical = self.roll_critical(actor, target, move_def);
        // A typeless move is Struggle, or the self-hit that borrows it: the cartridge reads its
        // data type, Normal, everywhere but the type chart, which it skips.
        let typeless = move_def.move_type == PokemonType::Typeless;
        let as_normal;
        let seen = if typeless {
            as_normal = ResolvedMove {
                move_type: PokemonType::Normal,
                ..move_def.clone()
            };
            &as_normal
        } else {
            move_def
        };
        // `BattleSystem_CalcCriticalMulti`: 2, or 3 with Sniper (its `onModifyDamage` joins the
        // multiplier).
        let actor_ability = self
            .state
            .pokemon(actor)
            .and_then(|pokemon| crate::engine::ability_running(self.state, self.rules, pokemon));
        let critical_boost = actor_ability
            .and_then(|ability| crate::engine::ability_behavior(self.rules, ability).damage_boost)
            .filter(|boost| boost.when == effects::abilities::DamageBoostWhen::Critical);
        let critical_multiplier = if critical.0 {
            let (numerator, denominator) =
                critical_boost.map_or((1, 1), |boost| exact_fraction(boost.modifier));
            u32::from(self.rules.damage.critical_multiplier.numerator) * numerator
                / (u32::from(self.rules.damage.critical_multiplier.denominator) * denominator)
        } else {
            1
        };
        let helping_hand = self
            .state
            .pokemon(actor)
            .and_then(|pokemon| pokemon.volatiles.helping_hand_multiplier());
        if let Some((attack, level, defense)) = self.beat_up_stats(target, move_def) {
            return self.cartridge_beat_up(
                actor,
                target,
                move_def,
                critical,
                critical_multiplier,
                helping_hand,
                (attack, level, defense),
            );
        }
        let power = self.cartridge_power(actor, target, seen, actor_item, profile, typeless);

        // The stats: the stored value, every modifier in the profile's order, then the stage.
        // On its own confusion self-hit an Unaware Pokémon is both sides of the check
        // (`Battler_IgnorableAbility(attacker, attacker, ABILITY_UNAWARE)`): no stage counts.
        let own_unaware = self_hit
            && actor_ability.is_some_and(|ability| {
                crate::engine::ability_behavior(self.rules, ability).unaware
            });
        let unstaged = |parts: StatParts| StatParts {
            stage: if own_unaware { 0 } else { parts.stage },
            ..parts
        };
        let offense_parts =
            offensive_parts(self.state, self.rules, actor, target, seen, critical.0).map(unstaged);
        let mut offense_participants =
            self.offense_participants(actor, target, seen, actor_ability, actor_item);
        // The self-hit sees no sun (`CALC_SELF_HIT` passes no field conditions): no Flower Gift
        // (R-M8-108).
        if self_hit {
            offense_participants.retain(|(descriptor, _)| {
                !matches!(descriptor.source, ModifierSource::AbilityInstance { ability, .. }
                    if crate::engine::ability_behavior(self.rules, ability).flower_gift)
            });
        }
        offense_participants
            .sort_by_key(|(descriptor, _)| rank(profile.offense, descriptor.source));
        let (offense_base, offense_modified) =
            self.cartridge_stat(offense_parts, offense_participants, 0);
        let offense = OffensiveStat(offense_parts.map_or(0, |parts| {
            stage_value(
                self.rules.stats.stage_table,
                clamp_u16(offense_modified),
                parts.stage,
            )
        }));
        self.trace.push(TraceEvent::OffensiveStatResolved {
            actor,
            target,
            move_id: move_def.id,
            base: offense_base,
            after_ability: offense_modified,
            after_item: offense.0,
        });
        let defense_parts =
            defensive_parts(self.state, self.rules, actor, target, seen, critical.0).map(unstaged);
        let mut defense_participants = self.defense_participants(actor, target, seen);
        if let Some(parts) = defense_parts
            && let boost = weather_defense_boost(self.state, self.rules, target, parts.stat)
            && boost != Fraction::ONE
            && let Some(weather) = self.state.field.weather
        {
            defense_participants.push((
                ModifierDescriptor::new(
                    ModifierStage::DefensiveStat,
                    ModifierSource::Weather(weather.kind),
                    10,
                ),
                crate::engine::damage::fraction_4096(
                    u32::from(boost.numerator),
                    u32::from(boost.denominator),
                ),
            ));
        }
        defense_participants
            .sort_by_key(|(descriptor, _)| rank(profile.defense, descriptor.source));
        let (_, mut defense_modified) = self.cartridge_stat(defense_parts, defense_participants, 1);
        // Explosion / Self-Destruct (`BATTLE_EFFECT_HALVE_DEFENSE`): the Defense after its
        // modifiers, before its stage, halved.
        if move_def.behavior.halves_defense
            && self.rules.conditions.self_faint.explosion_halves_defense
            && defense_parts.is_some_and(|parts| parts.stat == crate::state::Stat::Defense)
        {
            defense_modified /= 2;
        }
        let defense = DefensiveStat(defense_parts.map_or(1, |parts| {
            stage_value(
                self.rules.stats.stage_table,
                clamp_u16(defense_modified),
                parts.stage,
            )
            .max(1)
        }));
        self.trace.push(TraceEvent::DefensiveStatResolved {
            actor,
            target,
            move_id: move_def.id,
            value: defense.0,
        });

        let level = self.state.pokemon(actor).map_or(1, |pokemon| pokemon.level);
        let base = base_damage(self.rules.damage.algorithm, level, power, offense, defense);

        let weather_modifier = if self_hit {
            Fraction::ONE
        } else {
            environment::weather_damage_modifier(self.state, self.rules, actor, target, seen)
        };
        self.trace.push(TraceEvent::WeatherModifierResolved {
            actor,
            move_id: move_def.id,
            weather: self.state.field.weather.map(|weather| weather.kind),
            modifier: weather_modifier,
        });
        // Solar Beam in any weather but sun: the cartridge halves the damage after the weather
        // (`FIELD_CONDITION_SOLAR_DOWN`), not the power.
        let weather_weakened = (!self_hit)
            .then_some(seen.behavior.power_boost)
            .flatten()
            .filter(|boost| {
                boost.condition == PowerCondition::WeakeningWeather
                    && crate::engine::power_condition_holds(
                        self.state,
                        self.rules,
                        actor,
                        target,
                        boost.condition,
                        false,
                    )
            });
        // `forceSTAB` (the combined Pledges, §42.9): the move counts as the user's type.
        let stab = move_def.flags.force_stab
            || (!typeless
                && self.state.pokemon(actor).is_some_and(|pokemon| {
                    crate::engine::types_of(self.state, self.rules, pokemon)
                        .contains(move_def.move_type)
                }));
        let stab_modifier = if stab {
            actor_ability
                .and_then(|ability| {
                    crate::engine::ability_behavior(self.rules, ability).stab_modifier
                })
                .unwrap_or(self.rules.damage.stab_modifier)
        } else {
            Fraction::ONE
        };
        self.trace.push(TraceEvent::StabResolved {
            actor,
            move_id: move_def.id,
            move_type: move_def.move_type,
            modifier: stab_modifier,
        });
        let spread_applied = move_def.is_spread && target_count > 1;
        let burn_applied = seen.category == MoveCategory::Physical
            && !actor_ability.is_some_and(|ability| {
                crate::engine::ability_behavior(self.rules, ability).ignores_burn_drop
            })
            && self
                .state
                .pokemon(actor)
                .is_some_and(|pokemon| pokemon.has_status(NonVolatileStatus::Burn));
        let screen = crate::engine::screens::damage_modifier(
            self.state,
            self.rules,
            actor,
            target,
            seen.category,
            critical.0,
        );
        let random_roll = self.state.rng.range_inclusive_for(
            RandomRequestKind::DamageRoll,
            self.rules.damage.random_min,
            self.rules.damage.random_max,
        );
        let random_percent = ((u32::from(random_roll) * 100)
            / u32::from(self.rules.damage.random_denominator.max(1)))
            as u16;

        let mut damage = u32::from(base);
        for (index, kind) in self.rules.damage.modifier_order.iter().enumerate() {
            let before = damage;
            let source = match kind {
                DamageModifierKind::Burn if burn_applied => {
                    damage = times_fraction(damage, self.rules.damage.burn_modifier);
                    ModifierSource::Status(NonVolatileStatus::Burn)
                }
                DamageModifierKind::Burn => ModifierSource::Rule("Burn"),
                DamageModifierKind::ModifyDamagePhase1 => {
                    if let Some(modifier) = screen {
                        damage = times(damage, modifier);
                    }
                    ModifierSource::Rule("ModifyDamagePhase1")
                }
                DamageModifierKind::Spread => {
                    if spread_applied {
                        damage = times_fraction(damage, self.rules.damage.spread_modifier);
                    }
                    ModifierSource::Rule("Spread")
                }
                DamageModifierKind::Weather => {
                    damage = times_fraction(damage, weather_modifier);
                    if let Some(boost) = weather_weakened {
                        damage = times(damage, boost.modifier);
                    }
                    // Flash Fire (`ModifyDamagePhase1` in Showdown's gen4): after the weather.
                    if self.rules.conditions.flash_fire.boost
                        == crate::rules::FlashFireBoost::ModifyDamagePhase1
                        && self.flash_fire_boosts(actor, seen)
                    {
                        damage = times(damage, 6144);
                    }
                    self.state
                        .field
                        .weather
                        .map_or(ModifierSource::Rule("Weather"), |weather| {
                            ModifierSource::Weather(weather.kind)
                        })
                }
                DamageModifierKind::BaseOffset => {
                    damage += 2;
                    ModifierSource::Rule("BaseOffset")
                }
                DamageModifierKind::Critical => {
                    damage *= critical_multiplier;
                    ModifierSource::Rule("Critical")
                }
                // `BattleScript_CalcMoveDamage`: Life Orb, then the Metronome item's
                // `× (10 + n) / 10` (issues.md I21), each floored; the self-hit runs neither.
                DamageModifierKind::ModifyDamagePhase2 => {
                    let mut source = ModifierSource::Rule("ModifyDamagePhase2");
                    if let Some((item, fraction)) =
                        actor_item.filter(|_| !self_hit).and_then(|item| {
                            crate::engine::item_behavior(self.rules, item)
                                .damage_phase2
                                .map(|fraction| (item, fraction))
                        })
                    {
                        damage = times_fraction(damage, fraction);
                        source = ModifierSource::ItemInstance {
                            holder: actor,
                            item,
                        };
                    }
                    // Me First (Gen 4's `onModifyDamagePhase2`): the taken-over move ×1.5.
                    if self.rules.conditions.calls.me_first_boost
                        == Some(crate::rules::MeFirstBoost::DamagePhase2)
                        && !self_hit
                        && self
                            .state
                            .pokemon(actor)
                            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::MeFirst))
                    {
                        damage = times_fraction(damage, crate::rules::Fraction::new(3, 2));
                        source = ModifierSource::Volatile(VolatileConditionKind::MeFirst);
                    }
                    if let MetronomeRules::Cartridge { .. } = self.rules.damage.metronome
                        && !self_hit
                        && let Some((_, count)) = self
                            .state
                            .pokemon(actor)
                            .and_then(|pokemon| pokemon.volatiles.metronome())
                    {
                        damage = damage * (10 + u32::from(count)) / 10;
                        source = ModifierSource::Volatile(VolatileConditionKind::Metronome);
                    }
                    source
                }
                DamageModifierKind::Random => {
                    if damage > 0 {
                        damage = (damage * u32::from(random_roll)
                            / u32::from(self.rules.damage.random_denominator))
                        .max(1);
                    }
                    ModifierSource::Rule("Random")
                }
                DamageModifierKind::Stab => {
                    if !typeless {
                        damage = times_fraction(damage, stab_modifier);
                    }
                    ModifierSource::Rule("STAB")
                }
                DamageModifierKind::TypeEffectiveness => {
                    if !typeless {
                        damage = self.cartridge_type_chart(damage, target, move_def, profile);
                    }
                    ModifierSource::Rule("TypeEffectiveness")
                }
                DamageModifierKind::ModifyDamage => {
                    if !typeless {
                        damage = self.cartridge_type_chart_tail(
                            damage,
                            actor,
                            target,
                            move_def,
                            actor_item,
                            type_effectiveness,
                        );
                    }
                    ModifierSource::Rule("ModifyDamage")
                }
                // Steps of other generations' orders.
                DamageModifierKind::PhysicalMinimum
                | DamageModifierKind::Minimum
                | DamageModifierKind::LegacyClamp => continue,
            };
            self.trace.push(TraceEvent::ModifierApplied {
                stage: ModifierStage::FinalDamage,
                source,
                priority: index as i16,
                from: before,
                to: damage,
            });
        }
        // The resist berry (`subscript_type_resist_berry`, in the HP update): eaten on the body,
        // it halves the final damage (`DivideVarByValue`).
        if !self_hit && self.eat_resist_berry(target, move_def, type_effectiveness) {
            let before = damage;
            damage = divide(damage, 1, 2);
            self.trace.push(TraceEvent::ModifierApplied {
                stage: ModifierStage::FinalDamage,
                source: ModifierSource::Rule("ResistBerry"),
                priority: self.rules.damage.modifier_order.len() as i16,
                from: before,
                to: damage,
            });
        }
        let final_damage = DamageAmount(clamp_u16(damage.max(1)));

        self.trace.push(TraceEvent::DamageCalculated {
            actor,
            target,
            move_id: move_def.id,
            base_damage: base,
            spread_applied,
            critical: critical.0,
            weather_modifier,
            stab_modifier,
            type_effectiveness,
            burn_applied,
            random_percent,
            final_damage: final_damage.0,
        });
        DamageBreakdown {
            base_power: power,
            offense,
            defense,
            base_damage: base,
            spread_applied,
            critical,
            weather_modifier,
            stab_modifier,
            type_effectiveness,
            burn_applied,
            random_percent,
            final_damage,
        }
    }

    /// The power: the move's formula (the cartridge's move script skips Smelling Salts' and
    /// Wake-Up Slap's doubling against a Substitute: `CheckSubstitute`), then every participant
    /// in the profile's order, one at a time, floored. Struggle skips what the profile says.
    fn cartridge_power(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        seen: &ResolvedMove,
        actor_item: Option<crate::ids::ItemId>,
        profile: &CartridgeProfile,
        typeless: bool,
    ) -> BasePower {
        let behind_substitute = self.state.pokemon(target).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .contains(VolatileConditionKind::Substitute)
        }) && !seen.flags.bypass_substitute;
        let start = if behind_substitute
            && matches!(
                seen.behavior.power_formula,
                Some(PowerFormula::DoubledAgainst(_))
            ) {
            BasePower(seen.base_power)
        } else {
            self.formula_base_power(actor, target, seen)
        };
        let mut participants = self.base_power_participants(actor, target, seen, actor_item);
        participants.sort_by_key(|(_, source, _)| rank(profile.power, *source));
        // Mud Sport / Water Sport halve the power once however many Pokémon used them
        // (`BattleSystem_AnyBattlersWithMoveEffect`); Showdown's gen4 volatile registers one
        // handler per holder (issues.md I22).
        let mut sports_seen = Vec::new();
        participants.retain(|(_, source, _)| match source {
            ModifierSource::Volatile(
                kind @ (VolatileConditionKind::MudSport | VolatileConditionKind::WaterSport),
            ) => {
                let first = !sports_seen.contains(kind);
                sports_seen.push(*kind);
                first
            }
            _ => true,
        });
        // Solar Beam's weakening is the damage's (see the Weather step).
        let weather_weakened = matches!(
            seen.behavior.power_boost,
            Some(boost) if boost.condition == PowerCondition::WeakeningWeather
        );
        let mut power = u32::from(start.0);
        let mut applications = Vec::new();
        {
            let ctx = BasePowerContext::new(self.state, self.rules, actor, Some(target), seen);
            for (priority, source, participant) in participants {
                let skipped = (typeless
                    && step_of(source).is_some_and(|step| profile.struggle_skips.contains(&step)))
                    || (weather_weakened && matches!(source, ModifierSource::Move(_)));
                if skipped {
                    continue;
                }
                let modifier = match participant {
                    PowerParticipant::Chain(handler) => {
                        handler(&ctx, BasePower(clamp_u16(power)), 4096)
                    }
                    PowerParticipant::Fixed(modifier) => Some(modifier),
                };
                let Some(modifier) = modifier else { continue };
                let from = power;
                power = times(power, modifier);
                applications.push((priority, source, from, power));
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
        BasePower(clamp_u16(power))
    }

    /// A stat's stored value through its participants, in order, floored at each: returns the
    /// stored value and the modified one (`floor`: what a missing stat reads).
    fn cartridge_stat(
        &mut self,
        parts: Option<StatParts>,
        participants: Vec<(ModifierDescriptor, u32)>,
        floor: u32,
    ) -> (u32, u32) {
        let Some(parts) = parts else {
            return (floor, floor);
        };
        let stored = u32::from(parts.stored);
        let mut value = stored;
        for (descriptor, modifier) in participants {
            let from = value;
            value = times(value, modifier);
            self.trace.push(TraceEvent::ModifierApplied {
                stage: descriptor.stage,
                source: descriptor.source,
                priority: descriptor.order.priority,
                from,
                to: value,
            });
        }
        (stored, value)
    }

    /// `BattleSystem_ApplyTypeChart`'s multipliers: each of the target's types the move is not
    /// neutral against, in the table's order, `× 20 / 10` or `× 5 / 10` through
    /// `BattleSystem_Divide`.
    fn cartridge_type_chart(
        &self,
        damage: u32,
        target: PokemonId,
        move_def: &ResolvedMove,
        profile: &CartridgeProfile,
    ) -> u32 {
        let mut factors = super::hit_gate::type_factors(self.state, self.rules, target, move_def);
        factors
            .sort_by_key(|(defending, _)| profile.chart_position(move_def.move_type, *defending));
        factors.iter().fold(damage, |damage, (_, factor)| {
            divide(
                damage,
                u32::from(factor.numerator) * 10,
                u32::from(factor.denominator) * 10,
            )
        })
    }

    /// The end of `BattleSystem_ApplyTypeChart`: on a super-effective hit the target's Filter /
    /// Solid Rock (`BattleSystem_Divide(damage × 3, 4)`) then the user's Expert Belt (`× 120 /
    /// 100`); on a resisted one the user's Tinted Lens (× 2).
    fn cartridge_type_chart_tail(
        &self,
        mut damage: u32,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        actor_item: Option<crate::ids::ItemId>,
        type_effectiveness: Fraction,
    ) -> u32 {
        if let Some(pokemon) = self.state.pokemon(target)
            && let Some(taken) =
                crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.damage_taken)
            && let Some(modifier) = taken.modifier(pokemon, move_def, type_effectiveness)
        {
            let (numerator, denominator) = exact_fraction(modifier);
            damage = divide(damage, numerator, denominator);
        }
        if let Some(item) = actor_item
            && let Some(boost) = crate::engine::item_behavior(self.rules, item).final_damage_boost
            && (!boost.only_super_effective
                || type_effectiveness.numerator > type_effectiveness.denominator)
        {
            damage = times(damage, boost.numerator);
        }
        if let Some(boost) = self.state.pokemon(actor).and_then(|p| {
            crate::engine::ability_handler(self.state, self.rules, p, |b| b.damage_boost)
        }) && boost.when == effects::abilities::DamageBoostWhen::NotVeryEffective
            && type_effectiveness.numerator < type_effectiveness.denominator
        {
            damage = times(damage, boost.modifier);
        }
        damage
    }

    /// Beat Up (`BtlCmd_BeatUp`): the member's base Attack and level against the target's base
    /// Defense at the move's power, `+ 2`, the critical hit, Helping Hand and the roll — no
    /// ability, item, burn, screen, weather or type.
    #[allow(clippy::too_many_arguments)]
    fn cartridge_beat_up(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        critical: CriticalHit,
        critical_multiplier: u32,
        helping_hand: Option<u32>,
        (attack, level, defense): (u32, u8, u32),
    ) -> DamageBreakdown {
        let power = BasePower(move_def.base_power);
        let base = base_damage(
            self.rules.damage.algorithm,
            level,
            power,
            OffensiveStat(attack),
            DefensiveStat(defense),
        );
        let mut damage = (u32::from(base) + 2) * critical_multiplier;
        if let Some(multiplier) = helping_hand {
            damage = times(damage, multiplier);
        }
        let random_roll = self.state.rng.range_inclusive_for(
            RandomRequestKind::DamageRoll,
            self.rules.damage.random_min,
            self.rules.damage.random_max,
        );
        damage = (damage * u32::from(random_roll)
            / u32::from(self.rules.damage.random_denominator))
        .max(1);
        let final_damage = DamageAmount(clamp_u16(damage));
        let random_percent = ((u32::from(random_roll) * 100)
            / u32::from(self.rules.damage.random_denominator.max(1)))
            as u16;
        self.trace.push(TraceEvent::DamageCalculated {
            actor,
            target,
            move_id: move_def.id,
            base_damage: base,
            spread_applied: false,
            critical: critical.0,
            weather_modifier: Fraction::ONE,
            stab_modifier: Fraction::ONE,
            type_effectiveness: Fraction::ONE,
            burn_applied: false,
            random_percent,
            final_damage: final_damage.0,
        });
        DamageBreakdown {
            base_power: power,
            offense: OffensiveStat(attack),
            defense: DefensiveStat(defense),
            base_damage: base,
            spread_applied: false,
            critical,
            weather_modifier: Fraction::ONE,
            stab_modifier: Fraction::ONE,
            type_effectiveness: Fraction::ONE,
            burn_applied: false,
            random_percent,
            final_damage,
        }
    }
}

fn clamp_u16(value: u32) -> u16 {
    value.min(u32::from(u16::MAX)) as u16
}
