//! Black Sludge: Leftovers for a Poison type (1/16 healed, same residual key 5.4), 1/8 of max HP
//! lost for anyone else (Showdown `damage`, floor, at least 1; a residual, so Magic Guard
//! refuses it).

use crate::dex::PokemonType;
use crate::engine::{
    BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest, EffectSource, HealRequest,
};
use crate::ids::{ItemId, PokemonId};

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(max_hp) = ctx.base_max_hp(holder) else {
        return;
    };
    let source = EffectSource::Item {
        pokemon: holder,
        item: ItemId::BLACK_SLUDGE,
    };
    if ctx.has_type(holder, PokemonType::Poison) {
        let amount = (max_hp / ctx.rules().conditions().leftovers_hp_divisor.max(1)).max(1);
        ctx.apply_heal(HealRequest {
            target: holder,
            amount,
            source,
            direct: false,
        });
    } else {
        ctx.apply_damage(DamageRequest {
            target: holder,
            amount: DamageAmount((max_hp / 8).max(1)),
            kind: DamageKind::Residual,
            source,
            flags: DamageFlags::default(),
        });
    }
}
