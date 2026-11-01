//! Moves that raise or lower the stats of whoever on the field qualifies (DESIGN.md §20.1,
//! R157): Flower Shield and Rototiller (`onHitField`: the Grass types — Rototiller's grounded
//! ones), Gear Up and Magnetic Flux (`onHitSide`: the user's side's Plus / Minus holders),
//! Venom Drench (`onHit`: poisoned foes), and Happy Hour, which does nothing in a battle.

use crate::dex::PokemonType;
use crate::engine::{
    BattleContext, ConditionApplyError, EffectSource, StatChangeCause, StatChangeRequest,
    StatChangeResult,
};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::{NonVolatileStatus, Stat};

/// The stat changes a move makes on `target`; `true` when one moved.
fn change(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    move_id: MoveId,
    target: PokemonId,
    changes: &[(Stat, i8)],
) -> bool {
    let mut any = false;
    for &(stat, stages) in changes {
        any |= matches!(
            ctx.try_change_stat_stage(StatChangeRequest {
                target,
                stat,
                stages,
                source: EffectSource::Move {
                    pokemon: user,
                    move_id,
                },
                cause: StatChangeCause::Move,
            }),
            StatChangeResult::Applied { .. }
        );
    }
    any
}

fn done(any: bool) -> Result<(), ConditionApplyError> {
    if any {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}

/// Flower Shield: +1 Defense to every Grass type on the field but one Max Guard keeps it off
/// (`!pokemon.volatiles['maxguard'] || runEvent('TryHit', …)`, the targets gathered first); it
/// fails when none rose.
pub fn flower_shield(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let grass: Vec<PokemonId> = ctx
        .active_pokemon()
        .into_iter()
        .filter(|&pokemon| !ctx.is_fainted(pokemon) && ctx.has_type(pokemon, PokemonType::Grass))
        .collect();
    let reached: Vec<PokemonId> = grass
        .into_iter()
        .filter(|&pokemon| !ctx.max_guard_refuses(user, pokemon, MoveId::FLOWER_SHIELD))
        .collect();
    let mut any = false;
    for pokemon in reached {
        any |= change(
            ctx,
            user,
            MoveId::FLOWER_SHIELD,
            pokemon,
            &[(Stat::Defense, 1)],
        );
    }
    done(any)
}

/// Rototiller: +1 Attack and Special Attack to every grounded Grass type on the field; an
/// airborne Pokémon is immune (`-immune`). It fails only when there is neither a grounded
/// Grass type nor an airborne Pokémon — the boosts themselves may change nothing.
pub fn rototiller(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let mut grass = Vec::new();
    let mut airborne = false;
    for pokemon in ctx.active_pokemon() {
        if ctx.is_fainted(pokemon) {
            continue;
        }
        if !ctx.is_grounded(pokemon) {
            airborne = true;
        } else if ctx.has_type(pokemon, PokemonType::Grass) {
            grass.push(pokemon);
        }
    }
    if grass.is_empty() && !airborne {
        return Err(ConditionApplyError::InvalidTarget);
    }
    for pokemon in grass {
        change(
            ctx,
            user,
            MoveId::ROTOTILLER,
            pokemon,
            &[(Stat::Attack, 1), (Stat::SpecialAttack, 1)],
        );
    }
    Ok(())
}

/// Gear Up / Magnetic Flux: `changes` to each standing Plus or Minus holder on the user's side;
/// they fail when there is none, or none of the stages moved.
fn plus_minus_side(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
    move_id: MoveId,
    changes: &[(Stat, i8)],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Side(side)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let holders: Vec<PokemonId> = ctx
        .active_pokemon()
        .into_iter()
        .filter(|pokemon| {
            ctx.side_of(*pokemon) == Some(side)
                && !ctx.is_fainted(*pokemon)
                && matches!(
                    ctx.ability_of(*pokemon),
                    Some(AbilityId::PLUS | AbilityId::MINUS)
                )
        })
        .collect();
    // A Max Guard keeps it off its holder (`runEvent('TryHit', …)`, as Flower Shield).
    let holders: Vec<PokemonId> = holders
        .into_iter()
        .filter(|&holder| !ctx.max_guard_refuses(user, holder, move_id))
        .collect();
    let mut any = false;
    for holder in holders {
        any |= change(ctx, user, move_id, holder, changes);
    }
    done(any)
}

pub fn gear_up(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    plus_minus_side(
        ctx,
        user,
        targets,
        MoveId::GEAR_UP,
        &[(Stat::Attack, 1), (Stat::SpecialAttack, 1)],
    )
}

pub fn magnetic_flux(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    plus_minus_side(
        ctx,
        user,
        targets,
        MoveId::MAGNETIC_FLUX,
        &[(Stat::Defense, 1), (Stat::SpecialDefense, 1)],
    )
}

/// Venom Drench: -1 Attack, Special Attack and Speed on each poisoned or badly poisoned target
/// it reached; it fails when none of them fell.
pub fn venom_drench(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let mut any = false;
    for target in targets {
        let ResolvedTarget::Pokemon(target) = *target else {
            continue;
        };
        if matches!(
            ctx.status_of(target),
            Some(NonVolatileStatus::Poison | NonVolatileStatus::Toxic)
        ) {
            any |= change(
                ctx,
                user,
                MoveId::VENOM_DRENCH,
                target,
                &[
                    (Stat::Attack, -1),
                    (Stat::SpecialAttack, -1),
                    (Stat::Speed, -1),
                ],
            );
        }
    }
    done(any)
}

/// Happy Hour: announces itself (`-activate`) and changes nothing in the battle.
pub fn happy_hour(
    _ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    Ok(())
}
