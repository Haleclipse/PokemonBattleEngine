//! Rest (Showdown `onTry` + `onHit`, DESIGN.md §20.2, R170): fails at full HP (Gen 1: or 255 /
//! 511 HP short, `heal_fails_255_short`), or with Insomnia or Vital Spirit (by id, as Showdown's
//! `hasAbility`; the abilities' own `onSetStatus` is not modelled yet); else the user falls
//! asleep — replacing any other status, through the checks every sleep meets (a terrain, Sweet
//! Veil, Leaf Guard but in Gen 4, Comatose …) — for `ConditionRules::rest_sleep_turns` (over the duration
//! the sleep rolled), and is healed to full. (`onTry`'s refusal of a user already asleep or
//! Comatose is Comatose's own refusal here; a sleeping user needs Sleep Talk.) Gen 2 lets a
//! sleeping user rest again (`ConditionRules::rest_renews_sleep`).

use crate::engine::{
    BattleContext, ConditionApplyError, EffectSource, HealRequest, StatusApplyResult,
};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::NonVolatileStatus;

/// Rest's `onTry`: not for a user asleep (but Gen 2) or Comatose, at full HP (Gen 1: or 255 /
/// 511 short), or with Insomnia or Vital Spirit.
pub fn tries(ctx: &mut BattleContext<'_>, user: PokemonId) -> Result<(), ConditionApplyError> {
    let (Some(hp), Some(max_hp)) = (ctx.hp(user), ctx.max_hp(user)) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    if ctx.counts_as_asleep(user) && !ctx.rules().conditions().rest_renews_sleep {
        return Err(ConditionApplyError::Immune);
    }
    if hp >= max_hp || crate::engine::heal_fails_255_short(&ctx.rules().conditions(), hp, max_hp) {
        return Err(ConditionApplyError::AlreadyActive);
    }
    // `-fail|user|[from] ability: Insomnia|[of] user` names it (R-M10-40).
    for ability in [AbilityId::INSOMNIA, AbilityId::VITAL_SPIRIT] {
        if ctx.has_ability(user, ability) {
            ctx.ability_activated(user, ability);
            return Err(ConditionApplyError::AlreadyActive);
        }
    }
    Ok(())
}

/// Rest past its `onTry`.
pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(max_hp) = ctx.max_hp(user) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let source = EffectSource::Move {
        pokemon: user,
        move_id: MoveId::REST,
    };
    // Gen 2: a user asleep already keeps its sleep, renewed below.
    let renewed = ctx.rules().conditions().rest_renews_sleep
        && ctx.status_of(user) == Some(NonVolatileStatus::Sleep);
    if !renewed
        && !matches!(
            ctx.replace_status(user, NonVolatileStatus::Sleep, source),
            StatusApplyResult::Applied { .. }
        )
    {
        return Err(ConditionApplyError::Immune);
    }
    ctx.set_sleep_turns(user, ctx.rules().conditions().rest_sleep_turns);
    ctx.apply_heal(HealRequest {
        target: user,
        amount: max_hp,
        source,
        direct: false,
    });
    Ok(())
}
