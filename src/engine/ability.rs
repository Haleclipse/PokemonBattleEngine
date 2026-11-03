//! The ability a Pokémon has and the one in effect (DESIGN.md §41.1, K.0): Showdown reads an
//! ability three ways, and every engine read goes through one of them —
//!
//! - what the Pokémon has (`getAbility`): `raw_ability`, for the effects that ask about the
//!   ability itself, suppressed or not;
//! - whether it counts (`hasAbility`, after `ignoringAbility`): `ability_active`;
//! - whether its handlers run (`runEvent`): `ability_running`, its behavior `ability_handlers`.
//!
//! Asking about one ability or one handler, `has_ability`, `ability_handler` and
//! `active_ability_handler` look at the ability first and at its suppression only when that
//! matters, as Showdown's events do (R-M10-10): the same answers, the suppression mostly unasked.
//!
//! The suppression is Showdown's `ignoringAbility` (§41.2), step by step: from Gen 5 a Pokémon
//! off the field has no ability in effect (`ConditionRules::bench_abilities_ignored`); an
//! ability that cannot be suppressed (`cantsuppress`) always counts; Gastro Acid sets it aside;
//! Ability Shield, or being a Neutralizing Gas holder, keeps it; otherwise a Neutralizing Gas
//! holder on the field sets it aside. Besides, while a move that ignores abilities is used
//! (`suppressing_ability`: Mold Breaker, Sunsteel Strike …), `ability_handlers` leaves out the
//! other Pokémon's breakable abilities — `ability_active` does not: an explicit check of one
//! (Levitate in `grounded`) asks `suppressing_ability` itself, as Showdown's does.

use crate::effects::abilities::AbilityBehavior;
use crate::ids::AbilityId;
use crate::rules::ResolvedRules;
use crate::state::{BattleState, PokemonState};

/// Showdown `getAbility`: the ability `pokemon` has, in effect or not.
pub fn raw_ability(pokemon: &PokemonState) -> AbilityId {
    pokemon.battle_ability()
}

/// Core Enforcer's `onHit` (Gen 7-8, `BattleContext::core_enforcer`): whether it suppresses
/// `target`'s ability — one that can be suppressed, on a Pokémon that has already moved this
/// turn (not newly switched in, no move still to come). The move's effects run before the
/// hit's `DamagingHit` (`runMoveEffects`), so the replay plans no reaction of an ability the
/// hit silences first.
pub fn core_enforcer_suppresses(
    state: &BattleState,
    rules: &ResolvedRules,
    target: crate::ids::PokemonId,
) -> bool {
    state.pokemon(target).is_some_and(|pokemon| {
        !rules
            .conditions
            .cant_suppress
            .contains(&raw_ability(pokemon))
            && !pokemon.history.newly_switched
            && !state.will_move(target)
    })
}

/// Showdown `ignoringAbility`: whether `pokemon`'s ability has no effect right now.
pub fn ignoring_ability(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> bool {
    if rules.conditions.bench_abilities_ignored
        && state
            .active_position(pokemon.id(), &rules.topology)
            .is_none()
    {
        return true;
    }
    // Certain abilities have no effect on a Pokémon that transformed, even one that cannot be
    // suppressed (DESIGN.md §43.3).
    let ability = raw_ability(pokemon);
    if pokemon.transform.is_some() && rules.conditions.no_transform.contains(&ability) {
        return true;
    }
    if rules.conditions.cant_suppress.contains(&ability) {
        return false;
    }
    if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::GastroAcid)
    {
        return true;
    }
    if super::has_item(state, rules, pokemon, crate::ids::ItemId::ABILITY_SHIELD)
        || neutralizes(rules, pokemon)
    {
        return false;
    }
    // A Neutralizing Gas holder on the field (Showdown asks its `ability` itself, not
    // `hasAbility`: no recursion) that has not transformed, that Gastro Acid does not hold and
    // whose `End` has not run.
    [crate::ids::SideId::P1, crate::ids::SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().flatten())
        .filter_map(|id| state.pokemon(*id))
        .any(|holder| {
            neutralizes(rules, holder)
                && holder.transform.is_none()
                // A Pokémon in its ally's mouth keeps its ability (Showdown's
                // `!this.volatiles['commanding']`, DESIGN.md §43.4).
                && !holder
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::Commanding)
                && !holder
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::GastroAcid)
                && !holder.ability_state.ending
        })
}

