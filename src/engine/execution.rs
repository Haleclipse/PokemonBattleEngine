use crate::ids::{PokemonId, SideId};
use crate::rules::ActionOrderStrategy;
use crate::state::{BattleOutcome, BattlePhase, BattleState};
use serde::{Deserialize, Serialize};

use super::{ForcedSwitchChoice, QueuedAction, ReplacementRequirement, SideChoice};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum BattleRequest {
    TurnChoices {
        sides: [SideId; 2],
    },
    ForcedSwitch {
        replacements: [ReplacementRequirement; 2],
    },
    /// Revival Blessing (DESIGN.md §30.5): `side` names one of its fainted Pokémon
    /// (`candidates`, in party order) to bring back at half HP (`BattleEngine::resolve_revive`).
    /// Asked mid-turn, right after the move; the other side waits.
    Revive {
        side: SideId,
        candidates: Vec<PokemonId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineStep {
    Progressed { phase: BattlePhase },
    Awaiting(BattleRequest),
    Finished(BattleOutcome),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PendingExecution {
    #[default]
    Idle,
    /// The battle's start, a lead ordered out by the leads' switch-in (Eject Pack): its
    /// replacement comes before the first turn's choices (`BattleEngine::run_lead_switch_ins`).
    Start,
    Turn {
        actions: Vec<QueuedAction>,
        strategy: ActionOrderStrategy,
        /// False once a Gen 1 faint cut the turn short (`FaintTurnEffect::SkipTurn`): the
        /// turn ends with no end-of-turn processing at all.
        residual: bool,
        /// True once an action of the turn has run: from then on a dynamic re-sort
        /// (`RecalculateAfterAction`) reshuffles every speed tie among the pending actions.
        resort_ties: bool,
    },
}

/// A moment of a battle: its state, what it was in the middle of, and the fingerprints of the
/// rules and dex it ran under. Not its trace — the trace is the battle's output, not its state
/// (DESIGN.md §47.0 item 6): a restored battle starts an empty one. Opaque in the stable API
/// (§47.5 item 6): its state, hidden information and all, is the internals' to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleSnapshot {
    #[cfg(feature = "internals")]
    pub state: BattleState,
    #[cfg(not(feature = "internals"))]
    pub(crate) state: BattleState,
    pub(crate) pending: PendingExecution,
    #[cfg(feature = "internals")]
    pub rules_fingerprint: String,
    #[cfg(not(feature = "internals"))]
    pub(crate) rules_fingerprint: String,
    #[cfg(feature = "internals")]
    pub dex_fingerprint: String,
    #[cfg(not(feature = "internals"))]
    pub(crate) dex_fingerprint: String,
}

impl BattleSnapshot {
    pub fn phase(&self) -> BattlePhase {
        self.state.phase
    }
}

// Keep these types referenced by the public execution module so future request variants can add
// payloads without creating a second choice model.
#[allow(dead_code)]
fn _choice_type_anchor(_: SideChoice, _: ForcedSwitchChoice) {}
