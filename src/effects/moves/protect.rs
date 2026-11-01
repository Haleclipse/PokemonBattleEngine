//! The stalling moves: Protect / Detect and their variants, a single-turn volatile that blocks
//! TryHit (§10.1, `ProtectKind`), and Endure. The consecutive-use odds are the Shell-owned
//! `stall` counter (`BattleContext::stall_move`, `ConditionRules::protect_stall`), and the move
//! fails when its user is the last to act.

use crate::engine::{BattleContext, ConditionApplyError, StallEffect};
use crate::ids::PokemonId;
use crate::rules::ResolvedTarget;
use crate::state::ProtectKind;

/// A stalling move's `onPrepareHit` (`BattleContext::stall_try`), before Protean's.
pub fn stalls(ctx: &mut BattleContext<'_>, user: PokemonId) -> Result<(), ConditionApplyError> {
    ctx.stall_try(user)
}

fn protect_as(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    kind: ProtectKind,
) -> Result<(), ConditionApplyError> {
    ctx.stall_move(user, StallEffect::Protect(kind))
}

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    protect_as(ctx, user, ProtectKind::Protect)
}

/// Max Guard (DESIGN.md §44.5): the Protect variant every status move becomes under Dynamax.
/// It blocks every move but its own fixed list (Max Guard itself never fails for that reason).
pub fn max_guard(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    protect_as(ctx, user, ProtectKind::MaxGuard)
}

pub fn kings_shield(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    protect_as(ctx, user, ProtectKind::KingsShield)
}

pub fn spiky_shield(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    protect_as(ctx, user, ProtectKind::SpikyShield)
}

pub fn baneful_bunker(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    protect_as(ctx, user, ProtectKind::BanefulBunker)
}

pub fn obstruct(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    protect_as(ctx, user, ProtectKind::Obstruct)
}

pub fn silk_trap(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    protect_as(ctx, user, ProtectKind::SilkTrap)
}

pub fn burning_bulwark(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    protect_as(ctx, user, ProtectKind::BurningBulwark)
}

/// Endure (`endure` volatile, `onDamage` priority −10): a move's damage leaves the user at 1 HP
/// at least this turn (`engine::endure_damage`).
pub fn endure(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    ctx.stall_move(user, StallEffect::Endure)
}

/// What a protection costs the attacker whose contact move it stopped (the protecting move's
/// `onTryHit`): a stat drop (King's Shield −1 Attack, Gen 6-7 −2; Obstruct −2 Defense; Silk Trap
/// −1 Speed), ⅛ of its max HP (Spiky Shield), or a status (Baneful Bunker: poison, Burning
/// Bulwark: burn).
pub fn punish_contact(
    ctx: &mut BattleContext<'_>,
    kind: ProtectKind,
    attacker: PokemonId,
    protector: PokemonId,
) {
    use crate::engine::{
        DamageAmount, DamageFlags, DamageKind, DamageRequest, EffectSource, Stat, StatChangeCause,
        StatChangeRequest,
    };
    use crate::ids::MoveId;
    use crate::state::NonVolatileStatus;
    let source = |move_id| EffectSource::Move {
        pokemon: protector,
        move_id,
    };
    let drop = |ctx: &mut BattleContext<'_>, move_id, stat, stages| {
        ctx.try_change_stat_stage(StatChangeRequest {
            target: attacker,
            stat,
            stages,
            source: source(move_id),
            cause: StatChangeCause::Move,
        });
    };
    match kind {
        // Max Guard has no reaction of its own (it blocks more than Protect does).
        ProtectKind::Protect | ProtectKind::MaxGuard => {}
        ProtectKind::KingsShield => {
            let stages = ctx.rules().conditions().kings_shield_attack_drop;
            drop(ctx, MoveId::KINGS_SHIELD, Stat::Attack, stages);
        }
        ProtectKind::Obstruct => drop(ctx, MoveId::OBSTRUCT, Stat::Defense, -2),
        ProtectKind::SilkTrap => drop(ctx, MoveId::SILK_TRAP, Stat::Speed, -1),
        ProtectKind::SpikyShield => {
            if let Some(max_hp) = ctx.base_max_hp(attacker) {
                ctx.apply_damage(DamageRequest {
                    target: attacker,
                    amount: DamageAmount((max_hp / 8).max(1)),
                    kind: DamageKind::Reaction,
                    source: source(MoveId::SPIKY_SHIELD),
                    flags: DamageFlags {
                        bypass_substitute: true,
                        spare_one_hp: false,
                        instafaint: false,
                        single_strike: false,
                        future_move: false,
                    },
                });
            }
        }
        ProtectKind::BanefulBunker => {
            ctx.try_set_status(
                attacker,
                NonVolatileStatus::Poison,
                source(MoveId::BANEFUL_BUNKER),
            );
        }
        ProtectKind::BurningBulwark => {
            ctx.try_set_status(
                attacker,
                NonVolatileStatus::Burn,
                source(MoveId::BURNING_BULWARK),
            );
        }
    }
}

/// Endure's `onDamage` (priority −10): a move's damage that would knock the holder out leaves it
/// at 1 HP.
pub fn endure_damage(
    ctx: &mut BattleContext<'_>,
    request: &crate::engine::DamageRequest,
) -> crate::engine::DamageDecision {
    if request.kind != crate::engine::DamageKind::Move {
        return crate::engine::DamageDecision::Allow;
    }
    super::leave_one_hp(ctx, request)
}
