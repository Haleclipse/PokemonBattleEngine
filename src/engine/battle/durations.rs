//! The residual's duration-only handlers (DESIGN.md §31.1, R190): Showdown's
//! `fieldEvent('Residual')` collects, besides the handlers with a callback, one handler for each
//! effect that only has a `duration` — a Pokémon's charge or Protect, a side's screen, the
//! field's Trick Room or terrain — and sorts them all together. Such a handler counts its
//! effect down; one that runs out calls `end` and moves on without `faintMessages()`, one that
//! goes on is followed by it. So when a Perish Song count leaves its faints waiting, the first
//! duration that does not run out settles them, and a battle decided there never counts the
//! later ones down.

use crate::ids::{PokemonId, SideId};
use crate::rules::{DurationEffect, UNORDERED_SLOT_SUB_ORDER, UNORDERED_VOLATILE_SUB_ORDER};
use crate::state::{
    FieldConditionKind, SideCondition, SideConditionKind, SlotHealKind, VolatileConditionKind,
};

use super::BattleEngine;
use super::lifecycle::ResidualRun;
use crate::engine::{BattleContext, TraceEvent};

/// One effect that only counts down in the residual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationHandler {
    Volatile(PokemonId, VolatileConditionKind),
    /// A hiding two-turn move's own condition (`fly`, `dig` …) beside its `twoturnmove`.
    ChargeHiding(PokemonId),
    Side(SideId, SideConditionKind),
    Field(FieldConditionKind),
    Terrain,
    /// Gen 4-7 Healing Wish / Lunar Dance waiting on a slot (`side`, `slot`).
    SlotHeal(SideId, usize, SlotHealKind),
}

/// A duration handler with its place in the residual (`comparePriority`'s order, speed and
/// sub-order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DurationInstance {
    pub handler: DurationHandler,
    pub order: u8,
    pub sub_order: u8,
    pub speed: u32,
}

impl BattleEngine {
    /// The residual's duration-only handlers (`ResidualRules::durations`; none in Gen 1): each
    /// active Pokémon's counting volatiles (no order, its speed, sub-order 2), then each side's
    /// conditions and the field's by the generation's keys (speed 0). A side's screens and
    /// Tailwind already counted down before the residual (Gen 3-4) are not among them. Handlers
    /// equal on every key — with each other or with a handler that has a callback — are
    /// shuffled as Showdown's `speedSort` does (`SpeedTie` decisions, `residual_tie_groups`); no
    /// frame shows their order unless the battle ends between them.
    pub(super) fn collect_durations(&self) -> Vec<DurationInstance> {
        let rules = self.rules.residual;
        if rules.durations.is_none() {
            return Vec::new();
        }
        let mut handlers = Vec::new();
        for holder in self.active_pokemon() {
            let Some(pokemon) = self.state.pokemon(holder) else {
                continue;
            };
            let speed = self.residual_speed(holder);
            // Thrash's lock is a handler of its own where the generation orders it (Gen 3-4,
            // `ResidualKind::Rampage`), and Gen 1-2 count it in uses, not turns.
            let rampage_apart = rules.key(crate::rules::ResidualKind::Rampage).is_some()
                || self.rules.conditions.rampage.counter.counts_uses();
            for kind in pokemon
                .volatiles
                .counting_down()
                .into_iter()
                .filter(|kind| !(rampage_apart && *kind == VolatileConditionKind::Rampage))
            {
                handlers.push(DurationInstance {
                    handler: DurationHandler::Volatile(holder, kind),
                    order: u8::MAX,
                    sub_order: UNORDERED_VOLATILE_SUB_ORDER,
                    speed,
                });
            }
            if pokemon.volatiles.charge_hiding().is_some() {
                handlers.push(DurationInstance {
                    handler: DurationHandler::ChargeHiding(holder),
                    order: u8::MAX,
                    sub_order: UNORDERED_VOLATILE_SUB_ORDER,
                    speed,
                });
            }
        }
        let screens_first = self.rules.conditions.side_protections.tick_first;
        let tailwind_first = self.rules.conditions.tailwind_ticks_first;
        for side in [SideId::P1, SideId::P2] {
            for entry in self.state.side(side).conditions.entries() {
                let counted_before = match entry {
                    SideCondition::Timed { .. } => screens_first,
                    SideCondition::Tailwind { .. } => tailwind_first,
                    _ => false,
                };
                let kind = entry.kind();
                if counted_before || !self.state.side(side).conditions.counts_down(kind) {
                    continue;
                }
                let (order, sub_order) = rules.duration_key(DurationEffect::Side(kind));
                handlers.push(DurationInstance {
                    handler: DurationHandler::Side(side, kind),
                    order,
                    sub_order,
                    speed: 0,
                });
            }
        }
        for condition in self.state.field.conditions() {
            let kind = condition.kind();
            let (order, sub_order) = rules.duration_key(DurationEffect::Field(kind));
            handlers.push(DurationInstance {
                handler: DurationHandler::Field(kind),
                order,
                sub_order,
                speed: 0,
            });
        }
        if self.state.field.terrain.is_some() {
            let (order, sub_order) = rules.duration_key(DurationEffect::Terrain);
            handlers.push(DurationInstance {
                handler: DurationHandler::Terrain,
                order,
                sub_order,
                speed: 0,
            });
        }
        // Gen 4-7 Healing Wish / Lunar Dance (a slot condition with a `duration`): a handler of
        // the Pokémon in the slot — fainted or not (`isSlotCondition`) — at its speed, no order,
        // sub-order 3. Its count going on settles the faints a Perish Song count left waiting.
        for side in [SideId::P1, SideId::P2] {
            for heal in &self.state.side(side).slot_heals {
                if heal.turns_remaining.is_none() {
                    continue;
                }
                // The engine empties a fainted Pokémon's slot at once; Showdown keeps it there
                // until the replacement, so the heal's user holds the handler meanwhile.
                let occupant = self
                    .state
                    .side(side)
                    .active
                    .get(heal.slot)
                    .copied()
                    .flatten()
                    .unwrap_or(heal.user);
                handlers.push(DurationInstance {
                    handler: DurationHandler::SlotHeal(side, heal.slot, heal.kind),
                    order: u8::MAX,
                    sub_order: UNORDERED_SLOT_SUB_ORDER,
                    speed: self.residual_speed(occupant),
                });
            }
        }
        handlers
    }

