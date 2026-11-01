//! Curse (DESIGN.md §15.16): how its two forms differ by generation.

use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurseRules {
    /// Gen 2 on the cartridge (pokecrystal `BattleCommand_Curse`; issues.md I73): a user that is
    /// not a Ghost type fails when its Attack and Defense are both at +6, and its Speed stays.
    /// Elsewhere each of the three stages changes on its own.
    pub fails_when_attack_and_defense_maxed: bool,
    /// Gen 9 (Showdown `onModifyMove`): a Ghost type's Curse aimed at an ally goes to a random
    /// adjacent foe instead.
    pub ghost_ally_target_goes_to_a_foe: bool,
}

impl CurseRules {
    pub const fn for_generation(generation: Generation) -> Self {
        Self {
            fails_when_attack_and_defense_maxed: matches!(
                generation,
                Generation::Gen1 | Generation::Gen2
            ),
            ghost_ally_target_goes_to_a_foe: matches!(generation, Generation::Gen9),
        }
    }
}
