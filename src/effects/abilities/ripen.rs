//! Ripen (Showdown `onChangeBoost`; the rest of it — `onTryHeal`, `onTryEatItem`, `onEatItem`,
//! `onSourceModifyDamage` — is `AbilityBehavior::ripen`, read where berries are eaten and where a
//! resist berry halves a hit).

use crate::engine::StatChangeContext;

/// `onChangeBoost`: a berry's stat change is doubled (`effect.isBerry`).
pub fn change_boost(ctx: &StatChangeContext<'_>, stages: i8) -> i8 {
    if ctx.source_is_berry() {
        stages.saturating_mul(2)
    } else {
        stages
    }
}
