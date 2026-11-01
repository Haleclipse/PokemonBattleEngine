//! Abilities that stop moves before they hit, and the switch-in warnings (DESIGN.md R275):
//! Bulletproof, Soundproof and Wonder Guard at TryHit; Dazzling, Queenly Majesty and Armor Tail
//! at their foes' TryMove (`AbilityBehavior::blocks_priority_moves`, `engine::priority_guard`);
//! Anticipation's and Forewarn's announcements as their holder comes in.

use crate::dex::{MoveCategory, PokemonType};
use crate::engine::{BattleContext, HitBlockReason, TryHitContext, TryHitResult};
use crate::ids::{AbilityId, MoveId, PokemonId};

fn refused(by: AbilityId, refuses: bool) -> TryHitResult {
    if refuses {
        TryHitResult::Blocked(HitBlockReason::Ability(by))
    } else {
        TryHitResult::Allow
    }
}

/// Bulletproof (`onTryHit`): a `bullet` move.
pub fn bulletproof(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    refused(AbilityId::BULLETPROOF, ctx.move_flags().bullet)
}

/// Soundproof (`onTryHit`): a `sound` move from another Pokémon — Gen 3-7 its holder's own too
/// (`ConditionRules::soundproof_blocks_own_moves`).
pub fn soundproof(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    refused(
        AbilityId::SOUNDPROOF,
        ctx.move_flags().sound
            && (ctx.user() != ctx.target() || ctx.rules().conditions().soundproof_blocks_own_moves),
    )
}

/// Soundproof (`onAllyTryHitSide`, Gen 6+): a sound move aimed at the holder's side is announced
/// as refused (`-immune`) and goes ahead — the holder's own too (Heal Bell, R-M10-53).
pub fn soundproof_side(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    _user: PokemonId,
    move_def: &crate::dex::ResolvedMove,
) {
    if move_def.flags.sound {
        ctx.ability_activated(holder, AbilityId::SOUNDPROOF);
    }
}

/// Wonder Guard (`onTryHit`): another Pokémon's damaging move that is not super effective, or
/// that the holder is immune to — Struggle and the other typeless hits aside (the gen4 mod's
/// `type === '???'`; Gen 5+ names Struggle, the only one left), Gen 3-4 Fire Fang
/// (`ConditionRules::wonder_guard_spared_moves`), and Sky Drop as it carries its target up (its
/// user not yet `skydrop`).
pub fn wonder_guard(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    if ctx.user() == ctx.target()
        || (ctx.move_id() == MoveId::SKY_DROP
            && !ctx.user_has_volatile(crate::state::VolatileConditionKind::SkyDrop))
        || ctx.move_category() == MoveCategory::Status
        || ctx.move_type() == PokemonType::Typeless
        || ctx
            .rules()
            .conditions()
            .wonder_guard_spared_moves
            .contains(&ctx.move_id())
    {
        return TryHitResult::Allow;
    }
    refused(AbilityId::WONDER_GUARD, !ctx.super_effective_on_target())
}

/// Anticipation (`onStart`): announced when a foe has a move the holder should fear
/// (`engine::anticipates`).
pub fn anticipation(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.anticipates(holder) {
        ctx.ability_activated(holder, AbilityId::ANTICIPATION);
    }
}

/// Forewarn (`onStart`): one of the foes' strongest moves is drawn (`sample`, among two or
/// more) and named — with its foe from Gen 5 (`ConditionRules::forewarn_names_the_foe`); Gen 4's
/// line gives the foe away only when one stands.
pub fn forewarn(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let warned = ctx.forewarn_candidates(holder);
    if warned.is_empty() {
        return;
    }
    let (move_id, foe) = warned[ctx.sample_index(warned.len())];
    ctx.ability_activated(holder, AbilityId::FOREWARN);
    let identified =
        ctx.rules().conditions().forewarn_names_the_foe || ctx.active_foes(holder).len() == 1;
    ctx.forewarned(holder, foe, move_id, identified);
}
