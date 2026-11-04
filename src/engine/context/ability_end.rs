//! An ability's end (DESIGN.md §41.2, K.1): Showdown `singleEvent('End', pokemon.getAbility())`,
//! the one `onEnd` every way an ability stops counting goes through — the Pokémon leaves the
//! field, faints, or has its ability suppressed while it stays. `End` is the one event
//! `ignoringAbility` does not stop, so it reads the ability the Pokémon has, suppressed or not.

use super::BattleContext;
use crate::ids::PokemonId;
use crate::state::VolatileConditionKind;

/// Why an ability ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityEndCause {
    /// Switched, dragged or passed out.
    Leaving,
    /// Fainted: the weather it suppressed is looked at again once the faints are processed
    /// (`BattleEngine::resolve_faints`), not here.
    Fainted,
    /// Suppressed on the field (Gastro Acid).
    Suppressed,
    /// Replaced by another while on the field (Skill Swap, Role Play …).
    Replaced,
}

impl BattleContext<'_> {
    /// The `onEnd` of `pokemon`'s ability.
    pub fn end_ability(&mut self, pokemon: PokemonId, cause: AbilityEndCause) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        let behavior =
            crate::engine::ability_behavior(self.rules, crate::engine::raw_ability(holder));
        let transformed = holder.transform.is_some();
        // Air Lock / Cloud Nine (`abilityState.ending`, `WeatherChange`): the weather counts
        // again.
        if behavior.suppresses_weather && cause != AbilityEndCause::Fainted {
            self.weather_changed_leaving(Some(pokemon));
        }
        // Desolate Land / Primordial Sea / Delta Stream (§23.5): the weather passes on or ends.
        if behavior.strong_weather.is_some() {
            self.strong_weather_holder_left(pokemon);
        }
        // A holder that Transformed has its gas end silently: the others' abilities do not start
        // again (Showdown's `onEnd` returns for `source.transformed`, which `transformInto` set
        // before `setAbility`; no cartridge lets the Koffing line use Transform, R-M8-123).
        if behavior.neutralizing_gas && !transformed {
            self.neutralizing_gas_ended(pokemon);
        }
        // Illusion's `onEnd`: suppressed or replaced, the holder is seen as itself; fainting
        // (`onFaint`), the disguise just goes. Leaving (`beingCalledBack`) it stays on until the
        // holder comes in again: a Heal Bell's cure off the field names the disguise (R-M11-51).
        if behavior.illusion {
            match cause {
                AbilityEndCause::Leaving => {}
                AbilityEndCause::Fainted => {
                    if let Some(holder) = self.state.pokemon_mut(pokemon) {
                        holder.illusion = None;
                    }
                }
                _ => self.end_illusion(pokemon),
            }
        }
        // Zen Mode (`onEnd`, DESIGN.md §43.4): a holder still standing goes back to its own
        // forme, silently.
        if let Some(rule) = behavior.self_forme {
            crate::effects::abilities::forme::end(self, pokemon, rule);
        }
        // Unburden (`removeVolatile('unburden')`): leaving, before Baton Pass hands the volatiles
        // on.
        if behavior.unburden {
            self.end_volatile(pokemon, VolatileConditionKind::Unburden);
        }
        // Flash Fire (`removeVolatile`), Protosynthesis / Quark Drive (`delete volatiles[..]`),
        // all with a silent `-end`. Slow Start's and Supreme Overlord's only announce it,
        // silently. Unnerve / As One (`unnerved`) and Gorilla Tactics (`choiceLock`) reset a
        // state only their own handlers read — none runs while the ability is suppressed, a
        // change of ability starts its state afresh, and so does a Pokémon coming in.
        let mut ended = Vec::new();
        ended.extend(behavior.ends_volatile);
        if behavior.protosynthesis {
            ended.push(VolatileConditionKind::Protosynthesis);
        }
        if behavior.quark_drive {
            ended.push(VolatileConditionKind::QuarkDrive);
        }
        for kind in ended {
            // The `-end … [silent]` line all the same: seen (DESIGN.md R-M10-26).
            if self
                .state
                .pokemon_mut(pokemon)
                .is_some_and(|holder| holder.volatiles.remove(kind))
            {
                self.trace.push(crate::engine::TraceEvent::VolatileEnded {
                    target: pokemon,
                    kind,
                    seen: true,
                });
            }
        }
    }

    /// Showdown `faintMessages` at the end of a move's hit loop (`hitStepMoveHitLoop`), before
    /// its recoil and `AfterMoveSecondarySelf`: a Neutralizing Gas holder the move knocked out
    /// ends its gas now — the abilities it suppressed run again for Rock Head, Magician, Life Orb's
    /// Sheer Force … — not at the action's end with the rest of its faint (whose `End` then finds
    /// the gas `ending` already).
    pub(crate) fn knocked_out_gas_ends(&mut self) {
        let holders: Vec<PokemonId> = self
            .active_pokemon()
            .into_iter()
            .filter(|holder| {
                self.state.pokemon(*holder).is_some_and(|pokemon| {
                    pokemon.hp == 0
                        && !pokemon.ability_state.ending
                        && crate::engine::ability_behavior(
                            self.rules,
                            crate::engine::raw_ability(pokemon),
                        )
                        .neutralizing_gas
                })
            })
            .collect();
        if holders.is_empty() {
            return;
        }
        // `faintMessages` takes the queue in order, each faint counted (`totalFainted`) and its
        // `Faint` event run before its ability's `End`: the faints up to the last knocked-out gas
        // holder are heard now, that gas still up for the ones before it (Soul-Heart stays
        // quiet, DESIGN.md R-M8-93), and counted for the Supreme Overlord the gas lets start
        // again (R-M10-61).
        let queue = self.state.faint_queue.clone();
        let heard = queue
            .iter()
            .rposition(|pokemon| holders.contains(pokemon))
            .map_or(&[][..], |last| &queue[..=last]);
        for &target in heard {
            let cause = match self
                .state
                .faint_causes
                .iter()
                .position(|cause| cause.target == target)
            {
                Some(index) => {
                    let cause = self.state.faint_causes[index];
                    self.state.faint_causes[index].listeners_run = true;
                    cause
                }
                None => {
                    let cause = crate::state::FaintCause {
                        target,
                        source: None,
                        by_move: false,
                        future_move: false,
                        listeners_run: false,
                        bonds_run: false,
                        after_faint_run: false,
                        counted: false,
                    };
                    self.state.faint_causes.push(crate::state::FaintCause {
                        listeners_run: true,
                        ..cause
                    });
                    cause
                }
            };
            self.count_faint(cause);
            self.faint_listeners(cause);
            if holders.contains(&target) {
                self.knocked_out_gas_ending(target);
            }
        }
        // A holder the queue does not hold (none expected) ends its gas all the same.
        for holder in holders {
            if !heard.contains(&holder) {
                self.knocked_out_gas_ending(holder);
            }
        }
    }

    /// One knocked-out gas ends, as `faintMessages` takes it: a holder another knocked-out gas
    /// still covers returns early, the last one starts the rest again. Each is done either way —
    /// gone in Showdown, `ending` here, so its faint's `End` later finds nothing to do.
    fn knocked_out_gas_ending(&mut self, holder: PokemonId) {
        self.neutralizing_gas_ended(holder);
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.ending = true;
        }
    }

    /// Neutralizing Gas coming in (its `onSwitchIn`): the gas is fresh (`ending` cleared), and
    /// each Pokémon on the field not behind Ability Shield (which says so: `-block|p|item: Ability
    /// Shield`, R-M10-55) has its strong weather ended — the other abilities get no `End`, they
    /// just stop counting.
    pub fn neutralizing_gas_started(&mut self, holder: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.ending = false;
        }
        for target in self.active_pokemon() {
            let Some(pokemon) = self.state.pokemon(target) else {
                continue;
            };
            if crate::engine::has_item(
                self.state,
                self.rules,
                pokemon,
                crate::ids::ItemId::ABILITY_SHIELD,
            ) {
                self.item_activated(target, crate::ids::ItemId::ABILITY_SHIELD);
                continue;
            }
            // The gas lifts a disguise (Illusion's `onEnd`, before the weather's).
            self.end_illusion(target);
            let Some(pokemon) = self.state.pokemon(target) else {
                continue;
            };
            if crate::engine::ability_behavior(self.rules, crate::engine::raw_ability(pokemon))
                .strong_weather
                .is_some()
            {
                self.end_ability(target, AbilityEndCause::Suppressed);
            }
        }
    }

    /// Neutralizing Gas's `onEnd`: unless another holder is still on the field, the gas clears
    /// (`abilityState.ending`) and the others' abilities start again, fastest first — save one
    /// that cannot be suppressed, which never stopped, and one behind Ability Shield. Showdown
    /// does this before the holder has left (its own FIXME); so does the engine.
    fn neutralizing_gas_ended(&mut self, holder: PokemonId) {
        // The others on the field — not a gas holder whose own `End` has run (`ending`: one the
        // same hit knocked out, processed just before; Showdown has it off the field already),
        // which neither keeps the rest suppressed nor starts again.
        let others: Vec<PokemonId> = self
            .active_pokemon()
            .into_iter()
            .filter(|other| {
                *other != holder
                    && self
                        .state
                        .pokemon(*other)
                        .is_some_and(|pokemon| !pokemon.ability_state.ending)
            })
            .collect();
        if others.iter().any(|other| {
            self.state.pokemon(*other).is_some_and(|pokemon| {
                crate::engine::ability_handler(self.state, self.rules, pokemon, |b| {
                    b.neutralizing_gas
                })
            })
        }) {
            return;
        }
        match self.state.pokemon_mut(holder) {
            Some(pokemon) if !pokemon.ability_state.ending => pokemon.ability_state.ending = true,
            _ => return,
        }
        for other in self.restart_order(others) {
            let Some(pokemon) = self.state.pokemon(other) else {
                continue;
            };
            if self
                .rules
                .conditions
                .cant_suppress
                .contains(&crate::engine::raw_ability(pokemon))
                || crate::engine::has_item(
                    self.state,
                    self.rules,
                    pokemon,
                    crate::ids::ItemId::ABILITY_SHIELD,
                )
            {
                continue;
            }
            self.start_ability(other);
            // Showdown's `onEnd` takes Gluttony's flag down after the `Start`: its berries wait
            // for the holder to take damage (R-M11-41).
            if let Some(pokemon) = self.state.pokemon_mut(other)
                && crate::engine::raw_ability(pokemon) == crate::ids::AbilityId::GLUTTONY
            {
                pokemon.ability_state.gluttony_waits = true;
            }
        }
    }

    /// Showdown `speedSort` of the abilities that start again: fastest first. Pokémon sharing a
    /// speed are shuffled, which shows only when two of them start something
    /// (`neutralizing_gas_restart_ties`): those draw a `SpeedTie` each; the others keep their
    /// position order.
    fn restart_order(&mut self, pokemon: Vec<PokemonId>) -> Vec<PokemonId> {
        let speeds: Vec<(PokemonId, u32)> = pokemon
            .into_iter()
            .map(|id| {
                (
                    id,
                    crate::engine::stats::action_speed(self.state, self.rules, id, None),
                )
            })
            .collect();
        let drawing = restart_ties(self.state, self.rules, &speeds);
        let mut keyed: Vec<(PokemonId, u32, u64)> = speeds
            .into_iter()
            .map(|(id, speed)| {
                let tie = if drawing.contains(&id) {
                    self.state
                        .rng
                        .next_u64_for(crate::state::RandomRequestKind::SpeedTie)
                } else {
                    0
                };
                (id, speed, tie)
            })
            .collect();
        keyed.sort_by_key(|(_, speed, tie)| std::cmp::Reverse((*speed, *tie)));
        keyed.into_iter().map(|(id, ..)| id).collect()
    }
}

