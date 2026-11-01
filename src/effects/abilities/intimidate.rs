//! Intimidate (Showdown `onStart`): −1 Attack on every adjacent foe, except a foe behind a
//! Substitute (`-immune`) — and, in Gen 4, one whose Substitute U-turn just broke; Gen 3-4 do
//! not announce the ability when no foe can be affected.

use crate::engine::{BattleContext, EffectSource, Stat};
use crate::ids::{AbilityId, PokemonId};

pub fn switch_in(ctx: &mut BattleContext<'_>, source: PokemonId) {
    let foes = ctx.adjacent_foes(source);
    let reachable: Vec<PokemonId> = foes
        .iter()
        .copied()
        .filter(|foe| !ctx.shielded_from_intimidation(*foe))
        .collect();
    // Showdown announces the ability inside its loop over the adjacent foes: nobody adjacent,
    // no announcement; Gen 3-4 also stay silent when every foe is out of reach.
    if foes.is_empty()
        || (reachable.is_empty()
            && ctx
                .rules()
                .conditions()
                .intimidate_silent_without_reachable_foe)
    {
        return;
    }
    ctx.ability_activated(source, AbilityId::INTIMIDATE);

    for target in reachable {
        ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
            target,
            stat: Stat::Attack,
            stages: -1,
            source: EffectSource::Ability {
                pokemon: source,
                ability: AbilityId::INTIMIDATE,
            },
            cause: crate::engine::StatChangeCause::Ability,
        });
    }
}
