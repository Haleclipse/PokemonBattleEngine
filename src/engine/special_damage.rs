//! Damage that is not the formula's (DESIGN.md §22): fixed damage (Seismic Toss, Super Fang
//! …), Gen 1's Counter, the record a retaliation answers, Psywave, a binding move's, the moves
//! that heal instead.

use super::*;

/// Damage that bypasses the formula (DESIGN.md §22.6): fixed damage, or a one-hit KO's.
/// `None` for an ordinary damaging move.
pub fn fixed_damage(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> Option<u16> {
    let user = state.pokemon(attacker)?;
    let foe = state.pokemon(target)?;
    if move_def.ohko.is_some() {
        let current = matches!(
            rules.hits.ohko,
            crate::rules::OhkoRules::LevelGatePercent {
                current_hp_damage: true,
                ..
            }
        );
        return Some(if current { foe.hp } else { foe.max_hp });
    }
    Some(match move_def.fixed_damage? {
        crate::dex::FixedDamage::Level => u16::from(user.level),
        crate::dex::FixedDamage::Amount(amount) => amount,
        // Super Fang, Ruination, Nature's Madness (`getUndynamaxedHP`, DESIGN.md §44.5): a
        // Dynamaxed target's HP reads as it would without the doubling.
        crate::dex::FixedDamage::HalfTargetHp => (foe.undynamaxed_hp(None) / 2).max(1),
        // Guardian of Alola: three quarters, or a quarter of that through a protection
        // (`z_broke_protection`; Showdown's own `-zbroken` in the callback, DESIGN.md §44.4;
        // its `getUndynamaxedHP()` too, §44.5). The quarter rounds half down
        // (`Math.ceil(hp75 / 4 - 0.5)`: 117 gives 29, R-M8-184).
        crate::dex::FixedDamage::ThreeQuartersTargetHp => {
            let three_quarters = (u32::from(foe.undynamaxed_hp(None)) * 3 / 4).max(1);
            if z_broke_protection(state, foe, move_def.priority) {
                ((three_quarters + 1) / 4).max(1) as u16
            } else {
                three_quarters as u16
            }
        }
        // Endeavor (`target.getUndynamaxedHP() - pokemon.hp`): the target's reads without the
        // doubling, the user's as it stands.
        // Against a Dynamaxed target whose HP without the doubling is below the user's (the
        // immunity reads the doubled HP), the difference is negative, and `spreadDamage` clamps
        // any damage but 0 to at least 1 (R-M9-38).
        crate::dex::FixedDamage::TargetHpMinusUserHp => {
            let target_hp = foe.undynamaxed_hp(None);
            if target_hp < user.hp {
                1
            } else {
                target_hp - user.hp
            }
        }
        // Drawn with the hit (`psywave_range`, `foreseen_damage`), not fixed.
        crate::dex::FixedDamage::Psywave | crate::dex::FixedDamage::Foreseen(_) => return None,
        crate::dex::FixedDamage::UserHp => user.hp,
        // Nothing to strike back at: 0, which the hit sequence takes as a failure.
        crate::dex::FixedDamage::Retaliation(kind) => {
            retaliation_record(state, rules, attacker, kind).map_or(0, |(_, damage)| damage)
        }
        crate::dex::FixedDamage::Bide => user.volatiles.bide().map_or(0, |bide| {
            u16::try_from(bide.damage.saturating_mul(2)).unwrap_or(u16::MAX)
        }),
    })
}

/// Gen 1 Counter (the gen1 mod's `damageCallback`, DESIGN.md §15.8): twice the battle's last
/// damage, when the move `target`'s side last used and last selected a counterable move (a
/// Normal or Fighting move with power, not Counter) — `None` (a failure) when neither is, when
/// only one is (the Desync Clause) or when there is no last damage.
/// A protection a Z-Move or a Max Move went through, for a quarter of its damage (Showdown's
/// `zBrokeProtect`, `-zbroken`, DESIGN.md §44.4): the target's own (Protect and its variants),
/// its side's Mat Block, or its side's Quick Guard against a move with raised priority —
/// `priority` as the move meets it (Gen 6+ the action's: a Z-Move made from Quick Attack is
/// ordered at its own 0). Wide Guard never meets one: they strike a single target (R-M11-112).
pub(crate) fn z_broke_protection(
    state: &crate::state::BattleState,
    target: &crate::state::PokemonState,
    priority: i8,
) -> bool {
    let conditions = &state.side(target.side).conditions;
    target.volatiles.protect_kind().is_some()
        || conditions.contains(crate::state::SideConditionKind::MatBlock)
        || (priority > 0 && conditions.contains(crate::state::SideConditionKind::QuickGuard))
}

pub fn gen1_counter_damage(
    state: &crate::state::BattleState,
    dex: &crate::dex::ResolvedDex,
    target: crate::ids::PokemonId,
) -> Option<u16> {
    let side = state.side(state.pokemon(target)?.side);
    let counterable = |move_id: Option<crate::ids::MoveId>| {
        move_id.and_then(|id| dex.move_by_id(id)).is_some_and(|m| {
            m.base_power > 0
                && matches!(
                    m.move_type,
                    crate::dex::PokemonType::Normal | crate::dex::PokemonType::Fighting
                )
                && m.id != crate::ids::MoveId::COUNTER
        })
    };
    let used = counterable(side.last_move);
    let selected = counterable(
        side.last_selected_move
            .and_then(crate::state::SelectedMove::move_id),
    );
    (used && selected && state.last_damage > 0).then(|| state.last_damage.saturating_mul(2))
}

/// What a retaliating move strikes back at (DESIGN.md §15.8): the attack record it answers and
/// the damage it deals — Counter / Mirror Coat twice the last physical / special hit a foe
/// dealt the user this turn (Gen 2: the very last attack, which must be its attacker's last
/// move), Metal Burst / Comeuppance one and a half times the last damaging hit from a foe; at
/// least 1. Shared with the replay driver.
pub fn retaliation_record(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    kind: crate::dex::Retaliation,
) -> Option<(crate::state::AttackRecord, u16)> {
    use crate::dex::{MoveCategory, Retaliation};
    let pokemon = state.pokemon(user)?;
    let wanted = match kind {
        Retaliation::Counter => Some(MoveCategory::Physical),
        Retaliation::MirrorCoat => Some(MoveCategory::Special),
        Retaliation::MetalBurst => None,
    };
    // Metal Burst reads the attacks (`attackedBy`), not a damaging hit that is none (R-M9-28).
    let answers = |record: &crate::state::AttackRecord| {
        record.damage.is_some()
            && match wanted {
                Some(category) => record.category == category,
                None => !record.damaging_hit_only,
            }
    };
    let record = if rules.conditions.retaliation.last_attack_only && wanted.is_some() {
        let last = *pokemon.history.attacked_this_turn.last()?;
        let attacker_last = state.pokemon(last.source).and_then(|p| p.last_move);
        // Gen 2 (issues.md I40): not against another Counter / Mirror Coat, nor for nothing.
        let refused = rules
            .conditions
            .retaliation
            .fails_on_retaliation_or_no_damage
            && (crate::effects::moves::definition(last.move_id)
                .strikes_back_at
                .is_some()
                || last.damage.is_none_or(|damage| damage == 0));
        (!refused && answers(&last) && attacker_last == Some(last.move_id)).then_some(last)?
    } else {
        *pokemon
            .history
            .attacked_this_turn
            .iter()
            .rev()
            .filter(|record| {
                state
                    .pokemon(record.source)
                    .is_some_and(|source| source.side != pokemon.side)
            })
            .find(|record| answers(record))?
    };
    let taken = u32::from(record.damage.unwrap_or(0));
    let damage = match kind {
        Retaliation::Counter | Retaliation::MirrorCoat => taken * 2,
        Retaliation::MetalBurst => taken * 3 / 2,
    };
    let damage = if rules.conditions.retaliation.last_attack_only {
        damage
    } else {
        damage.max(1)
    };
    Some((record, damage.min(u32::from(u16::MAX)) as u16))
}

/// A hit that heals its target instead of damaging it (base power 0, so no critical-hit check
/// or damage roll): Present's drawn heal, Pollen Puff aimed at an ally. Shared with the replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealInstead {
    /// Present (`move.heal`): the generation's rounding (`ConditionRules::heal_rounds_half_up`);
    /// a full-HP target makes the move fail (`-fail|target|heal`).
    Drawn(crate::rules::Fraction),
    /// Pollen Puff: rounded down; a full-HP ally is `HitRules::ally_heal_fails_at_full_hp`'s.
    Ally(crate::rules::Fraction),
}

