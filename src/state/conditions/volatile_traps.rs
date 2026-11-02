//! `VolatileState`'s traps: a binding move's partial trap and its lock, being trapped (Mean
//! Look, Block), Octolock, Ingrain, Commander.

use crate::ids::{MoveId, PokemonId};

use super::*;

impl VolatileState {
    pub fn set_partially_trapped(&mut self, source: PokemonId, move_id: MoveId, turns: u8) -> bool {
        self.set_partially_trapped_banded(source, move_id, turns, false)
    }

    /// `set_partially_trapped`, with whether the trapper held Binding Band.
    pub fn set_partially_trapped_banded(
        &mut self,
        source: PokemonId,
        move_id: MoveId,
        turns: u8,
        banded: bool,
    ) -> bool {
        if self.contains(VolatileConditionKind::PartiallyTrapped) {
            return false;
        }
        self.entries.push(VolatileCondition::PartiallyTrapped {
            source,
            move_id,
            turns,
            banded,
        });
        true
    }

    /// Whether the trapper held Binding Band as it bound this Pokémon.
    pub fn partially_trapped_banded(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry,
                VolatileCondition::PartiallyTrapped { banded: true, .. }
            )
        })
    }

    /// The trapper and the move that bound this Pokémon.
    pub fn partially_trapped_by(&self) -> Option<(PokemonId, MoveId)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::PartiallyTrapped {
                source, move_id, ..
            } => Some((*source, *move_id)),
            _ => None,
        })
    }

    pub fn set_ingrain(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Ingrain) {
            return false;
        }
        self.entries.push(VolatileCondition::Ingrain);
        true
    }

    /// Commander's two volatiles (`addVolatile('commanding')` / `('commanded')`): false when
    /// the holder already has one.
    pub fn set_commanding(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Commanding) {
            return false;
        }
        self.entries.push(VolatileCondition::Commanding);
        true
    }

    pub fn set_commanded(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Commanded) {
            return false;
        }
        self.entries.push(VolatileCondition::Commanded);
        true
    }

    /// The volatiles that last only while their source stays on the field (Showdown's
    /// `onUpdate` checks `effectState.source.isActive`), with that source: Attract, Syrup Bomb.
    pub fn bound(&self) -> Vec<(VolatileConditionKind, PokemonId)> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                VolatileCondition::Attract { source }
                | VolatileCondition::SyrupBomb { source, .. } => Some((entry.kind(), *source)),
                _ => None,
            })
            .collect()
    }

    /// Showdown `addVolatile('trapped', source)`: fails while the holder is already trapped.
    pub fn set_trapped(&mut self, source: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::Trapped) {
            return false;
        }
        self.entries.push(VolatileCondition::Trapped { source });
        true
    }

    /// Who keeps the holder trapped (Mean Look's user).
    pub fn trapped_by(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Trapped { source } => Some(*source),
            _ => None,
        })
    }

    /// The holder is now a trapper; returns whether it was not one yet.
    pub fn set_trapper(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Trapper) {
            return false;
        }
        self.entries.push(VolatileCondition::Trapper);
        true
    }

    /// Gen 1's `partialtrappinglock`: the lock a binding move's first hit starts.
    pub fn set_partial_trapping_lock(
        &mut self,
        target: PokemonId,
        move_id: MoveId,
        damage: u16,
        turns: u8,
    ) {
        self.remove(VolatileConditionKind::PartialTrappingLock);
        self.entries.push(VolatileCondition::PartialTrappingLock {
            target,
            move_id,
            damage,
            turns,
            total: turns,
        });
    }

    /// The Gen 1 binding lock: (target, move, damage, turns left, turns rolled).
    pub fn partial_trapping_lock(&self) -> Option<(PokemonId, MoveId, u16, u8, u8)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::PartialTrappingLock {
                target,
                move_id,
                damage,
                turns,
                total,
            } => Some((*target, *move_id, *damage, *turns, *total)),
            _ => None,
        })
    }

    /// Gen 1's `fakepartiallytrapped` (duration 2), facing `counterpart`.
    pub fn set_fake_partially_trapped(&mut self, counterpart: PokemonId) {
        self.remove(VolatileConditionKind::FakePartiallyTrapped);
        self.entries.push(VolatileCondition::FakePartiallyTrapped {
            counterpart,
            turns: 2,
        });
    }

    pub fn fake_partially_trapped_counterpart(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::FakePartiallyTrapped { counterpart, .. } => Some(*counterpart),
            _ => None,
        })
    }

    /// Showdown `addVolatile('octolock', source)`: fails while one is already there.
    pub fn set_octolock(&mut self, source: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::Octolock) {
            return false;
        }
        self.entries.push(VolatileCondition::Octolock { source });
        true
    }

    /// Who octolocked the holder.
    pub fn octolocked_by(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Octolock { source } => Some(*source),
            _ => None,
        })
    }

    /// Gen 2-4 Baton Pass (`copyVolatileFrom` moves the links): the holder is now trapped by
    /// `to`, which took `from`'s place.
    pub fn retarget_trap(&mut self, from: PokemonId, to: PokemonId) {
        for entry in &mut self.entries {
            if let VolatileCondition::Trapped { source } = entry
                && *source == from
            {
                *source = to;
            }
        }
    }
}
