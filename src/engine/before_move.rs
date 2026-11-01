//! BeforeMove: may an already-queued Pokémon act? (DESIGN.md §15, ROADMAP §6.2)
//!
//! Checked in Showdown's `onBeforeMove` priority order, so a sleeping Pokémon never rolls for
//! paralysis and a flinched one never hits itself in confusion:
//!
//! ```text
//! Recharge (11)  →  Sleep / Freeze (10)  →  Truant / Gen 1 bound (9)  →  Flinch (8)  →  Disable (7)
//!   →  Heal Block / Throat Chop (6)  →  Gravity (6)  →  Taunt (5)  →  Confusion (3)
//!   →  Attract (2)  →  Paralysis (1)  →  Choice lock (0)
//! ```
//!
//! Gen 3-4 follow the cartridge instead (pokeemerald `AtkCanceller_UnableToUseMove`,
//! pokeplatinum `CheckStatusState`; issues.md I66, I105): Sleep / Freeze → Truant → Recharge → Flinch →
//! Disable → Taunt → Imprison → Gravity / Heal Block → Confusion → Paralysis → Attract. From Gen 5
//! Imprison (4) follows Taunt (5).
//!
//! Every random decision is a typed `RandomRequestKind`, every counter change and cure is a
//! trace event, and the confusion self-hit is an ordinary `DamageRequest` so it obeys the same
//! mutation boundary as any other HP change.

use crate::dex::{ResolvedDex, ResolvedMove};
use crate::effects::moves::BasePower;
use crate::ids::PokemonId;
use crate::rules::{DamageAlgorithm, DamageModifierKind, Fraction, RechargeCheck, ResolvedRules};
use crate::state::{BattleState, NonVolatileStatus, RandomRequestKind, VolatileConditionKind};

