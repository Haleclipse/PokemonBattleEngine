//! One hit's damage (DESIGN.md §19, §22.6): the critical-hit check, fixed damage and one-hit
//! KOs, and the modern and legacy damage formulas.

use crate::dex::{MoveCategory, PokemonType, ResolvedMove};
use crate::effects;
use crate::effects::moves::BasePower;
use crate::ids::PokemonId;
use crate::rules::{
    DamageAlgorithm, DamageBonusStage, DamageModifierKind, Fraction, TypeItemStage,
};
use crate::state::{NonVolatileStatus, RandomRequestKind};

use crate::engine::damage::{
    CriticalHit, DamageAmount, DamageBreakdown, FinalDamageModifier, apply_final_modifiers,
    base_damage, chain_modifiers, fraction_4096, showdown_modify,
};
use crate::engine::environment;
use crate::engine::stats::{defensive_stat, offensive_stat};
use crate::engine::{
    ModifierDescriptor, ModifierSource, ModifierStage, OffensiveStatContext, TraceEvent,
};

use super::MovePipeline;

impl<'a> MovePipeline<'a> {
    /// The critical-hit check (DESIGN.md §22.6): `engine::critical_roll` decides whether and
    /// how it is rolled; a target whose ability blocks critical hits (Battle Armor) turns a
    /// landed one back, after the roll (Showdown `CriticalHit` event).
    pub(super) fn roll_critical(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> CriticalHit {
        let roll =
            crate::engine::critical_roll(self.state, self.rules, self.dex, actor, target, move_def);
        let (chance, landed) = match roll {
            crate::engine::CriticalRoll::Never => (Fraction::ZERO, false),
            crate::engine::CriticalRoll::Always => (Fraction::ONE, true),
            crate::engine::CriticalRoll::Chance(numerator, denominator) => (
                Fraction::new(numerator, denominator),
                self.state
                    .rng
                    .roll_ratio_for(RandomRequestKind::Critical, numerator, denominator),
            ),
        };
        // Lucky Chant on the target's side turns it back the same way. Disguise and Ice Face
        // (`onCriticalHit`) turn it back while the forme that takes hits whole is unbroken — Ice
        // Face against a physical move only (`move.category !== 'Physical'`), neither for a move
        // that strikes a Substitute in front of it (R-M10-78).
        let blocked = self.state.pokemon(target).is_some_and(|t| {
            let handlers = crate::engine::ability_handlers(self.state, self.rules, t);
            handlers.blocks_critical_hit
                || handlers.absorbing.is_some_and(|absorbing| {
                    absorbing.answers_before_the_hit(self.state, self.rules, target, move_def)
                })
        }) || crate::engine::protections::lucky_chant(self.state, target);
        let critical = CriticalHit(landed && !blocked);
        self.trace.push(TraceEvent::CriticalChecked {
            actor,
            target,
            move_id: move_def.id,
            chance,
            critical: critical.0,
        });
        critical
    }

    pub(super) fn fixed_damage_breakdown(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        amount: u16,
    ) -> DamageBreakdown {
        self.trace.push(TraceEvent::DamageCalculated {
            actor,
            target,
            move_id: move_def.id,
            base_damage: amount,
            spread_applied: false,
            critical: false,
            weather_modifier: Fraction::ONE,
            stab_modifier: Fraction::ONE,
            type_effectiveness: Fraction::ONE,
            burn_applied: false,
            random_percent: 100,
            final_damage: amount,
        });
        DamageBreakdown {
            base_power: BasePower(0),
            offense: crate::engine::OffensiveStat(0),
            defense: crate::engine::DefensiveStat(1),
            base_damage: amount,
            spread_applied: false,
            critical: CriticalHit(false),
            weather_modifier: Fraction::ONE,
            stab_modifier: Fraction::ONE,
            type_effectiveness: Fraction::ONE,
            burn_applied: false,
            random_percent: 100,
            final_damage: DamageAmount(amount),
        }
    }

    /// Gen 1-2's damage (DESIGN.md §31.2, §31.3): the critical hit is decided first, the base
    /// power goes through the same BasePower stage, and `legacy_damage::gen2_damage` /
    /// `gen1_damage` does the rest.
    pub(super) fn resolve_legacy_damage(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        type_effectiveness: Fraction,
    ) -> DamageBreakdown {
        let actor_item = crate::engine::held_item_id(self.state, self.rules, actor);
        let critical = self.roll_critical(actor, target, move_def);
        let power = self.resolve_base_power(actor, target, move_def, actor_item);
        // Showdown `getDamage`: a BasePower event that leaves 0 deals 0 (still a hit).
        if power.0 == 0 {
            return self.fixed_damage_breakdown(actor, target, move_def, 0);
        }
        let weather_modifier =
            environment::weather_damage_modifier(self.state, self.rules, actor, target, move_def);
        let item_boost = actor_item.filter(|item| {
            self.rules.damage.type_item_stage == TypeItemStage::ModifyDamage
                && matches!(
                    crate::engine::item_behavior(self.rules, *item).base_power_boost,
                    Some(effects::items::BasePowerBoost {
                        filter: effects::items::BoostFilter::MoveType(t),
                        ..
                    }) if t == move_def.move_type
                )
        });
        let stab = self.state.pokemon(actor).is_some_and(|pokemon| {
            crate::engine::types_of(self.state, self.rules, pokemon).contains(move_def.move_type)
        });
        let hit = crate::engine::legacy_damage::LegacyHit {
            attacker: actor,
            defender: target,
            base_power: power.0,
            move_type: Some(move_def.move_type),
            pair: if move_def.category == MoveCategory::Special {
                crate::engine::legacy_damage::StatPair::Special
            } else {
                crate::engine::legacy_damage::StatPair::Physical
            },
            critical: critical.0,
            variance: !move_def.no_damage_variance,
            type_effectiveness,
            weather: weather_modifier,
            item_boost,
            minimized: crate::engine::minimize_bonus(self.state, self.rules, target, move_def)
                == Some(DamageBonusStage::AfterRandom),
            type_index_damage: move_def.behavior.type_index_damage,
            beat_up: self.beat_up_stats(target, move_def),
            halves_defense: move_def.behavior.halves_defense
                && self.rules.conditions.self_faint.explosion_halves_defense,
            multiplier: self.gen2_damage_multiplier(actor, move_def),
        };
        let result = if self.rules.damage.algorithm == DamageAlgorithm::Gen1Legacy {
            crate::engine::legacy_damage::gen1_damage(self.state, self.rules, self.trace, &hit)
        } else {
            crate::engine::legacy_damage::gen2_damage(self.state, self.rules, self.trace, &hit)
        };
        self.trace.push(TraceEvent::OffensiveStatResolved {
            actor,
            target,
            move_id: move_def.id,
            base: result.offense,
            after_ability: result.offense,
            after_item: result.offense,
        });
        self.trace.push(TraceEvent::DefensiveStatResolved {
            actor,
            target,
            move_id: move_def.id,
            value: result.defense,
        });
        let stab_modifier = if stab {
            self.rules.damage.stab_modifier
        } else {
            Fraction::ONE
        };
        let random_percent = result.random.map_or(100, |roll| {
            (u32::from(roll) * 100 / u32::from(self.rules.damage.random_denominator)) as u16
        });
        let burn_applied = move_def.category == MoveCategory::Physical
            && self
                .state
                .pokemon(actor)
                .is_some_and(|pokemon| pokemon.has_status(NonVolatileStatus::Burn));
        self.trace.push(TraceEvent::DamageCalculated {
            actor,
            target,
            move_id: move_def.id,
            base_damage: result.base,
            spread_applied: false,
            critical: critical.0,
            weather_modifier,
            stab_modifier,
            type_effectiveness,
            burn_applied,
            random_percent,
            final_damage: result.damage,
        });
        DamageBreakdown {
            base_power: power,
            offense: crate::engine::OffensiveStat(result.offense),
            defense: crate::engine::DefensiveStat(result.defense),
            base_damage: result.base,
            spread_applied: false,
            critical,
            weather_modifier,
            stab_modifier,
            type_effectiveness,
            burn_applied,
            random_percent,
            final_damage: DamageAmount(result.damage),
        }
    }

    /// Gen 3-4 confusion self-hit (Showdown gen4 `getDamage(pokemon, pokemon, 40)`): the whole
    /// damage formula for a 40-power typeless physical pseudo-move the Pokémon uses on itself,
    /// so its Attack modifiers (Choice Band, Huge Power), base-power ones (Technician) and a
    /// burn all apply; it never crits and draws the usual damage roll.
    pub(crate) fn confusion_self_hit_damage(&mut self, pokemon: PokemonId) -> u16 {
        let pseudo_move = ResolvedMove {
            id: crate::ids::MoveId(0),
            name: "confused".into(),
            base_power: 40,
            accuracy: None,
            priority: 0,
            category: crate::dex::MoveCategory::Physical,
            // The cartridge's self-hit is Struggle's data (pokeplatinum `MOVE_STRUGGLE`), which
            // Slow Start reads; Showdown's pseudo-move has none (issues.md I177).
            data_category: crate::dex::MoveCategory::Physical,
            move_type: crate::dex::PokemonType::Typeless,
            target: crate::rules::TargetPattern::SelfOnly,
            is_spread: false,
            max_pp: 0,
            primary_effect: crate::dex::PrimaryEffect::Damage,
            flags: crate::dex::MoveFlags {
                protectable: false,
                bypass_substitute: true,
                contact: false,
                type_immunity: false,
                force_stab: false,
                immunity_ignored: false,
                sheer_force: false,
                sheer_force_boost: false,
                parental_bond: false,
                parental_bond_exempt: false,
                future_move: false,
                powder: false,
                defrost: false,
                fail_encore: false,
                heal: false,
                cant_use_twice: false,
                sound: false,
                bullet: false,
                punch: false,
                bite: false,
                pulse: false,
                slicing: false,
                gravity: false,
                stalling: false,
                wind: false,
                minimize: false,
                must_pressure: false,
                sleep_usable: false,
                callable: false,
                no_assist: false,
                no_sleep_talk: false,
                fail_copycat: false,
                fail_mimic: false,
                no_sketch: false,
                fail_instruct: false,
                fail_me_first: false,
                mirror: false,
                reflectable: false,
                snatch: false,
                dance: false,
                charge: false,
                recharge: false,
                pledge_combo: false,
            },
            secondaries: Vec::new(),
            has_secondaries: false,
            multi_hit: crate::dex::MultiHit::Single,
            recoil: None,
            drain: None,
            self_switch: None,
            force_switch: false,
            crit_ratio: 1,
            will_crit: Some(false),
            no_damage_variance: false,
            multi_accuracy: false,
            tracks_target: false,
            calls_move: false,
            breaks_protect: false,
            steals_boosts: false,
            smart_target: false,
            non_ghost_target: None,
            fixed_damage: None,
            self_destruct: None,
            ohko: None,
            stat_sources: crate::dex::StatSources::default(),
            thaws_target: false,
            ignores_ability: false,
            z_crystal: None,
            is_max: false,
            z_power: None,
            z_status: None,
            max_power: None,
            behavior: crate::effects::moves::MoveBehavior::default(),
            unsupported: Vec::new(),
        };
        let breakdown = match self.rules.damage.algorithm {
            DamageAlgorithm::Cartridge(profile) => self.resolve_cartridge_damage(
                pokemon,
                pokemon,
                &pseudo_move,
                1,
                Fraction::ONE,
                profile,
                true,
            ),
            _ => self.resolve_standard_damage(pokemon, pokemon, &pseudo_move, 1, Fraction::ONE),
        };
        breakdown.final_damage.0
    }

    /// Every `ModifyAtk` / `ModifySpA` participant of the hit, unsorted, with its 4096-based
    /// modifier: the user's ability (`onModifyAtk`, data boosts), the target's (Thick Fat's
    /// `onSourceModifyAtk`), a Ruin ability, a Paradox volatile, the user's item, its species
    /// item and Gen 3's type-boosting item.
    pub(super) fn offense_participants(
        &self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        actor_ability: Option<crate::ids::AbilityId>,
        actor_item: Option<crate::ids::ItemId>,
    ) -> Vec<(ModifierDescriptor, u32)> {
        let mut offense_participants: Vec<(ModifierDescriptor, u32)> = Vec::new();
        {
            let mut ctx =
                OffensiveStatContext::new(self.state, self.rules, actor, target, move_def);
            if let Some(ability) = actor_ability {
                let behavior = crate::engine::ability_behavior(self.rules, ability);
                let source = ModifierSource::AbilityInstance {
                    holder: actor,
                    ability,
                };
                if let Some(ordered) = behavior.modify_offense
                    && let Some(modifier) = effects::abilities::modify_offense(behavior, &mut ctx)
                {
                    offense_participants.push((
                        ModifierDescriptor::new(
                            ModifierStage::OffensiveStat,
                            source,
                            ordered.priority,
                        ),
                        modifier,
                    ));
                }
                // Gorilla Tactics' `onModifyAtk`: no boost while the holder is Dynamaxed
                // (§44.5).
                if let Some(offense) = behavior.offense_boost
                    && !(behavior.boost_suspended_by_dynamax
                        && crate::engine::dynamaxed(self.state, actor))
                    && let Some(modifier) =
                        offense.modifier_for(self.state, self.rules, actor, target, move_def)
                {
                    offense_participants.push((
                        ModifierDescriptor::new(
                            ModifierStage::OffensiveStat,
                            source,
                            offense.priority(self.rules),
                        ),
                        modifier,
                    ));
                }
            }
            // The target's ability (`onSourceModifyAtk` / `onSourceModifySpA`: Thick Fat).
            if let Some(ability) = self
                .state
                .pokemon(target)
                .and_then(|p| crate::engine::ability_running(self.state, self.rules, p))
                && let Some(boost) =
                    crate::engine::ability_behavior(self.rules, ability).foe_offense
                && let Some(modifier) =
                    boost.modifier_for(self.state, self.rules, target, actor, move_def)
            {
                offense_participants.push((
                    ModifierDescriptor::new(
                        ModifierStage::OffensiveStat,
                        ModifierSource::AbilityInstance {
                            holder: target,
                            ability,
                        },
                        boost.priority,
                    ),
                    modifier,
                ));
            }
            // Tablets / Vessel of Ruin: the stat the event modifies follows the move's category
            // (Body Press's Defense is modified as Attack).
            let stat = if move_def.category == MoveCategory::Physical {
                crate::state::Stat::Attack
            } else {
                crate::state::Stat::SpecialAttack
            };
            if let Some(entry) = self.ruin(actor, stat, ModifierStage::OffensiveStat) {
                offense_participants.push(entry);
            }
            // Flower Gift on the user's side (`onAllyModifyAtk`, priority 3): Attack only.
            if stat == crate::state::Stat::Attack {
                offense_participants.extend(self.flower_gift(
                    actor,
                    ModifierStage::OffensiveStat,
                    3,
                ));
            }
            // Quark Drive's / Protosynthesis's volatile on the user's raised stat (`onModifyAtk`
            // / `onModifySpA`, priority 5).
            if let Some(entry) = self.paradox_boost(actor, stat, ModifierStage::OffensiveStat, 5) {
                offense_participants.push(entry);
            }
            // Flash Fire's volatile on a Fire move (Gen 5+ `onModifyAtk` / `onModifySpA`,
            // priority 5).
            if self.rules.conditions.flash_fire.boost == crate::rules::FlashFireBoost::OffensiveStat
                && self.flash_fire_boosts(actor, move_def)
            {
                offense_participants.push((
                    ModifierDescriptor::new(
                        ModifierStage::OffensiveStat,
                        ModifierSource::Volatile(crate::state::VolatileConditionKind::FlashFire),
                        5,
                    ),
                    6144,
                ));
            }
            if let Some(item) = actor_item {
                let behavior = crate::engine::item_behavior(self.rules, item);
                // The Choice items' `onModifyAtk` / `onModifySpA`: no boost while the holder is
                // Dynamaxed (§44.5).
                if let Some(ordered) = behavior.modify_offense
                    && !(behavior.boosts_suspended_by_dynamax
                        && crate::engine::dynamaxed(self.state, actor))
                    && let Some(modifier) = effects::items::modify_offense(behavior, &mut ctx)
                {
                    offense_participants.push((
                        ModifierDescriptor::new(
                            ModifierStage::OffensiveStat,
                            ModifierSource::ItemInstance {
                                holder: actor,
                                item,
                            },
                            ordered.priority,
                        ),
                        modifier,
                    ));
                }
                // A species' own item (Light Ball, Thick Club, Deep Sea Tooth …), on the stat
                // the event modifies.
                let boost = if stat == crate::state::Stat::Attack {
                    behavior.attack_boost
                } else {
                    behavior.special_attack_boost
                };
                if let Some(boost) = boost
                    && let Some(modifier) = self
                        .state
                        .pokemon(actor)
                        .and_then(|p| boost.modifier_for(self.rules, p))
                {
                    offense_participants.push((
                        ModifierDescriptor::new(
                            ModifierStage::OffensiveStat,
                            ModifierSource::ItemInstance {
                                holder: actor,
                                item,
                            },
                            boost.priority,
                        ),
                        modifier,
                    ));
                }
                // Gen 3: a type-boosting item multiplies the attacking stat (×1.1, Sea Incense
                // ×1.05): the gen3 mod's `onModifyAtk` / `onModifySpA` (priority 1), pokeemerald's
                // `sHoldEffectToType` in `CalculateBaseDamage` — a Guts holder's Silk Scarf comes
                // before the Guts (R-M11-79, issues.md I218).
                if self.rules.damage.type_item_stage == TypeItemStage::OffensiveStat
                    && let Some(boost) = behavior.base_power_boost
                    && boost.filter == effects::items::BoostFilter::MoveType(move_def.move_type)
                {
                    offense_participants.push((
                        ModifierDescriptor::new(
                            ModifierStage::OffensiveStat,
                            ModifierSource::ItemInstance {
                                holder: actor,
                                item,
                            },
                            1,
                        ),
                        boost.stat_numerator,
                    ));
                }
            }
        }
        offense_participants
    }

    /// Every `ModifyDef` / `ModifySpD` participant on the stat the move reads, unsorted: the
    /// target's ability on its Defense (`onModifyDef`: Marvel Scale, Fur Coat), Sword / Beads of
    /// Ruin, a Paradox volatile, the target's item (Metal Powder, Deep Sea Scale, Gen 3-6 Soul
    /// Dew, Eviolite).
    pub(super) fn defense_participants(
        &self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
    ) -> Vec<(ModifierDescriptor, u32)> {
        let defends_with_defense = match move_def.stat_sources.defense_stat {
            Some(stat) => stat == crate::engine::Stat::Defense,
            None => move_def.category == MoveCategory::Physical,
        };
        let mut defense_participants: Vec<(ModifierDescriptor, u32)> = Vec::new();
        if defends_with_defense
            && let Some(ability) = self
                .state
                .pokemon(target)
                .and_then(|p| crate::engine::ability_running(self.state, self.rules, p))
            && let Some(boost) = crate::engine::ability_behavior(self.rules, ability).defense_boost
            && let Some(modifier) =
                boost.modifier_for(self.state, self.rules, target, actor, move_def)
        {
            defense_participants.push((
                ModifierDescriptor::new(
                    ModifierStage::DefensiveStat,
                    ModifierSource::AbilityInstance {
                        holder: target,
                        ability,
                    },
                    boost.priority,
                ),
                modifier,
            ));
        }
        let stat = if defends_with_defense {
            crate::state::Stat::Defense
        } else {
            crate::state::Stat::SpecialDefense
        };
        if let Some(entry) = self.ruin(target, stat, ModifierStage::DefensiveStat) {
            defense_participants.push(entry);
        }
        if let Some(entry) = self.paradox_boost(target, stat, ModifierStage::DefensiveStat, 6) {
            defense_participants.push(entry);
        }
        // Flower Gift on the target's side (`onAllyModifySpD`, priority 4).
        if stat == crate::state::Stat::SpecialDefense {
            defense_participants.extend(self.flower_gift(target, ModifierStage::DefensiveStat, 4));
        }
        // The target's item (Metal Powder, Deep Sea Scale, Gen 3-6 Soul Dew).
        if let Some(pokemon) = self.state.pokemon(target)
            && let Some((item, behavior)) = crate::engine::held_item(self.state, self.rules, target)
        {
            let boost = if defends_with_defense {
                behavior.defense_boost
            } else {
                behavior.special_defense_boost
            };
            if let Some(boost) = boost
                && let Some(modifier) = boost.modifier_for(self.rules, pokemon)
            {
                defense_participants.push((
                    ModifierDescriptor::new(
                        ModifierStage::DefensiveStat,
                        ModifierSource::ItemInstance {
                            holder: target,
                            item,
                        },
                        boost.priority,
                    ),
                    modifier,
                ));
            }
            // Eviolite: either defense of a holder that is not fully evolved.
            if let Some(boost) = behavior.unevolved_defenses
                && pokemon
                    .species
                    .and_then(|species| self.dex.species_by_id(species))
                    .is_some_and(|species| species.nfe)
            {
                defense_participants.push((
                    ModifierDescriptor::new(
                        ModifierStage::DefensiveStat,
                        ModifierSource::ItemInstance {
                            holder: target,
                            item,
                        },
                        boost.priority,
                    ),
                    boost.numerator,
                ));
            }
        }
        defense_participants
    }

    /// The target's resist berry (`onSourceModifyDamage`): a hit of its type — super effective
    /// but for Chilan Berry — on the body, not a Substitute, makes the holder eat it (`eatItem`,
    /// in the middle of the damage calculation). Whether it was eaten.
    pub(super) fn eat_resist_berry(
        &mut self,
        target: PokemonId,
        move_def: &ResolvedMove,
        type_effectiveness: Fraction,
    ) -> bool {
        self.state.pokemon(target).is_some_and(|pokemon| {
            crate::engine::held_item(self.state, self.rules, target)
                .and_then(|(_, item)| item.weakening_berry)
                .is_some_and(|berry| {
                    move_def.move_type == berry.move_type
                        && (!berry.super_effective_only
                            || type_effectiveness.numerator > type_effectiveness.denominator)
                })
                && !(pokemon
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::Substitute)
                    && !move_def.flags.bypass_substitute)
        }) && crate::engine::BattleContext::new(self.state, self.rules, self.trace)
            .eat_item(target)
            .is_some()
    }

