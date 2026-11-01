//! Curse (DESIGN.md §15.16): two moves in one, by its user's type as it is used. Not a Ghost
//! type: Speed down, then Attack and Defense up (the cartridge's order, Gen 5+ Showdown's).
//! A Ghost type: a curse on the target (`BattleContext::curse`), for half the user's max HP.

use crate::dex::PokemonType;
use crate::engine::{
    BattleContext, ConditionApplyError, EffectSource, StatChangeCause, StatChangeRequest,
    StatChangeResult,
};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::Stat;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    // The kind is the user's type as the move hits (its `onTryHit`), the target as it was
    // chosen (its `onModifyMove`): a user Protean made a Ghost after a non-Ghost Curse aimed at
    // itself curses itself (DESIGN.md R263).
    if ctx.has_type(user, PokemonType::Ghost) {
        let target = targets
            .iter()
            .find_map(|target| match target {
                ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
                _ => None,
            })
            .ok_or(ConditionApplyError::InvalidTarget)?;
        return ctx.curse(user, target);
    }
    // Gen 2 (the cartridge, `CurseRules`): with Attack and Defense both at +6 it fails.
    if ctx
        .rules()
        .conditions()
        .curse
        .fails_when_attack_and_defense_maxed
        && ctx.stage(user, Stat::Attack) >= 6
        && ctx.stage(user, Stat::Defense) >= 6
    {
        return Err(ConditionApplyError::AlreadyActive);
    }
    let source = EffectSource::Move {
        pokemon: user,
        move_id: MoveId::CURSE,
    };
    let changed = [(Stat::Speed, -1), (Stat::Attack, 1), (Stat::Defense, 1)]
        .into_iter()
        .fold(false, |any, (stat, stages)| {
            let result = ctx.try_change_stat_stage(StatChangeRequest {
                target: user,
                stat,
                stages,
                source,
                cause: StatChangeCause::Move,
            });
            matches!(result, StatChangeResult::Applied { .. }) || any
        });
    if changed {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}
