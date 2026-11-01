//! Items the field makes their holder use up (DESIGN.md R199): Electric, Grassy, Misty and
//! Psychic Seed (Showdown `onStart` / `onTerrainChange`) and Room Service (`onStart` /
//! `onAnyPseudoWeatherChange`), each `useItem` then its `boosts`. Their `onStart` runs at
//! `onSwitchInPriority` −1.

use crate::engine::BattleContext;
use crate::ids::PokemonId;
use crate::state::{FieldConditionKind, Stat};

use super::{FieldItemWhen, ItemBehavior, UsedOnField};

pub(super) fn used_on_field(when: FieldItemWhen, boosts: &'static [(Stat, i8)]) -> ItemBehavior {
    ItemBehavior {
        used_on_field: Some(UsedOnField { when, boosts }),
        switch_in: Some(use_if_the_field_has_it),
        switch_in_priority: -1,
        ..ItemBehavior::default()
    }
}

/// `onStart`, `onTerrainChange`, `onAnyPseudoWeatherChange`: the holder uses the item up while
/// the field has what it is for.
pub fn use_if_the_field_has_it(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some((item, used)) = ctx
        .held_item(holder)
        .and_then(|(item, behavior)| behavior.used_on_field.map(|used| (item, used)))
    else {
        return;
    };
    let has_it = match used.when {
        FieldItemWhen::Terrain(kind) => ctx.terrain() == Some(kind),
        FieldItemWhen::TrickRoom => ctx.field_has(FieldConditionKind::TrickRoom),
    };
    if has_it {
        super::hit_items::use_for_boosts(ctx, holder, item, used.boosts);
    }
}
