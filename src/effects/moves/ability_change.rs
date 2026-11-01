//! The moves that change abilities (DESIGN.md §41.3, K.3): Skill Swap trades the user's and the
//! target's; Role Play copies the target's onto the user, Doodle onto the user and its allies;
//! Entrainment gives the target the user's, Simple Beam gives it Simple, Worry Seed Insomnia
//! (`BattleContext::set_ability`).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::{AbilityId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::VolatileConditionKind;

fn target_of(targets: &[ResolvedTarget]) -> Result<PokemonId, ConditionApplyError> {
    targets
        .iter()
        .find_map(|target| match target {
            ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
            _ => None,
        })
        .ok_or(ConditionApplyError::InvalidTarget)
}

/// Skill Swap (`onHit`: `battle.skillSwap(source, target)`).
pub fn skill_swap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    if ctx.skill_swap(user, target) {
        // Gen 4: both start Truant's count over (§15.21).
        ctx.restart_truant_parity(user);
        ctx.restart_truant_parity(target);
        Ok(())
    } else {
        Err(ConditionApplyError::Immune)
    }
}

/// Role Play: `onTryHit` refuses the same ability (Gen 5+: `AbilityChangeRules::role_play_same_fails`;
/// the Gen 3-4 cartridge has no such check, §41.10 A6), a target's `failroleplay` one and a
/// user's that cannot be suppressed; `onHit` gives the user the target's ability (`setAbility`,
/// which the user's Ability Shield, or Gen 4 Griseous Orb, refuses).
pub fn role_play(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    let (Some(mine), Some(theirs)) = (ctx.raw_ability_of(user), ctx.raw_ability_of(target)) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let rules = ctx.rules().conditions();
    if (rules.ability_change.role_play_same_fails && mine == theirs)
        || rules.fail_role_play.contains(&theirs)
        || rules.cant_suppress.contains(&mine)
    {
        return Err(ConditionApplyError::Immune);
    }
    ctx.set_ability(user, theirs, Some(target), None)
        .map(|_| ctx.restart_truant_parity(user))
        .ok_or(ConditionApplyError::Immune)
}

/// Entrainment: `onTryHit` refuses the same ability, a target's that cannot be suppressed or is
/// Truant, and a user's `noentrain` one; `onHit` gives the target the user's ability
/// (`setAbility`, which the target's Ability Shield refuses).
pub fn entrainment(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    // `entrainment.onTryHit`: a Dynamaxed target refuses outright (DESIGN.md §44.5).
    if ctx
        .battle_state()
        .pokemon(target)
        .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::Dynamax))
    {
        return Err(ConditionApplyError::Immune);
    }
    let (Some(mine), Some(theirs)) = (ctx.raw_ability_of(user), ctx.raw_ability_of(target)) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let rules = ctx.rules().conditions();
    if mine == theirs
        || rules.cant_suppress.contains(&theirs)
        || theirs == AbilityId::TRUANT
        || rules.no_entrain.contains(&mine)
    {
        return Err(ConditionApplyError::Immune);
    }
    ctx.set_ability(target, mine, Some(user), None)
        .ok_or(ConditionApplyError::Immune)?;
    // A foe's target is stale while it stays in (`volatileStaleness`, DESIGN.md §45.7).
    ctx.make_volatile_stale(target, user);
    Ok(())
}

/// Simple Beam: `onTryHit` refuses a target whose ability cannot be suppressed, or is Simple or
/// Truant already; `onHit` gives it Simple.
pub fn simple_beam(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    let Some(theirs) = ctx.raw_ability_of(target) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    if ctx.rules().conditions().cant_suppress.contains(&theirs)
        || theirs == AbilityId::SIMPLE
        || theirs == AbilityId::TRUANT
    {
        return Err(ConditionApplyError::Immune);
    }
    ctx.set_ability(target, AbilityId::SIMPLE, None, None)
        .map(|_| ())
        .ok_or(ConditionApplyError::Immune)
}

/// Worry Seed: `onTryImmunity` refuses a target whose ability is Truant or Insomnia already
/// (`TryImmunity::NotTruantOrInsomnia`), `onTryHit` one whose ability cannot be suppressed;
/// `onHit` gives it Insomnia (`setAbility`, which Ability Shield or a Gen 4 Griseous Orb
/// refuses) and wakes it (`cureStatus`) — DESIGN.md R270.
pub fn worry_seed(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    let Some(theirs) = ctx.raw_ability_of(target) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    if ctx.rules().conditions().cant_suppress.contains(&theirs) {
        return Err(ConditionApplyError::Immune);
    }
    ctx.set_ability(target, AbilityId::INSOMNIA, None, None)
        .ok_or(ConditionApplyError::Immune)?;
    if ctx.status_of(target) == Some(crate::state::NonVolatileStatus::Sleep) {
        ctx.cure_status(
            target,
            crate::engine::CureReason::Move(crate::ids::MoveId::WORRY_SEED),
        );
    }
    Ok(())
}

/// Doodle (`onHit`): unless the target's ability is `failroleplay`, the user and each standing
/// ally, slot by slot, take it — those with it already or one that cannot be suppressed aside.
/// The move never fails (`NOT_FAIL`; Showdown shows `-fail` when nobody took it).
pub fn doodle(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = target_of(targets)?;
    let Some(theirs) = ctx.raw_ability_of(target) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let rules = ctx.rules().conditions();
    if rules.fail_role_play.contains(&theirs) {
        return Ok(());
    }
    let side = ctx.side_of(user);
    let members: Vec<PokemonId> = ctx
        .active_pokemon()
        .into_iter()
        .filter(|member| ctx.side_of(*member) == side && !ctx.is_fainted(*member))
        .collect();
    for member in members {
        let Some(ability) = ctx.raw_ability_of(member) else {
            continue;
        };
        if ability == theirs || rules.cant_suppress.contains(&ability) {
            continue;
        }
        let _ = ctx.set_ability(member, theirs, None, None);
    }
    Ok(())
}
