//! The Move Shell (DESIGN.md §15): the one sequence every move runs — BeforeMove, PP, targeting,
//! TryMove, the per-target loop through the hit gate and the primary effect, the once-per-move
//! secondaries and the after-move steps — and the move result it leaves (§22.8).

use std::ops::ControlFlow;

use crate::dex::{MultiHit, PrimaryEffect, ResolvedMove, SecondaryTrigger, StatusMoveEffect};
use crate::effects;
use crate::ids::{MoveId, MoveSlot, PokemonId};
use crate::rules::{DeclaredTarget, Fraction, MovesSlot, ResolvedRules, ResolvedTarget, Rounding};
use crate::state::{
    AttackRecord, MoveResult, NonVolatileStatus, RandomRequestKind, VolatileConditionKind,
};

use crate::engine::damage::HitBlockReason;
use crate::engine::{
    BattleContext, BattleError, DamageAmount, EffectSource, MoveFailReason, TraceEvent,
};

use super::{CallFrame, HitOutcome, MAX_CALL_DEPTH, MoveExecution, MovePipeline};

mod commit;
mod finish;
mod hit_batch;
mod on_try;
mod prepare_hit;
mod start;
mod targets;
mod try_move;

/// A stage of `run_move` (DESIGN.md §48.3): on to the next with what it found, or the move's
/// end here — its execution as it stands — or an engine error.
type MoveStage<T> = Result<ControlFlow<MoveExecution, T>, BattleError>;

/// What `run_move` is handed: the move, where the action aimed it, how the action came to it.
pub(super) struct MoveRunInput {
    pub(super) actor: PokemonId,
    pub(super) slot: MoveSlot,
    pub(super) move_id: MoveId,
    pub(super) move_def: ResolvedMove,
    pub(super) declared_target: Option<DeclaredTarget>,
    pub(super) action_target: Option<DeclaredTarget>,
    /// An Encore turned the action into this move (`OverrideAction`).
    pub(super) overridden: bool,
    pub(super) resolved: Option<crate::rules::ResolvedTargets>,
    pub(super) struggling: bool,
}

/// What the hits did (`hit_batch`): each hit's outcome, whether any landed, whether a
/// whole-move operation changed nothing, whether a custom primary went through on a Pokémon.
struct HitBatch {
    hits: Vec<HitOutcome>,
    any_landed_hit: bool,
    move_failed: bool,
    custom_hit_pokemon: bool,
}

/// What the move's end reads (`finish_move`): the move as it hit, its targets, how it started,
/// whether it faints its user whatever it meets (read before PrepareHit), a combined Pledge's
/// side condition, and what the hits did.
struct MoveEnd {
    actor: PokemonId,
    move_id: MoveId,
    move_def: ResolvedMove,
    targets: crate::rules::ResolvedTargets,
    releasing: bool,
    defrost_after: bool,
    always: bool,
    pledge_condition: Option<crate::engine::pledges::PledgeCombo>,
    batch: HitBatch,
}

/// A use of a move as it starts (`start_move`): the move as `ModifyType` / `ModifyMove` left
/// it, its targets as they then stand, and what the stages after read of its start.
struct MoveRun {
    actor: PokemonId,
    slot: MoveSlot,
    move_id: MoveId,
    move_def: ResolvedMove,
    declared_target: Option<DeclaredTarget>,
    action_target: Option<DeclaredTarget>,
    overridden: bool,
    resolved: Option<crate::rules::ResolvedTargets>,
    struggling: bool,
    /// Gen 1: the binding lock goes on against the Pokémon it caught.
    continuing_bind: bool,
    /// Gen 2: a frozen user of a `defrost` move thaws once the move has landed.
    defrost_after: bool,
    /// The attacking turn of a two-turn move.
    releasing: bool,
}

impl<'a> MovePipeline<'a> {
    /// Showdown `runEvent('AfterMove')`'s `onAnyAfterMove` handlers of the Pokémon on the field,
    /// fastest first, a holder's ability (Opportunist) before its item (White Herb, Mirror
    /// Herb, Eject Pack).
    fn any_after_move(&mut self) {
        let active: Vec<PokemonId> = self
            .state
            .sides
            .iter()
            .flat_map(|side| side.active.iter().flatten().copied())
            .collect();
        let mut holders: Vec<(PokemonId, u32, u8, crate::effects::SwitchHandler)> = Vec::new();
        for pokemon in active {
            let speed = crate::engine::stats::handler_speed(self.state, pokemon);
            if let Some(handler) = self.state.pokemon(pokemon).and_then(|p| {
                crate::engine::ability_handler(self.state, self.rules, p, |b| b.any_after_move)
            }) {
                holders.push((pokemon, speed, 0, handler));
            }
            if let Some(handler) = crate::engine::held_item(self.state, self.rules, pokemon)
                .and_then(|(_, behavior)| behavior.any_after_move)
            {
                holders.push((pokemon, speed, 1, handler));
            }
        }
        holders.sort_by_key(|(_, speed, sub, _)| (std::cmp::Reverse(*speed), *sub));
        for (pokemon, _, _, handler) in holders {
            handler(
                &mut BattleContext::new(self.state, self.rules, self.trace),
                pokemon,
            );
        }
    }