    /// A Ruin ability lowering `pokemon`'s `stat` in this damage calculation (Showdown
    /// `onAnyModifyAtk` … `onAnyModifySpD`, priority 0): an active holder, unless `pokemon`
    /// has that ability itself.
    fn ruin(
        &self,
        pokemon: PokemonId,
        stat: crate::state::Stat,
        stage: ModifierStage,
    ) -> Option<(ModifierDescriptor, u32)> {
        let own = self
            .state
            .pokemon(pokemon)
            .and_then(|p| crate::engine::ability_handler(self.state, self.rules, p, |b| b.ruin));
        if own == Some(stat) {
            return None;
        }
        let (holder, ability) =
            crate::engine::active_holder(self.state, self.rules, |b| b.ruin == Some(stat))?;
        Some((
            ModifierDescriptor::new(
                stage,
                ModifierSource::AbilityInstance { holder, ability },
                0,
            ),
            effects::abilities::RUIN,
        ))
    }

    /// Flower Gift (§23.6): ×1.5 for `pokemon` in the sun it sees, from each active holder on its
    /// side (itself included; Gen 5+ a Cherrim), or once for them all
    /// (`FlowerGiftRules::once_per_side`).
    fn flower_gift(
        &self,
        pokemon: PokemonId,
        stage: ModifierStage,
        priority: i16,
    ) -> Vec<(ModifierDescriptor, u32)> {
        let rules = self.rules.weather.flower_gift;
        let Some(side) = self.state.pokemon(pokemon).map(|p| p.side) else {
            return Vec::new();
        };
        if crate::engine::weather_for(self.state, self.rules, pokemon)
            != Some(crate::state::WeatherKind::Sun)
        {
            return Vec::new();
        }
        let mut boosts: Vec<(ModifierDescriptor, u32)> = self
            .state
            .side(side)
            .active
            .iter()
            .flatten()
            .filter_map(|holder| self.state.pokemon(*holder))
            .filter(|holder| {
                !holder.is_fainted()
                    && if stage != ModifierStage::DefensiveStat && rules.attack_ignores_mold_breaker
                    {
                        crate::engine::active_ability_handler(self.state, self.rules, holder, |b| {
                            b.flower_gift
                        })
                    } else {
                        crate::engine::ability_handler(self.state, self.rules, holder, |b| {
                            b.flower_gift
                        })
                    }
                    && (!rules.holder_must_be_cherrim
                        || matches!(
                            holder.species,
                            Some(
                                crate::ids::SpeciesId::CHERRIM
                                    | crate::ids::SpeciesId::CHERRIM_SUNSHINE
                            )
                        ))
            })
            .map(|holder| {
                (
                    ModifierDescriptor::new(
                        stage,
                        ModifierSource::AbilityInstance {
                            holder: holder.id(),
                            ability: crate::engine::raw_ability(holder),
                        },
                        priority,
                    ),
                    6144,
                )
            })
            .collect();
        if rules.once_per_side {
            boosts.truncate(1);
        }
        boosts
    }

