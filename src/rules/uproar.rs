//! Uproar (Showdown `uproar`, DESIGN.md §15.12): how long it lasts, who it wakes and when, who
//! it keeps awake, and what ends it early, as a generation has it.

use crate::engine::BeforeMoveBlockReason;
use crate::spec::Generation;

/// When an uproar wakes the sleeping Pokémon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UproarWake {
    /// Gen 5+ (Showdown's `onTryHit`): every hit of the move wakes every active Pokémon asleep.
    OnHit,
    /// Gen 3-4 on the cartridge (pokeemerald `ENDTURN_UPROAR` / `CANCELLER_ASLEEP`,
    /// pokeplatinum `MON_COND_CHECK_STATE_UPROAR` / `CHECK_STATUS_STATE_SLEEP`; issues.md I63):
    /// the move itself wakes nobody; the holder's turn-end step wakes every active Pokémon
    /// asleep (in battler order, before the count), and a sleeping Pokémon about to move wakes
    /// while any uproar goes on.
    Cartridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UproarRules {
    /// The turns it lasts, the one it starts included, drawn as it starts: Gen 3 2-5, Gen 4
    /// 3-6; Gen 5+ always 3 (no draw).
    pub turns: (u8, u8),
    pub wake: UproarWake,
    /// Gen 3-4 (pokeemerald `UproarWakeUpCheck`, pokeplatinum `subscript_fall_asleep` /
    /// `subscript_wake_up`): a Soundproof Pokémon is out of it — it may fall asleep and is not
    /// woken.
    pub soundproof_sleeps: bool,
    /// Gen 3 (pokeemerald `AtkCanceller_UnableToUseMove`'s `CancelMultiTurnMoves`): the blocks
    /// that end the uproar at once, silently — Truant, a flinch, love, Disable, Taunt, Imprison.
    /// Gen 4's Truant leaves the uproar alone (`subscript_loafing_around`).
    pub cancel_blocks: &'static [BeforeMoveBlockReason],
    /// Gen 3-4 (`WasUnableToUseMove`, `BattleContext_MoveFailed`): the blocks after which the
    /// holder's turn-end step ends it (after its wake-ups), with a type immunity
    /// (`immunity_fails`).
    pub failing_blocks: &'static [BeforeMoveBlockReason],
    pub immunity_fails: bool,
    /// Gen 3 (pokeemerald `Cmd_attackcanceler`): a protected target ends it at once, silently.
    pub protect_cancels: bool,
    /// Gen 3 (pokeemerald `SetMoveEffect`): a holder put to sleep (a Soundproof one) or frozen
    /// loses it at once.
    pub frozen_or_asleep_cancels: bool,
    /// Gen 3-4 (pokeemerald `BattleScript_EffectYawn`'s `jumpifcantmakeasleep`, pokeplatinum
    /// `subscript_yawn`): Yawn fails while an uproar goes on, unless its target is out of it.
    pub yawn_fails: bool,
    /// Gen 3 (pokeemerald `BattleScript_PrintMoveMissed`'s `ppreduce`, issues.md I65): a
    /// locked turn that misses or meets Protect spends a PP.
    pub miss_spends_pp: bool,
}

const GEN3_CANCELLING: &[BeforeMoveBlockReason] = &[
    BeforeMoveBlockReason::Truant,
    BeforeMoveBlockReason::Flinch,
    BeforeMoveBlockReason::Attract,
    BeforeMoveBlockReason::Disabled,
    BeforeMoveBlockReason::Taunt,
    BeforeMoveBlockReason::Imprison,
];

const GEN3_FAILING: &[BeforeMoveBlockReason] = &[
    BeforeMoveBlockReason::Paralysis,
    BeforeMoveBlockReason::Confusion,
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

impl UproarRules {
    pub const fn for_generation(generation: Generation) -> Self {
        match generation {
            Generation::Gen3 => Self {
                turns: (2, 5),
                wake: UproarWake::Cartridge,
                soundproof_sleeps: true,
                cancel_blocks: GEN3_CANCELLING,
                failing_blocks: GEN3_FAILING,
                immunity_fails: true,
                protect_cancels: true,
                frozen_or_asleep_cancels: true,
                yawn_fails: true,
                miss_spends_pp: true,
            },
            Generation::Gen4 => Self {
                turns: (3, 6),
                wake: UproarWake::Cartridge,
                soundproof_sleeps: true,
                cancel_blocks: &[],
                failing_blocks: GEN4_FAILING,
                immunity_fails: true,
                protect_cancels: false,
                frozen_or_asleep_cancels: false,
                yawn_fails: true,
                miss_spends_pp: false,
            },
            // Gen 1-2 have no Uproar.
            _ => Self {
                turns: (3, 3),
                wake: UproarWake::OnHit,
                soundproof_sleeps: false,
                cancel_blocks: &[],
                failing_blocks: &[],
                immunity_fails: false,
                protect_cancels: false,
                frozen_or_asleep_cancels: false,
                yawn_fails: false,
                miss_spends_pp: false,
            },
        }
    }
}
