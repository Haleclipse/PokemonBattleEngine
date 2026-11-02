//! `BattleContext`'s major status (DESIGN.md §22): a status tried, carried (Psycho Shift, Synchronize)
//! or inflicted by an ability, and cured.

use crate::engine::{CureReason, TraceEvent};
use crate::ids::{AbilityId, PokemonId};
use crate::state::{NonVolatileStatus, VolatileConditionKind};

use super::*;

impl<'a> BattleContext<'a> {
    /// Removes the Pokémon's major status (Shed Skin, Natural Cure, Heal Bell …); false when
    /// it had none or is down. The toxic counter goes with the status.
    pub fn cure_status(&mut self, target: PokemonId, reason: CureReason) -> bool {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return false;
        };
        if pokemon.is_fainted() {
            return false;
        }
        let Some(status) = pokemon.status.take() else {
            return false;
        };
        // Showdown `cureStatus`: a Nightmare ends with the sleep.
        let nightmare_ended = status.kind == NonVolatileStatus::Sleep
            && pokemon.volatiles.remove(VolatileConditionKind::Nightmare);
        self.trace.push(TraceEvent::StatusCured {
            target,
            status: status.kind,
            reason,
        });
        if nightmare_ended {
            self.trace.push(TraceEvent::VolatileEnded {
                target,
                kind: VolatileConditionKind::Nightmare,
                seen: true,
            });
        }
        true
    }

    /// A status `holder`'s ability inflicts on its own (Static, Flame Body, Poison Point, Effect
    /// Spore, Poison Touch): the ability shows itself only on the status's line — a sleep from Gen
    /// 5 (`StatusLifecycleRules::ability_sleep_named`); a status that does not land shows nothing.
    pub fn inflict_status_by_ability(
        &mut self,
        holder: PokemonId,
        target: PokemonId,
        status: NonVolatileStatus,
        ability: AbilityId,
    ) -> crate::engine::StatusApplyResult {
        let result = self.try_set_status(
            target,
            status,
            EffectSource::Ability {
                pokemon: holder,
                ability,
            },
        );
        if matches!(result, crate::engine::StatusApplyResult::Applied { .. })
            && (status != NonVolatileStatus::Sleep
                || self.rules.status.lifecycle.ability_sleep_named)
        {
            self.ability_activated(holder, ability);
        } else {
            self.ability_at_work(holder, ability);
        }
        result
    }

    pub fn try_set_status(
        &mut self,
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
    ) -> crate::engine::StatusApplyResult {
        crate::engine::status::apply_status(
            self.state, self.rules, self.trace, target, status, source, false,
        )
    }

    /// `try_set_status` for a status the effect carries itself (`TrySetStatusContext::carried`:
    /// a status move's own, one Synchronize passes on).
    pub fn try_set_carried_status(
        &mut self,
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
    ) -> crate::engine::StatusApplyResult {
        crate::engine::status::apply_status(
            self.state, self.rules, self.trace, target, status, source, true,
        )
    }

    /// `try_set_status` for a status whose doer the source does not name (Yawn's sleep: the
    /// yawner's side).
    pub fn try_set_status_inflicted(
        &mut self,
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
        inflicted_by: Option<crate::ids::SideId>,
    ) -> crate::engine::StatusApplyResult {
        crate::engine::status::set_status(
            self.state,
            self.rules,
            self.trace,
            target,
            status,
            source,
            crate::engine::status::SetStatusHow {
                replace: false,
                inflicted_by,
                carried: false,
            },
        )
    }
}
