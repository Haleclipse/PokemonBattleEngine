//! Gen 2's damage formula as its own strategy (DESIGN.md §31.2): Showdown's gen2 `getStat`
//! and `getDamage`, step for step. None of the modern pipeline's machinery applies — no
//! 4096-based chains, burn is part of the Attack stat, a critical hit doubles the damage and
//! decides which stages count, a high stat rolls over — so the algorithm is a separate
//! function rather than conditions inside the modern one. What it shares with the modern path
//! is the base power stage, the generation's type chart and weather classification, and the
//! order table (`DamageRules::modifier_order`, `GEN2_DAMAGE_ORDER`), which drives the steps.

use crate::dex::PokemonType;
use crate::effects::moves::BasePower;
use crate::ids::{ItemId, PokemonId};
use crate::rules::{DamageAlgorithm, DamageModifierKind, Fraction, ResolvedRules, StatRules};
use crate::state::{BattleState, NonVolatileStatus, PokemonState, RandomRequestKind, Stat};

use super::damage::base_damage;
use super::stats::stage_value;
use super::{BattleTrace, DefensiveStat, ModifierSource, ModifierStage, OffensiveStat, TraceEvent};

/// Which stat pair a hit reads (Showdown `atk`/`def` or `spa`/`spd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatPair {
    Physical,
    Special,
}

/// One Gen 2 damage calculation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LegacyHit {
    pub attacker: PokemonId,
    pub defender: PokemonId,
    pub base_power: u16,
    /// `None`: typeless (`???`: confusion's self-hit) — no weather, STAB or item boost.
    pub move_type: Option<PokemonType>,
    pub pair: StatPair,
    pub critical: bool,
    /// False for Showdown `noDamageVariance` (confusion's self-hit rolls nothing).
    pub variance: bool,
    pub type_effectiveness: Fraction,
    pub weather: Fraction,
    /// The attacker's type-boosting item when it matches the move (`onModifyDamage`, ×1.1).
    pub item_boost: Option<ItemId>,
    /// A `minimize` move against a Minimized defender: the final damage doubles
    /// (`DamageBonusStage::AfterRandom`).
    pub minimized: bool,
    /// Gen 2 Present's glitch (`MoveBehavior::type_index_damage`).
    pub type_index_damage: bool,
    /// Gen 2 Beat Up: the member's base Attack and level, the target's base Defense.
    pub beat_up: Option<(u32, u8, u32)>,
    /// Gen 1-2 Explosion / Self-Destruct: the Defense halved after the rollover (floored, at
    /// least 1).
    pub halves_defense: bool,
    /// Gen 2 Fury Cutter (pokecrystal `BattleCommand_FuryCutter`, §15.18) and Rage
    /// (`BattleCommand_RageDamage`, §15.19): the damage multiplied by this after the type
    /// matchups, before the random factor (at most 65535) — the move's doublings, the rage
    /// count + 1.
    pub multiplier: Option<(crate::ids::MoveId, u32)>,
}

