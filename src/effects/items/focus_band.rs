//! Focus Band (Showdown `onDamage`, priority -40: after Sturdy): a move's damage that would knock
//! the holder out leaves it at 1 on the generation's chance (`ConditionRules::focus_band_chance`);
//! the band stays held. Showdown rolls the chance for every damage the holder takes; only one
//! that would knock it out from a move can change anything, so only that one draws here. Where
//! the band lasts the move (Gen 3-4, `ConditionRules::focus_band_lasts_the_move`), a move's hits
//! are the `MovePipeline`'s (`HoldsForTheMove::OnChance`); this handler still takes the rest of a
//! move's damage — a confusion self-hit, a crash.

use crate::engine::{BattleContext, DamageAmount, DamageDecision, DamageKind, DamageRequest};
use crate::ids::ItemId;
use crate::state::RandomRequestKind;

pub fn damage_resolution(ctx: &mut BattleContext<'_>, request: &DamageRequest) -> DamageDecision {
    let Some(hp) = ctx.hp(request.target) else {
        return DamageDecision::Allow;
    };
    let rules = ctx.rules();
    if !request.from_a_move(&rules)
        || request.amount.0 < hp
        || (rules.conditions().focus_band_lasts_the_move && request.kind == DamageKind::Move)
    {
        return DamageDecision::Allow;
    }
    let chance = rules.conditions().focus_band_chance;
    if ctx.chance(RandomRequestKind::FocusBand, chance) {
        ctx.item_activated(request.target, ItemId::FOCUS_BAND);
        DamageDecision::Reduce(DamageAmount(hp - 1))
    } else {
        DamageDecision::Allow
    }
}
