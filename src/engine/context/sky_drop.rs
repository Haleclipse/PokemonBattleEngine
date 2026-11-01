//! Sky Drop (DESIGN.md §15.23, Gen 5-7): its first turn carries the target up
//! (`VolatileCondition::SkyDrop` on the user with the two-turn lock, `SkyDropped` on the target);
//! the next drops it. While up, both are out of reach of all but the moves that reach a Pokémon
//! in the air, the target cannot move, switch or be dragged out, and the user cannot be dragged
//! out either.

use crate::ids::{MoveId, PokemonId, PositionId};
use crate::state::VolatileConditionKind;

use super::BattleContext;
use crate::engine::TraceEvent;

impl<'a> BattleContext<'a> {
    /// The charging turn's `onTryHit` that went through: `user` carries `target` up (`-prepare`;
    /// the two-turn lock aimed at `aimed`, its `skydrop` volatile).
    pub fn lift(&mut self, user: PokemonId, target: PokemonId, aimed: Option<PositionId>) {
        let turns = self.rules.conditions.two_turn_move_turns;
        if let Some(pokemon) = self.state.pokemon_mut(user)
            && pokemon
                .volatiles
                // Its own volatiles hide the two (`two_turn::hiding`).
                .set_two_turn_move(MoveId::SKY_DROP, turns, aimed, None)
        {
            self.trace.push(TraceEvent::VolatileStarted {
                target: user,
                kind: VolatileConditionKind::TwoTurnMove,
            });
        }
        if let Some(pokemon) = self.state.pokemon_mut(user)
            && pokemon.volatiles.set_sky_drop(target)
        {
            self.trace.push(TraceEvent::VolatileStarted {
                target: user,
                kind: VolatileConditionKind::SkyDrop,
            });
        }
        if let Some(pokemon) = self.state.pokemon_mut(target)
            && pokemon.volatiles.set_sky_dropped(user)
        {
            self.trace.push(TraceEvent::VolatileStarted {
                target,
                kind: VolatileConditionKind::SkyDropped,
            });
        }
        self.trace.push(TraceEvent::ChargeStarted {
            actor: user,
            move_id: MoveId::SKY_DROP,
        });
    }

    /// Showdown `isSkyDropped`: Sky Drop holds `pokemon` in the air.
    pub fn is_sky_dropped(&self, pokemon: PokemonId) -> bool {
        sky_dropped(self.state, pokemon)
    }

    /// `user`'s `skydrop` volatile ends (the drop, its count running out, Gravity, the user
    /// leaving the field): the Pokémon it carried is down again. Returns that Pokémon.
    pub fn end_sky_drop(&mut self, user: PokemonId) -> Option<PokemonId> {
        let target = self
            .state
            .pokemon(user)
            .and_then(|pokemon| pokemon.volatiles.sky_drop_target())?;
        self.end_volatile(user, VolatileConditionKind::SkyDrop);
        // The `skydrop` volatile is the charge's own condition (`twoturnmove`'s
        // `addVolatile(move.id)`): the charge is over with it, `twoturnmove` alone stays.
        if let Some(pokemon) = self.state.pokemon_mut(user) {
            pokemon.volatiles.release_two_turn_move();
        }
        if self
            .state
            .pokemon(target)
            .and_then(|pokemon| pokemon.volatiles.sky_dropped_by())
            == Some(user)
        {
            self.end_volatile(target, VolatileConditionKind::SkyDropped);
        }
        Some(target)
    }
}

/// Whether Sky Drop holds `pokemon` in the air, as its user or as the Pokémon carried up
/// (Showdown `isSkyDropped`, and the user's `skydrop` volatile).
pub fn sky_dropped(state: &crate::state::BattleState, pokemon: PokemonId) -> bool {
    state.pokemon(pokemon).is_some_and(|p| {
        p.volatiles.contains(VolatileConditionKind::SkyDrop)
            || p.volatiles.contains(VolatileConditionKind::SkyDropped)
    })
}