/// The cartridge's internal index of a type (Gen 2), as Showdown's gen2 `getDamage` lists it
/// for Present; 0 (Normal) reads as 1 there (`|| 1`), and so here (issues.md I3).
fn gen2_type_index(pokemon_type: PokemonType) -> u32 {
    match pokemon_type {
        PokemonType::Fighting => 1,
        PokemonType::Flying => 2,
        PokemonType::Poison => 3,
        PokemonType::Ground => 4,
        PokemonType::Rock => 5,
        PokemonType::Bug => 7,
        PokemonType::Ghost => 8,
        PokemonType::Steel => 9,
        PokemonType::Fire => 20,
        PokemonType::Water => 21,
        PokemonType::Grass => 22,
        PokemonType::Electric => 23,
        PokemonType::Psychic => 24,
        PokemonType::Ice => 25,
        PokemonType::Dragon => 26,
        PokemonType::Dark => 27,
        _ => 1,
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LegacyDamage {
    pub offense: u32,
    pub defense: u32,
    pub base: u16,
    pub damage: u16,
    /// The random roll drawn (217..=255), if the damage was above 1.
    pub random: Option<u16>,
}

/// Showdown gen2 `getStat` for an offensive or defensive stat: the stage (unless a critical
/// hit ignores it), the burn halving of Attack, the 1..=999 clamp. (Reflect / Light Screen come
/// after it, in `gen2_damage`, then the species items, in `species_item_stat`.)
pub(crate) fn legacy_stat(
    rules: &StatRules,
    value: u16,
    stage: i8,
    burned_attack: bool,
    unboosted: bool,
) -> u32 {
    let mut stat = if unboosted {
        u32::from(value)
    } else {
        stage_value(rules.stage_table, value, stage)
    };
    if burned_attack {
        stat /= 2;
    }
    if let Some(cap) = rules.cap {
        stat = stat.clamp(1, u32::from(cap));
    }
    stat
}

/// The end of Showdown gen2 `getStat` (`StatRules::species_item_stats`): the holder's own
/// species item multiplies the stat, after the clamp (Metal Powder's ×1.5 floored).
pub(crate) fn species_item_stat(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    stat: Stat,
    value: u32,
) -> u32 {
    let held = super::held_item_id(state, rules, pokemon.id());
    rules
        .stats
        .species_item_stats
        .iter()
        .find(|entry| {
            held == Some(entry.item)
                && entry.stats.contains(&stat)
                && pokemon
                    .species
                    .is_some_and(|species| entry.species.iter().any(|s| s.same_species(species)))
        })
        .map_or(value, |entry| entry.multiplier.apply_u32(value))
}

/// Showdown gen2 `getDamage` after the base power and the critical hit are decided.
pub(crate) fn gen2_damage(
    state: &mut BattleState,
    rules: &ResolvedRules,
    trace: &mut BattleTrace,
    hit: &LegacyHit,
) -> LegacyDamage {
    let (Some(attacker), Some(defender)) =
        (state.pokemon(hit.attacker), state.pokemon(hit.defender))
    else {
        return LegacyDamage {
            offense: 0,
            defense: 1,
            base: 0,
            damage: 0,
            random: None,
        };
    };
    let (atk_value, atk_stage, def_value, def_stage) = match hit.pair {
        StatPair::Physical => (
            attacker.battle_stats().attack,
            attacker.stages.attack,
            defender.battle_stats().defense,
            defender.stages.defense,
        ),
        StatPair::Special => (
            attacker.battle_stats().special_attack,
            attacker.stages.special_attack,
            defender.battle_stats().special_defense,
            defender.stages.special_defense,
        ),
    };
    // A critical hit ignores both stages — and the burn halving — unless the attacker's stage
    // is ahead of the defender's.
    let unboosted = hit.critical && atk_stage <= def_stage;
    let burned = hit.pair == StatPair::Physical
        && attacker.has_status(NonVolatileStatus::Burn)
        && !unboosted;
    let stats = rules.stats;
    let (atk_stat, def_stat) = match hit.pair {
        StatPair::Physical => (Stat::Attack, Stat::Defense),
        StatPair::Special => (Stat::SpecialAttack, Stat::SpecialDefense),
    };
    let mut offense = legacy_stat(&stats, atk_value, atk_stage, burned, unboosted);
    offense = species_item_stat(state, rules, attacker, atk_stat, offense);
    let mut defense = legacy_stat(&stats, def_value, def_stage, false, unboosted);
    // Reflect / Light Screen on the defender's side double the stat after the cap, unless the
    // critical hit ignores the stages (a confusion self-hit meets its own side's Reflect).
    if !unboosted
        && super::screens::doubles_defense(
            state,
            rules,
            hit.defender,
            hit.pair == StatPair::Physical,
        )
    {
        defense *= 2;
    }
    defense = species_item_stat(state, rules, defender, def_stat, defense);
    let mut level = attacker.level;
    if let Some((attack, member_level, base_defense)) = hit.beat_up {
        offense = attack;
        defense = base_defense;
        level = member_level;
    }
    // Present's glitch: Attack 10, the attacker's last type's index for the Defense and the
    // defender's for the level.
    if hit.type_index_damage {
        let last = |types: crate::engine::TypeSet| types.secondary.unwrap_or(types.primary);
        offense = 10;
        defense = gen2_type_index(last(crate::engine::types_of(state, rules, attacker)));
        level = gen2_type_index(last(crate::engine::types_of(state, rules, defender))) as u8;
    }
    // Either stat at 256 or more: both are divided by 4 and taken modulo 256 (the rollover).
    if offense >= 256 || defense >= 256 {
        offense = (offense / 4 % 256).max(1);
        defense = (defense / 4 % 256).max(1);
    }
    if hit.halves_defense && hit.pair == StatPair::Physical {
        defense = (defense / 2).max(1);
    }
    let attacker_types = crate::engine::types_of(state, rules, attacker);
    let base = base_damage(
        DamageAlgorithm::Gen2,
        level,
        BasePower(hit.base_power),
        OffensiveStat(offense),
        DefensiveStat(defense),
    );

    let mut damage = u32::from(base);
    let mut random = None;
    for kind in rules.damage.modifier_order {
        let from = damage;
        let source = match kind {
            DamageModifierKind::Critical if hit.critical => {
                damage = rules.damage.critical_multiplier.apply_u32(damage);
                ModifierSource::Rule("Critical")
            }
            // The type-boosting item (pokecrystal `BattleCommand_DamageCalc`: × 110 / 100).
            DamageModifierKind::ModifyDamage => match hit.item_boost {
                Some(item) => {
                    damage = damage * 11 / 10;
                    ModifierSource::ItemInstance {
                        holder: hit.attacker,
                        item,
                    }
                }
                None => continue,
            },
            DamageModifierKind::LegacyClamp => {
                damage = damage.clamp(1, 997);
                ModifierSource::Rule("LegacyClamp")
            }
            DamageModifierKind::BaseOffset => {
                damage += 2;
                ModifierSource::Rule("BaseOffset")
            }
            DamageModifierKind::Weather if hit.move_type.is_some() => {
                damage = hit.weather.apply_u32(damage);
                ModifierSource::Rule("Weather")
            }
            DamageModifierKind::Stab
                if hit.move_type.is_some_and(|t| attacker_types.contains(t)) =>
            {
                damage += damage / 2;
                ModifierSource::Rule("STAB")
            }
            DamageModifierKind::TypeEffectiveness => {
                damage = hit.type_effectiveness.apply_u32(damage);
                ModifierSource::Rule("TypeEffectiveness")
            }
            DamageModifierKind::Random if hit.variance && damage > 1 => {
                let roll = state.rng.range_inclusive_for(
                    RandomRequestKind::DamageRoll,
                    rules.damage.random_min,
                    rules.damage.random_max,
                );
                random = Some(roll);
                damage = damage * u32::from(roll) / u32::from(rules.damage.random_denominator);
                ModifierSource::Rule("Random")
            }
            _ => continue,
        };
        trace.push(TraceEvent::ModifierApplied {
            stage: ModifierStage::FinalDamage,
            source,
            priority: 0,
            from,
            to: damage,
        });
        // Gen 2 Fury Cutter, Rage: right after the type matchups (pokecrystal `furycutter`,
        // `ragedamage` follow `stab`), before the random factor — which a damage of 1 no longer
        // skips once multiplied.
        if matches!(kind, DamageModifierKind::TypeEffectiveness)
            && let Some((move_id, multiplier)) = hit.multiplier
            && multiplier > 1
        {
            let from = damage;
            damage = (damage * multiplier).min(u32::from(u16::MAX));
            trace.push(TraceEvent::ModifierApplied {
                stage: ModifierStage::FinalDamage,
                source: ModifierSource::Move(move_id),
                priority: 0,
                from,
                to: damage,
            });
        }
    }
    let mut damage = damage.clamp(1, u32::from(u16::MAX));
    // Minimize (pokecrystal `doubleminimizedamage`): the damage calculation's last step, at
    // most 65535 (issues.md I50).
    if hit.minimized {
        let from = damage;
        damage = (damage * 2).min(u32::from(u16::MAX));
        trace.push(TraceEvent::ModifierApplied {
            stage: ModifierStage::FinalDamage,
            source: ModifierSource::Volatile(crate::state::VolatileConditionKind::Minimize),
            priority: 0,
            from,
            to: damage,
        });
    }
    LegacyDamage {
        offense,
        defense,
        base,
        damage: damage as u16,
        random,
    }
}

/// Showdown gen1 `getDamage` after the base power and the critical hit are decided: the
/// Pokémon's modified stats (the stored ones on a critical hit, which also doubles the level),
/// the defender's Reflect / Light Screen, the rollover, then `GEN1_DAMAGE_ORDER` — the 0..=997
/// clamp, `+ 2`, STAB, type effectiveness one defending type at a time (×20/10 or ×5/10, floored
/// each), and the random roll only when the damage is above 1. A damage of 0 stays 0.
pub(crate) fn gen1_damage(
    state: &mut BattleState,
    rules: &ResolvedRules,
    trace: &mut BattleTrace,
    hit: &LegacyHit,
) -> LegacyDamage {
    use crate::state::Stat;
    let (Some(attacker), Some(defender)) =
        (state.pokemon(hit.attacker), state.pokemon(hit.defender))
    else {
        return LegacyDamage {
            offense: 0,
            defense: 1,
            base: 0,
            damage: 0,
            random: None,
        };
    };
    let (atk_stat, def_stat) = match hit.pair {
        StatPair::Physical => (Stat::Attack, Stat::Defense),
        StatPair::Special => (Stat::SpecialAttack, Stat::SpecialDefense),
    };
    // Showdown `getStat(stat, unmodified)`: the modified stat, or the stored one — and for a
    // critical hit the Pokémon's own (`baseStoredStats`), not Transform's copy.
    let read = |pokemon: &crate::state::PokemonState, stat: Stat| -> u32 {
        let stored = pokemon.battle_stats().get(stat).unwrap_or(1);
        let value = if hit.critical {
            pokemon.own_stats().get(stat).unwrap_or(1)
        } else {
            pokemon
                .modified_stats
                .and_then(|modified| modified.get(stat))
                .unwrap_or(stored)
        };
        u32::from(value)
    };
    let mut offense = read(attacker, atk_stat);
    let mut defense = read(defender, def_stat);
    // The defender's Reflect / Light Screen double its stat before the rollover; a critical hit
    // reads the stored stats without them.
    if !hit.critical
        && super::screens::doubles_defense(
            state,
            rules,
            hit.defender,
            hit.pair == StatPair::Physical,
        )
    {
        defense *= 2;
    }
    if offense >= 256 || defense >= 256 {
        offense = (offense / 4 % 256).max(1);
        defense = (defense / 4 % 256).max(1);
    }
    if hit.halves_defense && hit.pair == StatPair::Physical {
        defense = (defense / 2).max(1);
    }
    let level = if hit.critical {
        attacker.level.saturating_mul(2)
    } else {
        attacker.level
    };
    let attacker_types = crate::engine::types_of(state, rules, attacker);
    let defender_types = crate::engine::types_of(state, rules, defender);
    let base = base_damage(
        DamageAlgorithm::Gen1Legacy,
        level,
        BasePower(hit.base_power),
        OffensiveStat(offense),
        DefensiveStat(defense),
    );

    let mut damage = u32::from(base);
    let mut random = None;
    for kind in rules.damage.modifier_order {
        let from = damage;
        let source = match kind {
            DamageModifierKind::LegacyClamp => {
                damage = damage.clamp(u32::from(rules.damage.legacy_clamp_min), 997);
                ModifierSource::Rule("LegacyClamp")
            }
            DamageModifierKind::BaseOffset => {
                damage += 2;
                ModifierSource::Rule("BaseOffset")
            }
            DamageModifierKind::Stab
                if hit.move_type.is_some_and(|t| attacker_types.contains(t)) =>
            {
                damage += damage / 2;
                ModifierSource::Rule("STAB")
            }
            DamageModifierKind::TypeEffectiveness => {
                let Some(move_type) = hit.move_type else {
                    continue;
                };
                for defending in defender_types.iter() {
                    let modifier = rules.type_chart().effectiveness(move_type, defending);
                    if modifier.numerator > modifier.denominator {
                        damage = damage * 20 / 10;
                    } else if modifier.numerator < modifier.denominator && !modifier.is_zero() {
                        damage = damage * 5 / 10;
                    }
                }
                ModifierSource::Rule("TypeEffectiveness")
            }
            DamageModifierKind::Random if hit.variance && damage > 1 => {
                let roll = state.rng.range_inclusive_for(
                    RandomRequestKind::DamageRoll,
                    rules.damage.random_min,
                    rules.damage.random_max,
                );
                random = Some(roll);
                damage = damage * u32::from(roll) / u32::from(rules.damage.random_denominator);
                ModifierSource::Rule("Random")
            }
            _ => continue,
        };
        trace.push(TraceEvent::ModifierApplied {
            stage: ModifierStage::FinalDamage,
            source,
            priority: 0,
            from,
            to: damage,
        });
    }
    LegacyDamage {
        offense,
        defense,
        base,
        damage: damage.min(u32::from(u16::MAX)) as u16,
        random,
    }
}

/// Gen 1 confusion's self-hit (Showdown gen1 `confusion` `onBeforeMove`): 40 power from the
/// modified Attack against the modified Defense, `+ 2`, nothing else — no clamp, no rollover,
/// no random roll.
pub(crate) fn gen1_confusion_damage(state: &BattleState, pokemon: PokemonId) -> u16 {
    use crate::state::Stat;
    let Some(pokemon) = state.pokemon(pokemon) else {
        return 0;
    };
    let stat = |stat: Stat| {
        let stored = pokemon.battle_stats().get(stat).unwrap_or(1);
        u32::from(
            pokemon
                .modified_stats
                .and_then(|modified| modified.get(stat))
                .unwrap_or(stored),
        )
    };
    let level_term = 2 * u32::from(pokemon.level) / 5 + 2;
    let damage = level_term * stat(Stat::Attack) * 40 / stat(Stat::Defense).max(1) / 50 + 2;
    damage.min(u32::from(u16::MAX)) as u16
}
