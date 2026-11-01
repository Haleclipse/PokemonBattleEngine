//! Contrary / Simple: rewrite every stat change before it is capped (Showdown `onChangeBoost`),
//! but a status Z-Move's Z power (`effect.id === 'zpower'`; Bulbapedia: "Stat stage increases
//! caused by the Z-Power effects of status Z-Moves are not inverted by Contrary", nor modified by
//! Simple — R-M9-44).

use crate::engine::{StatChangeCause, StatChangeContext};

pub fn contrary(ctx: &StatChangeContext<'_>, stages: i8) -> i8 {
    if ctx.request().cause == StatChangeCause::ZPower {
        return stages;
    }
    stages.saturating_neg()
}

pub fn simple(ctx: &StatChangeContext<'_>, stages: i8) -> i8 {
    if ctx.request().cause == StatChangeCause::ZPower {
        return stages;
    }
    stages.saturating_mul(2)
}
