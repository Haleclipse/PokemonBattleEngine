//! Items a hit makes their holder use or eat (DESIGN.md R198, R208): Absorb Bulb, Cell
//! Battery, Luminous Moss, Snowball, Weakness Policy (Showdown `onDamagingHit` → `useItem`, the
//! item's `boosts`), Kee Berry and Maranga Berry (`onAfterMoveSecondary` → `eatItem`), Jaboca
//! Berry and Rowap Berry (`onDamagingHit` → `eatItem`, the attacker hurt), Enigma Berry (`onHit`
//! → `eatItem`, a heal).

use crate::dex::MoveCategory;
use crate::engine::{
    AfterHit, AfterHitHandler, BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest,
    DamagingHit, DamagingHitReactionHandler, EffectSource, OrderedHandler, StatChangeCause,
    StatChangeRequest,
};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::rules::{BerryHeal, Fraction};
use crate::state::Stat;

use super::{EatEffect, ItemBehavior, UsedOnHit, UsedOnHitWhen};

/// No `onDamagingHitOrder`: after every ordered reaction (Showdown sorts a missing order last).
const UNORDERED: i16 = i16::MAX;

pub(super) fn recoil_berry(category: MoveCategory) -> ItemBehavior {
    ItemBehavior {
        recoil_berry: Some(category),
        damaging_hit_reaction: Some(OrderedHandler::new(
            UNORDERED,
            recoil_on_hit as DamagingHitReactionHandler,
        )),
        eat_effect: Some(EatEffect::Nothing),
        ..ItemBehavior::default()
    }
}

pub(super) fn enigma_berry() -> ItemBehavior {
    ItemBehavior {
        super_effective_heal: Some(Fraction::new(1, 4)),
        on_hit: Some(heal_on_super_effective_hit as DamagingHitReactionHandler),
        eat_effect: Some(EatEffect::Nothing),
        ..ItemBehavior::default()
    }
}

pub(super) fn used_on_hit(when: UsedOnHitWhen, boosts: &'static [(Stat, i8)]) -> ItemBehavior {
    ItemBehavior {
        used_on_hit: Some(UsedOnHit { when, boosts }),
        damaging_hit_reaction: Some(OrderedHandler::new(
            UNORDERED,
            use_on_hit as DamagingHitReactionHandler,
        )),
        ..ItemBehavior::default()
    }
}

/// Kee Berry, Maranga Berry. Their `onAfterMoveSecondary` has no priority: after the holder's
/// ability's (Pickpocket), the ability's effect type sorting first (1, as Red Card).
pub(super) fn eaten_after_a_hit(category: MoveCategory, effect: EatEffect) -> ItemBehavior {
    ItemBehavior {
        eaten_after_a_hit: Some(category),
        after_hit: Some(OrderedHandler::new(1, eat_after_a_hit as AfterHitHandler)),
        eat_effect: Some(effect),
        ..ItemBehavior::default()
    }
}

/// `onDamagingHit`: a hit that qualifies makes the holder — standing (`useItem` needs HP) — use
/// the item up, then the item's `boosts` change its stats (Showdown `useItem`: `-enditem`, then
/// `boost(item.boosts, holder, holder, item)`; the item is used up whatever the stages).
fn use_on_hit(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    let Some((item, used)) = ctx
        .held_item(hit.holder)
        .and_then(|(item, behavior)| behavior.used_on_hit.map(|used| (item, used)))
    else {
        return;
    };
    let qualifies = match used.when {
        UsedOnHitWhen::MoveType(move_type) => hit.move_type == move_type,
        UsedOnHitWhen::SuperEffective => hit.super_effective && !hit.fixed_damage,
    };
    if !qualifies || ctx.hp(hit.holder).is_none_or(|hp| hp == 0) {
        return;
    }
    use_for_boosts(ctx, hit.holder, item, used.boosts);
}

