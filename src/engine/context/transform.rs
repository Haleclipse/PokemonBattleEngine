//! Transform (Showdown `pokemon.transformInto`, DESIGN.md §41.7): the user battles as a copy of
//! its target — species, types, stats, ability, moves — kept in `PokemonState::transform` while
//! its own values stay as they are; leaving the field drops the copy.

use super::{AbilityEndCause, BattleContext};
use crate::engine::TraceEvent;
use crate::ids::{AbilityId, PokemonId};
use crate::state::{MoveSlotState, TransformState, VolatileConditionKind};

impl<'a> BattleContext<'a> {
    /// The foe standing straight across from `pokemon` (Showdown `side.foe.active[length - 1 -
    /// position]`: in doubles the left faces the right), if any.
    pub fn foe_across(&self, pokemon: PokemonId) -> Option<PokemonId> {
        let topology = &self.rules.topology;
        let spec = topology.spec(self.state.active_position(pokemon, topology)?)?;
        let slots = usize::from(topology.active_slots_per_side());
        let foes = &self.state.side(spec.side.other()).active;
        foes.get(slots.checked_sub(1 + usize::from(spec.slot))?)
            .copied()
            .flatten()
    }

    /// `user` transforms into `target` (Transform's `onHit`, or Imposter's `by`): false when it
    /// cannot — either has transformed already (`TransformRules`), or the target hides behind a
    /// Substitute (Gen 5+). (A fainted target is never reached: it has left its slot.)
    pub fn transform_into(
        &mut self,
        user: PokemonId,
        target: PokemonId,
        by: Option<AbilityId>,
    ) -> bool {
        let rules = self.rules.conditions.transform;
        let (Some(pokemon), Some(into)) = (self.state.pokemon(user), self.state.pokemon(target))
        else {
            return false;
        };
        if pokemon.illusion.is_some()
            || into.illusion.is_some()
            || (rules.fails_into_transformed && into.transform.is_some())
            || (rules.fails_when_transformed && pokemon.transform.is_some())
            || (rules.fails_behind_substitute
                && into.volatiles.contains(VolatileConditionKind::Substitute))
        {
            return false;
        }
        let generation = self.rules.generation;
        let moves = into
            .move_slots()
            .iter()
            .map(|slot| {
                let move_def = self.dex.and_then(|dex| dex.move_by_id(slot.move_id));
                let base = move_def.map_or(slot.max_pp, |m| m.max_pp);
                let pp = base.min(5);
                let max_pp = if rules.max_pp_capped {
                    pp
                } else {
                    move_def.map_or(slot.max_pp, |m| crate::dex::pp_with_ups(generation, m))
                };
                MoveSlotState {
                    move_id: slot.move_id,
                    pp,
                    max_pp,
                    used: false,
                    // `transformInto` copies the slots' targets too.
                    target: slot.target,
                }
            })
            .collect();
        let copy = TransformState {
            into: target,
            species: crate::engine::species_of(into),
            types: into.battle_types(),
            stats: *into.battle_stats(),
            ability: crate::engine::raw_ability(pokemon),
            moves,
            hidden_power: if rules.copies_hidden_power {
                into.hidden_power_of()
            } else {
                pokemon.hidden_power_of()
            },
        };
        let (stages, added_type, modified_stats, times_attacked, autotomize, ability) = (
            into.stages,
            into.added_type,
            into.modified_stats,
            into.history.times_attacked,
            into.volatiles.autotomize_count(),
            crate::engine::raw_ability(into),
        );
        let Some(pokemon) = self.state.pokemon_mut(user) else {
            return false;
        };
        pokemon.transform = Some(Box::new(copy));
        pokemon.added_type = added_type;
        // Gen 1 copies the modified stats too (only Gen 1 has them).
        if pokemon.modified_stats.is_some() {
            pokemon.modified_stats = modified_stats;
        }
        pokemon.stages = stages;
        pokemon.history.times_attacked = times_attacked;
        if rules.copies_autotomize {
            pokemon.volatiles.set_autotomize_count(autotomize);
        }
        if rules.ends_disable {
            pokemon.volatiles.remove(VolatileConditionKind::Disable);
        }
        if rules.clears_announcements {
            pokemon.history.announced.clear();
        }
        if rules.copies_crit_volatiles {
            self.copy_crit_volatiles(user, target);
        }
        let mut copied = [None; 4];
        if let Some(transform) = self.state.pokemon(user).and_then(|p| p.transform.as_ref()) {
            for (copy, slot) in copied.iter_mut().zip(&transform.moves) {
                *copy = Some(slot.move_id);
            }
        }
        self.trace.push(TraceEvent::Transformed {
            user,
            into: target,
            by,
            copied,
        });
        if rules.copies_ability {
            self.end_ability(user, AbilityEndCause::Replaced);
            let Some(pokemon) = self.state.pokemon_mut(user) else {
                return true;
            };
            let old = pokemon.change_ability(ability);
            self.state.restart_ability_state(user);
            self.trace.push(TraceEvent::AbilityChanged {
                pokemon: user,
                from: old,
                to: ability,
                line: crate::engine::AbilityChangeLine::Copied,
            });
            if rules.starts_different_ability && old != ability {
                self.start_ability(user);
            }
        }
        true
    }
}
