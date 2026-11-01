//! Toxic Debris (`onDamagingHit`, no order: after every ordered reaction): a physical hit on the
//! holder lays a layer of Toxic Spikes on its foes' side (the attacker's, or the foes' when an
//! ally hit it) while fewer than two are down — even as the hit knocks the holder out.

use crate::dex::MoveCategory;
use crate::engine::{BattleContext, DamagingHit};
use crate::ids::AbilityId;
use crate::state::SideConditionKind;

pub fn damaging_hit(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if hit.category != MoveCategory::Physical {
        return;
    }
    let Some(side) = ctx.side_of(hit.holder).map(|side| side.other()) else {
        return;
    };
    if ctx.hazard_layers(side, SideConditionKind::ToxicSpikes) >= 2 {
        return;
    }
    ctx.ability_activated(hit.holder, AbilityId::TOXIC_DEBRIS);
    ctx.lay_hazard(side, SideConditionKind::ToxicSpikes, hit.holder);
}
