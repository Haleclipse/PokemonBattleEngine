//! Berries eaten at an `Update` (DESIGN.md §40.3, `ItemUpdate::Eat`): the status berries while
//! the holder has the status (or confusion) they cure, the HP berries once its HP is low enough;
//! eating one does its `EatEffect`. Lum Berry also eats itself the moment a status is set
//! (`onAfterSetStatus`, priority −1: after every other listener).

use crate::engine::{BattleContext, EffectSource};
use crate::ids::PokemonId;
use crate::state::NonVolatileStatus;

use super::{
    EatEffect, EatTrigger, ItemBehavior, ItemUpdate, ResidualHandler, ResidualParticipant,
};
use crate::rules::{BerryHeal, HpShare, ResidualKind};

pub(super) const PARALYSIS: &[NonVolatileStatus] = &[NonVolatileStatus::Paralysis];
pub(super) const SLEEP: &[NonVolatileStatus] = &[NonVolatileStatus::Sleep];
pub(super) const POISON: &[NonVolatileStatus] =
    &[NonVolatileStatus::Poison, NonVolatileStatus::Toxic];
pub(super) const BURN: &[NonVolatileStatus] = &[NonVolatileStatus::Burn];
pub(super) const FREEZE: &[NonVolatileStatus] = &[NonVolatileStatus::Freeze];

/// Cheri, Chesto, Pecha, Rawst, Aspear Berry and their Gen 2 forerunners: eaten while the
/// holder has one of `statuses`, which eating cures.
pub(super) fn status_berry(statuses: &'static [NonVolatileStatus]) -> ItemBehavior {
    ItemBehavior {
        update: Some(ItemUpdate::Eat(EatTrigger::Status(statuses))),
        eat_effect: Some(EatEffect::CureStatus(statuses)),
        ..ItemBehavior::default()
    }
}

/// Persim Berry, Bitter Berry: eaten while the holder is confused, which eating ends.
pub(super) fn confusion_berry() -> ItemBehavior {
    ItemBehavior {
        update: Some(ItemUpdate::Eat(EatTrigger::Confused)),
        eat_effect: Some(EatEffect::CureConfusion),
        ..ItemBehavior::default()
    }
}

/// Lum Berry, Miracle Berry: eaten while the holder has any status or is confused, and cures
/// both. Lum Berry's `onAfterSetStatus` only exists where the generation's item declares it.
pub(super) fn cure_all_berry() -> ItemBehavior {
    ItemBehavior {
        update: Some(ItemUpdate::Eat(EatTrigger::StatusOrConfused)),
        eat_effect: Some(EatEffect::CureStatusAndConfusion),
        after_set_status: Some(eat_on_status),
        ..ItemBehavior::default()
    }
}

/// Oran Berry, Sitrus Berry, Gen 2's Berry and Gold Berry: eaten at half HP — at an `Update`
/// (Gen 4+ `onUpdate`) or in the residual (Gen 2-3 `onResidual`; each generation's item has one
/// of the two).
pub(super) fn healing_berry(heal: EatEffect) -> ItemBehavior {
    ItemBehavior {
        update: Some(ItemUpdate::Eat(EatTrigger::HpAtMost(HpShare::Half))),
        residual: Some(ResidualParticipant {
            kind: ResidualKind::Berry,
            handler: eat_at_half as ResidualHandler,
        }),
        eat_effect: Some(heal),
        ..ItemBehavior::default()
    }
}

/// Figy, Wiki, Mago, Aguav, Iapapa Berry: eaten at the generation's share of HP (Gen 3 in the
/// residual); a holder whose nature lowers `disliked` gets confused.
pub(super) fn confusion_berry_heal(disliked: crate::state::Stat) -> ItemBehavior {
    ItemBehavior {
        update: Some(ItemUpdate::Eat(EatTrigger::ConfusionBerryHp)),
        residual: Some(ResidualParticipant {
            kind: ResidualKind::Berry,
            handler: eat_confusion_berry as ResidualHandler,
        }),
        eat_effect: Some(EatEffect::ConfusionBerryHeal(disliked)),
        ..ItemBehavior::default()
    }
}

/// Leppa Berry, Mystery Berry: eaten once a move of the holder's has no PP left — at an
/// `Update` (Gen 4+), at the end of the turn (Gen 2-3, `EatTrigger::at_turn_end`: the generation's
/// item declares no `onResidual`, the residual feeds it all the same) — for `amount` PP.
pub(super) fn pp_berry(amount: u8) -> ItemBehavior {
    ItemBehavior {
        update: Some(ItemUpdate::Eat(EatTrigger::MoveOutOfPp)),
        eat_effect: Some(EatEffect::RestorePp(amount)),
        ..ItemBehavior::default()
    }
}

/// Liechi, Ganlon, Salac, Petaya, Apicot, Lansat, Starf Berry: eaten at a quarter of the HP
/// (half with Gluttony) — at an `Update` (Gen 4+ `onUpdate`) or in the residual (Gen 3
/// `onResidual`).
pub(super) fn pinch_berry(effect: EatEffect) -> ItemBehavior {
    ItemBehavior {
        update: Some(ItemUpdate::Eat(EatTrigger::HpAtMost(HpShare::Pinch))),
        residual: Some(ResidualParticipant {
            kind: ResidualKind::Berry,
            handler: eat_at_pinch as ResidualHandler,
        }),
        eat_effect: Some(effect),
        ..ItemBehavior::default()
    }
}

/// The residual's `onResidual` of a Gen 3 pinch berry: eaten at a quarter of the HP, while its
/// effect has room (issues.md I37).
pub(super) fn eat_at_pinch(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.berry_hp_reached(holder, HpShare::Pinch) && ctx.berry_has_room(holder) {
        ctx.eat_item(holder);
    }
}

/// The residual's `onResidual` of an HP berry (Gen 2-3): eaten at half HP.
fn eat_at_half(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.berry_hp_reached(holder, HpShare::Half) {
        ctx.eat_item(holder);
    }
}

/// The residual's `onResidual` of a Gen 3 confusion berry.
fn eat_confusion_berry(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let share = ctx.rules().conditions().berries.confusion_berry_at;
    if ctx.berry_hp_reached(holder, share) {
        ctx.eat_item(holder);
    }
}

/// Gen 2 Berry Juice's `onResidual`: used up at half HP for 20.
pub(super) fn use_juice(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.use_to_heal(holder, HpShare::Half, BerryHeal::Fixed(20));
}

/// Lum Berry's `onAfterSetStatus`: the holder eats it as soon as it gets a status.
fn eat_on_status(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    _status: NonVolatileStatus,
    _source: EffectSource,
) {
    ctx.eat_item(holder);
}