use super::damage::{
    DefensiveStat, FinalDamageModifier, OffensiveStat, apply_final_modifiers, base_damage,
};
use super::stats::apply_stage;
use super::{
    BattleContext, BattleTrace, CureReason, DamageAmount, DamageFlags, DamageKind, DamageRequest,
    EffectSource, ModifierSource, TraceEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeforeMoveBlockReason {
    Sleep,
    /// Gen 1: the turn the sleeper wakes is lost as well, with no `cant` (Showdown gen1
    /// `slp.onBeforeMove` writes `cant|p|slp` only while `time > 0`): no action anyone sees
    /// (R-M10-53).
    WakingTurn,
    Freeze,
    Paralysis,
    Flinch,
    Confusion,
    /// The turn after Hyper Beam is spent recharging.
    Recharge,
    /// The chosen move is the one Disable sealed.
    Disabled,
    /// Gravity refuses the airborne moves (`gravity` flag) chosen before it came up.
    Gravity,
    /// Taunt refuses status moves.
    Taunt,
    /// A foe holding Imprison knows the move.
    Imprison,
    /// Heal Block refuses moves that heal.
    HealBlock,
    /// Throat Chop refuses sound moves.
    ThroatChop,
    /// Immobilised by love (Attract).
    Attract,
    /// A Choice item or Gorilla Tactics only allows the move it locked onto.
    ChoiceLock,
    /// Gen 1: caught by a binding move (`partiallytrapped`), the Pokémon cannot move.
    PartiallyTrapped,
    /// Gen 5+ Focus Punch (`beforeMoveCallback`): hit before it struck, the user lost its focus.
    LostFocus,
    /// Truant: the holder loafs this turn (`cant … ability: Truant`, DESIGN.md §15.21).
    Truant,
    /// Sky Drop carries the Pokémon in the air (its condition's `onFoeBeforeMove`, priority 12:
    /// `null`, no message; DESIGN.md §15.23).
    SkyDropped,
    /// Commander's `commanding` (its condition's `onBeforeTurn` cancels the queued action):
    /// the Tatsugiri in its ally's mouth acts not at all, without a word (DESIGN.md §43.4).
    Commanding,
    VolatileRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanMoveResult {
    Allow,
    Blocked(BeforeMoveBlockReason),
}

/// Whether `pokemon` draws the full-paralysis chance before it moves: it is paralysed, and not
/// a Gen 3-4 Magic Guard holder (`StatusLifecycleRules::magic_guard_skips_full_paralysis`,
/// Showdown gen4 `par.onBeforeMove`'s `hasAbility('magicguard')`). The replay asks the same.
pub fn rolls_full_paralysis(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &crate::state::PokemonState,
) -> bool {
    pokemon.has_status(NonVolatileStatus::Paralysis)
        && !(rules.status.lifecycle.magic_guard_skips_full_paralysis
            && super::has_ability(state, rules, pokemon, crate::ids::AbilityId::MAGIC_GUARD))
}

/// Heal Block, Throat Chop and Gravity refuse a move both before it (`onBeforeMove`, priority
/// 6) and as it is modified (`onModifyMove`), so a move another one called too — `cant`, no
/// `move` line (DESIGN.md R-M8-70). Heal Block (a volatile) runs before Gravity (a field
/// condition, no speed); no move is both.
pub(crate) fn refused_as_the_move_starts(
    state: &BattleState,
    pokemon: &crate::state::PokemonState,
    move_def: &ResolvedMove,
) -> Option<BeforeMoveBlockReason> {
    if move_def.flags.heal && pokemon.volatiles.contains(VolatileConditionKind::HealBlock) {
        return Some(BeforeMoveBlockReason::HealBlock);
    }
    if move_def.flags.sound
        && pokemon
            .volatiles
            .contains(VolatileConditionKind::ThroatChop)
    {
        return Some(BeforeMoveBlockReason::ThroatChop);
    }
    // Gravity's `onBeforeMove` (priority 6): `cant … move: Gravity`.
    (move_def.flags.gravity
        && state
            .field
            .contains(crate::state::FieldConditionKind::Gravity))
    .then_some(BeforeMoveBlockReason::Gravity)
}

/// `is_z`: the move is a Z-Move (Showdown `isZ`; a status move's keeps its own data).
pub(crate) fn resolve(
    state: &mut BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    trace: &mut BattleTrace,
    actor: PokemonId,
    move_def: &ResolvedMove,
    is_z: bool,
) -> CanMoveResult {
    let lifecycle = rules.status.lifecycle;

    // Sky Drop's carried Pokémon (priority 12, before anything): its action comes to nothing,
    // silently, and does not count as one (`activeMoveActions--`).
    if let Some(pokemon) = state.pokemon_mut(actor)
        && pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::SkyDropped)
    {
        pokemon.active_move_actions = pokemon.active_move_actions.saturating_sub(1);
        return CanMoveResult::Blocked(BeforeMoveBlockReason::SkyDropped);
    }
    // Commander (§43.4): the Tatsugiri in its ally's mouth acts not at all — its queued action
    // was cancelled before the turn, so the log has nothing for it.
    if let Some(pokemon) = state.pokemon_mut(actor)
        && pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::Commanding)
    {
        pokemon.active_move_actions = pokemon.active_move_actions.saturating_sub(1);
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Commanding);
    }

    // 0. Recharge: the action is spent, the flag is cleared (Showdown priority 11; Gen 3-4's
    //    comes after sleep and freeze, step 1b; Gen 1's after them and flinch, step 2a).
    if rules.conditions.recharge_check == RechargeCheck::First && recharge(state, trace, actor) {
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Recharge);
    }

    // Gen 6 Stance Change (priority 11, after the recharge's volatile): the forme for the move
    // the Pokémon is about to try, before sleep or paralysis can stop it (DESIGN.md §43.4).
    if let Some(stance) = state
        .pokemon(actor)
        .and_then(|pokemon| super::ability_handler(state, rules, pokemon, |b| b.before_move_stance))
    {
        BattleContext::new(state, rules, trace).stance_change(
            actor,
            stance,
            move_def.id,
            move_def.category,
        );
    }

    // 1. Major status that can stop the action outright.
    if let Some(status) = state.pokemon(actor).and_then(|pokemon| pokemon.status) {
        match status.kind {
            // Gen 3-4 (the cartridge, issues.md I63): an uproar on the field wakes the
            // sleeper as it is about to move; it goes on with its move.
            NonVolatileStatus::Sleep
                if BattleContext::new(state, rules, trace).uproar_wakes_mover(actor) => {}
            NonVolatileStatus::Sleep => {
                // Showdown `time` convention: decrement first, wake at 0; Early Bird counts
                // twice (`hasAbility('earlybird')`; no callback of its own).
                let step = if state.pokemon(actor).is_some_and(|p| {
                    super::has_ability(state, rules, p, crate::ids::AbilityId::EARLY_BIRD)
                }) {
                    2
                } else {
                    1
                };
                let remaining = status
                    .sleep_turns_remaining
                    .unwrap_or(1)
                    .saturating_sub(step);
                if remaining > 0 {
                    let sleep_usable = move_def.flags.sleep_usable;
                    if let Some(pokemon) = state.pokemon_mut(actor)
                        && let Some(status) = &mut pokemon.status
                    {
                        status.sleep_turns_remaining = Some(remaining);
                        // Gen 3 (`skippedTime`): a turn spent on Snore / Sleep Talk is owed back
                        // at the next entry; a turn sleep stopped settles the debt.
                        if lifecycle.sleep_usable_turns_come_back {
                            status.sleep_turns_skipped = if sleep_usable {
                                status.sleep_turns_skipped.saturating_add(1)
                            } else {
                                0
                            };
                        }
                    }
                    // Snore, Sleep Talk (`sleepUsable`): `cant|user|slp`, then the move goes
                    // on to the other checks (DESIGN.md §42.1).
                    if !move_def.flags.sleep_usable {
                        return CanMoveResult::Blocked(BeforeMoveBlockReason::Sleep);
                    }
                    trace.push(TraceEvent::MovesWhileAsleep { actor });
                } else {
                    BattleContext::new(state, rules, trace).cure_status(actor, CureReason::WokeUp);
                    if !lifecycle.sleep_wake_turn_can_act {
                        return CanMoveResult::Blocked(BeforeMoveBlockReason::WakingTurn);
                    }
                }
            }
            NonVolatileStatus::Freeze => {
                // Gen 2 rolls the thaw at the end of the turn instead (`FreezeThaw`); a
                // `defrost` move skips the roll from Gen 5 (`super::freeze_thaw_rolls`).
                let thaws = super::freeze_thaw_rolls(state, rules, actor, move_def)
                    && state.rng.roll_ratio_for(
                        RandomRequestKind::Thaw,
                        lifecycle.thaw_chance.numerator,
                        lifecycle.thaw_chance.denominator,
                    );
                if thaws {
                    BattleContext::new(state, rules, trace).cure_status(actor, CureReason::Thawed);
                } else if !(super::defrost_move_for(state, rules, actor, move_def)
                    && lifecycle.defrost_user != crate::rules::DefrostUser::Never)
                {
                    return CanMoveResult::Blocked(BeforeMoveBlockReason::Freeze);
                }
                // Otherwise a frozen user of a `defrost` move goes on to the other checks;
                // the move thaws it (`MovePipeline::defrost_user`).
            }
            _ => {}
        }
    }

    // 1a. Truant (priority 9; the cartridges' between freeze and the recharge, issues.md I105):
    //     the holder loafs every other turn. Gen 3-4's recharge turn then passes with it.
    if BattleContext::new(state, rules, trace).truant_loafs(actor) {
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Truant);
    }

    // 1b. Gen 3-4 on the cartridge (issues.md I66): the recharge turn after sleep and freeze.
    if rules.conditions.recharge_check == RechargeCheck::AfterSleepAndFreeze
        && recharge(state, trace, actor)
    {
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Recharge);
    }

    // 1c. Gen 1 `partiallytrapped` (priority 9): caught by a binding move, no move at all.
    if rules.conditions.switching.binding_locks_user
        && state.pokemon(actor).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .contains(VolatileConditionKind::PartiallyTrapped)
        })
    {
        return CanMoveResult::Blocked(BeforeMoveBlockReason::PartiallyTrapped);
    }

    // 2. Flinch stops the move but stays until its one-turn duration runs out at the end of
    //    the residual (Showdown `flinch` `duration: 1`): a battle decided in the residual ends
    //    with it still on.
    let flinched = state
        .pokemon(actor)
        .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::Flinch));
    if flinched {
        // The flinch's `onBeforeMove` runs the Flinch event: Steadfast raises a stat.
        if let Some(ability) = state
            .pokemon(actor)
            .and_then(|pokemon| crate::engine::ability_running(state, rules, pokemon))
            && let Some(stat) = super::ability_behavior(rules, ability).flinch_boost
        {
            BattleContext::new(state, rules, trace).try_change_stat_stage(
                super::StatChangeRequest {
                    target: actor,
                    stat,
                    stages: 1,
                    source: EffectSource::Ability {
                        pokemon: actor,
                        ability,
                    },
                    cause: super::StatChangeCause::Ability,
                },
            );
        }
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Flinch);
    }

    // 2a. Gen 1's recharge turn (priority 7).
    if rules.conditions.recharge_check == RechargeCheck::AfterFlinch
        && recharge(state, trace, actor)
    {
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Recharge);
    }

    // 2b. Disable (7), Heal Block and Throat Chop (6) and Taunt (5) refuse the move before
    //     confusion gets a say (Gen 3-4 on the cartridge: Taunt right after Disable). Each
    //     leaves a Z-Move alone, Gravity and Imprison too (`move.isZ`): a status move's Z
    //     version, which keeps the move's id, category and flags, goes ahead (R-M10-41).
    let taunted = state.pokemon(actor).is_some_and(|pokemon| {
        pokemon.volatiles.contains(VolatileConditionKind::Taunt)
            && super::choice::taunt_refuses(rules, move_def)
    });
    let imprisoned = super::context::imprisoned_by(state, actor, move_def.id).is_some();
    // Gen 1 (priority 6): every BeforeMove counts the Disable down first; at 0 it ends and the
    // move goes ahead.
    if rules.conditions.disable_gen1
        && let Some(pokemon) = state.pokemon_mut(actor)
        && pokemon.volatiles.tick_timed(VolatileConditionKind::Disable) == Some(0)
    {
        trace.push(TraceEvent::VolatileEnded {
            target: actor,
            kind: VolatileConditionKind::Disable,
            seen: true,
        });
    }
    if !is_z && let Some(pokemon) = state.pokemon(actor) {
        if pokemon.volatiles.disabled_move() == Some(move_def.id) {
            return CanMoveResult::Blocked(BeforeMoveBlockReason::Disabled);
        }
        if taunted && rules.conditions.taunt_before_field_blocks {
            return CanMoveResult::Blocked(BeforeMoveBlockReason::Taunt);
        }
        // Imprison right after Taunt (Gen 3-4 on the cartridge, issues.md I66).
        if imprisoned && rules.conditions.taunt_before_field_blocks {
            return CanMoveResult::Blocked(BeforeMoveBlockReason::Imprison);
        }
        if let Some(reason) = refused_as_the_move_starts(state, pokemon, move_def) {
            return CanMoveResult::Blocked(reason);
        }
        if taunted {
            return CanMoveResult::Blocked(BeforeMoveBlockReason::Taunt);
        }
        // Imprison (`onFoeBeforeMovePriority` 4): a move a foe holding it knows.
        if imprisoned {
            return CanMoveResult::Blocked(BeforeMoveBlockReason::Imprison);
        }
    }

    // 3. Confusion: count down, snap out at 0, otherwise maybe hit yourself.
    let confused = state
        .pokemon(actor)
        .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::Confusion));
    if confused {
        let remaining = state
            .pokemon_mut(actor)
            .and_then(|pokemon| pokemon.volatiles.tick_confusion())
            .unwrap_or(0);
        if remaining == 0 {
            trace.push(TraceEvent::VolatileEnded {
                target: actor,
                kind: VolatileConditionKind::Confusion,
                seen: true,
            });
        } else if state.rng.roll_ratio_for(
            RandomRequestKind::ConfusionSelfHit,
            lifecycle.confusion_self_hit.numerator,
            lifecycle.confusion_self_hit.denominator,
        ) {
            let damage = confusion_self_damage(state, rules, dex, trace, actor, move_def);
            trace.push(TraceEvent::ConfusionSelfHit {
                target: actor,
                damage,
            });
            let (target, kind) = match super::self_damage_recipient(state, rules, actor) {
                super::SelfDamageRecipient::Own => (Some(actor), DamageKind::SelfHit),
                super::SelfDamageRecipient::FoeSubstitute(foe) => (Some(foe), DamageKind::Move),
                super::SelfDamageRecipient::Nothing => (None, DamageKind::SelfHit),
            };
            if let Some(target) = target {
                let mut ctx = BattleContext::new(state, rules, trace);
                let damage = if target == actor {
                    ctx.focus_sash_holds_self_hit(target, damage)
                } else {
                    damage
                };
                ctx.apply_damage(DamageRequest {
                    target,
                    amount: DamageAmount(damage),
                    kind,
                    source: EffectSource::Volatile(VolatileConditionKind::Confusion),
                    // Showdown's self-hit is a pseudo-move (`effectType: 'Move'`): Berserk and
                    // Anger Shell hold the healing berries back after it (issues.md I36).
                    flags: DamageFlags {
                        single_strike: true,
                        future_move: false,
                        ..DamageFlags::default()
                    },
                });
            } else if rules.conditions.retaliation.gen1_last_damage {
                // Gen 1: behind its own Substitute, with none across, the self-hit strikes nothing
                // but is the last damage all the same (`directDamage` sets `lastDamage` before it
                // looks at the Substitutes; pokered's `wDamage`): a Bide stores it, a Counter
                // answers it (DESIGN.md R-M9-25).
                state.last_damage = damage;
            }
            // Gen 1 (`confusion` `onBeforeMove`): the binding lock ends too.
            end_binding_lock(state, trace, actor);
            return CanMoveResult::Blocked(BeforeMoveBlockReason::Confusion);
        }
    }

    // 3a. Attract (priority 2): half the time the infatuated Pokémon cannot move — before
    //     paralysis (1), except in Gen 2, whose paralysis shares priority 2 and comes first, and
    //     in Gen 3-4 on the cartridge (issues.md I66).
    let attract_after_paralysis = rules.conditions.attract_after_paralysis;
    if !attract_after_paralysis && infatuated(state, actor) {
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Attract);
    }

    // 4. Full paralysis.
    let paralysed = state
        .pokemon(actor)
        .is_some_and(|pokemon| rolls_full_paralysis(state, rules, pokemon));
    if paralysed
        && state.rng.roll_ratio_for(
            RandomRequestKind::FullParalysis,
            lifecycle.full_paralysis.numerator,
            lifecycle.full_paralysis.denominator,
        )
    {
        // Gen 1 (`par` `onBeforeMove`): the binding lock ends too.
        end_binding_lock(state, trace, actor);
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Paralysis);
    }
    if attract_after_paralysis && infatuated(state, actor) {
        return CanMoveResult::Blocked(BeforeMoveBlockReason::Attract);
    }

    // 5. Choice lock (0), Gen 7+ (`ItemTransferRules::choice_lock_before_move`): only the locked
    // move goes through (Showdown `-fail`) — and Struggle.
    // The lock stays, but holds nothing while the Choice item is set aside (`ignoringItem()`:
    // Klutz, Embargo, Magic Room) or gone (Knock Off, Thief, Corrosive Gas: Gen 7+'s
    // `onBeforeMove` removes it here, which a frame before the turn's end sees; Gen 3-6 wait for
    // the turn's `DisableMove`, `BattleEngine::next_turn`). Gorilla Tactics' `onBeforeMove` (0)
    // refuses the same way for the ability's lock, while the ability runs.
    if rules.conditions.items.choice_lock_before_move
        && state.pokemon_mut(actor).is_some_and(|pokemon| {
            pokemon.volatiles.choice_locked_move().is_some()
                && !super::holds_choice_item(rules, pokemon)
                && pokemon.volatiles.remove(VolatileConditionKind::ChoiceLock)
        })
    {
        trace.push(TraceEvent::VolatileEnded {
            target: actor,
            kind: VolatileConditionKind::ChoiceLock,
            seen: true,
        });
    }
    let locks = state.pokemon(actor).map(|pokemon| {
        [
            pokemon
                .volatiles
                .choice_locked_move()
                .filter(|_| rules.conditions.items.choice_lock_before_move)
                .filter(|_| super::held_item_of(state, rules, pokemon).is_some()),
            pokemon
                .ability_state
                .choice_lock
                .filter(|_| super::ability_handler(state, rules, pokemon, |b| b.choice_lock)),
        ]
    });
    // A Z-powered move is neither refused by a Choice item's lock nor by Gorilla Tactics'
    // (`move.isZOrMaxPowered`, DESIGN.md §44.4) — and a Dynamaxed holder's locks refuse nothing
    // (`choicelock.onBeforeMove`'s `!pokemon.volatiles['dynamax']`; Gorilla Tactics' skips every
    // Max Move, §44.5).
    let z_powered = state.active_move.is_some_and(|active| active.z_powered);
    let dynamaxed = super::dynamaxed(state, actor);
    if !z_powered
        && !dynamaxed
        && locks
            .into_iter()
            .flatten()
            .flatten()
            .any(|locked| locked != move_def.id)
        && move_def.id != crate::ids::MoveId::STRUGGLE
    {
        return CanMoveResult::Blocked(BeforeMoveBlockReason::ChoiceLock);
    }

    CanMoveResult::Allow
}

