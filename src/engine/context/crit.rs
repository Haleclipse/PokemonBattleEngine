//! The critical-hit volatiles (DESIGN.md §20.1, R162): Focus Energy, Dragon Cheer and Laser
//! Focus as moves start them, and what Psych Up copies of them.

use crate::dex::PokemonType;
use crate::engine::{ConditionApplyError, TraceEvent};
use crate::ids::PokemonId;
use crate::state::VolatileConditionKind;

use super::BattleContext;

/// Laser Focus's `duration`.
const LASER_FOCUS_TURNS: u8 = 2;

impl<'a> BattleContext<'a> {
    fn started(&mut self, target: PokemonId, kind: VolatileConditionKind) {
        self.trace
            .push(TraceEvent::VolatileStarted { target, kind });
    }

    /// Focus Energy (`onStart`: refused under Dragon Cheer; no `onRestart`, so refused when
    /// already there).
    pub fn set_focus_energy(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon
            .volatiles
            .contains(VolatileConditionKind::DragonCheer)
            || !pokemon
                .volatiles
                .set_marker(VolatileConditionKind::FocusEnergy)
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.started(target, VolatileConditionKind::FocusEnergy);
        Ok(())
    }

    /// Dragon Cheer (`onStart`: refused under Focus Energy): +2 for a Dragon type as it starts,
    /// else +1 (`effectState.hasDragonType`).
    pub fn set_dragon_cheer(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let dragon = self.has_type(target, PokemonType::Dragon);
        self.set_dragon_cheer_stages(target, if dragon { 2 } else { 1 })
    }

    fn set_dragon_cheer_stages(
        &mut self,
        target: PokemonId,
        stages: u8,
    ) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon
            .volatiles
            .contains(VolatileConditionKind::FocusEnergy)
            || !pokemon.volatiles.set_dragon_cheer(stages)
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.started(target, VolatileConditionKind::DragonCheer);
        Ok(())
    }

    /// Charge (`addVolatile('charge')`): started, or started again (`onRestart`) when already
    /// there — a standing Pokémon only.
    pub fn charge(&mut self, target: PokemonId) {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return;
        };
        if pokemon.is_fainted() {
            return;
        }
        pokemon.volatiles.set_marker(VolatileConditionKind::Charge);
        self.started(target, VolatileConditionKind::Charge);
    }

    /// Laser Focus: two turns, started again (`onRestart`) when already there.
    pub fn set_laser_focus(&mut self, target: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(target) {
            pokemon.volatiles.set_laser_focus(LASER_FOCUS_TURNS);
            self.started(target, VolatileConditionKind::LaserFocus);
        }
    }

    /// Gen 6+ Psych Up (`volatilesToCopy`): the user drops its own critical-hit volatiles, then
    /// takes those the target has — Dragon Cheer with the target's stages, Laser Focus anew.
    /// The ends are seen: Dragon Cheer and Focus Energy have no line of their own, but the
    /// `-copyboost` shows the user's now the target's, as the clients read it (R-M10-45, which
    /// R-M10-41's unseen ends got wrong).
    pub fn copy_crit_volatiles(&mut self, user: PokemonId, target: PokemonId) {
        self.end_volatile(user, VolatileConditionKind::DragonCheer);
        self.end_volatile(user, VolatileConditionKind::FocusEnergy);
        self.end_volatile(user, VolatileConditionKind::LaserFocus);
        let Some(source) = self.state.pokemon(target) else {
            return;
        };
        let dragon_cheer = source.volatiles.dragon_cheer_stages();
        let focus_energy = source
            .volatiles
            .contains(VolatileConditionKind::FocusEnergy);
        let laser_focus = source.volatiles.contains(VolatileConditionKind::LaserFocus);
        if dragon_cheer > 0 {
            let _ = self.set_dragon_cheer_stages(user, dragon_cheer);
        }
        if focus_energy {
            let _ = self.set_focus_energy(user);
        }
        if laser_focus {
            self.set_laser_focus(user);
        }
    }
}
