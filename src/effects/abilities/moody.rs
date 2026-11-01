//! Moody (Showdown `onResidual`, order 28.2; DESIGN.md §20.2, R166): one stat below +6 drawn at
//! random rises by 2, then another above -6 (not that one) falls by 1 — the five stats, Gen 5-7
//! accuracy and evasion too (`ConditionRules::moody_draws_accuracy_evasion`).

use crate::engine::BattleContext;
use crate::ids::PokemonId;
use crate::state::Stat;

use super::hit_boosts::boost;

/// The stats Moody draws from, in Showdown's `boosts` order.
pub fn moody_stats(accuracy_evasion: bool) -> &'static [Stat] {
    const FIVE: [Stat; 5] = [
        Stat::Attack,
        Stat::Defense,
        Stat::SpecialAttack,
        Stat::SpecialDefense,
        Stat::Speed,
    ];
    if accuracy_evasion {
        &crate::engine::STAGED_STATS
    } else {
        &FIVE
    }
}

/// The stats that may rise: every one below +6 (`pokemon.boosts[stat] < 6`).
pub fn raisable(stage: impl Fn(Stat) -> i8, accuracy_evasion: bool) -> Vec<Stat> {
    moody_stats(accuracy_evasion)
        .iter()
        .copied()
        .filter(|stat| stage(*stat) < 6)
        .collect()
}

/// The stats that may fall: every one above -6 other than the one that rose
/// (`pokemon.boosts[stat] > -6 && stat !== randomStat`). Both filters read the holder's stages
/// **as the handler runs** — the replay takes them from the residual's own events (§20.2).
pub fn lowerable(
    stage: impl Fn(Stat) -> i8,
    accuracy_evasion: bool,
    raised: Option<Stat>,
) -> Vec<Stat> {
    moody_stats(accuracy_evasion)
        .iter()
        .copied()
        .filter(|stat| stage(*stat) > -6 && Some(*stat) != raised)
        .collect()
}

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let accuracy_evasion = ctx.rules().conditions().moody_draws_accuracy_evasion;
    let raisable = raisable(|stat| ctx.stage(holder, stat), accuracy_evasion);
    let raised = (!raisable.is_empty()).then(|| raisable[ctx.sample_index(raisable.len())]);
    let lowerable = lowerable(|stat| ctx.stage(holder, stat), accuracy_evasion, raised);
    let lowered = (!lowerable.is_empty()).then(|| lowerable[ctx.sample_index(lowerable.len())]);
    let changes: Vec<(Stat, i8)> = raised
        .map(|stat| (stat, 2))
        .into_iter()
        .chain(lowered.map(|stat| (stat, -1)))
        .collect();
    boost(ctx, holder, &changes);
}
