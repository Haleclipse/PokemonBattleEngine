//! Thrash, Petal Dance, Outrage and Raging Fury (Showdown `lockedmove`, DESIGN.md §15.12), as a
//! generation counts the lock and ends it.

use crate::engine::BeforeMoveBlockReason;
use crate::spec::Generation;

/// How the lock is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RampageCounter {
    /// Gen 1 (pokered `ThrashPetalDanceEffect` / `.ThrashingAboutCheck`, Showdown's gen1 mod):
    /// as the move first starts (before its hit test) 2-3 uses left (`time`); each later use
    /// counts one down as it starts, and the one that reaches 0 confuses its user (a confusion
    /// already there rolled anew) before it strikes; the lock ends once that move is over. A
    /// full paralysis or a confusion self-hit ends it without confusion; sleep, freeze, a
    /// flinch keep it. No end-of-turn count.
    Gen1Uses,
    /// Gen 2 on the cartridge (pokecrystal `BattleCommand_CheckRampage` /
    /// `BattleCommand_Rampage`, `CantMove`; issues.md I108, I109): as the first use starts —
    /// before its hit test — 1-2 more uses (the drawn 2-3 less one); each later use counts one
    /// down as it starts, and the one that reaches 0 confuses its user for 2-3 turns (a confusion
    /// already there rolled anew; Safeguard on its side spares it) before it strikes; the lock
    /// ends once that move is over. Every BeforeMove stop ends it silently — sleep and freeze
    /// too, at the user's next move. No end-of-turn count.
    Gen2Uses,
    /// Gen 3-4 on the cartridge (pokeemerald `MOVE_EFFECT_THRASH`, `ENDTURN_THRASH`;
    /// pokeplatinum `subscript_thrash`, `MON_COND_CHECK_STATE_THRASH`; issues.md I61): the
    /// first hit sets 2-3 turns, counted down at the end of every turn (the residual's order 10,
    /// sub-order 12); a later hit leaves the count alone, a miss does not stop it. At 0 the lock
    /// ends and confuses its user unless it is confused already. A turn the move failed
    /// (`failing_blocks`, a type immunity) ends it silently at the end of the turn.
    Cartridge,
    /// Gen 5+ (Showdown `lockedmove`): a hidden length of 2-3 turns (`trueDuration`) and a
    /// visible `duration` 2 that every hit puts back to 2 while the length allows; the lock
    /// ends with the move once `duration` reads 1 (or at the end of the turn, when the move was
    /// stopped before it ran), confusing its user on its last turn.
    HiddenLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RampageRules {
    pub counter: RampageCounter,
    /// Gen 3-4: the BeforeMove blocks that count as a failed move (pokeemerald
    /// `WasUnableToUseMove`: full paralysis, a flinch, a confusion self-hit, love, Disable,
    /// Taunt, Imprison; pokeplatinum `BattleContext_MoveFailed` adds Gravity). Sleep, freeze
    /// and recharging are not among them. Gen 3's Truant ends the lock at once
    /// (`CANCELER_TRUANT`'s `CancelMultiTurnMoves`), which no frame tells from a failed turn;
    /// Gen 4's keeps it going (`subscript_loafing_around` unlocks no Thrash).
    pub failing_blocks: &'static [BeforeMoveBlockReason],
    /// Gen 3 (pokeemerald `Cmd_attackcanceler`: `CancelMultiTurnMoves` on a protected target):
    /// Protect ends the lock at once, silently. Gen 4 keeps it going.
    pub protect_cancels: bool,
    /// Gen 3 (pokeemerald: a sleep or freeze inflicted `CancelMultiTurnMoves`): an asleep or
    /// frozen user loses the lock at the end of the turn without confusion. Gen 4-9: only
    /// sleep, and only on a turn the lock does not end (pokeplatinum `SetupNextTurn`; Showdown
    /// `onResidual`).
    pub frozen_or_asleep_cancels: bool,
    /// Gen 3 (pokeemerald `BattleScript_PrintMoveMissed`'s `ppreduce`, issues.md I65): a
    /// locked turn that misses or meets Protect spends a PP.
    pub miss_spends_pp: bool,
}

const GEN3_FAILING: &[BeforeMoveBlockReason] = &[
    BeforeMoveBlockReason::Paralysis,
    BeforeMoveBlockReason::Flinch,
    BeforeMoveBlockReason::Confusion,
    BeforeMoveBlockReason::Attract,
    BeforeMoveBlockReason::Disabled,
    BeforeMoveBlockReason::Taunt,
    BeforeMoveBlockReason::Imprison,
    BeforeMoveBlockReason::Truant,
];

const GEN4_FAILING: &[BeforeMoveBlockReason] = &[
    BeforeMoveBlockReason::Paralysis,
    BeforeMoveBlockReason::Flinch,
    BeforeMoveBlockReason::Confusion,
    BeforeMoveBlockReason::Attract,
    BeforeMoveBlockReason::Disabled,
    BeforeMoveBlockReason::Taunt,
    BeforeMoveBlockReason::Imprison,
    BeforeMoveBlockReason::Gravity,
];

impl RampageCounter {
    /// Gen 1-2: the lock is counted in uses, from its first use's start (before the hit test).
    pub const fn counts_uses(self) -> bool {
        matches!(self, Self::Gen1Uses | Self::Gen2Uses)
    }
}

impl RampageRules {
    pub const fn for_generation(generation: Generation) -> Self {
        match generation {
            Generation::Gen1 => Self {
                counter: RampageCounter::Gen1Uses,
                failing_blocks: &[],
                protect_cancels: false,
                frozen_or_asleep_cancels: false,
                miss_spends_pp: false,
            },
            Generation::Gen2 => Self {
                counter: RampageCounter::Gen2Uses,
                failing_blocks: &[],
                protect_cancels: false,
                frozen_or_asleep_cancels: false,
                miss_spends_pp: false,
            },
            Generation::Gen3 => Self {
                counter: RampageCounter::Cartridge,
                failing_blocks: GEN3_FAILING,
                protect_cancels: true,
                frozen_or_asleep_cancels: true,
                miss_spends_pp: true,
            },
            Generation::Gen4 => Self {
                counter: RampageCounter::Cartridge,
                failing_blocks: GEN4_FAILING,
                protect_cancels: false,
                frozen_or_asleep_cancels: false,
                miss_spends_pp: false,
            },
            _ => Self {
                counter: RampageCounter::HiddenLength,
                failing_blocks: &[],
                protect_cancels: false,
                frozen_or_asleep_cancels: false,
                miss_spends_pp: false,
            },
        }
    }
}
