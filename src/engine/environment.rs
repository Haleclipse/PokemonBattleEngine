use crate::dex::{PokemonType, ResolvedMove};
use crate::ids::{MoveId, PokemonId};
use crate::rules::{Fraction, ResolvedRules};
use crate::state::{BattleState, FieldConditionKind, PokemonState, TerrainKind, WeatherKind};

/// Showdown `isGrounded` (DESIGN.md §41.1, K.0), the single entry for whether a Pokémon is on the
/// ground: Gravity, then Ingrain (Gen 4+), Smack Down, Iron Ball, a Flying type (Roost: through
/// `types_of`) and Levitate, then Magnet Rise, Telekinesis and Air Balloon (K.7b).
pub(crate) fn grounded(state: &BattleState, rules: &ResolvedRules, pokemon: &PokemonState) -> bool {
    grounded_negating(state, rules, pokemon, false)
}

/// The same with `isGrounded(negateImmunity)` (a Ring Target holder hit by a Ground move).
pub(crate) fn grounded_negating(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    negate_immunity: bool,
) -> bool {
    grounding(state, rules, pokemon, negate_immunity) == Grounding::Grounded
}

/// Showdown `isGrounded`'s three answers: on the ground (true), off it (false), or off it by
/// Levitate (null — a Ground move's `-immune` names the ability).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grounding {
    Grounded,
    Airborne,
    Levitating,
}

pub(crate) fn grounding(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    negate_immunity: bool,
) -> Grounding {
    use Grounding::{Airborne, Grounded, Levitating};
    if state.field.contains(FieldConditionKind::Gravity) {
        return Grounded;
    }
    if rules.conditions.ingrain_grounds
        && pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::Ingrain)
    {
        return Grounded;
    }
    if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::SmackDown)
    {
        return Grounded;
    }
    let item = super::held_item_id(state, rules, pokemon.id());
    if item == Some(crate::ids::ItemId::IRON_BALL) {
        return Grounded;
    }
    if !negate_immunity && super::has_type(state, rules, pokemon, PokemonType::Flying) {
        return Airborne;
    }
    // Levitate (`hasAbility('levitate') && !suppressingAbility(this)`): a move that ignores
    // abilities grounds it.
    if super::has_ability(state, rules, pokemon, crate::ids::AbilityId::LEVITATE)
        && !super::suppressing_ability(state, rules, pokemon)
    {
        return Levitating;
    }
    if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::MagnetRise)
        || pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::Telekinesis)
        || item == Some(crate::ids::ItemId::AIR_BALLOON)
    {
        Airborne
    } else {
        Grounded
    }
}

/// The moves Grassy Terrain weakens against a grounded target (its `onBasePower` names them).
const GRASSY_WEAKENED: [MoveId; 3] = [MoveId::EARTHQUAKE, MoveId::BULLDOZE, MoveId::MAGNITUDE];

/// The terrain's `onBasePower` (priority 6), 4096-based, or `None` (DESIGN.md §23.2): Electric,
/// Psychic and Grassy Terrain strengthen a grounded user's move of their type (Grassy's does not
/// ask whether the user is semi-invulnerable); Grassy Terrain halves Earthquake, Bulldoze and
/// Magnitude, Misty Terrain Dragon moves, against a grounded target that is not.
pub(crate) fn terrain_power_modifier(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    defender: PokemonId,
    move_def: &ResolvedMove,
) -> Option<u32> {
    let terrain = state.field.terrain?.kind;
    let grounded = |id: PokemonId| state.pokemon(id).is_some_and(|p| grounded(state, rules, p));
    let hiding = |id: PokemonId| {
        state
            .pokemon(id)
            .is_some_and(|p| super::two_turn::hiding(p).is_some())
    };
    let boost = rules.conditions.terrain.boost;
    let move_type = move_def.move_type;
    match terrain {
        TerrainKind::Electric => {
            (move_type == PokemonType::Electric && grounded(attacker) && !hiding(attacker))
                .then_some(boost)
        }
        TerrainKind::Psychic => {
            (move_type == PokemonType::Psychic && grounded(attacker) && !hiding(attacker))
                .then_some(boost)
        }
        TerrainKind::Grassy => {
            if GRASSY_WEAKENED.contains(&move_def.id) && grounded(defender) && !hiding(defender) {
                Some(2048)
            } else {
                (move_type == PokemonType::Grass && grounded(attacker)).then_some(boost)
            }
        }
        TerrainKind::Misty => {
            (move_type == PokemonType::Dragon && grounded(defender) && !hiding(defender))
                .then_some(2048)
        }
    }
}

/// The weather's `onWeatherModifyDamage`: sun and rain, as the defender sees them (Air Lock,
/// Utility Umbrella: `defender.effectiveWeather()`), strengthen or weaken Fire and Water moves;
/// sun the attacker sees boosts Hydro Steam like a Fire move.
pub(crate) fn weather_damage_modifier(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    defender: PokemonId,
    move_def: &ResolvedMove,
) -> Fraction {
    if move_def.behavior.sun_boosted
        && super::weather_for(state, rules, attacker) == Some(WeatherKind::Sun)
    {
        return rules.conditions.sun_fire_modifier;
    }
    let Some(weather) = super::weather_for(state, rules, defender) else {
        return Fraction::ONE;
    };

    // Gen 2 Solar Beam in the rain: the only move that waits for the sun.
    if weather == WeatherKind::Rain
        && let Some(modifier) = rules.conditions.rain_halves_sun_charge
        && move_def
            .behavior
            .two_turn
            .is_some_and(|spec| spec.skip == crate::dex::ChargeSkip::InSun)
    {
        return modifier;
    }
    match (weather, move_def.move_type) {
        (WeatherKind::Sun, PokemonType::Fire) => rules.conditions.sun_fire_modifier,
        (WeatherKind::Sun, PokemonType::Water) => rules.conditions.sun_water_modifier,
        (WeatherKind::Rain, PokemonType::Fire) => rules.conditions.rain_fire_modifier,
        (WeatherKind::Rain, PokemonType::Water) => rules.conditions.rain_water_modifier,
        _ => Fraction::ONE,
    }
}
