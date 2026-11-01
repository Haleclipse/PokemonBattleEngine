//! Trace (DESIGN.md §41.3, K.4b): Showdown's `onStart` / `onUpdate` with `abilityState.seek`,
//! which the Gen 3-4 cartridges match once their oracle patches are in (issues.md I83, I84): the
//! holder looks for a foe whose ability it can copy as it comes in and at every `Update` until
//! it copies one.

use super::BattleContext;
use crate::ids::PokemonId;
use crate::state::RandomRequestKind;

impl BattleContext<'_> {
    /// Trace's `onStart`: the holder starts looking (`seek`) and looks at once — unless it holds
    /// Ability Shield, which says so (`-block|holder|item: Ability Shield`, R-M10-55: it would
    /// refuse the ability anyway).
    pub fn start_trace(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let shielded = crate::engine::has_item(
            self.state,
            self.rules,
            pokemon,
            crate::ids::ItemId::ABILITY_SHIELD,
        );
        if shielded {
            self.item_activated(holder, crate::ids::ItemId::ABILITY_SHIELD);
        }
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.trace_seek = !shielded;
        }
        self.trace_ability(holder);
    }

    /// Trace's `onUpdate` while it looks (`seek`): among the foes it could copy
    /// (`engine::trace_candidates`) — one, or a `Sample` among two or more — it takes the
    /// ability (`setAbility`, which starts it as the generation has it; Gen 4 at once, as the
    /// switch-in check Trace runs in goes on to the holder: the holder's part of
    /// `announce_unannounced` — a foe whose switch-in comes next starts its own then). None: it
    /// looks again at the next `Update`.
    pub(crate) fn trace_ability(&mut self, holder: PokemonId) {
        if !self
            .state
            .pokemon(holder)
            .is_some_and(|pokemon| pokemon.ability_state.trace_seek)
        {
            return;
        }
        let candidates = crate::engine::trace_candidates(self.state, self.rules, holder);
        let target = match candidates.len() {
            0 => return,
            1 => candidates[0],
            count => {
                let index = self.state.rng.range_inclusive_for(
                    RandomRequestKind::Sample,
                    0,
                    count as u16 - 1,
                );
                candidates[usize::from(index)]
            }
        };
        let Some(ability) = self.raw_ability_of(target) else {
            return;
        };
        if self
            .set_ability(holder, ability, Some(target), None)
            .is_some()
            && self.rules.conditions.ability_change.start_new
                == crate::rules::StartNewAbility::Unannounced
        {
            self.announce_if_unannounced(holder);
        }
    }
}
