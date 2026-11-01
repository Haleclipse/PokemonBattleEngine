//! Imposter (`onSwitchIn`, DESIGN.md §41.7): as the holder enters, it transforms into the foe
//! straight across. Not `onStart`: a Skill Swap or Neutralizing Gas leaving does not run it.

use crate::engine::BattleContext;
use crate::ids::{AbilityId, PokemonId};

pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if let Some(target) = ctx.foe_across(holder) {
        ctx.transform_into(holder, target, Some(AbilityId::IMPOSTER));
    }
}