    /// Quark Drive's or Protosynthesis's volatile raising `pokemon`'s `stat` in this damage
    /// calculation (×5325 / 4096 at `priority`: 5 for Attack / Special Attack, 6 for the
    /// defenses) — not while `pokemon`'s ability is ignored (its handlers ask `ignoringAbility()`:
    /// Neutralizing Gas leaves the volatile in place).
    /// Whether `actor`'s Flash Fire boost applies to `move_def` (a Fire move).
    pub(super) fn flash_fire_boosts(&self, actor: PokemonId, move_def: &ResolvedMove) -> bool {
        move_def.move_type == crate::dex::PokemonType::Fire
            && self.state.pokemon(actor).is_some_and(|p| {
                p.volatiles
                    .contains(crate::state::VolatileConditionKind::FlashFire)
            })
    }

    fn paradox_boost(
        &self,
        pokemon: PokemonId,
        stat: crate::state::Stat,
        stage: ModifierStage,
        priority: i16,
    ) -> Option<(ModifierDescriptor, u32)> {
        let holder = self.state.pokemon(pokemon)?;
        let (kind, raised) = holder.volatiles.paradox_boost()?;
        (raised == stat && !crate::engine::ignoring_ability(self.state, self.rules, holder)).then(
            || {
                (
                    ModifierDescriptor::new(stage, ModifierSource::Volatile(kind), priority),
                    5325,
                )
            },
        )
    }

