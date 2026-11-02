//! `BattleEngine`'s action queue (DESIGN.md §12): the turn's actions prepared and ordered —
//! priority, Quick Claw and Custap, speed ties drawn — the queue built and re-sorted, actions
//! moved up or back (Round, a Pledge, Shell Trap, Instruct, Quash) and taken one by one.

use crate::dex::MoveCategory;
use crate::effects::PriorityBoost;
use crate::engine::stats::action_speed;
use crate::engine::{
    ActionChoice, ActionOrder, ActionQueue, ActionStep, BattleError, MechanicChoice,
    PendingExecution, PriorityChance, QueuedAction, SideChoice, SlotChoice, TraceEvent,
};
use crate::ids::PokemonId;
use crate::state::{FieldConditionKind, RandomRequestKind, SelectedMove, VolatileConditionKind};

use super::*;

impl BattleEngine {
    /// Round's `onTry` (Showdown `queue.prioritizeAction`): the next queued action using
    /// `move_id`, either side's, goes to the front — ahead of everything still queued, the
    /// Gen 8+ re-sort included (its order becomes 3 in Showdown) — and remembers what moved it.
    /// Nothing reads the queue between the `onTry` and the end of the move, so the move reports
    /// it and the queue changes afterwards.
    pub(super) fn prioritize_same_move(&mut self, move_id: crate::ids::MoveId) {
        let state = &self.state;
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        // The move the action uses as the turn began (`action.move`): passed over when it is a
        // Max Move or a Z-Move (`action.maxMove || action.zmove`: a Dynamaxed ally's Round is Max
        // Strike, DESIGN.md R-M9-27) or a recharge, the slot's Round a recharging Pokémon names.
        let uses = |action: &QueuedAction| {
            action.step == ActionStep::Act
                && matches!(action.choice, ActionChoice::Move { .. })
                && walked_move(state, action) == Some(move_id)
        };
        let Some(index) = actions.iter().position(uses) else {
            return;
        };
        let mut action = actions.remove(index);
        action.order.action_priority = self.rules.action_order.prioritized_action_priority;
        action.prioritized_by = Some(move_id);
        actions.insert(0, action);
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: crate::engine::QueueReorderReason::Prioritized,
        });
    }

    /// Whether the queue is re-sorted before the next action (Gen 8+, a move comes next).
    pub(super) fn resorts_before_next(&self) -> bool {
        matches!(
            &self.pending,
            PendingExecution::Turn {
                actions,
                strategy: crate::rules::ActionOrderStrategy::RecalculateAfterAction,
                ..
            } if !actions.is_empty() && next_is_move(actions)
        )
    }

    /// Instruct's injected action (DESIGN.md §42.9): the target uses the move in `slot` (its
    /// last move) at once — a fresh action at the front of the queue, its own action for the
    /// turn untouched (`queue.resolveAction` + `prioritizeAction`). Its order is the one the move
    /// would have had (its priority and the actor's speed), as Showdown's `resolveAction` builds
    /// it.
    pub(super) fn instruct_action(
        &mut self,
        actor: PokemonId,
        slot: usize,
        target_position: Option<crate::ids::PositionId>,
    ) {
        let Some(slot) = crate::ids::MoveSlot::new(slot as u8) else {
            return;
        };
        if self.state.pokemon(actor).is_none()
            || self
                .state
                .active_position(actor, &self.rules.topology)
                .is_none()
        {
            return;
        }
        if !matches!(self.pending, PendingExecution::Turn { .. }) {
            return;
        }
        // Showdown builds the action as any (`resolveAction`): `ModifyPriority` (a Prankster
        // boost stays with the move), and `FractionalPriority`'s chances roll and announce
        // themselves (Quick Draw, Quick Claw; a Custap Berry at a pinch is eaten) — though
        // `prioritizeAction` puts it first whatever they give (R-M9-32).
        let choice = SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot,
                target: target_position.map(crate::rules::DeclaredTarget),
                mechanic: None,
            },
        };
        let Ok(prepared) = self.prepare_action(&choice, None, false) else {
            return;
        };
        let fractional = self.roll_fractional(&prepared);
        let order = ActionOrder {
            action_priority: self.rules.action_order.prioritized_action_priority,
            ..self.settled_order(&prepared, fractional)
        };
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        actions.insert(
            0,
            QueuedAction {
                actor,
                choice: ActionChoice::Move {
                    slot,
                    target: target_position.map(crate::rules::DeclaredTarget),
                    mechanic: None,
                },
                step: crate::engine::ActionStep::Act,
                order,
                prioritized_by: None,
                instructed: true,
                original_target: target_position.and_then(|position| {
                    self.state
                        .pokemon_at_position(position, &self.rules.topology)
                }),
            },
        );
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: crate::engine::QueueReorderReason::Prioritized,
        });
    }

    /// A Pledge's promotion (DESIGN.md §42.9): the first queued action of an *ally* using
    /// `move_id` goes to the front, with `by` (the waiting move) as its `sourceEffect`.
    pub(super) fn prioritize_ally_move(
        &mut self,
        by_move: crate::ids::MoveId,
        by: PokemonId,
        move_id: crate::ids::MoveId,
    ) {
        let side = self.state.pokemon(by).map(|pokemon| pokemon.side);
        let state = &self.state;
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        let Some(index) = actions.iter().position(|action| {
            side == state.pokemon(action.actor).map(|pokemon| pokemon.side)
                && action.step == ActionStep::Act
                && matches!(action.choice, ActionChoice::Move { .. })
                && walked_move(state, action) == Some(move_id)
        }) else {
            return;
        };
        let mut action = actions.remove(index);
        action.order.action_priority = self.rules.action_order.prioritized_action_priority;
        action.prioritized_by = Some(by_move);
        actions.insert(0, action);
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: crate::engine::QueueReorderReason::Prioritized,
        });
    }

    /// Showdown `queue.prioritizeAction` for Shell Trap: `actor`'s pending action goes to the
    /// front of the queue.
    pub(super) fn prioritize_actor(&mut self, actor: PokemonId) {
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        let Some(index) = actions.iter().position(|action| action.actor == actor) else {
            return;
        };
        let mut action = actions.remove(index);
        action.order.action_priority = self.rules.action_order.prioritized_action_priority;
        actions.insert(0, action);
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: crate::engine::QueueReorderReason::Prioritized,
        });
    }

    /// Quash's request (DESIGN.md §42.4): `actor`'s pending action goes last — to the end of
    /// the queue (Gen 5-7), or into the bracket after every move the next re-sort orders (Gen
    /// 8+).
    pub(super) fn quash_actor(&mut self, actor: PokemonId) {
        let quash = self.rules.action_order.quash;
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        let Some(index) = actions.iter().position(|action| action.actor == actor) else {
            return;
        };
        match quash {
            crate::rules::QuashOrder::End => {
                let action = actions.remove(index);
                actions.push(action);
            }
            crate::rules::QuashOrder::LastBracket(priority) => {
                actions[index].order.action_priority = priority;
            }
        }
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: crate::engine::QueueReorderReason::Quashed,
        });
    }

    /// Whether the next queued action of the turn is a switch.
    pub(super) fn next_pending_is_switch(&self) -> bool {
        matches!(
            &self.pending,
            PendingExecution::Turn { actions, .. }
                if actions
                    .first()
                    .is_some_and(|action| matches!(action.choice, ActionChoice::Switch { .. }))
        )
    }

    pub(super) fn take_next_pending_action(&mut self) -> Option<QueuedAction> {
        // Gen 8+: every pending action's order as the re-sort will see it.
        let refreshed: Vec<ActionOrder> = match &self.pending {
            PendingExecution::Turn {
                actions,
                strategy: crate::rules::ActionOrderStrategy::RecalculateAfterAction,
                ..
            } => actions
                .iter()
                .map(|action| self.refreshed_order(action))
                .collect(),
            _ => Vec::new(),
        };
        let PendingExecution::Turn {
            actions,
            strategy,
            resort_ties,
            ..
        } = &mut self.pending
        else {
            return None;
        };
        if actions.is_empty() {
            return None;
        }

        if *strategy == crate::rules::ActionOrderStrategy::RecalculateAfterAction {
            // Gen 8+ `updateSpeed` after an action followed by a move.
            if *resort_ties && next_is_move(actions) {
                crate::engine::stats::update_handler_speeds(&mut self.state, &self.rules);
            }
            let trick_room = self.state.field.contains(FieldConditionKind::TrickRoom);
            for (action, order) in actions.iter_mut().zip(refreshed) {
                action.order = order;
            }
            // Showdown re-sorts the queue after every action followed by a move, and its
            // `speedSort` shuffles each group of tied actions anew: one SpeedTie decision per
            // tied action, in queue order (as at the start of the turn).
            if *resort_ties && next_is_move(actions) {
                let keys: Vec<_> = actions.iter().map(|action| action.order.key()).collect();
                for (index, action) in actions.iter_mut().enumerate() {
                    if tied_at(&keys, index) {
                        action.order.tie_break =
                            self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
                    }
                }
            }
            for action in actions.iter() {
                self.trace.push(TraceEvent::ActionOrderResolved {
                    actor: action.actor,
                    action_priority: action.order.action_priority,
                    move_priority: action.order.move_priority,
                    fractional_priority: action.order.fractional_priority,
                    effective_speed: action.order.speed,
                    trick_room,
                    tie_break: action.order.tie_break,
                });
            }
            ActionQueue::sort_actions(actions, trick_room);
            self.trace.push(TraceEvent::ActionQueueReordered {
                reason: crate::engine::QueueReorderReason::DynamicSpeed,
            });
        }

        *resort_ties = true;
        Some(actions.remove(0))
    }

    internal!(
        /// The Pokémon that draw a `SpeedTie` as Neutralizing Gas clears and `holder`'s `End`
        /// starts the others' abilities again (`BattleContext::end_ability`), in draw order: a
        /// replay plans them from the order the abilities announce themselves.
        pub fn neutralizing_gas_restart_ties(&self, holder: PokemonId) -> Vec<PokemonId> {
            let speeds: Vec<(PokemonId, u32)> = self
                .active_pokemon()
                .into_iter()
                .filter(|other| *other != holder)
                .map(|id| {
                    (
                        id,
                        crate::engine::stats::action_speed(&self.state, &self.rules, id, None),
                    )
                })
                .collect();
            crate::engine::context::restart_ties(&self.state, &self.rules, &speeds)
        }
    );

    internal!(
        /// The SpeedTie decisions of the step about to run (Gen 8+ re-sorts the queue before each
        /// first action, when a move comes next): each tied pending action with its group, in the
        /// order the SpeedTie decisions are drawn. Empty when the step draws none. Lets a replay
        /// plan those decisions from the order Showdown reported.
        pub fn pending_speed_ties(&self) -> Vec<(PokemonId, usize)> {
            let PendingExecution::Turn {
                actions,
                strategy,
                resort_ties,
                ..
            } = &self.pending
            else {
                return Vec::new();
            };
            if *strategy != crate::rules::ActionOrderStrategy::RecalculateAfterAction
                || !*resort_ties
                || !next_is_move(actions)
            {
                return Vec::new();
            }
            let keys: Vec<_> = actions
                .iter()
                .map(|action| self.refreshed_order(action).key())
                .collect();
            let mut groups: Vec<(i16, i8, i8, u32)> = Vec::new();
            let mut tied = Vec::new();
            for (index, action) in actions.iter().enumerate() {
                if !tied_at(&keys, index) {
                    continue;
                }
                let group = groups
                    .iter()
                    .position(|g| *g == keys[index])
                    .unwrap_or_else(|| {
                        groups.push(keys[index]);
                        groups.len() - 1
                    });
                tied.push((action.actor, group));
            }
            tied
        }
    );

    internal!(
        /// The Pokémon the switches still queued this turn will bring in (a turn a mid-turn
        /// request paused: an Eject Pack's). Lets a replay keep its party order for them.
        pub fn pending_switch_ins(&self) -> Vec<PokemonId> {
            let PendingExecution::Turn { actions, .. } = &self.pending else {
                return Vec::new();
            };
            actions
                .iter()
                .filter_map(|action| match action.choice {
                    ActionChoice::Switch { pokemon } => Some(pokemon),
                    _ => None,
                })
                .collect()
        }
    );

    /// The ordering of one declared action before its priority chances are rolled: action
    /// priority, move priority, the fixed fractional priority and the chances after it
    /// (DESIGN.md §12.4), and the action speed (`quick_claw` overrides the turn's Gen 2-3
    /// Quick Claw roll for a preview).
    /// `struggles`: whether the move action is Struggle — decided as the action is chosen
    /// (`must_struggle`: Showdown's `chooseMove` with no move left), and from then on what the
    /// turn decided (`state.struggling`): a later re-sort keeps the chosen move's priority, a
    /// move sealed since included (R-M10-8).
    fn prepare_action(
        &self,
        slot_choice: &SlotChoice,
        quick_claw: Option<bool>,
        struggles: bool,
    ) -> Result<PreparedAction, BattleError> {
        let actor = slot_choice.actor;
        let speed = action_speed(&self.state, &self.rules, actor, quick_claw);
        // The move the action runs with (its priority and category): `None` for a switch or a
        // pass, Showdown's `recharge` pseudo-move for a recharging Pokémon (priority 0; not a
        // real move, so no category at all: Quick Draw, which only skips Status moves, still
        // rolls), Struggle (priority 0, Physical) for one with no move left.
        let mut prankster_boosted = false;
        let (action_priority, move_priority, category) = match slot_choice.action {
            ActionChoice::Move { .. }
                if self.state.pokemon(actor).is_some_and(|pokemon| {
                    pokemon
                        .volatiles
                        .contains(VolatileConditionKind::MustRecharge)
                }) =>
            {
                (self.rules.action_order.move_action_priority, 0, Some(None))
            }
            // Struggle as the action was chosen: an item lost since (an Assault Vest Corrosive
            // Gas took) leaves the action Struggle when the Gen 8+ queue re-sorts, and a move
            // sealed since (a Taunt on a Pokémon of status moves alone) leaves it its move.
            ActionChoice::Move { .. } if struggles => (
                self.rules.action_order.move_action_priority,
                0,
                Some(Some(MoveCategory::Physical)),
            ),
            // Gen 1 binding (§30.9): a bound Pokémon's action is `cannotmove` (priority 0) unless
            // it is held to a move (Bide: that move); one locked into its binding move uses that
            // move.
            ActionChoice::Move { .. }
                if self.rules.conditions.switching.binding_locks_user
                    && self.state.pokemon(actor).is_some_and(|pokemon| {
                        !pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
                            && !pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                            && ((pokemon
                                .volatiles
                                .contains(VolatileConditionKind::PartiallyTrapped)
                                && !binding::held_to_a_move(pokemon))
                                || pokemon.volatiles.partial_trapping_lock().is_some())
                    }) =>
            {
                let locked = self
                    .state
                    .pokemon(actor)
                    .and_then(|pokemon| pokemon.volatiles.partial_trapping_lock())
                    .and_then(|(_, move_id, ..)| self.dex.move_by_id(move_id));
                (
                    self.rules.action_order.move_action_priority,
                    locked.map_or(0, |move_def| move_def.priority),
                    Some(locked.map(|move_def| move_def.category)),
                )
            }
            // Gen 1: a frozen or sleeping Pokémon acts with its side's last selected move.
            ActionChoice::Move { .. }
                if self
                    .rules
                    .action_order
                    .sleeping_frozen_act_with_last_selected
                    && self.state.pokemon(actor).is_some_and(|pokemon| {
                        pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
                            || pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                    }) =>
            {
                let selected = self
                    .state
                    .pokemon(actor)
                    .and_then(|pokemon| self.state.side(pokemon.side).last_selected_move)
                    .and_then(SelectedMove::move_id)
                    .and_then(|move_id| self.dex.move_by_id(move_id));
                (
                    self.rules.action_order.move_action_priority,
                    selected.map_or(0, |move_def| move_def.priority),
                    Some(Some(selected.map_or(MoveCategory::Physical, |move_def| {
                        move_def.category
                    }))),
                )
            }
            ActionChoice::Move { slot, mechanic, .. } => {
                let move_id = self
                    .action_move(actor, slot)
                    .ok_or(BattleError::InvalidMoveSlot { actor, slot })?;
                let move_def = self
                    .dex
                    .move_by_id(move_id)
                    .ok_or(BattleError::UnknownMove(move_id))?;
                // A Dynamaxed Pokémon's move is its Max Move, and `getActionSpeed` reads the
                // priority of the move it will use — `dex.moves.get(maxMove.id).priority`, which
                // Max Guard has at +4 (DESIGN.md §44.5).
                let modified = crate::engine::modified_move_cow(
                    &self.state,
                    &self.rules,
                    actor,
                    move_def,
                    false,
                )
                .0;
                // A damaging move's Z-Move orders by its own priority too (`getActionSpeed`:
                // `dex.moves.get(zMove.id).priority`, 0 — Black Hole Eclipse from Sucker Punch
                // goes at 0, though as it runs it keeps the base move's, R-M9-35); a status
                // move's Z version is the move itself.
                let z_ordering = (mechanic == Some(MechanicChoice::ZMove))
                    .then(|| {
                        crate::engine::z_move_execution(
                            &self.state,
                            &self.rules,
                            &self.dex,
                            actor,
                            move_def,
                        )
                    })
                    .flatten()
                    .filter(|z| z.move_def.id != move_def.id)
                    .and_then(|z| self.dex.move_by_id(z.move_def.id));
                let ordering_def = (mechanic == Some(MechanicChoice::Dynamax)
                    || self.state.pokemon(actor).is_some_and(|pokemon| {
                        pokemon.volatiles.contains(VolatileConditionKind::Dynamax)
                    }))
                .then(|| crate::engine::max_move_for(&self.dex, &modified))
                .flatten()
                .and_then(|max| self.dex.move_by_id(max))
                .or(z_ordering)
                .unwrap_or(move_def);
                let (priority, prankster) = self.modified_priority(actor, ordering_def);
                prankster_boosted = prankster;
                (
                    self.rules.action_order.move_action_priority,
                    priority,
                    Some(Some(modified.category)),
                )
            }
            ActionChoice::Switch { .. } => {
                (self.rules.action_order.switch_action_priority, 0, None)
            }
            ActionChoice::Pass => (self.rules.action_order.pass_action_priority, 0, None),
        };
        let (fractional, chances) = match category {
            Some(category) => fractional_priority(&self.state, &self.rules, actor, category),
            None => (0, Vec::new()),
        };
        let custap = category.is_some() && self.custap_goes_off(actor);
        Ok(PreparedAction {
            actor,
            choice: slot_choice.action.clone(),
            action_priority,
            move_priority,
            prankster_boosted,
            fractional,
            chances,
            custap,
            speed,
        })
    }

    /// The move a move action uses: a locked Pokémon's lock (Showdown's request offers
    /// `getLockedMove` / `getSemiLockedMove` alone, and the action names it — a called Bide or
    /// two-turn move outside the slots too), else the slot's. Its priority orders the action
    /// (R-M8-72) and Me First / Sucker Punch read it (R-M8-66).
    pub(super) fn action_move(
        &self,
        actor: PokemonId,
        slot: crate::ids::MoveSlot,
    ) -> Option<crate::ids::MoveId> {
        let pokemon = self.state.pokemon(actor)?;
        match pokemon
            .hard_lock()
            .map(|lock| lock.move_id)
            .or_else(|| pokemon.semi_locked_move())
        {
            Some(locked) => Some(locked),
            None => pokemon.move_slot(slot).map(|slot| slot.move_id),
        }
    }

    /// Custap Berry (Gen 4 `onBeforeTurn`, Gen 5+ `onFractionalPriority`, priority −2: after
    /// Stall and the chances): the holder is at a quarter of its HP (half with Gluttony) and would
    /// eat it — no foe's Unnerve refuses it.
    fn custap_goes_off(&self, actor: PokemonId) -> bool {
        let Some(pokemon) = self.state.pokemon(actor) else {
            return false;
        };
        let gluttony =
            crate::engine::ability_handler(&self.state, &self.rules, pokemon, |b| b.gluttony);
        crate::engine::held_item(&self.state, &self.rules, actor)
            .is_some_and(|(_, behavior)| behavior.pinch_priority)
            && self.rules.conditions.berries.reached(
                crate::rules::HpShare::Pinch,
                pokemon.hp,
                pokemon.max_hp,
                gluttony,
            )
            && !crate::engine::context::unnerved_by_a_foe(&self.state, &self.rules, actor)
    }

    /// A Custap Berry that went off (`PreparedAction::custap`) puts the action first in its
    /// bracket: Gen 5+ the holder eats it now (`-activate … [consumed]`), Gen 4 as its move
    /// begins (`BattleState::custap_ready`). False when the holder could not eat it after all.
    fn custap(&mut self, actor: PokemonId) -> bool {
        if self.rules.action_order.custap_eaten_as_the_move_begins {
            self.state.custap_ready.push(actor);
            return true;
        }
        let mut ctx =
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        match ctx.eat_item(actor) {
            Some(item) => {
                ctx.item_activated(actor, item);
                true
            }
            None => false,
        }
    }

    /// Showdown `ModifyPriority` (DESIGN.md §12.5) on the user's ability: Prankster +1 for a
    /// status move, which is then `pranksterBoosted`; Gale Wings +1 for a Flying move (from
    /// Gen 7 at full HP); Triage +3 for a healing move — after the move's own (Grassy Glide +1
    /// in Grassy Terrain for a grounded user).
    fn modified_priority(
        &self,
        actor: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) -> (i8, bool) {
        let mut priority = move_def.priority;
        let Some(pokemon) = self.state.pokemon(actor) else {
            return (priority, false);
        };
        if let Some(terrain) = move_def.behavior.terrain_priority
            && self.state.field.terrain.map(|t| t.kind) == Some(terrain)
            && crate::engine::environment::grounded(&self.state, &self.rules, pokemon)
        {
            priority += 1;
        }
        let mut prankster = false;
        match crate::engine::ability_handler(&self.state, &self.rules, pokemon, |b| {
            b.modify_priority
        }) {
            Some(PriorityBoost::Prankster) if move_def.category == MoveCategory::Status => {
                priority += 1;
                prankster = true;
            }
            Some(PriorityBoost::GaleWings)
                if move_def.move_type == crate::dex::PokemonType::Flying
                    && (!self.rules.action_order.gale_wings_full_hp
                        || pokemon.hp == pokemon.max_hp) =>
            {
                priority += 1;
            }
            Some(PriorityBoost::Triage) if move_def.flags.heal => priority += 3,
            _ => {}
        }
        (priority, prankster)
    }

    /// A queued action's order as a Gen 8+ re-sort sees it (Showdown `getActionSpeed` again):
    /// its move priority re-run (Gale Wings may have lost its full HP) and its speed updated;
    /// the fractional priority and tie break stay.
    pub(super) fn refreshed_order(&self, action: &QueuedAction) -> ActionOrder {
        let mut order = action.order;
        order.speed = action_speed(&self.state, &self.rules, action.actor, None);
        let slot_choice = SlotChoice {
            actor: action.actor,
            action: action.choice.clone(),
        };
        let struggles = self.state.struggling.contains(&action.actor);
        if let Ok(prepared) = self.prepare_action(&slot_choice, None, struggles) {
            order.move_priority = prepared.move_priority;
            order.prankster_boosted = prepared.prankster_boosted;
        }
        order
    }

    /// Showdown `FractionalPriority` for a move action (DESIGN.md §12.4): Lagging Tail / Full
    /// Showdown `FractionalPriority` for a move action `resolveAction` builds (DESIGN.md §12.4):
    /// its chances in handler order, each that goes off announced, then a Custap Berry at a
    /// pinch (eaten now from Gen 5). The fractional priority they leave.
    fn roll_fractional(&mut self, action: &PreparedAction) -> i8 {
        let mut fractional = settle_fractional(action, |chance| {
            let (numerator, denominator) = chance.odds();
            let rolled = self.state.rng.roll_ratio_for(
                RandomRequestKind::PriorityChance,
                numerator,
                denominator,
            );
            if rolled {
                self.trace.push(activation(action.actor, chance));
                self.state.priority_chance_activated.push(action.actor);
            }
            rolled
        });
        if action.custap && fractional <= 0 && self.custap(action.actor) {
            fractional = 1;
        }
        fractional
    }

    /// Declared actions in canonical order (P1 then P2, by position), as Showdown's
    /// `resolveAction` meets them (and rolls their priority chances).
    fn prepared_actions(
        &self,
        choices: &[SideChoice; 2],
        quick_claw: Option<bool>,
    ) -> Result<Vec<PreparedAction>, BattleError> {
        let mut prepared = Vec::new();
        for side_choice in choices {
            let mut canonical_actions = side_choice.actions.iter().collect::<Vec<_>>();
            canonical_actions.sort_by_key(|slot_choice| {
                self.state
                    .active_position(slot_choice.actor, &self.rules.topology)
                    .map(|position| position.index())
                    .unwrap_or(usize::MAX)
            });
            for slot_choice in canonical_actions {
                let struggles = crate::engine::choice::must_struggle(
                    &self.state,
                    &self.rules,
                    &self.dex,
                    slot_choice.actor,
                );
                prepared.push(self.prepare_action(slot_choice, quick_claw, struggles)?);
            }
        }
        Ok(prepared)
    }

    /// The ordering of a prepared action once its chances are settled (Gen 4: a delayed move
    /// orders slowest first).
    fn settled_order(&self, action: &PreparedAction, fractional: i8) -> ActionOrder {
        ActionOrder {
            action_priority: action.action_priority,
            move_priority: action.move_priority,
            fractional_priority: fractional,
            speed: action.speed,
            slowest_first: fractional < 0 && self.rules.action_order.delayed_slowest_first,
            fastest_first: fractional > 0 && self.rules.action_order.boosted_fastest_first,
            tie_break: 0,
            prankster_boosted: action.prankster_boosted,
        }
    }

    /// Whether some Pokémon of the battle could use this turn's Gen 2-3 Quick Claw roll: the
    /// roll is only drawn then.
    fn quick_claw_turn_holder(&self) -> Option<PokemonId> {
        self.rules.action_order.quick_claw_turn_roll?;
        self.state
            .all_pokemon()
            .iter()
            .find(|pokemon| {
                !pokemon.is_fainted()
                    && crate::engine::held_item_id(&self.state, &self.rules, pokemon.id())
                        == Some(crate::ids::ItemId::QUICK_CLAW)
            })
            .map(|pokemon| pokemon.id())
    }

    /// Showdown `side.commitChoices` (`lastSelectedMove`): a move picked from the menu becomes
    /// the side's last selected move, Struggle too; a partially trapped Pokémon's turn clears
    /// it (`cannotmove`); a frozen, sleeping, recharging or otherwise locked Pokémon leaves it.
    pub(super) fn record_selected_moves(&mut self, choices: &[SideChoice; 2]) {
        for side_choice in choices {
            for slot_choice in &side_choice.actions {
                let ActionChoice::Move { slot, .. } = slot_choice.action else {
                    continue;
                };
                let actor = slot_choice.actor;
                let Some(pokemon) = self.state.pokemon(actor) else {
                    continue;
                };
                let mut selected_slot = None;
                let selected = if pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
                    || pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                    || binding::held_to_a_move(pokemon)
                    // The recharge turn's `recharge` choice has no slot (R-M8-107).
                    || crate::engine::choice::recharging(&self.state, actor)
                {
                    continue;
                } else if pokemon
                    .volatiles
                    .contains(VolatileConditionKind::PartiallyTrapped)
                {
                    Some(SelectedMove::CannotMove)
                } else if crate::engine::choice::must_struggle(
                    &self.state,
                    &self.rules,
                    &self.dex,
                    actor,
                ) {
                    Some(SelectedMove::Move(crate::ids::MoveId::STRUGGLE))
                } else {
                    selected_slot = Some(slot);
                    pokemon
                        .move_slot(slot)
                        .map(|slot| SelectedMove::Move(slot.move_id))
                };
                let side = pokemon.side;
                self.state.side_mut(side).last_selected_move = selected;
                // Gen 1 Mimic reads this back (`side.lastSelectedMoveSlot`): only a pick from the
                // menu sets it; frozen, sleeping and locked Pokémon leave it as it was.
                if let Some(slot) = selected_slot {
                    self.state.side_mut(side).last_selected_move_slot = slot.index();
                }
            }
        }
    }

    internal!(
        /// The turn order these choices would produce, with every priority chance decided by
        /// `activated` instead of drawn (DESIGN.md §12.4): a replay that knows the outcomes (from
        /// an oracle's log or action order) can pre-fill the semantic tape. The Gen 2-3 turn roll
        /// is asked of the first Quick Claw holder.
        pub fn preview_action_order(
            &self,
            choices: &[SideChoice; 2],
            activated: &dyn Fn(PokemonId, PriorityChance) -> bool,
        ) -> Result<ActionOrderPreview, BattleError> {
            let mut chances = Vec::new();
            let quick_claw = self.quick_claw_turn_holder().map(|holder| {
                let rolled = activated(holder, PriorityChance::QuickClawTurn);
                chances.push((holder, PriorityChance::QuickClawTurn, rolled));
                rolled
            });
            let prepared = self.prepared_actions(choices, Some(quick_claw.unwrap_or(false)))?;
            let mut orders = Vec::new();
            for action in &prepared {
                let mut fractional = settle_fractional(action, |chance| {
                    let rolled = activated(action.actor, chance);
                    chances.push((action.actor, chance, rolled));
                    rolled
                });
                if action.custap && fractional <= 0 {
                    fractional = 1;
                }
                orders.push((action.actor, self.settled_order(action, fractional)));
            }
            let keys: Vec<_> = orders.iter().map(|(_, order)| order.key()).collect();
            let mut groups: Vec<(i16, i8, i8, u32)> = Vec::new();
            let mut ties = Vec::new();
            for (index, (actor, _)) in orders.iter().enumerate() {
                if !tied_at(&keys, index) {
                    continue;
                }
                let group = groups
                    .iter()
                    .position(|g| *g == keys[index])
                    .unwrap_or_else(|| {
                        groups.push(keys[index]);
                        groups.len() - 1
                    });
                ties.push((*actor, group));
            }
            let trick_room = self.state.field.contains(FieldConditionKind::TrickRoom);
            let mut queued: Vec<QueuedAction> = prepared
                .iter()
                .zip(&orders)
                .map(|(action, (actor, order))| QueuedAction {
                    actor: *actor,
                    choice: action.choice.clone(),
                    step: ActionStep::of(&action.choice),
                    order: *order,
                    prioritized_by: None,
                    instructed: false,
                    original_target: crate::engine::action::original_target(
                        &self.state,
                        &self.rules.topology,
                        &action.choice,
                    ),
                })
                .collect();
            ActionQueue::sort_actions(&mut queued, trick_room);
            let mut order: Vec<Vec<PokemonId>> = Vec::new();
            let mut last_key = None;
            for action in &queued {
                let key = action.order.key();
                match order.last_mut() {
                    Some(bracket) if last_key == Some(key) => bracket.push(action.actor),
                    _ => order.push(vec![action.actor]),
                }
                last_key = Some(key);
            }
            Ok(ActionOrderPreview {
                chances,
                order,
                ties,
            })
        }
    );

    pub(super) fn build_action_queue(
        &mut self,
        choices: &[SideChoice; 2],
    ) -> Result<ActionQueue, BattleError> {
        // Gen 2-3: the turn's one Quick Claw roll (Showdown draws it as the turn starts).
        self.state.quick_claw_roll = false;
        if let (Some(odds), Some(_)) = (
            self.rules.action_order.quick_claw_turn_roll,
            self.quick_claw_turn_holder(),
        ) {
            self.state.quick_claw_roll = self.state.rng.roll_ratio_for(
                RandomRequestKind::PriorityChance,
                odds.numerator,
                odds.denominator,
            );
        }
        // Showdown `commitChoices`: `updateSpeed` before the queue is sorted.
        crate::engine::stats::update_handler_speeds(&mut self.state, &self.rules);
        let prepared = self.prepared_actions(choices, None)?;

        // Showdown `resolveAction`: every move action rolls its priority chances, in canonical
        // order, before the queue is sorted; a chance that goes off announces itself.
        self.state.priority_chance_activated.clear();
        let mut orders = Vec::new();
        self.state.custap_ready.clear();
        for action in &prepared {
            let fractional = self.roll_fractional(action);
            orders.push(self.settled_order(action, fractional));
        }

        // A move choice with a gimmick becomes two queue entries (DESIGN.md §44.2): the gimmick's
        // step (order 106, its own action priority, the move's speed) and the move itself.
        let trick_room = self.state.field.contains(FieldConditionKind::TrickRoom);
        let mut entries: Vec<(ActionStep, ActionOrder, ActionChoice, PokemonId)> = Vec::new();
        for (action, order) in prepared.into_iter().zip(orders) {
            let step = ActionStep::of(&action.choice);
            if step != ActionStep::Act {
                entries.push((
                    step,
                    ActionOrder {
                        action_priority: step.action_priority(&self.rules.action_order, 0),
                        move_priority: 0,
                        fractional_priority: 0,
                        speed: order.speed,
                        slowest_first: false,
                        fastest_first: false,
                        tie_break: 0,
                        prankster_boosted: false,
                    },
                    action.choice.clone(),
                    action.actor,
                ));
                // The move action keeps no mechanic: the gimmick is its own step (a Dynamaxed
                // Pokémon's later moves convert on their own, Q2's answer).
                entries.push((
                    ActionStep::Act,
                    order,
                    without_mechanic(action.choice),
                    action.actor,
                ));
                continue;
            }
            entries.push((ActionStep::Act, order, action.choice, action.actor));
        }
        // A SpeedTie decision is drawn only for actions that genuinely tie (same priorities and
        // speed), one per tied action in canonical order; Showdown draws nothing otherwise.
        let keys: Vec<_> = entries.iter().map(|(_, order, _, _)| order.key()).collect();
        let mut queue = ActionQueue::default();
        for (index, (step, mut order, choice, actor)) in entries.into_iter().enumerate() {
            if tied_at(&keys, index) {
                order.tie_break = self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
            }
            self.trace.push(TraceEvent::ActionOrderResolved {
                actor,
                action_priority: order.action_priority,
                move_priority: order.move_priority,
                fractional_priority: order.fractional_priority,
                effective_speed: order.speed,
                trick_room,
                tie_break: order.tie_break,
            });
            let original_target =
                crate::engine::action::original_target(&self.state, &self.rules.topology, &choice);
            queue.push(QueuedAction {
                actor,
                choice,
                step,
                order,
                prioritized_by: None,
                instructed: false,
                original_target,
            });
        }

        queue.sort_with_trick_room(trick_room);
        Ok(queue)
    }
}
