//! `BattleContext`'s side of Bide (DESIGN.md §15.10): the volatile as it starts, and the damage
//! it stores.

use crate::ids::PokemonId;
use crate::state::{RandomRequestKind, VolatileConditionKind};

use super::BattleContext;
use crate::engine::{ConditionApplyError, TraceEvent};

impl<'a> BattleContext<'a> {
    /// Bide on its user (`addVolatile('bide')`): its `duration` (Gen 2's `durationCallback`
    /// draws 3 or 4) or, Gen 1, its `time` (2 or 3, drawn by `onStart`). Refused when already
    /// there.
    pub fn start_bide(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        let rules = self.rules.conditions.retaliation.bide;
        let pokemon = self
            .state
            .pokemon(user)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if pokemon.is_fainted() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        if pokemon.volatiles.contains(VolatileConditionKind::Bide) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        let (min, max) = rules.turns;
        let drawn = if min == max {
            min
        } else {
            self.state.rng.range_inclusive_for(
                RandomRequestKind::Duration,
                u16::from(min),
                u16::from(max),
            ) as u8
        };
        let (turns, time) = if rules.gen1 {
            (None, drawn)
        } else {
            (Some(drawn), 0)
        };
        if let Some(pokemon) = self.state.pokemon_mut(user) {
            pokemon.volatiles.start_bide(turns, time, rules.traps);
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target: user,
            kind: VolatileConditionKind::Bide,
        });
        Ok(())
    }

    /// Gen 2+ Bide's `onDamage` (priority -101: after every other handler has had its say, before
    /// the HP cap): a move's damage to the holder is stored, and its user remembered.
    pub(crate) fn store_bide_damage(&mut self, target: PokemonId, amount: u16, source: PokemonId) {
        if self.rules.conditions.retaliation.bide.gen1 {
            return;
        }
        if let Some(bide) = self
            .state
            .pokemon_mut(target)
            .and_then(|pokemon| pokemon.volatiles.bide_mut())
        {
            bide.damage = bide.damage.saturating_add(u32::from(amount));
            bide.source = Some(source);
        }
    }

    /// Gen 1 (`faintMessages`, the Desync Clause Mod): a faint wipes what every Bide on the field
    /// stored.
    pub(crate) fn wipe_gen1_bide_damage(&mut self) {
        if !self.rules.conditions.retaliation.bide.gen1 {
            return;
        }
        for pokemon in self.active_pokemon() {
            if let Some(bide) = self
                .state
                .pokemon_mut(pokemon)
                .and_then(|p| p.volatiles.bide_mut())
            {
                bide.damage = 0;
            }
        }
    }
}
