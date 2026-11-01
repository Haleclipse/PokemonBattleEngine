//! Lightning Rod / Storm Drain: a move of the absorbed type aimed at the holder by someone
//! else is refused and raises the holder's Special Attack by one stage (Showdown `onTryHit`:
//! `-ability … boost` + `-boost`, or `-immune` when already at +6). In doubles the same
//! abilities pull single-target moves of that type onto the holder (`redirect_type`).

use crate::dex::PokemonType;
use crate::engine::{
    BattleContext, EffectSource, HitBlockReason, StatChangeCause, StatChangeRequest,
    StatChangeResult, TryHitContext, TryHitResult,
};
use crate::ids::{AbilityId, PokemonId};
use crate::state::Stat;

pub fn lightning_rod_try_hit(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    absorb_try_hit(ctx, PokemonType::Electric, AbilityId::LIGHTNING_ROD)
}

pub fn storm_drain_try_hit(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    absorb_try_hit(ctx, PokemonType::Water, AbilityId::STORM_DRAIN)
}

fn absorb_try_hit(
    ctx: &mut TryHitContext<'_>,
    absorbed: PokemonType,
    ability: AbilityId,
) -> TryHitResult {
    if ctx.move_type() == absorbed && ctx.user() != ctx.target() {
        TryHitResult::Blocked(HitBlockReason::Ability(ability))
    } else {
        TryHitResult::Allow
    }
}

/// The refusal's side effect: +1 Special Attack, announced as the ability when it lands.
pub fn absorb(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(ability) = ctx.ability_of(holder) else {
        return;
    };
    let result = ctx.try_change_stat_stage(StatChangeRequest {
        target: holder,
        stat: Stat::SpecialAttack,
        stages: 1,
        source: EffectSource::Ability {
            pokemon: holder,
            ability,
        },
        cause: StatChangeCause::Ability,
    });
    if matches!(result, StatChangeResult::Applied { .. }) {
        ctx.ability_activated(holder, ability);
    }
}
