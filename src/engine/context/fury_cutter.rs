//! Fury Cutter's chain (DESIGN.md §15.18), as `FuryCutterRules` keeps it.

use crate::engine::TraceEvent;
use crate::ids::PokemonId;
use crate::rules::FuryCutterModel;
use crate::state::VolatileConditionKind;

use super::BattleContext;

/// Gen 2-4: the count stops growing here (the cartridges double at most four times).
const COUNT_CAP: u8 = 5;

impl<'a> BattleContext<'a> {
    /// A use of Fury Cutter reaching the damage calculation (Showdown `basePowerCallback`; the
    /// cartridges' count after the accuracy check): Gen 5+ starts the volatile or doubles its
    /// multiplier up to the cap, for 2 turns again; Gen 2-4 count the use.
    pub fn advance_fury_cutter(&mut self, user: PokemonId) {
        let Some(pokemon) = self.state.pokemon_mut(user) else {
            return;
        };
        let current = pokemon.volatiles.fury_cutter();
        let (level, turns) = match self.rules.conditions.fury_cutter.model {
            FuryCutterModel::Volatile { cap } => match current {
                None => (1, Some(2)),
                Some((level, _)) => (if level < cap { level * 2 } else { level }, Some(2)),
            },
            FuryCutterModel::Count { .. } => (
                current.map_or(1, |(count, _)| (count + 1).min(COUNT_CAP)),
                None,
            ),
        };
        if pokemon.volatiles.set_fury_cutter(level, turns) {
            self.trace.push(TraceEvent::VolatileStarted {
                target: user,
                kind: VolatileConditionKind::FuryCutter,
            });
        }
    }

    /// The chain ends (the cartridges' count back to 0).
    pub fn end_fury_cutter(&mut self, user: PokemonId) {
        self.end_volatile(user, VolatileConditionKind::FuryCutter);
    }
}
