//! Events on a side (DESIGN.md §20.2, R167): a side condition starting — Showdown
//! `addSideCondition` runs the condition's `SideStart`, then `runEvent('SideConditionStart',
//! side, source, condition)`, which asks every Pokémon on that side (Wind Power, Wind Rider on
//! Tailwind) — and a move aimed at its user's own side (`runEvent('TryHitSide')`: the partners'
//! `onAllyTryHitSide`, Sap Sipper).

use crate::ids::{PokemonId, SideId};
use crate::state::SideConditionKind;

use super::BattleContext;
use crate::engine::TraceEvent;

impl<'a> BattleContext<'a> {
    /// Whether `kind` is up on `side`.
    pub fn side_has_condition(&self, side: SideId, kind: SideConditionKind) -> bool {
        self.state.side(side).conditions.contains(kind)
    }

    /// The combined Pledge's side condition (Showdown `runMoveEffects`' `sideCondition` /
    /// `self.sideCondition`, DESIGN.md §42.9): four turns, announced like any other side
    /// condition. A second one on the same side renews it (Showdown's `addSideCondition`).
    pub fn set_pledge_condition(&mut self, side: SideId, kind: SideConditionKind) {
        let turns = self.rules.conditions.calls.pledge_turns;
        let fresh = !self.state.side(side).conditions.contains(kind);
        self.state.side_mut(side).conditions.set_timed(kind, turns);
        if fresh {
            self.side_condition_started(side, kind);
        }
    }

    /// Showdown `tryMoveHit`'s `runEvent('TryHitSide')` for a move `user` aims at its own side
    /// (Aromatherapy): the `AbilityBehavior::ally_try_hit_side` of each standing Pokémon there —
    /// the user's own too (`alliesAndSelf`): Soundproof names a user's own (R-M10-53), Sap
    /// Sipper leaves it out itself (`source === effectState.target`).
    pub(crate) fn ally_try_hit_side(
        &mut self,
        user: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) {
        let Some(side) = self.side_of(user) else {
            return;
        };
        let listeners: Vec<(PokemonId, crate::effects::abilities::AllyTryHitSideHandler)> = self
            .state
            .side(side)
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|pokemon| !self.is_fainted(*pokemon))
            .filter_map(|pokemon| {
                let ability = self.running_ability(pokemon)?;
                crate::engine::ability_behavior(self.rules, ability)
                    .ally_try_hit_side
                    .map(|handler| (pokemon, handler))
            })
            .collect();
        for (holder, handler) in listeners {
            handler(self, holder, user, move_def);
        }
    }

    /// `kind` has just started on `side` (not a layer added to one already up): traced, then
    /// each standing active Pokémon on `side` with an `AbilityBehavior::side_condition_start`
    /// reacts, in field order (each reacts on itself alone).
    pub(crate) fn side_condition_started(&mut self, side: SideId, kind: SideConditionKind) {
        self.trace
            .push(TraceEvent::SideConditionStarted { side, kind });
        let listeners: Vec<(
            PokemonId,
            crate::effects::abilities::SideConditionStartHandler,
        )> = self
            .state
            .side(side)
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|pokemon| !self.is_fainted(*pokemon))
            .filter_map(|pokemon| {
                let ability = self.running_ability(pokemon)?;
                crate::engine::ability_behavior(self.rules, ability)
                    .side_condition_start
                    .map(|handler| (pokemon, handler))
            })
            .collect();
        for (holder, handler) in listeners {
            handler(self, holder, kind);
        }
    }
}
