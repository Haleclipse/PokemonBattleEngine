//! The moves that change types (DESIGN.md §41.4, K.5): Soak and Magic Powder give the target one
//! type, Camouflage gives the user one, Forest's Curse and Trick-or-Treat add one, Reflect Type
//! copies the target's onto the user (`BattleContext::set_type` / `add_type`); Electrify makes
//! the target's move of the turn Electric (`engine::modified_move`).

use crate::dex::PokemonType;
use crate::engine::{BattleContext, ConditionApplyError, EffectSource};
use crate::ids::{MoveId, PokemonId};
use crate::rules::{Camouflage, Conversion, ResolvedTarget};
use crate::state::{PokemonTypes, TerrainKind};

fn target_of(targets: &[ResolvedTarget]) -> Result<PokemonId, ConditionApplyError> {
    targets
        .iter()
        .find_map(|target| match target {
            ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
            _ => None,
        })
        .ok_or(ConditionApplyError::InvalidTarget)
}

/// `onHit`: the target becomes `type_` alone (`setType`) — failing when it is that type alone
/// already (`getTypes().join()`, asked when `checks_same`) or its types are locked.
fn become_type(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
    type_: PokemonType,
    move_id: MoveId,
    checks_same: bool,
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    let already = ctx
        .types(target)
        .is_some_and(|types| types.iter().eq([type_]));
    let source = EffectSource::Move {
        pokemon: user,
        move_id,
    };
    if (checks_same && already) || !ctx.set_type(target, PokemonTypes::mono(type_), source) {
        return Err(ConditionApplyError::Immune);
    }
    Ok(())
}

/// Soak: the target becomes pure Water. Gen 6+ fails on one that already is
/// (`TypeRules::soak_fails_if_same`; the gen5 `onHit` asks only `setType`).
pub fn soak(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let checks_same = ctx.rules().conditions().type_change.soak_fails_if_same;
    become_type(
        ctx,
        user,
        targets,
        PokemonType::Water,
        MoveId::SOAK,
        checks_same,
    )
}

/// Magic Powder: the target becomes pure Psychic, failing on one that already is.
pub fn magic_powder(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    become_type(
        ctx,
        user,
        targets,
        PokemonType::Psychic,
        MoveId::MAGIC_POWDER,
        true,
    )
}

/// Camouflage (`TypeRules::camouflage`): Gen 3-4 Normal, failing for a user that has Normal;
/// Gen 5 Ground; Gen 6-7 the terrain's type (Normal without one), failing for a user that is
/// that type alone.
pub fn camouflage(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let user_target = [ResolvedTarget::Pokemon(user)];
    match ctx.rules().conditions().type_change.camouflage {
        Camouflage::Normal => {
            if ctx.has_type(user, PokemonType::Normal) {
                return Err(ConditionApplyError::Immune);
            }
            become_type(
                ctx,
                user,
                &user_target,
                PokemonType::Normal,
                MoveId::CAMOUFLAGE,
                false,
            )
        }
        Camouflage::Ground => become_type(
            ctx,
            user,
            &user_target,
            PokemonType::Ground,
            MoveId::CAMOUFLAGE,
            false,
        ),
        Camouflage::ByTerrain => {
            let type_ = match ctx.terrain() {
                Some(TerrainKind::Electric) => PokemonType::Electric,
                Some(TerrainKind::Grassy) => PokemonType::Grass,
                Some(TerrainKind::Misty) => PokemonType::Fairy,
                Some(TerrainKind::Psychic) => PokemonType::Psychic,
                None => PokemonType::Normal,
            };
            become_type(ctx, user, &user_target, type_, MoveId::CAMOUFLAGE, true)
        }
    }
}

/// `onHit`: fails on a target that has `type_` already (`hasType`, the added one included),
/// else adds it (`addType`, replacing an added type before).
fn add_type(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
    type_: PokemonType,
    move_id: MoveId,
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    if ctx.has_type(target, type_) {
        return Err(ConditionApplyError::Immune);
    }
    ctx.add_type(
        target,
        type_,
        EffectSource::Move {
            pokemon: user,
            move_id,
        },
    );
    Ok(())
}

/// Forest's Curse: adds Grass.
pub fn forests_curse(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    add_type(
        ctx,
        user,
        targets,
        PokemonType::Grass,
        MoveId::FORESTS_CURSE,
    )
}

