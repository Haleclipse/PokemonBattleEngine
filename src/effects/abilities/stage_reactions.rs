//! Abilities that raise the holder's stats as it enters or when a foe lowers them (DESIGN.md
//! §20.2, R127): Download, Intrepid Sword, Dauntless Shield (`onStart`), Defiant and
//! Competitive (`onAfterEachBoost`).

use crate::engine::{BattleContext, StatChangeRequest};
use crate::ids::PokemonId;
use crate::state::Stat;

use super::hit_boosts::boost;

/// Download: +1 Special Attack when the foes' Defense (at its stage, unmodified) totals at least
/// their Special Defense, +1 Attack otherwise — nothing without a foe to read. Gen 4 skips a
/// foe behind a Substitute (`ConditionRules::download_skips_substitutes`). Under Wonder Room
/// each stat is read at the other's stage (Showdown `getStat(stat, false, true)`: "Download
/// ignores Wonder Room's effect, but this results in stat stages being calculated on the
/// opposite defensive stat").
pub fn download(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let skip_substitutes = ctx.rules().conditions().download_skips_substitutes;
    let wonder_room = ctx
        .battle_state()
        .field
        .contains(crate::state::FieldConditionKind::WonderRoom);
    let (defense_stage, special_defense_stage) = if wonder_room {
        (Stat::SpecialDefense, Stat::Defense)
    } else {
        (Stat::Defense, Stat::SpecialDefense)
    };
    let (defense, special_defense) = ctx
        .active_foes(holder)
        .into_iter()
        .filter(|foe| {
            !(skip_substitutes
                && ctx.has_volatile(*foe, crate::state::VolatileConditionKind::Substitute))
        })
        .fold((0, 0), |(def, spd), foe| {
            (
                def + ctx.stat_at_stage_of(foe, Stat::Defense, defense_stage),
                spd + ctx.stat_at_stage_of(foe, Stat::SpecialDefense, special_defense_stage),
            )
        });
    if defense > 0 && defense >= special_defense {
        boost(ctx, holder, &[(Stat::SpecialAttack, 1)]);
    } else if special_defense > 0 {
        boost(ctx, holder, &[(Stat::Attack, 1)]);
    }
}

/// Intrepid Sword: +1 Attack as the holder enters — from Gen 9 once per battle.
pub fn intrepid_sword(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.first_entry_boost(holder, Stat::Attack) {
        boost(ctx, holder, &[(Stat::Attack, 1)]);
    }
}

/// Dauntless Shield: +1 Defense as the holder enters — from Gen 9 once per battle.
pub fn dauntless_shield(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.first_entry_boost(holder, Stat::Defense) {
        boost(ctx, holder, &[(Stat::Defense, 1)]);
    }
}

/// Defiant / Competitive (`onAfterEachBoost`): a stage another side's Pokémon lowered — each
/// one, so a two-stat drop answers twice — raises this stat by two. A drop Mist or an ability
/// refused never happened; one Mirror Armor sends back comes from the Mirror Armor holder.
fn answer_drop(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    request: StatChangeRequest,
    stat: Stat,
) {
    let by_foe = request.source.pokemon().is_some_and(|source| {
        ctx.side_of(source)
            .is_some_and(|side| Some(side) != ctx.side_of(holder))
    });
    if request.stages < 0 && by_foe {
        boost(ctx, holder, &[(stat, 2)]);
    }
}

pub fn defiant(ctx: &mut BattleContext<'_>, holder: PokemonId, request: StatChangeRequest) {
    answer_drop(ctx, holder, request, Stat::Attack);
}

pub fn competitive(ctx: &mut BattleContext<'_>, holder: PokemonId, request: StatChangeRequest) {
    answer_drop(ctx, holder, request, Stat::SpecialAttack);
}
