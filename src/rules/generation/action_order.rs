//! How a generation orders the turn's actions (`ActionOrderRules`).

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionOrderStrategy {
    StaticAtTurnStart,
    RecalculateAfterAction,
}

/// Terastallization's step (Showdown `resolveAction`'s order 106). The engine's `action_priority`
/// counts *down* (a higher value acts earlier): Showdown's order 106 sits between the switch
/// (103) and the moves (200, the engine's 0), so the step reads 5 — a switch still goes first and
/// a move that was put at the front of the queue (`prioritizeAction`, the engine's 100) still
/// beats it, as in Showdown (3 < 106).
pub const TERA_ACTION_PRIORITY: i16 = 5;

/// Dynamax's step (Showdown `resolveAction`'s order 105): before Terastallization's, after the
/// switches.
pub const DYNAMAX_ACTION_PRIORITY: i16 = 6;

/// Mega Evolution's and Ultra Burst's step (Showdown `resolveAction`'s order 104): after the
/// switches (103, the engine's 8) and before Dynamax (105, 6).
pub const MEGA_ACTION_PRIORITY: i16 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionOrderRules {
    pub strategy: ActionOrderStrategy,
    /// Gen 7 (Showdown's `runAction`): a Mega Evolution takes the Pokémon's move action out of
    /// the queue and puts it back at its new speed (DESIGN.md §44.3).
    pub mega_reorders_move: bool,
    pub switch_action_priority: i16,
    pub move_action_priority: i16,
    pub pass_action_priority: i16,
    /// Terastallization (`resolveAction`'s order 106): a step of its own, after switches and
    /// before every move (DESIGN.md §44.2).
    pub tera_action_priority: i16,
    /// Dynamax (`resolveAction`'s order 105): the step before Terastallization's.
    pub dynamax_action_priority: i16,
    /// Mega Evolution and Ultra Burst (`resolveAction`'s order 104): after the switches,
    /// before Dynamax's step.
    pub mega_action_priority: i16,
    /// An action moved to the front of the queue (Showdown `prioritizeAction`: order 3, ahead of
    /// everything a turn queues).
    pub prioritized_action_priority: i16,
    /// Gen 1 (the cartridge's move menu, Showdown `side.lastSelectedMove`): a frozen or
    /// sleeping Pokémon acts with the move its side last selected from the menu, so the turn
    /// order takes that move's priority (Quick Attack's +1, Counter's −1).
    pub sleeping_frozen_act_with_last_selected: bool,
    /// Gen 1 (Showdown gen1 `side.commitChoices`: `choice.moveid = this.lastSelectedMove`; the
    /// cartridge skips the menu and keeps `wPlayerSelectedMove`): a recharging Pokémon's action
    /// is its side's last selected move — Hyper Beam again when a partial-trapping move clears
    /// the recharge before it acts (R-M8-107); one Metronome called meets the Desync Clause,
    /// its PP paid from Metronome's slot (`PpRules::desync_clause`, R-M11-105).
    pub recharge_acts_with_last_selected: bool,
    /// Gen 2-3 Quick Claw (Showdown `quickClawRoll`, drawn as the turn starts: 60/256 in Gen 2,
    /// 1/5 in Gen 3): when it succeeds every holder acts at Speed 65535 for the rest of the
    /// turn. `None` from Gen 4, where Quick Claw is a fractional priority rolled per move.
    pub quick_claw_turn_roll: Option<Fraction>,
    /// Lagging Tail and Full Incense's `onFractionalPriority`, in tenths of a priority step:
    /// Gen 4 −2 (run before Stall, which keeps a negative value), Gen 5+ −1.
    pub delay_item_tenths: i8,
    /// Gen 4 `getActionSpeed`: a move with a negative fractional priority orders by reversed
    /// Speed (slowest first), Trick Room or not.
    pub delayed_slowest_first: bool,
    /// Gen 4 (issues.md I44: pokeplatinum `BattleSystem_CompareBattlerSpeed` compares two
    /// Pokémon whose Quick Claw or Custap Berry went off by their Speed before its Trick Room
    /// branch): a move with a positive fractional priority orders fastest first, Trick Room or
    /// not.
    pub boosted_fastest_first: bool,
    /// Gen 4 Custap Berry (Showdown gen4 `onBeforeTurn` / `onCustap`, pokeplatinum's
    /// `subscript_check_quick_claw` as the move begins): its holder is put first in its bracket
    /// as the turn starts and eats the berry as its move begins. Gen 5+
    /// (`onFractionalPriority`): it eats it as the turn's actions are ordered.
    pub custap_eaten_as_the_move_begins: bool,
    /// Gale Wings needs its holder at full HP from Gen 7 (the gen6 mod drops the check).
    pub gale_wings_full_hp: bool,
    /// Gen 7+ `hitStepTryImmunity`: a Dark-type foe is immune to a Prankster-boosted move.
    pub prankster_dark_immunity: bool,
    /// Gen 6+ `getActionSpeed` writes the modified priority back to the move
    /// (`action.move.priority`): Quick Guard then sees Prankster's +1. Gen 5 keeps the base.
    pub boosted_priority_at_hit: bool,
    /// Where Quash sends the target's action (DESIGN.md §42.4).
    pub quash: QuashOrder,
}

/// Where Quash sends the target's pending action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuashOrder {
    /// Gen 5-7 (the gen7 mod: `priority = -7.1`, `cancelMove`, back in before the residual):
    /// to the end of the queue, after anything already there.
    End,
    /// Gen 8+ (`action.order = 201`): into a bracket of its own after every move, which the
    /// re-sort before each move orders like any other — this action priority.
    LastBracket(i16),
}

/// The generation's action order.
pub(super) fn resolve(generation: Generation) -> ActionOrderRules {
    ActionOrderRules {
        tera_action_priority: TERA_ACTION_PRIORITY,
        dynamax_action_priority: DYNAMAX_ACTION_PRIORITY,
        mega_action_priority: MEGA_ACTION_PRIORITY,
        strategy: if matches!(generation, Generation::Gen8 | Generation::Gen9) {
            ActionOrderStrategy::RecalculateAfterAction
        } else {
            ActionOrderStrategy::StaticAtTurnStart
        },
        // A switch runs before a Mega Evolution's step of the same turn (Showdown's 103 <
        // 104): the engine's scale counts down, so the switch's value is the higher one.
        switch_action_priority: 8,
        mega_reorders_move: generation == Generation::Gen7,
        move_action_priority: 0,
        pass_action_priority: -100,
        prioritized_action_priority: 100,
        sleeping_frozen_act_with_last_selected: generation == Generation::Gen1,
        recharge_acts_with_last_selected: generation == Generation::Gen1,
        quick_claw_turn_roll: match generation {
            Generation::Gen2 => Some(Fraction::new(60, 256)),
            Generation::Gen3 => Some(Fraction::new(1, 5)),
            _ => None,
        },
        delay_item_tenths: if generation == Generation::Gen4 {
            -2
        } else {
            -1
        },
        delayed_slowest_first: generation == Generation::Gen4,
        boosted_fastest_first: generation == Generation::Gen4,
        custap_eaten_as_the_move_begins: generation == Generation::Gen4,
        gale_wings_full_hp: generation >= Generation::Gen7,
        prankster_dark_immunity: generation >= Generation::Gen7,
        boosted_priority_at_hit: generation >= Generation::Gen6,
        quash: if generation >= Generation::Gen8 {
            QuashOrder::LastBracket(-1)
        } else {
            QuashOrder::End
        },
    }
}
