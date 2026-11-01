//! Abilities refusing stat drops at the TryBoost stage besides Clear Body (Showdown
//! `onTryBoost`). A drop the holder causes itself (Close Combat, Superpower) always goes
//! through (`if (source && target === source) return`); each refused stat is simply not
//! changed, the rest of a multi-stat request (Tickle, Noble Roar) still applies.

use crate::engine::{EffectSource, StatChangeBlockReason, StatChangeContext, TryStatChangeResult};
use crate::ids::AbilityId;
use crate::state::Stat;

/// A drop someone else causes, of `stat` (or of any stat).
fn foe_drop(ctx: &StatChangeContext<'_>, stat: Option<Stat>) -> bool {
    ctx.stages() < 0 && !ctx.is_self_inflicted() && stat.is_none_or(|stat| stat == ctx.stat())
}

/// `shown`: the refusal's `-fail|holder|unboost|…|[from] ability: …` line is written.
fn refused(by: AbilityId, refuses: bool, shown: bool) -> TryStatChangeResult {
    if refuses {
        TryStatChangeResult::Blocked {
            reason: StatChangeBlockReason::Ability(by),
            shown,
        }
    } else {
        TryStatChangeResult::Allow
    }
}

/// Clear Body's kind and Big Pecks announce no secondary effect's drop, nor Octolock's
/// (`!effect.secondaries && effect.id !== 'octolock'`).
pub(super) fn announced_unless_octolock(ctx: &StatChangeContext<'_>) -> bool {
    !ctx.is_secondary() && !ctx.is_octolock()
}

/// White Smoke: every drop someone else causes, as Clear Body.
pub fn white_smoke(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(
        AbilityId::WHITE_SMOKE,
        foe_drop(ctx, None),
        announced_unless_octolock(ctx),
    )
}

/// Full Metal Body: every drop someone else causes, as Clear Body.
pub fn full_metal_body(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(
        AbilityId::FULL_METAL_BODY,
        foe_drop(ctx, None),
        announced_unless_octolock(ctx),
    )
}

/// Hyper Cutter: Attack drops someone else causes, announced but a secondary effect's.
pub fn hyper_cutter(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(
        AbilityId::HYPER_CUTTER,
        foe_drop(ctx, Some(Stat::Attack)),
        !ctx.is_secondary(),
    )
}

/// Big Pecks: Defense drops someone else causes.
pub fn big_pecks(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(
        AbilityId::BIG_PECKS,
        foe_drop(ctx, Some(Stat::Defense)),
        announced_unless_octolock(ctx),
    )
}

/// Intimidate's Attack change (`effect.name === 'Intimidate' && boost.atk`), whatever its sign:
/// Contrary has already turned the drop into a rise.
fn intimidate_attack(ctx: &StatChangeContext<'_>) -> bool {
    matches!(
        ctx.source(),
        EffectSource::Ability {
            ability: AbilityId::INTIMIDATE,
            ..
        }
    ) && ctx.stat() == Stat::Attack
}

/// Inner Focus (Gen 8+): Intimidate's Attack change, no other drop; always announced.
pub fn inner_focus(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(AbilityId::INNER_FOCUS, intimidate_attack(ctx), true)
}

/// Scrappy (Gen 8+; the gen7 mod has no `onTryBoost`): Intimidate's Attack change.
pub fn scrappy(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(AbilityId::SCRAPPY, intimidate_attack(ctx), true)
}

/// Keen Eye: accuracy drops someone else causes, announced but a secondary effect's.
pub fn keen_eye(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(
        AbilityId::KEEN_EYE,
        foe_drop(ctx, Some(Stat::Accuracy)),
        !ctx.is_secondary(),
    )
}

/// Illuminate (Gen 9): accuracy drops someone else causes, as Keen Eye.
pub fn illuminate(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(
        AbilityId::ILLUMINATE,
        foe_drop(ctx, Some(Stat::Accuracy)),
        !ctx.is_secondary(),
    )
}

/// Mind's Eye: accuracy drops someone else causes, as Keen Eye.
pub fn minds_eye(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(
        AbilityId::MINDS_EYE,
        foe_drop(ctx, Some(Stat::Accuracy)),
        !ctx.is_secondary(),
    )
}

/// Oblivious (Gen 8+; the gen7 mod has no `onTryBoost`): Intimidate's Attack change.
pub fn oblivious(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(AbilityId::OBLIVIOUS, intimidate_attack(ctx), true)
}

/// Own Tempo (Gen 8+; the gen7 mod has no `onTryBoost`): Intimidate's Attack change.
pub fn own_tempo(ctx: &StatChangeContext<'_>) -> TryStatChangeResult {
    refused(AbilityId::OWN_TEMPO, intimidate_attack(ctx), true)
}
