//! Rocky Helmet: a contact hit that reached the holder's HP costs the attacker 1/6 of its max
//! HP (Showdown `onDamagingHit`, order 2: after the holder's ability reaction).

use crate::engine::{
    BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest, DamagingHit, EffectSource,
};
use crate::ids::ItemId;

pub fn damaging_hit_reaction(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    let needs_damage = ctx.rules().hits().contact_reaction_needs_damage;
    if !hit.contact || (needs_damage && hit.damage == 0) || ctx.is_fainted(hit.attacker) {
        return;
    }
    let Some(max_hp) = ctx.base_max_hp(hit.attacker) else {
        return;
    };
    // Its damage's line names it (`[from] item: Rocky Helmet`), nothing before: one Magic Guard
    // keeps off shows nothing (R-M10-37).
    ctx.apply_damage(DamageRequest {
        target: hit.attacker,
        amount: DamageAmount((max_hp / 6).max(1)),
        kind: DamageKind::Reaction,
        source: EffectSource::Item {
            pokemon: hit.holder,
            item: ItemId::ROCKY_HELMET,
        },
        flags: DamageFlags {
            bypass_substitute: true,
            spare_one_hp: false,
            instafaint: ctx.rules().hits().contact_reaction_instafaint,
            single_strike: false,
            future_move: false,
        },
    });
}
