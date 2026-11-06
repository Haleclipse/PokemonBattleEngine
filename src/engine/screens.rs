//! The screen a hit meets (DESIGN.md §10.7): Reflect against physical moves, Light Screen
//! against special ones, Aurora Veil against both — as a doubled stat (Gen 1-2) or a damage
//! modifier (Gen 3+), `ScreenRules::model` says which.

use crate::dex::MoveCategory;
use crate::ids::PokemonId;
use crate::rules::{CrowdedSide, ResolvedRules, ScreenModel};
use crate::state::{BattleState, SideConditionKind, VolatileConditionKind};

/// Gen 1-2: whether `defender`'s Defense (`physical`) or Special (Defense) counts double — its
/// own Gen 1 volatile, or its side's Gen 2 screen. The caller leaves it out when a critical hit
/// ignores it.
pub(crate) fn doubles_defense(
    state: &BattleState,
    rules: &ResolvedRules,
    defender: PokemonId,
    physical: bool,
) -> bool {
    let Some(pokemon) = state.pokemon(defender) else {
        return false;
    };
    match rules.conditions.screens.model {
        ScreenModel::DoubledStatVolatile => pokemon.volatiles.contains(if physical {
            VolatileConditionKind::Reflect
        } else {
            VolatileConditionKind::LightScreen
        }),
        ScreenModel::DoubledStatSide => state.side(pokemon.side).conditions.contains(if physical {
            SideConditionKind::Reflect
        } else {
            SideConditionKind::LightScreen
        }),
        ScreenModel::DamageModifier => false,
    }
}

/// Gen 3+ (`onAnyModifyDamage`, Gen 3-4 `onAnyModifyDamagePhase1`): the 4096-based modifier a
/// screen on `target`'s side puts on `attacker`'s hit of `category` — Reflect on a physical one,
/// Light Screen on a special one, Aurora Veil on either when the matching screen is not up
/// (it steps aside) — or `None`: no screen, a critical hit, a hit on oneself, or an Infiltrator's
/// (`move.infiltrates`).
pub(crate) fn damage_modifier(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
    category: MoveCategory,
    critical: bool,
) -> Option<u32> {
    let screens = rules.conditions.screens;
    if screens.model != ScreenModel::DamageModifier
        || critical
        || attacker == target
        || super::protections::infiltrates(state, rules, attacker)
    {
        return None;
    }
    let side = state.pokemon(target)?.side;
    let conditions = &state.side(side).conditions;
    let screen = match category {
        MoveCategory::Physical => SideConditionKind::Reflect,
        MoveCategory::Special => SideConditionKind::LightScreen,
        MoveCategory::Status => return None,
    };
    if !conditions.contains(screen) && !conditions.contains(SideConditionKind::AuroraVeil) {
        return None;
    }
    Some(if crowded(state, rules, target) {
        screens.crowded_modifier
    } else {
        screens.modifier
    })
}

/// Whether `target`'s side counts as more than one (`ScreenRules::crowded`): for a screen, and
/// for the Gen 3 spread modifier.
pub(crate) fn crowded(state: &BattleState, rules: &ResolvedRules, target: PokemonId) -> bool {
    let partner = |counts: &dyn Fn(PokemonId) -> bool| {
        state.pokemon(target).is_some_and(|pokemon| {
            state
                .side(pokemon.side)
                .active
                .iter()
                .flatten()
                .any(|partner| *partner != target && counts(*partner))
        })
    };
    match rules.conditions.screens.crowded {
        CrowdedSide::PartnerPresent => partner(&|partner| state.not_fainted(partner)),
        CrowdedSide::PartnerStanding => {
            partner(&|partner| state.pokemon(partner).is_some_and(|p| !p.is_fainted()))
        }
        CrowdedSide::TwoSlots => rules.topology.active_slots_per_side() > 1,
    }
}
