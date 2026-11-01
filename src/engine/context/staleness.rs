//! Showdown's staleness writers (DESIGN.md §45.7). Each records how a Pokémon may be keeping
//! the battle from ending, for Endless Battle Clause to read at the turn's end
//! (`BattleEngine::end_turn_rules`). None of them shows in the battle.

use crate::ids::{ItemId, PokemonId};
use crate::state::Staleness;

use super::BattleContext;

/// Showdown `RESTORATIVE_BERRIES`: the berries that restore PP or HP.
pub(crate) const RESTORATIVE_BERRIES: [ItemId; 9] = [
    ItemId::LEPPA_BERRY,
    ItemId::AGUAV_BERRY,
    ItemId::ENIGMA_BERRY,
    ItemId::FIGY_BERRY,
    ItemId::IAPAPA_BERRY,
    ItemId::MAGO_BERRY,
    ItemId::SITRUS_BERRY,
    ItemId::WIKI_BERRY,
    ItemId::ORAN_BERRY,
];

impl<'a> BattleContext<'a> {
    /// A foe's doing made `target` stale for the battle (`staleness = 'external'`):
    /// - Heal Pulse or Floral Healing healed it and `source` is not its ally;
    /// - Bug Bite or Pluck took its Leppa Berry and the user ate it;
    /// - it ate a Leppa Berry flung at it.
    pub(crate) fn make_stale(&mut self, target: PokemonId, source: Option<PokemonId>) {
        if source.is_some_and(|source| self.side_of(source) == self.side_of(target)) {
            return;
        }
        if let Some(pokemon) = self.state.pokemon_mut(target) {
            pokemon.staleness = Some(Staleness::External);
        }
    }

    /// `target` took an ability from `source` (Skill Swap, Wandering Spirit, Entrainment): it
    /// is stale for its stint on the field when `source` is not its ally
    /// (`volatileStaleness = 'external'`).
    pub(crate) fn make_volatile_stale(&mut self, target: PokemonId, source: PokemonId) {
        if self.side_of(source) == self.side_of(target) {
            return;
        }
        if let Some(pokemon) = self.state.pokemon_mut(target) {
            pokemon.volatile_staleness = Some(Staleness::External);
        }
    }

    /// `eatItem` of `item` by `holder`: when the berry is a restorative one, an item change that
    /// gave it (`pendingStaleness`) makes the holder stale by its own doing. That does not
    /// overwrite a foe's doing. The pending mark goes either way.
    pub(super) fn eat_settles_staleness(&mut self, holder: PokemonId, item: ItemId) {
        if !RESTORATIVE_BERRIES.contains(&item) {
            return;
        }
        // The mark is taken first, so it goes even when the holder is stale already.
        if let Some(pokemon) = self.state.pokemon_mut(holder)
            && std::mem::take(&mut pokemon.pending_staleness)
            && pokemon.staleness.is_none()
        {
            pokemon.staleness = Some(Staleness::Internal);
        }
    }
}
