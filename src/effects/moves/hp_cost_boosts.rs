//! Moves that pay HP for stat stages (Showdown `directDamage`, DESIGN.md §20.1, R155): Belly
//! Drum (`onHit`), Fillet Away and Clangorous Soul (`onTry`, `onTryHit`, `onHit`, `boosts`).

use crate::engine::{
    BattleContext, ConditionApplyError, DamageRequest, EffectSource, StatChangeCause,
    StatChangeRequest, StatChangeResult,
};
use crate::ids::{MoveId, PokemonId};
use crate::rules::{BellyDrumRule, ResolvedTarget};
use crate::state::Stat;

fn raise(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    move_id: MoveId,
    stat: Stat,
    stages: i8,
) -> bool {
    matches!(
        ctx.try_change_stat_stage(StatChangeRequest {
            target: user,
            stat,
            stages,
            source: EffectSource::Move {
                pokemon: user,
                move_id,
            },
            cause: StatChangeCause::Move,
        }),
        StatChangeResult::Applied { .. }
    )
}

fn hp(ctx: &BattleContext<'_>, user: PokemonId) -> (u32, u32) {
    (
        u32::from(ctx.hp(user).unwrap_or(0)),
        u32::from(ctx.max_hp(user).unwrap_or(0)),
    )
}

/// Belly Drum (`BellyDrumRule`).
pub fn belly_drum(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let (hp, max_hp) = hp(ctx, user);
    let stage = ctx.stage(user, Stat::Attack);
    let half_or_less = 2 * hp <= max_hp;
    let source = EffectSource::Move {
        pokemon: user,
        move_id: MoveId::BELLY_DRUM,
    };
    match ctx.rules().conditions().belly_drum {
        BellyDrumRule::Maximize => {
            if half_or_less || stage >= 6 || max_hp == 1 {
                return Err(ConditionApplyError::NotEnoughHp);
            }
            ctx.apply_damage(DamageRequest::cost(user, (max_hp / 2) as u16, source));
            raise(ctx, user, MoveId::BELLY_DRUM, Stat::Attack, 12);
        }
        BellyDrumRule::ClimbUnderCap => {
            if stage >= 6 {
                return Err(ConditionApplyError::AlreadyActive);
            }
            if half_or_less {
                raise(ctx, user, MoveId::BELLY_DRUM, Stat::Attack, 2);
                return Err(ConditionApplyError::NotEnoughHp);
            }
            ctx.apply_damage(DamageRequest::cost(user, (max_hp / 2) as u16, source));
            let mut current = stage;
            while current < 6 {
                let left = current;
                current = (current + 2).min(6);
                if ctx.capped_stat_reading(user, Stat::Attack, left) >= 999 {
                    current -= 1;
                    break;
                }
            }
            raise(ctx, user, MoveId::BELLY_DRUM, Stat::Attack, current - stage);
        }
    }
    Ok(())
}

/// Fillet Away / Clangorous Soul's `onTry`: fail at or below `percent` of the max HP (or with 1
/// max HP).
fn affords(
    ctx: &BattleContext<'_>,
    user: PokemonId,
    percent: u32,
) -> Result<(), ConditionApplyError> {
    let (hp, max_hp) = hp(ctx, user);
    if 100 * hp <= max_hp * percent || max_hp == 1 {
        return Err(ConditionApplyError::NotEnoughHp);
    }
    Ok(())
}

pub fn fillet_away_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    affords(ctx, user, 50)
}

pub fn clangorous_soul_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    affords(ctx, user, 33)
}

/// Fillet Away / Clangorous Soul past their `onTry`: the stat stages, and — once one of them
/// moved — the HP cost of `percent` of the max HP.
fn pay_for_boosts(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    move_id: MoveId,
    percent: u32,
    boosts: &[(Stat, i8)],
) -> Result<(), ConditionApplyError> {
    let (_, max_hp) = hp(ctx, user);
    let mut any = false;
    for &(stat, stages) in boosts {
        any |= raise(ctx, user, move_id, stat, stages);
    }
    if !any {
        return Err(ConditionApplyError::AlreadyActive);
    }
    ctx.apply_damage(DamageRequest::cost(
        user,
        (max_hp * percent / 100) as u16,
        EffectSource::Move {
            pokemon: user,
            move_id,
        },
    ));
    Ok(())
}

pub fn fillet_away(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    pay_for_boosts(
        ctx,
        user,
        MoveId::FILLET_AWAY,
        50,
        &[
            (Stat::Attack, 2),
            (Stat::SpecialAttack, 2),
            (Stat::Speed, 2),
        ],
    )
}

pub fn clangorous_soul(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    pay_for_boosts(
        ctx,
        user,
        MoveId::CLANGOROUS_SOUL,
        33,
        &[
            (Stat::Attack, 1),
            (Stat::Defense, 1),
            (Stat::SpecialAttack, 1),
            (Stat::SpecialDefense, 1),
            (Stat::Speed, 1),
        ],
    )
}
