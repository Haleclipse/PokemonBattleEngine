use crate::ids::PokemonId;
use serde::{Deserialize, Serialize};

use super::ActionChoice;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOrder {
    pub action_priority: i16,
    pub move_priority: i8,
    /// Showdown's fractional priority in tenths of a step (DESIGN.md §12.4): Stall and the
    /// delaying items below 0, a Quick Claw or Quick Draw that went off above.
    pub fractional_priority: i8,
    pub speed: u32,
    /// Gen 4: a move delayed by a negative fractional priority orders slowest first, Trick
    /// Room or not (`ActionOrderRules::delayed_slowest_first`).
    pub slowest_first: bool,
    /// Gen 4: a move a Quick Claw or Custap Berry put first orders fastest first, Trick Room or
    /// not (`ActionOrderRules::boosted_fastest_first`, issues.md I44).
    pub fastest_first: bool,
    pub tie_break: u64,
    /// Not an ordering key: Prankster raised the move's priority (Showdown
    /// `pranksterBoosted`), carried to the move (DESIGN.md §12.5).
    pub prankster_boosted: bool,
}

impl ActionOrder {
    /// What decides the order before the tie break: two actions with equal keys are a genuine
    /// speed tie.
    pub fn key(&self) -> (i16, i8, i8, u32) {
        (
            self.action_priority,
            self.move_priority,
            self.fractional_priority,
            self.speed,
        )
    }
}

/// The queue's comparison (`sort_actions`' own): `Less` means `left` runs first. The keys
/// count *down* — a higher action priority, move priority or speed acts earlier — except in
/// Trick Room (and a Gen 4 delayed move) where speed counts up.
pub(crate) fn compare_orders(
    left: &ActionOrder,
    right: &ActionOrder,
    trick_room: bool,
) -> std::cmp::Ordering {
    right
        .action_priority
        .cmp(&left.action_priority)
        .then_with(|| right.move_priority.cmp(&left.move_priority))
        .then_with(|| right.fractional_priority.cmp(&left.fractional_priority))
        .then_with(|| {
            // A bracket of delayed (or put-first) Gen 4 moves holds nothing else, so either
            // side's flag speaks for both.
            if left.fastest_first {
                right.speed.cmp(&left.speed)
            } else if trick_room || left.slowest_first {
                left.speed.cmp(&right.speed)
            } else {
                right.speed.cmp(&left.speed)
            }
        })
        .then_with(|| right.tie_break.cmp(&left.tie_break))
}

/// Whether `left`'s action runs before `right`'s (Showdown's `comparePriority` > 0, for
/// `insertChoice`'s place among tied actions).
pub(crate) fn order_before(left: &ActionOrder, right: &ActionOrder, trick_room: bool) -> bool {
    compare_orders(left, right, trick_room) == std::cmp::Ordering::Less
}

/// A chance to act first within the priority bracket (DESIGN.md §12.4), one
/// `PriorityChance` decision each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PriorityChance {
    /// Gen 2-3 Showdown `quickClawRoll`: one roll as the turn starts (60/256, 1/5); when it
    /// succeeds every Quick Claw holder acts at Speed 65535.
    QuickClawTurn,
    /// Gen 4+ Quick Claw: 1 in 5 per move action, rolled only while nothing raised the
    /// action's fractional priority.
    QuickClaw,
    /// Quick Draw: 3 in 10 per damaging move action, rolled before Quick Claw.
    QuickDraw,
}

impl PriorityChance {
    /// The per-action odds (the Gen 2-3 turn roll's are the generation's,
    /// `ActionOrderRules::quick_claw_turn_roll`).
    pub const fn odds(self) -> (u16, u16) {
        match self {
            Self::QuickClawTurn | Self::QuickClaw => (1, 5),
            Self::QuickDraw => (3, 10),
        }
    }
}

