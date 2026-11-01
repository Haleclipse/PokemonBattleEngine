//! Sturdy: from Gen 5 a move that would knock the holder out from full HP leaves it at 1 (the
//! confusion self-hit's pseudo-move too)
//! (Showdown `onDamage`, priority -30 → runs after Magic Guard, before Focus Sash;
//! `HitRules::sturdy_survives_from_full_hp`). Gen 3-4 Sturdy only blocks OHKO moves, which is
//! not modelled because OHKO moves are Unsupported.

use crate::engine::{BattleContext, DamageAmount, DamageDecision, DamageRequest};
use crate::ids::AbilityId;

pub fn damage_resolution(ctx: &mut BattleContext<'_>, request: &DamageRequest) -> DamageDecision {
    let (Some(hp), Some(max_hp)) = (ctx.hp(request.target), ctx.max_hp(request.target)) else {
        return DamageDecision::Allow;
    };
    if ctx.rules().hits().sturdy_survives_from_full_hp
        && request.from_a_move(&ctx.rules())
        && hp == max_hp
        && request.amount.0 >= hp
    {
        ctx.ability_activated(request.target, AbilityId::STURDY);
        DamageDecision::Reduce(DamageAmount(hp - 1))
    } else {
        DamageDecision::Allow
    }
}
