//! `VolatileState`'s conditions that restrict what the Pokémon can choose: Taunt, Encore,
//! Disable, a Choice lock, Throat Chop, Heal Block, Embargo, a recharge.

use crate::ids::MoveId;

use super::*;

impl VolatileState {
    pub fn set_taunt(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::Taunt) {
            return false;
        }
        self.entries.push(VolatileCondition::Taunt { turns });
        true
    }

    pub fn set_encore(&mut self, move_id: MoveId, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::Encore) {
            return false;
        }
        self.entries
            .push(VolatileCondition::Encore { move_id, turns });
        true
    }

    pub fn set_disable(&mut self, move_id: MoveId, turns: u8) -> bool {
        self.set_disable_in(move_id, turns, None)
    }

    /// `set_disable`, with Gen 1's disabled slot.
    pub fn set_disable_in(&mut self, move_id: MoveId, turns: u8, slot: Option<u8>) -> bool {
        if self.contains(VolatileConditionKind::Disable) {
            return false;
        }
        self.entries.push(VolatileCondition::Disable {
            move_id,
            turns,
            slot,
        });
        true
    }

    pub fn set_must_recharge(&mut self, turns: Option<u8>) -> bool {
        if self.contains(VolatileConditionKind::MustRecharge) {
            return false;
        }
        self.entries.push(VolatileCondition::MustRecharge { turns });
        true
    }

    /// Locks (or re-locks onto the same move); a different move never replaces a live lock.
    pub fn set_choice_lock(&mut self, move_id: MoveId) -> bool {
        if self.contains(VolatileConditionKind::ChoiceLock) {
            return false;
        }
        self.entries.push(VolatileCondition::ChoiceLock { move_id });
        true
    }

    /// Embargo for `turns`; false when already there.
    pub fn set_embargo(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::Embargo) {
            return false;
        }
        self.entries.push(VolatileCondition::Embargo { turns });
        true
    }

    /// Heal Block for `turns`; false when already blocked (a second one does not renew it).
    pub fn set_heal_block(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::HealBlock) {
            return false;
        }
        self.entries.push(VolatileCondition::HealBlock { turns });
        true
    }

    /// Throat Chop for `turns`; false when already there (no renewal).
    pub fn set_throat_chop(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::ThroatChop) {
            return false;
        }
        self.entries.push(VolatileCondition::ThroatChop { turns });
        true
    }

    pub fn encored_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Encore { move_id, .. } => Some(*move_id),
            _ => None,
        })
    }

    pub fn disabled_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Disable { move_id, .. } => Some(*move_id),
            _ => None,
        })
    }

    /// Gen 1's disabled slot (`VolatileCondition::Disable::slot`).
    pub fn disabled_slot(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Disable { slot, .. } => *slot,
            _ => None,
        })
    }

    pub fn choice_locked_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::ChoiceLock { move_id } => Some(*move_id),
            _ => None,
        })
    }
}
