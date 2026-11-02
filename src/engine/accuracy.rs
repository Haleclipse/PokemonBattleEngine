//! Accuracy (DESIGN.md §15.5, §22): a move's accuracy in basis points, its roll and the
//! modifiers on the way (stages, Unaware, Micle Berry, Gravity, No Guard), Lock-On's and
//! Minimize's sure hits, one-hit KO moves' block.

use super::*;

/// The chance, in basis points, that `attacker`'s move with `base_accuracy` hits `target` right
/// now (accuracy and evasion stages applied; `None` = never misses). 10 000 or more means no
/// Accuracy decision is drawn. Shared by the hit gate and the replay driver.
pub fn accuracy_basis_points(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> u32 {
    accuracy_roll(state, rules, attacker, target, move_def)
        .map_or(10_000, |(numerator, denominator)| {
            u32::from(numerator) * 10_000 / u32::from(denominator)
        })
}

/// Showdown's accuracy check for `attacker`'s move on `target` right now, as the ratio it rolls
/// (`None`: no roll, the move hits) — percent in Gen 3+, a byte out of 256 in Gen 1-2
/// (`HitRules::chance_scale`). Shared by the hit gate and the replay driver.
pub fn accuracy_roll(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> Option<(u16, u16)> {
    if recharge_sleep_bug(state, rules, target, move_def)
        || no_guard(state, rules, attacker, target)
        // Lock-On / Mind Reader (§15.14).
        || context::locked_on(state, rules, attacker, target)
        || gen1_bound_by(state, rules, target, attacker)
        // Gen 3+ Pursuit's `onModifyMove`: a Pokémon switching out is not missed (§15.11).
        || (move_def.behavior.sure_hit_on_switching && state.called_back == Some(target))
        || minimized_sure_hit(state, rules, target, move_def)
        // Glaive Rush's `onAccuracy` on its user: every move at it hits (§15.20).
        || state.pokemon(target).is_some_and(|p| {
            p.volatiles
                .contains(crate::state::VolatileConditionKind::GlaiveRush)
        })
        // Telekinesis's `onAccuracy` on its holder: every move but a one-hit KO hits it (§41.5).
        || (move_def.ohko.is_none()
            && state.pokemon(target).is_some_and(|p| {
                p.volatiles
                    .contains(crate::state::VolatileConditionKind::Telekinesis)
            }))
        // Gen 1 Roar / Whirlwind (pokered `ResidualEffects1`: no accuracy test; I74, §15.17).
        || matches!(
            move_def.primary_effect,
            crate::dex::PrimaryEffect::Status(crate::dex::StatusMoveEffect::Inert)
        )
    {
        return None;
    }
    let base = if sure_hit(state, rules, attacker, move_def) {
        None
    } else {
        match move_def.behavior.weather_accuracy {
            Some(rule) => power::weather_accuracy(state, rules, target, move_def.accuracy, rule),
            None => move_def.accuracy,
        }
    };
    let modifiers = accuracy_modifiers(state, rules, attacker, target, move_def);
    // One-hit KO moves (Gen 2+) have their own accuracy (DESIGN.md §22.6).
    if move_def.ohko.is_some()
        && let (Some(user), Some(foe)) = (state.pokemon(attacker), state.pokemon(target))
    {
        let level_gap = u32::from(user.level.saturating_sub(foe.level));
        match rules.hits.ohko {
            crate::rules::OhkoRules::SpeedGate => {}
            crate::rules::OhkoRules::LevelGateByteBonus => {
                let byte = u32::from(base.unwrap_or(30)) * 255 / 100 + 2 * level_gap;
                return stats::byte_accuracy(
                    user.stages.accuracy,
                    foe.stages.evasion,
                    byte.min(255),
                    modifiers.byte_penalty,
                );
            }
            crate::rules::OhkoRules::LevelGatePercent { one_point_less, .. } => {
                let typed_penalty = move_def
                    .ohko
                    .and_then(|ohko| ohko.immune_type)
                    .is_some_and(|t| !crate::engine::types_of(state, rules, user).contains(t));
                let accuracy = if typed_penalty {
                    20
                } else {
                    u32::from(base.unwrap_or(30))
                } + level_gap;
                // Gen 3: `Random() % 100 + 1 < chance` — a chance of 100 still misses once in 100.
                let accuracy = if one_point_less {
                    accuracy.saturating_sub(1)
                } else {
                    accuracy
                };
                return (accuracy < 100).then_some((accuracy as u16, 100));
            }
        }
    }
    let self_target = move_def.target == crate::rules::TargetPattern::SelfOnly;
    // Wonder Skin first: a status move aimed at the holder has 50 % before anything else.
    let mut base = base;
    if let Some(foe) = state.pokemon(target)
        && move_def.category == crate::dex::MoveCategory::Status
        && base.is_some()
        && let Some(accuracy) = ability_handler(state, rules, foe, |b| b.status_accuracy)
    {
        base = Some(accuracy);
    }
    stats::accuracy_ratio(
        state,
        rules,
        attacker,
        target,
        base,
        self_target,
        move_def.stat_sources.ignore_evasion || unaware_ignores(state, rules, attacker, target),
        unaware_ignores(state, rules, target, attacker),
        &modifiers,
    )
}

/// Unaware (`onAnyModifyBoost`): whether `holder`'s ability sets aside `other`'s stat stages
/// in the move between them — `holder` using the move (the target's Defense, Special Defense
/// and evasion) or taken by it (the user's Attack, Defense, Special Attack and accuracy). A
/// Pokémon's own Unaware leaves its own stages alone (a confusion self-hit). Gen 5+, like every
/// `onAny` handler (`active_holder`), a holder at 0 HP has none: the user of a move that knocks
/// it out first (`selfdestruct: 'always'`) meets its target's stages (`StatRules::
/// unaware_needs_hp`, DESIGN.md R-M8-90).
pub(crate) fn unaware_ignores(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    holder: crate::ids::PokemonId,
    other: crate::ids::PokemonId,
) -> bool {
    holder != other
        && state
            .pokemon(holder)
            .filter(|p| !(rules.stats.unaware_needs_hp && p.is_fainted()))
            .is_some_and(|p| ability_handler(state, rules, p, |b| b.unaware))
}

/// The accuracy modifiers of `attacker`'s move on `target` (Showdown `ModifyAccuracy`, the
/// cartridge's modifiers): the user's ability (Compound Eyes, Hustle), the target's (Sand Veil,
/// Snow Cloak, Tangled Feet), the user's side's Victory Star, the target's item (Bright Powder,
/// Lax Incense), the user's item (Wide Lens, Zoom Lens). The Gen 5+ chain follows Showdown's
/// priorities (the abilities at −1, the items at −2); the Gen 3-4 percentages the cartridge's
/// order (`AccuracyStep`).
fn accuracy_modifiers(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> stats::AccuracyModifiers {
    let mut modifiers = stats::AccuracyModifiers::none();
    let (Some(user), Some(foe)) = (state.pokemon(attacker), state.pokemon(target)) else {
        return modifiers;
    };
    // Gravity's `onModifyAccuracy` (priority 0: ahead of the abilities and items).
    let gravity = state
        .field
        .contains(crate::state::FieldConditionKind::Gravity);
    if gravity {
        modifiers.chain = damage::chain_modifiers(modifiers.chain, GRAVITY_ACCURACY.0);
    }
    let mut steps: Vec<crate::effects::abilities::AccuracyModifier> = Vec::new();
    let mut add =
        |modifiers: &mut stats::AccuracyModifiers,
         modifier: Option<crate::effects::abilities::AccuracyModifier>| {
            if let Some(modifier) = modifier {
                modifiers.chain = damage::chain_modifiers(modifiers.chain, modifier.chain);
                modifiers.byte_penalty += modifier.byte_penalty;
                steps.push(modifier);
            }
        };
    add(
        &mut modifiers,
        ability_handler(state, rules, user, |b| b.accuracy_boost)
            .and_then(|boost| boost.modifier_for(state, rules, attacker, target, move_def)),
    );
    add(
        &mut modifiers,
        ability_handler(state, rules, foe, |b| b.evasion_boost)
            .and_then(|boost| boost.modifier_for(state, rules, target, attacker, move_def)),
    );
    for ally in state.side(user.side).active.iter().flatten() {
        if let Some(modifier) = state
            .pokemon(*ally)
            .filter(|p| !p.is_fainted())
            .and_then(|p| ability_handler(state, rules, p, |b| b.ally_accuracy_boost))
        {
            modifiers.chain = damage::chain_modifiers(modifiers.chain, modifier);
        }
    }
    add(
        &mut modifiers,
        held_item(state, rules, target).and_then(|(_, item)| {
            item.evasion_boost
                .and_then(|boost| boost.modifier_for(state, rules, target, attacker, move_def))
        }),
    );
    add(
        &mut modifiers,
        held_item(state, rules, attacker).and_then(|(_, item)| {
            item.accuracy_boost
                .and_then(|boost| boost.modifier_for(state, rules, attacker, target, move_def))
        }),
    );
    steps.sort_by_key(|modifier| modifier.step);
    modifiers.percentages = steps.iter().map(|m| (m.percent(rules), 100)).collect();
    // Micle Berry's boost (issues.md I42): Gen 5+ `onSourceAccuracy`, `chainModify([4915,
    // 4096])` once the stages are in; Gen 4 ×120 / 100 after the items, before Gravity.
    if micle_boosts(state, rules, attacker, move_def) {
        modifiers.after_stages = MICLE_ACCURACY.0;
        modifiers.percentages.push(MICLE_ACCURACY.1);
    }
    // The cartridge's Gravity comes last (pokeplatinum: after Micle Berry, ×10 / 6).
    if gravity {
        modifiers.percentages.push(GRAVITY_ACCURACY.1);
    }
    modifiers
}

/// Micle Berry's accuracy boost: Showdown's chain [4915, 4096] (Gen 5+) and the Gen 4 cartridge's
/// ×120 / 100 (pokeplatinum `BattleControllerPlayer_CheckMoveHitAccuracy`).
const MICLE_ACCURACY: (u32, (u32, u32)) = (4915, (120, 100));

/// Whether `attacker`'s move uses its Micle Berry boost (`BerryRules::micle`, issues.md I42):
/// not a one-hit KO move; Gen 5+ every move that runs the `Accuracy` event (not a status move
/// on its user, not — from Gen 8, which checks it in the hit steps — a move sure to hit through
/// the user's type: Gen 6-7's sure hit is the `toxic` volatile's own `onSourceAccuracy`, in the
/// event with Micle's, R-M10-63); Gen 4 only a move with an accuracy to check.
pub(crate) fn micle_boosts(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    let micle = rules.conditions.berries.micle;
    state.pokemon(attacker).is_some_and(|pokemon| {
        pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::MicleBerry)
    }) && move_def.ohko.is_none()
        && if micle.sure_hit_moves_use_it {
            !(move_def.target == crate::rules::TargetPattern::SelfOnly
                && move_def.category == crate::dex::MoveCategory::Status)
                && !(rules.conditions.typed_sure_hit == Some(crate::rules::TypedSureHit::HitSteps)
                    && sure_hit(state, rules, attacker, move_def))
        } else {
            move_def.accuracy.is_some()
        }
}

/// Gravity's accuracy boost: Showdown's chain [6840, 4096] (Gen 5+) and the Gen 4 cartridge's
/// ×10 / 6 (pokeplatinum `BattleControllerPlayer_CheckMoveHitAccuracy`, issues.md I5).
const GRAVITY_ACCURACY: (u32, (u32, u32)) = (6840, (10, 6));

/// No Guard (`onAnyAccuracy`, `onAnyInvulnerability`): on the user or the target, the move
/// hits — a semi-invulnerable target included. An `onAny` handler is one of the Pokémon on the
/// field beside the target and its foes (`findEventHandlers`: `alliesAndSelf`, `foes`), so a
/// foreseen attack's user that has left the field, or fallen, brings none (R-M8-165).
pub fn no_guard(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
) -> bool {
    [attacker, target].into_iter().any(|id| {
        state.active_position(id, &rules.topology).is_some()
            && state.pokemon(id).is_some_and(|p| {
                !p.is_fainted() && ability_handler(state, rules, p, |b| b.no_guard)
            })
    })
}

/// What an accuracy check spent (`spend_accuracy_check`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpentAccuracy {
    pub micle_berry: bool,
    pub locked_on: bool,
}

/// What an accuracy check of `actor`'s move on `target` uses up, taken off `state`: Micle
/// Berry's boost goes with the first check that uses it (issues.md I42), and a Lock-On sitting
/// on the target (Gen 2-4, `LockOnModel::OnTarget`: I68, I69) with the check that reads it. The
/// hit gate's accuracy step spends them; the replay driver asks the same for the checks after
/// the first of a `multiaccuracy` move (Triple Kick).
pub fn spend_accuracy_check(
    state: &mut crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> SpentAccuracy {
    let micle_berry = micle_boosts(state, rules, actor, move_def)
        && state.pokemon_mut(actor).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .remove(crate::state::VolatileConditionKind::MicleBerry)
        });
    let locked_on = rules.conditions.lock_on.model == crate::rules::LockOnModel::OnTarget
        && context::locked_on(state, rules, actor, target)
        && state.pokemon_mut(target).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .remove(crate::state::VolatileConditionKind::LockedOn)
        });
    SpentAccuracy {
        micle_berry,
        locked_on,
    }
}

