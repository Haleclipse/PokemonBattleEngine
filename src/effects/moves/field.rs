//! The pseudo-weather moves (DESIGN.md §23.4): Gravity, Wonder Room, Ion Deluge, Mud Sport,
//! Water Sport, Fairy Lock — and the slot conditions of Wish, Healing Wish, Lunar Dance and
//! Revival Blessing; the work is `BattleContext`'s
//! (`engine/context/field.rs`).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::FieldConditionKind;

fn started(started: bool) -> Result<(), ConditionApplyError> {
    if started {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}

pub fn gravity(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    started(ctx.start_gravity())
}

pub fn magic_room(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.toggle_magic_room();
    Ok(())
}

pub fn wonder_room(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.toggle_wonder_room();
    Ok(())
}

pub fn ion_deluge(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    started(ctx.start_ion_deluge())
}

pub fn fairy_lock(
    ctx: &mut BattleContext<'_>,
    _user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let turns = ctx.rules().conditions().field.fairy_lock_turns;
    started(ctx.start_pseudo_weather(FieldConditionKind::FairyLock, turns))
}

pub fn mud_sport(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.start_sport(user, FieldConditionKind::MudSport)
}

pub fn water_sport(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.start_sport(user, FieldConditionKind::WaterSport)
}

pub fn wish(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.make_wish(user)
}

pub fn healing_wish(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.wish_for_the_next(user, crate::state::SlotHealKind::HealingWish)
}

pub fn lunar_dance(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.wish_for_the_next(user, crate::state::SlotHealKind::LunarDance)
}

pub fn revival_blessing(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.bless_revival(user)
}
