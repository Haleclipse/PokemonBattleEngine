//! A move's user fainting by its own move as a generation has it (DESIGN.md §30.4): Explosion,
//! Self-Destruct, Memento …

use crate::spec::Generation;

/// When a `selfdestruct: 'always'` move knocks its user out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfDestructTiming {
    /// Gen 3, Gen 5+ (`useMoveInner`): as the move starts, before anything about its targets —
    /// a move left with no target still costs its user.
    BeforeTargets,
    /// Gen 2 (`tryMoveHit`), Gen 4 (`useMoveInner` after the no-target check): only when the
    /// move has a target, before its hit steps.
    WithTargets,
    /// Gen 1 (`tryMoveHit`): once the move is over — hit, missed or not affecting — unless it
    /// broke a Substitute.
    AfterMove,
}

/// How Healing Wish and Lunar Dance hand their heal to the next Pokémon in the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotHealRules {
    /// Gen 4 (the gen4 mod): the user's slot is filled at once, mid-turn (`switchFlag`), and
    /// the newcomer is healed; the condition lasts the turn (`duration: 1`).
    Immediate,
    /// Gen 5-7 (the gen7 mod): the replacement at the end of the turn is healed, whatever it
    /// needs; the condition lapses after two residuals (`duration: 2`).
    NextEntrant,
    /// Gen 8+: the condition stays until a Pokémon that needs it (HP, status; Lunar Dance also
    /// PP) comes into the slot.
    WhenNeeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfFaintRules {
    pub timing: SelfDestructTiming,
    pub slot_heal: SlotHealRules,
    /// Gen 1-4: Explosion and Self-Destruct halve the target's Defense in their damage (after
    /// the stat's modifiers; Gen 1-2 after the rollover), floored, at least 1.
    pub explosion_halves_defense: bool,
}

impl SelfFaintRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        Self {
            timing: match generation {
                Generation::Gen1 => SelfDestructTiming::AfterMove,
                Generation::Gen2 | Generation::Gen4 => SelfDestructTiming::WithTargets,
                _ => SelfDestructTiming::BeforeTargets,
            },
            slot_heal: match generation {
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4 => {
                    SlotHealRules::Immediate
                }
                Generation::Gen5 | Generation::Gen6 | Generation::Gen7 => {
                    SlotHealRules::NextEntrant
                }
                _ => SlotHealRules::WhenNeeded,
            },
            explosion_halves_defense: matches!(
                generation,
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4
            ),
        }
    }
}
