//! `BattleContext`'s side of a user fainting by its own move (DESIGN.md §30.4): the faint itself
//! and Damp.

use crate::ids::{MoveId, PokemonId, SideId};

use super::BattleContext;
use crate::engine::{EffectSource, TraceEvent};

impl<'a> BattleContext<'a> {
    /// Showdown `battle.faint(pokemon)` (a `selfdestruct` move, Final Gambit's
    /// `damageCallback`): the Pokémon's HP goes to 0 at once — no Substitute, Sturdy or Focus
    /// Sash in the way — and it joins the faint queue; it leaves the field with the others.
    pub fn faint_by_own_move(&mut self, pokemon: PokemonId, move_id: MoveId) {
        self.faint_directly(pokemon, EffectSource::Move { pokemon, move_id });
    }

    /// Showdown `pokemon.faint()` from anything (a `selfdestruct` move, Perish Song's count
    /// reaching 0): the HP goes to 0 with no damage dealt — Showdown logs no `-damage`, only the
    /// `faint` message with the others — and the Pokémon joins the faint queue.
    pub fn faint_directly(&mut self, pokemon: PokemonId, source: EffectSource) {
        let Some(state) = self.state.pokemon_mut(pokemon).filter(|p| !p.is_fainted()) else {
            return;
        };
        state.hp = 0;
        if !self.state.faint_queue.contains(&pokemon) {
            self.state.faint_queue.push(pokemon);
        }
        self.trace
            .push(TraceEvent::FaintedDirectly { pokemon, source });
    }

    /// Perish Song's count on `pokemon` (`addVolatile('perishsong')`): false for a Pokémon down
    /// or already counting.
    pub fn start_perish_song(&mut self, pokemon: PokemonId) -> bool {
        let started = self
            .state
            .pokemon_mut(pokemon)
            .filter(|p| !p.is_fainted())
            .is_some_and(|p| p.volatiles.set_perish_song());
        if started {
            self.trace.push(TraceEvent::VolatileStarted {
                target: pokemon,
                kind: crate::state::VolatileConditionKind::PerishSong,
            });
        }
        started
    }

    /// Whether `pokemon` is counting down to Perish Song.
    pub fn perishing(&self, pokemon: PokemonId) -> bool {
        self.state.pokemon(pokemon).is_some_and(|p| {
            p.volatiles
                .contains(crate::state::VolatileConditionKind::PerishSong)
        })
    }

    /// Whether an active Pokémon's Damp is in force (Showdown `onAny…` handlers: the active,
    /// not fainted Pokémon).
    pub fn damp_active(&self) -> bool {
        damp_holder(self.state, self.rules).is_some()
    }
}

/// The active Pokémon whose Damp stops the move, if any: Showdown's `runEvent('TryMove')` runs the
/// `onAnyTryMove` handlers by their holder's speed (`speedSort`), so the fastest holder's line
/// names it — the user's own Damp too, after a faster one's (R-M11-35). Holders tied in speed
/// keep the field's order (Showdown shuffles them, which the engine does not draw for).
pub(crate) fn damp_holder(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
) -> Option<PokemonId> {
    [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().flatten().copied())
        .filter(|id| {
            state.pokemon(*id).is_some_and(|p| {
                !p.is_fainted() && crate::engine::ability_handler(state, rules, p, |b| b.damp)
            })
        })
        .min_by_key(|id| std::cmp::Reverse(crate::engine::stats::handler_speed(state, *id)))
}
