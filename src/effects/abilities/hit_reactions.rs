//! Reactions to a hit that reached the holder's HP with no `onDamagingHitOrder` (they come
//! after Rough Skin and Rocky Helmet): Gooey and Tangling Hair lower a contact attacker's
//! Speed, Cotton Down every other active Pokémon's. The holder is the source of each drop, so
//! Clear Body refuses it and Mirror Armor sends it back. Showdown: `onDamagingHit`.

use crate::engine::{BattleContext, DamagingHit, EffectSource, StatChangeCause, StatChangeRequest};
use crate::ids::PokemonId;
use crate::state::Stat;

fn slow(ctx: &mut BattleContext<'_>, holder: PokemonId, target: PokemonId) {
    let Some(ability) = ctx.ability_of(holder) else {
        return;
    };
    ctx.try_change_stat_stage(StatChangeRequest {
        target,
        stat: Stat::Speed,
        stages: -1,
        source: EffectSource::Ability {
            pokemon: holder,
            ability,
        },
        cause: StatChangeCause::Ability,
    });
}

/// Gooey, Tangling Hair: a contact hit lowers the attacker's Speed by one.
pub fn slow_the_attacker(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if !hit.contact {
        if let Some(ability) = ctx.ability_of(hit.holder) {
            ctx.padded_contact(hit, ability);
        }
        return;
    }
    if let Some(ability) = ctx.ability_of(hit.holder) {
        ctx.ability_activated(hit.holder, ability);
    }
    slow(ctx, hit.holder, hit.attacker);
}

/// Cotton Down: any hit lowers the Speed of every other active Pokémon — the holder's partner
/// too — by one, side by side and slot by slot (Showdown `getAllActive`).
pub fn cotton_down(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    let others: Vec<PokemonId> = ctx
        .active_pokemon()
        .into_iter()
        .filter(|pokemon| *pokemon != hit.holder)
        .collect();
    if others.is_empty() {
        return;
    }
    if let Some(ability) = ctx.ability_of(hit.holder) {
        ctx.ability_activated(hit.holder, ability);
    }
    for pokemon in others {
        slow(ctx, hit.holder, pokemon);
    }
}
