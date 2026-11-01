//! Move formulas as data (DESIGN.md §22.7, §22.8): a move's own base power (Showdown
//! `basePowerCallback`) and its conditional BasePower participant (`onBasePower`), some of
//! them reading the event history (`state::PokemonHistory`); the weather's effect on its
//! accuracy (`onModifyMove`) and weather-dependent healing (`onHit`). Each
//! formula is a variant of a small enum in `effects::moves`; the per-generation constants are
//! in `DamageRules` / `ConditionRules`; the functions here are the only place they are
//! evaluated, shared by the pipeline and the replay driver.

use crate::dex::{ResolvedDex, ResolvedMove};
use crate::effects::moves::{PowerCondition, PowerFormula, WeatherAccuracy, WeatherHeal};
use crate::ids::PokemonId;
use crate::rules::{ResolvedRules, TargetHpPower};
use crate::state::{
    BattleState, MoveResult, NonVolatileStatus, PokemonState, StatStages, TerrainKind, WeatherKind,
};

use super::stats::effective_speed;

/// A Pokémon's weight in hectograms (Showdown `getWeight`: the `ModifyWeight` event — its
/// item's `onModifyWeight`, Float Stone — then at least 1). A Pokémon built without a species
/// weighs 1. Autotomize takes its 100 kg per use off the species' weight first (at least 1:
/// Showdown's `weighthg`, or the gen5 volatile's `onModifyWeight` at priority 2, ahead of
/// every other).
pub fn weight_hg(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    pokemon: PokemonId,
) -> u32 {
    let Some(pokemon) = state.pokemon(pokemon) else {
        return 1;
    };
    let Some(species) = super::species_of(pokemon).and_then(|species| dex.species_by_id(species))
    else {
        return 1;
    };
    // Showdown `ModifyWeight`: the highest priority first, the ability before the item on a
    // tie (Heavy Metal 1; Light Metal and Float Stone 0), each step truncated.
    let by_ability = super::ability_handler(state, rules, pokemon, |b| b.weight_modifier)
        .map(|modifier| (modifier.priority, 0, modifier.factor));
    let by_item = super::held_item(state, rules, pokemon.id())
        .and_then(|(_, item)| item.weight_modifier)
        .map(|factor| (0, 1, factor));
    let mut modifiers: Vec<(i16, u8, crate::rules::Fraction)> =
        by_ability.into_iter().chain(by_item).collect();
    modifiers.sort_by_key(|(priority, order, _)| (std::cmp::Reverse(*priority), *order));
    let lightened = species
        .weight_hg
        .saturating_sub(1000 * u32::from(pokemon.volatiles.autotomize_count()))
        .max(1);
    modifiers
        .into_iter()
        .fold(lightened, |weight, (_, _, factor)| factor.apply_u32(weight))
        .max(1)
}

fn positive_stages(stages: &StatStages) -> u32 {
    [
        stages.attack,
        stages.defense,
        stages.special_attack,
        stages.special_defense,
        stages.speed,
        stages.accuracy,
        stages.evasion,
    ]
    .into_iter()
    .filter(|stage| *stage > 0)
    .map(|stage| stage as u32)
    .sum()
}

/// What a power formula reads about this use of the move, beyond the battle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveUse {
    /// The hit of the sequence being resolved, 1 for the first (Showdown `move.hit`).
    pub hit: u8,
    /// The move that moved this action up the queue (Showdown `move.sourceEffect`: Round).
    pub prioritized_by: Option<crate::ids::MoveId>,
    /// The party member this hit is for (Beat Up).
    pub member: Option<PokemonId>,
    /// What called the move (Showdown `move.sourceEffect`: Metronome, Sleep Talk …, or an
    /// ability's), if anything did.
    pub called_by: Option<crate::state::CallSource>,
}

impl MoveUse {
    /// The first hit of a move used in its own turn order.
    pub const FIRST: Self = Self {
        hit: 1,
        prioritized_by: None,
        member: None,
        called_by: None,
    };
}

/// The base power a formula gives `attacker`'s move against `target` right now, floored and at
/// least 1 (Showdown `clampIntRange(basePower, 1)`).
#[allow(clippy::too_many_arguments)]
pub fn formula_power(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    attacker: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
    formula: PowerFormula,
    usage: MoveUse,
) -> u16 {
    raw_formula_power(
        state, rules, dex, attacker, target, move_def, formula, usage,
    )
    .clamp(1, u32::from(u16::MAX)) as u16
}

