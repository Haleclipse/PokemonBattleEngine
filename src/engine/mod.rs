mod ability;
mod action;
mod battle;
mod before_move;
mod choice;
mod context;
mod damage;
mod environment;
mod error;
mod execution;
mod gimmicks;
mod held_item;
mod legacy_damage;
mod modifier;
mod move_pipeline;
mod mutation;
mod power;
mod protections;
mod reactions;
mod screens;
mod stats;
pub use stats::effective_speed;
mod status;
mod trace;
pub mod two_turn;
mod typing;
mod warnings;
mod weather;

pub use crate::state::Stat;
mod calls;
pub mod pledges;
pub use calls::{
    assist_candidates, blocked_by_the_user, called_move_aims_at_random, copycat_move,
    metronome_candidates, mirror_move_from, nature_power_move, sleep_talk_candidates,
};

pub use ability::{
    Handler, ability_active, ability_handler, ability_handlers, ability_running,
    active_ability_handler, core_enforcer_suppresses, dancer_target, has_ability, ignoring_ability,
    poison_puppeteer_confuses, raw_ability, suppressing_ability, trace_candidates,
};
pub use action::{ActionOrder, ActionQueue, ActionStep, PriorityChance, QueuedAction};
pub use battle::{
    ActionOrderPreview, BattleEngine, DurationHandler, ReplacementRequirement, ReplacementSlot,
    ResidualTies, SeenPokemon, SideView, TurnResult, fractional_priority,
};
pub use before_move::{BeforeMoveBlockReason, CanMoveResult, rolls_full_paralysis};
pub use choice::{
    ActionChoice, ActorOptions, ChoiceError, ChoiceOptions, ForcedReplacement, ForcedSwitchChoice,
    MaxMoveOption, MechanicChoice, MechanicOptions, MoveOption, ReviveChoice, SideChoice,
    SlotChoice, Trapped, ZMoveOption, choice_options, mechanic_options, must_struggle,
    trapped_at_turn_end, trapped_by, trapped_by_ability, validate_side_choice,
};
pub(crate) use context::PendingFaints;
pub use context::STAGED_STATS;
pub use context::{
    AbilityEndCause, BasePowerContext, BattleContext, ConditionApplyError, EffectSource,
    OffensiveStatContext, StallEffect, TryHitContext,
};
pub use context::{
    AfterHit, AfterHitHandler, DamageResolutionHandler, DamagingHit, DamagingHitReactionHandler,
    ModifyStatChangeHandler, StatChangeContext, TryStatChangeHandler,
};
pub use context::{DrainedHeal, FormeChangeKind, HealDrain, uproar_wakes_before_move};
pub(crate) use damage::showdown_modify;
pub use damage::{
    AccuracyChance, CriticalHit, DamageAmount, DamageApplication, DamageBreakdown, DefensiveStat,
    HitBlockReason, OffensiveStat, TryHitResult,
};
pub use error::{BattleError, BattleInitError};
pub(crate) use execution::PendingExecution;
pub use execution::{BattleRequest, BattleSnapshot, EngineStep};
pub(crate) use gimmicks::spend_mechanic;
pub use gimmicks::{
    called_z_move, can_dynamax, max_move_execution, max_move_for, run_z_power, z_move_execution,
};
pub(crate) use held_item::{fling_item, ignores_items, natural_gift_berry, sets_any_item_aside};
pub use held_item::{
    has_item, held_item, held_item_id, held_item_of, holds_choice_item, item_refuses_take,
};
pub use modifier::{
    ModifierApplication, ModifierDescriptor, ModifierOrder, ModifierSource, ModifierStage,
    OrderedHandler, sort_by_showdown_priority, sort_descriptors,
};
pub(crate) use move_pipeline::MovePipeline;
pub use move_pipeline::{HitOutcome, MoveExecution, ZMoveExecution};
pub use move_pipeline::{
    curse_redirect_candidates, deciding_redirector, future_strike, random_retarget_candidates,
    redirector_cancels_smart_target, smart_targets, type_immunity,
};
pub use mutation::{
    DamageDecision, DamageFlags, DamageKind, DamageRequest, HealBlockReason, HealRequest,
    HealResult, StatChangeBlockReason, StatChangeCause, StatChangeRequest, StatChangeResult,
    TryStatChangeResult,
};
pub use power::{
    MoveUse, formula_power, power_condition_holds, weather_accuracy, weather_heal_amount,
    weight_hg, zero_power_does_nothing,
};
pub use protections::{infiltrates, priority_guard};
pub use reactions::{
    ChanceReactors, EFFECT_SPORE_SAMPLE, HIT_REACTION_CHANCE, chance_reactors, cursed_body_rolls,
    cute_charm_rolls, dealt_hit_status, effect_spore_roll, effect_spore_status, move_makes_contact,
    shields_secondaries,
};
pub use status::{
    StatusApplyResult, StatusBlockReason, TrySetStatusContext, TrySetStatusResult,
    ability_refuses_status, status_type_immune,
};
pub use trace::{
    AbilityChangeLine, BattleEndRule, BattleTrace, CureReason, MoveFailReason, QueueReorderReason,
    TraceCategory, TraceEvent, TraceFilter, TraceMode,
};
pub use typing::{
    StabModifier, TypeSet, conversion_candidates, conversion2_candidates, drive_type, has_type,
    item_species, memory_type, plate_type, pre_tera_types_of, protean_type, raw_memory_type,
    raw_plate_type, species_of, stab_modifier, types_locked, types_of,
};
pub use warnings::{anticipates, forewarn_candidates};
pub use weather::{field_weather, weather_for};

/// Whether a move's `onTry` lets it go ahead (Fake Out: only the user's first move action since
/// it came in, Showdown `activeMoveActions`; Dark Void: only Darkrai; Poltergeist: only against
/// a held item; Aurora Veil: only in hail / snow; Sucker Punch: only against a target about to
/// attack; Last Resort: only once every other move was used).
pub fn try_use_holds(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: Option<crate::ids::PokemonId>,
    try_use: crate::effects::moves::TryUse,
) -> bool {
    match try_use {
        crate::effects::moves::TryUse::FirstAction => state
            .pokemon(user)
            .is_some_and(|pokemon| pokemon.active_move_actions <= 1),
        crate::effects::moves::TryUse::UserHpBelowTarget => user_hp_below(state, user, target),
        crate::effects::moves::TryUse::UserSpecies(species) => state
            .pokemon(user)
            .is_some_and(|pokemon| species_of(pokemon) == Some(species)),
        crate::effects::moves::TryUse::UserSpeciesFamily(species) => {
            state.pokemon(user).is_some_and(|pokemon| {
                species_of(pokemon).is_some_and(|battling| battling.same_species(species))
            })
        }
        crate::effects::moves::TryUse::TargetHoldsItem => target
            .and_then(|target| state.pokemon(target))
            .is_some_and(|target| target.held_item.is_some()),
        crate::effects::moves::TryUse::InWeather(kind) => field_weather(state, rules) == Some(kind),
        crate::effects::moves::TryUse::TargetProtected => target
            .and_then(|target| state.pokemon(target))
            .is_some_and(|target| {
                target
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::Protect)
            }),
        crate::effects::moves::TryUse::InTerrain => state.field.terrain.is_some(),
        crate::effects::moves::TryUse::TargetAttacksNext => target.is_some_and(|target| {
            let recharging = state.pokemon(target).is_some_and(|pokemon| {
                pokemon
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::MustRecharge)
            });
            !recharging
                && state.will_move_with(target).is_some_and(|queued| {
                    queued.category != crate::dex::MoveCategory::Status
                        || (rules.conditions.calls.me_first_counts_as_an_attack
                            && queued.move_id == crate::ids::MoveId::ME_FIRST)
                })
        }),
        crate::effects::moves::TryUse::TargetUsesPriorityMove => target
            .and_then(|target| state.will_move_with(target))
            .is_some_and(|queued| {
                queued.priority > 0 && queued.category != crate::dex::MoveCategory::Status
            }),
        crate::effects::moves::TryUse::StrikesBack(kind) => {
            retaliation_record(state, rules, user, kind).is_some()
        }
        crate::effects::moves::TryUse::OtherMovesUsed => {
            state.pokemon(user).is_some_and(|pokemon| {
                pokemon.move_slots().len() >= 2
                    && pokemon
                        .move_slots()
                        .iter()
                        .any(|slot| slot.move_id == crate::ids::MoveId::LAST_RESORT)
                    && pokemon
                        .move_slots()
                        .iter()
                        .all(|slot| slot.move_id == crate::ids::MoveId::LAST_RESORT || slot.used)
            })
        }
        crate::effects::moves::TryUse::Stockpiled => state
            .pokemon(user)
            .is_some_and(|pokemon| pokemon.volatiles.stockpile_layers() > 0),
        crate::effects::moves::TryUse::UserHoldsBerry => state
            .pokemon(user)
            .and_then(|pokemon| pokemon.held_item)
            .is_some_and(|item| item_behavior(rules, item).is_berry),
        crate::effects::moves::TryUse::UserAteBerry => state
            .pokemon(user)
            .is_some_and(|pokemon| pokemon.history.ate_berry),
        crate::effects::moves::TryUse::UserAsleep => state.pokemon(user).is_some_and(|pokemon| {
            pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                || has_ability(state, rules, pokemon, crate::ids::AbilityId::COMATOSE)
        }),
    }
}

