//! Rough Skin / Iron Barbs: a contact hit that reached the holder's HP costs the attacker a
//! fraction of its max HP (`HitRules::rough_skin_divisor`: 1/16 in Gen 3, 1/8 from Gen 4;
//! Iron Barbs 1/8). Fires per hit, even on the hit that faints the holder. Before Gen 5 the
//! hit must have dealt damage (`contact_reaction_needs_damage`).

use crate::engine::{
    BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest, DamagingHit, EffectSource,
};
use crate::ids::AbilityId;

pub fn rough_skin(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    let divisor = ctx.rules().hits().rough_skin_divisor;
    contact_damage(ctx, hit, AbilityId::ROUGH_SKIN, divisor);
}

pub fn iron_barbs(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    contact_damage(ctx, hit, AbilityId::IRON_BARBS, 8);
}

fn contact_damage(ctx: &mut BattleContext<'_>, hit: DamagingHit, ability: AbilityId, divisor: u16) {
    if !hit.contact {
        ctx.padded_contact(hit, ability);
        return;
    }
    let needs_damage = ctx.rules().hits().contact_reaction_needs_damage;
    if (needs_damage && hit.damage == 0) || ctx.is_fainted(hit.attacker) {
        return;
    }
    let Some(max_hp) = ctx.base_max_hp(hit.attacker) else {
        return;
    };
    // Its damage's line names it (`[from] ability: Rough Skin`), nothing before: one Magic Guard
    // keeps off shows nothing (R-M10-37).
    ctx.apply_damage(DamageRequest {
        target: hit.attacker,
        amount: DamageAmount((max_hp / divisor.max(1)).max(1)),
        kind: DamageKind::Reaction,
        source: EffectSource::Ability {
            pokemon: hit.holder,
            ability,
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