pub fn heals_instead(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
    drawn: Option<crate::effects::moves::DrawnPower>,
) -> Option<HealInstead> {
    if let Some(crate::effects::moves::DrawnPower::HealTarget(fraction)) = drawn {
        return Some(HealInstead::Drawn(fraction));
    }
    let fraction = move_def.behavior.ally_heal?;
    let side = |pokemon| {
        state
            .active_position(pokemon, &rules.topology)
            .and_then(|position| rules.topology.spec(position))
            .map(|spec| spec.side)
    };
    (target != user && side(target).is_some() && side(target) == side(user))
        .then_some(HealInstead::Ally(fraction))
}

/// Psywave's roll for a user of `level` (`DamageRules::psywave`): the inclusive range drawn in
/// place of the damage roll. Shared with the replay driver.
pub fn psywave_range(rules: &crate::rules::ResolvedRules, level: u8) -> (u16, u16) {
    let level = u16::from(level);
    match rules.damage.psywave {
        // `random(0, trunc(1.5 × level))`.
        crate::rules::PsywaveDamage::Gen1 => (0, (level * 3 / 2).saturating_sub(1)),
        // `random(1, level + ⌊level / 2⌋)`.
        crate::rules::PsywaveDamage::Gen2 => (1, (level + level / 2).saturating_sub(1).max(1)),
        crate::rules::PsywaveDamage::Percent => (50, 150),
    }
}

