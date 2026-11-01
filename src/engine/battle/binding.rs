//! Gen 1 binding around the turn (DESIGN.md §30.9): who loses the turn's action, the lock's
//! and the caught Pokémon's turns counted down at the end of the turn, and the gen1 `endTurn`
//! clean-up of the pair.

use crate::ids::PokemonId;
use crate::state::{NonVolatileStatus, PokemonState, SelectedMove, VolatileConditionKind};

use super::BattleEngine;
use crate::engine::TraceEvent;

impl BattleEngine {
    /// Showdown gen1 `commitChoices`, run after the side's last selected move is recorded
    /// (`record_selected_moves`): the Pokémon that act `cannotmove`. One that picks from the
    /// menu does when it is partially trapped; one that does not — asleep, frozen or held to a
    /// move (`held_to_a_move`) — acts with its side's last selected move, which is
    /// `cannotmove` after a partially trapped teammate's turn or a Gen 1 Haze (R190) — except
    /// under Thrash's lock, whose `onBeforeTurn` puts the locked move back as the turn starts
    /// (`changeAction`; R273).
    pub(super) fn bound_movers(&self) -> Vec<PokemonId> {
        if !self.rules.conditions.switching.binding_locks_user {
            return Vec::new();
        }
        self.state
            .pending_movers
            .iter()
            .copied()
            .filter(|actor| {
                self.state.pokemon(*actor).is_some_and(|pokemon| {
                    if pokemon.volatiles.rampage().is_some() {
                        return false;
                    }
                    let picks = !pokemon.has_status(NonVolatileStatus::Sleep)
                        && !pokemon.has_status(NonVolatileStatus::Freeze)
                        && !held_to_a_move(pokemon);
                    if picks {
                        pokemon
                            .volatiles
                            .contains(VolatileConditionKind::PartiallyTrapped)
                    } else {
                        self.state.side(pokemon.side).last_selected_move
                            == Some(SelectedMove::CannotMove)
                    }
                })
            })
            .collect()
    }

    /// The residual's `duration` count-down (Gen 1 has no other residual): the lock, the caught
    /// Pokémon's `partiallytrapped` and `fakepartiallytrapped`, each gone at 0.
    pub(super) fn tick_binding(&mut self) {
        if !self.rules.conditions.switching.binding_locks_user {
            return;
        }
        for pokemon_id in self.active_pokemon() {
            for kind in [
                VolatileConditionKind::PartialTrappingLock,
                VolatileConditionKind::PartiallyTrapped,
                VolatileConditionKind::FakePartiallyTrapped,
            ] {
                if let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
                    && pokemon.volatiles.tick_timed(kind) == Some(0)
                {
                    self.trace.push(TraceEvent::VolatileEnded {
                        target: pokemon_id,
                        kind,
                        seen: true,
                    });
                }
            }
        }
    }

    /// Showdown gen1 `endTurn`: a lock whose Pokémon is down or no longer caught ends; a caught
    /// Pokémon whose binder is down or no longer locked goes free; a `fakepartiallytrapped`
    /// whose counterpart is down or shows none ends — each active Pokémon in turn.
    pub(super) fn clean_up_binding(&mut self) {
        if !self.rules.conditions.switching.binding_locks_user {
            return;
        }
        let gone = |state: &crate::state::BattleState, other: PokemonId, kind| {
            state
                .pokemon(other)
                .is_none_or(|p| p.is_fainted() || !p.volatiles.contains(kind))
        };
        for pokemon_id in self.active_pokemon() {
            let Some(pokemon) = self.state.pokemon(pokemon_id) else {
                continue;
            };
            let lock = pokemon.volatiles.partial_trapping_lock().map(|lock| lock.0);
            let binder = pokemon.volatiles.partially_trapped_by().map(|by| by.0);
            let fake = pokemon.volatiles.fake_partially_trapped_counterpart();
            let mut ended = Vec::new();
            if lock.is_some_and(|target| {
                gone(&self.state, target, VolatileConditionKind::PartiallyTrapped)
            }) {
                ended.push(VolatileConditionKind::PartialTrappingLock);
            }
            if binder.is_some_and(|source| {
                gone(
                    &self.state,
                    source,
                    VolatileConditionKind::PartialTrappingLock,
                )
            }) {
                ended.push(VolatileConditionKind::PartiallyTrapped);
            }
            if fake.is_some_and(|counterpart| {
                gone(
                    &self.state,
                    counterpart,
                    VolatileConditionKind::FakePartiallyTrapped,
                )
            }) {
                ended.push(VolatileConditionKind::FakePartiallyTrapped);
            }
            for kind in ended {
                if let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
                    && pokemon.volatiles.remove(kind)
                {
                    self.trace.push(TraceEvent::VolatileEnded {
                        target: pokemon_id,
                        kind,
                        seen: true,
                    });
                }
            }
        }
    }
}

/// Showdown gen1 `chooseMove`: a Pokémon held to a move — `getLockedMove()` (a charge, a
/// recharge) or `getSemiLockedMove()` (Bide, its own binding move) — submits that move, not the
/// Fight button. `commitChoices` then neither records it as the side's last selected move nor
/// turns it into `cannotmove` when the Pokémon is caught: the move runs, and `partiallytrapped`'s
/// `onBeforeMove` stops it (`cant`, R190).
pub(super) fn held_to_a_move(pokemon: &PokemonState) -> bool {
    pokemon.locked_move().is_some()
        || pokemon.semi_locked_move().is_some()
        || pokemon.volatiles.partial_trapping_lock().is_some()
        || pokemon
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
}