    pub(crate) fn execute(
        &mut self,
        actor: PokemonId,
        slot: MoveSlot,
        declared_target: Option<DeclaredTarget>,
    ) -> Result<MoveExecution, BattleError> {
        // A move this one called (Metronome, Sleep Talk, Copycat …): Showdown runs it inside
        // the same `useMove`, before the action's AfterMove events (DESIGN.md §42.1). The
        // called move — and anything it in turn calls, up to a bound nothing real reaches — is
        // what the action reports.
        let mut depth = 0;
        self.state.acting_slot = Some((actor, slot.index()));
        self.state.items_received_in_action.clear();
        let mut execution = self.run(actor, slot, declared_target)?;
        // The actor's own `moveThisTurnResult`: a call another Pokémon runs (a reflection) is its
        // user's `useMove`, which leaves the actor's result as its move had it — a move bounced
        // back failed (`hitStepTryHitEvent` turns TryHit's `null` into `false`).
        let mut own_result = None;
        // A call may be for another Pokémon than the actor: Magic Coat and Magic Bounce make the
        // Pokémon they hit use the move back (§42.8).
        while let Some(call) = self.state.pending_call.take() {
            depth += 1;
            if depth > MAX_CALL_DEPTH {
                break;
            }
            if call.user != actor {
                own_result.get_or_insert(execution.result);
            }
            let reflected = matches!(
                call.source,
                crate::state::CallSource::Move(crate::ids::MoveId::MAGIC_COAT)
                    | crate::state::CallSource::Ability(crate::ids::AbilityId::MAGIC_BOUNCE)
            );
            let lent = self
                .lent_priority(reflected)
                .map(|(priority, boosted)| (priority, boosted || call.reflected_prankster));
            // The action's own Z status move calling a move for its user (`sourceEffect.isZ`):
            // the first call of the chain; a move that called is not a Z-Move itself.
            let z_call = depth == 1
                && self.z_powered
                && self.max_base.is_none()
                && call.user == actor
                && matches!(call.source, crate::state::CallSource::Move(source) if Some(source) == self.outermost);
            let mut called =
                MovePipeline::new(&mut *self.state, self.rules, self.dex, &mut *self.trace)
                    .called(CallFrame {
                        source: call.source,
                        action_move: self.outermost,
                    })
                    .reflected(reflected)
                    .with_lent_priority(lent)
                    .with_z_caller(z_call);
            execution = called.execute_called(call)?;
            // What the called move asks of the queue is the action's, as the action's own move's
            // would be: a called Pledge that waited for its ally's (Assist's Water Pledge,
            // R-M8-65), a called Round (Me First's, Metronome's, R-M8-86), a Shell Trap a called
            // move's physical hit armed (Copycat's Avalanche, R-M8-143).
            let effects = called.take_effects();
            self.effects.absorb(effects);
        }
        self.state.acting_slot = None;
        // Gen 3-4 Choice items' `onAfterMove`: the item held now locks its holder onto the move —
        // the one the action chose (the gen4 `choicelock` condition reads `pokemon.lastMove`,
        // which a called move leaves alone), not one the action called (DESIGN.md §42.2).
        // Gen 3: not by an item its own move handed it (issues.md I184).
        let received_in_the_move = self
            .rules
            .conditions
            .items
            .choice_lock_skips_an_item_received_in_the_move
            && self.state.items_received_in_action.contains(&actor);
        self.state.items_received_in_action.clear();
        if !execution.stopped
            && self.rules.conditions.items.choice_locks_after_the_move
            && !received_in_the_move
            && let Some((_, item)) = crate::engine::held_item(self.state, self.rules, actor)
            && let Some(handler) = item.move_decided
        {
            let chosen = self.outermost.unwrap_or(execution.move_id);
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            handler(&mut ctx, actor, chosen);
        }
        // The `AfterMove` events are `runMove`'s: a Gen 3-4 Pursuit striking a Pokémon as it
        // switches out runs `useMove` alone (`skips_before_move`) and fires none of them — a
        // Charge it follows stays (R-M8-168).
        let after_move_events = !self.skips_before_move;
        // Showdown `runEvent('AfterMove')` (`runMove`, once the move is over, its faints still
        // pending): the `onAnyAfterMove` of the abilities and held items on the field.
        if after_move_events && !execution.stopped {
            self.any_after_move();
        }
        // Beak Blast's `onAfterMove`: its volatile ends once the move is over, with no word (seen
        // on until the turn ends).
        if after_move_events
            && !execution.stopped
            && self
                .dex
                .move_by_id(execution.move_id)
                .is_some_and(|m| m.behavior.ends_turn_start_volatile)
        {
            BattleContext::new(self.state, self.rules, self.trace)
                .end_volatile_unseen(actor, VolatileConditionKind::BeakBlast);
        }
        if after_move_events
            && !execution.stopped
            && !self.releasing
            && super::metronome::did_nothing(&execution)
        {
            self.metronome_takes_back(actor, execution.move_id);
        }
        // Showdown `onAfterMove`: a move that struck nothing still pays its self damage once
        // it is over (one that struck something paid it after the hit, in `run`).
        if after_move_events
            && !execution.stopped
            && !self.self_damage_cancelled
            && !struck_something(&execution.hits)
        {
            self.after_move_self_damage(actor, execution.move_id);
        }
        // Charge's `onAfterMove` / `onMoveAborted`: it ends after a move other than Charge — from
        // Gen 9 only an Electric one (its type as used; an aborted move's own: the recharge
        // turn's is `recharge`, Normal, R-M10-57). A move its own `beforeMoveCallback` stopped
        // (Focus Punch without its focus, Bide storing) runs neither event, and Charge stays.
        if after_move_events
            && actor_has_charge(self.state, actor)
            && execution.move_id != MoveId::CHARGE
            && (!execution.stopped || execution.aborted)
        {
            let electric = !self.recharged
                && self
                    .dex
                    .move_by_id(execution.move_id)
                    .is_some_and(|move_def| {
                        // The type as used: what `ModifyType` made it as the move started
                        // (`lastMoveUsed`), not what it would be now — a Doodle that took the
                        // user's Galvanize away was Electric all the same (R-M11-63).
                        let as_used = self
                            .state
                            .pokemon(actor)
                            .and_then(|pokemon| pokemon.last_move_used)
                            .filter(|(id, _)| *id == execution.move_id)
                            .map(|(_, move_type)| move_type);
                        let move_type = if execution.stopped {
                            move_def.move_type
                        } else if let Some(move_type) = as_used {
                            move_type
                        } else {
                            crate::engine::modified_move_cow(
                                self.state, self.rules, actor, move_def, false,
                            )
                            .0
                            .move_type
                        };
                        move_type == crate::dex::PokemonType::Electric
                    });
            if self.rules.conditions.charge_ends_after_any_move || electric {
                BattleContext::new(self.state, self.rules, self.trace)
                    .end_volatile(actor, VolatileConditionKind::Charge);
            }
        }
        // Spit Up's `onAfterMove`: the Stockpile ends.
        if after_move_events
            && !execution.stopped
            && self
                .dex
                .move_by_id(execution.move_id)
                .is_some_and(|move_def| move_def.behavior.ends_stockpile)
        {
            BattleContext::new(self.state, self.rules, self.trace).end_stockpile(actor);
        }
        // Thrash's lock and Uproar as the move is over (`onAfterMove`, §15.12).
        self.rampage_after_move(actor, &execution);
        self.uproar_after_move(actor, &execution);
        self.locked_miss_spends_pp(actor, &execution);
        self.fury_cutter_after_move(actor, &execution);
        self.rage_after_move(actor, &execution);
        self.rolling_after_move(actor, &execution);
        // Showdown `moveThisTurnResult`, and `lastSuccessfulMoveThisTurn` — which a Pokémon
        // that could not act (BeforeMove) leaves alone, as Showdown returns before setting it.
        // Both read the move executed — a called one's own result, not its caller's (Showdown's
        // `moveDidSomething` for the latter is the caller's `useMove`: a Fusion Bolt Sleep Talk
        // called into an immunity still powered Fusion Flare; issues.md I213, patched).
        let result = own_result.unwrap_or(execution.result);
        if let Some(pokemon) = self.state.pokemon_mut(actor) {
            pokemon.history.move_this_turn = Some(result);
        }
        if !execution.stopped {
            self.state.last_successful_move =
                (result == MoveResult::Succeeded).then_some(execution.move_id);
        }
        // Dancer (Showdown's `runMove` tail, after the move's and its user's `AfterMove` — a
        // lock's end and its fatigue, R-M8-78 — and the faints it caused): another Pokémon with
        // the ability dances along — slowest first (the stored speeds), a fainted one skips,
        // each using the same move at the original target when it is a foe of the dancer's and
        // the original user its ally, else at the original user (DESIGN.md §42.9). The original
        // target is Showdown's `activeTarget`: the last Pokémon the move's hit steps went through
        // — where a random draw (Petal Dance) or a redirection sent it, not what the action
        // named (R-M8-122). Only where the move the actor executed did something: one another
        // Pokémon took over (Snatch) or sent back failed for its user, and what that one used is
        // no dance of the action's (R-M10-69); a dance the action's move called counts by its
        // own result — Swords Dance at +6 that Assist called is not followed, a Feather Dance
        // Mirror Move called that landed is (issues.md I213: Showdown reads the caller's
        // `useMove`, `moveDidSomething`; the oracle is patched).
        if !execution.stopped
            && self.call.is_none()
            && self
                .dex
                .move_by_id(execution.move_id)
                .is_some_and(|move_def| move_def.flags.dance)
            && !matches!(result, MoveResult::Failed | MoveResult::Neither)
        {
            let active_target = execution.hits.last().map(|hit| match *hit {
                HitOutcome::Missed { target }
                | HitOutcome::Blocked { target, .. }
                | HitOutcome::Damaged { target, .. }
                | HitOutcome::Effect { target }
                | HitOutcome::Reflected { target }
                | HitOutcome::SubstituteDamaged { target, .. }
                | HitOutcome::NoEffect { target } => target,
            });
            self.dancers_dance(actor, execution.move_id, active_target);
        }
        // Gen 4 (pokeplatinum `BattleControllerPlayer_MoveEnd` → `BattleSystem_TriggerEffectOnSwitch`):
        // an ability gained during the move starts now, unless its kind of switch-in effect was
        // announced this stint already.
        if self.rules.conditions.ability_change.start_new
            == crate::rules::StartNewAbility::Unannounced
        {
            BattleContext::new(self.state, self.rules, self.trace).announce_unannounced();
        }
        Ok(execution)
    }