fn user_hp_below(
    state: &crate::state::BattleState,
    user: crate::ids::PokemonId,
    target: Option<crate::ids::PokemonId>,
) -> bool {
    match (state.pokemon(user), target.and_then(|t| state.pokemon(t))) {
        (Some(user), Some(foe)) => user.hp < foe.hp,
        _ => true,
    }
}

/// Who takes damage a Pokémon deals itself (a confusion self-hit, a crash).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelfDamageRecipient {
    Own,
    /// Gen 1: behind its own Substitute, the foe's Substitute takes it.
    FoeSubstitute(crate::ids::PokemonId),
    /// Gen 1: behind its own Substitute with no foe Substitute, nothing is dealt.
    Nothing,
}

/// Gen 1 (Showdown `directDamage`, not Stadium): confusion and Jump Kick / High Jump Kick crash
/// damage taken behind the Pokémon's own Substitute strikes the foe's Substitute instead, or
/// nothing when the foe has none (`ConditionRules::substitute_gen1`).
pub(crate) fn self_damage_recipient(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: crate::ids::PokemonId,
) -> SelfDamageRecipient {
    let behind_substitute = |id: crate::ids::PokemonId| {
        state.pokemon(id).is_some_and(|p| {
            p.volatiles
                .contains(crate::state::VolatileConditionKind::Substitute)
        })
    };
    if !rules.conditions.substitute_gen1 || !behind_substitute(actor) {
        return SelfDamageRecipient::Own;
    }
    match state
        .pokemon(actor)
        .and_then(|p| state.side(p.side.other()).active.first().copied().flatten())
        .filter(|foe| behind_substitute(*foe))
    {
        Some(foe) => SelfDamageRecipient::FoeSubstitute(foe),
        None => SelfDamageRecipient::Nothing,
    }
}

/// Whether `target` is immune to `user`'s move by the move's own `onTryImmunity`
/// (`MoveBehavior::try_immunity`: Endeavor, Dream Eater, Synchronoise, Octolock). Shared by the hit gate
/// and the replay driver.
pub fn move_immune(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    let Some(rule) = move_def.behavior.try_immunity else {
        return false;
    };
    let (Some(attacker), Some(foe)) = (state.pokemon(user), state.pokemon(target)) else {
        return false;
    };
    let holds = match rule {
        crate::effects::moves::TryImmunity::UserHpBelowTarget => {
            user_hp_below(state, user, Some(target))
        }
        crate::effects::moves::TryImmunity::TargetAsleep => {
            let asleep = counts_as_asleep(state, rules, foe);
            let behind_substitute = rules.hits.dream_eater_substitute_immune
                && foe
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::Substitute);
            asleep && !behind_substitute
        }
        crate::effects::moves::TryImmunity::SharesUserType => {
            crate::engine::types_of(state, rules, attacker)
                .iter()
                .any(|t| crate::engine::types_of(state, rules, foe).contains(t))
        }
        crate::effects::moves::TryImmunity::OppositeGender => foe.gender.opposite(attacker.gender),
        crate::effects::moves::TryImmunity::Trappable => {
            !(rules.conditions.ghost_escapes_trapping
                && crate::engine::types_of(state, rules, foe)
                    .contains(crate::dex::PokemonType::Ghost))
        }
        crate::effects::moves::TryImmunity::NoStickyHold => {
            !crate::engine::has_ability(state, rules, foe, crate::ids::AbilityId::STICKY_HOLD)
        }
        crate::effects::moves::TryImmunity::NotTruantOrInsomnia => !matches!(
            crate::engine::raw_ability(foe),
            crate::ids::AbilityId::TRUANT | crate::ids::AbilityId::INSOMNIA
        ),
        crate::effects::moves::TryImmunity::NotGrass => {
            !crate::engine::types_of(state, rules, foe).contains(crate::dex::PokemonType::Grass)
        }
    };
    !holds
}

/// The ability's behavior in this battle's generation (DESIGN.md §25.1): a lookup in the
/// generation's pre-resolved registry. Every engine lookup of an ability goes through here.
pub fn ability_behavior(
    rules: &crate::rules::ResolvedRules,
    ability: crate::ids::AbilityId,
) -> &'static crate::effects::abilities::AbilityBehavior {
    crate::effects::BehaviorRegistry::for_generation(rules.generation).ability(ability)
}

/// Whether the Pokémon is Dynamaxed (the `dynamax` volatile, DESIGN.md §44.5): what its entry
/// doubles the HP for and what suspends the Choice items', Gorilla Tactics' and Liquid Voice's
/// bonuses.
pub fn dynamaxed(state: &crate::state::BattleState, pokemon: crate::ids::PokemonId) -> bool {
    state.pokemon(pokemon).is_some_and(|pokemon| {
        pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::Dynamax)
    })
}

/// The abilities on `pokemon`'s side of the field that stand by it — its own and its active
/// allies' (Showdown's `onAlly…` handlers run for `alliesAndSelf`).
pub(crate) fn allied_abilities(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: crate::ids::PokemonId,
) -> Vec<&'static crate::effects::abilities::AbilityBehavior> {
    allied_ability_holders(state, rules, pokemon)
        .into_iter()
        .map(|(_, behavior)| behavior)
        .collect()
}

/// `allied_abilities`, each with its holder.
pub(crate) fn allied_ability_holders(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: crate::ids::PokemonId,
) -> Vec<(
    crate::ids::PokemonId,
    &'static crate::effects::abilities::AbilityBehavior,
)> {
    let Some(side) = state.pokemon(pokemon).map(|p| p.side) else {
        return Vec::new();
    };
    state
        .side(side)
        .active
        .iter()
        .flatten()
        .filter_map(|id| state.pokemon(*id).map(|ally| (*id, ally)))
        .filter(|(_, ally)| !ally.is_fainted())
        .map(|(id, ally)| (id, ability_handlers(state, rules, ally)))
        .collect()
}

/// Showdown `status === 'slp' || hasAbility('comatose')`: asleep, or counted as asleep
/// (Comatose) by the moves that ask.
pub fn counts_as_asleep(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: &crate::state::PokemonState,
) -> bool {
    pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
        || ability_handler(state, rules, pokemon, |b| b.counts_as_asleep)
}

