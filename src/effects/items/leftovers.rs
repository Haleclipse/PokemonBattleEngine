//! Leftovers: 1/16 of max HP at the end of every turn (Showdown residual order 5.4). Nothing
//! happens at full HP (the HealRequest is refused, no event).

use crate::engine::{BattleContext, EffectSource, HealRequest};
use crate::ids::{ItemId, PokemonId};

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(max_hp) = ctx.base_max_hp(holder) else {
        return;
    };
    let amount = (max_hp / ctx.rules().conditions().leftovers_hp_divisor.max(1)).max(1);
    ctx.apply_heal(HealRequest {
        target: holder,
        amount,
        source: EffectSource::Item {
            pokemon: holder,
            item: ItemId::LEFTOVERS,
        },
        direct: false,
    });
}
