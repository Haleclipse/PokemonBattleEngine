//! Lock-On and Mind Reader (DESIGN.md §15.14): where the lock is kept, how long it lasts, what it
//! reaches, and what Lock-On does when it is on already, as a generation has it.

use crate::spec::Generation;

/// Where the lock lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockOnModel {
    /// Gen 3+ (Showdown `lockon` on its user, `duration: 2`): the user's next moves against
    /// the target this turn and the next skip their accuracy check.
    OnUser,
    /// Gen 2 on the cartridge (pokecrystal `BattleCommand_LockOn` / `CheckHit`'s `.LockOn`;
    /// issues.md I69): a flag on the target, with no time limit, that the next accuracy check
    /// against it reads and clears. It stays when the user switches out; the target's Baton
    /// Pass hands it on.
    OnTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockOnRules {
    pub model: LockOnModel,
    /// Gen 3-4 on the cartridge (pokeemerald `Cmd_setalwayshitflag`, pokeplatinum
    /// `subscript_lock_on`; issues.md I68): Lock-On again succeeds and sets the lock anew (2
    /// turns, the new target); Showdown fails it while the user holds one.
    pub again_refreshes: bool,
    /// Gen 3-4 on the cartridge (`SwitchInClearSetData`, `BattleSystem_UpdateAfterSwitch`;
    /// issues.md I68): the user's Baton Pass hands the lock on with its 2 turns afresh, and the
    /// target's keeps it on the replacement.
    pub baton_pass_follows: bool,
}

impl LockOnRules {
    pub const fn for_generation(generation: Generation) -> Self {
        match generation {
            Generation::Gen1 | Generation::Gen2 => Self {
                model: LockOnModel::OnTarget,
                again_refreshes: true,
                baton_pass_follows: false,
            },
            Generation::Gen3 | Generation::Gen4 => Self {
                model: LockOnModel::OnUser,
                again_refreshes: true,
                baton_pass_follows: true,
            },
            _ => Self {
                model: LockOnModel::OnUser,
                again_refreshes: false,
                baton_pass_follows: false,
            },
        }
    }
}
