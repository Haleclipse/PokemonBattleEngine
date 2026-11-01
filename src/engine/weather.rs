//! The weather as the field and a Pokémon see it (DESIGN.md §23.1): Air Lock and Cloud Nine on
//! the field suppress it (Showdown `suppressingWeather`: every weather handler but the upkeep is
//! skipped, `isWeather` answers no); Utility Umbrella hides sun and rain from its holder
//! (`Pokemon.effectiveWeather`).

use crate::ids::{PokemonId, SideId};
use crate::rules::ResolvedRules;
use crate::state::{BattleState, WeatherKind};

/// Whether an active Pokémon's ability suppresses the weather (Air Lock, Cloud Nine).
pub(crate) fn suppressed(state: &BattleState, rules: &ResolvedRules) -> bool {
    suppressed_except(state, rules, None)
}

/// The same, `leaving` no longer counting (its ability ending as it switches out: Showdown's
/// `abilityState.ending`).
pub(crate) fn suppressed_except(
    state: &BattleState,
    rules: &ResolvedRules,
    leaving: Option<PokemonId>,
) -> bool {
    [SideId::P1, SideId::P2].into_iter().any(|side| {
        state.side(side).active.iter().flatten().any(|id| {
            Some(*id) != leaving
                && state.pokemon(*id).is_some_and(|p| {
                    !p.is_fainted()
                        && super::ability_handler(state, rules, p, |b| b.suppresses_weather)
                })
        })
    })
}

/// Showdown `field.effectiveWeather()` (`isWeather`): the weather, unless suppressed.
pub fn field_weather(state: &BattleState, rules: &ResolvedRules) -> Option<WeatherKind> {
    let weather = state.field.weather?.kind;
    (!suppressed(state, rules)).then_some(weather)
}

/// Showdown `pokemon.effectiveWeather()`: the field's, but no sun or rain for a Utility
/// Umbrella holder.
pub fn weather_for(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: PokemonId,
) -> Option<WeatherKind> {
    let weather = field_weather(state, rules)?;
    let umbrella = super::held_item(state, rules, pokemon).is_some_and(|(_, item)| item.umbrella);
    match weather {
        WeatherKind::Sun | WeatherKind::Rain if umbrella => None,
        other => Some(other),
    }
}

/// A strong weather's `onTryMove` (priority 1; not while suppressed): Desolate Land's extremely
/// harsh sun stops Water attacks, Primordial Sea's heavy rain Fire ones (DESIGN.md §23.5).
pub fn strong_weather_stops(
    state: &BattleState,
    rules: &ResolvedRules,
    move_def: &crate::dex::ResolvedMove,
) -> Option<WeatherKind> {
    let weather = state.field.weather.filter(|weather| weather.strong)?;
    if suppressed(state, rules) || move_def.category == crate::dex::MoveCategory::Status {
        return None;
    }
    let stopped = match weather.kind {
        WeatherKind::Sun => crate::dex::PokemonType::Water,
        WeatherKind::Rain => crate::dex::PokemonType::Fire,
        _ => return None,
    };
    (move_def.move_type == stopped).then_some(weather.kind)
}

/// Delta Stream's `onEffectiveness` (priority −1; not while suppressed): against a Flying
/// type, a damaging move's super effectiveness on that type is neutral.
pub(crate) fn strong_winds_blow(state: &BattleState, rules: &ResolvedRules) -> bool {
    field_weather(state, rules) == Some(WeatherKind::StrongWinds)
}