/// Whether a one-hit KO move cannot affect `target` at all (Showdown `-immune|target|[ohko]`):
/// Gen 1 a faster target, Gen 2+ a higher-level one, Gen 7+ Sheer Cold an Ice type.
pub fn ohko_blocked(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    let Some(ohko) = move_def.ohko else {
        return false;
    };
    let (Some(user), Some(foe)) = (state.pokemon(attacker), state.pokemon(target)) else {
        return false;
    };
    // Showdown `hitStepAccuracy`: a Dynamaxed target is immune to a one-hit KO move outright
    // (`-immune|target|[ohko]`, whatever the levels say; DESIGN.md §44.5).
    if foe
        .volatiles
        .contains(crate::state::VolatileConditionKind::Dynamax)
    {
        return true;
    }
    match rules.hits.ohko {
        crate::rules::OhkoRules::SpeedGate => {
            stats::effective_speed(state, rules, target)
                > stats::effective_speed(state, rules, attacker)
        }
        crate::rules::OhkoRules::LevelGateByteBonus => user.level < foe.level,
        crate::rules::OhkoRules::LevelGatePercent { .. } => {
            user.level < foe.level
                || ohko
                    .immune_type
                    .is_some_and(|t| crate::engine::types_of(state, rules, foe).contains(t))
        }
    }
}