/// A power of `null`: the hit does nothing, not even its critical-hit check or damage roll.
/// Gen 2-4 Beat Up with no member (every one fainted or statused; the gen4 mod's
/// `basePowerCallback`), and in Gen 2 (`DamageRules::zero_formula_power_does_nothing`) Return /
/// Frustration's formula giving 0 (`|| null`). Shared with the replay driver.
pub fn zero_power_does_nothing(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    attacker: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
) -> bool {
    if move_def.behavior.beat_up && rules.damage.beat_up != crate::rules::BeatUpStyle::Modern {
        return crate::engine::beat_up_members(state, rules, attacker).is_empty();
    }
    rules.damage.zero_formula_power_does_nothing
        && move_def.behavior.power_formula.is_some_and(|formula| {
            raw_formula_power(
                state,
                rules,
                dex,
                attacker,
                target,
                move_def,
                formula,
                MoveUse::FIRST,
            ) == 0
        })
}

#[allow(clippy::too_many_arguments)]
fn raw_formula_power(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    attacker: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
    formula: PowerFormula,
    usage: MoveUse,
) -> u32 {
    let (Some(user), Some(foe)) = (state.pokemon(attacker), state.pokemon(target)) else {
        return u32::from(move_def.base_power.max(1));
    };
    let base = u32::from(move_def.base_power);
    match formula {
        PowerFormula::UserHpScaled => base * u32::from(user.hp) / u32::from(user.max_hp.max(1)),
        PowerFormula::UserHpBands => {
            let bands = rules.damage.hp_bands;
            let ratio = (u32::from(user.hp) * bands.scale / u32::from(user.max_hp.max(1))).max(1);
            let index = bands
                .thresholds
                .iter()
                .position(|threshold| ratio < *threshold)
                .unwrap_or(5);
            [200, 150, 100, 80, 40, 20][index]
        }
        PowerFormula::SpeedRatioBands => {
            let ratio = effective_speed(state, rules, attacker)
                / effective_speed(state, rules, target).max(1);
            [40, 60, 80, 120, 150][ratio.min(4) as usize]
        }
        PowerFormula::InverseSpeedRatio => (25 * effective_speed(state, rules, target)
            / effective_speed(state, rules, attacker).max(1)
            + 1)
        .min(150),
        PowerFormula::UserPositiveBoosts => base + 20 * positive_stages(&user.stages),
        PowerFormula::TargetPositiveBoosts => (60 + 20 * positive_stages(&foe.stages)).min(200),
        PowerFormula::TargetHpScaled { max } => target_hp_power(rules, foe, u32::from(max)),
        // Hex, Infernal Parade: a status, or Comatose (`hasAbility('comatose')`).
        PowerFormula::DoubledAgainstStatus => {
            if foe.status.is_some() || crate::engine::counts_as_asleep(state, rules, foe) {
                base * 2
            } else {
                base
            }
        }
        // Fury Cutter (§15.18): its chain as this use left it (`BattleContext::advance_fury_cutter`
        // ran first).
        PowerFormula::FuryCutter => {
            let level = u32::from(user.volatiles.fury_cutter().map_or(1, |(level, _)| level));
            match rules.conditions.fury_cutter.model {
                crate::rules::FuryCutterModel::Volatile { .. } => (base * level).clamp(1, 160),
                crate::rules::FuryCutterModel::Count {
                    damage_stage: false,
                    ..
                } => base << (level.clamp(1, 5) - 1),
                crate::rules::FuryCutterModel::Count { .. } => base,
            }
        }
        // Rollout / Ice Ball (`basePowerCallback`): the doublings so far (`contactHitCount`, the
        // count before this hit's: one per hit, less the hits Gen 7 Disguise took), Defense Curl
        // once more.
        PowerFormula::Rolling => {
            let doublings = user.volatiles.rolling_doublings().unwrap_or(0);
            let curled = user
                .volatiles
                .contains(crate::state::VolatileConditionKind::DefenseCurl);
            (base << u32::from(doublings.min(8))) * if curled { 2 } else { 1 }
        }
        PowerFormula::DoubledAgainstSwitching => {
            if state.called_back == Some(target) {
                base * 2
            } else {
                base
            }
        }
        PowerFormula::DoubledInElectricTerrain => {
            let electric = state
                .field
                .terrain
                .is_some_and(|terrain| terrain.kind == TerrainKind::Electric);
            if electric && super::environment::grounded(state, rules, foe) {
                base * 2
            } else {
                base
            }
        }
        // Sets carry no happiness: Showdown's default, 255.
        PowerFormula::Happiness => u32::from(user.happiness) * 10 / 25,
        PowerFormula::PerHit(step) => u32::from(step) * u32::from(usage.hit.max(1)),
        PowerFormula::HiddenPower => u32::from(user.hidden_power_of().1),
        PowerFormula::StockpileLayers => 100 * u32::from(user.volatiles.stockpile_layers()),
        PowerFormula::BeatUp => match rules.damage.beat_up {
            crate::rules::BeatUpStyle::Modern => usage
                .member
                .and_then(|member| state.pokemon(member))
                .and_then(|member| member.species)
                .and_then(|species| dex.species_by_id(species))
                .map_or(base, |species| {
                    5 + u32::from(species.base_stats.attack) / 10
                }),
            _ => base,
        },
        PowerFormula::Echoed => {
            base * u32::from(state.field.echoed_voice_multiplier().unwrap_or(1))
        }
        PowerFormula::DoubledWhenPrioritized => {
            if usage.prioritized_by == Some(move_def.id) {
                base * 2
            } else {
                base
            }
        }
        PowerFormula::Unhappiness => (255 - u32::from(user.happiness)) * 10 / 25,
        // The slot of the move that called it, if one did (Showdown `move.sourceEffect ||
        // move.id`: a Metronome's Trump Card reads Metronome's PP; an ability's call finds no
        // slot, DESIGN.md R-M8-84).
        PowerFormula::PpLeftBands => {
            let slot_move = match usage.called_by {
                None => Some(move_def.id),
                Some(crate::state::CallSource::Move(caller)) => Some(caller),
                Some(crate::state::CallSource::Ability(_)) => None,
            };
            match slot_move
                .and_then(|id| user.move_slot_of(id))
                .map(|slot| slot.pp)
            {
                Some(0) => 200,
                Some(1) => 80,
                Some(2) => 60,
                Some(3) => 50,
                _ => 40,
            }
        }
        PowerFormula::AshGreninjaBonus => {
            if super::species_of(user) == Some(crate::ids::SpeciesId::GRENINJA_ASH)
                && user.transform.is_none()
                && super::has_ability(state, rules, user, crate::ids::AbilityId::BATTLE_BOND)
            {
                base + 5
            } else {
                base
            }
        }
        PowerFormula::DoubledAgainst(status) => {
            let comatose = status == crate::state::NonVolatileStatus::Sleep
                && crate::engine::counts_as_asleep(state, rules, foe);
            if foe.has_status(status) || comatose {
                base * 2
            } else {
                base
            }
        }
        PowerFormula::TargetWeightBands => {
            let weight = weight_hg(state, rules, dex, target);
            match weight {
                2000.. => 120,
                1000.. => 100,
                500.. => 80,
                250.. => 60,
                100.. => 40,
                _ => 20,
            }
        }
        PowerFormula::WeightRatioBands => {
            let (user_weight, foe_weight) = (
                weight_hg(state, rules, dex, attacker),
                weight_hg(state, rules, dex, target),
            );
            match (1..=5)
                .rev()
                .find(|factor| user_weight >= foe_weight * factor)
            {
                Some(5) => 120,
                Some(4) => 100,
                Some(3) => 80,
                Some(2) => 60,
                _ => 40,
            }
        }
        PowerFormula::DoubledIfTargetMoved => {
            let yet_to_move = state.will_move(target)
                || (foe.history.newly_switched && !rules.damage.newcomer_counts_as_moved);
            if yet_to_move { base } else { base * 2 }
        }
        PowerFormula::DoubledIfTargetYetToMove => {
            if state.will_move(target) || foe.history.newly_switched {
                base * 2
            } else {
                base
            }
        }
        PowerFormula::DoubledIfDamagedByTarget => {
            if user.history.damaged_this_turn_by(target) {
                base * 2
            } else {
                base
            }
        }
        PowerFormula::DoubledIfTargetHurt => {
            if foe.history.hurt_this_turn {
                base * 2
            } else {
                base
            }
        }
        PowerFormula::DoubledAfterFailure => {
            if user.history.move_last_turn == Some(MoveResult::Failed) {
                base * 2
            } else {
                base
            }
        }
        PowerFormula::TimesAttacked => (50 + 50 * u32::from(user.history.times_attacked)).min(350),
        PowerFormula::FaintedAllies => 50 + 50 * u32::from(state.side(user.side).total_fainted),
        PowerFormula::DoubledWithoutItem => {
            if user.held_item.is_none() {
                base * 2
            } else {
                base
            }
        }
    }
}

