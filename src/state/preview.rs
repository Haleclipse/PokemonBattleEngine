//! What a team preview showed before the battle (DESIGN.md §47.0 item 14): each side's members,
//! picked or not, as everyone saw them — the species (a hidden forme as its base species), the
//! level, the gender, the item mark, the Tera type, an open team sheet — and as their side knows
//! them. Set once as the battle starts (`BattleEngine::show_team_preview`), never changed after;
//! the views read it (`SideView::previewed`).

use crate::dex::PokemonType;
use crate::ids::{AbilityId, ItemId, MoveId, PokemonId, SpeciesId};

use super::Gender;

/// The members a team preview showed, by side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamPreview {
    /// P1's and P2's members, in the order registered.
    pub sides: [Vec<PreviewedMember>; 2],
}

/// One member a team preview showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewedMember {
    /// What everyone was shown of it.
    pub shown: MemberSheet,
    /// The member as its side knows it: everything.
    pub known: MemberSheet,
    /// The battle's Pokémon it is, when its side brought it: what an open team sheet showed of
    /// it is known of that Pokémon from the start.
    pub pokemon: Option<PokemonId>,
}

/// What is known of a previewed member: `None` for what is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberSheet {
    /// Its species: the base species where the preview hid the forme (`forme_hidden`).
    pub species: SpeciesId,
    /// The preview showed the species with its forme hidden (`Urshifu-*`).
    pub forme_hidden: bool,
    pub level: u8,
    pub gender: Gender,
    /// Its item: `Some(None)` for none (a Gen 4-7 preview marks the members holding one).
    pub item: Option<Option<ItemId>>,
    pub ability: Option<AbilityId>,
    /// All its moves, in order.
    pub moves: Option<Vec<MoveId>>,
    /// Its Tera type: `Some(None)` where it has none (before Gen 9).
    pub tera_type: Option<Option<PokemonType>>,
}

impl MemberSheet {
    /// Whether an open team sheet showed it: its moves are known.
    pub fn from_sheet(&self) -> bool {
        self.moves.is_some()
    }
}
