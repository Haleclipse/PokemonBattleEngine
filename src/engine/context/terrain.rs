//! `BattleContext`'s terrain reactions (DESIGN.md §23.2): Showdown's `TerrainChange` event,
//! which every active Pokémon sees when a terrain starts or ends — Quark Drive.

use crate::ids::{AbilityId, PokemonId, SideId};
use crate::state::{TerrainKind, VolatileConditionKind};

use super::BattleContext;
use crate::engine::TraceEvent;

impl<'a> BattleContext<'a> {
    /// Showdown `field.clearTerrain()` (Defog): the terrain ends at once, and every active
    /// Pokémon sees it. False when there was none.
    pub fn clear_terrain(&mut self) -> bool {
        let Some(terrain) = self.state.field.terrain.take() else {
            return false;
        };
        self.trace.push(TraceEvent::TerrainChanged {
            from: Some(terrain.kind),
            to: None,
        });
        self.terrain_changed();
        true
    }

    /// Showdown `eachEvent('TerrainChange')`: every active Pokémon's ability, then its item,
    /// reacts, fastest first (the order only shows in messages).
    pub fn terrain_changed(&mut self) {
        let mut active: Vec<(PokemonId, u32)> = [SideId::P1, SideId::P2]
            .into_iter()
            .flat_map(|side| self.state.side(side).active.iter().flatten().copied())
            .map(|pokemon| {
                let speed =
                    crate::engine::stats::action_speed(self.state, self.rules, pokemon, None);
                (pokemon, speed)
            })
            .collect();
        active.sort_by_key(|(_, speed)| std::cmp::Reverse(*speed));
        for (pokemon, _) in active {
            self.quark_drive(pokemon);
            self.mimicry(pokemon);
            // A terrain seed (`onTerrainChange`).
            if self.held_item(pokemon).is_some_and(|(_, behavior)| {
                matches!(
                    behavior.used_on_field,
                    Some(crate::effects::items::UsedOnField {
                        when: crate::effects::items::FieldItemWhen::Terrain(_),
                        ..
                    })
                )
            }) {
                crate::effects::items::use_if_the_field_has_it(self, pokemon);
            }
        }
    }

    /// Mimicry (`onStart`, `onTerrainChange`, §23.6): the holder's type is the terrain's —
    /// Electric, Grass, Fairy, Psychic — and its own types again with none (`setType`, which
    /// refuses Arceus, Silvally and a Terastallized holder, R-M8-156).
    pub fn mimicry(&mut self, pokemon: PokemonId) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        if holder.is_fainted()
            || !crate::engine::ability_handler(self.state, self.rules, holder, |b| b.mimicry)
            || self.set_type_refused(pokemon)
        {
            return;
        }
        let types = match self.state.field.terrain.map(|terrain| terrain.kind) {
            Some(TerrainKind::Electric) => {
                crate::state::PokemonTypes::mono(crate::dex::PokemonType::Electric)
            }
            Some(TerrainKind::Grassy) => {
                crate::state::PokemonTypes::mono(crate::dex::PokemonType::Grass)
            }
            Some(TerrainKind::Misty) => {
                crate::state::PokemonTypes::mono(crate::dex::PokemonType::Fairy)
            }
            Some(TerrainKind::Psychic) => {
                crate::state::PokemonTypes::mono(crate::dex::PokemonType::Psychic)
            }
            None => holder.base_types(),
        };
        let ability = crate::engine::raw_ability(holder);
        if self
            .state
            .pokemon_mut(pokemon)
            .is_some_and(|p| p.change_types(types))
        {
            self.trace.push(TraceEvent::TypesChanged {
                pokemon,
                types,
                source: crate::engine::EffectSource::Ability { pokemon, ability },
            });
        }
    }

    /// Quark Drive (`onStart`, `onTerrainChange`): in Electric Terrain the holder gets the
    /// `quarkdrive` volatile, which picks its best stat as it starts (`getBestStat(false,
    /// true)`); out of it, the volatile ends.
    pub fn quark_drive(&mut self, pokemon: PokemonId) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        if holder.is_fainted()
            || !crate::engine::ability_handler(self.state, self.rules, holder, |b| b.quark_drive)
        {
            return;
        }
        if self.state.field.terrain.map(|t| t.kind) != Some(TerrainKind::Electric) {
            // Not the volatile Booster Energy gave (`fromBooster`).
            if !holder.volatiles.paradox_boost_from_booster() {
                self.end_volatile(pokemon, VolatileConditionKind::QuarkDrive);
            }
            return;
        }
        let best = crate::engine::stats::best_stat(self.state, self.rules, pokemon);
        if self.state.pokemon_mut(pokemon).is_some_and(|p| {
            p.volatiles
                .set_paradox_boost(VolatileConditionKind::QuarkDrive, best, false)
        }) {
            self.ability_activated(pokemon, AbilityId::QUARK_DRIVE);
            self.trace.push(TraceEvent::VolatileStarted {
                target: pokemon,
                kind: VolatileConditionKind::QuarkDrive,
            });
        }
    }
}
