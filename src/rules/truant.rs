//! Truant (DESIGN.md §15.21): how a generation keeps track of the turns its holder loafs on.

use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruantModel {
    /// Gen 3 (pokeemerald `truantCounter`, Showdown gen3 `truantTurn`): a flag on the Pokémon
    /// (`PokemonState::truant_turn`). Coming in after the battle's start with Truant sets it,
    /// coming in otherwise clears it (issues.md I107: the cartridge clears the whole
    /// `DisableStruct` on every switch-in); every turn end the holder has Truant flips it. The
    /// holder loafs while it is set.
    TurnEndFlip,
    /// Gen 4 (pokeplatinum `moveEffectsData.truant`, issues.md I106): the parity of the turns the
    /// holder loafs on (`PokemonState::truant_turn`, odd: set), whatever happens in between —
    /// sleep, recharging. Coming in sets it to the parity of that turn (every Pokémon; the
    /// battle's start is turn 0), so the holder acts on the turn after and loafs on the next;
    /// Role Play (its user) and Skill Swap (both) set it to that of the turn after, so a Pokémon
    /// that gains Truant by them loafs on the next turn.
    TurnParity,
    /// Gen 5+ (Showdown `truant`): the `truant` volatile, which each move attempt that reaches the
    /// check flips — present, the attempt loafs and the volatile goes; absent, it comes. The
    /// ability's start clears it, and sets it again on a Pokémon past its first turn that has
    /// moved this turn or will not move any more (`onStart`).
    MoveFlip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TruantRules {
    pub model: TruantModel,
}

impl TruantRules {
    pub const fn for_generation(generation: Generation) -> Self {
        Self {
            model: match generation {
                // Gen 1-2 have no abilities.
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => TruantModel::TurnEndFlip,
                Generation::Gen4 => TruantModel::TurnParity,
                _ => TruantModel::MoveFlip,
            },
        }
    }
}
