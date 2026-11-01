//! Regenerator (Gen 5+, Showdown `onSwitchOut`): leaving the field by a switch heals a third of
//! the holder's max HP (truncated). Fainting is not a switch. The heal is `pokemon.heal`, with
//! no `TryHeal`: Heal Block does not stop it (DESIGN.md R-M8-90).

use crate::engine::{BattleContext, EffectSource, HealRequest};
use crate::ids::{AbilityId, PokemonId};

pub fn switch_out(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(max_hp) = ctx.base_max_hp(holder) else {
        return;
    };
    if ctx.is_fainted(holder) {
        return;
    }
    ctx.apply_heal(HealRequest {
        target: holder,
        amount: max_hp / 3,
        source: EffectSource::Ability {
            pokemon: holder,
            ability: AbilityId::REGENERATOR,
        },
        direct: true,
    });
}
