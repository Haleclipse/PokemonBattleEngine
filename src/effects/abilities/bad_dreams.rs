//! Bad Dreams (Showdown `onResidual`, order 28.2; Gen 4: 10.10; DESIGN.md §20.2, R129): every
//! foe on the field that is asleep — or has Comatose — loses 1/8 of its max HP.

use crate::engine::{
    BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest, EffectSource,
};
use crate::ids::{AbilityId, PokemonId};

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    for foe in ctx.active_foes(holder) {
        if !ctx.counts_as_asleep(foe) {
            continue;
        }
        let Some(max_hp) = ctx.base_max_hp(foe) else {
            continue;
        };
        ctx.apply_damage(DamageRequest {
            target: foe,
            amount: DamageAmount((max_hp / 8).max(1)),
            kind: DamageKind::Residual,
            source: EffectSource::Ability {
                pokemon: holder,
                ability: AbilityId::BAD_DREAMS,
            },
            flags: DamageFlags::default(),
        });
    }
}
