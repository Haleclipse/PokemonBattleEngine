//! Lock-On and Mind Reader (Showdown `lockon`; DESIGN.md §15.14): the lock as the move sets it,
//! and whose moves it lets through.

use crate::ids::{MoveId, PokemonId};
use crate::rules::{LockOnModel, ResolvedRules, SemiInvulnerableKind};
use crate::state::{BattleState, VolatileConditionKind};

use super::BattleContext;
use crate::engine::{ConditionApplyError, TraceEvent};

/// Showdown `lockon`'s `duration`: the turn it is set and the next.
pub(crate) const LOCK_ON_TURNS: u8 = 2;

/// Whether `attacker`'s moves against `target` skip their accuracy check (Showdown
/// `onSourceAccuracy`; Gen 2 on the cartridge: the target's flag, which any foe's move reads).
pub(crate) fn locked_on(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
) -> bool {
    match rules.conditions.lock_on.model {
        LockOnModel::OnUser => state
            .pokemon(attacker)
            .and_then(|p| p.volatiles.lock_on())
            .is_some_and(|(locked, _)| locked == target),
        LockOnModel::OnTarget => {
            attacker != target
                && state
                    .pokemon(target)
                    .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::LockedOn))
                && state
                    .pokemon(attacker)
                    .zip(state.pokemon(target))
                    .is_some_and(|(a, t)| a.side != t.side)
        }
    }
}

/// Whether the lock carries `attacker`'s `move_id` to a semi-invulnerable `target` (Showdown
/// `onSourceInvulnerability`, Gen 3+). Gen 2 on the cartridge (`CheckHit`'s `.LockOn`, before
/// `.FlyDigMoves`): a digging target is reached, a flying one by anything but Earthquake,
/// Fissure and Magnitude.
pub(crate) fn lock_on_reaches(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
    move_id: MoveId,
) -> bool {
    if !locked_on(state, rules, attacker, target) {
        return false;
    }
    match rules.conditions.lock_on.model {
        LockOnModel::OnUser => true,
        LockOnModel::OnTarget => !state.pokemon(target).is_some_and(|p| {
            crate::engine::two_turn::hiding(p) == Some(SemiInvulnerableKind::Flying)
                && matches!(
                    move_id,
                    MoveId::EARTHQUAKE | MoveId::FISSURE | MoveId::MAGNITUDE
                )
        }),
    }
}

impl<'a> BattleContext<'a> {
    /// Lock-On / Mind Reader's `onTryHit` and `onHit`. Gen 3+ (`LockOnModel::OnUser`): the
    /// user takes aim at `target` for 2 turns — refused while it holds a lock, unless the
    /// generation sets it anew (Gen 3-4 on the cartridge). Gen 2 (`OnTarget`, the cartridge):
    /// the target is flagged, whatever was there.
    pub fn lock_on(
        &mut self,
        user: PokemonId,
        target: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        let rules = self.rules.conditions.lock_on;
        match rules.model {
            LockOnModel::OnUser => {
                let held = self
                    .state
                    .pokemon(user)
                    .ok_or(ConditionApplyError::InvalidTarget)?
                    .volatiles
                    .lock_on()
                    .is_some();
                if held && !rules.again_refreshes {
                    return Err(ConditionApplyError::AlreadyActive);
                }
                if let Some(pokemon) = self.state.pokemon_mut(user) {
                    pokemon.volatiles.set_lock_on(target, LOCK_ON_TURNS);
                }
                self.trace.push(TraceEvent::VolatileStarted {
                    target: user,
                    kind: VolatileConditionKind::LockOn,
                });
                Ok(())
            }
            LockOnModel::OnTarget => {
                let _ = self.set_marker_volatile(target, VolatileConditionKind::LockedOn);
                Ok(())
            }
        }
    }

    /// Gen 2 on the cartridge: the accuracy check against `target` reads its flag and clears it.
    pub(crate) fn consume_locked_on(&mut self, target: PokemonId) {
        if self.rules.conditions.lock_on.model == LockOnModel::OnTarget {
            self.end_volatile(target, VolatileConditionKind::LockedOn);
        }
    }
}
