//! Stuff Cheeks and Teatime (DESIGN.md R221): moves that make Pokémon eat their berries
//! whatever would refuse it (`eatItem(true)`: `BattleContext::force_eat_item`).

use crate::engine::{
    BattleContext, ConditionApplyError, EffectSource, StatChangeCause, StatChangeRequest,
    StatChangeResult,
};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::Stat;

/// Stuff Cheeks (`onHit`): the user's Defense +2 — the move fails (`null`) when the stage did
/// not change — then it eats its berry.
pub fn stuff_cheeks(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let raised = ctx.try_change_stat_stage(StatChangeRequest {
        target: user,
        stat: Stat::Defense,
        stages: 2,
        source: EffectSource::Move {
            pokemon: user,
            move_id: MoveId::STUFF_CHEEKS,
        },
        cause: StatChangeCause::Move,
    });
    if !matches!(raised, StatChangeResult::Applied { .. }) {
        return Err(ConditionApplyError::NothingToDo);
    }
    ctx.force_eat_item(user);
    Ok(())
}

/// Teatime (`onHitField`): every active Pokémon holding a berry eats it — but one out of reach
/// in a charge (`Invulnerability`: `-miss`) or whose Max Guard, ability or item refuses the
/// move (`TryHit`). With nobody to eat, `-fail`, yet the move has not failed (`NOT_FAIL`).
pub fn teatime(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let mut eaters = Vec::new();
    for pokemon in ctx.active_pokemon() {
        // `onHitField`: each Pokémon's `Invulnerability`, then its `runEvent('TryHit')` — a Max
        // Guard holder's refuses it before any berry is asked about (R-M11-45).
        if ctx.is_fainted(pokemon)
            || ctx.out_of_reach(user, pokemon)
            || ctx.max_guard_refuses(user, pokemon, MoveId::TEATIME)
            || ctx.holders_refuse(user, pokemon, MoveId::TEATIME)
        {
            continue;
        }
        if ctx.holds_berry(pokemon) {
            eaters.push(pokemon);
        }
    }
    for pokemon in eaters {
        ctx.force_eat_item(pokemon);
    }
    Ok(())
}