/// The item's behavior in this battle's generation (DESIGN.md §25.1, §31.2), from the
/// generation's pre-resolved registry. Every engine lookup of an item goes through here.
pub fn item_behavior(
    rules: &crate::rules::ResolvedRules,
    item: crate::ids::ItemId,
) -> &'static crate::effects::items::ItemBehavior {
    crate::effects::BehaviorRegistry::for_generation(rules.generation).item(item)
}

/// Whether `pokemon` shrugs off powder moves (Spore, Rage Powder's redirection) and, for the
/// ability / item, weather chip: a Grass type from Gen 6, Overcoat from Gen 6, Safety Goggles.
/// Returns the source of the immunity.
pub fn powder_immunity(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: crate::ids::PokemonId,
) -> Option<EffectSource> {
    if let Some(by) = powder_holds(state, rules, pokemon) {
        return Some(by);
    }
    let p = state.pokemon(pokemon)?;
    if !rules.conditions.powder_immunity {
        return None;
    }
    crate::engine::types_of(state, rules, p)
        .contains(crate::dex::PokemonType::Grass)
        .then_some(EffectSource::Rule)
}

/// Whether `pokemon`'s ability or item shrugs powder moves off at the TryHit step (Overcoat,
/// Safety Goggles; Showdown's `onTryHit` handlers, DESIGN.md R279). The Grass type's powder
/// immunity is the TryImmunity step's, after the abilities: Sap Sipper absorbs a Sleep Powder
/// before the powder is even looked at.
pub fn powder_holds(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: crate::ids::PokemonId,
) -> Option<EffectSource> {
    // Showdown's handlers ask `getImmunity('powder', …)`, which only exists from Gen 6; before
    // it Overcoat and Safety Goggles let every powder move through (their weather half stays).
    if !rules.conditions.powder_immunity {
        return None;
    }
    let p = state.pokemon(pokemon)?;
    // The same question passes a Grass type over (`getImmunity` is the type's own immunity):
    // the TryImmunity step turns the powder away, its `-immune` naming nothing (R-M10-39).
    if crate::engine::types_of(state, rules, p).contains(crate::dex::PokemonType::Grass) {
        return None;
    }
    if let Some((item, behavior)) = held_item(state, rules, pokemon)
        && behavior.weather_and_powder_immune
    {
        return Some(EffectSource::Item { pokemon, item });
    }
    if ability_handler(state, rules, p, |b| b.weather_and_powder_immune) {
        return Some(EffectSource::Ability {
            pokemon,
            ability: crate::engine::raw_ability(p),
        });
    }
    None
}

/// Gen 7+ `hitStepTryImmunity`: a Dark-type foe is immune to a move Prankster boosted
/// (`boosted`, Showdown `pranksterBoosted`) while its user still has Prankster.
pub fn prankster_immune(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    boosted: bool,
) -> bool {
    let (Some(user), Some(foe)) = (state.pokemon(user), state.pokemon(target)) else {
        return false;
    };
    rules.action_order.prankster_dark_immunity
        && boosted
        && has_ability(state, rules, user, crate::ids::AbilityId::PRANKSTER)
        && user.side != foe.side
        && crate::engine::types_of(state, rules, foe).contains(crate::dex::PokemonType::Dark)
}

