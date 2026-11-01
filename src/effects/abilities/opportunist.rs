//! Opportunist (DESIGN.md §44.1): Mirror Herb as an ability (R247). A raise its foes get
//! (Showdown `onFoeAfterBoost`) is kept, and at the next `onAnySwitchIn` (−3), `onAnyAfterMove`
//! or `onResidual` (29) the holder gets the raises itself (`boost`, the ability its source) — no
//! item to use up, so it copies again and again. Its `onEnd` drops what it kept: the kept raises
//! live in the ability's state (`AbilityState::opportunist`), which a new ability or a new stint
//! on the field starts over. Its `onAnyAfterMega` and `onAnyAfterTerastallization` come with
//! those gimmicks (DESIGN.md §44.3, §44.6).

use crate::engine::{
    BattleContext, EffectSource, StatChangeCause, StatChangeRequest, StatChangeResult,
};
use crate::ids::{AbilityId, ItemId, PokemonId};

/// `onFoeAfterBoost`: a foe's stage that went up (Showdown's capped `boost[i] > 0`) is kept, but
/// not a raise another Opportunist or a Mirror Herb passed on (`effect.name`).
pub(super) fn keep_a_foes_raise(
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
    if let Some(state) = ctx.ability_state_mut(holder) {
        state.opportunist.add(request.stat, to - from);
    }
}

/// The copy: the holder gets the raises it kept, in the order they were kept, and keeps none.
/// Only a holder on the field runs it; one that fainted gets nothing (`boost` refuses it, as the
/// stat change does here) and keeps nothing either.
pub fn copy_kept_raises(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(raises) = ctx
        .ability_state_mut(holder)
        .map(|state| state.opportunist.take())
    else {
        return;
    };
    for (stat, stages) in raises {
        ctx.try_change_stat_stage(StatChangeRequest {
            target: holder,
            stat,
            stages,
            source: EffectSource::Ability {
                pokemon: holder,
                ability: AbilityId::OPPORTUNIST,
            },
            cause: StatChangeCause::Ability,
        });
    }
}