    /// The nested run of `call` (DESIGN.md §42.1): like `execute`, without the action layer's
    /// AfterMove events — Showdown's `useMove` runs the move alone and its own end is the
    /// caller's.
    fn execute_called(
        &mut self,
        call: crate::state::MoveCall,
    ) -> Result<MoveExecution, BattleError> {
        let move_def = self
            .dex
            .move_by_id(call.move_id)
            .cloned()
            .ok_or(BattleError::UnknownMove(call.move_id))?;
        // Called by a Z status move: a damaging move runs as its Z-Move, Z-powered (R-M9-41).
        let move_def = match self
            .z_call
            .then(|| crate::engine::called_z_move(self.state, self.dex, call.user, &move_def))
            .flatten()
        {
            Some(z_move) => {
                self.z_powered = true;
                z_move
            }
            None => move_def,
        };
        let declared = match call.target {
            crate::state::CallTarget::Random => None,
            crate::state::CallTarget::Pokemon(target) => self
                .state
                .active_position(target, &self.rules.topology)
                .map(DeclaredTarget),
        };
        // Showdown `useMove` without a target (`getRandomTarget`): a caller that names none
        // (Metronome, Assist, Copycat …) has a single-target move drawn among the adjacent foes,
        // as the action layer's fallback does — in a double battle there is no one target to
        // take (DESIGN.md R-M8-55).
        let resolved = match (
            self.rules.targeting.resolve(
                &self.rules.topology,
                self.state,
                call.user,
                move_def.target,
                declared,
            ),
            call.target,
        ) {
            (Ok(targets), _) => Some(targets),
            (Err(error), crate::state::CallTarget::Random) => {
                self.fallback_target(call.user, &move_def, declared, error)
            }
            // A caller that names the target (Me First, a reflection) has the move go there even
            // when the move picks its own target at random (`randomNormal`: Uproar, Thrash) —
            // Showdown's `getMoveTargets` keeps a given target that still stands (R-M8-66).
            (Err(_), crate::state::CallTarget::Pokemon(target)) => self
                .state
                .pokemon(target)
                .filter(|pokemon| !pokemon.is_fainted())
                .filter(|_| {
                    self.state
                        .active_position(target, &self.rules.topology)
                        .is_some()
                })
                .map(|_| crate::rules::ResolvedTargets {
                    targets: vec![ResolvedTarget::Pokemon(target)],
                }),
        };
        self.execute_called_with(call, move_def, declared, resolved)
    }

