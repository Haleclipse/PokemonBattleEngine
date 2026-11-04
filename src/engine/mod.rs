mod ability;
mod accuracy;
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
mod move_checks;
mod move_pipeline;
mod move_prep;
mod mutation;
mod power;
mod protections;
mod reactions;
mod rolls;
mod screens;
mod special_damage;
mod stats;
pub use stats::effective_speed;
mod status;
mod trace;
pub mod two_turn;
mod typing;
mod warnings;
mod weather;

pub use crate::state::Stat;
pub use accuracy::*;
pub use move_checks::*;
pub use move_prep::*;
pub use rolls::*;
pub use special_damage::*;
mod calls;
pub mod pledges;
pub use calls::{
    assist_candidates, blocked_by_the_user, called_move_aims_at_random, copycat_move,
    metronome_candidates, mirror_move_from, nature_power_move, sleep_talk_candidates,
};

pub use ability::{
    Handler, ability_active, ability_handler, ability_handlers, ability_running,
    active_ability_handler, core_enforcer_suppresses, dancer_target, gluttony, has_ability,
    ignoring_ability, poison_puppeteer_confuses, raw_ability, suppressing_ability,
    trace_candidates,
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
        crate::rules::ThawOnHit::FireDamageButHiddenPower => {
            move_def.move_type == crate::dex::PokemonType::Fire
                && move_def.category != MoveCategory::Status
                && move_def.id != crate::ids::MoveId::HIDDEN_POWER
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
