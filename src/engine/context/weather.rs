//! `BattleContext`'s weather reactions (DESIGN.md §23.1): Showdown's `WeatherChange` event,
//! which every active Pokémon sees when the weather starts, ends, or stops or resumes counting
//! (Air Lock, Cloud Nine) — Protosynthesis; and the strong weathers of Desolate Land,
//! Primordial Sea and Delta Stream (§23.5).

use crate::ids::{AbilityId, PokemonId, SideId};
use crate::state::{VolatileConditionKind, WeatherKind};

use super::BattleContext;
use crate::engine::TraceEvent;

impl<'a> BattleContext<'a> {
    /// The weather as `pokemon` sees it (`engine::weather_for`: suppression, Utility Umbrella).
    pub fn weather_for(&self, pokemon: PokemonId) -> Option<WeatherKind> {
        crate::engine::weather_for(self.state, self.rules, pokemon)
    }

    /// The field's weather (Showdown `field.isWeather`: `engine::field_weather`, unless
    /// suppressed; no Utility Umbrella).
    pub fn field_weather(&self) -> Option<WeatherKind> {
        crate::engine::field_weather(self.state, self.rules)
    }

    /// Showdown `field.clearWeather()` (Defog, Teraform Zero): the weather ends at once, and
    /// every active Pokémon sees it. False when there was none.
    pub fn clear_weather(&mut self) -> bool {
        let Some(weather) = self.state.field.weather.take() else {
            return false;
        };
        self.trace.push(TraceEvent::WeatherChanged {
            from: Some(weather.kind),
            to: None,
        });
        self.weather_changed();
        true
    }

    /// Showdown `eachEvent('WeatherChange')`: every active Pokémon's ability reacts, fastest
    /// first (the order only shows in messages).
    pub fn weather_changed(&mut self) {
        self.weather_changed_leaving(None);
    }

