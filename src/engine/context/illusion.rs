//! Illusion's disguise (Showdown `pokemon.illusion`, DESIGN.md §41.7).

use super::BattleContext;
use crate::engine::TraceEvent;
use crate::ids::PokemonId;

impl<'a> BattleContext<'a> {
    /// Illusion's `onBeforeSwitchIn` as `pokemon` takes its slot: it appears as the last party
    /// member after it in the party order (`side.pokemon`, past its own index) that has not
    /// fainted — an active partner to its right included. None there, no disguise. The one it
    /// went out in goes first (`pokemon.illusion = null`), the ability at work or not.
    pub fn take_disguise(&mut self, pokemon: PokemonId) {
        let Some(holder) = self.state.pokemon_mut(pokemon) else {
            return;
        };
        holder.illusion = None;
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        if !crate::engine::ability_handler(self.state, self.rules, holder, |b| b.illusion) {
            return;
        }
        let order = &self.state.side(holder.side).battle_order;
        let own = order
            .iter()
            .position(|id| *id == pokemon)
            .unwrap_or(order.len());
        let disguise = order[own + 1..]
            .iter()
            .rev()
            .copied()
            .find(|id| self.state.pokemon(*id).is_some_and(|p| !p.is_fainted()));
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.illusion = disguise;
        }
    }

    /// Illusion's `onEnd` (a damaging hit, Neutralizing Gas, Gastro Acid, a new ability): the
    /// holder is seen as itself (`replace`, `-end|…|Illusion`).
    pub fn end_illusion(&mut self, pokemon: PokemonId) {
        let lifted = self
            .state
            .pokemon_mut(pokemon)
            .and_then(|holder| holder.illusion.take());
        if lifted.is_some() {
            self.trace
                .push(TraceEvent::IllusionEnded { holder: pokemon });
        }
    }
}
