//! Moves that heal or cure through their own `onHit` (DESIGN.md §20.1, R156): Jungle Healing
//! and Lunar Blessing (the user and its allies: a quarter of the max HP and the status), Take
//! Heart (the user: +1 Special Attack and Special Defense and its status) and Strength Sap (the
//! user heals by the target's Attack, which then falls).

use crate::engine::{
    BattleContext, ConditionApplyError, CureReason, EffectSource, HealDrain, HealRequest,
    HealResult, StatChangeCause, StatChangeRequest, StatChangeResult,
};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::Stat;

fn by(user: PokemonId, move_id: MoveId) -> EffectSource {
    EffectSource::Move {
        pokemon: user,
        move_id,
    }
}

fn heal(ctx: &mut BattleContext<'_>, target: PokemonId, amount: u32, source: EffectSource) -> bool {
    matches!(
        ctx.apply_heal(HealRequest {
            target,
            amount: amount.min(u32::from(u16::MAX)) as u16,
            source,
            direct: false,
        }),
        HealResult::Applied { .. }
    )
}

fn stages(
    ctx: &mut BattleContext<'_>,
    target: PokemonId,
    changes: &[(Stat, i8)],
    source: EffectSource,
) -> bool {
    let mut any = false;
    for &(stat, stages) in changes {
        any |= matches!(
            ctx.try_change_stat_stage(StatChangeRequest {
                target,
                stat,
                stages,
                source,
                cause: StatChangeCause::Move,
            }),
            StatChangeResult::Applied { .. }
        );
    }
    any
}

/// Jungle Healing / Lunar Blessing: each of the user and its allies heals a quarter of its max
/// HP (`modify(maxhp, 0.25)`) and is cured of its status; the move fails when nobody gained
/// anything.
fn heal_and_cure_allies(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
    move_id: MoveId,
) -> Result<(), ConditionApplyError> {
    let mut any = false;
    for target in targets {
        let ResolvedTarget::Pokemon(target) = *target else {
            continue;
        };
        let Some(max_hp) = ctx.max_hp(target) else {
            continue;
        };
        let healed = heal(
            ctx,
            target,
            crate::engine::showdown_modify(u32::from(max_hp), 1024),
            by(user, move_id),
        );
        let cured = ctx.cure_status(target, CureReason::Move(move_id));
        any |= healed || cured;
    }
    if any {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}

pub fn jungle_healing(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    heal_and_cure_allies(ctx, user, targets, MoveId::JUNGLE_HEALING)
}

pub fn lunar_blessing(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    heal_and_cure_allies(ctx, user, targets, MoveId::LUNAR_BLESSING)
}

/// Take Heart: +1 Special Attack and Special Defense, then the user's status is cured; it
/// fails when neither did anything.
pub fn take_heart(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let raised = stages(
        ctx,
        user,
        &[(Stat::SpecialAttack, 1), (Stat::SpecialDefense, 1)],
        by(user, MoveId::TAKE_HEART),
    );
    let cured = ctx.cure_status(user, CureReason::Move(MoveId::TAKE_HEART));
    if raised || cured {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}

/// Strength Sap: against a target at -6 Attack it fails; otherwise the target's Attack (at its
/// stage, unmodified, read first) falls by 1 and the user heals that much — the move worked if
/// either did.
pub fn strength_sap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    if ctx.stage(target, Stat::Attack) == -6 {
        return Err(ConditionApplyError::AlreadyActive);
    }
    let attack = ctx.staged_stat(target, Stat::Attack);
    let source = by(user, MoveId::STRENGTH_SAP);
    let lowered = stages(ctx, target, &[(Stat::Attack, -1)], source);
    let healed = ctx.drain_heal(
        HealRequest {
            target: user,
            amount: attack.min(u32::from(u16::MAX)) as u16,
            source,
            direct: false,
        },
        target,
        HealDrain::StrengthSap,
    );
    if lowered || healed {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}
