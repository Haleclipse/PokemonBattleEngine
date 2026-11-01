//! What one side sees of the battle (DESIGN.md §41.7, user decision 2026-09-30): the engine
//! keeps the truth, and `BattleEngine::view_for` filters it for a side. For now the filter is
//! Illusion's alone — a foe in disguise shows the identity of the party member it appears as;
//! more hidden information (the foes' bench, items, unrevealed moves) is a later design on this
//! type.

use crate::ids::{PokemonId, PositionId, SideId, SpeciesId};
use crate::state::Gender;

use super::BattleEngine;

/// The battle as `viewer` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideView {
    pub viewer: SideId,
    /// Every active position, in the topology's order: what stands there, as `viewer` sees it.
    pub active: Vec<(PositionId, Option<SeenPokemon>)>,
}

/// An active Pokémon as one side sees it: its own HP, with the identity it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeenPokemon {
    /// The Pokémon itself. On the viewer's own side always; on the other side only while it
    /// shows its own identity (`None` under an Illusion).
    pub pokemon: Option<PokemonId>,
    /// The Pokémon whose identity it shows: itself, or Illusion's disguise to the other side.
    pub appears_as: PokemonId,
    pub species: Option<SpeciesId>,
    pub level: u8,
    pub gender: Gender,
    pub hp: u16,
    pub max_hp: u16,
}

impl BattleEngine {
    internal!(
        /// The battle as `viewer` sees it (DESIGN.md §41.7).
        pub fn view_for(&self, viewer: SideId) -> SideView {
            let topology = &self.rules.topology;
            let active = topology
                .positions()
                .iter()
                .map(|spec| {
                    let seen = self
                        .state
                        .pokemon_at_position(spec.id, topology)
                        .and_then(|id| self.state.pokemon(id))
                        .map(|pokemon| {
                            let disguise = pokemon.illusion.filter(|_| pokemon.side != viewer);
                            let shown = disguise
                                .and_then(|id| self.state.pokemon(id))
                                .unwrap_or(pokemon);
                            SeenPokemon {
                                pokemon: disguise.is_none().then(|| pokemon.id()),
                                appears_as: shown.id(),
                                species: crate::engine::species_of(shown),
                                level: shown.level,
                                gender: shown.gender,
                                hp: pokemon.hp,
                                max_hp: pokemon.max_hp,
                            }
                        });
                    (spec.id, seen)
                })
                .collect();
            SideView { viewer, active }
        }
    );
}