/// Gen 1's Hyper Beam sleep bug (`ConditionRules::recharge_sleep_bug`): a sleep move aimed at a
/// Pokémon that still owes its recharge turn.
pub(crate) fn recharge_sleep_bug(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    rules.conditions.recharge_sleep_bug
        && matches!(
            move_def.primary_effect,
            crate::dex::PrimaryEffect::Status(crate::dex::StatusMoveEffect::SetStatus(
                crate::state::NonVolatileStatus::Sleep
            ))
        )
        && state.pokemon(target).is_some_and(|p| {
            p.volatiles
                .contains(crate::state::VolatileConditionKind::MustRecharge)
        })
}

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
        // Guardian of Alola: three quarters, or a quarter of that through a protection — the
        // target's own or its side's Mat Block (Showdown's own `-zbroken` in the callback,
        // DESIGN.md §44.4; its `getUndynamaxedHP()` too, §44.5). The quarter rounds half down
        // (`Math.ceil(hp75 / 4 - 0.5)`: 117 gives 29, R-M8-184).
        crate::dex::FixedDamage::ThreeQuartersTargetHp => {
            let three_quarters = (u32::from(foe.undynamaxed_hp(None)) * 3 / 4).max(1);
            let protected = foe.volatiles.protect_kind().is_some()
                || state
                    .side(foe.side)
                    .conditions
                    .contains(crate::state::SideConditionKind::MatBlock);
            if protected {
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

/// The move as its `onPrepareHit` handlers leave it, once the targets are known. The move's
/// own first: Fling's power from the item it throws (`fling_item`) and the item's status or
/// flinch as a secondary without a chance — unless Gen 4 Embargo on the target keeps it away
/// (`ItemTransferRules::embargo_stops_flung_effects`, issues.md I92); Natural Gift's power from
/// its berry (`natural_gift_berry`). Then Parental Bond's: the move strikes twice, the
/// second hit weakened (`MoveFlags::parental_bond`, `DamageRules::parental_bond_second_hit`) —
/// a damaging single-hit move that is neither a spread hit (`targets > 1`) nor exempt
/// (`MoveFlags::parental_bond_exempt`). `target`: the move's (first) target. Shared by the
/// Shell and the replay driver.
pub fn prepared_move(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    mut move_def: crate::dex::ResolvedMove,
    targets: usize,
    target: Option<crate::ids::PokemonId>,
) -> crate::dex::ResolvedMove {
    if let Some(pokemon) = state.pokemon(user) {
        if move_def.behavior.fling
            && let Some((_, item, power)) = fling_item(state, rules, pokemon)
        {
            move_def.base_power = power;
            let embargoed = rules.conditions.items.embargo_stops_flung_effects
                && target
                    .and_then(|target| state.pokemon(target))
                    .is_some_and(|target| {
                        target
                            .volatiles
                            .contains(crate::state::VolatileConditionKind::Embargo)
                    });
            if let Some(effect) = item.fling_effect
                && !embargoed
            {
                use crate::dex::{SecondaryEffect, SecondaryEffectSpec, SecondaryTarget};
                use crate::effects::items::FlingEffect;
                let secondary = match effect {
                    FlingEffect::Status(status) => Some(SecondaryEffect::Status(status)),
                    FlingEffect::Flinch => Some(SecondaryEffect::Volatile(
                        crate::state::VolatileConditionKind::Flinch,
                    )),
                    // `fling.effect` is the move's `onHit`, not a secondary.
                    FlingEffect::ClearLoweredStages | FlingEffect::CureMentalHerbVolatiles => {
                        move_def.behavior.flung_on_hit = Some(effect);
                        None
                    }
                };
                if let Some(effect) = secondary {
                    move_def.secondaries.push(SecondaryEffectSpec {
                        chance: crate::dex::EffectChance::CERTAIN,
                        target: SecondaryTarget::Target,
                        trigger: crate::dex::SecondaryTrigger::PerTargetHit,
                        effect,
                        requires: None,
                        shares_roll: false,
                        in_secondaries: true,
                        from_item: false,
                    });
                }
            }
        }
        if move_def.behavior.natural_gift
            && let Some((_, gift)) = natural_gift_berry(state, rules, pokemon)
        {
            move_def.base_power = gift.power;
        }
    }
    let bond = state
        .pokemon(user)
        .is_some_and(|pokemon| ability_handler(state, rules, pokemon, |b| b.parental_bond))
        // `!(move.isZ || move.isMax)`: a Z-powered move strikes once (DESIGN.md §44.4), and so
        // does a Z-Move or a Max Move a custom game put in a slot (R-M8-80).
        && !state.active_move.is_some_and(|active| active.z_powered)
        && move_def.z_crystal.is_none()
        && !move_def.is_max
        && move_def.category != crate::dex::MoveCategory::Status
        && move_def.multi_hit == crate::dex::MultiHit::Single
        && !move_def.behavior.beat_up
        && !move_def.flags.parental_bond_exempt
        && targets <= 1;
    if bond {
        move_def.multi_hit = crate::dex::MultiHit::Fixed(2);
        move_def.flags.parental_bond = true;
    }
    move_def
}

/// The move as `user` uses it right now (Showdown `ModifyType` / `ModifyMove` as it starts):
/// the move's own type and category (Weather Ball, Revelation Dance, Hidden Power, Gen 2-4
/// Beat Up), then the user's ability retyping it (Aerilate …, Liquid Voice, Normalize) — the
/// move's own events run before the Pokémon's; in Gen 1-3 a category the move does not set
/// itself follows the new type; Infiltrator's moves bypass a Substitute from Gen 6. `physical`:
/// the move's own `onModifyMove` made it physical against its target (Shell Side Arm — the
/// caller decides, `physical_by_estimate` or a tie's coin), before the ability edits the move.
/// Returns the ability that changed the type when it also boosts the move
/// (`typeChangerBoosted`). Shared by the Shell and the replay driver.
pub(crate) fn modified_move_cow<'a>(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    move_def: &'a crate::dex::ResolvedMove,
    physical: bool,
) -> (
    std::borrow::Cow<'a, crate::dex::ResolvedMove>,
    Option<crate::ids::AbilityId>,
) {
    use crate::dex::PokemonType;
    use crate::effects::abilities::TypeChangeScope;
    use crate::effects::moves::OwnType;
    let mut modified = std::borrow::Cow::Borrowed(move_def);
    let Some(pokemon) = state.pokemon(user) else {
        return (modified, None);
    };
    match move_def.behavior.own_type {
        None => {}
        Some(OwnType::Weather) => {
            // Delta Stream's winds change nothing (Weather Ball's lists do not name them).
            let weather = weather_for(state, rules, user)
                .filter(|kind| *kind != crate::state::WeatherKind::StrongWinds);
            if let Some(weather) = weather {
                modified.to_mut().move_type = match weather {
                    crate::state::WeatherKind::Sun => PokemonType::Fire,
                    crate::state::WeatherKind::Rain => PokemonType::Water,
                    crate::state::WeatherKind::Sandstorm => PokemonType::Rock,
                    crate::state::WeatherKind::Snow | crate::state::WeatherKind::StrongWinds => {
                        PokemonType::Ice
                    }
                };
                modified.to_mut().base_power = modified.base_power.saturating_mul(2);
            }
        }
        Some(OwnType::UserFirstType) => {
            modified.to_mut().move_type = crate::engine::types_of(state, rules, pokemon).primary
        }
        Some(OwnType::UserSpecies(types)) => {
            if let Some((_, move_type)) = types
                .iter()
                .find(|(species, _)| species_of(pokemon) == Some(*species))
            {
                modified.to_mut().move_type = *move_type;
            }
        }
        Some(OwnType::HeldItemType(gift)) => {
            use crate::effects::moves::ItemGift;
            let gift_type = match gift {
                // Judgment (`onModifyType`): `if (item.id && item.onPlate && !item.zMove)` — a
                // Z-Crystal types Arceus (Multitype reads its `plate` like a Plate's) but never
                // the move itself.
                ItemGift::Plate => {
                    let z_crystal = crate::engine::held_item(state, rules, pokemon.id())
                        .is_some_and(|(item, _)| {
                            crate::engine::item_behavior(rules, item).take_refusal
                                == Some(crate::effects::items::TakeRefusal::ZCrystal)
                        });
                    (!z_crystal)
                        .then(|| crate::engine::plate_type(state, rules, pokemon))
                        .flatten()
                }
                // Multi-Attack's Memory event (or Techno Blast's Drive event): the held item's
                // own `onMemory` / `onDrive` (a suppressed item has none: `held_item`). The
                // event's 'Normal' default leaves the move's own type, which is Normal anyway.
                ItemGift::Memory => crate::engine::memory_type(state, rules, pokemon),
                ItemGift::Drive => crate::engine::drive_type(state, rules, pokemon),
            };
            if let Some(type_) = gift_type {
                modified.to_mut().move_type = type_;
            }
        }
        Some(OwnType::HiddenPower) => modified.to_mut().move_type = pokemon.hidden_power_of().0,
        Some(OwnType::Terrain) => {
            if let Some(terrain) = state.field.terrain.map(|t| t.kind)
                && environment::grounded(state, rules, pokemon)
            {
                modified.to_mut().move_type = terrain_type(terrain);
                modified.to_mut().base_power = modified.base_power.saturating_mul(2);
            }
        }
        Some(OwnType::NaturalGift) => {
            if let Some((_, gift)) = natural_gift_berry(state, rules, pokemon) {
                modified.to_mut().move_type = gift.move_type;
            }
        }
    }
    // Sky Drop (`onModifyMove`): the turn that carries the target up is sure to hit and makes
    // no contact.
    if move_def.behavior.sky_drop
        && !pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::SkyDrop)
    {
        modified.to_mut().accuracy = None;
        modified.to_mut().flags.contact = false;
    }
    // Curse (`onModifyMove`): a user that is not a Ghost type uses the other form, aimed where
    // `nonGhostTarget` says (itself).
    if let Some(pattern) = move_def.non_ghost_target
        && !crate::engine::has_type(state, rules, pokemon, PokemonType::Ghost)
    {
        modified.to_mut().target = pattern;
    }
    // Expanding Force: all adjacent foes, for a grounded user in its terrain.
    if let Some(kind) = move_def.behavior.terrain_spread
        && state.field.terrain.map(|t| t.kind) == Some(kind)
        && environment::grounded(state, rules, pokemon)
    {
        modified.to_mut().target = crate::rules::TargetPattern::AllAdjacentFoes;
        modified.to_mut().is_spread = true;
    }
    // A combined Water Pledge's side (`onModifyMove`): the side's moves double their
    // secondaries' chances (a Serene Grace flinch is left alone, DESIGN.md §42.9).
    if state
        .side(pokemon.side)
        .conditions
        .contains(crate::state::SideConditionKind::WaterPledge)
    {
        let serene =
            ability_handler(state, rules, pokemon, |b| b.secondary_chance_multiplier).is_some();
        for secondary in &mut modified.to_mut().secondaries {
            if serene
                && secondary.effect
                    == crate::dex::SecondaryEffect::Volatile(
                        crate::state::VolatileConditionKind::Flinch,
                    )
            {
                continue;
            }
            secondary.chance = secondary.chance.doubled();
        }
    }
    // Secret Power: the terrain's secondary in place of its own.
    if move_def.behavior.terrain_secondaries
        && let Some(terrain) = state.field.terrain.map(|t| t.kind)
    {
        use crate::dex::{SecondaryEffect, SecondaryEffectSpec, SecondaryTarget, SecondaryTrigger};
        let effect = match terrain {
            crate::state::TerrainKind::Electric => {
                SecondaryEffect::Status(crate::state::NonVolatileStatus::Paralysis)
            }
            crate::state::TerrainKind::Grassy => {
                SecondaryEffect::Status(crate::state::NonVolatileStatus::Sleep)
            }
            crate::state::TerrainKind::Misty => SecondaryEffect::StatStage {
                stat: crate::state::Stat::SpecialAttack,
                delta: -1,
            },
            crate::state::TerrainKind::Psychic => SecondaryEffect::StatStage {
                stat: crate::state::Stat::Speed,
                delta: -1,
            },
        };
        modified.to_mut().secondaries = vec![SecondaryEffectSpec {
            chance: crate::dex::EffectChance::percent(30),
            target: SecondaryTarget::Target,
            trigger: SecondaryTrigger::PerTargetHit,
            effect,
            requires: None,
            shares_roll: false,
            in_secondaries: true,
            from_item: false,
        }];
    }
    // Gen 2-4 Beat Up (its own `onModifyMove`): typeless, special until Gen 4 makes it
    // physical — a category it sets itself.
    let own_category = if move_def.behavior.beat_up {
        use crate::rules::BeatUpStyle;
        match rules.damage.beat_up {
            BeatUpStyle::Gen2 | BeatUpStyle::Gen3 => Some(crate::dex::MoveCategory::Special),
            BeatUpStyle::Gen4 => Some(crate::dex::MoveCategory::Physical),
            BeatUpStyle::Modern => None,
        }
    } else {
        None
    };
    if let Some(category) = own_category {
        modified.to_mut().move_type = PokemonType::Typeless;
        modified.to_mut().category = category;
    }
    // Gen 4 on the cartridge (issues.md I20): Beat Up's effect script sets
    // `SYSCTL_IGNORE_IMMUNITIES`, so no type immunity stops it — a Normalize user's Beat Up hits
    // a Ghost type.
    if move_def.behavior.beat_up && rules.damage.beat_up == crate::rules::BeatUpStyle::Gen4 {
        modified.to_mut().flags.type_immunity = false;
    }
    // Tera Blast and Tera Starstorm (`onModifyType` / `onModifyMove`, DESIGN.md §44.6): a
    // Terastallized Tera Blast takes the Tera type; a Terapagos-Stellar's Tera Starstorm is
    // Stellar and hits all adjacent foes. Both are physical when the user's Attack beats its
    // Special Attack at their stages without modifiers. Tera Blast re-reads its type below,
    // where a Normal-type ability would otherwise change it.
    // `onModifyType` / `onModifyMove`: the Stellar type and the all-adjacent-foes target belong
    // to Terapagos-Stellar itself; only the category needs it Terastallized.
    let stellar_species = move_def.behavior.tera_starstorm
        && species_of(pokemon) == Some(crate::ids::SpeciesId::TERAPAGOS_STELLAR);
    let stellar_user = stellar_species && pokemon.terastallized.is_some();
    if move_def.behavior.tera_blast {
        if let Some(tera) = pokemon.terastallized {
            modified.to_mut().move_type = tera;
        }
    } else if stellar_species {
        modified.to_mut().move_type = PokemonType::Stellar;
        modified.to_mut().target = crate::rules::TargetPattern::AllAdjacentFoes;
        modified.to_mut().is_spread = true;
    }
    if (move_def.behavior.tera_blast && pokemon.terastallized.is_some() || stellar_user)
        && stats::apply_stage(rules, pokemon.battle_stats().attack, pokemon.stages.attack)
            > stats::apply_stage(
                rules,
                pokemon.battle_stats().special_attack,
                pokemon.stages.special_attack,
            )
    {
        modified.to_mut().category = crate::dex::MoveCategory::Physical;
    }
    // Tera Blast's Stellar form: 100 power, and the user's Attack and Special Attack drop by
    // one once the move is over (the move data's `self`, added here as `onModifyMove` does).
    if move_def.behavior.tera_blast && pokemon.terastallized == Some(PokemonType::Stellar) {
        modified.to_mut().base_power = 100;
        for stat in [
            crate::state::Stat::Attack,
            crate::state::Stat::SpecialAttack,
        ] {
            modified
                .to_mut()
                .secondaries
                .push(crate::dex::SecondaryEffectSpec {
                    chance: crate::dex::EffectChance::percent(100),
                    target: crate::dex::SecondaryTarget::User,
                    trigger: crate::dex::SecondaryTrigger::OnceAfterMove,
                    effect: crate::dex::SecondaryEffect::StatStage { stat, delta: -1 },
                    requires: None,
                    shares_roll: false,
                    in_secondaries: false,
                    from_item: false,
                });
        }
    }
    // Photon Geyser (`onModifyMove`): physical when the user's Attack beats its Special Attack,
    // both at their stages and without modifiers (`getStat(.., false, true)`).
    if move_def.behavior.category_by_stats
        && stats::apply_stage(rules, pokemon.battle_stats().attack, pokemon.stages.attack)
            > stats::apply_stage(
                rules,
                pokemon.battle_stats().special_attack,
                pokemon.stages.special_attack,
            )
    {
        modified.to_mut().category = crate::dex::MoveCategory::Physical;
    }
    if physical {
        modified = std::borrow::Cow::Owned(made_physical(modified.into_owned()));
    }
    let mut changer = None;
    if let Some(change) = ability_handler(state, rules, pokemon, |b| b.modify_type) {
        let own_type = move_def
            .behavior
            .own_type
            .is_some_and(|own| own.spares_type_changers());
        // A Terastallized Tera Blast keeps its Tera type (every Normal-type ability's
        // `onModifyType` names it).
        let tera_blast_terastallized =
            move_def.behavior.tera_blast && pokemon.terastallized.is_some();
        // A Z-Move keeps its type (`!(move.isZ && move.category !== 'Status')`, every
        // type-changing ability's `onModifyType`, DESIGN.md §44.4).
        let z_move =
            move_def.z_crystal.is_some() && move_def.category != crate::dex::MoveCategory::Status;
        let applies = match change.scope {
            TypeChangeScope::NormalMoves => {
                modified.move_type == PokemonType::Normal
                    && !own_type
                    && !tera_blast_terastallized
                    && !z_move
            }
            // Liquid Voice (the only sound-scoped changer): a Dynamaxed holder's sound move keeps
            // its type (`!pokemon.volatiles['dynamax']` in its `onModifyType`, DESIGN.md §44.5) —
            // the Max Move it becomes follows the base move's own type.
            TypeChangeScope::SoundMoves => {
                move_def.flags.sound
                    && !pokemon
                        .volatiles
                        .contains(crate::state::VolatileConditionKind::Dynamax)
            }
            TypeChangeScope::AllMoves => {
                move_def.id != crate::ids::MoveId::STRUGGLE
                    && !(rules.damage.all_moves_type_change_spares_own_type && own_type)
                    && !tera_blast_terastallized
                    && !z_move
            }
        };
        if applies {
            modified.to_mut().move_type = change.to;
            if ability_handler(state, rules, pokemon, |b| b.type_change_boost) {
                changer = Some(raw_ability(pokemon));
            }
        }
    }
    // Ion Deluge (`onModifyType`, priority −2, after the ability's): a Normal move is Electric
    // for the rest of the turn.
    if modified.move_type == PokemonType::Normal
        && state
            .field
            .contains(crate::state::FieldConditionKind::IonDeluge)
    {
        modified.to_mut().move_type = PokemonType::Electric;
    }
    // Electrify (`onModifyType`, priority −2 as Ion Deluge's): the electrified Pokémon's move,
    // Struggle aside, is Electric.
    if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::Electrify)
        && move_def.id != crate::ids::MoveId::STRUGGLE
    {
        modified.to_mut().move_type = PokemonType::Electric;
    }
    if rules.damage.category_follows_type
        && own_category.is_none()
        && modified.category != crate::dex::MoveCategory::Status
        && modified.move_type != move_def.move_type
    {
        modified.to_mut().category = match modified.move_type {
            PokemonType::Fire
            | PokemonType::Water
            | PokemonType::Grass
            | PokemonType::Ice
            | PokemonType::Electric
            | PokemonType::Dark
            | PokemonType::Psychic
            | PokemonType::Dragon => crate::dex::MoveCategory::Special,
            _ => crate::dex::MoveCategory::Physical,
        };
    }
    // The user's ability editing the move (`onModifyMove` as data), on its final type.
    if let Some(edit) = ability_handler(state, rules, pokemon, |b| b.move_edit) {
        if edit.ignore_evasion {
            modified.to_mut().stat_sources.ignore_evasion = true;
        }
        if edit.normal_fighting_ignore_immunity
            && matches!(
                modified.move_type,
                PokemonType::Normal | PokemonType::Fighting
            )
        {
            modified.to_mut().flags.immunity_ignored = true;
        }
        if edit.no_contact {
            modified.to_mut().flags.contact = false;
        }
        if edit.tracks_target {
            modified.to_mut().tracks_target = true;
        }
        if edit.contact_ignores_protect && modified.flags.contact {
            modified.to_mut().flags.protectable = false;
        }
        if edit.max_hits {
            if let crate::dex::MultiHit::Range { max, .. } = modified.multi_hit {
                modified.to_mut().multi_hit = crate::dex::MultiHit::Fixed(max);
            }
            modified.to_mut().multi_accuracy = false;
        }
        // (`!move.hasSheerForceBoost`: a move Sheer Force only boosts keeps everything.) The
        // move's `self` goes with the secondaries, and so does its `selfBoost` (Showdown names
        // Clangorous Soulblaze, the one move with both `selfBoost` and secondaries; DESIGN.md
        // R-M8-84).
        if edit.sheer_force && modified.has_secondaries && !modified.flags.sheer_force_boost {
            modified.to_mut().secondaries.retain(|secondary| {
                !secondary.in_secondaries
                    && secondary.trigger == crate::dex::SecondaryTrigger::PerTargetHit
            });
            modified.to_mut().has_secondaries = false;
            modified.to_mut().flags.sheer_force = true;
        }
        if edit.ignores_abilities
            || (edit.status_ignores_abilities
                && modified.category == crate::dex::MoveCategory::Status)
        {
            modified.to_mut().ignores_ability = true;
        }
        if let Some(percent) = edit.flinch_chance
            && modified.category != crate::dex::MoveCategory::Status
            && !modified.secondaries.iter().any(|secondary| {
                secondary.effect
                    == crate::dex::SecondaryEffect::Volatile(
                        crate::state::VolatileConditionKind::Flinch,
                    )
            })
        {
            modified
                .to_mut()
                .secondaries
                .push(crate::dex::SecondaryEffectSpec {
                    chance: crate::dex::EffectChance::percent(percent),
                    target: crate::dex::SecondaryTarget::Target,
                    trigger: crate::dex::SecondaryTrigger::PerTargetHit,
                    effect: crate::dex::SecondaryEffect::Volatile(
                        crate::state::VolatileConditionKind::Flinch,
                    ),
                    requires: None,
                    shares_roll: false,
                    in_secondaries: true,
                    from_item: false,
                });
            modified.to_mut().has_secondaries = true;
        }
    }
    // The user's item editing the move (`onModifyMove` as data), after the ability: Loaded
    // Dice works on the hits Skill Link left (Showdown counts them in `hitStepMoveHitLoop`).
    if let Some(edit) = held_item(state, rules, user).and_then(|(_, item)| item.move_edit) {
        if edit.punch_no_contact && modified.flags.punch {
            modified.to_mut().flags.contact = false;
        }
        if edit.adds_flinch && modified.category != crate::dex::MoveCategory::Status {
            let flinch =
                crate::dex::SecondaryEffect::Volatile(crate::state::VolatileConditionKind::Flinch);
            let item_flinch = rules.hits.item_flinch;
            let adds = match item_flinch.moves {
                Some(moves) => moves.contains(&modified.id),
                None => !modified
                    .secondaries
                    .iter()
                    .any(|secondary| secondary.in_secondaries && secondary.effect == flinch),
            };
            if adds {
                modified
                    .to_mut()
                    .secondaries
                    .push(crate::dex::SecondaryEffectSpec {
                        chance: crate::dex::EffectChance::percent(item_flinch.chance_percent),
                        target: crate::dex::SecondaryTarget::Target,
                        trigger: crate::dex::SecondaryTrigger::PerTargetHit,
                        effect: flinch,
                        requires: None,
                        shares_roll: false,
                        in_secondaries: true,
                        from_item: true,
                    });
            }
        }
        if edit.loaded_dice {
            modified.to_mut().multi_accuracy = false;
            modified.to_mut().multi_hit = match modified.multi_hit {
                crate::dex::MultiHit::Range { min: 2, max: 5 } => {
                    crate::dex::MultiHit::Range { min: 4, max: 5 }
                }
                crate::dex::MultiHit::Fixed(10) => crate::dex::MultiHit::Range { min: 4, max: 10 },
                other => other,
            };
        }
    }
    // Infiltrator (`move.infiltrates`): from Gen 6 a Substitute lets the move through like a
    // `bypasssub` one (Showdown `substitute`'s `onTryPrimaryHit`).
    if ability_handler(state, rules, pokemon, |b| b.infiltrates)
        && rules.conditions.substitute_honors_infiltrates
    {
        modified.to_mut().flags.bypass_substitute = true;
    }
    (modified, changer)
}

