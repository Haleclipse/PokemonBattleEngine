//! PP taken by something else than a move's own use (DESIGN.md §15.15): Spite, Eerie Spell,
//! Pressure, as a generation has them.

use crate::ids::MoveId;
use crate::spec::Generation;

/// How much PP Spite takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiteAmount {
    /// Gen 4+ (Showdown `deductPP(move, 4)`; pokeplatinum `BtlCmd_TrySpite`).
    Fixed(u8),
    /// Gen 2-3: a draw of `min..=max` (Showdown `random(2, 6)`; pokecrystal `BattleRandom & 3 + 2`,
    /// pokeemerald `(Random() & 3) + 2`). On the cartridge the draw comes after the checks, so a
    /// Spite that fails draws nothing (issues.md I71).
    Rolled { min: u8, max: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiteRules {
    pub amount: SpiteAmount,
    /// The least PP the target's last move must have left: Gen 3 on the cartridge 2
    /// (pokeemerald `Cmd_tryspiteppreduce`: `pp[i] > 1`; issues.md I71), elsewhere 1.
    pub min_pp: u8,
    /// Gen 3 on the cartridge (`Cmd_tryspiteppreduce`'s `CancelMultiTurnMoves` once the move is
    /// out of PP; issues.md I71): a move Spite empties ends the target's multi-turn move — a
    /// two-turn move's charge, Thrash's lock (without its confusion), Uproar, Bide, Rollout.
    pub emptied_cancels_multi_turn: bool,
}

/// Whose Pressure a move meets: each such holder makes it cost one more PP
/// (`engine::pressure_holders`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureReach {
    /// Gen 5+ (Showdown `getMoveTargets`' `pressureTargets`, `onDeductPP` sparing the user's
    /// side): the foes among the move's targets after redirection — a field move's foes, an
    /// own-side or foe-side move's nobody — and every foe for a `mustpressure` move.
    Targets,
    /// Gen 4 on the cartridge (pokeplatinum `BattleControllerPlayer_DecrementPP`; issues.md
    /// I72): by the move's range — a field or all-adjacent move every other Pokémon, a foes' or
    /// foe-side move and Imprison the foes; a move for the user's side, the user or an ally,
    /// and Counter / Mirror Coat / Metal Burst (aimed at the user as they start) nobody;
    /// otherwise its target. Allies count.
    Gen4,
    /// Gen 3 on the cartridge (pokeemerald `Cmd_ppreduce`, `PressurePPLoseOnUsingImprison`,
    /// `PressurePPLoseOnUsingPerishSong`; I72): a move aimed at the user — field and own-side
    /// moves too — nobody, but Imprison the foes and Perish Song every other Pokémon; a foes'
    /// or foe-side move the foes; an all-adjacent move every other Pokémon; otherwise its
    /// target. Allies count.
    Gen3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpRules {
    pub spite: SpiteRules,
    pub pressure: PressureReach,
    /// The slot a move's own use takes its PP from, and Mimic and Sketch write.
    pub moves_slot: MovesSlot,
    /// Who pays the Pressure a move another move called meets (DESIGN.md R-M8-59, R-M8-67).
    pub called_move_pressure: CalledMovePressure,
    /// Gen 1 (the gen1 `runSwitch`): a Pokémon coming in puts its side's last selected slot back
    /// to the first (`lastSelectedMoveSlot = 0`); the last selected move stays (R-M8-150).
    pub switch_in_resets_last_selected_slot: bool,
    /// Gen 1 (Showdown's Desync Clause Mod, the gen1 `runMove`): an action uses the side's last
    /// selected move, and when the slot it pays its PP from holds another — a frozen Pokémon
    /// that came in after a teammate's pick, thawed by a Fire move — it ends there, its PP spent,
    /// with no `move` line (R-M8-150).
    pub desync_clause: bool,
}

/// Which slot is the move's: the one its PP comes off, the one Mimic and Sketch write. It
/// matters when a Pokémon has the same move twice — a Sketch that copied Mimic while Mimic's own
/// copy hid it (issues.md I170, DESIGN.md R-M8-76).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovesSlot {
    /// Gen 1 (the gen1 `deductPP`, Mimic's `source.side.lastSelectedMoveSlot`): the slot the
    /// side last picked from the menu, whatever the action names — a charged move a Metronome
    /// called pays from the Metronome's slot as it goes out (DESIGN.md R-M8-54); a called
    /// Mimic overwrites its caller.
    TheLastSelected,
    /// Gen 2-3 on the cartridge (pokecrystal `BattleCommand_DoTurn`'s `wCurMoveNum`; pokeemerald
    /// `gCurrMovePos` in `Cmd_ppreduce`, `Cmd_mimicattackcopy`, `Cmd_copymovepermanently`): the
    /// slot the action chose, else (a called Mimic) the first holding the move. Showdown's gen2-3
    /// take the first (the oracle patched, I170). Gen 2's Mimic writes its last Mimic instead,
    /// but there no Pokémon has a move twice: Sketch fails, Mimic refuses a move it knows.
    TheChosen,
    /// Gen 4+ (pokeplatinum `Battler_SlotForMove`, `BtlCmd_TryMimic`, `BtlCmd_TrySketch`;
    /// Showdown `getMoveData`, `source.moves.indexOf`): the first slot holding the move, the
    /// slot the action chose or not (the user's decision for Gen 5+, I170).
    TheFirstHoldingIt,
}

