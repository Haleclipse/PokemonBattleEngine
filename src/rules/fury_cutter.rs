//! Fury Cutter (DESIGN.md §15.18): how a generation keeps its chain going.

use crate::engine::BeforeMoveBlockReason;
use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuryCutterModel {
    /// Gen 5+ (Showdown `furycutter`): a volatile of 2 turns, started or renewed by every use that
    /// reaches the damage calculation — its multiplier doubled up to `cap` — the power `base ×
    /// multiplier`, at most 160.
    Volatile { cap: u8 },
    /// Gen 2-4 on the cartridge (pokecrystal `BattleCommand_FuryCutter`, pokeemerald
    /// `Cmd_furycuttercalc`, pokeplatinum `BtlCmd_CalcFuryCutterPower`; issues.md I75): a count
    /// of the uses that landed (a Substitute's too), with no time limit, doubling `min(count, 5)
    /// − 1` times; a use that misses or does nothing ends it (`FuryCutterRules` lists the
    /// rest).
    Count {
        /// Gen 2: the doubling is the damage's, after STAB and the type matchups and before the
        /// random factor, not the power's.
        damage_stage: bool,
        /// Gen 2: a use of another move ends it (the cartridge's move choice); Gen 3-4 keep it.
        another_move_ends_it: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuryCutterRules {
    pub model: FuryCutterModel,
    /// The BeforeMove blocks that end the chain, whatever the move: Gen 3 `CancelMultiTurnMoves`
    /// in `AtkCanceller_UnableToUseMove` (Truant's loafing, recharging, a flinch, Disable, Taunt,
    /// Imprison, love — not sleep, freeze, paralysis or confusion); Gen 4 the subscripts that
    /// `UnlockMoveChoice` (Truant's loafing, a flinch, full paralysis, a confusion self-hit,
    /// love, Taunt, Disable, Imprison, Gravity, recharging).
    pub ending_blocks: &'static [BeforeMoveBlockReason],
    /// Gen 3-4: sleep or freeze put on the user ends it (pokeemerald `SetMoveEffect`,
    /// pokeplatinum `subscript_fall_asleep` / `subscript_freeze`).
    pub sleep_or_freeze_ends_it: bool,
}

const GEN3_ENDING: &[BeforeMoveBlockReason] = &[
    BeforeMoveBlockReason::Truant,
    BeforeMoveBlockReason::Recharge,
    BeforeMoveBlockReason::Flinch,
    BeforeMoveBlockReason::Disabled,
    BeforeMoveBlockReason::Taunt,
    BeforeMoveBlockReason::Imprison,
    BeforeMoveBlockReason::Attract,
];

const GEN4_ENDING: &[BeforeMoveBlockReason] = &[
    BeforeMoveBlockReason::Truant,
    BeforeMoveBlockReason::Flinch,
    BeforeMoveBlockReason::Paralysis,
    BeforeMoveBlockReason::Confusion,
    BeforeMoveBlockReason::Attract,
    BeforeMoveBlockReason::Taunt,
    BeforeMoveBlockReason::Disabled,
    BeforeMoveBlockReason::Imprison,
    BeforeMoveBlockReason::Gravity,
    BeforeMoveBlockReason::Recharge,
];

impl FuryCutterRules {
    pub const fn for_generation(generation: Generation) -> Self {
        match generation {
            Generation::Gen1 | Generation::Gen2 => Self {
                model: FuryCutterModel::Count {
                    damage_stage: true,
                    another_move_ends_it: true,
                },
                ending_blocks: &[],
                sleep_or_freeze_ends_it: false,
            },
            Generation::Gen3 => Self {
                model: FuryCutterModel::Count {
                    damage_stage: false,
                    another_move_ends_it: false,
                },
                ending_blocks: GEN3_ENDING,
                sleep_or_freeze_ends_it: true,
            },
            Generation::Gen4 => Self {
                model: FuryCutterModel::Count {
                    damage_stage: false,
                    another_move_ends_it: false,
                },
                ending_blocks: GEN4_ENDING,
                sleep_or_freeze_ends_it: true,
            },
            Generation::Gen5 => Self {
                model: FuryCutterModel::Volatile { cap: 8 },
                ending_blocks: &[],
                sleep_or_freeze_ends_it: false,
            },
            _ => Self {
                model: FuryCutterModel::Volatile { cap: 4 },
                ending_blocks: &[],
                sleep_or_freeze_ends_it: false,
            },
        }
    }
}