    /// Counts one handler's effect down: `Ended` when it ran out (its `end` instead of the
    /// settling), `Ran` while it goes on; `None` when there is no handler any more — its holder
    /// has fainted, or a prior handler removed the effect (Showdown `fieldEvent`'s checks).
    pub(super) fn run_duration(&mut self, handler: DurationHandler) -> Option<ResidualRun> {
        let ended = match handler {
            DurationHandler::Volatile(holder, VolatileConditionKind::Rampage) => {
                self.state.active_position(holder, &self.rules.topology)?;
                return self.rampage_duration(holder);
            }
            DurationHandler::Volatile(holder, kind) => {
                self.state.active_position(holder, &self.rules.topology)?;
                let ended = self.state.pokemon_mut(holder)?.volatiles.count_down(kind)?;
                if ended {
                    // A `-singleturn` one ends with no line: the observers see it gone as the turn
                    // ends — not where the battle ends in it (R-M10-44).
                    let seen = !self
                        .rules
                        .conditions
                        .volatile_visibility
                        .for_the_turn
                        .contains(kind);
                    self.trace.push(TraceEvent::VolatileEnded {
                        target: holder,
                        kind,
                        seen,
                    });
                }
                ended
            }
            // Silent: its end only takes the user out of hiding.
            DurationHandler::ChargeHiding(holder) => {
                self.state.active_position(holder, &self.rules.topology)?;
                self.state
                    .pokemon_mut(holder)?
                    .volatiles
                    .count_down_charge_hiding()?
            }
            DurationHandler::Side(side, kind) => {
                let ended = self.state.side_mut(side).conditions.count_down(kind)?;
                if ended {
                    self.trace
                        .push(TraceEvent::SideConditionEnded { side, kind });
                }
                ended
            }
            DurationHandler::Field(kind) => {
                let ended = self.state.field.count_down_condition(kind)?;
                if ended {
                    self.trace.push(TraceEvent::FieldConditionEnded { kind });
                }
                ended
            }
            DurationHandler::SlotHeal(side, slot, kind) => {
                let heals = &mut self.state.side_mut(side).slot_heals;
                let index = heals
                    .iter()
                    .position(|heal| heal.slot == slot && heal.kind == kind)?;
                let turns = heals[index].turns_remaining.as_mut()?;
                *turns = turns.saturating_sub(1);
                let ended = *turns == 0;
                if ended {
                    heals.remove(index);
                }
                ended
            }
            DurationHandler::Terrain => {
                let (terrain, ended) = self.state.field.count_down_terrain()?;
                if ended {
                    self.trace.push(TraceEvent::TerrainChanged {
                        from: Some(terrain),
                        to: None,
                    });
                    // Showdown `clearTerrain` → `eachEvent('TerrainChange')` (Quark Drive).
                    BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                        .terrain_changed();
                }
                ended
            }
        };
        Some(if ended {
            ResidualRun::Ended
        } else {
            ResidualRun::Ran
        })
    }

