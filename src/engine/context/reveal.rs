//! `BattleContext`'s items used up and what the battle shows of items and abilities (DESIGN.md
//! §47.0 item 12): an item consumed, seen or not, announced, revealed, at work; an ability at
//! work or activated, Forewarn's warning.

use crate::engine::TraceEvent;
use crate::ids::{AbilityId, ItemId, MoveId, PokemonId};

use super::*;

impl<'a> BattleContext<'a> {
    /// Showdown `useItem` (Focus Sash, a gem, Red Card, Eject Button): the held item is used
    /// up and traced; it becomes the holder's last item and the holder has used an item this
    /// turn (§39.1). Returns the item. A fainted holder uses nothing but a gem (`useItem`: `!this.hp
    /// && !isGem`): Seed Sower's terrain as its holder falls leaves the seed held, while the gem
    /// of an Explosion user, down before its move strikes, still goes. (Showdown's holder out of
    /// play never reaches it here: only active Pokémon use items.)
    pub fn consume_item(&mut self, holder: PokemonId) -> Option<ItemId> {
        self.use_up_item(holder, true)
    }

    /// `consume_item` with no message (Natural Gift's berry: `setItem('')`).
    pub fn consume_item_unseen(&mut self, holder: PokemonId) -> Option<ItemId> {
        self.use_up_item(holder, false)
    }

    fn use_up_item(&mut self, holder: PokemonId, seen: bool) -> Option<ItemId> {
        let pokemon = self.state.pokemon(holder)?;
        if pokemon.hp == 0
            && !pokemon
                .held_item
                .is_some_and(|item| crate::engine::item_behavior(self.rules, item).is_gem)
        {
            return None;
        }
        let pokemon = self.state.pokemon_mut(holder)?;
        let item = pokemon.held_item.take()?;
        pokemon.item_state = crate::state::ItemState::default();
        pokemon.history.last_item = Some(item);
        pokemon.history.used_item_this_turn = true;
        self.trace
            .push(TraceEvent::ItemConsumed { holder, item, seen });
        self.after_use_item(holder);
        Some(item)
    }

    /// `item` names itself on `holder` (Showdown `-item|holder|item`: an Air Balloon's `onStart`).
    pub fn announce_item(&mut self, holder: PokemonId, item: ItemId) {
        self.trace.push(TraceEvent::ItemAnnounced { holder, item });
    }

    /// Frisk names `holder`'s item, whatever sets it aside (Showdown reads `target.item`).
    /// Returns whether it held one.
    /// Returns whether it held one; `identified` whether the line lets everyone tell whose it is
    /// (`TraceEvent::ItemRevealed`).
    pub fn reveal_item(&mut self, holder: PokemonId, by: PokemonId, identified: bool) -> bool {
        let Some(item) = self.state.pokemon(holder).and_then(|p| p.held_item) else {
            return false;
        };
        self.trace.push(TraceEvent::ItemRevealed {
            holder,
            item,
            by,
            identified,
        });
        true
    }

    pub fn item_activated(&mut self, holder: PokemonId, item: ItemId) {
        self.trace.push(TraceEvent::ItemActivated { holder, item });
    }

    pub fn ability_activated(&mut self, source: PokemonId, ability: AbilityId) {
        self.trace
            .push(TraceEvent::AbilityActivated { source, ability });
    }

    /// The ability is at work, shown to no one but its holder's side (`AbilityAtWork`).
    pub fn ability_at_work(&mut self, holder: PokemonId, ability: AbilityId) {
        self.trace
            .push(TraceEvent::AbilityAtWork { holder, ability });
    }

    /// Forewarn named `foe`'s `move_id` (`MoveForewarned`).
    pub fn forewarned(
        &mut self,
        holder: PokemonId,
        foe: PokemonId,
        move_id: MoveId,
        identified: bool,
    ) {
        self.trace.push(TraceEvent::MoveForewarned {
            holder,
            foe,
            move_id,
            identified,
        });
    }
}