/// `modified_move_cow`, owned: the move as its user's edits leave it. A caller that only reads
/// the result (the action's order) takes the `Cow` instead and copies nothing for a move no edit
/// touches (R-M10-10).
pub fn modified_move(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
    physical: bool,
) -> (crate::dex::ResolvedMove, Option<crate::ids::AbilityId>) {
    let (modified, changer) = modified_move_cow(state, rules, user, move_def, physical);
    (modified.into_owned(), changer)
}

/// A terrain's type (Terrain Pulse).
pub(crate) const fn terrain_type(terrain: crate::state::TerrainKind) -> crate::dex::PokemonType {
    match terrain {
        crate::state::TerrainKind::Electric => crate::dex::PokemonType::Electric,
        crate::state::TerrainKind::Grassy => crate::dex::PokemonType::Grass,
        crate::state::TerrainKind::Misty => crate::dex::PokemonType::Fairy,
        crate::state::TerrainKind::Psychic => crate::dex::PokemonType::Psychic,
    }
}

/// The first active Pokémon — side 1's positions, then side 2's — whose ability `pick`
/// accepts: the holder of an `onAny` handler (Showdown runs those from every active Pokémon).
pub(crate) fn active_holder(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pick: impl Fn(&crate::effects::abilities::AbilityBehavior) -> bool,
) -> Option<(crate::ids::PokemonId, crate::ids::AbilityId)> {
    [crate::ids::SideId::P1, crate::ids::SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().flatten().copied())
        .filter_map(|id| {
            state
                .pokemon(id)
                .filter(|pokemon| !pokemon.is_fainted())
                .and_then(|pokemon| ability_running(state, rules, pokemon))
                .map(|ability| (id, ability))
        })
        .find(|(_, ability)| pick(ability_behavior(rules, *ability)))
}

