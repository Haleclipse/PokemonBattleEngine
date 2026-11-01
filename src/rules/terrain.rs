//! The terrains as a generation has them (DESIGN.md §23.2): how long one lasts and how much it
//! strengthens or weakens a move.

use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainRules {
    /// Showdown `duration` (5) and `durationCallback`: 8 when the setter holds Terrain Extender.
    pub turns: u8,
    pub extender_turns: u8,
    /// The `onBasePower` boost a grounded user's Electric / Grass / Psychic move gets, 4096-based:
    /// Gen 6-7 `chainModify(1.5)` = 6144, Gen 8+ [5325, 4096].
    pub boost: u32,
    /// Misty Terrain keeps confusion off a grounded Pokémon (`onTryAddVolatile`) from Gen 7;
    /// the gen6 mod drops the handler.
    pub misty_blocks_confusion: bool,
}

impl TerrainRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        Self {
            turns: 5,
            extender_turns: 8,
            boost: match generation {
                Generation::Gen8 | Generation::Gen9 => 5325,
                _ => 6144,
            },
            misty_blocks_confusion: matches!(
                generation,
                Generation::Gen7 | Generation::Gen8 | Generation::Gen9
            ),
        }
    }
}