    /// The same as `leaving`'s ability ends (Air Lock / Cloud Nine's `onEnd` as it switches
    /// out): it no longer suppresses the weather.
    pub fn weather_changed_leaving(&mut self, leaving: Option<PokemonId>) {
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
            if Some(pokemon) != leaving {
                self.protosynthesis_seeing(pokemon, leaving);
                self.forecast_seeing(pokemon, leaving);
                self.flower_gift_seeing(pokemon, leaving);
                // Ice Face answers a fresh change only: the weather resuming because Air
                // Lock / Cloud Nine ended is not one (`sourceEffect.suppressWeather`).
                if leaving.is_none()
                    && let Some(handler) = self.state.pokemon(pokemon).and_then(|holder| {
                        crate::engine::ability_handler(self.state, self.rules, holder, |b| {
                            b.weather_change
                        })
                    })
                {
                    handler(self, pokemon);
                }
            }
        }
    }

    /// Forecast (`onStart`, `onWeatherChange`, §23.6): Castform takes the forme of the weather
    /// it sees (`pokemon.effectiveWeather()`: none while Air Lock / Cloud Nine is out, no sun or
    /// rain under a Utility Umbrella; Desolate Land and Primordial Sea count) — Sunny (Fire),
    /// Rainy (Water), Snowy (Ice), its own types otherwise.
    pub fn forecast(&mut self, pokemon: PokemonId) {
        self.forecast_seeing(pokemon, None);
    }

    fn forecast_seeing(&mut self, pokemon: PokemonId, leaving: Option<PokemonId>) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        let Some(forecast) =
            crate::engine::ability_handler(self.state, self.rules, holder, |b| b.forecast)
        else {
            return;
        };
        // Its own species, not transformed (`baseSpecies.baseSpecies !== 'Castform' ||
        // transformed`).
        if holder.is_fainted()
            || holder.species != Some(forecast.holder)
            || holder.transform.is_some()
        {
            return;
        }
        let forme = match self.effective_weather(pokemon, leaving) {
            Some(WeatherKind::Sun) => forecast.sun,
            Some(WeatherKind::Rain) => forecast.rain,
            Some(WeatherKind::Snow) => forecast.snow,
            _ => forecast.holder,
        };
        let ability = crate::engine::raw_ability(holder);
        self.forme_change(
            pokemon,
            forme,
            crate::engine::FormeChangeKind::InBattle,
            crate::engine::EffectSource::Ability { pokemon, ability },
        );
    }

    /// Showdown `pokemon.effectiveWeather()` as `leaving`'s ability ends: none while Air Lock /
    /// Cloud Nine is out, no sun or rain under a Utility Umbrella.
    fn effective_weather(
        &self,
        pokemon: PokemonId,
        leaving: Option<PokemonId>,
    ) -> Option<WeatherKind> {
        let umbrella = crate::engine::held_item(self.state, self.rules, pokemon)
            .is_some_and(|(_, item)| item.umbrella);
        self.state
            .field
            .weather
            .map(|weather| weather.kind)
            .filter(|_| !crate::engine::weather::suppressed_except(self.state, self.rules, leaving))
            .filter(|kind| !(umbrella && matches!(kind, WeatherKind::Sun | WeatherKind::Rain)))
    }

    /// Flower Gift (`onStart`, `onWeatherChange`, DESIGN.md §43.2): a Cherrim on the field, not
    /// transformed, is in its Sunshine forme while it sees the sun (Desolate Land's too), its
    /// Overcast one otherwise.
    pub fn flower_gift(&mut self, pokemon: PokemonId) {
        self.flower_gift_seeing(pokemon, None);
    }

    fn flower_gift_seeing(&mut self, pokemon: PokemonId, leaving: Option<PokemonId>) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        if !crate::engine::ability_handler(self.state, self.rules, holder, |b| b.flower_gift)
            || holder.is_fainted()
            || holder.transform.is_some()
            || !holder
                .species
                .is_some_and(|own| own.same_species(crate::ids::SpeciesId::CHERRIM))
            || self
                .state
                .active_position(pokemon, &self.rules.topology)
                .is_none()
        {
            return;
        }
        let forme = if self.effective_weather(pokemon, leaving) == Some(WeatherKind::Sun) {
            crate::ids::SpeciesId::CHERRIM_SUNSHINE
        } else {
            crate::ids::SpeciesId::CHERRIM
        };
        let ability = crate::engine::raw_ability(holder);
        self.forme_change(
            pokemon,
            forme,
            crate::engine::FormeChangeKind::InBattle,
            crate::engine::EffectSource::Ability { pokemon, ability },
        );
    }

    /// Desolate Land / Primordial Sea / Delta Stream (`onStart` → `setWeather`): the strong
    /// weather replaces any other, a strong one too; the same one already up changes nothing
    /// (`setWeather` from an ability, Gen 6+).
    pub fn set_strong_weather(
        &mut self,
        kind: WeatherKind,
        source: PokemonId,
        ability: AbilityId,
    ) -> bool {
        if self
            .state
            .field
            .weather
            .is_some_and(|weather| weather.strong && weather.kind == kind)
        {
            return false;
        }
        self.ability_activated(source, ability);
        let previous = self
            .state
            .field
            .set_strong_weather(kind, source)
            .map(|weather| weather.kind);
        self.trace.push(TraceEvent::WeatherChanged {
            from: previous,
            to: Some(kind),
        });
        self.weather_changed();
        true
    }

    /// The strong weathers' holders' `onAnySetWeather`: while one is up, no other weather starts
    /// (a weather move fails, a weather ability does not activate).
    pub fn strong_weather_up(&self) -> bool {
        self.state
            .field
            .weather
            .is_some_and(|weather| weather.strong)
    }

    /// A strong weather's `onEnd` as `leaving` leaves the field (switched, dragged out or down):
    /// the weather it set passes to another active Pokémon with the same ability, or ends — only
    /// when it is that ability's strong weather and `leaving` its source: a rain the holder's own
    /// Rain Dance started while Gastro Acid kept its ability quiet stays (the user's decision,
    /// issues.md I198, R-M8-170; Showdown's `onEnd` asks only whose the weather is, which the
    /// oracle patches).
    pub fn strong_weather_holder_left(&mut self, leaving: PokemonId) {
        // The ability that ends (its own id, `hasAbility('deltastream')`): the holder's, even
        // when Gastro Acid is what ends it.
        let Some(ability) = self.state.pokemon(leaving).map(crate::engine::raw_ability) else {
            return;
        };
        let strong = crate::engine::ability_behavior(self.rules, ability).strong_weather;
        let Some(weather) = self.state.field.weather.filter(|weather| {
            weather.source == leaving && weather.strong && Some(weather.kind) == strong
        }) else {
            return;
        };
        let heir = self.active_pokemon().into_iter().find(|other| {
            *other != leaving
                && self.state.pokemon(*other).is_some_and(|pokemon| {
                    crate::engine::has_ability(self.state, self.rules, pokemon, ability)
                        && pokemon.hp > 0
                })
        });
        match heir {
            Some(heir) => {
                if let Some(current) = self.state.field.weather.as_mut() {
                    current.source = heir;
                }
            }
            None => {
                self.state.field.weather = None;
                self.trace.push(TraceEvent::WeatherChanged {
                    from: Some(weather.kind),
                    to: None,
                });
                self.weather_changed_leaving(Some(leaving));
            }
        }
    }

    /// Protosynthesis (`onStart`, `onWeatherChange`): in the sun (`isWeather`: suppression
    /// counts, Utility Umbrella does not) the holder gets the `protosynthesis` volatile, which
    /// picks its best stat as it starts; out of it, the volatile ends.
    pub fn protosynthesis(&mut self, pokemon: PokemonId) {
        self.protosynthesis_seeing(pokemon, None);
    }

    /// Protosynthesis's `isWeather('sunnyday')`: the sun, not Desolate Land's, and not
    /// suppressed (but for `leaving`'s Cloud Nine).
    fn protosynthesis_sun(&self, leaving: Option<PokemonId>) -> bool {
        self.state
            .field
            .weather
            .is_some_and(|w| w.kind == WeatherKind::Sun && !w.strong)
            && !crate::engine::weather::suppressed_except(self.state, self.rules, leaving)
    }

    /// Booster Energy (`onUpdate`, once its `onStart` ran since the holder came in): a
    /// Protosynthesis holder out of the sun, or a Quark Drive holder out of Electric Terrain, uses
    /// it up (`useItem`) and gets the ability's volatile from it (`fromBooster`: the weather or
    /// terrain ending later leaves it be). Its `pokemon.transformed` exception waits for
    /// Transform.
    pub fn use_booster_energy(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        if pokemon.is_fainted() || !pokemon.item_state.started {
            return;
        }
        let handlers = crate::engine::ability_handlers(self.state, self.rules, pokemon);
        let (kind, ability) = if handlers.protosynthesis && !self.protosynthesis_sun(None) {
            (
                VolatileConditionKind::Protosynthesis,
                AbilityId::PROTOSYNTHESIS,
            )
        } else if handlers.quark_drive
            && self.state.field.terrain.map(|t| t.kind) != Some(crate::state::TerrainKind::Electric)
        {
            (VolatileConditionKind::QuarkDrive, AbilityId::QUARK_DRIVE)
        } else {
            return;
        };
        if self.consume_item(holder).is_none() {
            return;
        }
        let best = crate::engine::stats::best_stat(self.state, self.rules, holder);
        if self
            .state
            .pokemon_mut(holder)
            .is_some_and(|p| p.volatiles.set_paradox_boost(kind, best, true))
        {
            self.ability_activated(holder, ability);
            self.trace.push(TraceEvent::VolatileStarted {
                target: holder,
                kind,
            });
        }
    }

    fn protosynthesis_seeing(&mut self, pokemon: PokemonId, leaving: Option<PokemonId>) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        if holder.is_fainted()
            || !crate::engine::ability_handler(self.state, self.rules, holder, |b| b.protosynthesis)
        {
            return;
        }
        if !self.protosynthesis_sun(leaving) {
            // Not the volatile Booster Energy gave (`fromBooster`).
            if !holder.volatiles.paradox_boost_from_booster() {
                self.end_volatile(pokemon, VolatileConditionKind::Protosynthesis);
            }
            return;
        }
        let best = crate::engine::stats::best_stat(self.state, self.rules, pokemon);
        if self.state.pokemon_mut(pokemon).is_some_and(|p| {
            p.volatiles
                .set_paradox_boost(VolatileConditionKind::Protosynthesis, best, false)
        }) {
            self.ability_activated(pokemon, AbilityId::PROTOSYNTHESIS);
            self.trace.push(TraceEvent::VolatileStarted {
                target: pokemon,
                kind: VolatileConditionKind::Protosynthesis,
            });
        }
    }
}