    /// Showdown `runEffectiveness`'s Tera Shell (R-M9-45): a move's first hit forgets what an
    /// earlier move left (`move.hit === 1`); a hit it resists is announced (`-activate`) and
    /// leaves the move's later hits resisted, whatever HP is left by then.
    fn tera_shell(
        &mut self,
        target: PokemonId,
        move_def: &ResolvedMove,
        effectiveness: Fraction,
    ) -> Fraction {
        if !super::hit_gate::tera_shell_holds(self.state, self.rules, target) {
            return effectiveness;
        }
        if self.hit <= 1 {
            self.tera_shell_resisted
                .retain(|pokemon| *pokemon != target);
        }
        if self.tera_shell_resisted.contains(&target) {
            return Fraction::new(1, 2);
        }
        if !super::hit_gate::tera_shell_resists(
            self.state,
            self.rules,
            target,
            move_def,
            effectiveness,
        ) {
            return effectiveness;
        }
        self.trace.push(TraceEvent::AbilityActivated {
            source: target,
            ability: crate::ids::AbilityId::TERA_SHELL,
        });
        self.tera_shell_resisted.push(target);
        Fraction::new(1, 2)
    }

    /// The type effectiveness a hit's `DamagingHit` reactions read (Showdown
    /// `getMoveHitData(move).typeMod`): Tera Shell's ×0.5 once it resisted the move (R-M9-45).
    pub(super) fn hit_effectiveness(&self, target: PokemonId, effectiveness: Fraction) -> Fraction {
        if self.tera_shell_resisted.contains(&target) {
            Fraction::new(1, 2)
        } else {
            effectiveness
        }
    }

