//! `BattleEngine`'s lifecycles (DESIGN.md §30.1, ROADMAP §7), one file each: the switches
//! (`switching.rs`), the residual scheduler (`residual.rs`) and each condition's residual
//! (`residual_effects.rs`), the faint step (`faint.rs`). Here, what they share: the speed order
//! with its tie draws, the `Update` event, Gen 3's status berries before a request.

use super::BattleEngine;
use crate::engine::stats::current_handler_speed;
use crate::engine::{BattleContext, PendingFaints, TraceEvent};
use crate::ids::PokemonId;
use crate::state::RandomRequestKind;

impl BattleEngine {
    /// Fastest first — slowest first under Trick Room: Showdown's `insertChoice` refreshes each
    /// entrant's `speed` (`getActionSpeed`) as its `runSwitch` is queued, and the `SwitchIn`
    /// handlers go by it (R-M11-120). Pokémon sharing a speed draw one SpeedTie decision each
    /// (higher first), everyone else draws nothing, so the PRNG only advances on a real tie.
    /// The draws are traced (`SwitchInTiesDrawn`).
    pub(super) fn speed_ordered(&mut self, pokemon: Vec<PokemonId>) -> Vec<PokemonId> {
        let entries = pokemon
            .into_iter()
            .map(|id| (id, current_handler_speed(&self.state, &self.rules, id)))
            .collect();
        self.tie_ordered(entries, |tied| TraceEvent::SwitchInTiesDrawn { tied })
    }

    /// `entries` by their speed, highest first, as `speed_ordered` orders Pokémon: an entry
    /// sharing its speed draws a SpeedTie decision, the draws traced by `traced`.
    pub(super) fn tie_ordered(
        &mut self,
        entries: Vec<(PokemonId, u32)>,
        traced: fn(Vec<(PokemonId, u32)>) -> TraceEvent,
    ) -> Vec<PokemonId> {
        let mut entries: Vec<(PokemonId, u32, u64)> = entries
            .into_iter()
            .map(|(id, speed)| (id, speed, 0))
            .collect();
        // The ties are listed for the trace alone (DESIGN.md §47.5 item 5).
        let listed = self.trace.keeps(crate::engine::TraceCategory::Check);
        let mut tied = Vec::new();
        for index in 0..entries.len() {
            let speed = entries[index].1;
            if entries
                .iter()
                .enumerate()
                .any(|(other, entry)| other != index && entry.1 == speed)
            {
                entries[index].2 = self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
                if listed {
                    tied.push((entries[index].0, speed));
                }
            }
        }
        if !tied.is_empty() {
            self.trace
                .push_with(crate::engine::TraceCategory::Check, || traced(tied));
        }
        entries.sort_by_key(|entry| std::cmp::Reverse((entry.1, entry.2)));
        entries.into_iter().map(|entry| entry.0).collect()
    }

    /// Showdown `eachEvent('Update')` (DESIGN.md §40); where it falls is the generation's
    /// `UpdateRules`.
    pub(super) fn run_update(&mut self, faints: PendingFaints) {
        BattleContext::new(&mut self.state, &self.rules, &mut self.trace).run_update(faints);
    }

    /// A replacement request is about to cut the Gen 1-4 `Update` short: Gen 3 still feeds the
    /// status berries (`BattleContext::feed_status_berries_before_request`, I34).
    pub(super) fn feed_status_berries_before_request(&mut self) {
        BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
            .feed_status_berries_before_request();
    }
}