/// The damage a Psywave roll deals (Gen 1's 0 fails the move).
pub fn psywave_damage(rules: &crate::rules::ResolvedRules, level: u8, roll: u16) -> u16 {
    match rules.damage.psywave {
        crate::rules::PsywaveDamage::Percent => (roll * u16::from(level) / 100).max(1),
        crate::rules::PsywaveDamage::Gen1 | crate::rules::PsywaveDamage::Gen2 => roll,
    }
}

/// Gen 1 Recover / Soft-Boiled / Rest (`ConditionRules::heal_fails_255_short`): a heal fails
/// when the Pokémon is 255 or 511 HP short of its max, unless its HP is a multiple of 256.
pub(crate) fn heal_fails_255_short(
    conditions: &crate::rules::ConditionRules,
    hp: u16,
    max_hp: u16,
) -> bool {
    let short = max_hp.saturating_sub(hp);
    conditions.heal_fails_255_short && (short == 255 || short == 511) && !hp.is_multiple_of(256)
}

/// Gen 1 `partiallytrapped`'s `onAccuracy`: every move of the Pokémon that bound `target`
/// hits it (DESIGN.md §30.9). Shared with the replay driver.
pub fn gen1_bound_by(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: crate::ids::PokemonId,
    source: crate::ids::PokemonId,
) -> bool {
    rules.conditions.switching.binding_locks_user
        && state
            .pokemon(target)
            .and_then(|p| p.volatiles.partially_trapped_by())
            .is_some_and(|(by, _)| by == source)
}

/// Gen 1: the binding move a user is locked into (`partialtrappinglock`, §30.9), which it
/// uses whatever its action named (Showdown `chooseMove`: the `lockedMove`). Shared with the
/// replay driver.
pub fn binding_locked_move(
    state: &crate::state::BattleState,
    user: crate::ids::PokemonId,
) -> Option<crate::ids::MoveId> {
    let (_, move_id, ..) = state.pokemon(user)?.volatiles.partial_trapping_lock()?;
    Some(move_id)
}

/// A Gen 1 binding move's fixed damage (gen1 `getDamage`, DESIGN.md §30.9): 0 when a Normal-type
/// one hits a Ghost type (`ignoreImmunity`), the first hit's damage again while the user's lock
/// holds this target; `None` otherwise (the damage is rolled). Shared with the replay driver.
pub fn binding_damage(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> Option<DamageAmount> {
    if !rules.conditions.switching.binding_locks_user || !move_def.behavior.binding_lock {
        return None;
    }
    let foe = state.pokemon(target)?;
    if move_def.move_type == crate::dex::PokemonType::Normal
        && crate::engine::types_of(state, rules, foe).contains(crate::dex::PokemonType::Ghost)
    {
        return Some(DamageAmount(0));
    }
    let (locked, _, damage, ..) = state.pokemon(attacker)?.volatiles.partial_trapping_lock()?;
    (locked == target).then_some(DamageAmount(damage))
}