/// Gen 1: a fully paralysed or self-hitting user drops its binding lock
/// (`removeVolatile('partialtrappinglock')`); only Gen 1 has one.
fn end_binding_lock(state: &mut BattleState, trace: &mut BattleTrace, actor: PokemonId) {
    if state.pokemon_mut(actor).is_some_and(|pokemon| {
        pokemon
            .volatiles
            .remove(VolatileConditionKind::PartialTrappingLock)
    }) {
        trace.push(TraceEvent::VolatileEnded {
            target: actor,
            kind: VolatileConditionKind::PartialTrappingLock,
            seen: true,
        });
    }
}

/// Attract's `onBeforeMove`: an infatuated Pokémon draws an `Infatuation` decision, 1 in 2 to
/// be immobilised by love.
fn infatuated(state: &mut BattleState, actor: PokemonId) -> bool {
    state
        .pokemon(actor)
        .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::Attract))
        && state
            .rng
            .roll_ratio_for(RandomRequestKind::Infatuation, 1, 2)
}

/// Spends a pending recharge turn: clears the flag and says whether there was one. The
/// recharge turn is a loafing turn too (Showdown `mustrecharge`'s `onBeforeMove`:
/// `removeVolatile('truant')`, §15.21).
fn recharge(state: &mut BattleState, trace: &mut BattleTrace, actor: PokemonId) -> bool {
    let recharging = state.pokemon_mut(actor).is_some_and(|pokemon| {
        pokemon
            .volatiles
            .remove(VolatileConditionKind::MustRecharge)
    });
    if recharging {
        trace.push(TraceEvent::VolatileEnded {
            target: actor,
            kind: VolatileConditionKind::MustRecharge,
            seen: true,
        });
        if state
            .pokemon_mut(actor)
            .is_some_and(|pokemon| pokemon.volatiles.remove(VolatileConditionKind::Truant))
        {
            trace.push(TraceEvent::VolatileEnded {
                target: actor,
                kind: VolatileConditionKind::Truant,
                seen: true,
            });
        }
    }
    recharging
}

