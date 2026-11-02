//! `VolatileState`'s multi-turn moves and locks: a two-turn move's charge and its hiding, Sky
//! Drop, Rollout's chain and Gen 7's storage, Thrash's rampage, Fury Cutter, Rage, Uproar,
//! Bide, the Metronome item's count, Focus Punch's focus, Shell Trap.

use crate::ids::{MoveId, PokemonId, PositionId};

use super::*;

impl VolatileState {
    /// Starts a two-turn move's charge, out of reach where `hides` says while it lasts; a
    /// Pokémon already in one is left alone.
    pub fn set_two_turn_move(
        &mut self,
        move_id: MoveId,
        turns: u8,
        target: Option<PositionId>,
        hides: Option<crate::rules::SemiInvulnerableKind>,
    ) -> bool {
        if self.contains(VolatileConditionKind::TwoTurnMove) {
            return false;
        }
        self.entries.push(VolatileCondition::TwoTurnMove {
            move_id,
            turns,
            charging: true,
            target,
            hiding: hides.map(|kind| (kind, turns)),
        });
        true
    }

    /// Gen 1 Fly / Dig's hiding starts (`addVolatile('invulnerability')`); false when the
    /// Pokémon is hidden already.
    pub fn set_invulnerability(&mut self, kind: crate::rules::SemiInvulnerableKind) -> bool {
        if self.contains(VolatileConditionKind::Invulnerability) {
            return false;
        }
        self.entries
            .push(VolatileCondition::Invulnerability { kind });
        true
    }

