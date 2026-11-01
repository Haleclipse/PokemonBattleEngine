//! Moves that raise the user's stats with a side effect of their own (DESIGN.md §20.1,
//! R163, R169): No Retreat (it cannot switch out afterwards), Tidy Up (every Substitute and
//! entry hazard is cleared first) and Autotomize (100 kg lighter).

use crate::engine::{
    BattleContext, ConditionApplyError, EffectSource, StatChangeCause, StatChangeRequest,
    StatChangeResult,
};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::{Stat, VolatileConditionKind};

fn raise(ctx: &mut BattleContext<'_>, user: PokemonId, move_id: MoveId, stats: &[Stat]) -> bool {
    let mut any = false;
    for &stat in stats {
        any |= matches!(
            ctx.try_change_stat_stage(StatChangeRequest {
                target: user,
                stat,
                stages: 1,
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

/// No Retreat's `onTry`: fails when already there.
pub fn no_retreat_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    if ctx.has_volatile(user, VolatileConditionKind::NoRetreat) {
        return Err(ConditionApplyError::AlreadyActive);
    }
    Ok(())
}

/// No Retreat (`boosts`; `volatileStatus`): +1 to all five stats, then the volatile that keeps
/// it in — left out when Mean Look's kind already traps it (its `onTry`'s
/// `delete move.volatileStatus`).
pub fn no_retreat(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let trapped = ctx.has_volatile(user, VolatileConditionKind::Trapped);
    let raised = raise(
        ctx,
        user,
        MoveId::NO_RETREAT,
        &[
            Stat::Attack,
            Stat::Defense,
            Stat::SpecialAttack,
            Stat::SpecialDefense,
            Stat::Speed,
        ],
    );
    let started = !trapped
        && ctx
            .set_marker_volatile(user, VolatileConditionKind::NoRetreat)
            .is_ok();
    if raised || started {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}

/// Tidy Up (`onHit`): every active Pokémon's Substitute ends, the entry hazards come off both
/// sides, then +1 Attack and Speed; it works if any of that did.
pub fn tidy_up(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let mut cleared = false;
    for pokemon in ctx.active_pokemon() {
        cleared |= ctx.end_volatile(pokemon, VolatileConditionKind::Substitute);
    }
    cleared |= ctx.tidy_up_hazards(user);
    let raised = raise(ctx, user, MoveId::TIDY_UP, &[Stat::Attack, Stat::Speed]);
    if raised || cleared {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}

/// Autotomize (`onTryHit`, `boosts`, then `onHit` — the gen5 mod's `volatileStatus`): fails
/// when the user's Speed cannot move (+6, or -6 with Contrary, which turns the rise around);
/// else Speed +2 and 100 kg off (`BattleContext::autotomize`).
pub fn autotomize(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let limit = if ctx.has_ability(user, AbilityId::CONTRARY) {
        -6
    } else {
        6
    };
    if ctx.stage(user, Stat::Speed) == limit {
        return Err(ConditionApplyError::AlreadyActive);
    }
    ctx.try_change_stat_stage(StatChangeRequest {
        target: user,
        stat: Stat::Speed,
        stages: 2,
        source: EffectSource::Move {
            pokemon: user,
            move_id: MoveId::AUTOTOMIZE,
        },
        cause: StatChangeCause::Move,
    });
    ctx.autotomize(user);
    Ok(())
}
