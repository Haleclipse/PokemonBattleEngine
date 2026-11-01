//! The abilities around a Pokémon going down (DESIGN.md §30.4, §30.6): Aftermath and Innards
//! Out (`onDamagingHit`, order 1) strike back at the attacker whose hit knocked their holder
//! out; Damp (`BattleContext::damp_active`) stops Aftermath's damage; Perish Body starts Perish
//! Song on both.

use crate::engine::{
    BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest, DamagingHit, EffectSource,
};
use crate::ids::AbilityId;

/// Aftermath (Gen 4+): a contact hit that knocked the holder out costs the attacker a quarter
/// of its max HP — nothing while a Damp is active (`onAnyDamage`). Gen 5-6 (the gen6 mod) knock
/// the attacker out at once (`contact_reaction_instafaint`).
pub fn aftermath(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if !ctx.is_fainted(hit.holder) {
        return;
    }
    if !hit.contact {
        ctx.padded_contact(hit, AbilityId::AFTERMATH);
        return;
    }
    if ctx.is_fainted(hit.attacker) {
        return;
    }
    let Some(max_hp) = ctx.base_max_hp(hit.attacker) else {
        return;
    };
    if ctx.damp_active() {
        return;
    }
    strike_back(
        ctx,
        hit,
        AbilityId::AFTERMATH,
        (max_hp / 4).max(1),
        ctx.rules().hits().contact_reaction_instafaint,
    );
}

/// Innards Out (Gen 7+): the hit that knocked the holder out costs the attacker the HP the
/// holder lost to it — with the move's earlier hits (Showdown `damage + move.totalDamage`).
/// `target.getUndynamaxedHP(damage)` (DESIGN.md §44.5): a Dynamaxed holder's damage reads as
/// it would without the doubling.
pub fn innards_out(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if !ctx.is_fainted(hit.holder) || ctx.is_fainted(hit.attacker) {
        return;
    }
    let lost = hit.damage.saturating_add(hit.earlier_damage);
    let amount = ctx
        .battle_state()
        .pokemon(hit.holder)
        .map_or(lost, |pokemon| pokemon.undynamaxed_hp(Some(lost)));
    strike_back(ctx, hit, AbilityId::INNARDS_OUT, amount.max(1), false);
}

fn strike_back(
    ctx: &mut BattleContext<'_>,
    hit: DamagingHit,
    ability: AbilityId,
    amount: u16,
    instafaint: bool,
) {
    // Its damage's line names it, nothing before: one Magic Guard keeps off shows nothing.
    ctx.apply_damage(DamageRequest {
        target: hit.attacker,
        amount: DamageAmount(amount),
        kind: DamageKind::Reaction,
        source: EffectSource::Ability {
            pokemon: hit.holder,
            ability,
        },
        flags: DamageFlags {
            bypass_substitute: true,
            spare_one_hp: false,
            instafaint,
            single_strike: false,
            future_move: false,
        },
    });
}

/// Perish Body (Gen 8+, `onDamagingHit`, no order): a contact hit from an attacker not already
/// counting starts Perish Song on the attacker and the holder (each only if not already
/// counting, and not once down).
pub fn perish_body(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if !hit.contact || ctx.perishing(hit.attacker) {
        return;
    }
    ctx.ability_activated(hit.holder, AbilityId::PERISH_BODY);
    ctx.start_perish_song(hit.attacker);
    ctx.start_perish_song(hit.holder);
}