    /// The nested run with the move and its targets already in hand (Dancer resolves them
    /// ahead of time for its own announcement).
    fn execute_called_with(
        &mut self,
        call: crate::state::MoveCall,
        mut move_def: crate::dex::ResolvedMove,
        declared: Option<DeclaredTarget>,
        resolved: Option<crate::rules::ResolvedTargets>,
    ) -> Result<MoveExecution, BattleError> {
        // Showdown `useMoveInner` (`move.ignoreAbility = sourceEffect.ignoreAbility`): a called
        // move ignores abilities as its caller does, not as its own data says — the calling move
        // of the same user as ModifyMove left it (that user's Mold Breaker, Mycelium Might on a
        // status caller), never a condition or an ability making another Pokémon use the move
        // (Snatch, Magic Coat, Magic Bounce, Dancer). The user's own ModifyMove adds Mold
        // Breaker's again. Sunsteel Strike called by another move does not ignore abilities
        // (Bulbapedia, R-M9-43).
        move_def.ignores_ability = self
            .state
            .active_move
            .as_ref()
            .is_some_and(|active| active.user == call.user && active.ignores_ability);
        if move_def.primary_effect == PrimaryEffect::Unsupported {
            return Ok(MoveExecution {
                actor: call.user,
                move_id: call.move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Failed,
            });
        }
        // A move another one called meets Heal Block, Throat Chop and Gravity as it is modified
        // (`onModifyMove`): refused before its `move` line, the caller's action going on to its
        // end (R-M8-70). A full `runMove` (Dancer) meets them at BeforeMove, below.
        if self.skips_before_move
            && let Some(reason) = self.state.pokemon(call.user).and_then(|pokemon| {
                crate::engine::before_move::refused_as_the_move_starts(
                    self.state, pokemon, &move_def,
                )
            })
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor: call.user,
                move_id: call.move_id,
                reason: MoveFailReason::BeforeMove(reason),
            });
            return Ok(MoveExecution {
                actor: call.user,
                move_id: call.move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Failed,
            });
        }
        // A full `runMove` (Dancer): BeforeMove runs, its volatiles' endings included, and a
        // block is the answer.
        if !self.skips_before_move
            && let crate::engine::CanMoveResult::Blocked(reason) =
                self.before_move(call.user, &move_def, false)
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor: call.user,
                move_id: call.move_id,
                reason: MoveFailReason::BeforeMove(reason),
            });
            return Ok(MoveExecution {
                actor: call.user,
                move_id: call.move_id,
                hits: Vec::new(),
                stopped: true,
                aborted: true,
                result: MoveResult::Failed,
            });
        }
        self.run_move(MoveRunInput {
            actor: call.user,
            slot: MoveSlot::new(0).unwrap(),
            move_id: call.move_id,
            move_def,
            declared_target: declared,
            action_target: declared,
            overridden: false,
            resolved,
            struggling: false,
        })
    }

    /// Showdown's `BeforeMove` event for `actor`'s move, an action's or a dance's full `runMove`:
    /// Grudge's, Gen 5+ Rage's and Glaive Rush's `onBeforeMove` (priority 100, ahead of every
    /// check) end them whatever comes (§15.9, §15.19, §15.20); then the checks; then Destiny
    /// Bond's `onBeforeMove` (priority -1) ends it unless the move is Destiny Bond, and its
    /// `onMoveAborted` whatever the move when a check stopped it (a dance too, R-M8-135).
    fn before_move(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        is_z: bool,
    ) -> crate::engine::CanMoveResult {
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.end_volatile(actor, VolatileConditionKind::Grudge);
        ctx.end_volatile(actor, VolatileConditionKind::GlaiveRush);
        self.rage_before_move(actor);
        let can_move = crate::engine::before_move::resolve(
            self.state, self.rules, self.dex, self.trace, actor, move_def, is_z,
        );
        if move_def.id != MoveId::DESTINY_BOND
            || matches!(can_move, crate::engine::CanMoveResult::Blocked(_))
        {
            BattleContext::new(self.state, self.rules, self.trace)
                .end_volatile(actor, VolatileConditionKind::DestinyBond);
        }
        can_move
    }

    pub(super) fn run(
        &mut self,
        actor: PokemonId,
        slot: MoveSlot,
        declared_target: Option<DeclaredTarget>,
    ) -> Result<MoveExecution, BattleError> {
        if self
            .state
            .active_position(actor, &self.rules.topology)
            .is_none()
        {
            return Err(BattleError::ActorNotActive(actor));
        }

        let move_id = self
            .state
            .pokemon(actor)
            .and_then(|pokemon| pokemon.move_slot(slot))
            .map(|move_slot| move_slot.move_id)
            .ok_or(BattleError::InvalidMoveSlot { actor, slot })?;
        // Showdown `runMove`: `activeMoveActions++` before BeforeMove (a fully paralysed turn
        // still counts). A move run through `useMove` alone does not count (§15.11).
        if !self.skips_before_move
            && let Some(pokemon) = self.state.pokemon_mut(actor)
        {
            pokemon.active_move_actions = pokemon.active_move_actions.saturating_add(1);
        }
        // Showdown `OverrideAction` (Encore): an Encore that took hold after the choice was made
        // replaces the chosen move with the encored one this very turn, and the target is
        // chosen anew (`getRandomTarget`: the implicit target of `TargetingRules::resolve`).
        // No move was left to choose (Showdown's request held Struggle alone): the action is
        // Struggle, at a random adjacent foe, whatever slot it names.
        let struggling = self
            .state
            .struggling
            .iter()
            .position(|pokemon| *pokemon == actor)
            .map(|index| self.state.struggling.remove(index))
            .is_some();
        // A move a hard lock holds that the user's slots do not (Outrage, Rollout, a charge …
        // called by Metronome, Copycat or Assist, §42.1) is the action's move whatever slot it
        // named — Showdown's request offers `getLockedMove` alone — and from Gen 4 an Encore
        // overrides it as any action's (`onOverrideAction` at `runAction`; Gen 3 keeps the lock,
        // `CallRules::encore_overrides_a_called_lock`, R-M8-137).
        let locked_elsewhere = self
            .state
            .pokemon(actor)
            .and_then(|p| p.hard_lock())
            .filter(|lock| p_has_no_slot(self.state, actor, lock.move_id))
            .map(|lock| lock.move_id);
        // A Z-Move's or a Max Move's action is never overridden (`runMove` asks `OverrideAction`
        // only when `!zMove && !maxMove`): an Encore that landed earlier in the turn leaves it
        // alone (R-M9-31).
        let encored = if struggling || self.z_move.is_some() || self.max_move.is_some() {
            None
        } else {
            self.encore_override(
                actor,
                locked_elsewhere
                    .filter(|_| self.rules.conditions.calls.encore_overrides_a_called_lock)
                    .unwrap_or(move_id),
            )
        };
        // Showdown keeps the action's `targetLoc` through the override: it becomes the user's
        // `lastMoveTargetLoc`, where a two-turn move's charge aims.
        let action_target = declared_target;
        let overridden = encored.is_some();
        let (slot, move_id, declared_target) = match encored {
            _ if struggling => (slot, MoveId::STRUGGLE, None),
            Some((encored_slot, encored)) => {
                self.trace.push(TraceEvent::MoveOverridden {
                    actor,
                    chosen: move_id,
                    used: encored,
                });
                (encored_slot, encored, None)
            }
            None => (slot, move_id, declared_target),
        };
        // Gen 1: a user locked into its binding move uses it, whatever the action named.
        let (slot, move_id) = self.binding_locked_move(actor, slot, move_id);
        let move_id = match locked_elsewhere {
            Some(locked)
                if encored.is_none()
                    || !self.rules.conditions.calls.encore_overrides_a_called_lock =>
            {
                locked
            }
            _ => move_id,
        };
        // A hard lock aims as it holds: a two-turn move's attack where its charge aimed, a
        // Rollout chain where it started (§15.12), the others anew. Showdown's `chooseMove` gives
        // a locked action the lock's `targetLoc` (`lockedMoveTargetLoc`) and refuses one the
        // choice names, which validation does not ask here (`validate_move_action`): a target
        // submitted with it steers nothing (R-M10-20).
        let declared_target = match self.state.pokemon(actor).and_then(|p| p.hard_lock()) {
            Some(lock) if lock.move_id == move_id => lock.target.map(DeclaredTarget),
            _ => declared_target,
        };

        let move_def = self
            .dex
            .move_by_id(move_id)
            .cloned()
            .ok_or(BattleError::UnknownMove(move_id))?;

        // A Z-Move runs in place of the slot's move (Showdown `runMove`: `move =
        // getActiveZMove(baseMove, pokemon)` before every check, DESIGN.md §44.4): the Z-Move's
        // own data (type, target, flags) with the base move's power, category and priority. Its
        // PP pays from the slot, and it is the move that becomes the last move. A Pokémon left
        // with Struggle drops it; an Encore does not override it (above).
        // Showdown `maxMove.baseMove` (DESIGN.md §44.5): a Max Move remembers the move it was
        // made from; Encore, Spite and Copycat resolve a Max Move's `lastMove` through it.
        let mut max_base = None;
        let (move_id, move_def) = match self.max_move.take() {
            // A Dynamaxed Pokémon's move is its Max Move (DESIGN.md §44.5). Struggle is itself.
            Some(max) if encored.is_none() && !struggling && max.base == move_id => {
                max_base = Some(max.base);
                (max.move_def.id, max.move_def)
            }
            _ => match self.z_move.take() {
                Some(z) if encored.is_none() && !struggling && z.base == move_id => {
                    // Its power waits for the move to be used (`spend_z_power`).
                    let resolved = (z.move_def.id, z.move_def.clone());
                    self.z_power = Some(z);
                    resolved
                }
                _ => (move_id, move_def),
            },
        };
        self.max_base = max_base;

        if move_def.primary_effect == PrimaryEffect::Unsupported {
            // Choice validation refuses these. This is the defensive boundary for callers that
            // bypass validation; the move fails loudly instead of doing nothing.
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::Unsupported,
            });
            return Ok(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: false,
                aborted: false,
                result: MoveResult::Failed,
            });
        }

        // Execution-time targeting (ROADMAP §6.3), before BeforeMove as Showdown's `runMove`
        // does it (`getTarget`, then `getRandomTarget` after an Encore override): the declared
        // target may be gone by now, and a random eligible foe is drawn even when the user then
        // turns out to be fully paralysed. A move left with no target at all fails only after
        // BeforeMove (`useMove`'s `-notarget`).
        // Showdown `getTarget`: a move that tracks its target (Snipe Shot; any move of a Stalwart
        // or Propeller Tail user, `hasAbility` before `ModifyMove`) goes at the action's
        // `originalTarget` while it is on the field, wherever an Ally Switch put it (R-M8-106).
        // The outermost move's only: a called move takes its own target.
        let original_target = self.original_target.take();
        let tracks_target = move_def.tracks_target
            || self.state.pokemon(actor).is_some_and(|user| {
                crate::engine::ability_handler(self.state, self.rules, user, |b| b.move_edit)
                    .is_some_and(|edit| edit.tracks_target)
            });
        // A move an Encore put in the chosen one's place tracks nothing: Showdown's
        // `OverrideAction` draws its target with `getRandomTarget` (R-M8-124).
        let declared_target = match original_target {
            Some(original) if tracks_target && encored.is_none() => self
                .state
                .active_position(original, &self.rules.topology)
                .map(DeclaredTarget)
                .or(declared_target),
            _ => declared_target,
        };
        let resolved = match self.rules.targeting.resolve(
            &self.rules.topology,
            self.state,
            actor,
            move_def.target,
            declared_target,
        ) {
            Ok(targets) => Some(targets),
            Err(error) => self.fallback_target(actor, &move_def, declared_target, error),
        };

        // Gen 2 (pokecrystal: the move choice): a move other than Fury Cutter ends its count.
        if move_id != MoveId::FURY_CUTTER
            && matches!(
                self.rules.conditions.fury_cutter.model,
                crate::rules::FuryCutterModel::Count {
                    another_move_ends_it: true,
                    ..
                }
            )
        {
            BattleContext::new(self.state, self.rules, self.trace).end_fury_cutter(actor);
        }
        let can_move = if self.skips_before_move {
            crate::engine::CanMoveResult::Allow
        } else {
            self.before_move(actor, &move_def, self.z_power.is_some())
        };
        self.trace.push(TraceEvent::BeforeMoveResolved {
            actor,
            own_move: self.shown_own_move(move_id),
            result: can_move,
        });
        if let crate::engine::CanMoveResult::Blocked(reason) = can_move {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::BeforeMove(reason),
            });
            // Gen 4: a turn lost to anything but sleep or recharging did no work
            // (`MOVE_STATUS_NO_MORE_WORK`) — unless it was a two-turn move's attacking turn.
            let charged = self
                .state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.charging_move() == Some(move_id));
            if !charged
                && !matches!(
                    reason,
                    crate::engine::BeforeMoveBlockReason::Sleep
                        | crate::engine::BeforeMoveBlockReason::WakingTurn
                        | crate::engine::BeforeMoveBlockReason::Recharge
                )
            {
                self.metronome_takes_back(actor, move_id);
            }
            // Thrash's lock (§15.12): Gen 1's full paralysis and self-hit end it, Gen 3-4 mark
            // the turn failed.
            self.rampage_blocked(actor, reason);
            self.uproar_blocked(actor, reason);
            // Gen 3-4 (the cartridges): some blocks end Fury Cutter's count, whatever the move.
            if self
                .rules
                .conditions
                .fury_cutter
                .ending_blocks
                .contains(&reason)
            {
                BattleContext::new(self.state, self.rules, self.trace).end_fury_cutter(actor);
            }
            // Gen 3-4 on the cartridge (issues.md I67): the attempt leaves no last move
            // (pokeemerald `MOVE_UNAVAILABLE`, pokeplatinum `MOVE_NONE`) — Encore and Disable
            // find nothing to act on. Gen 1: only a turn lost to sleep or a freeze does.
            let clears_last_move = self.rules.conditions.blocked_move_clears_last_move
                || (self.rules.conditions.sleep_or_freeze_clears_last_move
                    && matches!(
                        reason,
                        crate::engine::BeforeMoveBlockReason::Sleep
                            | crate::engine::BeforeMoveBlockReason::WakingTurn
                            | crate::engine::BeforeMoveBlockReason::Freeze
                    ));
            if clears_last_move && let Some(pokemon) = self.state.pokemon_mut(actor) {
                pokemon.last_move = None;
                pokemon.last_move_encore = None;
                pokemon.last_move_base = None;
            }
            // Gen 2+ Bide's `onMoveAborted`: stopped, the holder lets it go (§15.10); Gen 1 only
            // after a full paralysis or a confusion self-hit (the gen1 `par` / `confusion`;
            // pokered `MonHurtItselfOrFullyParalysed` clears `STORING_ENERGY`).
            if !self.rules.conditions.retaliation.bide.gen1
                || matches!(
                    reason,
                    crate::engine::BeforeMoveBlockReason::Paralysis
                        | crate::engine::BeforeMoveBlockReason::Confusion
                )
            {
                // Gen 1's has no `onEnd`: gone with no word; Gen 2+'s `-end … [silent]`.
                let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                if self.rules.conditions.retaliation.bide.gen1 {
                    ctx.end_volatile_unseen(actor, VolatileConditionKind::Bide);
                } else {
                    ctx.end_volatile(actor, VolatileConditionKind::Bide);
                }
            }
            // Showdown `MoveAborted`: a two-turn move in progress is dropped (the user comes
            // back within reach, the lock is gone). Gen 1 (`ChargeModel::Gen1`): only full
            // paralysis, Disable and a confusion self-hit drop it, and only the self-hit brings
            // Fly / Dig's user back within reach.
            use crate::engine::BeforeMoveBlockReason as Reason;
            let (drops_charge, drops_hiding) = if self.rules.two_turn.gen1() {
                (
                    matches!(
                        reason,
                        Reason::Paralysis | Reason::Disabled | Reason::Confusion
                    ),
                    reason == Reason::Confusion,
                )
            } else {
                (true, false)
            };
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            if drops_charge {
                // Sky Drop's `twoturnmove` `onEnd` ends the `skydrop` volatile with it.
                if ctx.end_volatile(actor, VolatileConditionKind::TwoTurnMove) {
                    ctx.end_sky_drop(actor);
                }
            }
            if drops_hiding {
                ctx.end_volatile(actor, VolatileConditionKind::Invulnerability);
            }
            self.recharged = reason == crate::engine::BeforeMoveBlockReason::Recharge;
            // Showdown's BeforeMove handlers return `false` (the move failed, for Stomping
            // Tantrum) — except the recharge turn's `null`: there was nothing to choose.
            let result = if matches!(
                reason,
                crate::engine::BeforeMoveBlockReason::Recharge
                    | crate::engine::BeforeMoveBlockReason::SkyDropped
            ) {
                MoveResult::Neither
            } else {
                MoveResult::Failed
            };
            return Ok(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: true,
                aborted: true,
                result,
            });
        }
        // Bide's volatile past its first turn (`onBeforeMove`, `beforeMoveCallback`, §15.10).
        if let Some(execution) = self.bide_turn(actor, move_id) {
            return Ok(execution);
        }
        // Gen 5+ Focus Punch's `beforeMoveCallback` (§15.8): a user that lost its focus does
        // not move — no PP spent, the move failed.
        if move_def.behavior.lost_focus_stops
            && self
                .state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.lost_focus())
        {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::BeforeMove(crate::engine::BeforeMoveBlockReason::LostFocus),
            });
            return Ok(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: true,
                aborted: false,
                result: MoveResult::Failed,
            });
        }
        // Showdown `runMove`: a move with no PP left as its turn comes (Spite or Eerie Spell took
        // it after it was chosen) does not happen — `cant|user|nopp|move`, a failure for Stomping
        // Tantrum, before anything the move itself does (§15.15). A locked turn spends none.
        if !struggling && self.out_of_pp(actor, move_id) {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::NoPp,
            });
            return Ok(MoveExecution {
                actor,
                move_id,
                hits: Vec::new(),
                stopped: true,
                aborted: false,
                result: MoveResult::Failed,
            });
        }
        self.run_move(MoveRunInput {
            actor,
            slot,
            move_id,
            move_def,
            declared_target,
            action_target,
            overridden,
            resolved,
            struggling,
        })
    }

    /// The move itself, stage by stage (DESIGN.md §48.3): as it starts (`start_move`: Showdown
    /// `ModifyType` / `ModifyMove`), its use paid and recorded (`commit_use`: `runMove`'s
    /// `deductPP`, the `move` line, `moveUsed`), then its targets, TryMove, PrepareHit, the hits
    /// and its end. A call comes here without the action around it (BeforeMove, §42.1).
    fn run_move(&mut self, input: MoveRunInput) -> Result<MoveExecution, BattleError> {
        let run = self.start_move(input);
        let continuing_lock = match self.commit_use(&run)? {
            ControlFlow::Continue(continuing_lock) => continuing_lock,
            ControlFlow::Break(execution) => return Ok(execution),
        };
        let (targets, smart) = match self.move_targets(&run)? {
            ControlFlow::Continue(found) => found,
            ControlFlow::Break(execution) => return Ok(execution),
        };
        self.pressure_pp(&run, continuing_lock, &targets);
        match self.try_move(&run, &targets)? {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(execution) => return Ok(execution),
        }
        match self.on_try(&run, &targets)? {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(execution) => return Ok(execution),
        }
        let MoveRun {
            actor,
            move_id,
            move_def,
            defrost_after,
            releasing,
            ..
        } = run;
        // As `try_move` read them: whether the move faints its user whatever it meets
        // (Explosion and its kin), and how many Pokémon it targets.
        let always = move_def.self_destruct == Some(crate::dex::SelfDestruct::Always);
        let pokemon_target_count = targets.pokemon_count();

        let (move_def, pledge_condition) =
            match self.prepare_hit(actor, move_id, move_def, &targets, pokemon_target_count)? {
                ControlFlow::Continue(prepared) => prepared,
                ControlFlow::Break(execution) => return Ok(execution),
            };

        // Gen 1-2 open with Thrash's lock, ahead of the hit test (§15.12): it starts, or counts a
        // use down.
        self.counted_rampage_use(actor, &move_def);

        self.hp_before_hits = self
            .state
            .all_pokemon()
            .iter()
            .map(|pokemon| (pokemon.id(), pokemon.hp))
            .collect();

        let batch = self.hit_batch(
            actor,
            move_id,
            &move_def,
            &targets,
            smart,
            pokemon_target_count,
        )?;

        self.finish_move(MoveEnd {
            actor,
            move_id,
            move_def,
            targets,
            releasing,
            defrost_after,
            always,
            pledge_condition,
            batch,
        })
    }

    /// Dancer's dance-along (DESIGN.md §42.9): every other standing Pokémon with Dancer uses
    /// the move the actor just used, slowest first, each with the full `runMove` (BeforeMove
    /// included) but without PP. One in a semi-invulnerable charge (Fly, Dig …) or carried up
    /// by Sky Drop does not dance (`!isSemiInvulnerable()`, R-M9-30).
    fn dancers_dance(&mut self, actor: PokemonId, move_id: MoveId, original: Option<PokemonId>) {
        let mut dancers: Vec<(PokemonId, u32, u32)> = self
            .state
            .all_pokemon()
            .iter()
            .filter(|pokemon| pokemon.id() != actor && !pokemon.is_fainted())
            .filter(|pokemon| {
                self.state
                    .active_position(pokemon.id(), &self.rules.topology)
                    .is_some()
                    && crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.dancer)
                    && crate::engine::two_turn::hiding(pokemon).is_none()
            })
            .map(|pokemon| {
                (
                    pokemon.id(),
                    u32::from(pokemon.battle_stats().speed),
                    pokemon.ability_state.effect_order,
                )
            })
            .collect();
        if dancers.is_empty() {
            return;
        }
        // Lowest stored Speed first; ties to the Pokémon that has had the ability the shortest
        // time (Showdown sorts by `storedStats.spe`, then `abilityState.effectOrder` descending:
        // the later-started first, R-M8-182).
        dancers.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| b.2.cmp(&a.2)));
        for (dancer, _, _) in dancers {
            // Showdown's `faintMessages()` between the dances: one that fell skips its turn.
            if self
                .state
                .pokemon(dancer)
                .is_none_or(|pokemon| pokemon.is_fainted())
            {
                continue;
            }
            let target = Some(crate::engine::dancer_target(
                self.state, actor, dancer, original,
            ));
            self.trace
                .push(TraceEvent::DancerActivated { dancer, move_id });
            let call = crate::state::MoveCall {
                user: dancer,
                move_id,
                target: target.map_or(
                    crate::state::CallTarget::Random,
                    crate::state::CallTarget::Pokemon,
                ),
                source: crate::state::CallSource::Ability(crate::ids::AbilityId::DANCER),
                reflected_prankster: false,
            };
            let Some(move_def) = self.dex.move_by_id(move_id).cloned() else {
                continue;
            };
            let declared = target
                .and_then(|target| self.state.active_position(target, &self.rules.topology))
                .map(crate::rules::DeclaredTarget);
            // Showdown's `getTarget` draws a `randomNormal` dance's target (Petal Dance) among
            // the dancer's adjacent foes whatever location it is given (`getRandomTarget`,
            // DESIGN.md R-M8-78).
            let resolved = match self.rules.targeting.resolve(
                &self.rules.topology,
                self.state,
                dancer,
                move_def.target,
                declared,
            ) {
                Ok(targets) => Some(targets),
                Err(error) => self.fallback_target(dancer, &move_def, declared, error),
            };
            // Showdown's `noLock` (the "Dancer Petal Dance hack" in `runMove`): an external move
            // deletes the `lockedmove` the dance just set, unless the dancer already carried one
            // (its own rampage goes on). Deleted outright, with no `-end` line and no confusion.
            let had_rampage = self
                .state
                .pokemon(dancer)
                .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::Rampage));
            let lent = self.lent_priority(false);
            let mut dance =
                MovePipeline::new(&mut *self.state, self.rules, self.dex, &mut *self.trace)
                    .called(CallFrame {
                        source: call.source,
                        action_move: None,
                    })
                    .with_before_move(true)
                    .with_lent_priority(lent)
                    .with_external();
            let _ = dance.execute_called_with(call, move_def, declared, resolved);
            // A dance asks of the queue as any run does (none can yet: no physical dance meets
            // a Shell Trap, R-M11-18).
            let effects = dance.take_effects();
            self.effects.absorb(effects);
            if !had_rampage && let Some(pokemon) = self.state.pokemon_mut(dancer) {
                pokemon.volatiles.remove(VolatileConditionKind::Rampage);
            }
        }
    }

    /// The reflection a spread move's target just made (Magic Coat, Magic Bounce), run at once —
    /// Showdown's `useMove` inside `onTryHit` — before the move meets its other targets.
    fn run_reflection_now(&mut self) -> Result<(), BattleError> {
        if let Some(call) = self.state.pending_call.take() {
            let action_move = self
                .call
                .and_then(|frame| frame.action_move)
                .or(self.outermost);
            let lent = self
                .lent_priority(true)
                .map(|(priority, boosted)| (priority, boosted || call.reflected_prankster));
            let mut reflection =
                MovePipeline::new(&mut *self.state, self.rules, self.dex, &mut *self.trace)
                    .called(CallFrame {
                        source: call.source,
                        action_move,
                    })
                    .reflected(true)
                    .with_lent_priority(lent);
            reflection.execute_called(call)?;
            // A reflection asks of the queue as any run does (none can yet: a reflected move is a
            // status move, R-M11-18).
            let effects = reflection.take_effects();
            self.effects.absorb(effects);
        }
        Ok(())
    }

    /// Showdown `useMoveInner`: a move used inside another takes the active move's priority
    /// (`activeMove.priority`: from Gen 6 the action's, which Psychic Terrain, Quick Guard and
    /// Dazzling read at the hit) and, unless it bounced back, its Prankster boost (DESIGN.md
    /// R-M8-81).
    fn lent_priority(&self, bounced: bool) -> Option<(i8, bool)> {
        self.action_priority
            .map(|priority| (priority, self.prankster_boosted && !bounced))
    }

    /// Snatch's take of the move (`snatch_steals`), but of a move Snatch itself called
    /// (`snatched`) only where the generation takes it again (R-M8-68).
    fn snatch_takes(&mut self, actor: PokemonId, move_def: &ResolvedMove, snatched: bool) -> bool {
        (!snatched || self.rules.conditions.calls.snatch_takes_a_snatched_move)
            && self.snatch_steals(actor, move_def)
    }

    /// Snatch's `onAnyPrepareHit` (DESIGN.md §42.8): the fastest Snatch holder that is not the
    /// user takes a `snatch`-flagged move over — its volatile ends, it announces itself, it pays
    /// the user's Pressure where the generation has it (`CallRules::taker_pays_pressure`) and it
    /// uses the move itself, at a random target. A Z-Move or a Max Move is not taken (`move.isZ ||
    /// move.isMax`: Z-Tailwind, R-M10-43).
    fn snatch_steals(&mut self, actor: PokemonId, move_def: &ResolvedMove) -> bool {
        if !move_def.flags.snatch || self.z_powered {
            return false;
        }
        let mut holders: Vec<(PokemonId, u32)> = self
            .state
            .all_pokemon()
            .iter()
            .filter(|pokemon| {
                pokemon.id() != actor
                    && !pokemon.is_fainted()
                    && self
                        .state
                        .active_position(pokemon.id(), &self.rules.topology)
                        .is_some()
                    && pokemon.volatiles.contains(VolatileConditionKind::Snatch)
            })
            .map(|pokemon| {
                (
                    pokemon.id(),
                    crate::engine::stats::handler_speed(self.state, pokemon.id()),
                )
            })
            .collect();
        if holders.is_empty() {
            return false;
        }
        // `onAnyPrepareHit` runs every handler; the fastest holder's steal takes the move, and
        // a second holder's find it gone (`removeVolatile` inside the handler). Holders sharing
        // a handler speed are shuffled (`speedSort`): each draws a SpeedTie (DESIGN.md R-M8-93).
        let speeds: Vec<u32> = holders.iter().map(|(_, speed)| *speed).collect();
        let mut keyed: Vec<(PokemonId, u32, u64)> = holders
            .iter()
            .map(|&(id, speed)| {
                let tie = if speeds.iter().filter(|other| **other == speed).count() > 1 {
                    self.state.rng.next_u64_for(RandomRequestKind::SpeedTie)
                } else {
                    0
                };
                (id, speed, tie)
            })
            .collect();
        keyed.sort_by_key(|(_, speed, tie)| std::cmp::Reverse((*speed, *tie)));
        holders = keyed
            .into_iter()
            .map(|(id, speed, _)| (id, speed))
            .collect();
        let (snatcher, _) = holders[0];
        // Gone as it steals, with no word (`removeVolatile('snatch')`): seen on until the turn ends.
        BattleContext::new(self.state, self.rules, self.trace)
            .end_volatile_unseen(snatcher, VolatileConditionKind::Snatch);
        self.trace.push(TraceEvent::MoveSnatched {
            snatcher,
            from: actor,
            move_id: move_def.id,
        });
        crate::engine::context::takes_over_under_pressure(
            self.state,
            self.rules,
            self.trace,
            snatcher,
            actor,
            MoveId::SNATCH,
        );
        BattleContext::new(self.state, self.rules, self.trace).call_move(
            snatcher,
            move_def.id,
            crate::state::CallTarget::Random,
            crate::state::CallSource::Move(crate::ids::MoveId::SNATCH),
        );
        true
    }

    /// The other Pledge an ally still has queued this turn (Showdown's `onPrepareHit` walks
    /// `this.queue.list` for an active ally using one of the other two), or `None`.
    fn pledge_partner(&self, actor: PokemonId, move_id: MoveId) -> Option<MoveId> {
        let side = self.state.pokemon(actor).map(|pokemon| pokemon.side)?;
        let queued = &self.state.queued_moves;
        queued
            .iter()
            .filter(|queued| queued.actor != actor)
            // `action.maxMove || action.zmove`: a Dynamaxed ally's or a Z-Powered Pledge is no
            // partner (DESIGN.md R-M9-27); nor is a recharging ally's slot (`recharge`).
            .filter(|queued| queued.walked_move().is_some())
            .filter(|queued| {
                // Still queued this turn (`queue.willMove`): an ally that already ran is no
                // partner any more.
                self.state.pending_movers.contains(&queued.actor)
                    && self
                        .state
                        .pokemon(queued.actor)
                        .is_some_and(|pokemon| pokemon.side == side && !pokemon.is_fainted())
                    && self
                        .state
                        .active_position(queued.actor, &self.rules.topology)
                        .is_some()
            })
            .map(|queued| queued.move_id)
            .find(|other| crate::engine::pledges::is_pledge(*other) && *other != move_id)
    }

    /// Sparkling Aria's `onAfterMove`, once the move is over: unless the user is down or Sheer
    /// Force took the secondary, each target it hit that is still up is cured of a burn when the
    /// secondary's mark reached it — or whenever the move hit several targets (which gets past a
    /// Shield Dust). The marks go either way.
    /// Fling's and Natural Gift's `onPrepareHit` as the move goes ahead (`engine::prepared_move`
    /// set the power): Fling marks the item it throws (`VolatileCondition::Fling`: the user
    /// loses it at the next `Update`), Natural Gift uses its berry up (`setItem('')`, last item,
    /// `AfterUseItem`: not eaten). False when there is no item to throw or berry to use.
    /// Powder (the `powder` condition's `onTryMove`): its holder's Fire move fails, the holder
    /// losing a quarter of its max HP, rounded (`round(maxhp / 4)`, at least 1; Magic Guard
    /// refuses it).
    fn powder_stops(&mut self, actor: PokemonId, move_def: &ResolvedMove) -> Option<MoveExecution> {
        let pokemon = self.state.pokemon(actor)?;
        if move_def.move_type != crate::dex::PokemonType::Fire
            || !pokemon.volatiles.contains(VolatileConditionKind::Powder)
        {
            return None;
        }
        let amount = ((pokemon.max_hp + 2) / 4).max(1);
        self.trace.push(TraceEvent::MoveFailed {
            actor,
            move_id: move_def.id,
            reason: MoveFailReason::Powder,
        });
        BattleContext::new(self.state, self.rules, self.trace).apply_damage(
            crate::engine::DamageRequest {
                target: actor,
                amount: crate::engine::DamageAmount(amount),
                kind: crate::engine::DamageKind::Other,
                source: crate::engine::EffectSource::Volatile(VolatileConditionKind::Powder),
                flags: crate::engine::DamageFlags::default(),
            },
        );
        self.self_damage_cancelled = true;
        Some(MoveExecution {
            actor,
            move_id: move_def.id,
            hits: Vec::new(),
            stopped: false,
            aborted: false,
            result: MoveResult::Failed,
        })
    }

    fn prepare_item_move(&mut self, actor: PokemonId, move_def: &ResolvedMove) -> bool {
        let Some(pokemon) = self.state.pokemon(actor) else {
            return false;
        };
        if move_def.behavior.fling {
            let Some((item, _, _)) = crate::engine::fling_item(self.state, self.rules, pokemon)
            else {
                return false;
            };
            BattleContext::new(self.state, self.rules, self.trace).start_fling(actor, item);
            return true;
        }
        if crate::engine::natural_gift_berry(self.state, self.rules, pokemon).is_none() {
            return false;
        }
        // `pokemon.setItem('')`: no message.
        BattleContext::new(self.state, self.rules, self.trace).consume_item_unseen(actor);
        true
    }

    fn sparkling_aria(&mut self, actor: PokemonId, move_def: &ResolvedMove, hits: &[HitOutcome]) {
        let mut hit_targets: Vec<PokemonId> = Vec::new();
        for hit in hits {
            if let HitOutcome::Damaged { target, .. } | HitOutcome::SubstituteDamaged { target, .. } =
                hit
                && !hit_targets.contains(target)
            {
                hit_targets.push(*target);
            }
        }
        let cures = !move_def.flags.sheer_force
            && self.state.pokemon(actor).is_some_and(|p| !p.is_fainted());
        for target in hit_targets.iter().copied().filter(|t| *t != actor) {
            let marked = self
                .state
                .pokemon_mut(target)
                .is_some_and(|p| p.volatiles.remove(VolatileConditionKind::SparklingAria));
            if marked {
                self.trace.push(TraceEvent::VolatileEnded {
                    target,
                    kind: VolatileConditionKind::SparklingAria,
                    seen: true,
                });
            }
            let burned = self
                .state
                .pokemon(target)
                .is_some_and(|p| !p.is_fainted() && p.has_status(NonVolatileStatus::Burn));
            if cures && burned && (marked || hit_targets.len() > 1) {
                let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                ctx.cure_status(target, crate::engine::CureReason::Move(move_def.id));
            }
        }
    }

    /// Showdown `selfdestruct: 'ifHit'` (Memento, Final Gambit): the user faints once the move
    /// reached a target (`runMoveEffects`: `damage[i] !== false`).
    fn self_destruct_if_hit(&mut self, actor: PokemonId, move_def: &ResolvedMove, landed: bool) {
        if landed && move_def.self_destruct == Some(crate::dex::SelfDestruct::IfHit) {
            BattleContext::new(self.state, self.rules, self.trace)
                .faint_by_own_move(actor, move_def.id);
        }
    }

    /// Mind Blown / Steel Beam: the user loses a share of its max HP (rounded; Magic Guard
    /// refuses it; instafaint), once — after the first hit that struck something (inside the hit
    /// loop: a user it knocks out strikes no second Parental Bond hit, DESIGN.md R-M8-84), else
    /// once the move is over (Showdown `onAfterMove`). A move stopped at BeforeMove never gets
    /// here.
    pub(super) fn after_move_self_damage(&mut self, actor: PokemonId, move_id: MoveId) {
        if self.self_damage_paid {
            return;
        }
        let Some(fraction) = self
            .dex
            .move_by_id(move_id)
            .and_then(|move_def| move_def.behavior.self_damage_after_move)
        else {
            return;
        };
        let Some((max_hp, hp_before)) = self
            .state
            .pokemon(actor)
            .filter(|pokemon| !pokemon.is_fainted())
            .map(|pokemon| (pokemon.max_hp, pokemon.hp))
        else {
            return;
        };
        let amount = Rounding::HalfUp.apply(u32::from(max_hp), fraction) as u16;
        self.self_damage_paid = true;
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: actor,
            amount: DamageAmount(amount),
            // Not `recoil` (Showdown's effect is the move's condition): Magic Guard refuses it,
            // Rock Head does not.
            kind: crate::engine::DamageKind::Other,
            source: EffectSource::Move {
                pokemon: actor,
                move_id,
            },
            // `this.damage(…, true)`: a user this knocks out faints before the targets its move
            // knocked out.
            flags: crate::engine::DamageFlags {
                bypass_substitute: true,
                spare_one_hp: false,
                instafaint: true,
                single_strike: false,
                future_move: false,
            },
        });
        ctx.emergency_exit(actor, hp_before);
    }

    /// The encored move's slot when an Encore now locks the actor onto another move than the
    /// one it chose.
    /// Whether `move_id` comes with no PP left on a turn that would spend some (not a two-turn
    /// move's attack, a hard lock's later turn or Gen 1's binding lock).
    fn out_of_pp(&self, actor: PokemonId, move_id: MoveId) -> bool {
        let Some(pokemon) = self.state.pokemon(actor) else {
            return false;
        };
        let locked = pokemon.volatiles.charging_move() == Some(move_id)
            || pokemon
                .hard_lock()
                .is_some_and(|lock| lock.move_id == move_id)
            || pokemon.volatiles.partial_trapping_lock().is_some();
        !locked
            && pokemon
                .move_slots()
                .iter()
                .find(|slot| slot.move_id == move_id)
                .is_some_and(|slot| slot.pp == 0)
    }

    /// The Z power, spent as the move is used (Showdown `runMove`: `-zpower` and
    /// `side.zMoveUsed` once BeforeMove, the PP and `moveUsed` are through; `useMove` runs
    /// `runZPower` after the `move` line). A Z-Move that sleep, a flinch or a full paralysis
    /// stopped spends nothing: no status move's Z effect, and the side keeps its Z power
    /// (DESIGN.md R-M9-29).
    fn spend_z_power(&mut self, actor: PokemonId, z: &super::ZMoveExecution) {
        crate::engine::spend_mechanic(
            self.state,
            self.trace,
            actor,
            crate::engine::MechanicChoice::ZMove,
        )
        .expect("the Z-Move's user is in the battle");
        let crystal = self.state.pokemon(actor).and_then(|p| p.held_item);
        self.trace.push(TraceEvent::ZPower {
            actor,
            base: z.base,
            z_move: z.move_def.id,
            crystal,
        });
        if let Some(effect) = &z.status_effect {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            crate::engine::run_z_power(&mut ctx, actor, z.move_def.id, effect);
        }
    }

    /// The action's move `move_id` as its lines show it (`TraceEvent::MoveUsed::own_move`): the
    /// move itself, or the one a status Z-Move or a signature one was made from (`zMoveFrom`);
    /// none for Struggle, and a generic Z-Move or a Max Move, which hide theirs.
    fn shown_own_move(&self, move_id: MoveId) -> Option<MoveId> {
        if move_id == MoveId::STRUGGLE || self.max_base.is_some() {
            return None;
        }
        match &self.z_power {
            None => Some(move_id),
            Some(z) if z.move_def.id == z.base => Some(z.base),
            Some(z) => z
                .move_def
                .z_crystal
                .and_then(|crystal| self.dex.item_by_id(crystal))
                .and_then(|crystal| crystal.z_move_from),
        }
    }

    fn encore_override(&self, actor: PokemonId, chosen: MoveId) -> Option<(MoveSlot, MoveId)> {
        let pokemon = self.state.pokemon(actor)?;
        let encored = pokemon.volatiles.encored_move()?;
        if encored == chosen {
            return None;
        }
        let index = pokemon
            .move_slots()
            .iter()
            .position(|slot| slot.move_id == encored)?;
        Some((MoveSlot::new(index as u8)?, encored))
    }

    /// Showdown `gotAttacked` and `timesAttacked` after a move's hits on one target: a target
    /// the last hit reached (its HP, or its body for a status move) records the attack, and
    /// a damaging move's hits count towards Rage Fist. A last hit on a Substitute records
    /// nothing from Gen 4 (`spreadMoveHit` drops such a target from the move's list); Gen 3
    /// records it with no HP taken (`HitRules::records_substitute_hits`: the gen3 `tryMoveHit`'s
    /// `gotAttacked` whatever the hit did; pokeemerald's `MOVEEND_MIRROR_MOVE` writes the taken
    /// move unless it had no effect). Mirror Move finds it, Counter does not: Gen 3's answers
    /// HP damage (`onDamage`), R-M8-102.
    pub(super) fn record_attack(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &crate::dex::ResolvedMove,
        outcomes: &[HitOutcome],
    ) {
        let move_id = move_def.id;
        if actor == target {
            return;
        }
        let damage = match outcomes.last() {
            Some(HitOutcome::Damaged { amount, .. }) => Some(amount.0),
            Some(HitOutcome::Effect { .. }) => None,
            Some(HitOutcome::SubstituteDamaged { .. })
                if self.rules.hits.records_substitute_hits =>
            {
                None
            }
            _ => return,
        };
        let position = self.state.active_position(actor, &self.rules.topology);
        // The category the move was used with, not the dex's, for a move whose own
        // `onModifyMove` changes it — Shell Side Arm's estimate, Photon Geyser's and Light That
        // Burns the Sky's stats, a Terastallized Tera Blast's (R-M8-174): Showdown's
        // `getCategory(move)` reads the ActiveMove, which Counter and Mirror Coat answer
        // (DESIGN.md R-M8-44). Every other move's category (Gen 2-3's type-based one, Beat Up's
        // own) is the dex's (DESIGN.md R-M8-45) — but a Max Move's or a Z-Move's, which takes the
        // base move's category into the ActiveMove (`getActiveMaxMove` / `getActiveZMove`;
        // the dex has the Max Moves Physical): a Max Overgrowth from Giga Drain is special to
        // Counter (R-M9-40).
        let category = if move_def.behavior.announcement
            == Some(crate::effects::moves::Announcement::Category)
            || move_def.behavior.category_by_stats
            || move_def.behavior.tera_blast
            || self.z_powered
        {
            move_def.category
        } else {
            self.dex
                .move_by_id(move_id)
                .map_or(crate::dex::MoveCategory::Status, |m| m.category)
        };
        if let Some(pokemon) = self.state.pokemon_mut(target) {
            pokemon
                .history
                .attacked_by
                .retain(|record| record.source != actor);
            pokemon.history.attacked_by.push(AttackRecord {
                source: actor,
                damage,
                move_id,
                position,
                category,
                damaging_hit_only: false,
            });
            pokemon.history.attacked_this_turn.push(AttackRecord {
                source: actor,
                damage,
                move_id,
                position,
                category,
                damaging_hit_only: false,
            });
            if damage.is_some() {
                pokemon.history.times_attacked = pokemon
                    .history
                    .times_attacked
                    .saturating_add(outcomes.len() as u16);
            }
        }
    }

    /// A frozen user of a `defrost` move: thawed now (Gen 3+), or — returned `true` — once the
    /// move has landed (Gen 2).
    pub(super) fn defrost_user(&mut self, actor: PokemonId, move_def: &ResolvedMove) -> bool {
        use crate::rules::DefrostUser;
        let frozen = self
            .state
            .pokemon(actor)
            .is_some_and(|p| p.has_status(NonVolatileStatus::Freeze));
        if !frozen || !move_def.flags.defrost {
            return false;
        }
        match self.rules.status.lifecycle.defrost_user {
            DefrostUser::SkipsRoll | DefrostUser::AfterRoll => {
                self.thaw(actor, crate::engine::CureReason::Move(move_def.id));
                false
            }
            DefrostUser::AfterMove => true,
            DefrostUser::Never => false,
        }
    }

    /// Cures a frozen Pokémon (`-curestatus … frz`).
    pub(super) fn thaw(&mut self, pokemon: PokemonId, reason: crate::engine::CureReason) {
        if self
            .state
            .pokemon(pokemon)
            .is_some_and(|p| p.has_status(NonVolatileStatus::Freeze))
        {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            ctx.cure_status(pokemon, reason);
        }
    }
}

