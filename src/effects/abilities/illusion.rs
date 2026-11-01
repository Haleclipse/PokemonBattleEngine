//! Illusion (DESIGN.md §41.7): the holder appears as a party member (`onBeforeSwitchIn`,
//! `BattleContext::take_disguise`) until a damaging hit (`onDamagingHit`, here), a suppression or
//! a change of ability (`onEnd`) lifts it.

use crate::engine::{BattleContext, DamagingHit};

pub fn damaging_hit(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    ctx.end_illusion(hit.holder);
}
