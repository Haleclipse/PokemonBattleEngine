//! The Metronome item by generation (DESIGN.md R126; issues.md I21): how its count moves
//! and what it multiplies.

use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetronomeRules {
    /// Gen 4 on the cartridge (pokeplatinum `BattleSystem_UpdateMetronomeCount` /
    /// `BattleSystem_VerifyMetronomeCount`, `BattleScript_CalcMoveDamage`): the chosen move
    /// counts as it starts — one more, up to `cap`, when it is the move counted before, from 0
    /// otherwise — but not on the attacking turn of a two-turn move; a move that did nothing
    /// (it missed, failed, met an immunity or a Protect) or a turn lost to anything but sleep or
    /// recharging takes one back. The damage is `× (10 + n) / 10`, floored, after Life Orb.
    Cartridge { cap: u8 },
    /// Gen 5+ (Showdown's `metronome` condition): `onTryMove` — one more when the move is the
    /// one counted before and succeeded last turn, the attacking turn of a two-turn move counts
    /// from 1, anything else from 0 — and `onModifyDamage`, `modifiers[min(n, 5)]` in the
    /// ModifyDamage chain.
    Showdown { modifiers: &'static [u32] },
}

impl MetronomeRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        match generation {
            Generation::Gen4 => Self::Cartridge { cap: 10 },
            _ => Self::Showdown {
                modifiers: &[4096, 4915, 5734, 6553, 7372, 8192],
            },
        }
    }
}
