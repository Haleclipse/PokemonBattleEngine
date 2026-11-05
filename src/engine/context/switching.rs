//! `BattleContext`'s switches asked mid-turn (DESIGN.md §30): a drag out and what blocks it, a
//! switch out, a self-switch, Emergency Exit and Wimp Out, a switch cancelled.

use crate::engine::TraceEvent;
use crate::ids::{PokemonId, SideId};
use crate::state::VolatileConditionKind;

use super::*;

impl<'a> BattleContext<'a> {
    /// Whether `side` has a healthy Pokémon on the bench (Showdown `canSwitch`).
    pub fn can_switch(&self, side: SideId) -> bool {
        !self.state.healthy_bench(side).is_empty()
    }

    pub fn is_drag_pending(&self, pokemon: PokemonId) -> bool {
        self.state.drag_queue.contains(&pokemon)
    }

    /// Whether `pokemon` is leaving the field by its own switch, Pursuit striking it as it goes
    /// (Showdown `beingCalledBack`, `BattleState::called_back`).
    pub fn is_called_back(&self, pokemon: PokemonId) -> bool {
        self.state.called_back == Some(pokemon)
    }

    /// Orders `target` out for a random bench Pokémon at the end of the action (Roar, Dragon
    /// Tail, Red Card). Refused when the target is not standing, its side has no bench, or it
    /// is already being dragged. Whether a phazing status move stands: a target held in place
    /// stands it still, but for Commander's.
    pub fn request_drag_out(&mut self, target: PokemonId, source: EffectSource) -> bool {
        let Some(side) = self.side_of(target) else {
            return false;
        };
        if self.is_fainted(target)
            || !self.is_active(target)
            || !self.can_switch(side)
            || self.is_drag_pending(target)
        {
            return false;
        }
        // Showdown `DragOut` event: Suction Cups, Guard Dog, Ingrain and Dynamax hold the target
        // (`null`: the phazing status move still hit — a Gen 4 Mirror Move copies it); only
        // Commander's volatiles refuse (`false`: it fails, `-fail`). A damaging phazer or Red
        // Card just does not drag.
        if let Some(by) = self.drag_out_blocker(target) {
            self.trace.push(TraceEvent::DragOutBlocked { target, by });
            return !matches!(
                by,
                EffectSource::Volatile(
                    VolatileConditionKind::Commanding | VolatileConditionKind::Commanded
                )
            );
        }
        self.state.drag_queue.push(target);
        self.trace
            .push(TraceEvent::DragOutRequested { target, source });
        true
    }

