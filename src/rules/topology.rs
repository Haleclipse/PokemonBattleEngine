use crate::ids::{PositionId, SideId};
use crate::spec::BattleMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionSpec {
    pub id: PositionId,
    pub side: SideId,
    pub slot: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleTopology {
    mode: BattleMode,
    active_slots_per_side: u8,
    positions: Vec<PositionSpec>,
}

impl BattleTopology {
    pub fn singles() -> Self {
        Self::build(BattleMode::Singles, 1)
    }

    pub fn doubles() -> Self {
        Self::build(BattleMode::Doubles, 2)
    }

    fn build(mode: BattleMode, active_slots_per_side: u8) -> Self {
        let mut positions = Vec::new();
        for side in [SideId::P1, SideId::P2] {
            for slot in 0..active_slots_per_side {
                let id = PositionId::new_unchecked(side.ordinal() * active_slots_per_side + slot);
                positions.push(PositionSpec { id, side, slot });
            }
        }

        Self {
            mode,
            active_slots_per_side,
            positions,
        }
    }

    pub fn mode(&self) -> BattleMode {
        self.mode
    }

    pub fn active_slots_per_side(&self) -> u8 {
        self.active_slots_per_side
    }

    pub fn positions(&self) -> &[PositionSpec] {
        &self.positions
    }

    pub fn position(&self, side: SideId, slot: u8) -> Option<PositionId> {
        self.positions
            .iter()
            .find(|position| position.side == side && position.slot == slot)
            .map(|position| position.id)
    }

    pub fn spec(&self, position: PositionId) -> Option<PositionSpec> {
        self.positions
            .iter()
            .copied()
            .find(|candidate| candidate.id == position)
    }

    pub fn all_foes(&self, source: PositionId) -> Vec<PositionId> {
        let Some(source) = self.spec(source) else {
            return Vec::new();
        };

        self.positions
            .iter()
            .filter(|candidate| candidate.side != source.side)
            .map(|candidate| candidate.id)
            .collect()
    }

    pub fn adjacent_foes(&self, source: PositionId) -> Vec<PositionId> {
        // Singles and Doubles both treat all opposing active slots as adjacent.
        // Triples can later replace this with a real adjacency graph without changing effects.
        self.all_foes(source)
    }

    /// Allies first, then foes: the order Showdown resolves `allAdjacent` targets in (and
    /// reports a spread move's hits).
    pub fn all_adjacent(&self, source: PositionId) -> Vec<PositionId> {
        let mut positions = self.adjacent_allies(source);
        positions.extend(self.adjacent_foes(source));
        positions
    }

    pub fn adjacent_allies(&self, source: PositionId) -> Vec<PositionId> {
        let Some(source) = self.spec(source) else {
            return Vec::new();
        };

        self.positions
            .iter()
            .filter(|candidate| candidate.side == source.side && candidate.id != source.id)
            .map(|candidate| candidate.id)
            .collect()
    }
}