/// Whose PP the Pressure a called move meets comes off: the caller is a move that calls one
/// (`callsMove`: Metronome, Assist, Sleep Talk …); a reflection or a snatch — a condition's
/// call — charges nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalledMovePressure {
    /// Gen 3 on the cartridge (pokeemerald `Cmd_ppreduce`; issues.md I167): Metronome, Assist,
    /// Mirror Move and Nature Power leave their PP to the called move's script, whose `ppreduce`
    /// takes it, with one more for each Pressure the called move meets, from the action's slot
    /// (`gCurrMovePos`) — Metronome's when it called Nature Power. `spared_after` (Sleep Talk)
    /// pays its own first and turns the called move's off (`HITMARKER_NO_PPDEDUCT`). Showdown's
    /// gen3 `useMoveInner` charges nothing for a called move.
    TheActionsMove { spared_after: MoveId },
    /// Gen 4+ (Showdown `useMoveInner`'s `callerMoveForPressure`): the caller that called it,
    /// Sleep Talk too — nobody when that caller is not in the slots (a Nature Power Metronome
    /// called).
    TheCaller,
}

impl PpRules {
    pub const fn for_generation(generation: Generation) -> Self {
        let spite = match generation {
            Generation::Gen1 | Generation::Gen2 => SpiteRules {
                amount: SpiteAmount::Rolled { min: 2, max: 5 },
                min_pp: 1,
                emptied_cancels_multi_turn: false,
            },
            Generation::Gen3 => SpiteRules {
                amount: SpiteAmount::Rolled { min: 2, max: 5 },
                min_pp: 2,
                emptied_cancels_multi_turn: true,
            },
            _ => SpiteRules {
                amount: SpiteAmount::Fixed(4),
                min_pp: 1,
                emptied_cancels_multi_turn: false,
            },
        };
        let pressure = match generation {
            Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => PressureReach::Gen3,
            Generation::Gen4 => PressureReach::Gen4,
            _ => PressureReach::Targets,
        };
        Self {
            spite,
            pressure,
            moves_slot: match generation {
                Generation::Gen1 => MovesSlot::TheLastSelected,
                Generation::Gen2 | Generation::Gen3 => MovesSlot::TheChosen,
                _ => MovesSlot::TheFirstHoldingIt,
            },
            switch_in_resets_last_selected_slot: matches!(generation, Generation::Gen1),
            desync_clause: matches!(generation, Generation::Gen1),
            called_move_pressure: match generation {
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => {
                    CalledMovePressure::TheActionsMove {
                        spared_after: MoveId::SLEEP_TALK,
                    }
                }
                _ => CalledMovePressure::TheCaller,
            },
        }
    }
}