/// Beat Up's members, one hit each, in party order (Showdown `side.pokemon`): neither fainted nor
/// statused — from Gen 5 the user whatever its status. Shared with the replay.
pub fn beat_up_members(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
) -> Vec<crate::ids::PokemonId> {
    let Some(side) = state.pokemon(user).map(|pokemon| pokemon.side) else {
        return Vec::new();
    };
    // Gen 5+, and Gen 4 on the cartridge (`BeatUpEligibleMon`: the user's own slot counts
    // whatever its status, issues.md I20): the user always takes part.
    let user_always = matches!(
        rules.damage.beat_up,
        crate::rules::BeatUpStyle::Gen4 | crate::rules::BeatUpStyle::Modern
    );
    state
        .side(side)
        .battle_order
        .iter()
        .copied()
        .filter(|member| {
            (user_always && *member == user)
                || state
                    .pokemon(*member)
                    .is_some_and(|pokemon| !pokemon.is_fainted() && pokemon.status.is_none())
        })
        .collect()
}

/// Shell Side Arm (`onModifyMove`): whether its physical estimate against `target` beats the
/// special one, `None` on a tie (a coin decides). Each estimate is
/// `⌊⌊⌊⌊2L/5 + 2⌋ × power × attack⌋ / defense⌋ / 50⌋` with the stats' stages and none of their
/// modifiers (Showdown `getStat(stat, false, true)`). Shared with the replay.
pub fn physical_by_estimate(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> Option<bool> {
    let (Some(user), Some(foe)) = (state.pokemon(user), state.pokemon(target)) else {
        return Some(false);
    };
    let staged = |value, stage| stats::stage_value(rules.stats.stage_table, value, stage);
    let estimate = |attack: u32, defense: u32| {
        (2 * u32::from(user.level) / 5 + 2) * u32::from(move_def.base_power) * attack
            / defense.max(1)
            / 50
    };
    let physical = estimate(
        staged(user.battle_stats().attack, user.stages.attack),
        staged(foe.battle_stats().defense, foe.stages.defense),
    );
    let special = estimate(
        staged(
            user.battle_stats().special_attack,
            user.stages.special_attack,
        ),
        staged(
            foe.battle_stats().special_defense,
            foe.stages.special_defense,
        ),
    );
    (physical != special).then_some(physical > special)
}

/// The move made physical, with contact (Shell Side Arm's `onModifyMove`).
fn made_physical(mut move_def: crate::dex::ResolvedMove) -> crate::dex::ResolvedMove {
    move_def.category = crate::dex::MoveCategory::Physical;
    move_def.flags.contact = true;
    move_def
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

/// A timed volatile's `onStart` adjustment for `target` right now (Showdown: `activeTurns`,
/// `queue.willMove`). Shared by the condition setters and the replay driver, which subtracts it
/// from a boundary frame to recover the rolled duration.
pub fn duration_adjustment(
    state: &crate::state::BattleState,
    adjust: crate::rules::DurationAdjust,
    target: crate::ids::PokemonId,
) -> i16 {
    use crate::rules::DurationAdjust;
    let will_move = state.will_move(target);
    let settled = state.pokemon(target).is_some_and(|p| p.active_turns > 0);
    match adjust {
        DurationAdjust::None => 0,
        DurationAdjust::PlusIfActedAndSettled => i16::from(settled && !will_move),
        DurationAdjust::PlusIfWontMove => i16::from(!will_move),
        DurationAdjust::MinusIfWillMove => -i16::from(will_move),
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

/// The TryHit event of the target's own ability and item (Good as Gold, Lightning Rod …):
/// the reason one of them refuses `move_def` from `user`, if one does. The hit gate runs it
/// after the side's protections; a field move that runs TryHit per Pokémon (Perish Song's
/// `onHitField`) runs it alone.
pub(crate) fn holders_try_hit(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> Option<HitBlockReason> {
    let pokemon = state.pokemon(target)?;
    let mut ctx = TryHitContext::new(state, rules, user, target, move_def);
    if let TryHitResult::Blocked(reason) =
        crate::effects::abilities::try_hit(ability_handlers(state, rules, pokemon), &mut ctx)
    {
        return Some(reason);
    }
    let (_, item) = held_item(state, rules, target)?;
    let mut ctx = TryHitContext::new(state, rules, user, target, move_def);
    match crate::effects::items::try_hit(item, &mut ctx) {
        TryHitResult::Blocked(reason) => Some(reason),
        _ => None,
    }
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

/// Whether a frozen user of `move_def` rolls to thaw at BeforeMove: Gen 3+ do (Gen 2 in the
/// residual instead), except with a `defrost` move from Gen 5 (`defrost_move_for`). Shared
/// with the replay driver.
pub fn freeze_thaw_rolls(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    let lifecycle = rules.status.lifecycle;
    lifecycle.thaw_timing == crate::rules::ThawTiming::BeforeMove
        && !(defrost_move_for(state, rules, user, move_def)
            && lifecycle.defrost_user == crate::rules::DefrostUser::SkipsRoll)
}

/// Whether `move_def` is a `defrost` move for its frozen `user` (Showdown `frz.onBeforeMove`):
/// one that burns up its user's type (Burn Up) is not for a user without that type, which rolls
/// to thaw as for any other move (`!(move.id === 'burnup' && !pokemon.hasType('Fire'))`).
pub fn defrost_move_for(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    move_def.flags.defrost
        && move_def.behavior.burns_up.is_none_or(|type_| {
            state
                .pokemon(user)
                .is_some_and(|pokemon| types_of(state, rules, pokemon).contains(type_))
        })
}

/// Whether `move_def` thaws a frozen target it hits on the HP (DESIGN.md §22.6): Gen 3+ a
/// damaging Fire move or a `thawsTarget` one, Gen 1-2 one whose secondary is a burn.
pub fn thaws_target_on_hit(
    rules: &crate::rules::ResolvedRules,
    move_def: &crate::dex::ResolvedMove,
    picked: Option<crate::state::NonVolatileStatus>,
) -> bool {
    use crate::dex::{MoveCategory, SecondaryEffect};
    match rules.status.lifecycle.thaw_on_hit {
        crate::rules::ThawOnHit::FireDamage => {
            move_def.thaws_target
                || (move_def.move_type == crate::dex::PokemonType::Fire
                    && move_def.category != MoveCategory::Status)
        }
        // Gen 2 `frz.onAfterMoveSecondary`: a burn secondary, or Tri Attack's `statusRoll`
        // drawn as a burn (`picked`), whether or not the secondary landed.
        crate::rules::ThawOnHit::BurnSecondary => {
            matches!(
                move_def.secondaries.as_slice(),
                [secondary] if secondary.effect
                    == SecondaryEffect::Status(crate::state::NonVolatileStatus::Burn)
            ) || picked == Some(crate::state::NonVolatileStatus::Burn)
        }
    }
}

/// How a hit's critical-hit check goes (DESIGN.md §22.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriticalRoll {
    /// No roll, not critical (a ratio of 0; Gen 1 without a species).
    Never,
    /// No roll, critical (Showdown `willCrit`).
    Always,
    /// `numerator / denominator`.
    Chance(u16, u16),
}

/// The critical-hit check of `attacker`'s damaging move on `target`: the move's crit ratio plus
/// the attacker's item / ability boosts (`onModifyCritRatio`; Merciless makes it 5 against a
/// poisoned target), through the generation's table (`DamageRules::critical`); Gen 1 from the
/// species' base Speed. Moves with fixed damage or a one-hit KO never reach it. Shared by the
/// hit sequence and the replay driver.
pub fn critical_roll(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    dex: &crate::dex::ResolvedDex,
    attacker: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> CriticalRoll {
    use crate::rules::CriticalTable;
    match move_def.will_crit {
        Some(true) => return CriticalRoll::Always,
        Some(false) => return CriticalRoll::Never,
        None => {}
    }
    let Some(pokemon) = state.pokemon(attacker) else {
        return CriticalRoll::Never;
    };
    let item = held_item(state, rules, attacker).map(|(_, item)| item);
    let item_boost = item.map_or(0, |item| item.crit_ratio_boost);
    let ability_boost = ability_handler(state, rules, pokemon, |b| b.crit_ratio_boost);
    // A species' own item (Leek, Stick, Lucky Punch): +2, or in Gen 2 the whole ratio (the
    // gen2 mod's `return 3` at priority −1; pokecrystal jumps straight to the tally).
    let species_stages = item
        .and_then(|item| item.species_crit_boost)
        .and_then(|boost| boost.stages_for(item_species(rules, pokemon)));
    // The critical-hit volatiles' `onModifyCritRatio`: Focus Energy and Dragon Cheer add
    // stages, Laser Focus makes it 5 (certain).
    let focus_energy = pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::FocusEnergy);
    let volatile_stages = if focus_energy {
        rules.conditions.focus_energy_stages
    } else {
        0
    } + pokemon.volatiles.dragon_cheer_stages();
    let ratio = match species_stages {
        Some(stages) if rules.damage.species_crit_item_sets_stage => 1 + stages,
        _ => {
            move_def.crit_ratio
                + item_boost
                + ability_boost
                + species_stages.unwrap_or(0)
                + volatile_stages
        }
    };
    let ratio = if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::LaserFocus)
    {
        ratio.max(5)
    } else {
        ratio
    };
    let poisoned = state.pokemon(target).is_some_and(|foe| {
        foe.has_status(crate::state::NonVolatileStatus::Poison)
            || foe.has_status(crate::state::NonVolatileStatus::Toxic)
    });
    let ratio = if poisoned && ability_handler(state, rules, pokemon, |b| b.crit_against_poisoned) {
        ratio.max(5)
    } else {
        ratio
    };
    let chance = match rules.damage.critical {
        CriticalTable::Gen1BaseSpeed => pokemon
            .species
            .and_then(|species| dex.species_by_id(species))
            .map(|species| {
                CriticalTable::gen1_chance(species.base_stats.speed, ratio, focus_energy)
            }),
        table => table.chance(ratio),
    };
    match chance {
        None | Some((0, _)) => CriticalRoll::Never,
        Some((numerator, denominator)) if numerator >= denominator => CriticalRoll::Always,
        Some((numerator, denominator)) => CriticalRoll::Chance(numerator, denominator),
    }
}

/// How a secondary effect is decided for `actor`'s move on `affected` right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryRoll {
    /// Nothing is rolled; the effect is attempted.
    Certain,
    /// Nothing is rolled and nothing happens (Gen 2: a burn / freeze against a target of the
    /// move's type, a flinch against a sleeping or frozen target).
    Skipped,
    /// `numerator / denominator`.
    Chance(u16, u16),
}

/// The speed simultaneous entrants run their switch-ins by, fastest first (`speed_ordered`).
/// Shared with the replay.
pub fn entry_speed(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: crate::ids::PokemonId,
) -> u32 {
    stats::action_speed(state, rules, pokemon, None)
}

/// The speed the residual orders a Pokémon's handlers by (Showdown `updateSpeed` as the residual
/// starts: `getActionSpeed`, inverted under Trick Room). Shared with the replay.
pub fn residual_speed(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: crate::ids::PokemonId,
) -> u32 {
    stats::current_handler_speed(state, rules, pokemon)
}

/// Whether hit `hit` of `hits` (the count the sequence was decided to have) leaves the
/// secondary out: Gen 1 rolls a multi-hit move's secondaries on its last hit only
/// (`HitRules::secondaries_on_last_hit`); Parental Bond with Secret Power (its
/// `onSourceModifySecondaries`) keeps the first hit to a flinch, so Secret Power's own secondary
/// rolls on the second hit alone. Shared with the replay.
pub fn secondary_withheld(
    rules: &crate::rules::ResolvedRules,
    move_def: &crate::dex::ResolvedMove,
    hit: u8,
    hits: u8,
    secondary: &crate::dex::SecondaryEffectSpec,
) -> bool {
    (rules.hits.secondaries_on_last_hit && hit < hits)
        || (move_def.flags.parental_bond
            && move_def.behavior.terrain_secondaries
            && hit < 2
            && secondary.in_secondaries
            && secondary.effect
                != crate::dex::SecondaryEffect::Volatile(
                    crate::state::VolatileConditionKind::Flinch,
                ))
}

/// The secondary's decision (DESIGN.md §31.2): Gen 3+ its percent after the user's ability
/// (Serene Grace), certain at 100 %; Gen 1-2 a byte out of 256 for any declared chance, after
/// Gen 2's pre-roll checks. Shared by the hit sequence and the replay driver.
pub fn secondary_roll(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: crate::ids::PokemonId,
    affected: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
    secondary: &crate::dex::SecondaryEffectSpec,
) -> SecondaryRoll {
    use crate::dex::SecondaryEffect;
    use crate::state::{NonVolatileStatus, VolatileConditionKind};
    // The secondary's own condition (Burning Jealousy, Gen 4-5 Chatter): unmet, nothing rolls.
    let unmet = match secondary.requires {
        None => false,
        Some(crate::dex::SecondaryRequirement::TargetStatsRaisedThisTurn) => state
            .pokemon(affected)
            .is_none_or(|target| !target.history.stats_raised_this_turn),
        Some(crate::dex::SecondaryRequirement::UserSpecies(species)) => state
            .pokemon(actor)
            .is_none_or(|user| species_of(user) != Some(species)),
    };
    if unmet {
        return SecondaryRoll::Skipped;
    }
    // Shield Dust / Covert Cloak (`onModifySecondaries`) drop a target-directed secondary before
    // its roll; a user-directed one stays, and so does a binding move's own trap.
    if secondary.in_secondaries
        && secondary.target == crate::dex::SecondaryTarget::Target
        && shields_secondaries(state, rules, affected)
    {
        return SecondaryRoll::Skipped;
    }
    if let Some(target) = state.pokemon(affected) {
        let skipped = match secondary.effect {
            SecondaryEffect::Status(status) => {
                rules.hits.secondary_same_type_immunity.contains(&status)
                    && crate::engine::types_of(state, rules, target).contains(move_def.move_type)
            }
            SecondaryEffect::Volatile(VolatileConditionKind::Flinch) => {
                rules.hits.flinch_needs_awake_target
                    && !secondary.from_item
                    && (target.has_status(NonVolatileStatus::Sleep)
                        || target.has_status(NonVolatileStatus::Freeze))
            }
            _ => false,
        };
        if skipped {
            return SecondaryRoll::Skipped;
        }
    }
    match rules.hits.chance_scale {
        crate::rules::ChanceScale::Percent => {
            // An item's flinch (King's Rock) escapes Serene Grace where the generation rolls it
            // apart from the move's secondaries (`ItemFlinchRules`).
            let chance = if secondary.from_item && !rules.hits.item_flinch.doubled_by_serene_grace {
                secondary.chance.basis_points().min(10_000)
            } else {
                effective_secondary_chance(state, rules, actor, secondary.chance.basis_points())
            };
            if chance >= 10_000 {
                SecondaryRoll::Certain
            } else {
                SecondaryRoll::Chance(chance, 10_000)
            }
        }
        crate::rules::ChanceScale::Byte => match secondary.chance.0 {
            None => SecondaryRoll::Certain,
            Some(basis_points) => {
                SecondaryRoll::Chance((u32::from(basis_points) * 255 / 10_000) as u16, 256)
            }
        },
        // Showdown gen1 `moveHit`: `⌈chance × 256 / 100⌉`, one less for confusion.
        crate::rules::ChanceScale::ByteAlwaysRolls => match secondary.chance.0 {
            None => SecondaryRoll::Certain,
            Some(basis_points) => {
                let mut numerator = (u32::from(basis_points) * 256).div_ceil(10_000);
                if secondary.effect == SecondaryEffect::Volatile(VolatileConditionKind::Confusion) {
                    numerator -= 1;
                }
                if numerator >= 256 {
                    SecondaryRoll::Certain
                } else {
                    SecondaryRoll::Chance(numerator as u16, 256)
                }
            }
        },
    }
}

/// A secondary's chance in basis points after the user's ability (Serene Grace doubles it),
/// capped at certain; shared by the hit sequence and the replay driver.
pub fn effective_secondary_chance(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: crate::ids::PokemonId,
    base_basis_points: u16,
) -> u16 {
    let multiplier = state
        .pokemon(actor)
        .and_then(|p| ability_handler(state, rules, p, |b| b.secondary_chance_multiplier))
        .unwrap_or(1);
    base_basis_points.saturating_mul(multiplier).min(10_000)
}
