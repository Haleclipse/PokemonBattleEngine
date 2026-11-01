//! Natural Cure (Showdown `onSwitchOut`): leaving the field by a switch cures the holder's
//! status. Fainting is not a switch.

use crate::engine::{BattleContext, CureReason};
use crate::ids::{AbilityId, PokemonId};

pub fn switch_out(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.cure_status(holder, CureReason::Ability(AbilityId::NATURAL_CURE)) {
        // `-curestatus … [from] ability: Natural Cure|[silent]`.
        ctx.ability_at_work(holder, AbilityId::NATURAL_CURE);
    }
}