    /// Gen 5+ Thrash's lock in the residual (Showdown `lockedmove`: a `duration` with an
    /// `onResidual`): its `duration` counts down — at 0 the lock ends, confusing its user on its
    /// last turn (`onEnd`); otherwise an asleep user loses it silently, and the hidden length
    /// counts down (`onResidual`).
    fn rampage_duration(&mut self, holder: PokemonId) -> Option<ResidualRun> {
        let rampage = self.state.pokemon_mut(holder)?.volatiles.rampage_mut()?;
        rampage.turns = rampage.turns.saturating_sub(1);
        if rampage.turns == 0 {
            let confuse = rampage.left <= 1;
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .end_rampage(holder, confuse);
            return Some(ResidualRun::Ended);
        }
        rampage.left = rampage.left.saturating_sub(1);
        if self
            .state
            .pokemon(holder)
            .is_some_and(|p| p.has_status(crate::state::NonVolatileStatus::Sleep))
        {
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .end_rampage(holder, false);
        }
        Some(ResidualRun::Ran)
    }

    /// Gen 3-4 Thrash's lock at its place in the residual (the cartridge's `ENDTURN_THRASH`,
    /// `MON_COND_CHECK_STATE_THRASH`; issues.md I61): the count goes down; at 0 the lock ends
    /// and confuses its user, unless the turn failed (or, Gen 3, the user is asleep or frozen);
    /// otherwise a failed turn, or an asleep user (Gen 3: or a frozen one), loses it silently.
    pub(super) fn run_rampage_residual(&mut self, holder: PokemonId) -> ResidualRun {
        let rules = self.rules.conditions.rampage;
        let Some(pokemon) = self.state.pokemon(holder) else {
            return ResidualRun::Ran;
        };
        let asleep = pokemon.has_status(crate::state::NonVolatileStatus::Sleep);
        let frozen = pokemon.has_status(crate::state::NonVolatileStatus::Freeze);
        let Some(rampage) = self
            .state
            .pokemon_mut(holder)
            .and_then(|p| p.volatiles.rampage_mut())
        else {
            return ResidualRun::Ran;
        };
        rampage.turns = rampage.turns.saturating_sub(1);
        let cancelled = rampage.failed || (rules.frozen_or_asleep_cancels && (asleep || frozen));
        let ends = rampage.turns == 0;
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        if ends {
            ctx.end_rampage(holder, !cancelled);
            return ResidualRun::Ended;
        }
        if cancelled || asleep {
            ctx.end_rampage(holder, false);
        }
        ResidualRun::Ran
    }

    /// Uproar's turn-end step (§15.12). Gen 3-4 (the cartridge: pokeemerald `ENDTURN_UPROAR`,
    /// pokeplatinum `MON_COND_CHECK_STATE_UPROAR`; issues.md I63): every active Pokémon asleep
    /// wakes (a Soundproof one stays asleep), then the count goes down; at 0, or after a turn
    /// its move failed, the uproar ends. Gen 5+ (Showdown's `duration`, then `onResidual`): the
    /// count goes down — at 0 the uproar ends (`end`, no settling after it) — and Throat Chop
    /// ends it.
    pub(super) fn run_uproar(&mut self, holder: PokemonId) -> ResidualRun {
        let cartridge = self.rules.conditions.uproar.wake == crate::rules::UproarWake::Cartridge;
        if cartridge {
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .uproar_wakes_at_turn_end();
        }
        let Some(pokemon) = self.state.pokemon_mut(holder) else {
            return ResidualRun::Ran;
        };
        let throat_chopped = pokemon
            .volatiles
            .contains(VolatileConditionKind::ThroatChop);
        let Some(uproar) = pokemon.volatiles.uproar_mut() else {
            return ResidualRun::Ran;
        };
        uproar.turns = uproar.turns.saturating_sub(1);
        let (ran_out, failed) = (uproar.turns == 0, uproar.failed);
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        if ran_out && !cartridge {
            ctx.end_uproar(holder);
            return ResidualRun::Ended;
        }
        if ran_out || failed || (!cartridge && throat_chopped) {
            ctx.end_uproar(holder);
        }
        ResidualRun::Ran
    }
}
