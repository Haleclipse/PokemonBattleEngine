//! Clearing entry hazards as a generation does it (DESIGN.md §14.2): Defog.

use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefogRules {
    /// Gen 6+ (the gen7 mod's `onHit`): the user's side loses its entry hazards too, and the
    /// move fails when it neither lowered the target's evasion nor removed a hazard (removing
    /// a screen, Safeguard or Mist does not count). Gen 4-5 (the gen5 mod) clear the target's
    /// side only and never fail.
    pub clears_own_side: bool,
    /// Gen 8+: the terrain ends too (`field.clearTerrain()`).
    pub clears_terrain: bool,
}

impl DefogRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        Self {
            clears_own_side: matches!(
                generation,
                Generation::Gen6 | Generation::Gen7 | Generation::Gen8 | Generation::Gen9
            ),
            clears_terrain: matches!(generation, Generation::Gen8 | Generation::Gen9),
        }
    }
}
