//! Shed Skin (Showdown residual order 5.3, Gen 4: 10.3): a `ShedSkin` decision at the end of
//! every turn the holder is statused; success cures it.

use crate::engine::{BattleContext, CureReason};
use crate::ids::{AbilityId, PokemonId};
use crate::state::RandomRequestKind;

pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if !ctx.has_status(holder) {
        return;
    }
    let chance = ctx.rules().status().lifecycle.shed_skin_chance;
    let triggered = ctx.chance(RandomRequestKind::ShedSkin, chance);
    ctx.shed_skin_checked(holder, triggered);
    if triggered {
        ctx.ability_activated(holder, AbilityId::SHED_SKIN);
        ctx.cure_status(holder, CureReason::Ability(AbilityId::SHED_SKIN));
    }
}
