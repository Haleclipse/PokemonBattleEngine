//! Reflect, Light Screen, Aurora Veil (DESIGN.md §10.7), Mist, Safeguard, Lucky Chant (§10.8):
//! the protection goes up where the move aims — the user's side (`sideCondition`) or the user
//! itself (`volatileStatus`: Gen 1 screens, whose `onTryHit` fails when one is already up, and
//! Gen 1-2 Mist). Aurora Veil's `onTry` (only in hail / snow) is `TryUse::InWeather`.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::SideConditionKind;

pub fn reflect(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_side_protection(user, targets, SideConditionKind::Reflect)
}

pub fn light_screen(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_side_protection(user, targets, SideConditionKind::LightScreen)
}

pub fn aurora_veil(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_side_protection(user, targets, SideConditionKind::AuroraVeil)
}

pub fn mist(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_side_protection(user, targets, SideConditionKind::Mist)
}

pub fn safeguard(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_side_protection(user, targets, SideConditionKind::Safeguard)
}

pub fn lucky_chant(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.set_side_protection(user, targets, SideConditionKind::LuckyChant)
}
