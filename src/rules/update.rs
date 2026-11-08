//! When Showdown's `Update` runs (`eachEvent('Update')`, DESIGN.md §40), by generation: the
//! event that ends an infatuation or a Syrup Bomb whose source has left and that makes a
//! Pokémon eat its berry. The evidence, line by line, is in the §40 record.

use crate::spec::Generation;

/// Where a move's own `Update`s fall (the hit loop of `tryMoveHit` / `hitStepMoveHitLoop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveUpdates {
    /// Gen 1: none (the gen1 mod's `tryMoveHit`).
    None,
    /// Gen 2: after every hit of a multi-hit move (the gen2 mod's `tryMoveHit`).
    EachMultiHit,
    /// Gen 3: after every hit of a multi-hit move, and after each target's hit — its recoil
    /// included, its faints not yet processed — unless the move did nothing to it (the gen3
    /// mod's `tryMoveHit`, which `useMoveInner` calls target by target).
    EachMultiHitAndTarget,
    /// Gen 4+: after every hit, a single one too, and once the hit loop is over and its faints
    /// processed, when a target took damage (`hitStepMoveHitLoop`).
    EachHitAndMove,
}

impl MoveUpdates {
    /// An `Update` after each hit of a move that hits `multi_hit` times or once.
    pub const fn after_hit(self, multi_hit: bool) -> bool {
        match self {
            Self::None => false,
            Self::EachMultiHit | Self::EachMultiHitAndTarget => multi_hit,
            Self::EachHitAndMove => true,
        }
    }

    /// An `Update` after each target's hit (Gen 3).
    pub const fn after_target(self) -> bool {
        matches!(self, Self::EachMultiHitAndTarget)
    }

    /// An `Update` once the move's hits are over and their faints processed (Gen 4+).
    pub const fn after_move(self) -> bool {
        matches!(self, Self::EachHitAndMove)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateRules {
    /// Showdown `runAction` ends with `eachEvent('Update')` before it asks for a replacement from
    /// Gen 5; before that the `Update` comes only when no replacement is asked for — so a
    /// replacement request still sees what an `Update` would end.
    pub before_replacement: bool,
    /// Gen 1-4: the battle's `start` action (placing the leads) ends with an `Update` before
    /// the leads' switch-in effects; from Gen 5 it does not.
    pub after_lead_placement: bool,
    pub in_move: MoveUpdates,
    /// Gen 7+: right after the weather's `eachEvent('Weather')` in the residual.
    pub after_weather: bool,
    /// Gen 3-4: a dragged-in Pokémon's switch-in effects wait for their own action (the queued
    /// `runSwitch`), after the dragging action's `Update`; from Gen 5 they run at once, with
    /// no `Update` between (`switchIn` → `runSwitch` for a drag). So do Gen 2's, within the
    /// move (pokecrystal's `BattleCommand_ForceSwitch` goes straight on to `SpikesDamage`; the
    /// oracle's `cartridgeGen2SwitchIn`, issues.md I219): the next `Update` is the action's,
    /// after the user's residual (R-M11-103).
    pub drag_switch_in_deferred: bool,
}

impl UpdateRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        let modern = matches!(
            generation,
            Generation::Gen5
                | Generation::Gen6
                | Generation::Gen7
                | Generation::Gen8
                | Generation::Gen9
        );
        Self {
            before_replacement: modern,
            after_lead_placement: !modern,
            in_move: match generation {
                Generation::Gen1 => MoveUpdates::None,
                Generation::Gen2 => MoveUpdates::EachMultiHit,
                Generation::Gen3 => MoveUpdates::EachMultiHitAndTarget,
                _ => MoveUpdates::EachHitAndMove,
            },
            after_weather: matches!(
                generation,
                Generation::Gen7 | Generation::Gen8 | Generation::Gen9
            ),
            drag_switch_in_deferred: matches!(generation, Generation::Gen3 | Generation::Gen4),
        }
    }
}
