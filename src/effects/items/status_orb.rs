//! Toxic Orb / Flame Orb: the holder's own bad poison / burn at the end of the turn (Showdown
//! residual order 28.3, Gen 4: 10.20). The status pipeline decides whether it takes (an existing
//! status, a type or an ability refuses it); Synchronize never passes a self-inflicted status.

use crate::engine::{BattleContext, EffectSource};
use crate::ids::{ItemId, PokemonId};
use crate::state::NonVolatileStatus;

pub fn toxic_orb(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    orb(ctx, holder, ItemId::TOXIC_ORB, NonVolatileStatus::Toxic);
}

pub fn flame_orb(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    orb(ctx, holder, ItemId::FLAME_ORB, NonVolatileStatus::Burn);
}

fn orb(ctx: &mut BattleContext<'_>, holder: PokemonId, item: ItemId, status: NonVolatileStatus) {
    ctx.try_set_status(
        holder,
        status,
        EffectSource::Item {
            pokemon: holder,
            item,
        },
    );
}