/// Trick-or-Treat: adds Ghost; a second-slot target about to Curse aims it at its ally.
pub fn trick_or_treat(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    add_type(
        ctx,
        user,
        targets,
        PokemonType::Ghost,
        MoveId::TRICK_OR_TREAT,
    )?;
    // The "Curse glitch" (`BattleContext::curse_glitch`).
    ctx.curse_glitch(target_of(targets)?);
    Ok(())
}

/// Reflect Type: fails for a user whose types are locked (Arceus, Silvally); the user takes
/// the target's types without the added one and without `???` (`getTypes(true)`) — Normal when
/// none is left but an added type, else it fails — then the target's added type.
pub fn reflect_type(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    if ctx.types_locked(user) {
        return Err(ConditionApplyError::Immune);
    }
    let Some(theirs) = ctx.types(target) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let mut kept = theirs
        .without_added()
        .filter(|t| *t != PokemonType::Typeless);
    let types = match (kept.next(), kept.next()) {
        (Some(primary), None) => PokemonTypes::mono(primary),
        (Some(primary), Some(secondary)) => PokemonTypes::dual(primary, secondary),
        (None, _) if theirs.added.is_some() => PokemonTypes::mono(PokemonType::Normal),
        (None, _) => return Err(ConditionApplyError::Immune),
    };
    let source = EffectSource::Move {
        pokemon: user,
        move_id: MoveId::REFLECT_TYPE,
    };
    ctx.set_type(user, types, source);
    if let Some(added) = theirs.added {
        ctx.add_type(user, added, source);
    }
    Ok(())
}

/// Electrify: `onTryHit` fails on a target that has moved this turn (`!queue.willMove(target) &&
/// target.activeTurns`); the target gets `electrify` for the turn (`-singleturn`).
pub fn electrify(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    let moved = !ctx.will_move(target) && ctx.active_turns(target) > 0;
    if moved {
        return Err(ConditionApplyError::Immune);
    }
    ctx.set_marker_volatile(target, crate::state::VolatileConditionKind::Electrify)
}

/// Conversion (`TypeRules::conversion`): Gen 1 the user takes the target's types; Gen 2-5 one of
/// `engine::conversion_candidates`, drawn (`sample`), failing without one; Gen 6+ its first
/// slot's type, failing for a user that has it. `setType` refusing fails it too.
pub fn conversion(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let source = EffectSource::Move {
        pokemon: user,
        move_id: MoveId::CONVERSION,
    };
    let types = match ctx.rules().conditions().type_change.conversion {
        Conversion::CopyTarget => {
            let target = target_of(targets)?;
            let theirs = ctx
                .types(target)
                .ok_or(ConditionApplyError::InvalidTarget)?;
            let mut own = theirs.without_added();
            match (own.next(), own.next()) {
                (Some(primary), Some(secondary)) => PokemonTypes::dual(primary, secondary),
                (Some(primary), None) => PokemonTypes::mono(primary),
                (None, _) => return Err(ConditionApplyError::Immune),
            }
        }
        Conversion::RandomMoveType { .. } => {
            let candidates = ctx.conversion_candidates(user);
            if candidates.is_empty() {
                return Err(ConditionApplyError::Immune);
            }
            PokemonTypes::mono(candidates[ctx.sample_index(candidates.len())])
        }
        Conversion::FirstSlot => {
            let first = ctx
                .first_slot_type(user)
                .ok_or(ConditionApplyError::Immune)?;
            if ctx.has_type(user, first) {
                return Err(ConditionApplyError::Immune);
            }
            PokemonTypes::mono(first)
        }
    };
    if ctx.set_type(user, types, source) {
        Ok(())
    } else {
        Err(ConditionApplyError::Immune)
    }
}

/// Conversion 2: the user becomes one of `engine::conversion2_candidates` (the types that
/// resist the target's last move), drawn (`sample`); it fails on a target that has used no
/// move, without a candidate, or when `setType` refuses.
pub fn conversion2(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    let candidates = ctx.conversion2_candidates(user, target);
    if candidates.is_empty() {
        return Err(ConditionApplyError::Immune);
    }
    let type_ = candidates[ctx.sample_index(candidates.len())];
    let source = EffectSource::Move {
        pokemon: user,
        move_id: MoveId::CONVERSION_2,
    };
    if ctx.set_type(user, PokemonTypes::mono(type_), source) {
        Ok(())
    } else {
        Err(ConditionApplyError::Immune)
    }
}