/// Showdown `useItem` for an item with `boosts`: used up (`-enditem`), then the stat changes, the
/// item their source (made whatever the stages).
pub(super) fn use_for_boosts(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    item: crate::ids::ItemId,
    boosts: &[(Stat, i8)],
) {
    if ctx.consume_item(holder).is_none() {
        return;
    }
    for &(stat, stages) in boosts {
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

/// `onAfterMoveSecondary`: a move of the berry's category (Kee Berry physical, Maranga Berry
/// special) that hit the holder makes it eat the berry (`EatEffect::Boost`).
fn eat_after_a_hit(ctx: &mut BattleContext<'_>, hit: AfterHit) {
    if ctx
        .held_item(hit.holder)
        .and_then(|(_, behavior)| behavior.eaten_after_a_hit)
        == Some(hit.category)
    {
        ctx.eat_item(hit.holder);
    }
}

/// Jaboca / Rowap Berry's `onDamagingHit` (every hit; pokeplatinum likewise after every hit,
/// and for U-turn before the user leaves: issues.md I45): a hit of the berry's category makes
/// the holder eat it — unless the attacker has Magic Guard, or (`BerryRules::
/// recoil_berries_need_the_attacker`) no longer stands on the field — and the attacker loses an eighth of its
/// max HP (a quarter with the holder's Ripen), the berry its source.
fn recoil_on_hit(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if ctx
        .held_item(hit.holder)
        .and_then(|(_, behavior)| behavior.recoil_berry)
        != Some(hit.category)
    {
        return;
    }
    if ctx
        .rules()
        .conditions()
        .berries
        .recoil_berries_need_the_attacker
        && (ctx.is_fainted(hit.attacker) || !ctx.is_active(hit.attacker))
    {
        return;
    }
    if ctx.ability_of(hit.attacker) == Some(AbilityId::MAGIC_GUARD) {
        return;
    }
    let Some(item) = ctx.eat_item(hit.holder) else {
        return;
    };
    // Gen 5-6 eat the berry for an attacker already down; `damage` then finds no HP to take.
    let Some(max_hp) = ctx
        .base_max_hp(hit.attacker)
        .filter(|_| !ctx.is_fainted(hit.attacker))
    else {
        return;
    };
    // `target.hasAbility('ripen')`.
    let ripen = ctx.ability_of(hit.holder) == Some(AbilityId::RIPEN);
    let share = if ripen { 4 } else { 8 };
    ctx.apply_damage(DamageRequest {
        target: hit.attacker,
        amount: DamageAmount((max_hp / share).max(1)),
        kind: DamageKind::Reaction,
        source: EffectSource::Item {
            pokemon: hit.holder,
            item,
        },
        flags: DamageFlags {
            bypass_substitute: true,
            spare_one_hp: false,
            // The gen6 mod's `damage(…, true)` (Gen 5-6), as Rough Skin's.
            instafaint: ctx.rules().hits().contact_reaction_instafaint,
            single_strike: false,
            future_move: false,
        },
    });
}

/// Enigma Berry's `onHit`: a super effective hit (Showdown `typeMod > 0`, never a fixed damage)
/// makes the holder eat it (`eat_item` refuses it where the heal would be refused) and restore
/// its share of its max HP — the berry's heal, so Ripen doubles it.
fn heal_on_super_effective_hit(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    let Some(share) = ctx
        .held_item(hit.holder)
        .and_then(|(_, behavior)| behavior.super_effective_heal)
    else {
        return;
    };
    if !hit.super_effective || hit.fixed_damage {
        return;
    }
    if let Some(item) = ctx.eat_item(hit.holder) {
        ctx.heal_from_berry(hit.holder, item, BerryHeal::MaxHp(share));
    }
}

/// Throat Spray's `onAfterMoveSecondarySelf`: a sound move of the holder's makes it use the item
/// up — standing (`useItem` needs HP) — for the item's `boosts`.
pub fn use_after_a_sound_move(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some((item, boosts)) = ctx
        .held_item(holder)
        .and_then(|(item, behavior)| behavior.used_after_a_sound_move.map(|b| (item, b)))
    else {
        return;
    };
    if ctx.is_fainted(holder) {
        return;
    }
    use_for_boosts(ctx, holder, item, boosts);
}

/// Adrenaline Orb's `onAfterBoost`: Intimidate meant to lower the holder's Attack — whether it
/// did, or something refused it (Showdown deletes `boost.atk`), but not when the Attack had no
/// room left to fall (`boost.atk === 0`) — and the holder's Speed is below +6: it uses the orb
/// up for +1 Speed.
pub(super) fn adrenaline_orb(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    request: crate::engine::StatChangeRequest,
    result: crate::engine::StatChangeResult,
) {
    let intimidate = matches!(
        request.source,
        EffectSource::Ability {
            ability: AbilityId::INTIMIDATE,
            ..
        }
    );
    if !intimidate
        || request.stat != Stat::Attack
        || matches!(
            result,
            crate::engine::StatChangeResult::Blocked(
                crate::engine::StatChangeBlockReason::NoChange
            )
        )
        || ctx.stage(holder, Stat::Speed) >= 6
        || ctx.is_fainted(holder)
    {
        return;
    }
    let Some((item, _)) = ctx.held_item(holder) else {
        return;
    };
    use_for_boosts(ctx, holder, item, &[(Stat::Speed, 1)]);
}

/// Blunder Policy, which has no callback of its own: Showdown's `hitStepAccuracy` asks
/// `hasItem('blunderpolicy')` when a move — not a one-hit KO move — misses on its accuracy roll,
/// and the holder uses it up for +2 Speed, the move the effect (`this.boost({spe: 2}, pokemon)`).
pub fn use_blunder_policy(ctx: &mut BattleContext<'_>, holder: PokemonId, move_id: MoveId) {
    if !ctx
        .held_item(holder)
        .is_some_and(|(item, _)| item == crate::ids::ItemId::BLUNDER_POLICY)
        || ctx.is_fainted(holder)
        || ctx.consume_item(holder).is_none()
    {
        return;
    }
    ctx.try_change_stat_stage(StatChangeRequest {
        target: holder,
        stat: Stat::Speed,
        stages: 2,
        source: EffectSource::Move {
            pokemon: holder,
            move_id,
        },
        cause: StatChangeCause::Move,
    });
}
