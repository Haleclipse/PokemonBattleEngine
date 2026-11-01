//! Water Absorb / Volt Absorb (Showdown `onTryHit`): a Water / Electric move from another
//! Pokémon is refused at TryHit and heals the holder 1/4 of its max HP (nothing at full HP;
//! Showdown then shows `-immune`). No accuracy check is made for the holder. Gen 3's Volt
//! Absorb lets Thunder Wave through (`HitRules::volt_absorb_spares_thunder_wave`).

use crate::dex::PokemonType;
use crate::engine::{
    BattleContext, EffectSource, HealRequest, HitBlockReason, TryHitContext, TryHitResult,
};
use crate::ids::{AbilityId, MoveId, PokemonId};

pub fn water_absorb(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    absorbs(ctx, PokemonType::Water, AbilityId::WATER_ABSORB)
}

/// Dry Skin takes Water moves like Water Absorb (a quarter of its max HP).
pub fn dry_skin(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    absorbs(ctx, PokemonType::Water, AbilityId::DRY_SKIN)
}

pub fn volt_absorb(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    if ctx.rules().hits().volt_absorb_spares_thunder_wave && ctx.move_id() == MoveId::THUNDER_WAVE {
        return TryHitResult::Allow;
    }
    absorbs(ctx, PokemonType::Electric, AbilityId::VOLT_ABSORB)
}

fn absorbs(ctx: &TryHitContext<'_>, move_type: PokemonType, ability: AbilityId) -> TryHitResult {
    if ctx.user() != ctx.target() && ctx.move_type() == move_type {
        TryHitResult::Blocked(HitBlockReason::Ability(ability))
    } else {
        TryHitResult::Allow
    }
}

pub fn heal_quarter(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(max_hp) = ctx.base_max_hp(holder) else {
        return;
    };
    let ability = ctx.ability_of(holder).unwrap_or(AbilityId::NONE);
    ctx.apply_heal(HealRequest {
        target: holder,
        amount: (max_hp / 4).max(1),
        source: EffectSource::Ability {
            pokemon: holder,
            ability,
        },
        direct: false,
    });
}
