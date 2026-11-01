//! Pain Split (Showdown `onHit`, DESIGN.md §20.1, R164): the user and the target both end at
//! the average of their HP (floored, at least 1), each capped at its own max — set outright
//! (`sethp`, `BattleContext::set_hp`), not damage or a heal.

use crate::engine::{BattleContext, ConditionApplyError, EffectSource};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let (Some(target_undynamaxed), Some(target_actual), Some(user_hp)) = (
        ctx.battle_state()
            .pokemon(target)
            .map(|pokemon| pokemon.undynamaxed_hp(None)),
        ctx.hp(target),
        ctx.hp(user),
    ) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    // Showdown's `onHit` reads the target's HP through `getUndynamaxedHP()` (DESIGN.md §44.5):
    // the average is in that space, and the target's new HP is the difference taken off its
    // actual HP (the doubling stays).
    let average = ((u32::from(target_undynamaxed) + u32::from(user_hp)) / 2).max(1) as u16;
    let target_change = i32::from(target_undynamaxed) - i32::from(average);
    let target_new = (i32::from(target_actual) - target_change).max(1) as u16;
    let source = EffectSource::Move {
        pokemon: user,
        move_id: MoveId::PAIN_SPLIT,
    };
    ctx.set_hp(target, target_new, source);
    ctx.set_hp(user, average, source);
    Ok(())
}
