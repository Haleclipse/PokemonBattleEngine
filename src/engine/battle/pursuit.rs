//! Pursuit (DESIGN.md §15.11): its `beforeTurnCallback` marks the foes as the turn begins, and
//! its volatile's `onBeforeSwitchOut` strikes a marked Pokémon as it leaves — a chosen switch
//! (`switchIn`), or a U-turn user, Eject Button holder … before the replacement request
//! (`runAction`); never a Baton Pass or Shed Tail user (`skipBeforeSwitchOutEventFlag`), nor a
//! Pokémon dragged out.

use crate::ids::{MoveSlot, PokemonId};
use crate::rules::{DeclaredTarget, PursuitDestinyBond, ResolvedTarget};
use crate::state::{NonVolatileStatus, RandomRequestKind, VolatileConditionKind};

use super::BattleEngine;
use crate::engine::{ActionChoice, BattleContext, BattleError, QueuedAction, TraceEvent};

impl BattleEngine {
    /// Pursuit's `beforeTurnCallback`, in action order: each Pokémon about to use it marks every
    /// foe on the field (Gen 2-3: the one it chose) — Gen 3-4 not while asleep or frozen.
    pub(super) fn mark_pursuit_targets(&mut self, actions: &[QueuedAction]) {
        let rules = self.rules.conditions.switching.pursuit;
        let mut marks: Vec<(PokemonId, PokemonId)> = Vec::new();
        for action in actions {
            let ActionChoice::Move { slot, target, .. } = action.choice else {
                continue;
            };
            let actor = action.actor;
            let Some(pokemon) = self.state.pokemon(actor) else {
                continue;
            };
            if self.state.struggling.contains(&actor) {
                continue;
            }
            // A Z-Pursuit or a Max Move marks nobody (`beforeTurnMove` is queued only when
            // `!action.maxMove && !action.zmove`, R-M9-30).
            if self
                .state
                .queued_moves
                .iter()
                .any(|queued| queued.actor == actor && (queued.z_powered || queued.max_move))
            {
                continue;
            }
            // The move the action uses (Showdown `chooseMove` takes `getLockedMove`): a recharge,
            // a charge's second turn, a rampage … in place of the slot named, so a recharging
            // Pokémon that named Pursuit marks nobody (R-M8-105).
            if pokemon
                .volatiles
                .contains(VolatileConditionKind::MustRecharge)
            {
                continue;
            }
            let Some(move_def) = pokemon
                .locked_move()
                .or_else(|| pokemon.move_slot(slot).map(|slot| slot.move_id))
                .and_then(|move_id| self.dex.move_by_id(move_id))
                .filter(|move_def| move_def.behavior.marks_pursuit)
            else {
                continue;
            };
            if rules.stopped_pursuer_marks_nobody
                && (pokemon.has_status(NonVolatileStatus::Sleep)
                    || pokemon.has_status(NonVolatileStatus::Freeze)
                    || crate::engine::context::truant_loafs_next(&self.state, &self.rules, pokemon))
            {
                continue;
            }
            let side = pokemon.side;
            let foes: Vec<PokemonId> = if rules.marks_every_foe {
                self.state
                    .side(side.other())
                    .active
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|foe| self.state.pokemon(*foe).is_some_and(|p| !p.is_fainted()))
                    .collect()
            } else {
                self.rules
                    .targeting
                    .resolve(
                        &self.rules.topology,
                        &self.state,
                        actor,
                        move_def.target,
                        target,
                    )
                    .ok()
                    .and_then(|targets| match targets.targets.first() {
                        Some(ResolvedTarget::Pokemon(chosen)) => Some(*chosen),
                        _ => None,
                    })
                    .filter(|chosen| self.state.pokemon(*chosen).is_some_and(|p| p.side != side))
                    .into_iter()
                    .collect()
            };
            marks.extend(foes.into_iter().map(|foe| (foe, actor)));
        }
        for (foe, source) in marks {
            let fresh = self
                .state
                .pokemon(foe)
                .is_some_and(|p| !p.volatiles.contains(VolatileConditionKind::Pursuit));
            if self
                .state
                .pokemon_mut(foe)
                .is_some_and(|p| p.volatiles.mark_pursuit(source))
                && fresh
            {
                self.trace.push(TraceEvent::VolatileStarted {
                    target: foe,
                    kind: VolatileConditionKind::Pursuit,
                });
            }
        }
    }

    /// A U-turn user, Eject Button holder … still to leave whose `BeforeSwitchOut` has a
    /// listener: a pursuer marked it, or it is Dynamaxed. Showdown `runAction` runs that event for
    /// every `switchFlag` Pokémon before the replacement request (`skipBeforeSwitchOutEventFlag`
    /// then spares its switch), as a step of its own.
    pub(super) fn before_switch_out_due(&self) -> Option<PokemonId> {
        [crate::ids::SideId::P1, crate::ids::SideId::P2]
            .into_iter()
            .find_map(|side| {
                let state = self.state.side(side);
                let outgoing = state.switch_out?;
                (state.passing.is_none()
                    && !self.state.skip_before_switch_out.contains(&outgoing)
                    && !self.state.healthy_bench(side).is_empty()
                    && self.state.pokemon(outgoing).is_some_and(|p| {
                        !p.is_fainted()
                            && (p.volatiles.contains(VolatileConditionKind::Pursuit)
                                || p.volatiles.contains(VolatileConditionKind::Dynamax))
                    }))
                .then_some(outgoing)
            })
    }

    /// Showdown `BeforeSwitchOut` on `outgoing`, in its listeners' order: Pursuit's, then
    /// Dynamax's (priority −1: the Dynamax ends, DESIGN.md R-M9-21). The two never meet (Pursuit
    /// is gone by Gen 8). Faints are the caller's.
    pub(super) fn before_switch_out(&mut self, outgoing: PokemonId) -> Result<(), BattleError> {
        self.pursuit_before_switch_out(outgoing)?;
        if self
            .state
            .pokemon(outgoing)
            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Dynamax))
        {
            self.end_dynamax(outgoing);
        }
        Ok(())
    }

    /// Pursuit's `onBeforeSwitchOut` on `outgoing`: Gen 5+ ends its Destiny Bond; then each
    /// pursuer, in marking order, whose move is still queued (`queue.cancelMove`, which takes
    /// it off the queue) and that stands — Gen 5+ also adjacent — uses Pursuit on it at once,
    /// at double power and (Gen 3+) never missing. Gen 2 ends the Destiny Bond when a pursuer is
    /// slower. The mark stays until the turn ends.
    fn pursuit_before_switch_out(&mut self, outgoing: PokemonId) -> Result<(), BattleError> {
        let pursuers = match self.state.pokemon(outgoing) {
            Some(pokemon) if pokemon.volatiles.contains(VolatileConditionKind::Pursuit) => {
                pokemon.volatiles.pursuers()
            }
            _ => return Ok(()),
        };
        let rules = self.rules.conditions.switching.pursuit;
        if rules.destiny_bond == PursuitDestinyBond::Ends {
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .end_volatile(outgoing, VolatileConditionKind::DestinyBond);
        }
        self.state.called_back = Some(outgoing);
        for source in pursuers {
            if rules.destiny_bond == PursuitDestinyBond::EndsWhenOutsped
                && self
                    .state
                    .pokemon(outgoing)
                    .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::DestinyBond))
            {
                let pursuer = crate::engine::stats::handler_speed(&self.state, source);
                let leaving = crate::engine::stats::handler_speed(&self.state, outgoing);
                let outsped = pursuer < leaving
                    || (pursuer == leaving
                        && self
                            .state
                            .rng
                            .roll_ratio_for(RandomRequestKind::SpeedTie, 1, 2));
                if outsped {
                    BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                        .end_volatile(outgoing, VolatileConditionKind::DestinyBond);
                }
            }
            if rules.adjacent_only && !self.adjacent(source, outgoing) {
                continue;
            }
            if !self.cancel_queued_move(source) {
                continue;
            }
            if self.state.pokemon(source).is_none_or(|p| p.is_fainted()) {
                continue;
            }
            let (Some(slot), Some(position)) = (
                self.state.pokemon(source).and_then(|p| {
                    p.move_slots()
                        .iter()
                        .position(|slot| slot.move_id == crate::ids::MoveId::PURSUIT)
                        .and_then(|index| MoveSlot::new(index as u8))
                }),
                self.state.active_position(outgoing, &self.rules.topology),
            ) else {
                continue;
            };
            let execution = crate::engine::MovePipeline::new(
                &mut self.state,
                &self.rules,
                &self.dex,
                &mut self.trace,
            )
            .without_before_move(rules.skips_before_move)
            .execute(source, slot, Some(DeclaredTarget(position)))?;
            // Gen 2 `runMove` → `AfterMoveSelf`: the pursuer's own after-action residual.
            self.run_after_own_action(source, execution.stopped, false);
        }
        self.state.called_back = None;
        Ok(())
    }

    /// Showdown `queue.cancelMove`: `actor`'s queued move is cancelled (`BattleContext::
    /// cancel_move`); false when it has none still pending this turn.
    fn cancel_queued_move(&mut self, actor: PokemonId) -> bool {
        if !self.state.pending_movers.contains(&actor) {
            return false;
        }
        BattleContext::new(&mut self.state, &self.rules, &mut self.trace).cancel_move(actor);
        true
    }

    fn adjacent(&self, a: PokemonId, b: PokemonId) -> bool {
        match (
            self.state.active_position(a, &self.rules.topology),
            self.state.active_position(b, &self.rules.topology),
        ) {
            (Some(a), Some(b)) => self.rules.topology.all_adjacent(a).contains(&b),
            _ => false,
        }
    }
}
