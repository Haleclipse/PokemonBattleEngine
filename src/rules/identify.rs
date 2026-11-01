//! Foresight, Odor Sleuth and Miracle Eye (DESIGN.md §41.5, K.7a): what an identified Pokémon's
//! evasion stage still counts for, and whether identifying it again works, as a generation has
//! it.

use crate::spec::Generation;

/// How the accuracy check reads an identified target's evasion stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifiedEvasion {
    /// Gen 2 on the cartridge (pokecrystal `CheckHit`'s `.StatModifiers`; issues.md I87): when
    /// the target's evasion stage is at least the user's accuracy stage, neither stage applies.
    /// Showdown's gen2 accuracy reads the stages as they are.
    NeitherWhenEvasionAtLeastAccuracy,
    /// Gen 3 on the cartridge (pokeemerald `Cmd_accuracycheck`: the user's accuracy stage alone;
    /// issues.md I88): the target's evasion stage does not count, lowered or raised.
    Ignored,
    /// Gen 4+ (pokeplatinum `MON_IS_IDENTIFIED`, Showdown's `onModifyBoost`): a raised evasion
    /// stage counts as 0; a lowered one still counts.
    RaisedIgnored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentifyRules {
    pub evasion: IdentifiedEvasion,
    /// Gen 3-4 on the cartridge (pokeemerald `Cmd_setforesight`, pokeplatinum
    /// `subscript_foresight` / `subscript_miracle_eye`; issues.md I89): the move sets its flag
    /// whatever the target has, so it succeeds on a target already identified by it or by the
    /// other move. Showdown fails it there (`addVolatile`, the moves' `onTryHit`); so do Gen 2
    /// (pokecrystal `BattleCommand_Foresight`) and, by Showdown, Gen 5+.
    pub again_succeeds: bool,
}

impl IdentifyRules {
    pub const fn for_generation(generation: Generation) -> Self {
        match generation {
            Generation::Gen1 | Generation::Gen2 => Self {
                evasion: IdentifiedEvasion::NeitherWhenEvasionAtLeastAccuracy,
                again_succeeds: false,
            },
            Generation::Gen3 => Self {
                evasion: IdentifiedEvasion::Ignored,
                again_succeeds: true,
            },
            Generation::Gen4 => Self {
                evasion: IdentifiedEvasion::RaisedIgnored,
                again_succeeds: true,
            },
            _ => Self {
                evasion: IdentifiedEvasion::RaisedIgnored,
                again_succeeds: false,
            },
        }
    }
}
