//! Technician (Showdown `onBasePowerPriority` 30): ×1.5 when the base power, with the chain so
//! far applied, is 60 or less.

use crate::effects::moves::BasePower;
use crate::engine::BasePowerContext;

pub fn base_power(_ctx: &BasePowerContext<'_>, power: BasePower, chain: u32) -> Option<u32> {
    let so_far = (u64::from(power.0) * u64::from(chain) + 2047) / 4096;
    (so_far <= 60).then_some(6144)
}
