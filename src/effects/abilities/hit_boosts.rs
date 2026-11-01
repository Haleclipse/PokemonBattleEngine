//! Abilities that raise (or trade) the holder's stats when a move reaches it (DESIGN.md §20.2,
//! R124): after a damaging hit (`onDamagingHit`: Justified, Rattled, Stamina, Steam Engine,
//! Water Compaction, Weak Armor), and in place of a hit they refuse (`onTryHit`: Motor Drive,
//! Well-Baked Body, Earth Eater, Sap Sipper — R167, with its `onAllyTryHitSide`). Rattled also
//! answers Intimidate (`onAfterBoost`, Gen 8+).

use crate::dex::{MoveCategory, PokemonType};
use crate::engine::{
    BattleContext, DamagingHit, EffectSource, HitBlockReason, StatChangeCause, StatChangeRequest,
    StatChangeResult, TryHitContext, TryHitResult,
};
use crate::ids::{AbilityId, PokemonId};
use crate::state::Stat;

/// The holder's own `this.boost(...)`, announced as its ability (`-ability … boost`) when a
/// stage moved; `true` when one did (Gen 6+ nothing once the holder's foes have no Pokémon
/// left: the stage change itself refuses, `StatChangeBlockReason::NoFoeLeft`).
pub(super) fn boost(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    changes: &[(Stat, i8)],
) -> bool {
    let Some(ability) = ctx.ability_of(holder) else {
        return false;
    };
    let mut any = false;
    for &(stat, stages) in changes {
        let result = ctx.try_change_stat_stage(StatChangeRequest {
            target: holder,
            stat,
            stages,
            source: EffectSource::Ability {
                pokemon: holder,
                ability,
            },
            cause: StatChangeCause::Ability,
        });
        any |= matches!(result, StatChangeResult::Applied { .. });
    }
    if any {
        ctx.ability_activated(holder, ability);
    }
    any
}

/// Justified: a Dark move's hit raises Attack.
pub fn justified(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if hit.move_type == PokemonType::Dark {
        boost(ctx, hit.holder, &[(Stat::Attack, 1)]);
    }
}

/// Rattled: a Dark, Bug or Ghost move's hit raises Speed.
pub fn rattled(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if matches!(
        hit.move_type,
        PokemonType::Dark | PokemonType::Bug | PokemonType::Ghost
    ) {
        boost(ctx, hit.holder, &[(Stat::Speed, 1)]);
    }
}

/// Rattled's `onAfterBoost` (Gen 8+): Intimidate changing its Attack raises its Speed.
pub fn rattled_after_boost(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    request: StatChangeRequest,
) {
    if request.stat == Stat::Attack
        && matches!(
            request.source,
            EffectSource::Ability {
                ability: AbilityId::INTIMIDATE,
                ..
            }
        )
    {
        boost(ctx, holder, &[(Stat::Speed, 1)]);
    }
}

/// Anger Point (`onHit`): a critical hit on the standing holder raises its Attack by 12 (to
/// +6: `-setboost`), as a stat change (Contrary turns it around).
pub fn anger_point(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    _user: PokemonId,
    critical: bool,
) {
    if critical && !ctx.is_fainted(holder) {
        boost(ctx, holder, &[(Stat::Attack, 12)]);
    }
}

/// Gen 4 Anger Point (the gen4 mod's `onAfterSubDamage`): a critical hit on the holder's
/// Substitute sets its Attack to +6 outright (`setBoost`, no stat change event).
pub fn anger_point_substitute(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    _user: PokemonId,
    critical: bool,
) {
    if critical && !ctx.is_fainted(holder) {
        ctx.set_stage(
            holder,
            Stat::Attack,
            6,
            EffectSource::Ability {
                pokemon: holder,
                ability: AbilityId::ANGER_POINT,
            },
        );
    }
}

