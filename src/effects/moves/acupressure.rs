//! Acupressure (Showdown `onHit`, DESIGN.md §20.1, R166): one of the target's stats below +6
//! (the seven, in `boosts` order) drawn at random (`sample`) rises by 2; it fails when none can
//! — and in Gen 4 against a target behind a Substitute
//! (`ConditionRules::acupressure_fails_behind_substitute`).

use crate::engine::{
    BattleContext, ConditionApplyError, EffectSource, STAGED_STATS, StatChangeCause,
    StatChangeRequest,
};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::{Stat, VolatileConditionKind};

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    if ctx.rules().conditions().acupressure_fails_behind_substitute
        && ctx.has_volatile(target, VolatileConditionKind::Substitute)
    {
        return Err(ConditionApplyError::Immune);
    }
    let stats: Vec<Stat> = STAGED_STATS
        .into_iter()
        .filter(|stat| ctx.stage(target, *stat) < 6)
        .collect();
    if stats.is_empty() {
        return Err(ConditionApplyError::AlreadyActive);
    }
    let stat = stats[ctx.sample_index(stats.len())];
    ctx.try_change_stat_stage(StatChangeRequest {
        target,
        stat,
        stages: 2,
        source: EffectSource::Move {
            pokemon: user,
            move_id: MoveId::ACUPRESSURE,
        },
        cause: StatChangeCause::Move,
    });
    Ok(())
}