fn target_hp_power(rules: &ResolvedRules, foe: &PokemonState, max: u32) -> u32 {
    let hp = u32::from(foe.hp);
    let max_hp = u32::from(foe.max_hp.max(1));
    match rules.damage.target_hp_power {
        TargetHpPower::Linear => hp * max / max_hp + 1,
        TargetHpPower::Scaled4096 => {
            let percent = hp * 4096 / max_hp;
            (max * 100 * percent + 2047) / 4096 / 100
        }
    }
}

/// Whether a move's own BasePower participant applies (Showdown `onBasePower`).
pub fn power_condition_holds(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
    condition: PowerCondition,
    super_effective: bool,
) -> bool {
    let (Some(user), Some(foe)) = (state.pokemon(attacker), state.pokemon(target)) else {
        return false;
    };
    match condition {
        PowerCondition::FieldCondition(kind) => state.field.contains(kind),
        PowerCondition::UserStatusedAwake => user
            .status
            .is_some_and(|status| status.kind != NonVolatileStatus::Sleep),
        PowerCondition::TargetAtHalfHp => u32::from(foe.hp) * 2 <= u32::from(foe.max_hp),
        PowerCondition::TargetPoisoned => {
            foe.has_status(NonVolatileStatus::Poison) || foe.has_status(NonVolatileStatus::Toxic)
        }
        PowerCondition::SuperEffective => super_effective,
        PowerCondition::ElectricTerrain => state
            .field
            .terrain
            .is_some_and(|terrain| terrain.kind == TerrainKind::Electric),
        PowerCondition::GroundedInTerrain(kind) => {
            state
                .field
                .terrain
                .is_some_and(|terrain| terrain.kind == kind)
                && super::environment::grounded(state, rules, user)
        }
        PowerCondition::AllyFaintedLastTurn => state.side(user.side).fainted_last_turn,
        PowerCondition::UserStatsLoweredThisTurn => user.history.stats_lowered_this_turn,
        PowerCondition::AfterMove(partner) => state.last_successful_move == Some(partner),
        PowerCondition::TargetItemTakable(move_id) => foe.held_item.is_some_and(|item| {
            !super::item_refuses_take(state, rules, item, target, target, Some(move_id))
        }),
        PowerCondition::WeakeningWeather => {
            weakening_weather(super::weather_for(state, rules, attacker))
        }
    }
}

