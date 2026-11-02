//! A hit's critical roll and the move's secondaries' rolls (DESIGN.md §22): the stage and
//! chance, what withholds a secondary (Sheer Force, Shield Dust, a Substitute), its chance.

use super::*;

/// How a hit's critical-hit check goes (DESIGN.md §22.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriticalRoll {
    /// No roll, not critical (a ratio of 0; Gen 1 without a species).
    Never,
    /// No roll, critical (Showdown `willCrit`).
    Always,
    /// `numerator / denominator`.
    Chance(u16, u16),
}

/// The critical-hit check of `attacker`'s damaging move on `target`: the move's crit ratio plus
/// the attacker's item / ability boosts (`onModifyCritRatio`; Merciless makes it 5 against a
/// poisoned target), through the generation's table (`DamageRules::critical`); Gen 1 from the
/// species' base Speed. Moves with fixed damage or a one-hit KO never reach it. Shared by the
/// hit sequence and the replay driver.
pub fn critical_roll(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    dex: &crate::dex::ResolvedDex,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> CriticalRoll {
    use crate::rules::CriticalTable;
    match move_def.will_crit {
        Some(true) => return CriticalRoll::Always,
        Some(false) => return CriticalRoll::Never,
        None => {}
    }
    let Some(pokemon) = state.pokemon(attacker) else {
        return CriticalRoll::Never;
    };
    let item = held_item(state, rules, attacker).map(|(_, item)| item);
    let item_boost = item.map_or(0, |item| item.crit_ratio_boost);
    let ability_boost = ability_handler(state, rules, pokemon, |b| b.crit_ratio_boost);
    // A species' own item (Leek, Stick, Lucky Punch): +2, or in Gen 2 the whole ratio (the
    // gen2 mod's `return 3` at priority −1; pokecrystal jumps straight to the tally).
    let species_stages = item
        .and_then(|item| item.species_crit_boost)
        .and_then(|boost| boost.stages_for(item_species(rules, pokemon)));
    // The critical-hit volatiles' `onModifyCritRatio`: Focus Energy and Dragon Cheer add
    // stages, Laser Focus makes it 5 (certain).
    let focus_energy = pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::FocusEnergy);
    let volatile_stages = if focus_energy {
        rules.conditions.focus_energy_stages
    } else {
        0
    } + pokemon.volatiles.dragon_cheer_stages();
    let ratio = match species_stages {
        Some(stages) if rules.damage.species_crit_item_sets_stage => 1 + stages,
        _ => {
            move_def.crit_ratio
                + item_boost
                + ability_boost
                + species_stages.unwrap_or(0)
                + volatile_stages
        }
    };
    let ratio = if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::LaserFocus)
    {
        ratio.max(5)
    } else {
        ratio
    };
    let poisoned = state.pokemon(target).is_some_and(|foe| {
        foe.has_status(crate::state::NonVolatileStatus::Poison)
            || foe.has_status(crate::state::NonVolatileStatus::Toxic)
    });
    let ratio = if poisoned && ability_handler(state, rules, pokemon, |b| b.crit_against_poisoned) {
        ratio.max(5)
    } else {
        ratio
    };
    let chance = match rules.damage.critical {
        CriticalTable::Gen1BaseSpeed => pokemon
            .species
            .and_then(|species| dex.species_by_id(species))
            .map(|species| {
                CriticalTable::gen1_chance(species.base_stats.speed, ratio, focus_energy)
            }),
        table => table.chance(ratio),
    };
    match chance {
        None | Some((0, _)) => CriticalRoll::Never,
        Some((numerator, denominator)) if numerator >= denominator => CriticalRoll::Always,
        Some((numerator, denominator)) => CriticalRoll::Chance(numerator, denominator),
    }
}

/// How a secondary effect is decided for `actor`'s move on `affected` right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryRoll {
    /// Nothing is rolled; the effect is attempted.
    Certain,
    /// Nothing is rolled and nothing happens (Gen 2: a burn / freeze against a target of the
    /// move's type, a flinch against a sleeping or frozen target).
    Skipped,
    /// `numerator / denominator`.
    Chance(u16, u16),
}

/// Whether hit `hit` of `hits` (the count the sequence was decided to have) leaves the
/// secondary out: Gen 1 rolls a multi-hit move's secondaries on its last hit only
/// (`HitRules::secondaries_on_last_hit`); Parental Bond with Secret Power (its
/// `onSourceModifySecondaries`) keeps the first hit to a flinch, so Secret Power's own secondary
/// rolls on the second hit alone. Shared with the replay.
pub fn secondary_withheld(
    rules: &crate::rules::ResolvedRules,
    move_def: &crate::dex::ResolvedMove,
    hit: u8,
    hits: u8,
    secondary: &crate::dex::SecondaryEffectSpec,
) -> bool {
    (rules.hits.secondaries_on_last_hit && hit < hits)
        || (move_def.flags.parental_bond
            && move_def.behavior.terrain_secondaries
            && hit < 2
            && secondary.in_secondaries
            && secondary.effect
                != crate::dex::SecondaryEffect::Volatile(
                    crate::state::VolatileConditionKind::Flinch,
                ))
}

