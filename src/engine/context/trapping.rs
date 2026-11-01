//! `BattleContext`'s trapping (DESIGN.md §30.8): Mean Look, Block, Jaw Lock … put a `Trapped`
//! volatile on the target, linked to a `Trapper` on its source (Showdown `addVolatile('trapped',
//! source, move, 'trapper')`), and the links are released as either leaves the field.

use crate::dex::PokemonType;
use crate::ids::{MoveId, PokemonId};
use crate::rules::{BINDING_LOCK_TURNS, BINDING_LOCK_WEIGHTS, GEN1_PARTIALLY_TRAPPED_TURNS};
use crate::state::{RandomRequestKind, Stat, VolatileConditionKind};

use super::BattleContext;
use crate::engine::{
    ConditionApplyError, EffectSource, StatChangeCause, StatChangeRequest, TraceEvent,
};

impl<'a> BattleContext<'a> {
    /// Showdown `target.addVolatile('trapped', source, move, 'trapper')`: both must be on the
    /// field and standing, the target not trapped yet and, from Gen 6, no Ghost type
    /// (`runStatusImmunity('trapped')`, `ghost_escapes_trapping`). The source becomes a trapper.
    pub fn trap(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        for pokemon in [target, source] {
            if self.is_fainted(pokemon) || !self.is_active(pokemon) {
                return Err(ConditionApplyError::InvalidTarget);
            }
        }
        if self.rules.conditions.ghost_escapes_trapping && self.has_type(target, PokemonType::Ghost)
        {
            return Err(ConditionApplyError::Immune);
        }
        let trapped = self
            .state
            .pokemon_mut(target)
            .is_some_and(|pokemon| pokemon.volatiles.set_trapped(source));
        if !trapped {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target,
            kind: VolatileConditionKind::Trapped,
        });
        if self
            .state
            .pokemon_mut(source)
            .is_some_and(|pokemon| pokemon.volatiles.set_trapper())
        {
            self.trace.push(TraceEvent::VolatileStarted {
                target: source,
                kind: VolatileConditionKind::Trapper,
            });
        }
        Ok(())
    }

    /// Showdown `removeLinkedVolatiles` as `leaving` leaves the field (switched, dragged out or
    /// down): the Pokémon it trapped go free, and the one that trapped it stops being a trapper
    /// once it traps nobody else.
    pub(crate) fn release_trap_links(&mut self, leaving: PokemonId) {
        let held_by = self
            .state
            .pokemon(leaving)
            .and_then(|pokemon| pokemon.volatiles.trapped_by());
        let freed: Vec<PokemonId> = self
            .state
            .pokemon_ids()
            .filter(|other| *other != leaving)
            .filter(|other| {
                self.state
                    .pokemon(*other)
                    .and_then(|pokemon| pokemon.volatiles.trapped_by())
                    == Some(leaving)
            })
            .collect();
        for other in freed {
            self.drop_volatile(other, VolatileConditionKind::Trapped);
        }
        if let Some(trapper) = held_by {
            self.drop_idle_trapper(trapper, leaving);
        }
    }

    /// Showdown `removeVolatile('trapped')`: `pokemon` goes free, and the one that trapped it
    /// stops being a trapper once it traps nobody else (`removeLinkedVolatiles`).
    pub(crate) fn free_trapped(&mut self, pokemon: PokemonId) {
        let Some(trapper) = self
            .state
            .pokemon(pokemon)
            .and_then(|pokemon| pokemon.volatiles.trapped_by())
        else {
            return;
        };
        self.drop_volatile(pokemon, VolatileConditionKind::Trapped);
        self.drop_idle_trapper(trapper, pokemon);
    }

    /// `trapper`'s `Trapper` ends unless it still traps someone other than `freed`.
    fn drop_idle_trapper(&mut self, trapper: PokemonId, freed: PokemonId) {
        if !self.state.pokemon_ids().any(|other| {
            other != freed
                && self
                    .state
                    .pokemon(other)
                    .and_then(|pokemon| pokemon.volatiles.trapped_by())
                    == Some(trapper)
        }) {
            self.end_volatile(trapper, VolatileConditionKind::Trapper);
        }
    }

    /// Octolock (`volatileStatus: 'octolock'`, failing while one is there; its Ghost immunity is
    /// the hit gate's, `TryImmunity::Trappable`): the target is octolocked by `source`.
    pub fn octolock(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        let started = self
            .state
            .pokemon_mut(target)
            .filter(|pokemon| !pokemon.is_fainted())
            .is_some_and(|pokemon| pokemon.volatiles.set_octolock(source));
        if !started {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target,
            kind: VolatileConditionKind::Octolock,
        });
        Ok(())
    }

    /// Octolock's residual (`onResidualOrder` 14): gone quietly once its source is off the field,
    /// down or new to it (`!source.activeTurns`); otherwise the holder's Defense and Special
    /// Defense −1, by the source's Octolock (Clear Body, Mirror Armor … apply).
    pub(crate) fn octolock_residual(&mut self, holder: PokemonId) {
        let Some(source) = self
            .state
            .pokemon(holder)
            .and_then(|pokemon| pokemon.volatiles.octolocked_by())
        else {
            return;
        };
        if !self.is_active(source) || self.is_fainted(source) || self.active_turns(source) == 0 {
            self.end_volatile(holder, VolatileConditionKind::Octolock);
            return;
        }
        for stat in [Stat::Defense, Stat::SpecialDefense] {
            self.try_change_stat_stage(StatChangeRequest {
                target: holder,
                stat,
                stages: -1,
                source: EffectSource::Move {
                    pokemon: source,
                    move_id: MoveId::OCTOLOCK,
                },
                cause: StatChangeCause::Move,
            });
        }
    }

    /// Gen 1 `partiallytrapped` on `target` by `source` (`duration: 2`; `onRestart` sets it to 2
    /// again): the target cannot move while it lasts.
    pub(crate) fn gen1_partially_trap(
        &mut self,
        target: PokemonId,
        source: PokemonId,
        move_id: MoveId,
    ) {
        let Some(pokemon) = self.state.pokemon_mut(target).filter(|p| !p.is_fainted()) else {
            return;
        };
        let fresh = !pokemon
            .volatiles
            .contains(VolatileConditionKind::PartiallyTrapped);
        pokemon
            .volatiles
            .remove(VolatileConditionKind::PartiallyTrapped);
        pokemon
            .volatiles
            .set_partially_trapped(source, move_id, GEN1_PARTIALLY_TRAPPED_TURNS);
        if fresh {
            self.trace.push(TraceEvent::VolatileStarted {
                target,
                kind: VolatileConditionKind::PartiallyTrapped,
            });
        }
    }

    /// Gen 1 `self.volatileStatus: 'partialtrappinglock'` after a binding move's hit: a user not
    /// locked yet rolls the lock's turns (`sample([2, 2, 2, 3, 3, 3, 4, 5])`) and locks onto
    /// its foe with the damage dealt; the lock's `onStart` partially traps the foe. A foe the
    /// hit knocked out leaves no lock: the turns are rolled (`durationCallback` comes first),
    /// then `onStart` finds no foe with HP (`target.foes()`).
    pub(crate) fn start_binding_lock(
        &mut self,
        user: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        damage: u16,
    ) {
        let locked = self.state.pokemon(user).is_none_or(|pokemon| {
            pokemon.is_fainted() || pokemon.volatiles.partial_trapping_lock().is_some()
        });
        if locked {
            return;
        }
        let turns = self.state.rng.sample_weighted_for(
            RandomRequestKind::Duration,
            &BINDING_LOCK_TURNS,
            &BINDING_LOCK_WEIGHTS,
        ) as u8;
        if self.is_fainted(target) {
            return;
        }
        if let Some(pokemon) = self.state.pokemon_mut(user) {
            pokemon
                .volatiles
                .set_partial_trapping_lock(target, move_id, damage, turns);
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target: user,
            kind: VolatileConditionKind::PartialTrappingLock,
        });
        self.gen1_partially_trap(target, user, move_id);
    }

    /// The Gen 1 lock's `onAfterMove`: the caught Pokémon down, the lock is gone; on its last
    /// turn both Pokémon show `fakepartiallytrapped` (unless it ran the full 5 turns); otherwise
    /// the target is partially trapped again.
    pub(crate) fn binding_lock_after_move(&mut self, user: PokemonId, target: PokemonId) {
        let Some((locked, move_id, _, turns, total)) = self
            .state
            .pokemon(user)
            .and_then(|pokemon| pokemon.volatiles.partial_trapping_lock())
        else {
            return;
        };
        if self.is_fainted(target) {
            self.end_volatile(user, VolatileConditionKind::PartialTrappingLock);
            return;
        }
        if turns == 1 {
            if total != 5 {
                for (holder, counterpart) in [(user, target), (target, user)] {
                    if let Some(pokemon) = self.state.pokemon_mut(holder) {
                        pokemon.volatiles.set_fake_partially_trapped(counterpart);
                    }
                    self.trace.push(TraceEvent::VolatileStarted {
                        target: holder,
                        kind: VolatileConditionKind::FakePartiallyTrapped,
                    });
                }
            }
        } else if target == locked {
            self.gen1_partially_trap(target, user, move_id);
        }
    }
}
