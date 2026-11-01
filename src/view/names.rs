//! The names of a battle's species, moves, abilities and items (DESIGN.md §47.0 item 15): Showdown's
//! English names, as the battle's generation has them, and the ids they name — for a host to show
//! what the views give by id, and to read a name back. Read-only; it shares the battle's dex (no
//! copy), and holds no borrow of the battle, which goes on while the names are kept.

use std::sync::Arc;

use crate::BattleEngine;
use crate::dex::ResolvedDex;
use crate::ids::{AbilityId, ItemId, MoveId, SpeciesId};

/// The names the battle's generation gives its ids (`BattleEngine::names`). An id or a name that
/// generation does not have is `None`; a name is read as Showdown reads one (`toID`: case, spaces
/// and punctuation aside — `Headlong Rush`, `headlongrush`).
#[derive(Clone)]
pub struct Names {
    dex: Arc<ResolvedDex>,
}

impl Names {
    pub fn species(&self, id: SpeciesId) -> Option<&str> {
        self.dex
            .species_by_id(id)
            .map(|species| species.name.as_str())
    }

    /// A move's name (`move` is a keyword).
    pub fn move_(&self, id: MoveId) -> Option<&str> {
        self.dex.move_by_id(id).map(|found| found.name.as_str())
    }

    pub fn ability(&self, id: AbilityId) -> Option<&str> {
        self.dex
            .ability_by_id(id)
            .map(|ability| ability.name.as_str())
    }

    pub fn item(&self, id: ItemId) -> Option<&str> {
        self.dex.item_by_id(id).map(|item| item.name.as_str())
    }

    pub fn species_id(&self, name: &str) -> Option<SpeciesId> {
        self.dex.species_by_name(name).map(|species| species.id)
    }

    pub fn move_id(&self, name: &str) -> Option<MoveId> {
        self.dex.move_by_name(name).map(|found| found.id)
    }

    pub fn ability_id(&self, name: &str) -> Option<AbilityId> {
        self.dex.ability_by_name(name).map(|ability| ability.id)
    }

    pub fn item_id(&self, name: &str) -> Option<ItemId> {
        self.dex.item_by_name(name).map(|item| item.id)
    }
}

impl BattleEngine {
    /// The names of the battle's ids (DESIGN.md §47.0 item 15): the battle's dex shared, never
    /// copied.
    pub fn names(&self) -> Names {
        Names {
            dex: self.shared_dex(),
        }
    }
}
