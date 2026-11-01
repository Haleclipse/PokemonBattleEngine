//! Perish Song and the moves that send their user out (DESIGN.md §30.6, §30.7): Parting Shot,
//! Chilly Reception, Teleport, Baton Pass, Shed Tail. `selfSwitch` is the Shell's
//! (`after_move`, Baton Pass and Shed Tail handing things on); Parting Shot decides for itself.

use crate::engine::{
    BattleContext, ConditionApplyError, EffectSource, StatChangeCause, StatChangeRequest,
    StatChangeResult,
};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::{Stat, WeatherKind};

/// Perish Song (`onHitField`): every active Pokémon not already counting starts its count —
/// one semi-invulnerable in a charge is missed (`Invulnerability`, No Guard aside), one whose ability or item
/// refuses the move is spared (`TryHit`: Good as Gold). Fails when nobody was reached or
/// started.
pub fn perish_song(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let mut result = false;
    for pokemon in ctx.active_pokemon() {
        if ctx.is_fainted(pokemon) {
            continue;
        }
        if ctx.out_of_reach(user, pokemon) || ctx.holders_refuse(user, pokemon, MoveId::PERISH_SONG)
        {
            result = true;
        } else if !ctx.perishing(pokemon) {
            result |= ctx.start_perish_song(pokemon);
        }
    }
    if result {
        Ok(())
    } else {
        Err(ConditionApplyError::NothingToDo)
    }
}

/// Parting Shot (`onHit`): the target's Attack and Special Attack −1, then its user goes out
/// — from Gen 7 only if a stat went down or the target has Mirror Armor
/// (`SwitchingRules::parting_shot_needs_a_drop`).
pub fn parting_shot(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let target = targets
        .iter()
        .find_map(|target| match target {
            ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
            _ => None,
        })
        .ok_or(ConditionApplyError::InvalidTarget)?;
    let mut lowered = false;
    for stat in [Stat::Attack, Stat::SpecialAttack] {
        lowered |= matches!(
            ctx.try_change_stat_stage(StatChangeRequest {
                target,
                stat,
                stages: -1,
                source: EffectSource::Move {
                    pokemon: user,
                    move_id: MoveId::PARTING_SHOT,
                },
                cause: StatChangeCause::Move,
            }),
            StatChangeResult::Applied { .. }
        );
    }
    let mirror_armor = ctx.ability_of(target) == Some(AbilityId::MIRROR_ARMOR);
    if lowered || mirror_armor || !ctx.rules().conditions().switching.parting_shot_needs_a_drop {
        ctx.request_self_switch(user);
    }
    Ok(())
}

/// Chilly Reception: snow (a weather move's five turns), then its user goes out (the Shell's
/// `selfSwitch`). It fails only when the snow was already up and nobody can come in.
pub fn chilly_reception(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let snowed = ctx.set_weather_by_move(WeatherKind::Snow, user);
    let can_switch = ctx.side_of(user).is_some_and(|side| ctx.can_switch(side));
    if snowed || can_switch {
        Ok(())
    } else {
        Err(ConditionApplyError::NothingToDo)
    }
}

/// Teleport's `onTry`: Gen 8+ somebody to come in; Gen 1-7 it always fails
/// (`SwitchingRules::teleport_switches`).
pub fn teleport_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    let can_switch = ctx.side_of(user).is_some_and(|side| ctx.can_switch(side));
    if ctx.rules().conditions().switching.teleport_switches && can_switch {
        Ok(())
    } else {
        Err(ConditionApplyError::NothingToDo)
    }
}

/// Teleport, past its `onTry`: its user goes out (the Shell's `selfSwitch`).
pub fn teleport(
    _ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    Ok(())
}

/// Baton Pass (`onHit`): fails with nobody to come in; otherwise its `selfSwitch:
/// 'copyvolatile'` sends the user out, handing on its stat stages and volatiles. (Its
/// `self.onHit` only spares it Pursuit's `BeforeSwitchOut`, which the engine does not have.)
pub fn baton_pass(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    match ctx.side_of(user) {
        Some(side) if ctx.can_switch(side) => Ok(()),
        _ => Err(ConditionApplyError::NothingToDo),
    }
}

/// Shed Tail: a Substitute at the cost of half the user's HP (`BattleContext::shed_tail`), then
/// its `selfSwitch: 'shedtail'` hands the Substitute alone on.
pub fn shed_tail(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.shed_tail(user)
}
