//! Weather-setting abilities (Showdown `onStart` → `field.setWeather`): Drought, Drizzle, Sand
//! Stream, Snow Warning. How long the weather lasts and whether it replaces the same weather
//! are `WeatherRules`, applied by `BattleContext::set_weather_by_ability`; the Groudon / Kyogre
//! orb exceptions are not modelled (the orbs are unsupported items).

use crate::engine::BattleContext;
use crate::ids::{AbilityId, PokemonId};
use crate::state::WeatherKind;

pub fn drought(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.set_weather_by_ability(WeatherKind::Sun, source, AbilityId::DROUGHT);
}

pub fn drizzle(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.set_weather_by_ability(WeatherKind::Rain, source, AbilityId::DRIZZLE);
}

pub fn sand_stream(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.set_weather_by_ability(WeatherKind::Sandstorm, source, AbilityId::SAND_STREAM);
}

/// Orichalcum Pulse sets the sun like Drought (its `-activate` is only a message); in the sun
/// already it is only announced (`else if (this.field.isWeather('sunnyday'))`, R-M10-77) — not
/// where Air Lock / Cloud Nine hide it (the effective weather) or a strong weather refused it
/// (its own `[fail]` line).
pub fn orichalcum_pulse(ctx: &mut BattleContext<'_>, source: PokemonId) {
    if !ctx.set_weather_by_ability(WeatherKind::Sun, source, AbilityId::ORICHALCUM_PULSE)
        && !ctx.strong_weather_up()
        && ctx.field_weather() == Some(WeatherKind::Sun)
    {
        ctx.ability_activated(source, AbilityId::ORICHALCUM_PULSE);
    }
}

pub fn snow_warning(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.set_weather_by_ability(WeatherKind::Snow, source, AbilityId::SNOW_WARNING);
}

/// Air Lock / Cloud Nine (`onSwitchIn` → `onStart`): the weather is suppressed from now on
/// (`engine::weather`); the `WeatherChange` listeners see it (Protosynthesis).
pub fn suppress_weather(ctx: &mut BattleContext<'_>, _source: PokemonId) {
    ctx.weather_changed();
}

/// Protosynthesis (`onStart`): its `WeatherChange` reaction, as it enters.
pub fn protosynthesis(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.protosynthesis(source);
}

/// Forecast's `onStart`: the forme of the weather Castform finds.
pub fn forecast(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.forecast(source);
}

/// Flower Gift's `onStart`: the forme of the weather Cherrim finds.
pub fn flower_gift(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.flower_gift(source);
}

/// Sand Spit (`onDamagingHit`): a hit that reached the holder starts a sandstorm, as a weather
/// ability does (`setWeather` with the ability as its source).
pub fn sand_spit(ctx: &mut BattleContext<'_>, hit: crate::engine::DamagingHit) {
    ctx.set_weather_by_ability(WeatherKind::Sandstorm, hit.holder, AbilityId::SAND_SPIT);
}

/// Gen 3 Rain Dish (its residual, 10.3) heals 1/16 of the holder's max HP in rain.
pub fn rain_dish(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.weather_for(holder) != Some(WeatherKind::Rain) {
        return;
    }
    let Some(max_hp) = ctx.base_max_hp(holder) else {
        return;
    };
    ctx.apply_heal(crate::engine::HealRequest {
        target: holder,
        amount: (max_hp / 16).max(1),
        source: crate::engine::EffectSource::Ability {
            pokemon: holder,
            ability: AbilityId::RAIN_DISH,
        },
        direct: false,
    });
}

/// Gen 5+ Hydration (its residual, 5.3) cures the holder's status in rain.
pub fn hydration(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.weather_for(holder) != Some(WeatherKind::Rain) || !ctx.has_status(holder) {
        return;
    }
    ctx.ability_activated(holder, AbilityId::HYDRATION);
    ctx.cure_status(
        holder,
        crate::engine::CureReason::Ability(AbilityId::HYDRATION),
    );
}
