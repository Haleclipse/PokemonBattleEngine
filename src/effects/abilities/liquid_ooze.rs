//! Liquid Ooze (Showdown `onSourceTryHeal`, DESIGN.md §20.2, R168): a heal taken off the
//! holder hurts the one taking it instead — as the generation has it
//! (`ConditionRules::liquid_ooze`): a drain (not Dream Eater's in Gen 3-4), Leech Seed and
//! Strength Sap; the damage knocks out at once in Gen 3-6.

use crate::engine::{
    BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest, DrainedHeal, EffectSource,
    HealDrain,
};
use crate::ids::{AbilityId, MoveId};

pub fn liquid_ooze(ctx: &mut BattleContext<'_>, heal: DrainedHeal) -> bool {
    let rules = ctx.rules().conditions().liquid_ooze;
    let oozes = match heal.drain {
        HealDrain::Move(move_id) => !(rules.spares_dream_eater && move_id == MoveId::DREAM_EATER),
        HealDrain::LeechSeed | HealDrain::StrengthSap => true,
    };
    if !oozes {
        return false;
    }
    ctx.apply_damage(DamageRequest {
        target: heal.receiver,
        amount: DamageAmount(heal.amount),
        kind: DamageKind::Reaction,
        source: EffectSource::Ability {
            pokemon: heal.drained,
            ability: AbilityId::LIQUID_OOZE,
        },
        flags: DamageFlags {
            instafaint: rules.instafaint,
            ..DamageFlags::default()
        },
    });
    true
}
