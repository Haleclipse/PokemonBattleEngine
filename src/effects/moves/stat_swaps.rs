//! Moves that rewrite stored stats until the Pokémon leaves the field (Showdown `storedStats`,
//! DESIGN.md §20.1, R160): Power Split and Guard Split average the user's and the target's,
//! Speed Swap trades Speed, Power Trick swaps the user's own Attack and Defense.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::{Stat, VolatileConditionKind};

fn the_target(targets: &[ResolvedTarget]) -> Result<PokemonId, ConditionApplyError> {
    match targets.first() {
        Some(ResolvedTarget::Pokemon(target)) => Ok(*target),
        _ => Err(ConditionApplyError::InvalidTarget),
    }
}

/// Both Pokémon take the average (floored) of their stored `stats`.
fn split(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
    stats: [Stat; 2],
) -> Result<(), ConditionApplyError> {
    let target = the_target(targets)?;
    for stat in stats {
        let average = ((u32::from(ctx.stored_stat(user, stat))
            + u32::from(ctx.stored_stat(target, stat)))
            / 2) as u16;
        ctx.set_stored_stat(target, stat, average);
        ctx.set_stored_stat(user, stat, average);
    }
    Ok(())
}

pub fn power_split(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    split(ctx, user, targets, [Stat::Attack, Stat::SpecialAttack])
}

pub fn guard_split(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    split(ctx, user, targets, [Stat::Defense, Stat::SpecialDefense])
}

/// Speed Swap: the user and the target trade stored Speed.
pub fn speed_swap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = the_target(targets)?;
    let (mine, theirs) = (
        ctx.stored_stat(user, Stat::Speed),
        ctx.stored_stat(target, Stat::Speed),
    );
    ctx.set_stored_stat(target, Stat::Speed, mine);
    ctx.set_stored_stat(user, Stat::Speed, theirs);
    Ok(())
}

/// Power Trick (`volatileStatus`): the user's stored Attack and Defense trade places as it
/// starts; used again (`onRestart`) it ends, and they trade back (`onEnd`).
pub fn power_trick(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    if ctx.has_volatile(user, VolatileConditionKind::PowerTrick) {
        ctx.end_volatile(user, VolatileConditionKind::PowerTrick);
    } else {
        ctx.set_marker_volatile(user, VolatileConditionKind::PowerTrick)?;
    }
    let (attack, defense) = (
        ctx.stored_stat(user, Stat::Attack),
        ctx.stored_stat(user, Stat::Defense),
    );
    ctx.set_stored_stat(user, Stat::Attack, defense);
    ctx.set_stored_stat(user, Stat::Defense, attack);
    Ok(())
}