/// The secondary's decision (DESIGN.md §31.2): Gen 3+ its percent after the user's ability
/// (Serene Grace), certain at 100 %; Gen 1-2 a byte out of 256 for any declared chance, after
/// Gen 2's pre-roll checks. Shared by the hit sequence and the replay driver.
pub fn secondary_roll(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: crate::ids::PokemonId,
    affected: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
    secondary: &crate::dex::SecondaryEffectSpec,
) -> SecondaryRoll {
    use crate::dex::SecondaryEffect;
    use crate::state::{NonVolatileStatus, VolatileConditionKind};
    // The secondary's own condition (Burning Jealousy, Gen 4-5 Chatter): unmet, nothing rolls.
    let unmet = match secondary.requires {
        None => false,
        Some(crate::dex::SecondaryRequirement::TargetStatsRaisedThisTurn) => state
            .pokemon(affected)
            .is_none_or(|target| !target.history.stats_raised_this_turn),
        Some(crate::dex::SecondaryRequirement::UserSpecies(species)) => state
            .pokemon(actor)
            .is_none_or(|user| species_of(user) != Some(species)),
    };
    if unmet {
        return SecondaryRoll::Skipped;
    }
    // Shield Dust / Covert Cloak (`onModifySecondaries`) drop a target-directed secondary before
    // its roll; a user-directed one stays, and so does a binding move's own trap.
    if secondary.in_secondaries
        && secondary.target == crate::dex::SecondaryTarget::Target
        && shields_secondaries(state, rules, affected)
    {
        return SecondaryRoll::Skipped;
    }
    if let Some(target) = state.pokemon(affected) {
        let skipped = match secondary.effect {
            SecondaryEffect::Status(status) => {
                rules.hits.secondary_same_type_immunity.contains(&status)
                    && crate::engine::types_of(state, rules, target).contains(move_def.move_type)
            }
            SecondaryEffect::Volatile(VolatileConditionKind::Flinch) => {
                rules.hits.flinch_needs_awake_target
                    && !secondary.from_item
                    && (target.has_status(NonVolatileStatus::Sleep)
                        || target.has_status(NonVolatileStatus::Freeze))
            }
            _ => false,
        };
        if skipped {
            return SecondaryRoll::Skipped;
        }
    }
    match rules.hits.chance_scale {
        crate::rules::ChanceScale::Percent => {
            // An item's flinch (King's Rock) escapes Serene Grace where the generation rolls it
            // apart from the move's secondaries (`ItemFlinchRules`).
            let chance = if secondary.from_item && !rules.hits.item_flinch.doubled_by_serene_grace {
                secondary.chance.basis_points().min(10_000)
            } else {
                effective_secondary_chance(state, rules, actor, secondary.chance.basis_points())
            };
            if chance >= 10_000 {
                SecondaryRoll::Certain
            } else {
                SecondaryRoll::Chance(chance, 10_000)
            }
        }
        crate::rules::ChanceScale::Byte => match secondary.chance.0 {
            None => SecondaryRoll::Certain,
            Some(basis_points) => {
                SecondaryRoll::Chance((u32::from(basis_points) * 255 / 10_000) as u16, 256)
            }
        },
        // Showdown gen1 `moveHit`: `⌈chance × 256 / 100⌉`, one less for confusion.
        crate::rules::ChanceScale::ByteAlwaysRolls => match secondary.chance.0 {
            None => SecondaryRoll::Certain,
            Some(basis_points) => {
                let mut numerator = (u32::from(basis_points) * 256).div_ceil(10_000);
                if secondary.effect == SecondaryEffect::Volatile(VolatileConditionKind::Confusion) {
                    numerator -= 1;
                }
                if numerator >= 256 {
                    SecondaryRoll::Certain
                } else {
                    SecondaryRoll::Chance(numerator as u16, 256)
                }
            }
        },
    }
}

/// A secondary's chance in basis points after the user's ability (Serene Grace doubles it),
/// capped at certain; shared by the hit sequence and the replay driver.
pub fn effective_secondary_chance(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: crate::ids::PokemonId,
    base_basis_points: u16,
) -> u16 {
    let multiplier = state
        .pokemon(actor)
        .and_then(|p| ability_handler(state, rules, p, |b| b.secondary_chance_multiplier))
        .unwrap_or(1);
    base_basis_points.saturating_mul(multiplier).min(10_000)
}
