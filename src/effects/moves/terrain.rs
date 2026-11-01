//! Electric / Grassy / Misty / Psychic Terrain (Showdown `terrain`, target `all`): the terrain
//! starts (`BattleContext::set_terrain`: Terrain Extender, the `TerrainChange` reactions); the
//! same terrain already up makes the move fail.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::TerrainKind;

fn start(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    kind: TerrainKind,
) -> Result<(), ConditionApplyError> {
    if ctx.set_terrain(kind, user) {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}

pub fn electric(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    start(ctx, user, TerrainKind::Electric)
}

pub fn grassy(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    start(ctx, user, TerrainKind::Grassy)
}

pub fn misty(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    start(ctx, user, TerrainKind::Misty)
}

pub fn psychic(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    start(ctx, user, TerrainKind::Psychic)
}
