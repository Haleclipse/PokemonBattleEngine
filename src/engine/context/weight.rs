//! Weight (DESIGN.md §20.1, R169): Autotomize takes 100 kg off its user per use, as long as
//! the user's species weight less what it already lost is above 0.1 kg (Showdown: the gen5
//! volatile's `effectState.multiplier`, later `weighthg` itself); `engine::weight_hg` reads it.

use crate::ids::PokemonId;
use crate::state::VolatileConditionKind;

use super::BattleContext;
use crate::engine::TraceEvent;

impl<'a> BattleContext<'a> {
    /// Autotomize's weight loss on `pokemon`: its volatile starts (or is used again), 100 kg
    /// lighter if it still weighs more than 0.1 kg before any `ModifyWeight` modifier.
    pub fn autotomize(&mut self, pokemon: PokemonId) {
        let species_weight = self
            .dex
            .zip(
                self.state
                    .pokemon(pokemon)
                    .and_then(crate::engine::species_of),
            )
            .and_then(|(dex, species)| dex.species_by_id(species))
            .map_or(1, |species| species.weight_hg);
        let Some(target) = self.state.pokemon_mut(pokemon) else {
            return;
        };
        let lost = 1000 * u32::from(target.volatiles.autotomize_count());
        let lighter = species_weight.saturating_sub(lost) > 1;
        target.volatiles.add_autotomize(lighter);
        // Each use that takes weight off shows (`-start|p|Autotomize`); one that cannot (a
        // Haunter's 0.1 kg) shows nothing, its volatile none of the observers' (R-M10-36).
        if lighter {
            self.trace.push(TraceEvent::VolatileStarted {
                target: pokemon,
                kind: VolatileConditionKind::Autotomize,
            });
        }
    }
}