/// The Pokémon among `speeds` (each with its action speed) that draw a `SpeedTie` as the
/// abilities start again: those sharing a speed with another whose ability starts something
/// (a switch-in handler or a strong weather), when there are at least two of them. Asked of
/// the ability each holds — the gas still sets them aside until its `End` — less the ones that
/// do not start again (Gastro Acid, `cantsuppress`, Ability Shield), so a replay can ask before
/// the action.
pub(crate) fn restart_ties(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    speeds: &[(PokemonId, u32)],
) -> Vec<PokemonId> {
    let starts = |id: PokemonId| {
        state.pokemon(id).is_some_and(|pokemon| {
            let ability = crate::engine::raw_ability(pokemon);
            let behavior = crate::engine::ability_behavior(rules, ability);
            !pokemon
                .volatiles
                .contains(VolatileConditionKind::GastroAcid)
                && !pokemon.ability_state.ending
                && !rules.conditions.cant_suppress.contains(&ability)
                && !crate::engine::has_item(
                    state,
                    rules,
                    pokemon,
                    crate::ids::ItemId::ABILITY_SHIELD,
                )
                && (behavior.switch_in.is_some() || behavior.strong_weather.is_some())
        })
    };
    speeds
        .iter()
        .filter(|(id, speed)| {
            starts(*id)
                && speeds
                    .iter()
                    .any(|(other, s)| other != id && s == speed && starts(*other))
        })
        .map(|(id, _)| *id)
        .collect()
}
