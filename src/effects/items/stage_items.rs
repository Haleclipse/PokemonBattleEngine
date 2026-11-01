//! Items that act on their holder's stat stages whenever a turn's step lets them look (DESIGN.md
//! R233, R247): White Herb (Showdown `onStart`, `onAnySwitchIn` −2, `onAnyAfterMove`,
//! `onResidual` 29, `onUse`) and Mirror Herb (`onFoeAfterBoost`, `onAnySwitchIn` −3,
//! `onAnyAfterMove`, `onResidual` 29, `onUse`); their `onAnyAfterMega` (and Mirror Herb's
//! `onAnyAfterTerastallization`) wait for Mega Evolution and Terastallization, family O.

use crate::effects::{ResidualHandler, ResidualParticipant};
use crate::engine::{
    BattleContext, EffectSource, StatChangeCause, StatChangeRequest, StatChangeResult,
};
use crate::ids::{AbilityId, ItemId, PokemonId};
use crate::rules::ResidualKind;

use super::{ItemAfterBoostHandler, ItemBehavior};

pub(super) fn white_herb() -> ItemBehavior {
    ItemBehavior {
        switch_in: Some(restore_lowered_stats),
        any_switch_in: Some((-2, restore_lowered_stats)),
        any_after_move: Some(restore_lowered_stats),
        // Showdown `onAnyAfterMega` (DESIGN.md §44.3): the same check.
        any_after_mega: Some(restore_lowered_stats),
        residual: Some(ResidualParticipant {
            kind: ResidualKind::WhiteHerb,
            handler: restore_lowered_stats as ResidualHandler,
        }),
        restores_lowered_stats: true,
        fling_effect: Some(crate::effects::items::FlingEffect::ClearLoweredStages),
        ..ItemBehavior::default()
    }
}

/// White Herb's check: a holder standing on the field with a stat stage below 0 uses the herb
/// up (`useItem`) and every lowered stage goes back to 0 (`setBoost`, no stat change event).
pub fn restore_lowered_stats(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some((item, behavior)) = ctx.held_item(holder) else {
        return;
    };
    if !behavior.restores_lowered_stats || ctx.is_fainted(holder) || !ctx.is_active(holder) {
        return;
    }
    if !has_lowered_stage(ctx, holder) || ctx.consume_item(holder).is_none() {
        return;
    }
    ctx.clear_lowered_stages(
        holder,
        EffectSource::Item {
            pokemon: holder,
            item,
        },
    );
}

/// Whether any of `pokemon`'s stat stages is below 0.
fn has_lowered_stage(ctx: &BattleContext<'_>, pokemon: PokemonId) -> bool {
    crate::engine::STAGED_STATS
        .into_iter()
        .any(|stat| ctx.stage(pokemon, stat) < 0)
}

pub(super) fn mirror_herb() -> ItemBehavior {
    ItemBehavior {
        foe_after_boost: Some(keep_a_foes_raise as ItemAfterBoostHandler),
        any_switch_in: Some((-3, use_kept_raises)),
        any_after_move: Some(use_kept_raises),
        any_after_terastallization: Some(use_kept_raises),
        any_after_mega: Some(use_kept_raises),
        residual: Some(ResidualParticipant {
            kind: ResidualKind::MirrorHerb,
            handler: use_kept_raises as ResidualHandler,
        }),
        copies_foe_raises: true,
        ..ItemBehavior::default()
    }
}

/// Mirror Herb's `onFoeAfterBoost`: a foe's stage that went up (Showdown's capped
/// `boost[i] > 0`) is kept, added to what the herb already kept of that stat, the stats in the
/// order they first came — but not a raise another Mirror Herb or Opportunist passed on
/// (`effect.name`).
fn keep_a_foes_raise(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    request: StatChangeRequest,
    result: StatChangeResult,
) {
    let StatChangeResult::Applied { from, to } = result else {
        return;
    };
    let passed_on = matches!(
        request.source,
        EffectSource::Item {
            item: ItemId::MIRROR_HERB,
            ..
        } | EffectSource::Ability {
            ability: AbilityId::OPPORTUNIST,
            ..
        }
    );
    if to <= from || passed_on {
        return;
    }
    let Some(state) = ctx.item_state_mut(holder) else {
        return;
    };
    match state
        .kept_raises
        .iter_mut()
        .find(|(stat, _)| *stat == request.stat)
    {
        Some((_, kept)) => *kept = kept.saturating_add(to - from),
        None => state.kept_raises.push((request.stat, to - from)),
    }
}

/// Mirror Herb's use: a holder on the field that kept a raise uses the herb up (`useItem`) and
/// gets the raises (`boost`, the herb its source), in the order they were kept.
pub fn use_kept_raises(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some((item, behavior)) = ctx.held_item(holder) else {
        return;
    };
    if !behavior.copies_foe_raises || ctx.is_fainted(holder) || !ctx.is_active(holder) {
        return;
    }
    let raises = ctx
        .item_state(holder)
        .map(|state| state.kept_raises.clone())
        .unwrap_or_default();
    if raises.is_empty() || ctx.consume_item(holder).is_none() {
        return;
    }
    for (stat, stages) in raises {
        ctx.try_change_stat_stage(StatChangeRequest {
            target: holder,
            stat,
            stages,
            source: EffectSource::Item {
                pokemon: holder,
                item,
            },
            cause: StatChangeCause::Item,
        });
    }
}
