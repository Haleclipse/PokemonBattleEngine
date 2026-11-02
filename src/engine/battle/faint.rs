//! `BattleEngine`'s faint step (Showdown `faintMessages`): the knocked-out Pokémon leave, the
//! forme they leave in, Gen 1's faint effect on the turn, the battle decided.

use super::BattleEngine;
use crate::engine::{BattleContext, PendingExecution, TraceEvent};
use crate::ids::{PokemonId, SideId};
use crate::rules::FaintTurnEffect;
use crate::state::{BattleOutcome, BattlePhase};

impl BattleEngine {
    /// The battle is decided (once). A decided battle has nothing left to run: the turn's
    /// remaining actions go with it, wherever the decision came from (a replacement's hazards, a
    /// revive, an action, a rule at the turn's end). `Finished` is only ever idle (DESIGN.md
    /// §11.2).
    pub(super) fn finish(&mut self, outcome: BattleOutcome) {
        if self.state.outcome.is_some() {
            return;
        }
        self.state.outcome = Some(outcome);
        self.set_phase(BattlePhase::Finished);
        self.pending = PendingExecution::Idle;
        self.trace.push(TraceEvent::BattleFinished { outcome });
    }

    /// Showdown `faint`'s `formeRegression` (DESIGN.md §44.3): a Pokémon that Mega Evolved, was
    /// reversed into its Primal forme, took a `-Tera` forme or Terastallized as Morpeko goes
    /// back to the species and ability its set named — the stats, types and max HP of that
    /// forme, the damage it took kept. The details change is silent; an in-battle forme
    /// (`forme`) was never permanent and is cleared by the switch-out itself.
    fn forme_regression(&mut self, pokemon: PokemonId) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        if !holder.forme_regression || holder.transform.is_some() {
            return;
        }
        let Some(base) = holder.set_species else {
            return;
        };
        let Some(forme) = holder.forme_of(base).copied() else {
            return;
        };
        let ability = holder.set_ability;
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.take_forme(&forme);
            holder.species = Some(base);
            holder.forme = None;
            holder.take_ability(ability);
            holder.forme_regression = false;
        }
        self.trace.push(TraceEvent::FormeRegressed {
            pokemon,
            species: base,
        });
    }

    /// Faint step (ROADMAP §7.2): Pokémon leave the field in the order their HP reached 0
    /// (`BattleState::faint_queue`), then the outcome is checked. A double knock-out of both
    /// sides' last Pokémon is a draw before Gen 5 and, from Gen 5, a win for the side whose
    /// Pokémon fainted last (Showdown `checkWin`).
    /// Processes the faint queue (Showdown `faintMessages`); returns whether anyone fainted.
    pub(super) fn resolve_faints(&mut self) -> bool {
        // Anything at 0 HP that never went through a DamageRequest (defensive).
        for position in self.rules.topology.positions() {
            if let Some(pokemon) = self.state.side(position.side).active[position.slot as usize]
                && self
                    .state
                    .pokemon(pokemon)
                    .is_some_and(|pokemon| pokemon.is_fainted())
                && !self.state.faint_queue.contains(&pokemon)
            {
                self.state.faint_queue.push(pokemon);
            }
        }

        // Showdown `faintMessages` takes the queue one entry at a time, so a faint handler sees
        // the faints still to come as queued (`foePokemonLeft`); `AfterFaint` gets the last
        // entry and how many the queue held. Faints announced at a hit loop's end are taken here
        // in the order Showdown took them there.
        self.state.announced_faints.clear();
        // The batch's `AfterFaint` leaves out the faints whose own batch ran it at a hit loop's
        // end (`after_faint_run`, R-M10-52): those were that `faintMessages`'s.
        let mut length = 0;
        let mut last_cause = None;
        let mut last_fainted = None;
        let mut any_fainted = false;
        // Air Lock / Cloud Nine's `onEnd` as their holder faints: the weather counts again — a
        // change that ability's `End` makes (`eachEvent('WeatherChange', this.effect)`, its
        // `suppressWeather`), not a fresh one: Ice Face does not answer it (R-M10-82).
        let mut weather_back = None;
        while !self.state.faint_queue.is_empty() {
            let pokemon = self.state.faint_queue.remove(0);
            let cause = self
                .state
                .faint_causes
                .iter()
                .position(|cause| cause.target == pokemon)
                .map(|index| self.state.faint_causes.remove(index))
                .unwrap_or(crate::state::FaintCause {
                    target: pokemon,
                    source: None,
                    by_move: false,
                    future_move: false,
                    listeners_run: false,
                    bonds_run: false,
                    after_faint_run: false,
                    counted: false,
                });
            if !cause.after_faint_run {
                last_cause = Some(cause);
                length += 1;
            }
            let Some(position) = self.state.active_position(pokemon, &self.rules.topology) else {
                continue;
            };
            let Some(spec) = self.rules.topology.spec(position) else {
                continue;
            };
            if self.state.pokemon(pokemon).is_some_and(|p| {
                crate::engine::ability_handler(&self.state, &self.rules, p, |b| {
                    b.suppresses_weather
                })
            }) {
                weather_back = Some(pokemon);
            }
            self.trace.push(TraceEvent::Fainted { pokemon, position });
            // Showdown `faintMessages`: `totalFainted++` first — the Supreme Overlord a fainting
            // gas holder's `End` lets start again counts it — unless the hit loop's end counted
            // it already (`counted`, R-M10-61).
            if !cause.counted {
                let side = self.state.side_mut(spec.side);
                side.total_fainted = side.total_fainted.saturating_add(1).min(100);
            }
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            ctx.faint_event(cause);
            // Showdown `faintMessages`: the `Faint` event, then the ability's `End`.
            ctx.end_ability(pokemon, crate::engine::AbilityEndCause::Fainted);
            // `formeRegression` (DESIGN.md §44.3): a forme an item or the Terastallization gave
            // goes back to the set's species and ability before the volatiles clear.
            self.forme_regression(pokemon);
            self.clear_on_switch(pokemon);
            // Showdown `faint`: `delete pokemon.terastallized` — a fainted Pokémon is no
            // longer Terastallized (its `-Tera` forme already went back above).
            if let Some(holder) = self.state.pokemon_mut(pokemon) {
                holder.terastallized = None;
                holder.stellar_boosted_types.clear();
            }
            let side = self.state.side_mut(spec.side);
            side.active[spec.slot as usize] = None;
            side.fallen[spec.slot as usize] = Some(pokemon);
            // Showdown `faint()` drops the switch-out order: a U-turn user knocked out by its
            // Life Orb (or Rough Skin) is replaced like any fainted Pokémon, not mid-turn.
            if side.switch_out == Some(pokemon) {
                side.switch_out = None;
            }
            side.fainted_this_turn = true;
            last_fainted = Some(spec.side);
            any_fainted = true;
        }
        if let Some(holder) = weather_back {
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .weather_changed_leaving(Some(holder));
        }
        if any_fainted {
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .wipe_gen1_bide_damage();
        }
        if any_fainted && self.rules.mode() == crate::spec::BattleMode::Singles {
            self.apply_singles_faint_effect();
        }

        let p1_alive = !self.state.living_team_members(SideId::P1).is_empty();
        let p2_alive = !self.state.living_team_members(SideId::P2).is_empty();
        let outcome = match (p1_alive, p2_alive) {
            (false, false) => Some(match last_fainted {
                Some(side) if self.rules.faint.last_to_faint_wins_double_ko => {
                    BattleOutcome::Winner(side)
                }
                _ => BattleOutcome::Draw,
            }),
            (false, true) => Some(BattleOutcome::Winner(SideId::P2)),
            (true, false) => Some(BattleOutcome::Winner(SideId::P1)),
            (true, true) => None,
        };

        if let Some(outcome) = outcome {
            self.finish(outcome);
        }

        // `AfterFaint`, even once the battle is decided: the hit loop's `faintMessages` skips
        // the win check while the move's user stands, so a user whose move took the last foe
        // down still gets it (Gen 5 Moxie). Elsewhere Showdown returns first, but every handler
        // so far wants a move's knock-out by a user still standing.
        if let Some(cause) = last_cause {
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .after_faint(cause, length);
        }
        any_fainted
    }

    /// Showdown `faintMessages` in a Gen 1-3 singles battle: a faint cuts the rest of the
    /// turn short — Gen 2 cancels the remaining moves, Gen 3 every remaining action, Gen 1
    /// the whole rest of the turn.
    fn apply_singles_faint_effect(&mut self) {
        let effect = self.rules.faint.singles_faint_effect;
        let crate::engine::PendingExecution::Turn {
            actions, residual, ..
        } = &mut self.pending
        else {
            return;
        };
        let before: Vec<PokemonId> = actions.iter().map(|action| action.actor).collect();
        match effect {
            FaintTurnEffect::None => return,
            FaintTurnEffect::CancelMoves => {
                actions.retain(|action| {
                    !matches!(action.choice, crate::engine::ActionChoice::Move { .. })
                });
            }
            FaintTurnEffect::CancelActions => actions.clear(),
            FaintTurnEffect::SkipTurn => {
                actions.clear();
                *residual = false;
            }
        }
        let kept: Vec<PokemonId> = actions.iter().map(|action| action.actor).collect();
        let cancelled: Vec<PokemonId> = before
            .into_iter()
            .filter(|actor| !kept.contains(actor))
            .collect();
        if cancelled.is_empty() {
            return;
        }
        self.state
            .pending_movers
            .retain(|mover| !cancelled.contains(mover));
        self.trace
            .push(TraceEvent::ActionsCancelledByFaint { actors: cancelled });
    }
}
