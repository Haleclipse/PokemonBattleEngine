//! Two-turn moves (DESIGN.md §15.5): the questions the Move Shell, the hit gate and the replay
//! driver ask about a charging move, answered from the move's `TwoTurnSpec` and the
//! generation's `TwoTurnRules`. No move-specific branch lives here.

use crate::dex::{ChargeSkip, ResolvedDex, ResolvedMove, TwoTurnSpec};
use crate::ids::{ItemId, PokemonId};
use crate::rules::{DamageBonusStage, Fraction, ResolvedRules, SemiInvulnerableKind};
use crate::state::{BattleState, PokemonState, WeatherKind};

/// The weather that lets `spec` attack on its first turn, if it is out as `user` sees it
/// (`attacker.effectiveWeather()`: Air Lock, Utility Umbrella).
pub fn charge_skipped_by(
    state: &BattleState,
    rules: &ResolvedRules,
    user: PokemonId,
    spec: TwoTurnSpec,
) -> Option<WeatherKind> {
    match spec.skip {
        ChargeSkip::Never => None,
        ChargeSkip::InSun => {
            super::weather_for(state, rules, user).filter(|kind| *kind == WeatherKind::Sun)
        }
        ChargeSkip::InRain => {
            super::weather_for(state, rules, user).filter(|kind| *kind == WeatherKind::Rain)
        }
    }
}

/// What lets a charging move attack on its first turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeSkipCause {
    /// The weather its spec names (Solar Beam's sun, Electro Shot's rain).
    Weather(WeatherKind),
    /// The user's Power Herb (Showdown `runEvent('ChargeMove')`, after the weather's return):
    /// used up as the move goes ahead.
    Item(ItemId),
}

/// Whether `user`'s charging move goes ahead at once, and why: the weather first (a Solar Beam
/// in the sun keeps its Power Herb), then a held item that `skips_charge` (Showdown `onTryMove`
/// returns before `runEvent('ChargeMove')` when the weather sends the move out). A pure query:
/// the Shell uses the item up.
pub fn charge_skip(
    state: &BattleState,
    rules: &ResolvedRules,
    user: PokemonId,
    spec: TwoTurnSpec,
) -> Option<ChargeSkipCause> {
    charge_skipped_by(state, rules, user, spec)
        .map(ChargeSkipCause::Weather)
        .or_else(|| {
            super::held_item(state, rules, user)
                .filter(|(_, item)| item.skips_charge)
                .map(|(item, _)| ChargeSkipCause::Item(item))
        })
}

/// Whether `actor` using `move_def` now spends the action charging (Showdown `onTryMove`
/// returning null) rather than releasing a charge or attacking at once.
pub fn will_charge(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    move_def: &ResolvedMove,
) -> bool {
    let Some(spec) = move_def.behavior.two_turn else {
        return false;
    };
    let Some(pokemon) = state.pokemon(actor) else {
        return false;
    };
    if pokemon.volatiles.charging_move() == Some(move_def.id) {
        return false;
    }
    charge_skip(state, rules, actor, spec).is_none()
}

/// Where `target` is hiding (Showdown `isSemiInvulnerable`), if it is in the charging turn of
/// a semi-invulnerable move — or, Gen 1, under its own hiding flag (`ChargeModel::Gen1`),
/// charge or not.
pub fn hiding(target: &PokemonState) -> Option<SemiInvulnerableKind> {
    if sky_drop_hides(target) {
        return Some(SemiInvulnerableKind::Flying);
    }
    target
        .volatiles
        .invulnerability()
        .or_else(|| target.volatiles.charging_hides())
}

/// Sky Drop holds `target` in the air, as its user or as the Pokémon carried up (§15.23): out of
/// reach as in Fly. Nothing doubles against it: its condition's `onAnyBasePower` reads the
/// `BasePower` event's attacker as the target (`runEvent('BasePower', source, target …)`), so it
/// only ever looks at the carrier's or the carried Pokémon's own moves, which never hit it.
fn sky_drop_hides(target: &PokemonState) -> bool {
    target
        .volatiles
        .contains(crate::state::VolatileConditionKind::SkyDrop)
        || target
            .volatiles
            .contains(crate::state::VolatileConditionKind::SkyDropped)
}

/// The spec of the move `target` is charging, if it hides it.
fn hiding_spec(dex: &ResolvedDex, target: &PokemonState) -> Option<TwoTurnSpec> {
    let move_id = target.volatiles.charging_move()?;
    dex.move_by_id(move_id)?
        .behavior
        .two_turn
        .filter(|spec| spec.invulnerable.is_some())
}

/// Showdown `hitStepInvulnerabilityEvent`: where `target` hides from `actor`'s move, if its
/// charge (Fly, Dig …) makes the move miss — unless the move is one the generation lets reach it,
/// a sure hit (`typed_sure_hit`), No Guard on either side, or a Lock-On that reaches it. Decided
/// before immunity, TryHit and accuracy; no random decision. The hit gate and the replay driver
/// ask the same question.
pub fn dodged_by(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    target: PokemonId,
    incoming: &ResolvedMove,
) -> Option<SemiInvulnerableKind> {
    let hidden = state.pokemon(target)?;
    // Sky Drop's user reaches the Pokémon it carries (`onAnyInvulnerability`).
    if hidden.volatiles.sky_dropped_by() == Some(actor) {
        return None;
    }
    let kind = hiding(hidden)?;
    (!rules.two_turn.bypasses(kind, incoming.id)
        && !crate::engine::sure_hit(state, rules, actor, incoming)
        && !crate::engine::no_guard(state, rules, actor, target)
        && !crate::engine::context::lock_on_reaches(state, rules, actor, target, incoming.id))
    .then_some(kind)
}

/// Showdown `onSourceModifyDamage` (Fly, Dig, Dive) / `onSourceBasePower` (Bounce; every one in
/// Gen 2): the bonus a reaching move gets against a hiding target, at the stage the hiding move
/// applies it (the generation may say otherwise, `TwoTurnRules::bonus_stage_override`).
pub fn damage_bonus(
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    target: &PokemonState,
    incoming: &ResolvedMove,
    stage: DamageBonusStage,
) -> Fraction {
    let two_turn = rules.two_turn;
    let Some(spec) = hiding_spec(dex, target) else {
        return Fraction::ONE;
    };
    match spec.invulnerable {
        Some(kind)
            if two_turn.bonus_stage(spec.bonus_stage) == stage
                && two_turn.doubles(kind, incoming.id) =>
        {
            two_turn.bonus
        }
        _ => Fraction::ONE,
    }
}
