//! Field conditions (DESIGN.md §30): the weather, the terrain, the pseudo-weathers (Trick
//! Room, Gravity …) — their kinds and values, and the field's state and its expiry.

use crate::ids::PokemonId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum FieldConditionKind {
    TrickRoom,
    EchoedVoice,
    /// Gravity (Gen 4+): every Pokémon grounded, accuracy raised, the airborne moves refused.
    Gravity,
    /// Wonder Room (Gen 5+): every Pokémon's stored Defense and Special Defense trade places.
    WonderRoom,
    /// Ion Deluge (Gen 6-7, and Plasma Fists): Normal moves are Electric for the rest of the
    /// turn.
    IonDeluge,
    /// Gen 6-7 Mud Sport / Water Sport: Electric / Fire moves weakened.
    MudSport,
    WaterSport,
    /// Fairy Lock (Gen 6+): no Pokémon can switch out the next turn.
    FairyLock,
    /// Magic Room (Gen 5+): no Pokémon's item has an effect (`engine::held_item`). Its
    /// duration counts down in the residual (`onFieldResidualOrder` 27.6, `ResidualRules::durations`).
    MagicRoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCondition {
    TrickRoom {
        source: PokemonId,
        turns_remaining: u8,
    },
    /// Echoed Voice's field condition (Showdown `echoedvoice`, duration 2): its `multiplier`
    /// grows by one, up to 5, each turn the move is used again before it runs out.
    EchoedVoice { multiplier: u8, turns_remaining: u8 },
    /// A pseudo-weather that only lasts a number of turns (Showdown `duration`, counted down
    /// at the end of the turn): Gravity, Wonder Room, Ion Deluge, Mud / Water Sport, Fairy
    /// Lock.
    Timed {
        kind: FieldConditionKind,
        turns_remaining: u8,
    },
}

impl FieldCondition {
    pub const fn kind(self) -> FieldConditionKind {
        match self {
            Self::TrickRoom { .. } => FieldConditionKind::TrickRoom,
            Self::EchoedVoice { .. } => FieldConditionKind::EchoedVoice,
            Self::Timed { kind, .. } => kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum WeatherKind {
    Sun,
    Rain,
    Sandstorm,
    Snow,
    /// Delta Stream's strong winds (Showdown `deltastream`), always a strong weather.
    StrongWinds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeatherState {
    pub kind: WeatherKind,
    pub source: PokemonId,
    pub turns_remaining: u8,
    /// Showdown's strong weathers (DESIGN.md §23.5): Desolate Land's extremely harsh sun
    /// (`kind` Sun), Primordial Sea's heavy rain (Rain), Delta Stream's winds. Everything that
    /// reads the sun or the rain reads them too; they never run out, stop the opposite type's
    /// attacks and keep every other weather from starting.
    pub strong: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum TerrainKind {
    Electric,
    Grassy,
    Psychic,
    Misty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainState {
    pub kind: TerrainKind,
    pub source: PokemonId,
    pub turns_remaining: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldState {
    conditions: Vec<FieldCondition>,
    pub weather: Option<WeatherState>,
    pub terrain: Option<TerrainState>,
}

impl FieldState {
    pub fn conditions(&self) -> &[FieldCondition] {
        &self.conditions
    }

    pub fn contains(&self, kind: FieldConditionKind) -> bool {
        self.conditions.iter().any(|entry| entry.kind() == kind)
    }

    /// Trick Room toggles: using it while active removes it.
    /// Returns `true` when the condition becomes active, `false` when it is removed.
    pub fn toggle_trick_room(&mut self, source: PokemonId, turns: u8) -> bool {
        if let Some(index) = self
            .conditions
            .iter()
            .position(|entry| entry.kind() == FieldConditionKind::TrickRoom)
        {
            self.conditions.remove(index);
            return false;
        }

        self.conditions.push(FieldCondition::TrickRoom {
            source,
            turns_remaining: turns.max(1),
        });
        true
    }

    pub fn trick_room_turns(&self) -> Option<u8> {
        self.conditions.iter().find_map(|entry| match entry {
            FieldCondition::TrickRoom {
                turns_remaining, ..
            } => Some(*turns_remaining),
            _ => None,
        })
    }

    /// Echoed Voice's `onTryMove` (`addPseudoWeather('echoedvoice')`): starts the condition at
    /// multiplier 1 for two turns, or — used again once a turn has passed (`onFieldRestart`
    /// with its duration no longer 2) — raises the multiplier, up to 5, for two more turns.
    /// Returns whether the condition started.
    pub fn echo(&mut self) -> bool {
        for entry in &mut self.conditions {
            if let FieldCondition::EchoedVoice {
                multiplier,
                turns_remaining,
            } = entry
            {
                if *turns_remaining != 2 {
                    *turns_remaining = 2;
                    *multiplier = (*multiplier + 1).min(5);
                }
                return false;
            }
        }
        self.conditions.push(FieldCondition::EchoedVoice {
            multiplier: 1,
            turns_remaining: 2,
        });
        true
    }

    /// Starts a pseudo-weather for `turns` (`FieldCondition::Timed`); false when it is already
    /// up (Showdown `addPseudoWeather` without `onFieldRestart`).
    pub fn start_timed(&mut self, kind: FieldConditionKind, turns: u8) -> bool {
        if self.contains(kind) {
            return false;
        }
        self.conditions.push(FieldCondition::Timed {
            kind,
            turns_remaining: turns.max(1),
        });
        true
    }

    /// Ends a pseudo-weather; false when it was not up.
    pub fn remove(&mut self, kind: FieldConditionKind) -> bool {
        let before = self.conditions.len();
        self.conditions.retain(|entry| entry.kind() != kind);
        self.conditions.len() != before
    }

    /// Echoed Voice's power multiplier while its condition lasts.
    pub fn echoed_voice_multiplier(&self) -> Option<u8> {
        self.conditions.iter().find_map(|entry| match entry {
            FieldCondition::EchoedVoice { multiplier, .. } => Some(*multiplier),
            _ => None,
        })
    }

    /// `turns` 0 means the weather never runs out (a weather ability before Gen 6).
    pub fn set_weather(
        &mut self,
        kind: WeatherKind,
        source: PokemonId,
        turns: u8,
    ) -> Option<WeatherState> {
        let previous = self.weather;
        self.weather = Some(WeatherState {
            kind,
            source,
            turns_remaining: turns,
            strong: false,
        });
        previous
    }

    /// A strong weather (Desolate Land, Primordial Sea, Delta Stream): no duration.
    pub fn set_strong_weather(
        &mut self,
        kind: WeatherKind,
        source: PokemonId,
    ) -> Option<WeatherState> {
        let previous = self.weather;
        self.weather = Some(WeatherState {
            kind,
            source,
            turns_remaining: 0,
            strong: true,
        });
        previous
    }

    /// The weather's residual tick (Showdown `duration--`, ending at 0): `Some(kind)` when it
    /// just ended, `None` while it goes on or when there is none. Permanent weather never ticks.
    pub fn tick_weather(&mut self) -> Option<WeatherKind> {
        match self.weather {
            Some(state) if state.turns_remaining == 0 => None,
            Some(state) if state.turns_remaining == 1 => {
                self.weather = None;
                Some(state.kind)
            }
            Some(mut state) => {
                state.turns_remaining -= 1;
                self.weather = Some(state);
                None
            }
            None => None,
        }
    }

    pub fn set_terrain(
        &mut self,
        kind: TerrainKind,
        source: PokemonId,
        turns: u8,
    ) -> Option<TerrainState> {
        let previous = self.terrain;
        self.terrain = Some(TerrainState {
            kind,
            source,
            turns_remaining: turns.max(1),
        });
        previous
    }

    /// One residual count off the field condition `kind`: `Some(true)` when it ran out and is
    /// gone, `Some(false)` while it goes on, `None` when the field does not have it.
    pub fn count_down_condition(&mut self, kind: FieldConditionKind) -> Option<bool> {
        let index = self
            .conditions
            .iter()
            .position(|entry| entry.kind() == kind)?;
        let (FieldCondition::TrickRoom {
            turns_remaining, ..
        }
        | FieldCondition::EchoedVoice {
            turns_remaining, ..
        }
        | FieldCondition::Timed {
            turns_remaining, ..
        }) = &mut self.conditions[index];
        if *turns_remaining <= 1 {
            self.conditions.remove(index);
            Some(true)
        } else {
            *turns_remaining -= 1;
            Some(false)
        }
    }

    /// One residual count off the terrain: `Some((kind, true))` when it ran out and is gone,
    /// `Some((kind, false))` while it goes on, `None` when there is none.
    pub fn count_down_terrain(&mut self) -> Option<(TerrainKind, bool)> {
        let mut state = self.terrain?;
        if state.turns_remaining <= 1 {
            self.terrain = None;
            Some((state.kind, true))
        } else {
            state.turns_remaining -= 1;
            self.terrain = Some(state);
            Some((state.kind, false))
        }
    }

    pub fn tick_end_turn(&mut self) -> FieldExpiry {
        let mut conditions = Vec::new();
        self.conditions.retain_mut(|entry| {
            let kind = entry.kind();
            let (FieldCondition::TrickRoom {
                turns_remaining, ..
            }
            | FieldCondition::EchoedVoice {
                turns_remaining, ..
            }
            | FieldCondition::Timed {
                turns_remaining, ..
            }) = entry;
            if *turns_remaining <= 1 {
                conditions.push(kind);
                false
            } else {
                *turns_remaining -= 1;
                true
            }
        });

        // Weather ticks in the residual schedule (`tick_weather`), not here.
        let terrain = match self.terrain {
            Some(state) if state.turns_remaining <= 1 => {
                self.terrain = None;
                Some(state.kind)
            }
            Some(mut state) => {
                state.turns_remaining -= 1;
                self.terrain = Some(state);
                None
            }
            None => None,
        };

        FieldExpiry {
            conditions,
            terrain,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldExpiry {
    pub conditions: Vec<FieldConditionKind>,
    pub terrain: Option<TerrainKind>,
}
