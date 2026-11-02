//! What a generation's faint does to the turn (`FaintRules`).

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaintRules {
    /// Gen 5+: when both sides' last Pokémon faint in the same step, the side whose Pokémon
    /// fainted last wins (Showdown `checkWin`); before that it is a draw.
    pub last_to_faint_wins_double_ko: bool,
    /// Gen 1-3: a Pokémon knocked out by a move is replaced right after that move, before the
    /// rest of the turn and its residual (Showdown `checkFainted` when the next queued action
    /// is a move or the residual); from Gen 4 at the end of the turn.
    pub replace_after_each_move: bool,
    /// What a faint does to the rest of a singles turn (Showdown `faintMessages`).
    pub singles_faint_effect: FaintTurnEffect,
    /// Gen 1: at a turn's end, a battle that cannot progress is a tie (DESIGN.md §45.7). This is
    /// the Gen 1 check of Showdown `maybeTriggerEndlessBattleClause`, and no format turns it off.
    /// It holds when every Pokémon on each side has fainted, is frozen, knows only Transform
    /// against a foe team that does too, or has no PP left against a team of Ghosts.
    pub no_progress_ties: bool,
}

/// What a faint does to the rest of a singles turn (Showdown `faintMessages`, DESIGN.md
/// §31.2). Doubles are never affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaintTurnEffect {
    /// Gen 4+: the remaining actions run.
    None,
    /// Gen 3: every remaining action — moves and switches — is cancelled; the residual runs.
    CancelActions,
    /// Gen 2: every remaining move is cancelled; switches still happen, then the residual.
    CancelMoves,
    /// Gen 1: the rest of the turn is skipped.
    SkipTurn,
}

/// What the generation's faint does to the turn.
pub(super) fn resolve(generation: Generation) -> FaintRules {
    FaintRules {
        last_to_faint_wins_double_ko: generation >= Generation::Gen5,
        replace_after_each_move: generation <= Generation::Gen3,
        singles_faint_effect: match generation {
            Generation::Gen1 => FaintTurnEffect::SkipTurn,
            Generation::Gen2 => FaintTurnEffect::CancelMoves,
            Generation::Gen3 => FaintTurnEffect::CancelActions,
            _ => FaintTurnEffect::None,
        },
        no_progress_ties: generation == Generation::Gen1,
    }
}
