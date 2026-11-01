//! Electromorphosis (DESIGN.md §20.2, R163): a damaging hit charges the holder (`onDamagingHit`,
//! order 1: `addVolatile('charge')`, started again when already there).

use crate::engine::{BattleContext, DamagingHit};
use crate::ids::AbilityId;

pub fn electromorphosis(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if ctx.is_fainted(hit.holder) {
        return;
    }
    ctx.ability_activated(hit.holder, AbilityId::ELECTROMORPHOSIS);
    ctx.charge(hit.holder);
}
