//! Berserk and Anger Shell (Showdown `onDamage`, `onTryEatItem`, `onAfterMoveSecondary`): a
//! move that strikes once (`DamageFlags::single_strike`) and hurts the holder holds its healing
//! berries back (`AbilityState::hp_berries_wait`, read by `BattleContext::eat_item`) until the
//! move's `AfterMoveSecondary`, where the holder changes its stats if the move took it from
//! above half its HP to half or below; any other damage lets the berries go at once.

use crate::engine::{BattleContext, DamageRequest};
use crate::ids::PokemonId;
use crate::state::Stat;

/// What the ability does once a move took its holder down to half its HP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HalfHpBoosts {
    /// The stat changes (`this.boost(...)`, in Showdown's order).
    pub changes: &'static [(Stat, i8)],
    /// The damage a multi-hit move counts: Anger Shell takes the move's total over every
    /// target (`move.totalDamage`, which Dragon Darts spreads over two); Berserk the holder's
    /// own (`!move.smartTarget`). A move that strikes once counts the holder's damage.
    pub counts_every_target: bool,
}

pub const BERSERK: HalfHpBoosts = HalfHpBoosts {
    changes: &[(Stat::SpecialAttack, 1)],
    counts_every_target: false,
};

pub const ANGER_SHELL: HalfHpBoosts = HalfHpBoosts {
    changes: &[
        (Stat::Attack, 1),
        (Stat::SpecialAttack, 1),
        (Stat::Speed, 1),
        (Stat::Defense, -1),
        (Stat::SpecialDefense, -1),
    ],
    counts_every_target: true,
};

/// `onDamage`: every damage that reaches the holder's HP through the `Damage` event (not a
/// `directDamage` cost) decides whether the healing berries wait.
pub fn damaged(ctx: &mut BattleContext<'_>, request: &DamageRequest) {
    if request.kind.is_direct() {
        return;
    }
    ctx.set_hp_berries_wait(request.target, request.flags.single_strike);
}

/// `onAfterMoveSecondary` on a Pokémon the move reached: the berries may go again; if another
/// Pokémon's move dealt damage, the holder is standing and `damage` (what the move took from
/// it, as `HalfHpBoosts::counts_every_target` says) took it from above half to half or below,
/// the stats change.
pub fn after_move_secondary(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    attacker: PokemonId,
    boosts: HalfHpBoosts,
    damage: u16,
    total_damage: u16,
) {
    ctx.set_hp_berries_wait(holder, false);
    if holder == attacker || total_damage == 0 {
        return;
    }
    let (Some(hp), Some(max_hp)) = (ctx.hp(holder), ctx.max_hp(holder)) else {
        return;
    };
    let (hp, max_hp) = (u32::from(hp), u32::from(max_hp));
    if hp > 0 && 2 * hp <= max_hp && 2 * (hp + u32::from(damage)) > max_hp {
        super::hit_boosts::boost(ctx, holder, boosts.changes);
    }
}
