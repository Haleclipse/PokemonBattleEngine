//! An ability changing in battle (DESIGN.md §41.3, K.3): Showdown's `setAbility` and
//! `battle.skillSwap`, and the start of a Pokémon's ability — on the field coming in, as
//! Neutralizing Gas clears, or once it has a new one, as the generation has it
//! (`AbilityChangeRules::start_new`).

use super::{AbilityEndCause, BattleContext, TraceEvent};
use crate::ids::{AbilityId, PokemonId};
use crate::rules::StartNewAbility;
use crate::state::VolatileConditionKind;

impl BattleContext<'_> {
    /// The start of `pokemon`'s ability as it comes in (Showdown `runSwitch`'s `SwitchIn` event):
    /// a breakable ability stays silent in a move that ignores abilities (`ability_running`: a
    /// Pokémon dragged in by a Mold Breaker Roar).
    pub fn switch_in_ability(&mut self, pokemon: PokemonId) {
        let ability = self
            .state
            .pokemon(pokemon)
            .and_then(|holder| crate::engine::ability_running(self.state, self.rules, holder));
        self.run_start(pokemon, ability);
        if let Some(entry) =
            ability.and_then(|ability| crate::engine::ability_behavior(self.rules, ability).entry)
        {
            entry(self, pokemon);
        }
    }

    /// The start of `pokemon`'s ability once it has a new one, or as Neutralizing Gas clears
    /// (Showdown `singleEvent('Start', getAbility())`: no breakable check, the ability in effect).
    pub fn start_ability(&mut self, pokemon: PokemonId) {
        let ability = self
            .state
            .pokemon(pokemon)
            .and_then(|holder| crate::engine::ability_active(self.state, self.rules, holder));
        self.run_start(pokemon, ability);
    }

    /// `ability`'s switch-in handler, the switch-in effect it announces noted for the stint
    /// (Gen 4's announced flags).
    fn run_start(&mut self, pokemon: PokemonId, ability: Option<AbilityId>) {
        let Some(ability) = ability else {
            return;
        };
        let behavior = crate::engine::ability_behavior(self.rules, ability);
        // Gorilla Tactics (`onStart`): no lock yet — after Neutralizing Gas, a lock taken before it
        // came holds nothing.
        if behavior.choice_lock
            && let Some(holder) = self.state.pokemon_mut(pokemon)
        {
            holder.ability_state.choice_lock = None;
        }
        if let Some(kind) = behavior.announces
            && let Some(holder) = self.state.pokemon_mut(pokemon)
            && !holder.history.announced.contains(&kind)
        {
            holder.history.announced.push(kind);
        }
        crate::effects::abilities::switch_in(self, behavior, pokemon);
    }

    /// Showdown `setAbility(ability, source)`: `holder`'s ability becomes `ability`. The old one
    /// ends; the new one starts as the generation has it. Returns the old ability, or `None` when
    /// the holder has no HP left (a Mummy hit's attacker Rocky Helmet knocked out), the new
    /// ability cannot be suppressed (Gen 9 Receiver taking Gulp Missile: `noreceiver` misses
    /// it), or the `SetAbility` event refused the change (Ability Shield, Gen 4 Griseous Orb).
    /// The old ability's `cantsuppress` check is left to the callers, which make it first (Role
    /// Play's `onTryHit` …). Its line (`-ability|holder|new|old|[from] …|[of] source`) names
    /// `source` where the effect has one (`AbilityChangeLine::Named`).
    /// `by`: the ability that makes the change and its holder (Showdown's `sourceEffect` an
    /// Ability: Mummy's, Lingering Aroma's), which a refusing Ability Shield names.
    pub fn set_ability(
        &mut self,
        holder: PokemonId,
        ability: AbilityId,
        source: Option<PokemonId>,
        by: Option<(PokemonId, AbilityId)>,
    ) -> Option<AbilityId> {
        let pokemon = self.state.pokemon(holder)?;
        if pokemon.hp == 0 || self.rules.conditions.cant_suppress.contains(&ability) {
            return None;
        }
        let old = crate::engine::raw_ability(pokemon);
        if self.refuses_ability_change(holder, by) {
            return None;
        }
        self.end_ability(holder, AbilityEndCause::Replaced);
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.change_ability(ability);
        }
        self.state.restart_ability_state(holder);
        self.trace.push(TraceEvent::AbilityChanged {
            pokemon: holder,
            from: old,
            to: ability,
            line: crate::engine::AbilityChangeLine::Named { source },
        });
        if self.rules.conditions.ability_change.start_new == StartNewAbility::AtOnce {
            self.start_ability(holder);
        }
        Some(old)
    }

    /// Showdown `battle.skillSwap(source, target)`: the two trade abilities. False when either
    /// has an ability that cannot be swapped, when Gen 5's two are the same, when either side is
    /// Dynamaxed (`source.volatiles['dynamax'] || target.volatiles['dynamax']`, DESIGN.md
    /// §44.5), or when an item refuses it (`SetAbility`, the target's first: Ability Shield,
    /// Gen 4 Griseous Orb). (Its fainted check is the move's: a fainted Pokémon is no target.)
    pub fn skill_swap(&mut self, source: PokemonId, target: PokemonId) -> bool {
        let (Some(user), Some(foe)) = (self.state.pokemon(source), self.state.pokemon(target))
        else {
            return false;
        };
        let dynamax = VolatileConditionKind::Dynamax;
        if user.volatiles.contains(dynamax) || foe.volatiles.contains(dynamax) {
            return false;
        }
        let (mine, theirs) = (
            crate::engine::raw_ability(user),
            crate::engine::raw_ability(foe),
        );
        let foes = user.side != foe.side;
        let rules = self.rules.conditions;
        if rules.fail_skill_swap.contains(&mine) || rules.fail_skill_swap.contains(&theirs) {
            return false;
        }
        if rules.ability_change.skill_swap_same_fails && mine == theirs {
            return false;
        }
        if self.refuses_ability_change(target, None) || self.refuses_ability_change(source, None) {
            return false;
        }
        self.end_ability(source, AbilityEndCause::Replaced);
        self.end_ability(target, AbilityEndCause::Replaced);
        for (pokemon, ability) in [(source, theirs), (target, mine)] {
            if let Some(holder) = self.state.pokemon_mut(pokemon) {
                holder.change_ability(ability);
            }
            self.state.restart_ability_state(pokemon);
        }
        // Gen 5+ between foes the line names the two (`battle.skillSwap`).
        let named = (self.rules.generation >= crate::spec::Generation::Gen5 && foes)
            .then_some((mine, theirs));
        self.trace.push(TraceEvent::AbilitiesSwapped {
            source,
            target,
            named,
        });
        // `source.volatileStaleness = undefined`; a foe's target is stale while it stays in
        // (DESIGN.md §45.7).
        if let Some(user) = self.state.pokemon_mut(source) {
            user.volatile_staleness = None;
        }
        self.make_volatile_stale(target, source);
        if rules.ability_change.start_new == StartNewAbility::AtOnce {
            self.start_ability(target);
            self.start_ability(source);
        }
        true
    }

    /// Showdown's `SetAbility` event: the item that refuses an ability change (`onSetAbility`:
    /// Ability Shield, Gen 4 Griseous Orb). Ability Shield says so (`-block|p|item: Ability
    /// Shield`), after the ability making the change where one does (`-ability|holder|Mummy`;
    /// Trace's not, R-M10-46); Griseous Orb's `onSetAbility: false` says nothing.
    fn refuses_ability_change(
        &mut self,
        pokemon: PokemonId,
        by: Option<(PokemonId, AbilityId)>,
    ) -> bool {
        // Gen 4's own Multitype, whose `onSetAbility: false` refuses every change (DESIGN.md
        // §43.4).
        if self.state.pokemon(pokemon).is_some_and(|holder| {
            crate::engine::ability_behavior(self.rules, crate::engine::raw_ability(holder))
                .refuses_ability_change
        }) {
            return true;
        }
        let Some((item, _)) = crate::engine::held_item(self.state, self.rules, pokemon)
            .filter(|(_, behavior)| behavior.refuses_ability_change)
        else {
            return false;
        };
        if item == crate::ids::ItemId::ABILITY_SHIELD {
            if let Some((holder, ability)) = by {
                self.ability_activated(holder, ability);
            }
            self.item_activated(pokemon, item);
        }
        true
    }

    /// Gen 4's switch-in check at a move's end (pokeplatinum `BattleSystem_TriggerEffectOnSwitch`):
    /// fastest first, each Pokémon whose ability's kind of switch-in effect has not been
    /// announced this stint (it gained the ability since it came in) starts it.
    pub fn announce_unannounced(&mut self) {
        let mut active: Vec<(PokemonId, u32)> = self
            .active_pokemon()
            .into_iter()
            .map(|id| {
                (
                    id,
                    crate::engine::stats::action_speed(self.state, self.rules, id, None),
                )
            })
            .collect();
        active.sort_by_key(|(_, speed)| std::cmp::Reverse(*speed));
        for (pokemon, _) in active {
            self.announce_if_unannounced(pokemon);
        }
    }

    /// One Pokémon's part of the check: its ability starts if it is of a kind of switch-in
    /// effect it has not announced this stint.
    pub(crate) fn announce_if_unannounced(&mut self, pokemon: PokemonId) {
        let unannounced = self.state.pokemon(pokemon).is_some_and(|holder| {
            !holder.is_fainted()
                && crate::engine::active_ability_handler(self.state, self.rules, holder, |b| {
                    b.announces
                })
                .is_some_and(|kind| !holder.history.announced.contains(&kind))
        });
        if unannounced {
            self.start_ability(pokemon);
        }
    }
}
