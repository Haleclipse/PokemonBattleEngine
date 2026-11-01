//! Sticky Barb (DESIGN.md R249): an eighth of the holder's max HP lost at the end of the turn
//! (Showdown `onResidual`: `damage`, floor, at least 1; Magic Guard refuses it), and a contact
//! move's user holding nothing takes it (`onHit`: `BattleContext::sticky_barb_transfer`).

use crate::effects::{ResidualHandler, ResidualParticipant};
use crate::engine::{
    BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest, EffectSource,
};
use crate::ids::{ItemId, PokemonId};
use crate::rules::ResidualKind;

use super::ItemBehavior;

pub(super) fn sticky_barb() -> ItemBehavior {
    ItemBehavior {
        residual: Some(ResidualParticipant {
            kind: ResidualKind::StickyBarb,
            handler: residual as ResidualHandler,
        }),
        sticks_to_contact: true,
        ..ItemBehavior::default()
    }
}

fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(max_hp) = ctx.base_max_hp(holder) else {
        return;
    };
    ctx.apply_damage(DamageRequest {
        target: holder,
        amount: DamageAmount((max_hp / 8).max(1)),
        kind: DamageKind::Residual,
        source: EffectSource::Item {
            pokemon: holder,
            item: ItemId::STICKY_BARB,
        },
        flags: DamageFlags::default(),
    });
}
