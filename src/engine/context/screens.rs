//! `BattleContext`'s side protections (DESIGN.md §10.7, §10.8): the screens (Reflect, Light
//! Screen, Aurora Veil), Mist, Safeguard and Lucky Chant going up, and the screens coming down
//! before their time (Brick Break's `onTryHit`, Screen Cleaner's `onStart`).

use crate::ids::{ItemId, PokemonId, SideId};
use crate::rules::ResolvedTarget;
use crate::state::{SideConditionKind, VolatileConditionKind};

use super::BattleContext;
use crate::engine::{ConditionApplyError, TraceEvent};

/// The side conditions a screen breaker or Screen Cleaner takes down, in Showdown's order.
const SCREENS: [SideConditionKind; 3] = [
    SideConditionKind::Reflect,
    SideConditionKind::LightScreen,
    SideConditionKind::AuroraVeil,
];

impl<'a> BattleContext<'a> {
    /// A screen / Mist / Safeguard / Lucky Chant move's effect where it aims: the side condition
    /// on a side for `SideProtectionRules::turns` — a screen `light_clay_turns` when the user
    /// holds Light Clay (Showdown `durationCallback`) — or, aimed at the user itself (Gen 1
    /// screens, Gen 1-2 Mist), its volatile. Fails when it is already up.
    pub fn set_side_protection(
        &mut self,
        user: PokemonId,
        targets: &[ResolvedTarget],
        kind: SideConditionKind,
    ) -> Result<(), ConditionApplyError> {
        if let Some(side) = Self::target_side(targets) {
            let turns = if SCREENS.contains(&kind)
                && crate::engine::held_item_id(self.state, self.rules, user)
                    == Some(ItemId::LIGHT_CLAY)
            {
                self.rules.conditions.screens.light_clay_turns
            } else {
                self.rules.conditions.side_protections.turns
            };
            if !self.state.side_mut(side).conditions.set_timed(kind, turns) {
                return Err(ConditionApplyError::AlreadyActive);
            }
            self.side_condition_started(side, kind);
            return Ok(());
        }
        let volatile = match kind {
            SideConditionKind::Reflect => VolatileConditionKind::Reflect,
            SideConditionKind::LightScreen => VolatileConditionKind::LightScreen,
            SideConditionKind::Mist => VolatileConditionKind::Mist,
            _ => return Err(ConditionApplyError::InvalidTarget),
        };
        let pokemon = self
            .state
            .pokemon_mut(user)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if !pokemon.volatiles.set_user_guard(volatile) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target: user,
            kind: volatile,
        });
        Ok(())
    }

    /// Brick Break / Psychic Fangs / Raging Bull (`onTryHit`, as the hit lands, before a
    /// Substitute can take it): the screens of the target's side end — Gen 3: of the user's
    /// foes (`ScreenRules::broken_on_user_foes`).
    pub fn break_screens(&mut self, user: PokemonId, target: PokemonId) {
        let side = if self.rules.conditions.screens.broken_on_user_foes {
            self.state.pokemon(user).map(|p| p.side.other())
        } else {
            self.state.pokemon(target).map(|p| p.side)
        };
        if let Some(side) = side {
            for kind in SCREENS {
                self.remove_side_condition(side, kind);
            }
        }
    }

    /// Whether either side has a screen up (Screen Cleaner announces itself only then).
    pub fn any_screen(&self) -> bool {
        [SideId::P1, SideId::P2].into_iter().any(|side| {
            SCREENS
                .into_iter()
                .any(|kind| self.state.side(side).conditions.contains(kind))
        })
    }

    /// Screen Cleaner: every screen on both sides ends, each kind on `first` (the holder's side)
    /// before the other.
    pub fn clear_screens(&mut self, first: SideId) {
        for kind in SCREENS {
            for side in [first, first.other()] {
                self.remove_side_condition(side, kind);
            }
        }
    }
}
