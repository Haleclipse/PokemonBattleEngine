//! The status-immunity abilities' handlers beyond their data (DESIGN.md R270): Thermal
//! Exchange's rise after a Fire hit, Pastel Veil's cure and guard of its allies, Oblivious's
//! TryHit. Their refusals (`blocks_statuses`, `status_immunity`, `blocks_volatiles`) and cures
//! (`AbilityUpdate`) are data in `AbilityBehavior`.

use crate::dex::PokemonType;
use crate::engine::{
    BattleContext, CureReason, DamagingHit, HitBlockReason, StatusBlockReason, TryHitContext,
    TryHitResult, TrySetStatusContext, TrySetStatusResult,
};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::state::{NonVolatileStatus, Stat};

/// Thermal Exchange (`onDamagingHit`): a Fire move's hit raises the holder's Attack.
pub fn thermal_exchange(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if hit.move_type == PokemonType::Fire {
        super::hit_boosts::boost(ctx, hit.holder, &[(Stat::Attack, 1)]);
    }
}

/// Pastel Veil's `onStart`, run again by its `onAnySwitchIn`: each Pokémon on the holder's side
/// (`alliesAndSelf`), the holder included, that is poisoned is cured (`-activate|holder|ability:
/// Pastel Veil` for each).
pub fn pastel_veil_start(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let side = ctx.side_of(holder);
    let poisoned: Vec<PokemonId> = ctx
        .active_pokemon()
        .into_iter()
        .filter(|member| ctx.side_of(*member) == side && !ctx.is_fainted(*member))
        .filter(|member| {
            matches!(
                ctx.status_of(*member),
                Some(NonVolatileStatus::Poison | NonVolatileStatus::Toxic)
            )
        })
        .collect();
    for member in poisoned {
        ctx.ability_activated(holder, AbilityId::PASTEL_VEIL);
        ctx.cure_status(member, CureReason::Ability(AbilityId::PASTEL_VEIL));
    }
}

/// Pastel Veil (`onAllySetStatus`): no poison on the holder's allies, whoever causes it.
pub fn pastel_veil_ally(ctx: &TrySetStatusContext<'_>) -> TrySetStatusResult {
    if matches!(
        ctx.status(),
        NonVolatileStatus::Poison | NonVolatileStatus::Toxic
    ) {
        // `-block|ally|ability: Pastel Veil|[of] holder` for a status its effect carries.
        TrySetStatusResult::Blocked {
            reason: StatusBlockReason::Ability(AbilityId::PASTEL_VEIL),
            shown: ctx.carried(),
        }
    } else {
        TrySetStatusResult::Allow
    }
}

/// Oblivious (`onTryHit`): Captivate is refused; Gen 6+ Attract and Taunt too
/// (`ConditionRules::oblivious_covers_taunt`; the gen5 mod's handler names Captivate alone).
pub fn oblivious_try_hit(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    let move_id = ctx.move_id();
    let refused = move_id == MoveId::CAPTIVATE
        || (ctx.rules().conditions().oblivious_covers_taunt
            && matches!(move_id, MoveId::ATTRACT | MoveId::TAUNT));
    if refused {
        TryHitResult::Blocked(HitBlockReason::Ability(AbilityId::OBLIVIOUS))
    } else {
        TryHitResult::Allow
    }
}
