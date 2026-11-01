//! Terrain-setting abilities (Showdown `onStart` / `onDamagingHit` → `field.setTerrain`, which
//! fails quietly under the same terrain): the Surges, Hadron Engine, Seed Sower; and Quark
//! Drive's `onStart` (its `TerrainChange` reaction, `BattleContext::quark_drive`).

use crate::engine::{BattleContext, DamagingHit};
use crate::ids::{AbilityId, PokemonId};
use crate::state::TerrainKind;

/// The terrain `ability` sets: its line names the ability (`-fieldstart|move: Misty
/// Terrain|[from] ability: Misty Surge|[of] holder`, R-M10-39). True when it started.
fn surge(
    ctx: &mut BattleContext<'_>,
    kind: TerrainKind,
    source: PokemonId,
    ability: AbilityId,
) -> bool {
    let started = ctx.set_terrain(kind, source);
    if started {
        ctx.ability_activated(source, ability);
    }
    started
}

pub fn electric_surge(ctx: &mut BattleContext<'_>, source: PokemonId) {
    surge(
        ctx,
        TerrainKind::Electric,
        source,
        AbilityId::ELECTRIC_SURGE,
    );
}

pub fn grassy_surge(ctx: &mut BattleContext<'_>, source: PokemonId) {
    surge(ctx, TerrainKind::Grassy, source, AbilityId::GRASSY_SURGE);
}

pub fn misty_surge(ctx: &mut BattleContext<'_>, source: PokemonId) {
    surge(ctx, TerrainKind::Misty, source, AbilityId::MISTY_SURGE);
}

pub fn psychic_surge(ctx: &mut BattleContext<'_>, source: PokemonId) {
    surge(ctx, TerrainKind::Psychic, source, AbilityId::PSYCHIC_SURGE);
}

/// Hadron Engine sets Electric Terrain like Electric Surge; under it already, it is only
/// announced (`-activate`).
pub fn hadron_engine(ctx: &mut BattleContext<'_>, source: PokemonId) {
    if !surge(ctx, TerrainKind::Electric, source, AbilityId::HADRON_ENGINE) {
        ctx.ability_activated(source, AbilityId::HADRON_ENGINE);
    }
}

/// Seed Sower (`onDamagingHit`): a hit that reached the holder sets Grassy Terrain.
pub fn seed_sower(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    surge(ctx, TerrainKind::Grassy, hit.holder, AbilityId::SEED_SOWER);
}

pub fn quark_drive(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.quark_drive(source);
}

/// Mimicry's `onStart`: the type of the terrain the holder finds.
pub fn mimicry(ctx: &mut BattleContext<'_>, source: PokemonId) {
    ctx.mimicry(source);
}