/// A 40-power typeless physical hit against oneself: own Attack vs own Defense with stages,
/// then the generation's final-damage order restricted to the steps that apply (base offset,
/// random roll, burn). No STAB, type, crit, spread or weather (Showdown `getDamage(…, 40)`).
/// Gen 2 runs its own formula with no damage variance (`noDamageVariance`), and the chosen
/// move's `selfdestruct` comes along: an Explosion its user hit itself instead of halves its own
/// Defense (the gen2 condition's self-hit move, DESIGN.md R281).
fn confusion_self_damage(
    state: &mut BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    trace: &mut BattleTrace,
    actor: PokemonId,
    chosen: &ResolvedMove,
) -> u16 {
    if rules.damage.algorithm == DamageAlgorithm::Gen1Legacy {
        return super::legacy_damage::gen1_confusion_damage(state, actor);
    }
    // Gen 3-4: the whole damage formula (Showdown gen4 `getDamage(pokemon, pokemon, 40)`); from
    // Gen 5 `getConfusionDamage` below: the stats alone, a burn does not halve it.
    if rules.status.lifecycle.confusion_self_hit_burn_halves {
        return super::move_pipeline::MovePipeline::new(state, rules, dex, trace)
            .confusion_self_hit_damage(actor);
    }
    if rules.damage.algorithm == DamageAlgorithm::Gen2 {
        let hit = super::legacy_damage::LegacyHit {
            attacker: actor,
            defender: actor,
            base_power: 40,
            move_type: None,
            pair: super::legacy_damage::StatPair::Physical,
            critical: false,
            variance: rules.status.lifecycle.confusion_self_hit_varies,
            type_effectiveness: Fraction::ONE,
            weather: Fraction::ONE,
            item_boost: None,
            minimized: false,
            type_index_damage: false,
            beat_up: None,
            halves_defense: chosen.behavior.halves_defense
                && rules.conditions.self_faint.explosion_halves_defense,
            multiplier: None,
        };
        return super::legacy_damage::gen2_damage(state, rules, trace, &hit).damage;
    }
    let Some(pokemon) = state.pokemon(actor) else {
        return 0;
    };
    let offense = OffensiveStat(apply_stage(
        rules,
        pokemon.battle_stats().attack,
        pokemon.stages.attack,
    ));
    // `calculateStat('def')`: Wonder Room swaps the stored defenses.
    let defense = DefensiveStat(apply_stage(
        rules,
        super::stats::stored_stat(state, pokemon, crate::state::Stat::Defense),
        pokemon.stages.defense,
    ));
    let base = base_damage(
        rules.damage.algorithm,
        pokemon.level,
        BasePower(40),
        offense,
        defense,
    );
    let roll = state.rng.range_inclusive_for(
        RandomRequestKind::DamageRoll,
        rules.damage.random_min,
        rules.damage.random_max,
    );
    let order = |kind: DamageModifierKind| -> i16 {
        rules
            .damage
            .modifier_order
            .iter()
            .position(|candidate| *candidate == kind)
            .map_or(i16::MAX, |index| index as i16)
    };
    let mut modifiers = vec![
        FinalDamageModifier::offset(
            ModifierSource::Rule("BaseOffset"),
            order(DamageModifierKind::BaseOffset),
            2,
        ),
        FinalDamageModifier::new(
            ModifierSource::Rule("Random"),
            order(DamageModifierKind::Random),
            Fraction::new(roll, rules.damage.random_denominator),
        ),
    ];
    let (amount, _) = apply_final_modifiers(base, &mut modifiers, rules.damage.rounding, 1);
    amount.0
}
