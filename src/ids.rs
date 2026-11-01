use core::fmt;

/// A dense table from an id's number to the row of the table holding it (DESIGN.md §47): a
/// lookup is one index, not a search — the behavior registry's abilities and items (R-M10-7),
/// the dex's moves, abilities and items (R-M10-12). An id two rows hold is the first one's, as
/// a search finds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DenseRows(Vec<u16>);

impl DenseRows {
    /// No row holds this id.
    const NONE: u16 = u16::MAX;

    /// The table of `ids`, in row order.
    pub(crate) fn new(ids: impl Iterator<Item = u16>) -> Self {
        let mut rows = Vec::new();
        for (row, id) in ids.enumerate() {
            let id = usize::from(id);
            if rows.len() <= id {
                rows.resize(id + 1, Self::NONE);
            }
            if rows[id] == Self::NONE {
                rows[id] = u16::try_from(row)
                    .ok()
                    .filter(|row| *row != Self::NONE)
                    .expect("fewer rows than u16::MAX");
            }
        }
        Self(rows)
    }

    /// The row holding `id`, if any.
    pub(crate) fn get(&self, id: u16) -> Option<usize> {
        self.0
            .get(usize::from(id))
            .copied()
            .filter(|row| *row != Self::NONE)
            .map(usize::from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PokemonId(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SideId {
    P1,
    P2,
}

impl SideId {
    pub const fn other(self) -> Self {
        match self {
            Self::P1 => Self::P2,
            Self::P2 => Self::P1,
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::P1 => 0,
            Self::P2 => 1,
        }
    }

    pub const fn ordinal(self) -> u8 {
        self.index() as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PositionId(u8);

impl PositionId {
    pub(crate) const fn new_unchecked(value: u8) -> Self {
        Self(value)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MoveSlot(u8);

impl MoveSlot {
    pub const FIRST: Self = Self(0);
    pub const SECOND: Self = Self(1);
    pub const THIRD: Self = Self(2);
    pub const FOURTH: Self = Self(3);

    pub const fn new(index: u8) -> Option<Self> {
        if index < 4 { Some(Self(index)) } else { None }
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MoveId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbilityId(pub u16);

impl AbilityId {
    /// No ability (a Pokémon built by hand, Gen 1-2).
    pub const NONE: Self = Self(0);
}

/// Items are generation-ordered (`gen * 1000 + n`, data/dex/README.md): Choice Band is the 8th
/// Gen 3 item alphabetically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(pub u16);

/// National dex number plus forme index (DESIGN.md §27.1). The base forme is 0; other formes are
/// numbered once, in order of first appearance, and never renumbered. Formes of one species
/// share `dex`, which is what species clauses and in-battle forme changes compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpeciesId {
    pub dex: u16,
    pub form: u8,
}

impl SpeciesId {
    pub const fn base(dex: u16) -> Self {
        Self { dex, form: 0 }
    }

    pub const fn is_base(self) -> bool {
        self.form == 0
    }

    pub const fn same_species(self, other: Self) -> bool {
        self.dex == other.dex
    }
}

impl fmt::Display for SpeciesId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.dex, self.form)
    }
}

// `MoveId`, `AbilityId` and `ItemId` constants for every move, ability and item in the id
// registry (build.rs, from data/dex/ids.json).
include!(concat!(env!("OUT_DIR"), "/ids.rs"));

#[cfg(test)]
mod tests {
    use super::DenseRows;

    /// The row of each id, the first of an id two rows hold, and none past the table or for an
    /// id no row holds.
    #[test]
    fn a_dense_table_finds_the_first_row_of_an_id() {
        let rows = DenseRows::new([5, 2, 5, 0].into_iter());
        assert_eq!(rows.get(5), Some(0));
        assert_eq!(rows.get(2), Some(1));
        assert_eq!(rows.get(0), Some(3));
        assert_eq!(rows.get(1), None);
        assert_eq!(rows.get(6), None);
        assert_eq!(rows.get(u16::MAX), None);
        assert_eq!(DenseRows::new(std::iter::empty()).get(0), None);
    }
}