/// A move that cannot miss this user right now (Toxic from a Poison type, Gen 6+): no accuracy
/// check, and a semi-invulnerable target is reached.
pub fn sure_hit(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    attacker: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    rules.conditions.typed_sure_hit.is_some()
        && move_def.behavior.sure_hit_user_type.is_some_and(|t| {
            state
                .pokemon(attacker)
                .is_some_and(|p| crate::engine::types_of(state, rules, p).contains(t))
        })
}

/// Where a `minimize` move `move_def` doubles against `target` (`ConditionRules::minimize_bonus`;
/// `None`: no Minimize, or not a `minimize` move).
pub(crate) fn minimize_bonus(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> Option<crate::rules::DamageBonusStage> {
    (move_def.flags.minimize
        && state.pokemon(target).is_some_and(|p| {
            p.volatiles
                .contains(crate::state::VolatileConditionKind::Minimize)
        }))
    .then_some(rules.conditions.minimize_bonus)
}

/// Minimize's `onAccuracy` (Gen 6+, `ConditionRules::minimize_sure_hit`): a `minimize` move is
/// sure to hit the holder. Shared with the replay driver.
pub fn minimized_sure_hit(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    rules.conditions.minimize_sure_hit
        && move_def.flags.minimize
        && state.pokemon(target).is_some_and(|p| {
            p.volatiles
                .contains(crate::state::VolatileConditionKind::Minimize)
        })
}