    fn drag_out_blocker(&self, target: PokemonId) -> Option<EffectSource> {
        let pokemon = self.state.pokemon(target)?;
        // Commander's two volatiles (`onDragOut` at priority 2, returning false; DESIGN.md
        // §43.4): neither the Tatsugiri in its ally's mouth nor its Dondozo is dragged out.
        for kind in [
            crate::state::VolatileConditionKind::Commanding,
            crate::state::VolatileConditionKind::Commanded,
        ] {
            if pokemon.volatiles.contains(kind) {
                return Some(EffectSource::Volatile(kind));
            }
        }
        if crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.blocks_drag_out) {
            return Some(EffectSource::Ability {
                pokemon: target,
                ability: crate::engine::raw_ability(pokemon),
            });
        }
        if pokemon.volatiles.contains(VolatileConditionKind::Ingrain) {
            return Some(EffectSource::Volatile(VolatileConditionKind::Ingrain));
        }
        // A Dynamaxed Pokémon is not dragged out (`dynamax.onDragOut`, DESIGN.md §44.5).
        if pokemon.volatiles.contains(VolatileConditionKind::Dynamax) {
            return Some(EffectSource::Volatile(VolatileConditionKind::Dynamax));
        }
        None
    }

    /// Whether any active Pokémon is already ordered out for a chosen replacement (U-turn,
    /// Eject Button).
    pub fn any_switch_out_pending(&self) -> bool {
        [SideId::P1, SideId::P2]
            .into_iter()
            .any(|side| self.state.side(side).switch_out.is_some())
    }

    /// Orders `pokemon` out for a replacement its trainer chooses (the mid-turn ForcedSwitch
    /// request); Eject Button uses it the way U-turn does.
    pub fn request_switch_out(&mut self, pokemon: PokemonId) -> bool {
        let Some(side) = self.side_of(pokemon) else {
            return false;
        };
        if self.is_fainted(pokemon) || !self.is_active(pokemon) || !self.can_switch(side) {
            return false;
        }
        let side = self.state.side_mut(side);
        side.switch_out = Some(pokemon);
        side.passing = None;
        side.switch_out_by_move = false;
        true
    }

    /// A move's order to its own user to leave (Parting Shot): `request_switch_out`, the order
    /// marked as the move's (`SideState::switch_out_by_move`).
    pub fn request_self_switch(&mut self, user: PokemonId) -> bool {
        let ordered = self.request_switch_out(user);
        if ordered && let Some(side) = self.side_of(user) {
            self.state.side_mut(side).switch_out_by_move = true;
        }
        ordered
    }

    /// Whether a Pokémon on the field is ordered out by an item or an ability (Showdown
    /// `switchFlag === true`), not by its own move.
    pub fn chosen_switch_pending(&self) -> bool {
        [SideId::P1, SideId::P2].into_iter().any(|side| {
            let side = self.state.side(side);
            side.switch_out.is_some() && !side.switch_out_by_move
        })
    }

    /// Showdown's `EmergencyExit` event (Emergency Exit, Wimp Out's `onEmergencyExit`): once
    /// `pokemon`'s HP has gone from above half its max (`hp_before`) to half or less while it
    /// stays up, it is ordered out for a replacement its trainer chooses — and every other
    /// chosen switch on the field (a U-turn's, an Eject Button's) is called off. Nothing happens
    /// when its side has no one to send in, it is being dragged out, or it already leaves.
    pub fn emergency_exit(&mut self, pokemon: PokemonId, hp_before: u16) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        let (hp, max_hp) = (holder.hp, holder.max_hp);
        let crossed = crossed_half(hp, max_hp, hp_before);
        self.emergency_exit_crossed(pokemon, crossed);
    }

    /// The residual's own check (`battle.mjs`'s `residual` action): both the HP and the half-max
    /// threshold read through `getUndynamaxedHP` (DESIGN.md §44.5) — unlike the after-move
    /// checks above, which compare the doubled numbers as they stand. `hp_before` is the
    /// undynamaxed HP the residual started with. Whether the ability ordered the holder out.
    pub fn emergency_exit_residual(&mut self, pokemon: PokemonId, hp_before: u16) -> bool {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return false;
        };
        let max_hp = holder.undynamaxed_hp(Some(holder.max_hp));
        let crossed = crossed_half(holder.undynamaxed_hp(None), max_hp, hp_before);
        self.emergency_exit_crossed(pokemon, crossed)
    }

    fn emergency_exit_crossed(&mut self, pokemon: PokemonId, crossed: bool) -> bool {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return false;
        };
        let (side, ability) = (holder.side, crate::engine::raw_ability(holder));
        let exits =
            crate::engine::ability_handler(self.state, self.rules, holder, |b| b.emergency_exit);
        if !crossed
            || !exits
            || !self.is_active(pokemon)
            || !self.can_switch(side)
            || self.is_drag_pending(pokemon)
            || self.state.side(side).switch_out == Some(pokemon)
        {
            return false;
        }
        for each in [SideId::P1, SideId::P2] {
            self.state.side_mut(each).switch_out = None;
        }
        self.state.side_mut(side).switch_out = Some(pokemon);
        self.state.side_mut(side).switch_out_by_move = false;
        self.ability_activated(pokemon, ability);
        true
    }

    pub fn cancel_switch_out(&mut self, pokemon: PokemonId) {
        if let Some(side) = self.side_of(pokemon)
            && self.state.side(side).switch_out == Some(pokemon)
        {
            self.state.side_mut(side).switch_out = None;
        }
    }
}