/// The weathers that weaken the moves of sunlight (Showdown's `raindance`, `primordialsea`,
/// `sandstorm`, `hail`, `snowscape`: Solar Beam's power, Moonlight's heal). Delta Stream's winds
/// are none of them (DESIGN.md R-M8-90).
fn weakening_weather(weather: Option<WeatherKind>) -> bool {
    matches!(
        weather,
        Some(WeatherKind::Rain | WeatherKind::Sandstorm | WeatherKind::Snow)
    )
}

/// A move's accuracy under the weather as `target` sees it (Showdown `onModifyMove`:
/// `target.effectiveWeather()`; Blizzard's `isWeather` is the same for snow): `None` never
/// misses.
pub fn weather_accuracy(
    state: &BattleState,
    rules: &ResolvedRules,
    target: PokemonId,
    accuracy: Option<u8>,
    rule: WeatherAccuracy,
) -> Option<u8> {
    let weather = super::weather_for(state, rules, target);
    match (rule, weather) {
        (WeatherAccuracy::RainSureSunHalf | WeatherAccuracy::RainSure, Some(WeatherKind::Rain)) => {
            None
        }
        (WeatherAccuracy::RainSureSunHalf, Some(WeatherKind::Sun)) => Some(50),
        (WeatherAccuracy::SnowSure, Some(WeatherKind::Snow)) => None,
        _ => accuracy,
    }
}

/// How much a weather-dependent healing move restores to `pokemon`.
pub fn weather_heal_amount(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: PokemonId,
    heal: WeatherHeal,
) -> u16 {
    let Some(max_hp) = state.pokemon(pokemon).map(|p| p.max_hp) else {
        return 0;
    };
    let amounts = rules.conditions.weather_heal;
    let weather = super::weather_for(state, rules, pokemon);
    let amount = match (heal, weather) {
        (WeatherHeal::Sunlight, Some(WeatherKind::Sun)) => amounts.boosted,
        (WeatherHeal::Sunlight, weather) if weakening_weather(weather) => amounts.weakened,
        (WeatherHeal::Sand, Some(WeatherKind::Sandstorm)) => amounts.boosted,
        _ => amounts.normal,
    };
    amount.of(max_hp)
}
