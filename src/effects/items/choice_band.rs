use crate::dex::MoveCategory;
use crate::engine::{BattleContext, OffensiveStatContext};
use crate::ids::{MoveId, PokemonId};

/// Choice Band: `chainModify(1.5)` on physical moves.
pub fn modify_offense(ctx: &mut OffensiveStatContext<'_>) -> Option<u32> {
    (ctx.move_category() == MoveCategory::Physical).then_some(6144)
}

/// Showdown `onModifyMove`: the first move used while holding the band locks the holder into
/// it (BeforeMove refuses anything else; choice validation mirrors the request-side lock).
/// Choice Specs: the same lock, ×1.5 on Special Attack.
pub fn modify_special_offense(ctx: &mut OffensiveStatContext<'_>) -> Option<u32> {
    (ctx.move_category() == MoveCategory::Special).then_some(6144)
}

pub fn move_decided(ctx: &mut BattleContext<'_>, holder: PokemonId, move_id: MoveId) {
    ctx.set_choice_lock(holder, move_id);
}

/// Showdown `onStart`: entering the field clears any lock (volatiles are cleared on switch,
/// so this is the explicit, idempotent statement of it).
pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.clear_choice_lock(holder);
}
