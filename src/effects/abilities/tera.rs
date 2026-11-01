//! The two abilities the Terastallization formes carry (DESIGN.md §44.6, O.2c): Tera Shift
//! (Terapagos's switch-in change into its Terastal forme) and Embody Aspect (Ogerpon's raised
//! stat once it stands in its `-Tera` forme).

use crate::engine::{
    BattleContext, EffectSource, FormeChangeKind, StatChangeCause, StatChangeRequest,
};
use crate::ids::{AbilityId, PokemonId, SpeciesId};
use crate::state::Stat;

/// Tera Shift (`onSwitchIn`, priority 2): a Terapagos that is not in its Terastal forme yet
/// announces the ability (`-activate`) and changes into it — permanent, and the forme brings its
/// own ability (Tera Shell).
pub fn tera_shift(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(own) = ctx.own_species(holder) else {
        return;
    };
    if !own.same_species(SpeciesId::TERAPAGOS)
        || ctx.species_of(holder) == Some(SpeciesId::TERAPAGOS_TERASTAL)
    {
        return;
    }
    ctx.ability_activated(holder, AbilityId::TERA_SHIFT);
    ctx.forme_change(
        holder,
        SpeciesId::TERAPAGOS_TERASTAL,
        FormeChangeKind::Permanent,
        EffectSource::Ability {
            pokemon: holder,
            ability: AbilityId::TERA_SHIFT,
        },
    );
}

/// Embody Aspect (its `onStart`): the holder stands in its `-Tera` forme and is Terastallized,
/// and its stat has not been raised yet (`embodied`, which outlives a switch-out).
pub fn embody_aspect(ctx: &mut BattleContext<'_>, holder: PokemonId, forme: SpeciesId, stat: Stat) {
    if ctx.own_species(holder) != Some(forme) || !ctx.terastallized(holder) || ctx.embodied(holder)
    {
        return;
    }
    ctx.mark_embodied(holder);
    ctx.try_change_stat_stage(StatChangeRequest {
        target: holder,
        stat,
        stages: 1,
        source: EffectSource::Ability {
            pokemon: holder,
            ability: ctx.raw_ability_of(holder).unwrap_or(AbilityId::NONE),
        },
        cause: StatChangeCause::Ability,
    });
}

/// The `-Tera` forme of an Ogerpon forme (Showdown `terastallize`'s
/// `toID(species.battleOnly || species.id)` + `'tera'`/`'tealtera'`): `None` when the species is
/// no Ogerpon or already one of the `-Tera` formes (the change is then a no-op).
pub fn ogerpon_tera_forme(species: SpeciesId) -> Option<SpeciesId> {
    Some(match species {
        SpeciesId::OGERPON => SpeciesId::OGERPON_TEAL_TERA,
        SpeciesId::OGERPON_WELLSPRING => SpeciesId::OGERPON_WELLSPRING_TERA,
        SpeciesId::OGERPON_HEARTHFLAME => SpeciesId::OGERPON_HEARTHFLAME_TERA,
        SpeciesId::OGERPON_CORNERSTONE => SpeciesId::OGERPON_CORNERSTONE_TERA,
        _ => return None,
    })
}