    /// Sky Drop: its user carries `target` up; false when it carries someone already.
    pub fn set_sky_drop(&mut self, target: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::SkyDrop) {
            return false;
        }
        self.entries
            .push(VolatileCondition::SkyDrop { target, turns: 2 });
        true
    }

    /// The Pokémon this one carries up with Sky Drop, if it does.
    pub fn sky_drop_target(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::SkyDrop { target, .. } => Some(*target),
            _ => None,
        })
    }

    /// Sky Drop carries this Pokémon up, by `by`.
    pub fn set_sky_dropped(&mut self, by: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::SkyDropped) {
            return false;
        }
        self.entries.push(VolatileCondition::SkyDropped { by });
        true
    }

    /// The Pokémon carrying this one up with Sky Drop, if any.
    pub fn sky_dropped_by(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::SkyDropped { by } => Some(*by),
            _ => None,
        })
    }

    /// Where Gen 1 Fly / Dig's hiding keeps the Pokémon, if it does.
    pub fn invulnerability(&self) -> Option<crate::rules::SemiInvulnerableKind> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Invulnerability { kind } => Some(*kind),
            _ => None,
        })
    }

    /// The move a two-turn move locks the Pokémon into (charging or already released).
    /// Rollout / Ice Ball as it starts (`addVolatile`): no hit yet, one turn; false when a chain
    /// is already going.
    pub fn start_rolling(&mut self, move_id: MoveId, target: Option<PositionId>) -> bool {
        if self.contains(VolatileConditionKind::Rolling) {
            return false;
        }
        self.entries.push(VolatileCondition::Rolling {
            move_id,
            hits: 0,
            doublings: 0,
            turns: 1,
            target,
        });
        true
    }

    /// The doublings the Rollout / Ice Ball chain's power has taken (`contactHitCount`).
    pub fn rolling_doublings(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rolling { doublings, .. } => Some(*doublings),
            _ => None,
        })
    }

    /// Gen 7 Disguise takes a hit of `move_id`'s chain (`contactHitCount--`): the hit counts,
    /// its doubling does not.
    pub fn delay_rolling(&mut self, move_id: MoveId) {
        for entry in &mut self.entries {
            if let VolatileCondition::Rolling {
                move_id: rolling,
                doublings,
                ..
            } = entry
                && *rolling == move_id
            {
                *doublings = doublings.saturating_sub(1);
            }
        }
    }

    /// The Rollout storage (`next`, `armed`), if the holder has one.
    pub fn rollout_storage(&self) -> Option<(u8, bool)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::RolloutStorage { next, armed } => Some((*next, *armed)),
            _ => None,
        })
    }

    /// A chain over with `doublings` short of its hits: the rest stored, from `doublings` on.
    pub fn store_rollout(&mut self, doublings: u8) {
        self.remove(VolatileConditionKind::RolloutStorage);
        self.entries.push(VolatileCondition::RolloutStorage {
            next: doublings,
            armed: false,
        });
    }

    /// A Rollout / Ice Ball starts on the storage: no chain, and a failure clears it.
    pub fn arm_rollout_storage(&mut self) {
        for entry in &mut self.entries {
            if let VolatileCondition::RolloutStorage { armed, .. } = entry {
                *armed = true;
            }
        }
    }

    /// A power computed on the storage: the doublings it takes, and whether that was the last
    /// stored one (the fifth hit's) — else the next waits for the power after it.
    pub fn take_rollout_storage(&mut self) -> Option<(u8, bool)> {
        let (doublings, _) = self.rollout_storage()?;
        for entry in &mut self.entries {
            if let VolatileCondition::RolloutStorage { next, armed } = entry {
                *next = next.saturating_add(1);
                *armed = false;
            }
        }
        Some((doublings, doublings >= ROLLOUT_STORED_UP_TO))
    }

    /// The Rollout / Ice Ball chain: the move, its hits so far and the position it aims at.
    pub fn rolling(&self) -> Option<(MoveId, u8, Option<PositionId>)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rolling {
                move_id,
                hits,
                target,
                ..
            } => Some((*move_id, *hits, *target)),
            _ => None,
        })
    }

    /// A Rollout / Ice Ball hit computed its damage (`basePowerCallback`): one more hit and
    /// doubling, and before the fifth hit the chain lasts another turn (`duration = 2`).
    pub fn advance_rolling(&mut self, max_hits: u8) {
        for entry in &mut self.entries {
            if let VolatileCondition::Rolling {
                hits,
                doublings,
                turns,
                ..
            } = entry
            {
                *hits = hits.saturating_add(1);
                *doublings = doublings.saturating_add(1);
                if *hits < max_hits {
                    *turns = 2;
                }
            }
        }
    }

    pub fn two_turn_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove { move_id, .. } => Some(*move_id),
            _ => None,
        })
    }

    /// Where the charge the Pokémon is in keeps it out of reach, if it does.
    pub fn charging_hides(&self) -> Option<crate::rules::SemiInvulnerableKind> {
        self.charge_hiding().map(|(kind, _)| kind)
    }

    /// The hiding move's own condition while the Pokémon charges (Showdown `fly`, `dig` …):
    /// where it hides and its count.
    pub fn charge_hiding(&self) -> Option<(crate::rules::SemiInvulnerableKind, u8)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove {
                charging: true,
                hiding,
                ..
            } => *hiding,
            _ => None,
        })
    }

    /// One residual of the hiding move's own condition: `Some(true)` when it ran out (the user
    /// is out of hiding), `None` without one.
    pub fn count_down_charge_hiding(&mut self) -> Option<bool> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove {
                charging: true,
                hiding,
                ..
            } => {
                let (_, turns) = hiding.as_mut()?;
                *turns = turns.saturating_sub(1);
                let ended = *turns == 0;
                if ended {
                    *hiding = None;
                }
                Some(ended)
            }
            _ => None,
        })
    }

    /// The move the Pokémon is charging right now (semi-invulnerable if the move says so).
    pub fn charging_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove {
                move_id,
                charging: true,
                ..
            } => Some(*move_id),
            _ => None,
        })
    }

    pub fn two_turn_target(&self) -> Option<PositionId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove { target, .. } => *target,
            _ => None,
        })
    }

    /// The charge is over: the attack goes out this action (Showdown removes the move's own
    /// condition; `twoturnmove` stays until the end of the turn).
    pub fn release_two_turn_move(&mut self) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::TwoTurnMove { charging, .. } = entry
                && *charging
            {
                *charging = false;
                return true;
            }
        }
        false
    }

    /// Gen 1 Reflect / Light Screen, Gen 1-2 Mist, Gen 3-5 Mud Sport / Water Sport on the
    /// user; false when it already has it (Gen 1's `onTryHit`, `addVolatile` failing) or `kind`
    /// is none of them.
    /// The Metronome item's count, starting at none; false when the holder already has one
    /// (`addVolatile` fails and the count goes on).
    pub fn start_metronome(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Metronome) {
            return false;
        }
        self.entries.push(VolatileCondition::Metronome {
            last_move: None,
            consecutive: 0,
        });
        true
    }

    /// The Metronome item's count: the move it counts and how many times in a row.
    pub fn metronome(&self) -> Option<(Option<MoveId>, u8)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Metronome {
                last_move,
                consecutive,
            } => Some((*last_move, *consecutive)),
            _ => None,
        })
    }

    /// Sets the Metronome item's count, where there is one.
    pub fn set_metronome(&mut self, move_id: Option<MoveId>, count: u8) {
        for entry in &mut self.entries {
            if let VolatileCondition::Metronome {
                last_move,
                consecutive,
            } = entry
            {
                *last_move = move_id;
                *consecutive = count;
            }
        }
    }

    /// Bide as it starts (`addVolatile('bide')`): false when already there.
    pub fn start_bide(&mut self, turns: Option<u8>, time: u8, traps: bool) -> bool {
        if self.contains(VolatileConditionKind::Bide) {
            return false;
        }
        self.entries.push(VolatileCondition::Bide(BideState {
            turns,
            time,
            damage: 0,
            source: None,
            traps,
        }));
        true
    }

    /// Thrash and its kind (`lockedmove`).
    pub fn rampage(&self) -> Option<&RampageState> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rampage(rampage) => Some(rampage),
            _ => None,
        })
    }

    pub fn rampage_mut(&mut self) -> Option<&mut RampageState> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::Rampage(rampage) => Some(rampage),
            _ => None,
        })
    }

    /// Starts the lock; false when the Pokémon already has one.
    pub fn start_rampage(&mut self, rampage: RampageState) -> bool {
        if self.contains(VolatileConditionKind::Rampage) {
            return false;
        }
        self.entries.push(VolatileCondition::Rampage(rampage));
        true
    }

    /// Fury Cutter's chain: its level (Gen 5+ the multiplier, Gen 2-4 the count) and the turns
    /// left (Gen 5+).
    pub fn fury_cutter(&self) -> Option<(u8, Option<u8>)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::FuryCutter { level, turns } => Some((*level, *turns)),
            _ => None,
        })
    }

    /// Starts Fury Cutter's chain at `level`, or sets the one there; true when it started.
    pub fn set_fury_cutter(&mut self, level: u8, turns: Option<u8>) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::FuryCutter {
                level: current,
                turns: left,
            } = entry
            {
                *current = level;
                *left = turns;
                return false;
            }
        }
        self.entries
            .push(VolatileCondition::FuryCutter { level, turns });
        true
    }

    /// Rage's count (Gen 2; 0 elsewhere), while the Pokémon is raging.
    pub fn rage(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rage { counter, .. } => Some(*counter),
            _ => None,
        })
    }

    /// The Pokémon starts raging (its count at 0) — Gen 1 (`locks`) locked into Rage, its
    /// accuracy byte at 255; false when it was already.
    pub fn start_rage(&mut self, locks: bool) -> bool {
        if self.rage().is_some() {
            return false;
        }
        self.entries.push(VolatileCondition::Rage {
            counter: 0,
            accuracy: locks.then_some(255),
        });
        true
    }

    /// Gen 2: a hit builds the raging Pokémon's count, up to 255 (pokecrystal `inc a; ret z`);
    /// false when it is not raging.
    pub fn count_rage(&mut self) -> bool {
        self.entries.iter_mut().any(|entry| match entry {
            VolatileCondition::Rage { counter, .. } => {
                *counter = counter.saturating_add(1);
                true
            }
            _ => false,
        })
    }

    /// Gen 1 Rage's lock: its accuracy byte.
    pub fn rage_lock(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rage { accuracy, .. } => *accuracy,
            _ => None,
        })
    }

    /// Gen 1 Rage's lock: the byte this use's check ended with.
    pub fn set_rage_lock(&mut self, byte: u8) {
        for entry in &mut self.entries {
            if let VolatileCondition::Rage {
                accuracy: Some(accuracy),
                ..
            } = entry
            {
                *accuracy = byte;
            }
        }
    }

    /// Uproar.
    pub fn uproar(&self) -> Option<&UproarState> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Uproar(uproar) => Some(uproar),
            _ => None,
        })
    }

    pub fn uproar_mut(&mut self) -> Option<&mut UproarState> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::Uproar(uproar) => Some(uproar),
            _ => None,
        })
    }

    /// Starts the uproar; false when the Pokémon already makes one.
    pub fn start_uproar(&mut self, uproar: UproarState) -> bool {
        if self.contains(VolatileConditionKind::Uproar) {
            return false;
        }
        self.entries.push(VolatileCondition::Uproar(uproar));
        true
    }

    pub fn bide(&self) -> Option<&BideState> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Bide(bide) => Some(bide),
            _ => None,
        })
    }

    pub fn bide_mut(&mut self) -> Option<&mut BideState> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::Bide(bide) => Some(bide),
            _ => None,
        })
    }

    /// Focus Punch's volatile: a damaging hit landed on the holder (`lostFocus`).
    pub fn lose_focus(&mut self) {
        for entry in &mut self.entries {
            if let VolatileCondition::FocusPunch { lost_focus, .. } = entry {
                *lost_focus = true;
            }
        }
    }

    pub fn lost_focus(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry,
                VolatileCondition::FocusPunch {
                    lost_focus: true,
                    ..
                }
            )
        })
    }

    /// Shell Trap's volatile: a foe's physical hit armed it (`gotHit`); false when it already
    /// was, or there is none.
    pub fn arm_shell_trap(&mut self) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::ShellTrap { got_hit, .. } = entry {
                let fresh = !*got_hit;
                *got_hit = true;
                return fresh;
            }
        }
        false
    }

    pub fn shell_trap_armed(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry, VolatileCondition::ShellTrap { got_hit: true, .. }))
    }
}
