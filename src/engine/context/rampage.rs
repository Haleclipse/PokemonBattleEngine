//! `BattleContext`'s side of Thrash and its kind (Showdown `lockedmove`, DESIGN.md §15.12): the
//! lock as it starts or goes on, and its end with the user's confusion.

use crate::ids::{MoveId, PokemonId};
use crate::rules::RampageCounter;
use crate::state::{RampageState, RandomRequestKind, VolatileConditionKind};

use super::BattleContext;
use crate::engine::TraceEvent;

/// Showdown `this.random(2, 4)`: the lock's length (Gen 1 uses, Gen 3-4 turns, Gen 5+ hidden
/// length).
const LENGTH: (u16, u16) = (2, 3);

/// Gen 2 Thrash's fatigue (pokecrystal `BattleCommand_CheckRampage`: `BattleRandom & 1`, plus 2):
/// a confusion of 2-3 turns, where any other lasts 2-5.
const GEN2_FATIGUE: (u8, u8) = (2, 3);

impl<'a> BattleContext<'a> {
    /// Showdown `selfDrops` → `addVolatile('lockedmove')` (Gen 3+): a hit by `move_id` starts the
    /// lock — Gen 3-4 the cartridge's 2-3 turns, Gen 5+ a hidden length of 2-3 turns with a
    /// `duration` of 2 — or, Gen 5+, puts the `duration` back to 2 while the length allows
    /// (`onRestart`); Gen 3-4 leave a running count alone. Nothing for a user that has fainted.
    pub fn rampage_hit(&mut self, user: PokemonId, move_id: MoveId) {
        let counter = self.rules.conditions.rampage.counter;
        let Some(pokemon) = self.state.pokemon(user).filter(|p| !p.is_fainted()) else {
            return;
        };
        if pokemon.volatiles.rampage().is_some() {
            if counter == RampageCounter::HiddenLength
                && let Some(rampage) = self
                    .state
                    .pokemon_mut(user)
                    .and_then(|p| p.volatiles.rampage_mut())
                && rampage.left >= 2
            {
                rampage.turns = 2;
            }
            return;
        }
        let drawn =
            self.state
                .rng
                .range_inclusive_for(RandomRequestKind::Duration, LENGTH.0, LENGTH.1)
                as u8;
        let rampage = match counter {
            RampageCounter::Gen1Uses | RampageCounter::Gen2Uses => return,
            RampageCounter::Cartridge => RampageState {
                move_id,
                turns: drawn,
                left: 0,
                accuracy: 0,
                failed: false,
            },
            RampageCounter::HiddenLength => RampageState {
                move_id,
                turns: 2,
                left: drawn,
                accuracy: 0,
                failed: false,
            },
        };
        self.start_rampage_volatile(user, rampage);
    }

    /// Gen 1-2 (`RampageCounter::counts_uses`), as a use of `move_id` starts — before its hit
    /// test (Gen 1 `tryMoveHit`; pokecrystal `BattleCommand_CheckRampage` /
    /// `BattleCommand_Rampage`): with no lock, one starts — Gen 1 with 2-3 uses left and the
    /// accuracy byte 255, Gen 2 with 1-2 more (the drawn 2-3 less one); a later use counts one
    /// down, and the one that reaches 0 confuses its user, a confusion already there rolled anew
    /// (Gen 1 `delete volatiles['confusion']`, `[silent]`). Gen 2's lasts 2-3 turns, and
    /// Safeguard on the user's side spares it (pokecrystal `SafeCheckSafeguard`, issues.md I108).
    pub fn counted_rampage_use(&mut self, user: PokemonId, move_id: MoveId) {
        let counter = self.rules.conditions.rampage.counter;
        let Some(pokemon) = self.state.pokemon(user) else {
            return;
        };
        if pokemon.volatiles.rampage().is_none() {
            let drawn =
                self.state
                    .rng
                    .range_inclusive_for(RandomRequestKind::Duration, LENGTH.0, LENGTH.1)
                    as u8;
            let (turns, accuracy) = if counter == RampageCounter::Gen2Uses {
                (drawn - 1, 0)
            } else {
                (drawn, 255)
            };
            self.start_rampage_volatile(
                user,
                RampageState {
                    move_id,
                    turns,
                    left: 0,
                    accuracy,
                    failed: false,
                },
            );
            return;
        }
        let ends = self
            .state
            .pokemon_mut(user)
            .and_then(|p| p.volatiles.rampage_mut())
            .is_some_and(|rampage| {
                rampage.turns = rampage.turns.saturating_sub(1);
                rampage.turns == 0
            });
        if !ends {
            return;
        }
        if counter == RampageCounter::Gen2Uses {
            let safeguarded = self.state.pokemon(user).is_some_and(|pokemon| {
                self.state
                    .side(pokemon.side)
                    .conditions
                    .contains(crate::state::SideConditionKind::Safeguard)
            });
            if safeguarded {
                return;
            }
        }
        if let Some(pokemon) = self.state.pokemon_mut(user) {
            pokemon.volatiles.remove(VolatileConditionKind::Confusion);
        }
        let _ = if counter == RampageCounter::Gen2Uses {
            self.try_set_confusion_lasting(user, user, GEN2_FATIGUE)
        } else {
            self.try_set_confusion(user, user)
        };
    }

    /// The lock ends (Showdown `removeVolatile('lockedmove')`): with `confuse`, its user is
    /// confused (`addVolatile('confusion')`, `[fatigue]`) — nothing when it is confused already
    /// or Misty Terrain keeps it from it.
    pub fn end_rampage(&mut self, user: PokemonId, confuse: bool) {
        if !self
            .state
            .pokemon_mut(user)
            .is_some_and(|p| p.volatiles.remove(VolatileConditionKind::Rampage))
        {
            return;
        }
        self.trace.push(TraceEvent::VolatileEnded {
            target: user,
            kind: VolatileConditionKind::Rampage,
            seen: true,
        });
        if confuse {
            let _ = self.try_set_confusion(user, user);
        }
    }

    fn start_rampage_volatile(&mut self, user: PokemonId, rampage: RampageState) {
        if self
            .state
            .pokemon_mut(user)
            .is_some_and(|p| p.volatiles.start_rampage(rampage))
        {
            self.trace.push(TraceEvent::VolatileStarted {
                target: user,
                kind: VolatileConditionKind::Rampage,
            });
        }
    }
}