    pub(super) fn resolve_standard_damage(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        target_count: usize,
        type_effectiveness: Fraction,
    ) -> DamageBreakdown {
        // Psywave draws its damage in place of the formula (Showdown `damageCallback`).
        if move_def.fixed_damage == Some(crate::dex::FixedDamage::Psywave) {
            let level = self.state.pokemon(actor).map_or(1, |p| p.level);
            let (min, max) = crate::engine::psywave_range(self.rules, level);
            let roll = self.state.rng.range_inclusive_for(
                crate::state::RandomRequestKind::DamageRoll,
                min,
                max,
            );
            let amount = crate::engine::psywave_damage(self.rules, level, roll);
            return self.fixed_damage_breakdown(actor, target, move_def, amount);
        }
        // Gen 2-3 Future Sight / Doom Desire (§15.22): the damage computed at use, its random
        // factor drawn as it strikes.
        if let Some(crate::dex::FixedDamage::Foreseen(base)) = move_def.fixed_damage {
            let amount = self.foreseen_damage(base);
            return self.fixed_damage_breakdown(actor, target, move_def, amount);
        }
        // Gen 1 Counter (§15.8): twice the battle's last damage, or it fails (0).
        if self.rules.conditions.retaliation.gen1_last_damage
            && move_def.fixed_damage
                == Some(crate::dex::FixedDamage::Retaliation(
                    crate::dex::Retaliation::Counter,
                ))
        {
            let amount =
                crate::engine::gen1_counter_damage(self.state, self.dex, target).unwrap_or(0);
            return self.fixed_damage_breakdown(actor, target, move_def, amount);
        }
        // Fixed damage and one-hit KOs return before the critical-hit check (Showdown
        // `getDamage`): no critical hit, no random roll, no modifier.
        if let Some(amount) =
            crate::engine::fixed_damage(self.state, self.rules, actor, target, move_def)
        {
            // Final Gambit's `damageCallback` knocks its user out as it takes the damage.
            if move_def.fixed_damage == Some(crate::dex::FixedDamage::UserHp) {
                crate::engine::BattleContext::new(self.state, self.rules, self.trace)
                    .faint_by_own_move(actor, move_def.id);
            }
            return self.fixed_damage_breakdown(actor, target, move_def, amount);
        }
        match self.rules.damage.algorithm {
            DamageAlgorithm::Gen1Legacy | DamageAlgorithm::Gen2 => {
                return self.resolve_legacy_damage(actor, target, move_def, type_effectiveness);
            }
            DamageAlgorithm::Cartridge(profile) => {
                return self.resolve_cartridge_damage(
                    actor,
                    target,
                    move_def,
                    target_count,
                    type_effectiveness,
                    profile,
                    false,
                );
            }
            DamageAlgorithm::Modern => {}
        }
        let actor_ability = self
            .state
            .pokemon(actor)
            .and_then(|pokemon| crate::engine::ability_running(self.state, self.rules, pokemon));
        let actor_item = crate::engine::held_item_id(self.state, self.rules, actor);

        // Showdown `getDamage` runs the critical-hit check before the BasePower event (a
        // participant may roll: Fickle Beam).
        let critical = self.roll_critical(actor, target, move_def);

        let power = self.resolve_base_power(actor, target, move_def, actor_item);
        // Showdown `getDamage`: a BasePower event that leaves 0 deals 0 (`if (!basePower)
        // return 0`, before the floor of 1 and the hacked Max Move's 0): still a hit.
        if power.0 == 0 {
            return self.fixed_damage_breakdown(actor, target, move_def, 0);
        }
        // Terastallization's 60-power floor (Showdown `getDamage`, DESIGN.md §44.1): a
        // Terastallized Pokémon's move of its Tera type (Stellar: a type whose boost it has not
        // used), below 60 power, non-priority and single-hit, with no variable power of its own.
        let power = {
            // "Hacked Max Moves have 0 base power, even if you Dynamax" (Showdown `getDamage`): a
            // Max Move in a move slot, used without the Dynamax that makes it one, strikes for
            // the minimum.
            if move_def.is_max && !self.z_powered() {
                BasePower(0)
            } else {
                let tera = self
                    .state
                    .pokemon(actor)
                    .map(|pokemon| (pokemon.terastallized, &pokemon.stellar_boosted_types));
                let tera_applies = match tera {
                    Some((Some(PokemonType::Stellar), boosted)) => {
                        !boosted.contains(&move_def.move_type)
                    }
                    Some((Some(_), _)) => self.state.pokemon(actor).is_some_and(|pokemon| {
                        crate::engine::types_of(self.state, self.rules, pokemon)
                            .contains(move_def.move_type)
                    }),
                    _ => false,
                };
                let variable_power = matches!(move_def.base_power, 0 | 150)
                    && move_def.behavior.power_formula.is_some();
                if tera_applies
                    && power.0 < 60
                    && move_def.priority <= 0
                    && move_def.multi_hit == crate::dex::MultiHit::Single
                    && !variable_power
                {
                    BasePower(60)
                } else {
                    power
                }
            }
        };
        // Tera Shell (the end of Showdown `runEffectiveness`, R-M9-45): the type effectiveness
        // every later step reads — the chart's factor, the items and abilities that ask it, the
        // resist berry.
        let type_effectiveness = self.tera_shell(target, move_def, type_effectiveness);

        let base_offense =
            offensive_stat(self.state, self.rules, actor, target, move_def, critical.0);
        // Showdown `ModifyAtk` / `ModifySpA`: every participant's `chainModify`, in priority
        // order, chained and applied once with `modify` — to the stat as a handler that rounds
        // it itself leaves it (`offense_on_stat`: Hustle's `this.modify`), whatever its place in
        // the order (R-M10-65).
        let mut offense_participants =
            self.offense_participants(actor, target, move_def, actor_ability, actor_item);
        crate::engine::sort_by_showdown_priority(&mut offense_participants);
        let rules = self.rules;
        let on_stat = |descriptor: &ModifierDescriptor| {
            matches!(descriptor.source, ModifierSource::AbilityInstance { ability, .. }
                if crate::engine::ability_behavior(rules, ability).offense_on_stat)
        };

        let mut stat = base_offense.0;
        let mut chain = 4096;
        let mut after_ability = base_offense;
        let mut offense_applications = Vec::new();
        for (descriptor, modifier) in offense_participants {
            let before = showdown_modify(stat, chain);
            if on_stat(&descriptor) {
                stat = showdown_modify(stat, modifier);
            } else {
                chain = chain_modifiers(chain, modifier);
            }
            let value = crate::engine::OffensiveStat(showdown_modify(stat, chain));
            if let ModifierSource::AbilityInstance { .. } = descriptor.source {
                after_ability = value;
            }
            offense_applications.push((descriptor, before, value.0));
        }
        let mut offense = crate::engine::OffensiveStat(showdown_modify(stat, chain));
        // Gen 3-4 Beat Up (its volatile's `ModifyAtk` / `ModifySpA` at priority -101, modifier
        // reset to 1): the member's base Attack, whatever came before.
        let beat_up = self.beat_up_stats(target, move_def);
        if let Some((attack, _, _)) = beat_up {
            offense = crate::engine::OffensiveStat(attack);
        }
        if actor_ability.is_none() {
            after_ability = base_offense;
        }
        // The attacking stat the formula uses, every modifier in.
        let after_item = offense;
        for (descriptor, from, to) in offense_applications {
            self.trace.push(TraceEvent::ModifierApplied {
                stage: descriptor.stage,
                source: descriptor.source,
                priority: descriptor.order.priority,
                from,
                to,
            });
        }
        self.trace.push(TraceEvent::OffensiveStatResolved {
            actor,
            target,
            move_id: move_def.id,
            base: base_offense.0,
            after_ability: after_ability.0,
            after_item: after_item.0,
        });

        let mut defense =
            defensive_stat(self.state, self.rules, actor, target, move_def, critical.0);
        // Showdown `ModifyDef` / `ModifySpD` on the stat the move reads, chained in priority
        // order and applied once with `modify`.
        let mut defense_participants = self.defense_participants(actor, target, move_def);
        crate::engine::sort_by_showdown_priority(&mut defense_participants);
        let chain = defense_participants
            .iter()
            .fold(4096, |chain, (_, modifier)| {
                chain_modifiers(chain, *modifier)
            });
        defense = crate::engine::DefensiveStat(showdown_modify(defense.0, chain));
        // Gen 3-4 Explosion / Self-Destruct: the Defense after its modifiers, halved.
        let defends_with_defense = match move_def.stat_sources.defense_stat {
            Some(stat) => stat == crate::engine::Stat::Defense,
            None => move_def.category == MoveCategory::Physical,
        };
        if move_def.behavior.halves_defense
            && self.rules.conditions.self_faint.explosion_halves_defense
            && defends_with_defense
        {
            defense = crate::engine::DefensiveStat((defense.0 / 2).max(1));
        }
        // Gen 3-4 Beat Up (`onFoeModifyDef` / `onFoeModifySpD`, priority -101): the target's
        // base Defense.
        if let Some((_, _, base_defense)) = beat_up {
            defense = crate::engine::DefensiveStat(base_defense);
        }
        self.trace.push(TraceEvent::DefensiveStatResolved {
            actor,
            target,
            move_id: move_def.id,
            value: defense.0,
        });

        let level = self.state.pokemon(actor).map_or(1, |pokemon| pokemon.level);
        let base = base_damage(self.rules.damage.algorithm, level, power, offense, defense);

        let weather_modifier =
            environment::weather_damage_modifier(self.state, self.rules, actor, target, move_def);
        self.trace.push(TraceEvent::WeatherModifierResolved {
            actor,
            move_id: move_def.id,
            weather: self.state.field.weather.map(|weather| weather.kind),
            modifier: weather_modifier,
        });

        // STAB (`getDamage`): the generation's, doubled by a matching Tera type, Stellar's
        // one-time per-type boost, then `onModifySTAB` (Adaptability, DESIGN.md §44.1).
        let stab_modifier = match self.state.pokemon(actor) {
            Some(pokemon) => {
                let multiplier = crate::engine::stab_modifier(
                    self.state,
                    self.rules,
                    pokemon,
                    move_def.move_type,
                    // The Pledge combos' `forceSTAB` (family L): the move counts as the user's
                    // own type (`onModifySTAB`'s `stab`).
                    move_def.flags.force_stab,
                );
                // Stellar's boost is used up for the type (`getDamage`'s `stellarBoostedTypes`),
                // except for a Terapagos-Stellar, which keeps it for every move; the move under
                // way keeps the note for its other targets.
                if multiplier.stellar_boosted {
                    if let Some(active) = self.state.active_move.as_mut() {
                        active.stellar_boosted = true;
                    }
                    let stellar_move =
                        crate::engine::species_of(self.state.pokemon(actor).expect("the actor"))
                            == Some(crate::ids::SpeciesId::TERAPAGOS_STELLAR);
                    if !stellar_move
                        && let Some(pokemon) = self.state.pokemon_mut(actor)
                        && !pokemon.stellar_boosted_types.contains(&move_def.move_type)
                    {
                        pokemon.stellar_boosted_types.push(move_def.move_type);
                    }
                }
                multiplier.modifier
            }
            None => Fraction::ONE,
        };
        self.trace.push(TraceEvent::StabResolved {
            actor,
            move_id: move_def.id,
            move_type: move_def.move_type,
            modifier: stab_modifier,
        });

        let spread_applied = move_def.is_spread
            && target_count > 1
            && (!self.rules.damage.spread_only_foes
                || move_def.target == crate::rules::TargetPattern::AllAdjacentFoes);
        let spread = if spread_applied {
            self.rules.damage.spread_modifier
        } else {
            Fraction::ONE
        };
        // Parental Bond's second hit, where the spread modifier would be (never both).
        let bond = if move_def.flags.parental_bond && self.hit > 1 {
            self.rules.damage.parental_bond_second_hit
        } else {
            Fraction::ONE
        };
        let critical_modifier = if critical.0 {
            self.rules.damage.critical_multiplier
        } else {
            Fraction::ONE
        };
        // Facade (Gen 6+) and Guts: a burned user's damage is not halved.
        let burn_exempt = (self.rules.damage.move_can_ignore_burn
            && move_def.behavior.ignores_burn_drop)
            || actor_ability.is_some_and(|ability| {
                crate::engine::ability_behavior(self.rules, ability).ignores_burn_drop
            });
        let burn_applied = move_def.category == MoveCategory::Physical
            && !burn_exempt
            && self
                .state
                .pokemon(actor)
                .is_some_and(|pokemon| pokemon.has_status(NonVolatileStatus::Burn));
        let burn = if burn_applied {
            self.rules.damage.burn_modifier
        } else {
            Fraction::ONE
        };
        // Showdown `noDamageVariance`: the top of the range, nothing drawn.
        let random_roll = if move_def.no_damage_variance {
            self.rules.damage.random_max
        } else {
            self.state.rng.range_inclusive_for(
                RandomRequestKind::DamageRoll,
                self.rules.damage.random_min,
                self.rules.damage.random_max,
            )
        };
        let random_percent = ((u32::from(random_roll) * 100)
            / u32::from(self.rules.damage.random_denominator.max(1)))
            as u16;

        // Showdown's `ModifyDamage` step (Gen 3+), one chained 4096-based modifier rounded once:
        // Gust / Twister against a Pokémon in the air, Earthquake / Magnitude against one
        // underground; the user's Life Orb; Expert Belt on a super-effective hit.
        let hiding_bonus = self.state.pokemon(target).map_or(Fraction::ONE, |t| {
            crate::engine::two_turn::damage_bonus(
                self.rules,
                self.dex,
                t,
                move_def,
                DamageBonusStage::FinalDamage,
            )
        });
        let mut modify_damage: u32 = 4096;
        if hiding_bonus != Fraction::ONE {
            modify_damage = chain_modifiers(
                modify_damage,
                u32::from(hiding_bonus.numerator) * 4096
                    / u32::from(hiding_bonus.denominator.max(1)),
            );
        }
        // The target's Minimize (its condition's `onSourceModifyDamage`): a `minimize` move ×2.
        if crate::engine::minimize_bonus(self.state, self.rules, target, move_def)
            == Some(DamageBonusStage::FinalDamage)
        {
            modify_damage = chain_modifiers(modify_damage, 8192);
        }
        // The target's Glaive Rush (its condition's `onSourceModifyDamage`, §15.20): ×2.
        if self.state.pokemon(target).is_some_and(|p| {
            p.volatiles
                .contains(crate::state::VolatileConditionKind::GlaiveRush)
        }) {
            modify_damage = chain_modifiers(modify_damage, 8192);
        }
        // The user's Metronome count (its `metronome` condition's `onModifyDamage`, Gen 5+).
        if let crate::rules::MetronomeRules::Showdown { modifiers } = self.rules.damage.metronome
            && let Some((_, count)) = self
                .state
                .pokemon(actor)
                .and_then(|pokemon| pokemon.volatiles.metronome())
            && let Some(modifier) = modifiers
                .get(usize::from(count))
                .or_else(|| modifiers.last())
        {
            modify_damage = chain_modifiers(modify_damage, *modifier);
        }
        // The user's item (Life Orb; Expert Belt on a super-effective hit).
        if let Some(item) = actor_item
            && let Some(boost) = crate::engine::item_behavior(self.rules, item).final_damage_boost
            && (!boost.only_super_effective
                || type_effectiveness.numerator > type_effectiveness.denominator)
        {
            modify_damage = chain_modifiers(modify_damage, boost.numerator);
        }
        // The user's ability (Sniper on a critical hit, Tinted Lens on a resisted one).
        if let Some(boost) = actor_ability
            .and_then(|ability| crate::engine::ability_behavior(self.rules, ability).damage_boost)
        {
            let applies = match boost.when {
                effects::abilities::DamageBoostWhen::Critical => critical.0,
                effects::abilities::DamageBoostWhen::NotVeryEffective => {
                    type_effectiveness.numerator < type_effectiveness.denominator
                }
                effects::abilities::DamageBoostWhen::SuperEffective => {
                    type_effectiveness.numerator > type_effectiveness.denominator
                }
            };
            if applies {
                modify_damage = chain_modifiers(modify_damage, boost.modifier);
            }
        }
        // The target's ability (`onSourceModifyDamage`: Filter, Multiscale, Fluffy …).
        if let Some(pokemon) = self.state.pokemon(target)
            && let Some(taken) =
                crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.damage_taken)
            && let Some(modifier) = taken.modifier(pokemon, move_def, type_effectiveness)
        {
            modify_damage = chain_modifiers(modify_damage, modifier);
        }
        // The target's resist berry (`onSourceModifyDamage`): a hit of its type — super effective
        // but for Chilan Berry — on the body, not a Substitute, makes the holder eat it
        // (`eatItem`, here in the middle of the damage calculation), and it is halved.
        let resist_berry_eaten = self.eat_resist_berry(target, move_def, type_effectiveness);
        if resist_berry_eaten {
            modify_damage = chain_modifiers(modify_damage, 2048);
        }
        // Friend Guard on the target's partners (Showdown `onAnyModifyDamage`, ×0.75).
        for ally in self.active_allies(target) {
            if let Some(modifier) = self.state.pokemon(ally).and_then(|p| {
                crate::engine::ability_handler(self.state, self.rules, p, |b| {
                    b.ally_damage_modifier
                })
            }) {
                modify_damage = chain_modifiers(modify_damage, modifier);
            }
        }
        // A screen on the target's side: Gen 3-4 in `ModifyDamagePhase1`, later last in the
        // ModifyDamage chain (a side's handler has no speed, so it sorts after the Pokémon's).
        let screen = crate::engine::screens::damage_modifier(
            self.state,
            self.rules,
            actor,
            target,
            move_def.category,
            critical.0,
        );
        let phase1_generation = self
            .rules
            .damage
            .modifier_order
            .contains(&DamageModifierKind::ModifyDamagePhase1);
        let mut phase1 = match screen {
            Some(modifier) if phase1_generation => modifier,
            Some(modifier) => {
                modify_damage = chain_modifiers(modify_damage, modifier);
                4096
            }
            None => 4096,
        };
        // Ripen (`onSourceModifyDamage`, priority −1: after the screen): while its `onEatItem`
        // has noted a resist berry — the one this hit made it eat, or one eaten before — the
        // hit is halved again and the note used up.
        if self.state.pokemon(target).is_some_and(|pokemon| {
            pokemon.ability_state.berry_weaken
                && crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.ripen)
        }) && let Some(pokemon) = self.state.pokemon_mut(target)
        {
            pokemon.ability_state.berry_weaken = false;
            modify_damage = chain_modifiers(modify_damage, 2048);
        }
        // Gen 3's Flash Fire (the gen4 condition's `onModifyDamagePhase1`), in the same event as
        // the screen.
        if self.rules.conditions.flash_fire.boost
            == crate::rules::FlashFireBoost::ModifyDamagePhase1
            && self.flash_fire_boosts(actor, move_def)
        {
            phase1 = chain_modifiers(phase1, 6144);
        }
        let weather_source = self
            .state
            .field
            .weather
            .map(|weather| ModifierSource::Weather(weather.kind))
            .unwrap_or(ModifierSource::Rule("Weather"));
        let order = |kind: DamageModifierKind| -> i16 {
            self.rules
                .damage
                .modifier_order
                .iter()
                .position(|candidate| *candidate == kind)
                .map_or(i16::MAX, |index| index as i16)
        };
        let mut final_modifiers = vec![
            FinalDamageModifier::offset(
                ModifierSource::Rule("BaseOffset"),
                order(DamageModifierKind::BaseOffset),
                2,
            ),
            // Showdown `modify(baseDamage, 0.75)`: 4096-based and rounded, not floored (57
            // gives 43).
            FinalDamageModifier::chain(
                ModifierSource::Rule("Spread"),
                order(DamageModifierKind::Spread),
                fraction_4096(u32::from(spread.numerator), u32::from(spread.denominator)),
            ),
            FinalDamageModifier::chain(
                ModifierSource::Rule("ParentalBond"),
                order(DamageModifierKind::Spread),
                fraction_4096(u32::from(bond.numerator), u32::from(bond.denominator)),
            ),
            FinalDamageModifier::new(
                weather_source,
                order(DamageModifierKind::Weather),
                weather_modifier,
            ),
            FinalDamageModifier::new(
                ModifierSource::Rule("Critical"),
                order(DamageModifierKind::Critical),
                critical_modifier,
            ),
            FinalDamageModifier::chain(
                ModifierSource::Rule("ModifyDamagePhase1"),
                order(DamageModifierKind::ModifyDamagePhase1),
                phase1,
            ),
            FinalDamageModifier::new(
                ModifierSource::Rule("Random"),
                order(DamageModifierKind::Random),
                Fraction::new(random_roll, self.rules.damage.random_denominator),
            ),
            // Showdown `modify(baseDamage, stab)`: 4096-based, rounded half down. For 1.5 and 2
            // that is the floored product. Stellar's 4915/4096 and a Terastallized
            // Adaptability's 2.25 round up from about x.5: Stellar's 60 becomes 72, not 71
            // (DESIGN.md R-M9-19).
            FinalDamageModifier::chain(
                ModifierSource::Rule("STAB"),
                order(DamageModifierKind::Stab),
                fraction_4096(
                    u32::from(stab_modifier.numerator),
                    u32::from(stab_modifier.denominator),
                ),
            ),
            FinalDamageModifier::new(
                ModifierSource::Rule("TypeEffectiveness"),
                order(DamageModifierKind::TypeEffectiveness),
                type_effectiveness,
            ),
            FinalDamageModifier::new(
                if burn_applied {
                    ModifierSource::Status(NonVolatileStatus::Burn)
                } else {
                    ModifierSource::Rule("Burn")
                },
                order(DamageModifierKind::Burn),
                burn,
            ),
            FinalDamageModifier::chain(
                ModifierSource::Rule("ModifyDamage"),
                order(DamageModifierKind::ModifyDamage),
                modify_damage,
            ),
        ];
        if move_def.category == MoveCategory::Physical
            && self
                .rules
                .damage
                .modifier_order
                .contains(&DamageModifierKind::PhysicalMinimum)
        {
            final_modifiers.push(FinalDamageModifier::at_least(
                ModifierSource::Rule("PhysicalMinimum"),
                order(DamageModifierKind::PhysicalMinimum),
                1,
            ));
        }
        // Gen 5 raises the damage to 1 before `ModifyDamage`, and not again after it.
        let early_minimum = self
            .rules
            .damage
            .modifier_order
            .contains(&DamageModifierKind::Minimum);
        if early_minimum {
            final_modifiers.push(FinalDamageModifier::at_least(
                ModifierSource::Rule("Minimum"),
                order(DamageModifierKind::Minimum),
                1,
            ));
        }
        // Behemoth Blade / Behemoth Bash / Dynamax Cannon against a Dynamaxed target: double
        // (`dynamax.onSourceModifyDamage`, DESIGN.md §44.5).
        if matches!(
            move_def.id,
            crate::ids::MoveId::BEHEMOTH_BLADE
                | crate::ids::MoveId::BEHEMOTH_BASH
                | crate::ids::MoveId::DYNAMAX_CANNON
        ) && self.state.pokemon(target).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .contains(crate::state::VolatileConditionKind::Dynamax)
        }) {
            final_modifiers.push(FinalDamageModifier::chain(
                ModifierSource::Rule("Dynamax"),
                order(DamageModifierKind::ModifyDamage),
                fraction_4096(2, 1),
            ));
        }
        // A Z-Move that reached a protected target: Showdown's `checkMoveBypassesProtect` marks
        // the hit `bypassProtect`, so the protection does not stop it but the damage is quartered
        // (`-zbroken`, after `ModifyDamage`, DESIGN.md §44.4) — its side's Mat Block or Quick
        // Guard too (`z_broke_protection`, R-M11-112).
        let priority = self.move_priority(move_def);
        if self.z_powered()
            && move_def.category != MoveCategory::Status
            && self.state.pokemon(target).is_some_and(|pokemon| {
                crate::engine::z_broke_protection(self.state, pokemon, priority)
            })
        {
            final_modifiers.push(FinalDamageModifier::chain(
                ModifierSource::Rule("ZBroken"),
                order(DamageModifierKind::ModifyDamage) + 1,
                fraction_4096(1, 4),
            ));
            self.trace.push(TraceEvent::ProtectBroken { target });
        }
        let (final_damage, final_applications) = apply_final_modifiers(
            base,
            &mut final_modifiers,
            self.rules.damage.rounding,
            if early_minimum { 0 } else { 1 },
        );
        for application in final_applications {
            self.trace.push(TraceEvent::ModifierApplied {
                stage: application.descriptor.stage,
                source: application.descriptor.source,
                priority: application.descriptor.order.priority,
                from: application.from,
                to: application.to,
            });
        }

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
}