/// Guard Dog (`onTryBoost`, priority 2): Intimidate's Attack change (`boost.atk`, whatever its
/// sign) is dropped and the holder's Attack rises by 1 instead (`-ability … boost`).
pub fn guard_dog(ctx: &mut BattleContext<'_>, request: StatChangeRequest) -> bool {
    let intimidate = matches!(
        request.source,
        EffectSource::Ability {
            ability: AbilityId::INTIMIDATE,
            ..
        }
    ) && request.stat == Stat::Attack;
    if intimidate {
        boost(ctx, request.target, &[(Stat::Attack, 1)]);
    }
    intimidate
}

/// Stamina: every damaging hit raises Defense.
pub fn stamina(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    boost(ctx, hit.holder, &[(Stat::Defense, 1)]);
}

/// Steam Engine: a Fire or Water move's hit raises Speed by six.
pub fn steam_engine(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if matches!(hit.move_type, PokemonType::Fire | PokemonType::Water) {
        boost(ctx, hit.holder, &[(Stat::Speed, 6)]);
    }
}

/// Water Compaction: a Water move's hit raises Defense by two.
pub fn water_compaction(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if hit.move_type == PokemonType::Water {
        boost(ctx, hit.holder, &[(Stat::Defense, 2)]);
    }
}

/// Weak Armor: a physical hit lowers Defense and raises Speed (`HitRules::weak_armor_speed`:
/// one stage before Gen 7, two from then), a change the holder makes to itself.
pub fn weak_armor(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if hit.category == MoveCategory::Physical {
        let speed = ctx.rules().hits().weak_armor_speed;
        boost(
            ctx,
            hit.holder,
            &[(Stat::Defense, -1), (Stat::Speed, speed)],
        );
    }
}

fn absorbs(ctx: &TryHitContext<'_>, absorbed: PokemonType, ability: AbilityId) -> TryHitResult {
    if ctx.user() != ctx.target() && ctx.move_type() == absorbed {
        TryHitResult::Blocked(HitBlockReason::Ability(ability))
    } else {
        TryHitResult::Allow
    }
}

/// Motor Drive: an Electric move from someone else is refused.
pub fn motor_drive(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    absorbs(ctx, PokemonType::Electric, AbilityId::MOTOR_DRIVE)
}

/// Motor Drive's refusal: +1 Speed (`-immune` when it cannot rise).
pub fn motor_drive_absorb(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    boost(ctx, holder, &[(Stat::Speed, 1)]);
}

/// Sap Sipper (`onTryHit`, priority 1): a Grass move from someone else is refused.
pub fn sap_sipper(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    absorbs(ctx, PokemonType::Grass, AbilityId::SAP_SIPPER)
}

/// Sap Sipper's refusal: +1 Attack (`-immune` when it cannot rise).
pub fn sap_sipper_absorb(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    boost(ctx, holder, &[(Stat::Attack, 1)]);
}

/// Sap Sipper's `onAllyTryHitSide`: a partner's Grass move aimed at their side (Aromatherapy)
/// raises the holder's Attack — not the holder's own (`source === this.effectState.target`).
pub fn sap_sipper_ally(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    user: PokemonId,
    move_def: &crate::dex::ResolvedMove,
) {
    if user != holder && move_def.move_type == PokemonType::Grass {
        boost(ctx, holder, &[(Stat::Attack, 1)]);
    }
}

/// Well-Baked Body: a Fire move from someone else is refused.
pub fn well_baked_body(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    absorbs(ctx, PokemonType::Fire, AbilityId::WELL_BAKED_BODY)
}

/// Well-Baked Body's refusal: +2 Defense.
pub fn well_baked_body_absorb(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    boost(ctx, holder, &[(Stat::Defense, 2)]);
}

/// Earth Eater: a Ground move from someone else is refused (and heals a quarter, like Volt
/// Absorb: `absorb::heal_quarter`).
pub fn earth_eater(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    absorbs(ctx, PokemonType::Ground, AbilityId::EARTH_EATER)
}
