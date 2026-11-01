//! The Pledge combo (DESIGN.md §42.9): two allies using different Pledge moves in the same turn
//! combine into one 150-power move whose type and side condition the pair decides. The table is
//! Showdown's `onModifyMove` per (move, `sourceEffect`) pair.

use crate::ids::MoveId;
use crate::state::SideConditionKind;

/// Which side the combined move's condition lands on (Showdown's `self` versus `sideCondition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PledgeSide {
    /// The user's own side (`self: {sideCondition: …}`).
    User,
    /// The side the move aims at.
    Target,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PledgeCombo {
    pub move_type: crate::dex::PokemonType,
    pub condition: SideConditionKind,
    pub side: PledgeSide,
}

/// The combo `by` (the waiting ally's move) makes of `move` (the one that runs first), or `None`
/// when the pair is not two different Pledges.
pub fn combo(by: MoveId, combined: MoveId) -> Option<PledgeCombo> {
    use crate::dex::PokemonType;
    let pair = |a: MoveId, b: MoveId| (by == a && combined == b) || (by == b && combined == a);
    if by == combined {
        return None;
    }
    if pair(MoveId::FIRE_PLEDGE, MoveId::WATER_PLEDGE) {
        return Some(PledgeCombo {
            move_type: PokemonType::Water,
            condition: SideConditionKind::WaterPledge,
            side: PledgeSide::User,
        });
    }
    if pair(MoveId::FIRE_PLEDGE, MoveId::GRASS_PLEDGE) {
        return Some(PledgeCombo {
            move_type: PokemonType::Fire,
            condition: SideConditionKind::FirePledge,
            side: PledgeSide::Target,
        });
    }
    if pair(MoveId::WATER_PLEDGE, MoveId::GRASS_PLEDGE) {
        return Some(PledgeCombo {
            move_type: PokemonType::Grass,
            condition: SideConditionKind::GrassPledge,
            side: PledgeSide::Target,
        });
    }
    None
}

/// Whether `move` is one of the three Pledges.
pub fn is_pledge(move_id: MoveId) -> bool {
    matches!(
        move_id,
        MoveId::FIRE_PLEDGE | MoveId::GRASS_PLEDGE | MoveId::WATER_PLEDGE
    )
}
