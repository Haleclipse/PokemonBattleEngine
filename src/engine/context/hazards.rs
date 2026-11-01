//! `BattleContext`'s entry hazards cleared, laid by a hit or swapped (DESIGN.md §14.2): Rapid
//! Spin, Mortal Spin, Stone Axe, Ceaseless Edge, Defog, Court Change, Toxic Debris.

use crate::ids::{MoveId, PokemonId, SideId};
use crate::state::{SideConditionKind, Stat, VolatileConditionKind};

use super::{BattleContext, ConditionApplyError};
use crate::engine::{
    EffectSource, StatChangeCause, StatChangeRequest, StatChangeResult, TraceEvent,
};

/// The entry hazards, in the order Showdown's lists remove them.
const HAZARDS: [SideConditionKind; 4] = [
    SideConditionKind::Spikes,
    SideConditionKind::ToxicSpikes,
    SideConditionKind::StealthRock,
    SideConditionKind::StickyWeb,
];

/// What Defog takes off the target's side before its hazards (Showdown's `removeTarget`); only
/// the hazards count as its success.
const DEFOG_TARGET_SIDE: [SideConditionKind; 5] = [
    SideConditionKind::Reflect,
    SideConditionKind::LightScreen,
    SideConditionKind::AuroraVeil,
    SideConditionKind::Safeguard,
    SideConditionKind::Mist,
];

/// Court Change's list: every side condition but the single-turn guards.
fn court_change_swaps(kind: SideConditionKind) -> bool {
    !matches!(
        kind,
        SideConditionKind::WideGuard
            | SideConditionKind::QuickGuard
            | SideConditionKind::CraftyShield
            | SideConditionKind::MatBlock
    )
}

impl<'a> BattleContext<'a> {
    /// Rapid Spin / Mortal Spin clearing the user's way: its Leech Seed ends, then its side's
    /// entry hazards, then its bind. Nothing for a user that is down.
    pub fn spin_away(&mut self, user: PokemonId) {
        let Some(side) = self
            .state
            .pokemon(user)
            .filter(|pokemon| !pokemon.is_fainted())
            .map(|pokemon| pokemon.side)
        else {
            return;
        };
        self.end_volatile(user, VolatileConditionKind::LeechSeed);
        for kind in HAZARDS {
            self.remove_side_condition(side, kind);
        }
        self.end_volatile(user, VolatileConditionKind::PartiallyTrapped);
    }

    /// Tidy Up: the entry hazards come off the user's side, then off the foes'; `true` when one
    /// did.
    pub fn tidy_up_hazards(&mut self, user: PokemonId) -> bool {
        let Some(side) = self.state.pokemon(user).map(|pokemon| pokemon.side) else {
            return false;
        };
        let mut removed = false;
        for side in [side, side.other()] {
            for kind in HAZARDS {
                removed |= self.remove_side_condition(side, kind);
            }
        }
        removed
    }

    /// The layers of Spikes / Toxic Spikes on `side` (1 for a hazard without layers that is up).
    pub fn hazard_layers(&self, side: SideId, kind: SideConditionKind) -> u8 {
        let conditions = &self.state.side(side).conditions;
        match kind {
            SideConditionKind::Spikes => conditions.spikes_layers(),
            SideConditionKind::ToxicSpikes => conditions.toxic_spikes_layers(),
            other => u8::from(conditions.contains(other)),
        }
    }

    /// Lays one entry hazard on `side` as `setSideCondition` would (Stone Axe, Ceaseless Edge,
    /// Toxic Debris): false when it could not take (Stealth Rock already up, the layers full).
    pub fn lay_hazard(&mut self, side: SideId, kind: SideConditionKind, source: PokemonId) -> bool {
        match kind {
            SideConditionKind::StealthRock => self.set_stealth_rock(side, source).is_ok(),
            SideConditionKind::Spikes => self.add_spikes(side, source).is_ok(),
            SideConditionKind::ToxicSpikes => self.add_toxic_spikes(side, source).is_ok(),
            SideConditionKind::StickyWeb => self.set_sticky_web(side, source).is_ok(),
            _ => false,
        }
    }

    /// Defog's `onHit` on `target` (`ConditionRules::defog`): the target's evasion −1 — not
    /// through a Substitute, unless (Gen 6+) the move infiltrates —, the screens, Safeguard,
    /// Mist and entry hazards off the target's side, then (Gen 6+) the hazards off the user's
    /// and (Gen 8+) the terrain. From Gen 6 it fails when it lowered no evasion and removed no
    /// hazard.
    pub fn defog(&mut self, user: PokemonId, target: PokemonId) -> Result<(), ConditionApplyError> {
        let rules = self.rules.conditions.defog;
        let (Some(user_side), Some(target_side)) = (self.side_of(user), self.side_of(target))
        else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        let behind_substitute = self.state.pokemon(target).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .contains(VolatileConditionKind::Substitute)
        });
        let infiltrates = self.rules.conditions.substitute_honors_infiltrates
            && crate::engine::infiltrates(self.state, self.rules, user);
        let mut success = false;
        if !behind_substitute || infiltrates {
            success = matches!(
                self.try_change_stat_stage(StatChangeRequest {
                    target,
                    stat: Stat::Evasion,
                    stages: -1,
                    source: EffectSource::Move {
                        pokemon: user,
                        move_id: MoveId::DEFOG,
                    },
                    cause: StatChangeCause::Move,
                }),
                StatChangeResult::Applied { .. }
            );
        }
        for kind in DEFOG_TARGET_SIDE {
            self.remove_side_condition(target_side, kind);
        }
        for kind in HAZARDS {
            success |= self.remove_side_condition(target_side, kind);
        }
        if rules.clears_own_side {
            for kind in HAZARDS {
                success |= self.remove_side_condition(user_side, kind);
            }
        }
        if rules.clears_terrain {
            self.clear_terrain();
        }
        if rules.clears_own_side && !success {
            return Err(ConditionApplyError::NothingToDo);
        }
        Ok(())
    }

    /// Court Change (`onHitField`): the two sides trade their conditions — hazards with their
    /// layers, screens and Tailwind with their turns left —, the guards staying. False when
    /// neither side had one.
    pub fn swap_side_conditions(&mut self) -> bool {
        let p1 = self
            .state
            .side_mut(SideId::P1)
            .conditions
            .take(court_change_swaps);
        let p2 = self
            .state
            .side_mut(SideId::P2)
            .conditions
            .take(court_change_swaps);
        if p1.is_empty() && p2.is_empty() {
            return false;
        }
        self.state.side_mut(SideId::P1).conditions.extend(p2);
        self.state.side_mut(SideId::P2).conditions.extend(p1);
        self.trace.push(TraceEvent::SideConditionsSwapped);
        true
    }
}
