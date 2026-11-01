//! Volatiles that act as their holder faints (DESIGN.md §15.9): Destiny Bond and Grudge, as a
//! generation has them.

use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaintBondRules {
    /// Gen 7+ Destiny Bond (`onPrepareHit: return !removeVolatile('destinybond')`): used again
    /// while its bond is up, it fails and the bond goes; the gen6 mod (Gen 2-6) renews it.
    pub destiny_bond_fails_in_a_row: bool,
}

impl FaintBondRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        Self {
            destiny_bond_fails_in_a_row: matches!(
                generation,
                Generation::Gen7 | Generation::Gen8 | Generation::Gen9
            ),
        }
    }
}
