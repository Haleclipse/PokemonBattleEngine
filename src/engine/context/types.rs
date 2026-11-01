//! Type changes (DESIGN.md §41.4, K.5): Showdown `setType` and `addType`. The reading side is
//! `engine::types_of`.

use super::BattleContext;
use crate::dex::PokemonType;
use crate::engine::{EffectSource, TraceEvent};
use crate::ids::PokemonId;
use crate::state::PokemonTypes;

impl BattleContext<'_> {
    /// `pokemon`'s types as the battle reads them (`engine::types_of`).
    pub fn types(&self, pokemon: PokemonId) -> Option<crate::engine::TypeSet> {
        self.state
            .pokemon(pokemon)
            .map(|holder| crate::engine::types_of(self.state, self.rules, holder))
    }

    /// `engine::conversion_candidates` for `user`. Needs the dex.
    pub fn conversion_candidates(&self, user: PokemonId) -> Vec<PokemonType> {
        self.dex.map_or_else(Vec::new, |dex| {
            crate::engine::conversion_candidates(self.state, self.rules, dex, user)
        })
    }

    /// `engine::conversion2_candidates` for `user` against `target` (empty without a last
    /// move). Needs the dex.
    pub fn conversion2_candidates(&self, user: PokemonId, target: PokemonId) -> Vec<PokemonType> {
        self.dex
            .and_then(|dex| {
                crate::engine::conversion2_candidates(self.state, self.rules, dex, user, target)
            })
            .unwrap_or_default()
    }

    /// The type of `pokemon`'s first move slot (Gen 6+ Conversion). Needs the dex.
    pub fn first_slot_type(&self, pokemon: PokemonId) -> Option<PokemonType> {
        let slot = self.state.pokemon(pokemon)?.move_slots().first()?;
        Some(self.dex?.move_by_id(slot.move_id)?.move_type)
    }

    /// Whether the generation keeps `pokemon`'s types as they are (`TypeRules::lock`): Gen 4 a
    /// Multitype holder, Gen 5+ Arceus and Silvally.
    pub fn types_locked(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_none_or(|holder| crate::engine::types_locked(self.state, self.rules, holder))
    }

    /// Whether Showdown's `setType` (not `enforce`d) refuses `pokemon`: its types are locked
    /// (`types_locked`), or it is Terastallized — only a forme change (`enforce`) changes those
    /// (DESIGN.md §44.6).
    pub fn set_type_refused(&self, pokemon: PokemonId) -> bool {
        self.types_locked(pokemon)
            || self
                .state
                .pokemon(pokemon)
                .is_some_and(|pokemon| pokemon.terastallized.is_some())
    }

    /// Showdown `setType` (not `enforce`d): `pokemon`'s types become `types` and an added type
    /// goes, unless `set_type_refused`. Returns whether it took.
    pub fn set_type(
        &mut self,
        pokemon: PokemonId,
        types: PokemonTypes,
        source: EffectSource,
    ) -> bool {
        if self.set_type_refused(pokemon) {
            return false;
        }
        let Some(holder) = self.state.pokemon_mut(pokemon) else {
            return false;
        };
        holder.set_types(types);
        self.trace.push(TraceEvent::TypesChanged {
            pokemon,
            types,
            source,
        });
        true
    }

    /// The "Curse glitch" of Trick-or-Treat (and Color Change): in a doubles battle, a Pokémon in
    /// its side's second slot whose move of this turn is Curse — chosen before it was a Ghost —
    /// has that action aim at the side's first slot (`action.targetLoc = -1`). Only where the
    /// Ghost's Curse goes where it is aimed (`target: normal`: Gen 6, Gen 9 — whose Curse then
    /// redirects an aim at an ally) does the aim matter: Gen 7-8's is `randomNormal`. Needs the
    /// dex.
    pub fn curse_glitch(&mut self, pokemon: PokemonId) {
        let aimed = self
            .dex
            .and_then(|dex| dex.move_by_id(crate::ids::MoveId::CURSE))
            .is_some_and(|curse| curse.target == crate::rules::TargetPattern::SelectedAdjacent);
        let second_slot = aimed
            && self.rules.topology.active_slots_per_side() == 2
            && self
                .state
                .active_position(pokemon, &self.rules.topology)
                .and_then(|position| self.rules.topology.spec(position))
                .is_some_and(|spec| spec.slot == 1);
        if second_slot
            && self
                .state
                .will_move_with(pokemon)
                .is_some_and(|queued| queued.move_id == crate::ids::MoveId::CURSE)
            && !self.state.curse_glitched.contains(&pokemon)
        {
            self.state.curse_glitched.push(pokemon);
        }
    }

    /// Protean, Libero (`onPrepareHit`, after the move's own): the user takes its move's type as
    /// `engine::protean_type` says.
    pub fn protean(&mut self, user: PokemonId, move_type: PokemonType) {
        let Some(pokemon) = self.state.pokemon(user) else {
            return;
        };
        if crate::engine::protean_type(self.state, self.rules, user, move_type).is_none() {
            return;
        }
        let once = self.rules.conditions.type_change.protean_once_per_entry;
        let ability = crate::engine::raw_ability(pokemon);
        let source = EffectSource::Ability {
            pokemon: user,
            ability,
        };
        if self.set_type(user, PokemonTypes::mono(move_type), source)
            && once
            && let Some(pokemon) = self.state.pokemon_mut(user)
        {
            pokemon.ability_state.protean_used = true;
        }
    }

    /// Showdown `addType`: `pokemon`'s added type becomes `type_` (one at a time: it replaces
    /// the one before).
    pub fn add_type(&mut self, pokemon: PokemonId, type_: PokemonType, source: EffectSource) {
        // Showdown `addType`: nothing on a Terastallized Pokémon (`terastallize` cleared its
        // added type).
        if self
            .state
            .pokemon(pokemon)
            .is_some_and(|pokemon| pokemon.terastallized.is_some())
        {
            return;
        }
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.added_type = Some(type_);
            self.trace.push(TraceEvent::TypeAdded {
                pokemon,
                type_,
                source,
            });
        }
    }
}
