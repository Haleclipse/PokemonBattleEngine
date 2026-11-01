//! Torment and Imprison (Showdown `torment`, `imprison`; DESIGN.md §15.13): the volatiles as
//! they start, and who keeps a Pokémon from a move.

use crate::ids::{MoveId, PokemonId};
use crate::state::{BattleState, VolatileConditionKind};

use super::BattleContext;
use crate::engine::ConditionApplyError;

/// The foe whose Imprison keeps `actor` from `move_id` (Showdown `imprison`'s
/// `onFoeDisableMove` / `onFoeBeforeMove`): any of its foes on the field — adjacent or not —
/// holding Imprison and knowing the move as it is now. Struggle is never imprisoned.
pub(crate) fn imprisoned_by(
    state: &BattleState,
    actor: PokemonId,
    move_id: MoveId,
) -> Option<PokemonId> {
    if move_id == MoveId::STRUGGLE {
        return None;
    }
    let side = state.pokemon(actor)?.side.other();
    state
        .side(side)
        .active
        .iter()
        .flatten()
        .copied()
        .find(|holder| {
            state.pokemon(*holder).is_some_and(|pokemon| {
                !pokemon.is_fainted()
                    && pokemon.volatiles.contains(VolatileConditionKind::Imprison)
                    && pokemon.move_slot_of(move_id).is_some()
            })
        })
}

/// Whether Torment keeps `pokemon` from `move_id` (Showdown `torment`'s `onDisableMove`): its
/// last move, Struggle aside.
pub(crate) fn tormented(state: &BattleState, pokemon: PokemonId, move_id: MoveId) -> bool {
    move_id != MoveId::STRUGGLE
        && state.pokemon(pokemon).is_some_and(|p| {
            p.volatiles.contains(VolatileConditionKind::Torment) && p.last_move == Some(move_id)
        })
}

impl<'a> BattleContext<'a> {
    /// Torment (Showdown `addVolatile('torment')`): refused when the target is tormented
    /// already — before `TryAddVolatile`, so an Aroma Veil does not name itself then (R-M10-60)
    /// —, an Aroma Veil covers it, or it is Dynamaxed (`torment`'s onStart deletes the volatile
    /// it would get and fails, DESIGN.md §44.5).
    pub fn set_torment(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self
            .state
            .pokemon(target)
            .filter(|pokemon| !pokemon.is_fainted())
        else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon.volatiles.contains(VolatileConditionKind::Torment) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        if self.ability_blocks_volatile(target, VolatileConditionKind::Torment) {
            return Err(ConditionApplyError::Immune);
        }
        if self
            .state
            .pokemon(target)
            .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::Dynamax))
        {
            return Err(ConditionApplyError::Immune);
        }
        self.start_timed(target, VolatileConditionKind::Torment, |v| {
            v.set_marker(VolatileConditionKind::Torment)
        })
    }

    /// Imprison on its user (Showdown `addVolatile('imprison')`): refused when it imprisons
    /// already, and — Gen 3-4's `onTryHit`, `needs_shared_move` — unless a foe on the field
    /// knows one of its moves.
    pub fn set_imprison(
        &mut self,
        user: PokemonId,
        needs_shared_move: bool,
    ) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon(user) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if needs_shared_move {
            let moves: Vec<MoveId> = pokemon
                .move_slots()
                .iter()
                .map(|slot| slot.move_id)
                .collect();
            let side = pokemon.side.other();
            let shared = self
                .state
                .side(side)
                .active
                .iter()
                .flatten()
                .filter_map(|foe| self.state.pokemon(*foe))
                .filter(|foe| !foe.is_fainted())
                .any(|foe| moves.iter().any(|id| foe.move_slot_of(*id).is_some()));
            if !shared {
                return Err(ConditionApplyError::Immune);
            }
        }
        self.set_marker_volatile(user, VolatileConditionKind::Imprison)
    }
}
