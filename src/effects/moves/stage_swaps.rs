//! Moves that clear, copy, swap or invert stat stages directly — no TryBoost, so Clear Body,
//! Mist or Contrary play no part (DESIGN.md §20.2, R131, R171): Haze (`onHitField`; Gen 1
//! `onHit`), Psych Up, Heart Swap, Power Swap, Guard Swap and Topsy-Turvy (`onHit`).

use crate::engine::{BattleContext, ConditionApplyError, CureReason, EffectSource, STAGED_STATS};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::{NonVolatileStatus, Stat};

fn the_target(targets: &[ResolvedTarget]) -> Result<PokemonId, ConditionApplyError> {
    match targets.first() {
        Some(ResolvedTarget::Pokemon(target)) => Ok(*target),
        _ => Err(ConditionApplyError::InvalidTarget),
    }
}

fn by(user: PokemonId, move_id: MoveId) -> EffectSource {
    EffectSource::Move {
        pokemon: user,
        move_id,
    }
}

/// Haze (Gen 2+ `onHitField`, Gen 1 `onHit`): every active Pokémon's stages back to 0 (Gen 1:
/// its modified stats back to the stored ones, `BattleContext::clear_stages`), and what the
/// generation's `HazeRules` add, Pokémon by Pokémon in field order: Gen 1 cures every other
/// Pokémon's status (a sleeping or frozen one that has not moved yet loses its action) and
/// turns the user's bad poison into poison; the volatiles it ends.
pub fn haze(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let rules = ctx.rules().conditions().haze;
    let source = by(user, MoveId::HAZE);
    for pokemon in ctx.active_pokemon() {
        ctx.clear_stages(pokemon, source);
        if rules.cures_statuses {
            if pokemon != user {
                if matches!(
                    ctx.status_of(pokemon),
                    Some(NonVolatileStatus::Sleep | NonVolatileStatus::Freeze)
                ) {
                    ctx.lose_action(pokemon);
                }
                ctx.cure_status(pokemon, CureReason::Move(MoveId::HAZE));
            } else if ctx.status_of(pokemon) == Some(NonVolatileStatus::Toxic) {
                ctx.replace_status(pokemon, NonVolatileStatus::Poison, source);
            }
        }
        for &kind in rules.ends {
            ctx.end_volatile(pokemon, kind);
        }
    }
    Ok(())
}

/// Psych Up: the user takes the target's stages, every one of them.
pub fn psych_up(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = the_target(targets)?;
    for stat in STAGED_STATS {
        let to = ctx.stage(target, stat);
        ctx.set_stage(user, stat, to, by(user, MoveId::PSYCH_UP));
    }
    if ctx.rules().conditions().psych_up_copies_crit_volatiles {
        ctx.copy_crit_volatiles(user, target);
    }
    Ok(())
}

/// The user and the target trade their stages in `stats`.
fn swap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
    stats: &[Stat],
    move_id: MoveId,
) -> Result<(), ConditionApplyError> {
    let target = the_target(targets)?;
    for &stat in stats {
        let (mine, theirs) = (ctx.stage(user, stat), ctx.stage(target, stat));
        ctx.set_stage(target, stat, mine, by(user, move_id));
        ctx.set_stage(user, stat, theirs, by(user, move_id));
    }
    Ok(())
}

pub fn heart_swap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    swap(ctx, user, targets, &STAGED_STATS, MoveId::HEART_SWAP)
}

pub fn power_swap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    swap(
        ctx,
        user,
        targets,
        &[Stat::Attack, Stat::SpecialAttack],
        MoveId::POWER_SWAP,
    )
}

pub fn guard_swap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    swap(
        ctx,
        user,
        targets,
        &[Stat::Defense, Stat::SpecialDefense],
        MoveId::GUARD_SWAP,
    )
}

/// Topsy-Turvy: the target's stages change sign; it fails when they are all 0.
pub fn topsy_turvy(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = the_target(targets)?;
    let mut inverted = false;
    for stat in STAGED_STATS {
        let stage = ctx.stage(target, stat);
        if stage != 0 {
            ctx.set_stage(target, stat, -stage, by(user, MoveId::TOPSY_TURVY));
            inverted = true;
        }
    }
    if inverted {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}