/// Whether `pokemon`'s ability is Neutralizing Gas (`AbilityBehavior::neutralizing_gas`).
fn neutralizes(rules: &ResolvedRules, pokemon: &PokemonState) -> bool {
    super::ability_behavior(rules, raw_ability(pokemon)).neutralizing_gas
}

/// Showdown `hasAbility`: the ability in effect, if any.
pub fn ability_active(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> Option<AbilityId> {
    (!ignoring_ability(state, rules, pokemon)).then_some(raw_ability(pokemon))
}

/// Showdown `suppressingAbility(target)`: a move that ignores abilities is being used, by a
/// Pokémon still on the field that is not `pokemon` (Gen 8+; before, its own too), and
/// `pokemon` does not hold Ability Shield.
pub fn suppressing_ability(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> bool {
    state.active_move.is_some_and(|active| {
        active.ignores_ability
            && state
                .active_position(active.user, &rules.topology)
                .is_some()
            && (active.user != pokemon.id() || !rules.conditions.mold_breaker_spares_user)
            && !super::has_item(state, rules, pokemon, crate::ids::ItemId::ABILITY_SHIELD)
    })
}

/// The ability whose handlers Showdown's events run for `pokemon` (`runEvent`): the one in
/// effect, unless it is breakable and a move ignores it (`suppressing_ability`).
pub fn ability_running(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> Option<AbilityId> {
    ability_active(state, rules, pokemon).filter(|ability| {
        !(rules.conditions.breakable.contains(ability)
            && suppressing_ability(state, rules, pokemon))
    })
}

/// Showdown Dancer's `dancersTarget` (`!target.isAlly(dancer) && pokemon.isAlly(dancer) ? target :
/// pokemon`): the move's target when it is a foe of the dancer and the user its ally, the user
/// otherwise. Public: the replay reads it for a dance BeforeMove stopped, which prints no move
/// line (R-M9-47).
pub fn dancer_target(
    state: &BattleState,
    actor: crate::ids::PokemonId,
    dancer: crate::ids::PokemonId,
    original: Option<crate::ids::PokemonId>,
) -> crate::ids::PokemonId {
    let side = |id| state.pokemon(id).map(|pokemon| pokemon.side);
    match original {
        Some(target)
            if side(target).is_some()
                && side(target) != side(dancer)
                && side(actor) == side(dancer) =>
        {
            target
        }
        _ => actor,
    }
}

/// Poison Puppeteer as the battle stands (`status_tricks::poison_puppeteer`): `holder` is a
/// Pecharunt whose ability runs, so a Pokémon its move poisons is confused too. Public: the
/// replay driver plans that confusion's duration only then (DESIGN.md R-M9-35).
pub fn poison_puppeteer_confuses(
    state: &BattleState,
    rules: &ResolvedRules,
    holder: crate::ids::PokemonId,
) -> bool {
    state.pokemon(holder).is_some_and(|pokemon| {
        ability_running(state, rules, pokemon) == Some(AbilityId::POISON_PUPPETEER)
            && pokemon.species == Some(crate::ids::SpeciesId::PECHARUNT)
    })
}

/// Whether `pokemon`'s Gluttony makes its pinch berries go at half its HP: the ability runs and
/// its flag is up (Showdown `hasAbility('gluttony') && abilityState.gluttony`) — a Neutralizing
/// Gas ending takes it down until the holder takes damage (`AbilityState::gluttony_waits`).
pub fn gluttony(state: &BattleState, rules: &ResolvedRules, pokemon: &PokemonState) -> bool {
    ability_handler(state, rules, pokemon, |b| b.gluttony) && !pokemon.ability_state.gluttony_waits
}

/// The handlers of `ability_running`: its behavior, none's when no ability runs. One handler is
/// `ability_handler`'s to find.
pub fn ability_handlers(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> &'static AbilityBehavior {
    match ability_running(state, rules, pokemon) {
        Some(ability) => super::ability_behavior(rules, ability),
        None => no_ability(rules),
    }
}

/// The behavior of no ability: no handler at all.
fn no_ability(rules: &ResolvedRules) -> &'static AbilityBehavior {
    crate::effects::BehaviorRegistry::for_generation(rules.generation).no_ability()
}

/// What a behavior's handler reads as (`ability_handler`): a flag, an `Option`, a number or a
/// list — `false`, `None`, 0 or nothing for a behavior without it.
pub trait Handler {
    fn present(&self) -> bool;
}

impl Handler for bool {
    fn present(&self) -> bool {
        *self
    }
}

impl<T> Handler for Option<T> {
    fn present(&self) -> bool {
        self.is_some()
    }
}

impl Handler for u8 {
    fn present(&self) -> bool {
        *self != 0
    }
}

impl Handler for i8 {
    fn present(&self) -> bool {
        *self != 0
    }
}

impl<T> Handler for &[T] {
    fn present(&self) -> bool {
        !self.is_empty()
    }
}

/// One handler of `ability_handlers`, looked up the way Showdown's events look one up: the
/// ability's own behavior is asked for it first (`getCallback`), whether the ability runs only
/// when it has it (`runEvent`'s `ignoringAbility` and `suppressingAbility`) — for an event,
/// most abilities have none, and their suppression goes unasked (R-M10-10). `handler` reads the
/// handler off a behavior and finds none on the behavior of no ability (checked in debug
/// builds), so the answer is `handler(ability_handlers(…))`.
pub fn ability_handler<T: Handler>(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    handler: impl Fn(&'static AbilityBehavior) -> T,
) -> T {
    handler_first(rules, pokemon, handler, || {
        ability_running(state, rules, pokemon).is_some()
    })
}

/// `ability_handler` for the ability in effect (`ability_active`): a behavior that an explicit
/// check of the ability reads (Klutz's `hasAbility`), a move that ignores abilities or not.
pub fn active_ability_handler<T: Handler>(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    handler: impl Fn(&'static AbilityBehavior) -> T,
) -> T {
    handler_first(rules, pokemon, handler, || {
        !ignoring_ability(state, rules, pokemon)
    })
}

/// `handler` of `pokemon`'s ability when `counts`, of no ability otherwise — `counts` asked only
/// when the ability has the handler.
fn handler_first<T: Handler>(
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    handler: impl Fn(&'static AbilityBehavior) -> T,
    counts: impl FnOnce() -> bool,
) -> T {
    debug_assert!(
        !handler(no_ability(rules)).present(),
        "a handler the behavior of no ability has"
    );
    let found = handler(super::ability_behavior(rules, raw_ability(pokemon)));
    if found.present() && !counts() {
        handler(no_ability(rules))
    } else {
        found
    }
}

/// Showdown `hasAbility(ability)`: `pokemon` has `ability`, in effect. The ability is compared
/// first, so whether it counts is asked only of a Pokémon that has it (R-M10-10).
pub fn has_ability(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    ability: AbilityId,
) -> bool {
    raw_ability(pokemon) == ability && !ignoring_ability(state, rules, pokemon)
}

/// Trace's candidates (Showdown `onUpdate`: `adjacentFoes()` that are not `notrace`; the Gen 3-4
/// cartridges, with their oracle patches, the same foes): the foes next to `holder` with HP left
/// whose ability Trace may copy, in position order. Shared by the engine and the replay driver.
pub fn trace_candidates(
    state: &BattleState,
    rules: &ResolvedRules,
    holder: crate::ids::PokemonId,
) -> Vec<crate::ids::PokemonId> {
    let Some(position) = state.active_position(holder, &rules.topology) else {
        return Vec::new();
    };
    rules
        .topology
        .adjacent_foes(position)
        .into_iter()
        .filter_map(|position| state.pokemon_at_position(position, &rules.topology))
        .filter(|foe| {
            state.pokemon(*foe).is_some_and(|pokemon| {
                !pokemon.is_fainted() && !rules.conditions.no_trace.contains(&raw_ability(pokemon))
            })
        })
        .collect()
}
