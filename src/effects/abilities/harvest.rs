//! Harvest (Gen 5+; Showdown `onResidual` 28.2): in the sun (`field.isWeather`: Air Lock and
//! Cloud Nine hide it, a Utility Umbrella does not) the berry the holder last ate comes back;
//! out of it a 1-in-2 chance is rolled at every turn's end, whether or not there is a berry to
//! bring back (`BattleContext::harvest`).

use crate::engine::BattleContext;
use crate::ids::PokemonId;
use crate::rules::Fraction;
use crate::state::{RandomRequestKind, WeatherKind};

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.field_weather() == Some(WeatherKind::Sun)
        || ctx.chance(RandomRequestKind::Harvest, Fraction::new(1, 2))
    {
        ctx.harvest(holder);
    }
}
