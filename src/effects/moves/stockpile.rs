//! Stockpile and Swallow (DESIGN.md §20.1, R159); Spit Up is data (`PowerFormula::
//! StockpileLayers`, `TryUse::Stockpiled`, `MoveBehavior::ends_stockpile`).

use crate::engine::{BattleContext, ConditionApplyError, EffectSource, HealRequest, HealResult};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;

/// Stockpile's `onTry`: fails at three layers.
pub fn stockpile_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    if ctx.stockpile_layers(user) >= 3 {
        return Err(ConditionApplyError::AlreadyActive);
    }
    Ok(())
}

/// Swallow's `onTry`: fails without a Stockpile.
pub fn swallow_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    if ctx.stockpile_layers(user) == 0 {
        return Err(ConditionApplyError::InvalidTarget);
    }
    Ok(())
}

/// Stockpile (`volatileStatus`): one more layer.
pub fn stockpile(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.stockpile(user)
}

/// Swallow past its `onTry`: heals a quarter, half or all of the max HP for one, two or three
/// layers (`modify`; a snatched Swallow's user with none heals as for one), then the Stockpile
/// ends. Healing nothing is a failure
/// unless `ConditionRules::swallow_nothing_not_fail`.
pub fn swallow(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let layers = ctx.stockpile_layers(user);
    let modifier = [1024, 2048, 4096][usize::from(layers.clamp(1, 3)) - 1];
    let max_hp = u32::from(ctx.max_hp(user).unwrap_or(0));
    let healed = matches!(
        ctx.apply_heal(HealRequest {
            target: user,
            amount: crate::engine::showdown_modify(max_hp, modifier).min(u32::from(u16::MAX))
                as u16,
            source: EffectSource::Move {
                pokemon: user,
                move_id: MoveId::SWALLOW,
            },
            direct: false,
        }),
        HealResult::Applied { .. }
    );
    ctx.end_stockpile(user);
    if healed || ctx.rules().conditions().swallow_nothing_not_fail {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}
