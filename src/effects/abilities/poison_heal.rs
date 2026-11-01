//! Poison Heal (Showdown `onDamage`, priority 1; DESIGN.md §20.2, R129): poison and bad poison
//! heal the holder by 1/8 of its max HP instead of hurting it. The damage is refused even when
//! the heal is not (full HP, Heal Block).

use crate::engine::{BattleContext, DamageDecision, DamageRequest, EffectSource, HealRequest};
use crate::ids::AbilityId;
use crate::state::NonVolatileStatus;

pub fn damage_resolution(ctx: &mut BattleContext<'_>, request: &DamageRequest) -> DamageDecision {
    if !matches!(
        request.source,
        EffectSource::Status(NonVolatileStatus::Poison | NonVolatileStatus::Toxic)
    ) {
        return DamageDecision::Allow;
    }
    if let Some(max_hp) = ctx.base_max_hp(request.target) {
        ctx.apply_heal(HealRequest {
            target: request.target,
            amount: (max_hp / 8).max(1),
            source: EffectSource::Ability {
                pokemon: request.target,
                ability: AbilityId::POISON_HEAL,
            },
            direct: false,
        });
    }
    DamageDecision::Prevent
}
