//! Spectral Thief (Showdown `stealsBoosts`, `hitStepStealBoosts`; DESIGN.md §20.1, R169):
//! once through the hit steps, before any hit, the target's positive stages go to the user.

use crate::engine::{EffectSource, StatChangeCause, StatChangeRequest, context::STAGED_STATS};
use crate::ids::{MoveId, PokemonId};

use super::BattleContext;

impl<'a> BattleContext<'a> {
    /// The target's positive stages (`-clearpositiveboost`) are raised on the user — a stat
    /// change of its own move (`boost(boosts, pokemon, pokemon)`: Contrary, Simple and the rest
    /// apply) — then set to 0 on the target (`setBoost`); each is a `StatChanged`.
    pub(crate) fn steal_boosts(&mut self, user: PokemonId, target: PokemonId, move_id: MoveId) {
        let stolen: Vec<_> = STAGED_STATS
            .into_iter()
            .map(|stat| (stat, self.stage(target, stat)))
            .filter(|(_, stage)| *stage > 0)
            .collect();
        if stolen.is_empty() {
            return;
        }
        let source = EffectSource::Move {
            pokemon: user,
            move_id,
        };
        for &(stat, stages) in &stolen {
            self.try_change_stat_stage(StatChangeRequest {
                target: user,
                stat,
                stages,
                source,
                cause: StatChangeCause::Move,
            });
        }
        for (stat, _) in stolen {
            self.set_stage(target, stat, 0, source);
        }
    }
}
