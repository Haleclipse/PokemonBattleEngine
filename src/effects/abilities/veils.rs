//! Abilities that keep statuses off a Pokémon or take them away (DESIGN.md §20.2, R115):
//! Comatose, Purifying Salt and Leaf Guard for the holder (`onSetStatus`), Sweet Veil and Flower
//! Veil for the holder and its allies (`onAllySetStatus`, `onAllyTryBoost`), and Healer's cure of
//! an adjacent ally at the end of the turn.

use crate::engine::{
    BattleContext, CureReason, EffectSource, StatChangeBlockReason, StatChangeContext,
    StatusBlockReason, TrySetStatusContext, TrySetStatusResult, TryStatChangeResult,
};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::rules::Fraction;
use crate::state::{NonVolatileStatus, RandomRequestKind, WeatherKind};

/// `shown`: the refusal's line is written (`TrySetStatusResult::Blocked`).
fn blocked_by(ability: AbilityId, blocks: bool, shown: bool) -> TrySetStatusResult {
    if blocks {
        TrySetStatusResult::Blocked {
            reason: StatusBlockReason::Ability(ability),
            shown,
        }
    } else {
        TrySetStatusResult::Allow
    }
}

/// Comatose: no status can be set on the holder; said for a status its effect carries.
pub fn comatose(ctx: &TrySetStatusContext<'_>) -> TrySetStatusResult {
    blocked_by(AbilityId::COMATOSE, true, ctx.carried())
}

/// Purifying Salt: no status can be set on the holder, as Comatose.
pub fn purifying_salt(ctx: &TrySetStatusContext<'_>) -> TrySetStatusResult {
    blocked_by(AbilityId::PURIFYING_SALT, true, ctx.carried())
}

/// Leaf Guard: no status in sun (the holder's `effectiveWeather()`) — but Rest's sleep in Gen 4
/// (`ConditionRules::leaf_guard_spares_rest`).
pub fn leaf_guard(ctx: &TrySetStatusContext<'_>) -> TrySetStatusResult {
    let rest = matches!(
        ctx.source(),
        EffectSource::Move {
            move_id: MoveId::REST,
            ..
        }
    );
    blocked_by(
        AbilityId::LEAF_GUARD,
        ctx.target_weather() == Some(WeatherKind::Sun)
            && !(rest && ctx.rules().conditions().leaf_guard_spares_rest),
        // Said for a status its effect carries; the gen4 mod's handler, the one sparing Rest,
        // says nothing.
        ctx.carried() && !ctx.rules().conditions().leaf_guard_spares_rest,
    )
}

/// Sweet Veil (`onAllySetStatus`): no sleep on the holder or its allies, whoever causes it;
/// always said.
pub fn sweet_veil(ctx: &TrySetStatusContext<'_>) -> TrySetStatusResult {
    blocked_by(
        AbilityId::SWEET_VEIL,
        ctx.status() == NonVolatileStatus::Sleep,
        true,
    )
}

/// Flower Veil (`onAllySetStatus`): no status someone else causes on a Grass-type holder or ally
/// — Yawn's sleep excepted (`effect.id !== 'yawn'`; its source is the volatile, nobody). Said
/// for Synchronize's or a move's with no secondary effect (`effect.effectType === 'Move' &&
/// !effect.secondaries`): one its effect carries, or Psycho Shift's.
pub fn flower_veil_status(ctx: &TrySetStatusContext<'_>) -> TrySetStatusResult {
    let psycho_shift = matches!(
        ctx.source(),
        EffectSource::Move {
            move_id: MoveId::PSYCHO_SHIFT,
            ..
        }
    );
    blocked_by(
        AbilityId::FLOWER_VEIL,
        ctx.target_has_type(crate::dex::PokemonType::Grass) && ctx.by_another(),
        ctx.carried() || psycho_shift,
    )
}

/// Flower Veil (`onAllyTryBoost`): no stat drop someone else causes on a Grass-type holder or
/// ally.
pub fn flower_veil_drops(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    if ctx.stages() < 0
        && !ctx.is_self_inflicted()
        && ctx.target_has_type(crate::dex::PokemonType::Grass)
    {
        // `-block|target|ability: Flower Veil|[of] holder`, but for a secondary effect.
        TryStatChangeResult::Blocked {
            reason: StatChangeBlockReason::Ability(AbilityId::FLOWER_VEIL),
            shown: !ctx.is_secondary(),
        }
    } else {
        TryStatChangeResult::Allow
    }
}

/// Healer (Showdown residual order 5.3): each adjacent ally with a status has a 3 in 10 chance
/// (`Healer` decision) to be cured.
pub fn healer(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    for ally in ctx.adjacent_allies(holder) {
        if !ctx.has_status(ally) {
            continue;
        }
        if ctx.chance(RandomRequestKind::Healer, Fraction::new(3, 10)) {
            ctx.ability_activated(holder, AbilityId::HEALER);
            ctx.cure_status(ally, CureReason::Ability(AbilityId::HEALER));
        }
    }
}
