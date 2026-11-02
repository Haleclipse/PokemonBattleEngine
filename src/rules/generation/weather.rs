//! A generation's weather (`WeatherRules`).

use super::*;

/// Weather (DESIGN.md §23.1): duration, who sets it for how long, what it damages and what it
/// shields. `WeatherKind::Snow` is hail before Gen 9 and snow from Gen 9.
/// A weather move used under the weather it sets: it fails (Gen 3+), or — Gen 2 — only
/// Sandstorm fails and Rain Dance / Sunny Day start the weather over for five turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameWeatherMove {
    Fails,
    OnlySandstormFails,
}

impl SameWeatherMove {
    pub fn fails(self, kind: crate::state::WeatherKind) -> bool {
        match self {
            Self::Fails => true,
            Self::OnlySandstormFails => kind == crate::state::WeatherKind::Sandstorm,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeatherRules {
    /// A weather-setting ability lasts forever before Gen 6 (Showdown `duration = 0`).
    pub ability_weather_permanent: bool,
    /// A weather move used under the same weather (Showdown `setWeather`).
    pub same_weather_move: SameWeatherMove,
    /// Sandstorm / hail chip: max HP divided by this, at least 1.
    pub damage_divisor: u16,
    pub sandstorm_immune_types: &'static [PokemonType],
    /// Hail damages (Gen 3-8); Gen 9 snow does not.
    pub hail_damages: bool,
    pub hail_immune_types: &'static [PokemonType],
    /// Gen 4+: a Rock type's Special Defense under sandstorm.
    pub sandstorm_rock_special_defense: Fraction,
    /// Gen 9: an Ice type's Defense under snow.
    pub snow_ice_defense: Fraction,
    /// Heat / Damp / Smooth / Icy Rock on the setter (Showdown `durationCallback`).
    pub rock_turns: u8,
    /// Flower Gift in the sun (DESIGN.md §23.6): Gen 5+ only a Cherrim's; Gen 4 anyone's, and
    /// once however many hold it (pokeplatinum `BattleSystem_CountAbility`).
    pub flower_gift: FlowerGiftRules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowerGiftRules {
    /// Gen 5+ (`effectState.target.baseSpecies.baseSpecies !== 'Cherrim'`): only a Cherrim.
    pub holder_must_be_cherrim: bool,
    /// Gen 4 (the cartridge counts holders): one boost for any number of holders; Gen 5+
    /// Showdown's handlers, one per holder.
    pub once_per_side: bool,
    /// Gen 4 (issues.md I70, pokeplatinum `BattleSystem_CalcMoveDamage`): the attacker's side
    /// is counted by the ability alone, so the attacker's Mold Breaker does not set an ally's
    /// Flower Gift aside (only the defender's side checks it); Showdown's handlers are breakable
    /// on both sides.
    pub attack_ignores_mold_breaker: bool,
}

/// The generation's weather.
pub(super) fn resolve(generation: Generation) -> WeatherRules {
    WeatherRules {
        ability_weather_permanent: generation <= Generation::Gen5,
        same_weather_move: if generation >= Generation::Gen3 {
            SameWeatherMove::Fails
        } else {
            SameWeatherMove::OnlySandstormFails
        },
        // Gen 2's sandstorm takes 1/8 (Showdown gen2 `onWeather`).
        damage_divisor: if generation <= Generation::Gen2 {
            8
        } else {
            16
        },
        sandstorm_immune_types: &[PokemonType::Rock, PokemonType::Ground, PokemonType::Steel],
        hail_damages: generation <= Generation::Gen8,
        hail_immune_types: &[PokemonType::Ice],
        sandstorm_rock_special_defense: if generation >= Generation::Gen4 {
            Fraction::new(3, 2)
        } else {
            Fraction::ONE
        },
        snow_ice_defense: if generation >= Generation::Gen9 {
            Fraction::new(3, 2)
        } else {
            Fraction::ONE
        },
        rock_turns: 8,
        flower_gift: FlowerGiftRules {
            holder_must_be_cherrim: generation >= Generation::Gen5,
            once_per_side: generation <= Generation::Gen4,
            attack_ignores_mold_breaker: generation <= Generation::Gen4,
        },
    }
}