/// Showdown `moveThisTurnResult` for a move that got past BeforeMove and its targeting
/// (DESIGN.md §22.8). Showdown's hit loop counts a target as hit unless its step returned
/// `false`: a status move taken by a Substitute, or a Gen 8+ weather heal at full HP
/// (`NOT_FAIL`; Gen 7's `null` is a failure), still "hits". A target that protected itself is
/// neither hit nor a failure, so a move every target protected against is neither; anything
/// else refused, missed or changed nothing.
pub(super) fn move_result(
    rules: &ResolvedRules,
    move_def: &ResolvedMove,
    hits: &[HitOutcome],
    any_landed_hit: bool,
    move_failed: bool,
) -> MoveResult {
    let quiet = |hit: &HitOutcome| match hit {
        HitOutcome::Blocked {
            reason: HitBlockReason::Substitute,
            ..
        } => true,
        HitOutcome::NoEffect { .. } => match move_def.primary_effect {
            PrimaryEffect::Status(StatusMoveEffect::WeatherHeal(_)) => {
                rules.conditions.weather_heal_full_hp_not_fail
            }
            // Gen 8+ Purify with nothing to cure (`NOT_FAIL`; the gen7 mod's `false` fails).
            PrimaryEffect::Status(StatusMoveEffect::CureTarget(_)) => {
                rules.conditions.cure_target_nothing_not_fail
            }
            PrimaryEffect::Status(StatusMoveEffect::TargetHeal(_)) => {
                rules.conditions.target_heal_nothing_not_fail
            }
            _ => false,
        },
        _ => false,
    };
    let protected = |hit: &HitOutcome| {
        matches!(
            hit,
            HitOutcome::Blocked {
                reason: HitBlockReason::Protect
                    | HitBlockReason::WideGuard
                    | HitBlockReason::QuickGuard
                    | HitBlockReason::CraftyShield
                    | HitBlockReason::MatBlock,
                ..
            }
        )
    };
    if any_landed_hit || hits.iter().any(quiet) {
        MoveResult::Succeeded
    } else if !move_failed && !hits.is_empty() && hits.iter().all(protected) {
        MoveResult::Neither
    } else {
        MoveResult::Failed
    }
}

/// Whether a hit reached a body or a Substitute (Showdown: some target's damage is not `false`).
/// Whether the Pokémon's own move slots lack `move_id` (a locked move a caller brought in).
fn p_has_no_slot(
    state: &crate::state::BattleState,
    pokemon: crate::ids::PokemonId,
    move_id: MoveId,
) -> bool {
    state.pokemon(pokemon).is_some_and(|pokemon| {
        !pokemon
            .move_slots()
            .iter()
            .any(|slot| slot.move_id == move_id)
    })
}

fn struck_something(hits: &[HitOutcome]) -> bool {
    hits.iter().any(|hit| {
        matches!(
            hit,
            HitOutcome::Damaged { .. } | HitOutcome::SubstituteDamaged { .. }
        )
    })
}

fn actor_has_charge(state: &crate::state::BattleState, actor: PokemonId) -> bool {
    state
        .pokemon(actor)
        .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Charge))
}
