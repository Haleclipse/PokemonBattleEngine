//! Abilities that act on the Pokémon around the holder as it enters (`onStart`; DESIGN.md
//! §20.2, R130, R165): Hospitality heals its adjacent partners, Supersweet Syrup lowers its
//! adjacent foes' evasion once per battle, Costar copies its partner's stages. Some only announce
//! themselves (`announce`).

use crate::engine::{BattleContext, EffectSource, HealRequest, StatChangeCause, StatChangeRequest};
use crate::ids::{AbilityId, PokemonId};
use crate::state::{Stat, VolatileConditionKind};

/// An ability that announces itself and does nothing else as it starts (Showdown
/// `this.add('-ability', pokemon, …)`): Dark Aura, Fairy Aura, Aura Break, the Ruin abilities and
/// Comatose as it starts (`onStart`), Air Lock and Cloud Nine as the holder enters the field
/// (`onSwitchIn`: not when it gets the ability, nor as Neutralizing Gas ends) — R-M10-39.
pub fn announce(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if let Some(ability) = ctx.ability_of(holder) {
        ctx.ability_activated(holder, ability);
    }
}

/// Hospitality: each adjacent partner regains 1/4 of its max HP (nothing at full HP or under
/// Heal Block).
pub fn hospitality(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    for ally in ctx.adjacent_allies(holder) {
        let Some(max_hp) = ctx.base_max_hp(ally) else {
            continue;
        };
        ctx.apply_heal(HealRequest {
            target: ally,
            amount: (max_hp / 4).max(1),
            source: EffectSource::Ability {
                pokemon: holder,
                ability: AbilityId::HOSPITALITY,
            },
            direct: false,
        });
    }
}

/// Supersweet Syrup: the first time it enters this battle (`BattleContext::first_syrup`), -1
/// evasion on each adjacent foe — one behind a Substitute is immune.
pub fn supersweet_syrup(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if !ctx.first_syrup(holder) {
        return;
    }
    ctx.ability_activated(holder, AbilityId::SUPERSWEET_SYRUP);
    for foe in ctx.adjacent_foes(holder) {
        if ctx.has_volatile(foe, VolatileConditionKind::Substitute) {
            continue;
        }
        ctx.try_change_stat_stage(StatChangeRequest {
            target: foe,
            stat: Stat::Evasion,
            stages: -1,
            source: EffectSource::Ability {
                pokemon: holder,
                ability: AbilityId::SUPERSWEET_SYRUP,
            },
            cause: StatChangeCause::Ability,
        });
    }
}

/// Costar (`onStart`, `onSwitchInPriority` -2): the holder takes its first standing partner's
/// stat stages and critical-hit volatiles (Focus Energy, Dragon Cheer, Laser Focus — its own
/// dropped first), as Psych Up does.
pub fn costar(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(side) = ctx.side_of(holder) else {
        return;
    };
    let Some(partner) = ctx.active_pokemon().into_iter().find(|pokemon| {
        *pokemon != holder && ctx.side_of(*pokemon) == Some(side) && !ctx.is_fainted(*pokemon)
    }) else {
        return;
    };
    let source = EffectSource::Ability {
        pokemon: holder,
        ability: AbilityId::COSTAR,
    };
    for stat in crate::engine::STAGED_STATS {
        let stage = ctx.stage(partner, stat);
        ctx.set_stage(holder, stat, stage, source);
    }
    ctx.copy_crit_volatiles(holder, partner);
    ctx.ability_activated(holder, AbilityId::COSTAR);
}