/// What a queue entry does (DESIGN.md §44.2): Showdown `resolveAction` splits a move choice with
/// a generation gimmick into the gimmick's own action (order 104-106) and the move (200). The
/// gimmick happens before every move of the turn, whatever the speeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionStep {
    /// The move itself (a switch and a pass are always this).
    Act,
    /// Mega Evolution (order 104): the step before Dynamax's (DESIGN.md §44.3).
    MegaEvolve,
    /// Ultra Burst (order 104 too, its own mechanic): the same step as Mega Evolution's.
    UltraBurst,
    /// Terastallization (order 106): the `ActionChoice::Move` carries the mechanic.
    Terastallize,
    /// Dynamax (order 105): the step before Terastallization's.
    Dynamax,
}

impl ActionStep {
    /// The step a choice's gimmick runs as (the move choice carries it); `Act` for everything
    /// else.
    pub fn of(choice: &ActionChoice) -> Self {
        match choice {
            ActionChoice::Move {
                mechanic: Some(crate::engine::MechanicChoice::Terastallize),
                ..
            } => Self::Terastallize,
            ActionChoice::Move {
                mechanic: Some(crate::engine::MechanicChoice::MegaEvolve),
                ..
            } => Self::MegaEvolve,
            ActionChoice::Move {
                mechanic: Some(crate::engine::MechanicChoice::UltraBurst),
                ..
            } => Self::UltraBurst,
            ActionChoice::Move {
                mechanic: Some(crate::engine::MechanicChoice::Dynamax),
                ..
            } => Self::Dynamax,
            _ => Self::Act,
        }
    }

    /// The ordering priority Showdown's `resolveAction` gives the step.
    pub fn action_priority(
        self,
        rules: &crate::rules::ActionOrderRules,
        move_priority: i16,
    ) -> i16 {
        match self {
            Self::Act => move_priority,
            Self::MegaEvolve | Self::UltraBurst => rules.mega_action_priority,
            Self::Terastallize => rules.tera_action_priority,
            Self::Dynamax => rules.dynamax_action_priority,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedAction {
    #[serde(with = "crate::snapshot::wire")]
    pub actor: PokemonId,
    #[serde(with = "crate::snapshot::wire")]
    pub choice: ActionChoice,
    /// The gimmick step this entry runs, before the move action of the same choice.
    pub step: ActionStep,
    pub order: ActionOrder,
    /// The move that moved this action to the front of the queue (Round's `onTry`, Showdown
    /// `queue.prioritizeAction`), read by the move as its `sourceEffect`.
    #[serde(with = "crate::snapshot::wire")]
    pub prioritized_by: Option<crate::ids::MoveId>,
    /// The action is one a move injected for its target (Instruct, DESIGN.md §42.9): it runs
    /// besides the target's own action for the turn, which it must not consume.
    pub instructed: bool,
    /// Showdown `originalTarget`: the Pokémon the declared target slot held as the action was
    /// queued. A move that tracks its target goes at it while it is on the field, wherever an
    /// Ally Switch put it (R-M8-106).
    #[serde(with = "crate::snapshot::wire")]
    pub original_target: Option<crate::ids::PokemonId>,
}

/// The Pokémon `choice`'s declared target slot holds now (`QueuedAction::original_target`).
pub(crate) fn original_target(
    state: &crate::state::BattleState,
    topology: &crate::rules::BattleTopology,
    choice: &ActionChoice,
) -> Option<crate::ids::PokemonId> {
    match choice {
        ActionChoice::Move {
            target: Some(target),
            ..
        } => state.pokemon_at_position(target.0, topology),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionQueue {
    actions: Vec<QueuedAction>,
}

impl ActionQueue {
    pub fn push(&mut self, action: QueuedAction) {
        self.actions.push(action);
    }

    /// Action class and move priority are always descending. Trick Room only reverses the speed
    /// comparison inside an equal-priority bracket; it does not reverse switches or move priority.
    pub fn sort_with_trick_room(&mut self, trick_room: bool) {
        Self::sort_actions(&mut self.actions, trick_room);
    }

    pub(crate) fn sort_actions(actions: &mut [QueuedAction], trick_room: bool) {
        actions.sort_by(|left, right| compare_orders(&left.order, &right.order, trick_room));
    }

    pub fn as_slice(&self) -> &[QueuedAction] {
        &self.actions
    }

    pub fn into_actions(self) -> Vec<QueuedAction> {
        self.actions
    }
}
