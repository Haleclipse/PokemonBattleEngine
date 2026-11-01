//! The switch, faint and residual lifecycles of `BattleEngine` (DESIGN.md §30.1, ROADMAP §7):
//! switch-in stage with entry hazards, the residual scheduler, the faint step and the
//! end-of-turn condition expiry. Split out of `battle/mod.rs` so the turn state machine stays
//! readable; every method here is still `BattleEngine`'s.

use crate::dex::PokemonType;
use crate::effects;
use crate::ids::{ItemId, MoveId, PokemonId, PositionId, SideId};
use crate::rules::{
    FaintTurnEffect, ResidualKind, StatusResidualTiming, SwitchInOrder, ToxicModel,
};
use crate::state::{
    BattleOutcome, BattlePhase, NonVolatileStatus, RandomRequestKind, SideCondition,
    SideConditionKind, StatStages, VolatileConditionKind, WeatherKind,
};

use super::BattleEngine;
use crate::engine::stats::action_speed;
use crate::engine::{
    BattleContext, BattleError, DamageApplication, EffectSource, PendingExecution, PendingFaints,
    TraceEvent,
};

impl BattleEngine {
    internal!(
        pub fn run_switch_in(&mut self, source: PokemonId) {
            self.run_switch_ins(&[source]);
        }
    );

    /// Showdown `runSwitch` for Pokémon that came in together, fastest first (DESIGN.md
    /// §30.1): Gen 1-4 each in turn, the faints of one settled before the next; Gen 5+ one
    /// `fieldEvent('SwitchIn')` over all of them (`ConditionRules::switch_in_order`).
    pub(super) fn run_switch_ins(&mut self, entrants: &[PokemonId]) {
        match self.rules.conditions.switch_in_order {
            SwitchInOrder::EachInTurn => {
                for (index, &pokemon) in entrants.iter().enumerate() {
                    if index > 0 {
                        self.resolve_faints();
                        if self.state.outcome.is_some() {
                            return;
                        }
                        // Showdown's `runAction` ends each Gen 1-4 `runSwitch` with an `Update`
                        // once its faints are settled: a berry the hazards brought into reach is
                        // eaten before the next entrant comes in (R-M8-172).
                        self.run_update(PendingFaints::Processed);
                    }
                    self.run_switch_in_steps(pokemon);
                }
            }
            SwitchInOrder::ByPriority => self.run_switch_in_event(entrants),
        }
    }

    /// Gen 1-4 (the gen4 mod's `runSwitch`): the entry hazards (`EntryHazard`), the
    /// `SwitchIn` event (Healing Wish / Lunar Dance's slot, `onSwitchInPriority` -1 after the
    /// hazards), Gen 2's status on entry, then the ability's and the item's `Start`.
    fn run_switch_in_steps(&mut self, source: PokemonId) {
        self.apply_entry_hazards(source);
        self.apply_slot_heals(source);
        // The status's `onSwitchIn` (Gen 3's sleep giving back its Snore / Sleep Talk turns).
        self.reset_sleep_on_entry(source);
        // Gen 1-2 `runSwitch`: whoever enters, every active Pokémon forgets its last move
        // (`lastMove`; Gen 2 Encore's `lastMoveEncore` stays).
        if self.rules.conditions.switch_in_clears_last_moves {
            for pokemon in self.active_pokemon() {
                if let Some(pokemon) = self.state.pokemon_mut(pokemon) {
                    pokemon.last_move = None;
                    pokemon.last_move_base = None;
                }
            }
        }
        // Gen 1 `runSwitch`: the entrant's side's last selected slot goes back to the first.
        if self.rules.conditions.pp.switch_in_resets_last_selected_slot
            && let Some(side) = self.state.pokemon(source).map(|pokemon| pokemon.side)
        {
            self.state.side_mut(side).last_selected_move_slot = 0;
        }
        self.metronome_binding_glitch(source);
        if self
            .state
            .pokemon(source)
            .is_none_or(|pokemon| pokemon.is_fainted())
        {
            return;
        }
        self.run_after_switch_in_status(source);
        if self
            .state
            .pokemon(source)
            .is_none_or(|pokemon| pokemon.is_fainted())
        {
            return;
        }
        // The species conditions' `onSwitchIn` (Gen 4's Arceus forme) run with the `SwitchIn`
        // event, before the ability's own handlers (DESIGN.md §43.4).
        BattleContext::new(&mut self.state, &self.rules, &mut self.trace).species_switch_in(source);
        self.run_switch_in_ability(source);
        self.run_switch_in_item(source, false);
        self.run_any_switch_in(&[source], i8::MIN);
        // Gen 4 (Showdown gen4 `runSwitch`): once the newcomer's ability and item have started
        // — so its Intimidate still sees them — its entry clears the `substitutebroken`
        // markers on its foes.
        if self.rules.conditions.substitute_broken_cleared_by_foe_entry
            && let Some(side) = self.state.pokemon(source).map(|p| p.side)
        {
            let foes: Vec<PokemonId> = self
                .state
                .side(side.other())
                .active
                .iter()
                .flatten()
                .copied()
                .collect();
            for foe in foes {
                if let Some(pokemon) = self.state.pokemon_mut(foe) {
                    pokemon
                        .volatiles
                        .remove(VolatileConditionKind::SubstituteBroken);
                }
            }
        }
    }

    /// Gen 5+ `fieldEvent('SwitchIn')` (Showdown `runSwitch`) over `entrants`, fastest first:
    /// every entrant's handlers sorted by priority (the ability's `onSwitchInPriority`), then
    /// the holder's speed (the entrants' order), then the effect type — the status (Gen 5
    /// sleep), the slot (Healing Wish), the side's entry hazards, the ability, the item.
    /// A fainted holder's handlers are skipped; the faints are settled after every handler and
    /// a decided battle stops the event. Showdown then asks Emergency Exit of the action's
    /// Pokémon alone — the first entrant.
    fn run_switch_in_event(&mut self, entrants: &[PokemonId]) {
        #[derive(Clone, Copy)]
        enum Step {
            Status,
            SlotHeal,
            Hazards,
            Ability,
            Item,
            AnySwitchIn(i8),
        }
        let first = entrants.first().copied();
        let hp_before = first
            .and_then(|pokemon| self.state.pokemon(pokemon))
            .map_or(0, |pokemon| pokemon.hp);
        let mut steps: Vec<(i8, usize, u8, Step)> = Vec::new();
        for (index, &pokemon) in entrants.iter().enumerate() {
            let ability_priority = self.state.pokemon(pokemon).map_or(0, |p| {
                crate::engine::ability_handler(&self.state, &self.rules, p, |b| {
                    b.switch_in_priority
                })
            });
            let item_priority = crate::engine::held_item(&self.state, &self.rules, pokemon)
                .map_or(0, |(_, behavior)| behavior.switch_in_priority);
            steps.extend([
                (0, index, 0, Step::Status),
                (0, index, 3, Step::SlotHeal),
                (0, index, 4, Step::Hazards),
                (ability_priority, index, 7, Step::Ability),
                (item_priority, index, 8, Step::Item),
            ]);
        }
        // The `onAnySwitchIn` handlers of the Pokémon on the field (White Herb −2, Opportunist
        // and Mirror Herb −3, Eject Pack −4; Pastel Veil, no priority), each priority once,
        // after the entrants' handlers of a higher one.
        let mut any: Vec<i8> = self
            .active_pokemon()
            .into_iter()
            .filter_map(|pokemon| crate::engine::held_item(&self.state, &self.rules, pokemon))
            .filter_map(|(_, behavior)| behavior.any_switch_in.map(|(priority, _)| priority))
            .collect();
        any.extend(self.active_pokemon().into_iter().filter_map(|pokemon| {
            let p = self.state.pokemon(pokemon)?;
            crate::engine::ability_handler(&self.state, &self.rules, p, |b| b.any_switch_in)
                .map(|(priority, _)| priority)
        }));
        any.sort_unstable();
        any.dedup();
        for priority in any {
            steps.push((priority, entrants.len(), 9, Step::AnySwitchIn(priority)));
        }
        steps.sort_by_key(|&(priority, index, sub_order, _)| {
            (std::cmp::Reverse(priority), index, sub_order)
        });
        for (_, index, _, step) in steps {
            if let Step::AnySwitchIn(priority) = step {
                self.run_any_switch_in(entrants, priority);
                self.resolve_faints();
                if self.state.outcome.is_some() {
                    return;
                }
                continue;
            }
            let pokemon = entrants[index];
            if self
                .state
                .pokemon(pokemon)
                .is_none_or(|pokemon| pokemon.is_fainted())
            {
                continue;
            }
            match step {
                Step::Status => self.reset_sleep_on_entry(pokemon),
                Step::SlotHeal => self.apply_slot_heals(pokemon),
                Step::Hazards => self.apply_entry_hazards(pokemon),
                Step::Ability => {
                    // The species conditions' `onSwitchIn` (Gen 4's Arceus forme) run in the
                    // same event, before the ability's own handlers (DESIGN.md §43.4).
                    BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                        .species_switch_in(pokemon);
                    self.run_switch_in_ability(pokemon);
                }
                Step::Item => self.run_switch_in_item(pokemon, true),
                Step::AnySwitchIn(_) => {}
            }
            self.resolve_faints();
            if self.state.outcome.is_some() {
                return;
            }
        }
        if let Some(pokemon) = first {
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            ctx.emergency_exit(pokemon, hp_before);
        }
    }

    /// Showdown `onAnySwitchIn` of the abilities and held items on the field,
    /// fastest first, a holder's ability before its item: those of `priority` (`i8::MIN`: every
    /// priority, the Gen 1-4 `runSwitch` that runs them after the newcomer's own handlers).
    /// Gen 3 (`AnySwitchInReach::EntrantAndFirstSlot`) lets only the newcomers' and P1's first
    /// slot's items look.
    pub(super) fn run_any_switch_in(&mut self, entrants: &[PokemonId], priority: i8) {
        let first_slot = self
            .rules
            .topology
            .position(SideId::P1, 0)
            .and_then(|position| {
                self.state
                    .pokemon_at_position(position, &self.rules.topology)
            });
        let mut holders: Vec<(PokemonId, u32, u8, crate::effects::SwitchHandler)> = Vec::new();
        for pokemon in self.active_pokemon() {
            let speed = crate::engine::stats::handler_speed(&self.state, pokemon);
            if let Some((own, handler)) = self.state.pokemon(pokemon).and_then(|p| {
                crate::engine::ability_handler(&self.state, &self.rules, p, |b| b.any_switch_in)
            }) && (priority == i8::MIN || own == priority)
            {
                holders.push((pokemon, speed, 0, handler));
            }
            if !(self.rules.conditions.any_switch_in_reach
                == crate::rules::AnySwitchInReach::Everyone
                || entrants.contains(&pokemon)
                || Some(pokemon) == first_slot)
            {
                continue;
            }
            if let Some((own, handler)) =
                crate::engine::held_item(&self.state, &self.rules, pokemon)
                    .and_then(|(_, behavior)| behavior.any_switch_in)
                && (priority == i8::MIN || own == priority)
            {
                holders.push((pokemon, speed, 1, handler));
            }
        }
        holders.sort_by_key(|(_, speed, sub, _)| (std::cmp::Reverse(*speed), *sub));
        for (pokemon, _, _, handler) in holders {
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            handler(&mut ctx, pokemon);
        }
    }

    /// The sleep counter as the sleeper comes in: Gen 5 (Showdown gen5 `slp.onSwitchIn`) starts it
    /// over; Gen 3 (gen3 `slp.onSwitchIn`) gives back the turns spent on Snore / Sleep Talk.
    fn reset_sleep_on_entry(&mut self, pokemon: PokemonId) {
        let lifecycle = self.rules.status.lifecycle;
        let Some(status) = self
            .state
            .pokemon_mut(pokemon)
            .and_then(|pokemon| pokemon.status.as_mut())
        else {
            return;
        };
        if lifecycle.sleep_resets_on_switch_in
            && let Some(rolled) = status.sleep_turns_rolled
        {
            status.sleep_turns_remaining = Some(rolled);
        }
        if lifecycle.sleep_usable_turns_come_back
            && let Some(remaining) = status.sleep_turns_remaining
        {
            status.sleep_turns_remaining =
                Some(remaining.saturating_add(status.sleep_turns_skipped));
            status.sleep_turns_skipped = 0;
        }
    }

    /// The Pokémon leaves by a switch, drag or replacement, not by fainting: Showdown
    /// `runEvent('SwitchOut')` (Natural Cure, Regenerator), then its ability's `End`.
    /// The Pokémon observers see in `pokemon`: itself, or the disguise its Illusion shows.
    fn seen_as(&self, pokemon: PokemonId) -> PokemonId {
        self.state
            .pokemon(pokemon)
            .and_then(|holder| holder.illusion)
            .unwrap_or(pokemon)
    }

    pub(super) fn run_switch_out(&mut self, source: PokemonId) {
        let Some(pokemon) = self.state.pokemon(source) else {
            return;
        };
        // A Dynamax ends with the switch (Showdown's `BeforeSwitchOut`, before the ability's
        // `SwitchOut`).
        if pokemon.volatiles.dynamax_base_max_hp().is_some() {
            self.end_dynamax(source);
        }
        let Some(pokemon) = self.state.pokemon(source) else {
            return;
        };
        // What observers last see of it, before its `SwitchOut` (Regenerator heals unseen).
        let seen_as = self.seen_as(source);
        let shown = self.state.pokemon(seen_as).unwrap_or(pokemon);
        let sighting = crate::state::Sighting {
            hp: pokemon.hp,
            max_hp: pokemon.max_hp,
            status: pokemon.status.as_ref().map(|status| status.kind),
            // Its own species, as its details show it: the forme it took for the battle (a
            // temporary one goes as it leaves).
            species: shown.species,
            level: pokemon.level,
            terastallized: pokemon.terastallized,
        };
        self.trace.push(TraceEvent::LeftTheField {
            pokemon: source,
            seen_as,
            sighting,
        });
        let Some(pokemon) = self.state.pokemon(source) else {
            return;
        };
        let behavior = crate::engine::ability_handlers(&self.state, &self.rules, pokemon);
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        effects::abilities::switch_out(&mut ctx, behavior, source);
        ctx.end_ability(source, crate::engine::AbilityEndCause::Leaving);
    }

    internal!(
        pub fn run_switch_in_ability(&mut self, source: PokemonId) {
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .switch_in_ability(source);
        }
    );

    /// The newcomer's item starts after its ability: Metronome's condition, the Choice items'
    /// lock cleared. Gen 1-4 `runSwitch` runs the item's `Start` (`singleEvent('Start')` on the
    /// ability, then on the item), which a suppressed item runs too (`singleEvent` skips
    /// `ignoringItem` for it), so this reads the held item itself. The Gen 5+ `SwitchIn` event
    /// (`as_switch_in`) runs the item's `onStart` as its `SwitchIn` handler (`getCallback`),
    /// which an item Klutz, Embargo or Magic Room suppresses misses (`engine::held_item`).
    fn run_switch_in_item(&mut self, source: PokemonId, as_switch_in: bool) {
        let behavior = if as_switch_in {
            crate::engine::held_item(&self.state, &self.rules, source).map(|(_, behavior)| behavior)
        } else {
            self.state
                .pokemon(source)
                .and_then(|pokemon| pokemon.held_item)
                .map(|item| crate::engine::item_behavior(&self.rules, item))
        };
        let Some(handler) = behavior.and_then(|behavior| behavior.switch_in) else {
            return;
        };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        handler(&mut ctx, source);
    }

    /// A chosen switch: cleanup, placement, then the newcomer's switch-in (DESIGN.md §30.1).
    pub(super) fn execute_switch(
        &mut self,
        outgoing: PokemonId,
        incoming: PokemonId,
    ) -> Result<(), BattleError> {
        let position = self
            .state
            .active_position(outgoing, &self.rules.topology)
            .ok_or(BattleError::ActorNotActive(outgoing))?;
        // Showdown `switchIn` → `BeforeSwitchOut` (Pursuit, §15.11), then Gen 5+ an `Update`. A
        // Pokémon it knocked out does not leave by its switch: Gen 5+ the switch is cancelled;
        // Gen 2-4 it faints and the Pokémon it chose comes in at once (`pursuitfaint`).
        if self
            .state
            .pokemon(outgoing)
            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Pursuit))
        {
            let pursuit = self.rules.conditions.switching.pursuit;
            self.before_switch_out(outgoing)?;
            // The Pursuit's knock-out is not processed yet (it is, after the action).
            if pursuit.update_after {
                self.run_update(PendingFaints::Pending);
            }
            if self.state.pokemon(outgoing).is_some_and(|p| p.is_fainted()) {
                if !pursuit.switch_survives_faint {
                    return Ok(());
                }
                self.resolve_faints();
                if self.state.outcome.is_some() {
                    return Ok(());
                }
                self.place(position, None, incoming)?;
                self.run_update(PendingFaints::Processed);
                self.run_switch_in(incoming);
                return Ok(());
            }
        }
        self.run_switch_out(outgoing);
        self.clear_on_switch(outgoing);
        self.place(position, Some(outgoing), incoming)?;
        // The `switch` action's `Update`; the switch-in is the queued `runSwitch` action.
        self.run_update(PendingFaints::Processed);
        self.run_switch_in(incoming);
        Ok(())
    }

    /// Puts `incoming` on `position` and traces it. The outgoing Pokémon, if any, must already
    /// be cleaned up; the switch-in stage is the caller's, so several entrants can be ordered.
    pub(super) fn place(
        &mut self,
        position: PositionId,
        outgoing: Option<PokemonId>,
        incoming: PokemonId,
    ) -> Result<(), BattleError> {
        let spec = self
            .rules
            .topology
            .spec(position)
            .ok_or(BattleError::InvalidReplacementPosition(position))?;
        // Showdown `switchIn`: the entrant's ability starts over, and none of its moves counts
        // as used (`moveSlot.used`, Last Resort).
        self.state.restart_ability_state(incoming);
        if let Some(pokemon) = self.state.pokemon_mut(incoming) {
            pokemon.item_state = crate::state::ItemState::default();
            for slot in pokemon.move_slots_mut() {
                slot.used = false;
            }
        }
        // Truant's bit as the Pokémon comes in (Gen 3-4, §15.21).
        BattleContext::new(&mut self.state, &self.rules, &mut self.trace).enter_truant(incoming);
        // Gen 3-4 (pokeemerald `Cmd_switchindataupdate`, pokeplatinum `BattleSystem_InitBattleMon`):
        // a Pokémon a Knock Off marked enters holding nothing (issues.md I54).
        if self.rules.conditions.items.knocked_off_enters_empty
            && let Some(pokemon) = self.state.pokemon_mut(incoming)
            && pokemon.history.item_knocked_off
        {
            pokemon.held_item = None;
        }
        let slot = spec.slot as usize;
        // Gen 1-4 `switchIn`: the Pokémon that held the position (switched out or fallen, still
        // at its index in the party order) hands its last item to the entrant.
        if self.rules.conditions.last_item_passes_to_entrant
            && let Some(previous) = self
                .state
                .side(spec.side)
                .battle_order
                .get(slot)
                .copied()
                .filter(|previous| *previous != incoming)
        {
            let last_item = self
                .state
                .pokemon_mut(previous)
                .and_then(|pokemon| pokemon.history.last_item.take());
            if let Some(pokemon) = self.state.pokemon_mut(incoming) {
                pokemon.history.last_item = last_item;
            }
        }
        // Showdown `switchIn`: a fallen Pokémon the entrant replaces gives its status up to the
        // faint (`oldActive.status = ''`) — a replacement in the middle of the turn (Gen 4 Healing
        // Wish) comes before `check_fainted` does it.
        if let Some(previous) = self
            .state
            .side(spec.side)
            .battle_order
            .get(slot)
            .copied()
            .filter(|previous| *previous != incoming)
            && let Some(pokemon) = self.state.pokemon_mut(previous)
            && pokemon.is_fainted()
            && let Some(status) = pokemon.status.take()
        {
            self.trace.push(TraceEvent::StatusCured {
                target: previous,
                status: status.kind,
                reason: crate::engine::CureReason::Fainted,
            });
        }
        let side = self.state.side_mut(spec.side);
        side.active[slot] = Some(incoming);
        // Showdown `switchIn`: the entrant takes its slot's index in the party order, the
        // Pokémon there takes the entrant's.
        if let Some(from) = side.battle_order.iter().position(|p| *p == incoming)
            && slot < side.battle_order.len()
        {
            side.battle_order.swap(from, slot);
        }
        // Showdown `runEvent('BeforeSwitchIn')`, its slot and party index taken: Illusion, and
        // Gen 4 Arceus's entry type (the species conditions' `onBeforeSwitchIn`).
        let mut context = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        context.take_disguise(incoming);
        context.species_before_switch_in(incoming);
        let seen_as = self.seen_as(incoming);
        self.trace.push(TraceEvent::Switched {
            side: spec.side,
            position,
            outgoing,
            incoming,
            seen_as,
        });
        Ok(())
    }

    /// Phazing (Showdown: after every action, before fainting): every Pokémon in the drag
    /// queue that is still standing is replaced by a random healthy bench Pokémon
    /// (`RandomTarget` decision over the bench in team order); its pending action this turn
    /// is cancelled and any chosen-switch order on it is dropped. The entrants then run their
    /// switch-in fastest first — in Gen 1-4 as the queued `runSwitch` action, after the dragging
    /// action's `Update` (`UpdateRules::drag_switch_in_deferred`).
    pub(super) fn process_drag_queue(&mut self) {
        let queue = std::mem::take(&mut self.state.drag_queue);
        let mut entrants = Vec::new();
        for target in queue {
            let Some(position) = self.state.active_position(target, &self.rules.topology) else {
                continue;
            };
            let Some(side) = self.state.pokemon(target).map(|p| p.side) else {
                continue;
            };
            if self.state.pokemon(target).is_some_and(|p| p.is_fainted()) {
                continue;
            }
            let bench = self.state.healthy_bench(side);
            if bench.is_empty() {
                continue;
            }
            let index = self.state.rng.range_inclusive_for(
                RandomRequestKind::RandomTarget,
                0,
                (bench.len() - 1) as u16,
            );
            let incoming = bench[usize::from(index)];
            self.trace.push(TraceEvent::DraggedOut { target, incoming });
            if self.state.side(side).switch_out == Some(target) {
                self.state.side_mut(side).switch_out = None;
            }
            if let PendingExecution::Turn { actions, .. } = &mut self.pending {
                actions.retain(|action| action.actor != target);
            }
            self.run_switch_out(target);
            self.clear_on_switch(target);
            if self.place(position, Some(target), incoming).is_err() {
                continue;
            }
            if self
                .rules
                .conditions
                .switching
                .dragged_skips_switch_in_status
            {
                self.state.dragged_in.push(incoming);
            }
            entrants.push(incoming);
        }
        if !entrants.is_empty() && self.rules.update.drag_switch_in_deferred {
            self.run_update(PendingFaints::Processed);
        }
        for pokemon in self.speed_ordered(entrants) {
            self.run_switch_in(pokemon);
        }
        self.state.dragged_in.clear();
    }

    internal!(
        /// Showdown `start`: the leads run their switch-in (Intimidate, a weather …) as one batch,
        /// fastest first by action speed (a Choice Scarf counts), ties shuffled. Call once, before
        /// the first step.
        pub fn run_lead_switch_ins(&mut self) {
            self.run_lead_switch_ins_inner();
            self.handed_back("run_lead_switch_ins");
        }
    );

    fn run_lead_switch_ins_inner(&mut self) {
        // Each lead's `switchIn`, side by side and slot by slot (Showdown's `start`): its
        // `BeforeSwitchIn` (Illusion) sees only the leads in before it — a Neutralizing Gas
        // holder among the later ones is not on the field yet.
        let leads: Vec<Vec<Option<PokemonId>>> = [SideId::P1, SideId::P2]
            .iter()
            .map(|side| std::mem::take(&mut self.state.side_mut(*side).active))
            .collect();
        for (side, slots) in [SideId::P1, SideId::P2].into_iter().zip(&leads) {
            self.state.side_mut(side).active = vec![None; slots.len()];
        }
        for (side, slots) in [SideId::P1, SideId::P2].into_iter().zip(&leads) {
            for (slot, lead) in slots.iter().enumerate() {
                self.state.side_mut(side).active[slot] = *lead;
                if let Some(lead) = lead {
                    // Its `switchIn` starts its ability state, in this order (R-M8-182).
                    self.state.stamp_ability_order(*lead);
                    let mut context =
                        BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                    context.take_disguise(*lead);
                    context.species_before_switch_in(*lead);
                    if let Some(position) = self.rules.topology.position(side, slot as u8) {
                        let seen_as = self.seen_as(*lead);
                        self.trace.push(TraceEvent::LeadSentOut {
                            pokemon: *lead,
                            position,
                            seen_as,
                        });
                    }
                }
            }
        }
        // The `start` action's `Update` (Gen 1-4), then the leads' `runSwitch` action's.
        if self.rules.update.after_lead_placement {
            self.run_update(PendingFaints::Processed);
        }
        let leads = self.speed_ordered(self.leads());
        // Showdown's leads come in on turn 0, `activeTurns` 0 (Gen 5+ Truant's `onStart` asks):
        // the start-of-battle `endTurn` that `create_new` stands in for counts them to 1 after.
        self.set_active_turns(&leads, 0);
        self.run_switch_ins(&leads);
        self.set_active_turns(&leads, 1);
        self.run_update(PendingFaints::Processed);
        // A lead ordered out by the leads' switch-in (Eject Pack after an Intimidate): Showdown's
        // `start` ends with the replacement request, and the start-of-battle `endTurn` that
        // `BattleEngine::try_new` stands in for waits for it — until then it is turn 0 and the
        // leads are newly in, on no turn yet.
        if self.mid_turn_replacement_needed() {
            self.state.turn = 0;
            for pokemon in self.active_pokemon() {
                if let Some(pokemon) = self.state.pokemon_mut(pokemon) {
                    pokemon.active_turns = 0;
                    pokemon.history.newly_switched = true;
                }
            }
            self.pending = PendingExecution::Start;
            self.set_phase(BattlePhase::ForcedSwitch);
        } else {
            // The start-of-battle `endTurn`'s rules (Gen 1: two teams that only Transform).
            self.end_turn_rules();
        }
    }

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

    fn set_active_turns(&mut self, pokemon: &[PokemonId], turns: u16) {
        for &id in pokemon {
            if let Some(pokemon) = self.state.pokemon_mut(id) {
                pokemon.active_turns = turns;
            }
        }
    }

    internal!(
        /// The ties `run_lead_switch_ins` draws: each tied lead with its action speed (the group),
        /// in the order the SpeedTie decisions are drawn. Lets a replay plan them from the order
        /// Showdown reported.
        pub fn lead_speed_ties(&self) -> Vec<(PokemonId, u32)> {
            let speeds = self.lead_speeds();
            speeds
                .iter()
                .filter(|(id, speed)| speeds.iter().any(|(other, s)| other != id && s == speed))
                .copied()
                .collect()
        }
    );

    internal!(
        /// Every lead with the action speed `run_lead_switch_ins` orders it by (fastest first, the
        /// ties as drawn): lets a replay place the leads' silent draws (DESIGN.md R-M8-84).
        pub fn lead_speeds(&self) -> Vec<(PokemonId, u32)> {
            self.leads()
                .into_iter()
                .map(|id| (id, action_speed(&self.state, &self.rules, id, None)))
                .collect()
        }
    );

    /// The Pokémon on the field, in position order.
    fn leads(&self) -> Vec<PokemonId> {
        [SideId::P1, SideId::P2]
            .into_iter()
            .flat_map(|side| self.state.side(side).active.iter().flatten().copied())
            .collect()
    }

    /// Fastest first; Pokémon sharing a speed draw one SpeedTie decision each (higher first),
    /// everyone else draws nothing, so the PRNG only advances on a real tie. The draws are
    /// traced (`SwitchInTiesDrawn`).
    pub(super) fn speed_ordered(&mut self, pokemon: Vec<PokemonId>) -> Vec<PokemonId> {
        let mut entries: Vec<(PokemonId, u32, u64)> = pokemon
            .into_iter()
            .map(|id| (id, action_speed(&self.state, &self.rules, id, None), 0))
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
                .push_with(crate::engine::TraceCategory::Check, || {
                    TraceEvent::SwitchInTiesDrawn { tied }
                });
        }
        entries.sort_by_key(|entry| std::cmp::Reverse((entry.1, entry.2)));
        entries.into_iter().map(|entry| entry.0).collect()
    }

    /// End-of-turn residual scheduler (ROADMAP §7.3): every residual instance on the field is
    /// collected with its generation order key, sorted (order, faster first, sub-order), and
    /// run in that order; instances tied on all three are shuffled (Showdown `speedSort`: one
    /// `SpeedTie` decision each, `residual_tie_groups`). An instance whose holder has left the
    /// field or fainted since collection is skipped, as Showdown skips handlers of fainted
    /// holders.
    ///
    /// Showdown's `residual` action starts with `updateSpeed`: every sort in it — the handlers
    /// here, the weather's `eachEvent` — reads the speeds of that moment (`residual_speeds`), so
    /// the Gen 4 Tailwind that ends first still doubles its side's place (R-M9-46).
    pub(super) fn run_end_turn_residual(&mut self) {
        self.residual_speeds = self
            .active_pokemon()
            .into_iter()
            .map(|id| {
                (
                    id,
                    crate::engine::residual_speed(&self.state, &self.rules, id),
                )
            })
            .collect();
        self.run_residual_handlers();
        self.residual_speeds.clear();
    }

    fn run_residual_handlers(&mut self) {
        self.set_phase(BattlePhase::EndTurn);
        self.state.turn_actions_done = true;

        // Gen 4 Tailwind counts down before anything else in the residual.
        if self.rules.conditions.tailwind_ticks_first {
            for side in [SideId::P1, SideId::P2] {
                if self.state.side_mut(side).conditions.tick_tailwind() {
                    self.trace.push(TraceEvent::SideConditionEnded {
                        side,
                        kind: crate::state::SideConditionKind::Tailwind,
                    });
                }
            }
        }
        // So do Gen 3-4 screens, Mist, Safeguard and Lucky Chant (`onSideResidualOrder` 1-6),
        // before the weather can decide the battle.
        if self.rules.conditions.side_protections.tick_first {
            for side in [SideId::P1, SideId::P2] {
                for kind in self.state.side_mut(side).conditions.tick_timed() {
                    self.trace
                        .push(TraceEvent::SideConditionEnded { side, kind });
                }
            }
        }

        // Showdown's `residual` action keeps every active Pokémon's HP for the Emergency Exit
        // check once the whole residual is over — `getUndynamaxedHP()`, the space the residual's
        // own check reads (DESIGN.md §44.5).
        let hp_before: Vec<(PokemonId, u16)> = self
            .active_pokemon()
            .into_iter()
            .filter_map(|id| self.state.pokemon(id).map(|p| (id, p.undynamaxed_hp(None))))
            .collect();

        // Showdown's slot conditions run for a fainted occupant too (`isSlotCondition`): a Wish
        // whose slot has nobody standing counts down all the same, and heals no one if it runs
        // out there.
        self.tick_unattended_wishes();
        self.tick_unattended_future_attacks();
        let collected = self.collect_residuals();
        // Gen 2+: the duration-only handlers take their place among the others (§31.1).
        let positioned = self.rules.residual.durations.is_some();
        let durations = self.collect_durations();
        let mut tie_breaks = vec![0; collected.len() + durations.len()];
        for (index, _) in residual_tie_groups(&collected, &durations) {
            tie_breaks[index] = self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
        }
        let mut handlers: Vec<(ResidualHandler, u64)> = collected
            .into_iter()
            .map(ResidualHandler::Instance)
            .chain(durations.into_iter().map(ResidualHandler::Duration))
            .zip(tie_breaks)
            .collect();
        // A stable sort: handlers equal on every key keep their collection order.
        handlers.sort_by_key(|(handler, tie_break)| {
            let (order, speed, sub_order, slot) = handler.key();
            (
                // Showdown `comparePriority`: order, then speed, then sub-order — so in Gen 3-4
                // (everything at order 10) the faster Pokémon finishes all its residuals first.
                order,
                std::cmp::Reverse(speed),
                sub_order,
                std::cmp::Reverse(*tie_break),
                slot,
            )
        });

        // A handler whose count ran out (Showdown `fieldEvent` calls its `end` and moves on
        // without `faintMessages()`) leaves its faint to the next handler that runs: Perish Song
        // taking both last Pokémon in one residual is a draw, and a duration that goes on after
        // it (a terrain, a charge) settles those faints before the later ones count down.
        let mut faints_waiting = false;
        for (handler, _) in handlers {
            // Showdown `fieldEvent`: `faintMessages()` after every handler, and the residual
            // stops the moment the battle is decided.
            if self.state.outcome.is_some() {
                break;
            }
            let instance = match handler {
                ResidualHandler::Duration(duration) => {
                    match self.run_duration(duration.handler) {
                        None | Some(ResidualRun::Ended) => {}
                        Some(ResidualRun::Ran) => {
                            self.resolve_faints();
                            faints_waiting = false;
                        }
                    }
                    continue;
                }
                ResidualHandler::Instance(instance) => instance,
            };
            let Some(holder) = instance.holder else {
                // The field's residual: the weather (its tick, then its chip on everyone).
                if self.state.field.weather.is_some() {
                    self.run_weather_residual();
                    self.resolve_faints();
                    faints_waiting = false;
                }
                continue;
            };
            // Showdown `fieldEvent`: the handlers of a Pokémon that has fainted are skipped, and
            // so is one whose condition a prior handler removed (a Healer's cure ending
            // Nightmare).
            let on_field = self
                .state
                .active_position(holder, &self.rules.topology)
                .is_some();
            // A slot condition whose holder has fallen since collection still counts down,
            // striking or healing no one (its `onEnd` asks for a standing target).
            if let Some((side, slot)) = instance.position
                && (!on_field || self.state.pokemon(holder).is_some_and(|p| p.is_fainted()))
            {
                self.count_down_slot_condition(instance.kind, side, slot);
                self.resolve_faints();
                faints_waiting = false;
                continue;
            }
            if !on_field || !self.residual_condition_held(holder, instance.kind) {
                continue;
            }
            // One whose faint is still waiting is not yet `fainted`: its handler runs (to no
            // effect at 0 HP) and the faints settle after it — unless its count runs out, which
            // leaves them waiting (the Roost of a Pokémon Perish Song took, R-M10-66).
            if self
                .state
                .pokemon(holder)
                .is_some_and(|pokemon| pokemon.is_fainted())
            {
                if self.count_runs_out(holder, instance.kind) {
                    faints_waiting = true;
                } else {
                    self.resolve_faints();
                    faints_waiting = false;
                }
                continue;
            }
            let counted_out = self.count_runs_out(holder, instance.kind);
            self.trace.push(TraceEvent::ResidualEffect {
                holder,
                kind: instance.kind,
                order: instance.order,
                sub_order: instance.sub_order,
            });
            if self.run_residual(holder, instance.kind) == ResidualRun::Ended || counted_out {
                faints_waiting = true;
            } else {
                self.resolve_faints();
                faints_waiting = false;
            }
        }
        // No handler ran after that count (Showdown's next `faintMessages`). Gen 1, whose
        // durations are not handlers of the residual, ticks every duration-only effect before
        // those faints settle.
        let durations_ticked = faints_waiting && !positioned;
        if faints_waiting {
            if !positioned {
                self.expire_conditions_end_turn(false);
            }
            self.resolve_faints();
        }

        // Status counters advance once per turn, after the ticks; the turn's declared moves are
        // all spent. Time on the field (`active_turns`) advances when the next turn starts
        // (`BattleEngine::next_turn`), as Showdown's `activeTurns` does.
        let active = self.active_pokemon();
        for pokemon_id in active {
            if let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
                && let Some(status) = &mut pokemon.status
            {
                status.advance_turn();
            }
        }
        self.state.pending_movers.clear();
        self.state.struggling.clear();

        // The durations (Tailwind 26, Trick Room / terrain 27, and every duration-only effect
        // after them) tick inside Showdown's residual, after every handler that can knock a
        // Pokémon out: a battle decided in the residual never gets there.
        if self.state.outcome.is_some() {
            return;
        }
        if !durations_ticked {
            self.expire_conditions_end_turn(positioned);
        }
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        let mut exited = false;
        for (pokemon, before) in hp_before {
            // The residual's own check reads through `getUndynamaxedHP` (DESIGN.md §44.5).
            exited |= ctx.emergency_exit_residual(pokemon, before);
        }
        // Showdown runs this `EmergencyExit` after the residual's `checkFainted`, whose switch
        // flags on the fainted the exit clears: their slots are asked for after the exit's. One
        // a hit in the residual already ordered out (a Future Sight setting off Wimp Out) clears
        // nothing (`target.switchFlag` returns first): one request for all (R-M8-130).
        self.state.faint_refills_held = exited;
    }

    /// The speed the residual and weather schedules order by: Showdown's residual starts with
    /// `updateSpeed` and sorts its handlers once, so it is the effective speed (stages,
    /// paralysis, Tailwind, Choice Scarf; Gen 2-3's Quick Claw) of that moment, inverted under
    /// Trick Room — which is also what a replay previewing the residual's ties reads.
    pub(super) fn residual_speed(&self, pokemon: PokemonId) -> u32 {
        self.residual_speeds
            .iter()
            .find(|(id, _)| *id == pokemon)
            .map_or_else(
                || crate::engine::residual_speed(&self.state, &self.rules, pokemon),
                |(_, speed)| *speed,
            )
    }

    /// The active Pokémon (in `active_pokemon` order) whose Speed ties another's as the weather
    /// chips them, by index, each with its tie group.
    fn weather_tie_groups(&self) -> Vec<(usize, usize)> {
        let speeds: Vec<u32> = self
            .active_pokemon()
            .into_iter()
            .map(|id| self.residual_speed(id))
            .collect();
        tie_groups(&speeds)
    }

    internal!(
        /// The end-of-turn decisions a replay has to plan before the residual runs (DESIGN.md
        /// §12.4): the residual handlers that tie, in the order their `SpeedTie` decisions are drawn
        /// with their tie groups, then — when the weather will chip this turn — the Pokémon that
        /// tie for its order.
        pub fn pending_residual_ties(&self) -> super::ResidualTies {
            let instances = self.collect_residuals();
            let durations = self.collect_durations();
            let tied = residual_tie_groups(&instances, &durations);
            let handlers = tied
                .iter()
                .filter_map(|&(index, group)| {
                    let instance = instances.get(index)?;
                    Some((instance.holder?, instance.kind, group))
                })
                .collect();
            let durations = tied
                .iter()
                .filter_map(|&(index, group)| {
                    Some((
                        durations.get(index.checked_sub(instances.len())?)?.handler,
                        group,
                    ))
                })
                .collect();
            let suppressed = crate::engine::weather::suppressed(&self.state, &self.rules);
            let chips = self.state.field.weather.is_some_and(|weather| {
                weather.turns_remaining != 1
                    && !suppressed
                    && match weather.kind {
                        WeatherKind::Sandstorm => true,
                        WeatherKind::Snow => self.rules.weather.hail_damages,
                        _ => false,
                    }
            });
            let active = self.active_pokemon();
            let weather = if chips {
                self.weather_tie_groups()
                    .into_iter()
                    .map(|(index, group)| (active[index], group))
                    .collect()
            } else {
                Vec::new()
            };
            super::ResidualTies {
                handlers,
                durations,
                weather,
            }
        }
    );

    pub(super) fn active_pokemon(&self) -> Vec<PokemonId> {
        let mut active = Vec::new();
        for side in [SideId::P1, SideId::P2] {
            active.extend(self.state.side(side).active.iter().copied().flatten());
        }
        active
    }

    pub(super) fn collect_residuals(&self) -> Vec<ResidualInstance> {
        let rules = self.rules.residual;
        let mut instances = Vec::new();
        if self.state.field.weather.is_some()
            && let Some((order, sub_order)) = rules.key(ResidualKind::Weather)
        {
            instances.push(ResidualInstance {
                holder: None,
                kind: ResidualKind::Weather,
                order,
                sub_order,
                speed: 0,
                slot: 0,
                position: None,
            });
        }
        for (slot, holder) in self.active_pokemon().into_iter().enumerate() {
            let Some(pokemon) = self.state.pokemon(holder) else {
                continue;
            };
            let speed = self.residual_speed(holder);
            // Gen 3 on the cartridge (issues.md I111): a strike goes by battler (pokeemerald
            // `HandleWishPerishSongOnTurnEnd`), not by speed — distinct keys, no ties.
            let future_battler = self.future_attack_battler(holder);
            // A kind the generation's table does not list has no end-of-turn residual there.
            let mut push = |kind: ResidualKind| {
                let Some((order, sub_order)) = rules.key(kind) else {
                    return;
                };
                let speed = match (kind, future_battler) {
                    (ResidualKind::FutureAttack, Some(battler)) => u32::MAX - battler,
                    _ => speed,
                };
                let position = match kind {
                    ResidualKind::Wish | ResidualKind::FutureAttack => self
                        .state
                        .active_position(holder, &self.rules.topology)
                        .and_then(|position| self.rules.topology.spec(position))
                        .map(|spec| (spec.side, usize::from(spec.slot))),
                    _ => None,
                };
                instances.push(ResidualInstance {
                    holder: Some(holder),
                    kind,
                    order,
                    sub_order,
                    speed,
                    slot,
                    position,
                });
            };
            // The held item as it is: Showdown collects a set-aside item's handler too and
            // skips it as it runs (`singleEvent` asks `ignoringItem()` then), so an item Magic
            // Room stops acts once the room ends earlier in the same residual.
            if let Some(behavior) = pokemon
                .held_item
                .map(|item| crate::engine::item_behavior(&self.rules, item))
            {
                if let Some(participant) = behavior.residual {
                    push(participant.kind);
                } else if matches!(behavior.update, Some(crate::effects::items::ItemUpdate::Eat(trigger)) if trigger.at_turn_end(&self.rules.conditions.berries))
                {
                    // Gen 2's status berries (pokecrystal `HandleHealingItems`, issues.md I33)
                    // and Gen 2-3's PP berries (`HandleMysteryberry`, `ENDTURN_ITEMS1`, I49) at
                    // the end of the turn.
                    push(ResidualKind::Berry);
                }
            }
            // The ability as it is: Showdown collects a suppressed ability's handler too and skips
            // it as it runs (`fieldEvent` asks `suppressingAbility` then), so an ability a
            // Neutralizing Gas stops acts once its holder faints earlier in the same residual.
            if let Some(participant) =
                crate::engine::ability_behavior(&self.rules, crate::engine::raw_ability(pokemon))
                    .residual
            {
                push(participant.kind);
            }
            if pokemon.volatiles.contains(VolatileConditionKind::Ingrain) {
                push(ResidualKind::Ingrain);
            }
            // Dynamax's counter (its own `onResidualPriority: -100`): last among the handlers.
            if pokemon.volatiles.contains(VolatileConditionKind::Dynamax) {
                push(ResidualKind::DynamaxCounter);
            }
            if self.wish_slot(holder).is_some() {
                push(ResidualKind::Wish);
            }
            if self.future_attack_slot(holder).is_some() {
                push(ResidualKind::FutureAttack);
            }
            if pokemon
                .volatiles
                .contains(VolatileConditionKind::PerishSong)
            {
                push(ResidualKind::PerishSong);
            }
            if pokemon.volatiles.contains(VolatileConditionKind::AquaRing) {
                push(ResidualKind::AquaRing);
            }
            if self.state.field.terrain.map(|t| t.kind) == Some(crate::state::TerrainKind::Grassy) {
                push(ResidualKind::GrassyTerrain);
            }
            // A combined Fire Pledge on this Pokémon's side chips it (a Fire type is spared,
            // in the instance's own check): the same side condition, per active Pokémon
            // (Showdown's `onResidual`, DESIGN.md §42.9).
            if self
                .state
                .side(pokemon.side)
                .conditions
                .contains(crate::state::SideConditionKind::FirePledge)
            {
                push(ResidualKind::FirePledge);
            }
            if pokemon.volatiles.contains(VolatileConditionKind::LeechSeed) {
                push(ResidualKind::LeechSeed);
            }
            if pokemon
                .volatiles
                .contains(VolatileConditionKind::PartiallyTrapped)
            {
                push(ResidualKind::PartiallyTrapped);
            }
            if pokemon.volatiles.contains(VolatileConditionKind::Octolock) {
                push(ResidualKind::Octolock);
            }
            match pokemon.status.map(|status| status.kind) {
                Some(NonVolatileStatus::Poison) => push(ResidualKind::Poison),
                Some(NonVolatileStatus::Toxic) => push(ResidualKind::Toxic),
                Some(NonVolatileStatus::Burn) => push(ResidualKind::Burn),
                Some(NonVolatileStatus::Freeze) => push(ResidualKind::FreezeThaw),
                _ => {}
            }
            for (kind, residual) in [
                (VolatileConditionKind::Taunt, ResidualKind::Taunt),
                (VolatileConditionKind::Encore, ResidualKind::Encore),
                (VolatileConditionKind::Disable, ResidualKind::Disable),
                (VolatileConditionKind::Nightmare, ResidualKind::Nightmare),
                (VolatileConditionKind::Curse, ResidualKind::Curse),
                (VolatileConditionKind::Yawn, ResidualKind::Yawn),
                (VolatileConditionKind::HealBlock, ResidualKind::HealBlock),
                (VolatileConditionKind::Embargo, ResidualKind::Embargo),
                (VolatileConditionKind::MagnetRise, ResidualKind::MagnetRise),
                (
                    VolatileConditionKind::Telekinesis,
                    ResidualKind::Telekinesis,
                ),
                (VolatileConditionKind::SaltCure, ResidualKind::SaltCure),
                (VolatileConditionKind::SyrupBomb, ResidualKind::SyrupBomb),
                (VolatileConditionKind::ThroatChop, ResidualKind::ThroatChop),
                (VolatileConditionKind::Roost, ResidualKind::Roost),
                (VolatileConditionKind::Electrify, ResidualKind::Electrify),
                (VolatileConditionKind::Powder, ResidualKind::Powder),
                (VolatileConditionKind::LaserFocus, ResidualKind::LaserFocus),
                (VolatileConditionKind::MicleBerry, ResidualKind::MicleBoost),
                (VolatileConditionKind::Rampage, ResidualKind::Rampage),
                (VolatileConditionKind::Uproar, ResidualKind::Uproar),
            ] {
                if pokemon.volatiles.contains(kind) {
                    push(residual);
                }
            }
        }
        instances
    }

    /// Whether `kind`'s residual on `holder` is a count running out (Showdown `fieldEvent`: the
    /// handler's `duration` reaches 0, `handler.end` runs and the loop goes on with no
    /// `faintMessages()`, R-M10-66): Roost, Electrify and Powder last the turn, the others have
    /// one turn left. Gen 2 on, where the durations are handlers of the residual. Uproar and a
    /// rampage are left out: a callback of theirs may end them too.
    fn count_runs_out(&self, holder: PokemonId, kind: ResidualKind) -> bool {
        if self.rules.residual.durations.is_none() {
            return false;
        }
        let volatile = match kind {
            ResidualKind::Roost | ResidualKind::Electrify | ResidualKind::Powder => return true,
            ResidualKind::Taunt => VolatileConditionKind::Taunt,
            ResidualKind::Encore => VolatileConditionKind::Encore,
            ResidualKind::Disable => VolatileConditionKind::Disable,
            ResidualKind::Yawn => VolatileConditionKind::Yawn,
            ResidualKind::HealBlock => VolatileConditionKind::HealBlock,
            ResidualKind::Embargo => VolatileConditionKind::Embargo,
            ResidualKind::MagnetRise => VolatileConditionKind::MagnetRise,
            ResidualKind::Telekinesis => VolatileConditionKind::Telekinesis,
            ResidualKind::ThroatChop => VolatileConditionKind::ThroatChop,
            ResidualKind::LaserFocus => VolatileConditionKind::LaserFocus,
            ResidualKind::SyrupBomb => VolatileConditionKind::SyrupBomb,
            ResidualKind::MicleBoost => VolatileConditionKind::MicleBerry,
            ResidualKind::PartiallyTrapped => VolatileConditionKind::PartiallyTrapped,
            _ => return false,
        };
        self.state
            .pokemon(holder)
            .and_then(|pokemon| pokemon.volatiles.timed_turns_left(volatile))
            == Some(1)
    }

    /// Whether the volatile or status a residual belongs to is still on its holder.
    fn residual_condition_held(&self, holder: PokemonId, kind: ResidualKind) -> bool {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return false;
        };
        let volatile = match kind {
            ResidualKind::Ingrain => VolatileConditionKind::Ingrain,
            ResidualKind::LeechSeed => VolatileConditionKind::LeechSeed,
            ResidualKind::PartiallyTrapped => VolatileConditionKind::PartiallyTrapped,
            ResidualKind::Octolock => VolatileConditionKind::Octolock,
            ResidualKind::PerishSong => VolatileConditionKind::PerishSong,
            ResidualKind::Taunt => VolatileConditionKind::Taunt,
            ResidualKind::Encore => VolatileConditionKind::Encore,
            ResidualKind::Disable => VolatileConditionKind::Disable,
            ResidualKind::Nightmare => VolatileConditionKind::Nightmare,
            ResidualKind::Curse => VolatileConditionKind::Curse,
            ResidualKind::Yawn => VolatileConditionKind::Yawn,
            ResidualKind::HealBlock => VolatileConditionKind::HealBlock,
            ResidualKind::Embargo => VolatileConditionKind::Embargo,
            ResidualKind::MagnetRise => VolatileConditionKind::MagnetRise,
            ResidualKind::Telekinesis => VolatileConditionKind::Telekinesis,
            ResidualKind::SaltCure => VolatileConditionKind::SaltCure,
            ResidualKind::SyrupBomb => VolatileConditionKind::SyrupBomb,
            ResidualKind::ThroatChop => VolatileConditionKind::ThroatChop,
            ResidualKind::Roost => VolatileConditionKind::Roost,
            ResidualKind::Electrify => VolatileConditionKind::Electrify,
            ResidualKind::Powder => VolatileConditionKind::Powder,
            ResidualKind::MicleBoost => VolatileConditionKind::MicleBerry,
            ResidualKind::Rampage => VolatileConditionKind::Rampage,
            ResidualKind::Uproar => VolatileConditionKind::Uproar,
            ResidualKind::Poison | ResidualKind::Toxic => {
                return pokemon.has_status(NonVolatileStatus::Poison)
                    || pokemon.has_status(NonVolatileStatus::Toxic);
            }
            ResidualKind::Burn => return pokemon.has_status(NonVolatileStatus::Burn),
            ResidualKind::FreezeThaw => return pokemon.has_status(NonVolatileStatus::Freeze),
            _ => return true,
        };
        pokemon.volatiles.contains(volatile)
    }

    /// A Wish's or a foreseen attack's count on `side`'s `slot`, its holder down: one residual
    /// off, gone at the last without its effect.
    fn count_down_slot_condition(&mut self, kind: ResidualKind, side: SideId, slot: usize) {
        let side = self.state.side_mut(side);
        match kind {
            ResidualKind::Wish => {
                if let Some(index) = side.wishes.iter().position(|wish| wish.slot == slot) {
                    let wish = &mut side.wishes[index];
                    wish.turns_remaining = wish.turns_remaining.saturating_sub(1);
                    if wish.turns_remaining == 0 {
                        side.wishes.remove(index);
                    }
                }
            }
            _ => {
                if let Some(index) = side.future_attacks.iter().position(|a| a.slot == slot) {
                    let attack = &mut side.future_attacks[index];
                    attack.turns_remaining = attack.turns_remaining.saturating_sub(1);
                    if attack.turns_remaining == 0 {
                        side.future_attacks.remove(index);
                    }
                }
            }
        }
    }

    /// Every Wish whose slot holds no standing Pokémon: one residual off, gone at the last.
    fn tick_unattended_wishes(&mut self) {
        for side in [SideId::P1, SideId::P2] {
            let unattended: Vec<usize> = self
                .state
                .side(side)
                .wishes
                .iter()
                .enumerate()
                .filter(|(_, wish)| {
                    self.state
                        .side(side)
                        .active
                        .get(wish.slot)
                        .copied()
                        .flatten()
                        .and_then(|id| self.state.pokemon(id))
                        .is_none_or(|pokemon| pokemon.is_fainted())
                })
                .map(|(index, _)| index)
                .collect();
            let wishes = &mut self.state.side_mut(side).wishes;
            for index in unattended.into_iter().rev() {
                wishes[index].turns_remaining = wishes[index].turns_remaining.saturating_sub(1);
                if wishes[index].turns_remaining == 0 {
                    wishes.remove(index);
                }
            }
        }
    }

    /// The side and index of the Wish pending on `holder`'s slot, if any.
    fn wish_slot(&self, holder: PokemonId) -> Option<(SideId, usize)> {
        let position = self.state.active_position(holder, &self.rules.topology)?;
        let spec = self.rules.topology.spec(position)?;
        let slot = usize::from(spec.slot);
        let index = self
            .state
            .side(spec.side)
            .wishes
            .iter()
            .position(|wish| wish.slot == slot)?;
        Some((spec.side, index))
    }

    /// Wish's residual (Showdown `onResidual`, Gen 3-4 its `duration`): counted down the turn it
    /// was made; the next turn it ends and heals whoever stands in the slot — half the wisher's
    /// max HP taken as it wished (Gen 5+) or half the recipient's (Gen 3-4:
    /// `FieldRules::wish_heals_wisher_share`), `[from] move: Wish`.
    fn run_wish(&mut self, holder: PokemonId) {
        let Some((side, index)) = self.wish_slot(holder) else {
            return;
        };
        let wishes = &mut self.state.side_mut(side).wishes;
        wishes[index].turns_remaining = wishes[index].turns_remaining.saturating_sub(1);
        if wishes[index].turns_remaining > 0 {
            return;
        }
        let wish = wishes.remove(index);
        let Some(max_hp) = self
            .state
            .pokemon(holder)
            .filter(|pokemon| !pokemon.is_fainted())
            .map(|pokemon| pokemon.max_hp)
        else {
            return;
        };
        let amount = if self.rules.conditions.field.wish_heals_wisher_share {
            wish.amount
        } else {
            max_hp / 2
        };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_heal(crate::engine::HealRequest {
            target: holder,
            amount,
            source: EffectSource::Move {
                pokemon: wish.source,
                move_id: crate::ids::MoveId::WISH,
            },
            direct: false,
        });
    }

    /// Runs one residual handler; `Ended` when its count ran out (Showdown's `handler.end`).
    pub(super) fn run_residual(&mut self, holder: PokemonId, kind: ResidualKind) -> ResidualRun {
        match kind {
            // Dynamax's `onResidual`: one turn older; the third turn's end is the `endTurn`
            // sweep (`expire_dynamax`, after the whole residual).
            ResidualKind::DynamaxCounter => {
                if let Some(pokemon) = self.state.pokemon_mut(holder) {
                    pokemon.volatiles.count_a_dynamax_turn();
                }
                return ResidualRun::Ran;
            }
            ResidualKind::Wish => self.run_wish(holder),
            ResidualKind::FutureAttack => return self.run_future_attack(holder),
            ResidualKind::PerishSong => return self.run_perish_song(holder),
            // Field-scoped; dispatched from `run_end_turn_residual` without a holder.
            ResidualKind::Weather => {}
            // A berry at the end of the turn: its own `onResidual` (Gen 2-3 HP and confusion
            // berries, Gen 2 Berry Juice), or a Gen 2 status berry's check.
            ResidualKind::Berry => {
                let Some((_, item)) = crate::engine::held_item(&self.state, &self.rules, holder)
                else {
                    return ResidualRun::Ran;
                };
                let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                match item.residual {
                    Some(participant) if participant.kind == kind => {
                        (participant.handler)(&mut ctx, holder);
                    }
                    Some(_) => {}
                    None => ctx.feed_turn_end_berry(holder),
                }
            }
            // Item residuals: the holder's item, if it still has the one that was scheduled.
            ResidualKind::Leftovers
            | ResidualKind::BlackSludge
            | ResidualKind::StatusOrb
            | ResidualKind::MicleBerry
            | ResidualKind::WhiteHerb
            | ResidualKind::MirrorHerb
            | ResidualKind::EjectPack
            | ResidualKind::StickyBarb => {
                let Some((_, item)) = crate::engine::held_item(&self.state, &self.rules, holder)
                else {
                    return ResidualRun::Ran;
                };
                if let Some(participant) = item.residual
                    && participant.kind == kind
                {
                    let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                    (participant.handler)(&mut ctx, holder);
                }
            }
            // Ability residuals (Shed Skin, Speed Boost, Slow Start, Gen 3 Truant, Rain Dish,
            // Hydration, Healer, Bad Dreams, Moody, Harvest, Cud Chew, Pickup); Harvest asks the dex whether
            // the last item is a berry.
            ResidualKind::ShedSkin
            | ResidualKind::SpeedBoost
            | ResidualKind::SlowStart
            | ResidualKind::Truant
            | ResidualKind::RainDish
            | ResidualKind::Hydration
            | ResidualKind::Healer
            | ResidualKind::BadDreams
            | ResidualKind::Moody
            | ResidualKind::Harvest
            | ResidualKind::CudChew
            | ResidualKind::Pickup
            | ResidualKind::FormeChange
            | ResidualKind::Opportunist => {
                let Some(ability) = self
                    .state
                    .pokemon(holder)
                    .and_then(|p| crate::engine::ability_running(&self.state, &self.rules, p))
                else {
                    return ResidualRun::Ran;
                };
                if let Some(participant) =
                    crate::engine::ability_behavior(&self.rules, ability).residual
                    && participant.kind == kind
                {
                    let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                        .with_dex(&self.dex);
                    (participant.handler)(&mut ctx, holder);
                }
            }
            ResidualKind::LeechSeed => self.run_leech_seed(holder),
            ResidualKind::Rampage => return self.run_rampage_residual(holder),
            ResidualKind::Uproar => return self.run_uproar(holder),
            ResidualKind::FreezeThaw => self.run_freeze_thaw(holder),
            ResidualKind::Ingrain | ResidualKind::AquaRing => {
                let Some(max_hp) = self.state.pokemon(holder).map(|p| p.base_max_hp()) else {
                    return ResidualRun::Ran;
                };
                let volatile = if kind == ResidualKind::Ingrain {
                    VolatileConditionKind::Ingrain
                } else {
                    VolatileConditionKind::AquaRing
                };
                let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                let amount = ctx.big_root(holder, (max_hp / 16).max(1));
                ctx.apply_heal(crate::engine::HealRequest {
                    target: holder,
                    amount,
                    source: EffectSource::Volatile(volatile),
                    direct: false,
                });
            }
            ResidualKind::PartiallyTrapped => self.run_partially_trapped(holder),
            ResidualKind::FirePledge => {
                let Some(pokemon) = self.state.pokemon(holder) else {
                    return ResidualRun::Ran;
                };
                // Fire types are spared (Showdown's `onResidual`: `!pokemon.hasType('Fire')`).
                if crate::engine::types_of(&self.state, &self.rules, pokemon)
                    .contains(crate::dex::PokemonType::Fire)
                {
                    return ResidualRun::Ran;
                }
                let amount = (pokemon.base_max_hp() / 8).max(1);
                let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                ctx.apply_damage(crate::engine::DamageRequest {
                    target: holder,
                    amount: crate::engine::DamageAmount(amount),
                    kind: crate::engine::DamageKind::Residual,
                    source: EffectSource::SideCondition(
                        crate::state::SideConditionKind::FirePledge,
                    ),
                    flags: crate::engine::DamageFlags::default(),
                });
            }
            ResidualKind::Octolock => {
                BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                    .octolock_residual(holder);
            }
            ResidualKind::GrassyTerrain => {
                let Some(pokemon) = self.state.pokemon(holder) else {
                    return ResidualRun::Ran;
                };
                if !crate::engine::environment::grounded(&self.state, &self.rules, pokemon)
                    || crate::engine::two_turn::hiding(pokemon).is_some()
                {
                    return ResidualRun::Ran;
                }
                let amount = (pokemon.base_max_hp() / 16).max(1);
                let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                ctx.apply_heal(crate::engine::HealRequest {
                    target: holder,
                    amount,
                    source: EffectSource::Terrain(crate::state::TerrainKind::Grassy),
                    direct: false,
                });
            }
            ResidualKind::Poison | ResidualKind::Toxic | ResidualKind::Burn => {
                self.run_status_residual(holder);
            }
            ResidualKind::Taunt => self.tick_volatile(holder, VolatileConditionKind::Taunt),
            ResidualKind::Encore => {
                // Showdown: `duration--` first (ending at 0), then `onResidual` ends it early
                // when the encored move has no PP left.
                self.tick_volatile(holder, VolatileConditionKind::Encore);
                let out_of_pp = self.state.pokemon(holder).is_some_and(|pokemon| {
                    pokemon.volatiles.encored_move().is_some_and(|encored| {
                        pokemon
                            .move_slot_of(encored)
                            .is_none_or(|slot| slot.pp == 0)
                    })
                });
                if out_of_pp
                    && let Some(pokemon) = self.state.pokemon_mut(holder)
                    && pokemon.volatiles.remove(VolatileConditionKind::Encore)
                {
                    self.trace.push(TraceEvent::VolatileEnded {
                        target: holder,
                        kind: VolatileConditionKind::Encore,
                        seen: true,
                    });
                }
            }
            ResidualKind::Disable => self.tick_volatile(holder, VolatileConditionKind::Disable),
            ResidualKind::Nightmare => self.run_nightmare(holder),
            ResidualKind::Curse => self.run_curse(holder),
            ResidualKind::Yawn => self.run_yawn(holder),
            ResidualKind::HealBlock => {
                self.tick_volatile(holder, VolatileConditionKind::HealBlock);
            }
            ResidualKind::Embargo => self.tick_volatile(holder, VolatileConditionKind::Embargo),
            ResidualKind::MagnetRise => {
                self.tick_volatile(holder, VolatileConditionKind::MagnetRise)
            }
            ResidualKind::Telekinesis => {
                self.tick_volatile(holder, VolatileConditionKind::Telekinesis)
            }
            ResidualKind::SaltCure => self.run_salt_cure(holder),
            ResidualKind::SyrupBomb => self.run_syrup_bomb(holder),
            ResidualKind::ThroatChop => {
                self.tick_volatile(holder, VolatileConditionKind::ThroatChop);
            }
            ResidualKind::LaserFocus => {
                self.tick_volatile(holder, VolatileConditionKind::LaserFocus);
            }
            ResidualKind::Roost | ResidualKind::Electrify | ResidualKind::Powder => {
                let kind = match kind {
                    ResidualKind::Roost => VolatileConditionKind::Roost,
                    ResidualKind::Electrify => VolatileConditionKind::Electrify,
                    _ => VolatileConditionKind::Powder,
                };
                BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                    .end_volatile(holder, kind);
            }
            ResidualKind::MicleBoost => {
                self.tick_volatile(holder, VolatileConditionKind::MicleBerry);
            }
        }
        ResidualRun::Ran
    }

    /// Perish Song (Showdown: the count ticks, and at 0 `onEnd` faints the holder).
    fn run_perish_song(&mut self, holder: PokemonId) -> ResidualRun {
        let left = self
            .state
            .pokemon_mut(holder)
            .filter(|p| !p.is_fainted())
            .and_then(|p| p.volatiles.tick_perish_song());
        if left != Some(0) {
            return ResidualRun::Ran;
        }
        self.trace.push(TraceEvent::VolatileEnded {
            target: holder,
            kind: VolatileConditionKind::PerishSong,
            seen: true,
        });
        BattleContext::new(&mut self.state, &self.rules, &mut self.trace).faint_directly(
            holder,
            EffectSource::Volatile(VolatileConditionKind::PerishSong),
        );
        ResidualRun::Ended
    }

    /// Nightmare (Showdown `onResidual`, Gen 2 `onAfterMoveSelf` while asleep): the holder
    /// loses a quarter of its max HP.
    fn run_nightmare(&mut self, holder: PokemonId) {
        let Some(max_hp) = self.state.pokemon(holder).map(|p| p.base_max_hp()) else {
            return;
        };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount((max_hp / 4).max(1)),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Volatile(VolatileConditionKind::Nightmare),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// Curse (Showdown `onResidual`, Gen 2 `onAfterMoveSelf`): the holder loses a quarter of its
    /// max HP (`baseMaxhp`).
    fn run_curse(&mut self, holder: PokemonId) {
        let Some(max_hp) = self.state.pokemon(holder).map(|p| p.base_max_hp()) else {
            return;
        };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount((max_hp / 4).max(1)),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Volatile(VolatileConditionKind::Curse),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// Salt Cure (Showdown `onResidual`): the holder loses an eighth of its max HP, a quarter
    /// for a Water or Steel type.
    fn run_salt_cure(&mut self, holder: PokemonId) {
        let Some((max_hp, soaked)) = self.state.pokemon(holder).map(|p| {
            (
                p.base_max_hp(),
                crate::engine::types_of(&self.state, &self.rules, p)
                    .contains(crate::dex::PokemonType::Water)
                    || crate::engine::types_of(&self.state, &self.rules, p)
                        .contains(crate::dex::PokemonType::Steel),
            )
        }) else {
            return;
        };
        let divisor = if soaked { 4 } else { 8 };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount((max_hp / divisor).max(1)),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Volatile(VolatileConditionKind::SaltCure),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// Syrup Bomb (Showdown `duration: 4`, `onResidual`): the countdown ticks first (ending on
    /// the fourth residual), then the holder's Speed drops by one — a drop the thrower causes,
    /// which Clear Body refuses and Mirror Armor sends back.
    fn run_syrup_bomb(&mut self, holder: PokemonId) {
        let Some(source) = self
            .state
            .pokemon(holder)
            .and_then(|p| p.volatiles.syrup_bomb_source())
        else {
            return;
        };
        let left = self.state.pokemon_mut(holder).and_then(|pokemon| {
            pokemon
                .volatiles
                .tick_timed(VolatileConditionKind::SyrupBomb)
        });
        if left == Some(0) {
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind: VolatileConditionKind::SyrupBomb,
                seen: true,
            });
            return;
        }
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
            target: holder,
            stat: crate::state::Stat::Speed,
            stages: -1,
            source: EffectSource::Move {
                pokemon: source,
                move_id: crate::ids::MoveId::SYRUP_BOMB,
            },
            cause: crate::engine::StatChangeCause::Move,
        });
    }

    /// Yawn (Showdown `duration: 2`, `onEnd`): the countdown ticks, and when it runs out the
    /// holder falls asleep — through the usual status checks, but Safeguard lets Yawn's own
    /// sleep through (`effect.id === 'yawn'`).
    fn run_yawn(&mut self, holder: PokemonId) {
        let source = self
            .state
            .pokemon(holder)
            .and_then(|pokemon| pokemon.volatiles.yawn_source());
        let left = self
            .state
            .pokemon_mut(holder)
            .and_then(|pokemon| pokemon.volatiles.tick_timed(VolatileConditionKind::Yawn));
        if left != Some(0) {
            return;
        }
        self.trace.push(TraceEvent::VolatileEnded {
            target: holder,
            kind: VolatileConditionKind::Yawn,
            seen: true,
        });
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.try_set_status_inflicted(
            holder,
            NonVolatileStatus::Sleep,
            EffectSource::Volatile(VolatileConditionKind::Yawn),
            source,
        );
    }

    /// The weather's residual (Showdown `onFieldResidual`): the duration ticks first (a weather
    /// set for 5 turns chips 4 times and ends on the fifth upkeep), then sandstorm / hail hit
    /// every Pokémon on the field, fastest first, unless a type or being underground shields
    /// it. The chip is a `DamageRequest` (Magic Guard refuses it like any non-move damage).
    pub(super) fn run_weather_residual(&mut self) {
        if let Some(ended) = self.state.field.tick_weather() {
            self.trace.push(TraceEvent::WeatherChanged {
                from: Some(ended),
                to: None,
            });
            // Showdown `clearWeather` → `eachEvent('WeatherChange')` (Protosynthesis).
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace).weather_changed();
            return;
        }
        let Some(weather) = self.state.field.weather else {
            return;
        };
        self.trace.push(TraceEvent::WeatherUpkeep {
            weather: weather.kind,
            turns_remaining: weather.turns_remaining,
        });
        // Showdown's upkeep runs `eachEvent('Weather')` — each active Pokémon's `onWeather`
        // (Dry Skin, Rain Dish, Ice Body, Solar Power …), then the weather's own chip — whose
        // handlers Air Lock / Cloud Nine suppress, every one (`runEvent` skips the `Weather`
        // event while `suppressingWeather`; sandstorm, hail and snow skip the event itself).
        // The `Update` that follows it from Gen 7 then has nothing to act on either: the
        // weather is the residual's first step.
        if crate::engine::weather::suppressed(&self.state, &self.rules) {
            return;
        }
        self.run_weather_event(weather);
        // Showdown `eachEvent`: from Gen 7 an `Update` follows `eachEvent('Weather')`; the
        // chip's faints are still pending.
        if self.rules.update.after_weather {
            self.run_update(PendingFaints::Pending);
        }
    }

    /// Showdown `eachEvent('Weather')`, the weather in effect.
    fn run_weather_event(&mut self, weather: crate::state::WeatherState) {
        let rules = self.rules.weather;
        let immune_types: Option<&[crate::dex::PokemonType]> = match weather.kind {
            WeatherKind::Sandstorm => Some(rules.sandstorm_immune_types),
            WeatherKind::Snow if rules.hail_damages => Some(rules.hail_immune_types),
            WeatherKind::Sun | WeatherKind::Rain | WeatherKind::Snow | WeatherKind::StrongWinds => {
                None
            }
        };
        // Showdown `eachEvent('Weather')`: every active Pokémon fastest first, ties shuffled (a
        // `SpeedTie` decision each when the weather chips; otherwise only the order of the
        // messages would show it).
        let mut targets: Vec<(PokemonId, u32, u64)> = self
            .active_pokemon()
            .into_iter()
            .map(|id| (id, self.residual_speed(id), 0))
            .collect();
        if immune_types.is_some() {
            for (index, _) in self.weather_tie_groups() {
                targets[index].2 = self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
            }
        }
        targets.sort_by_key(|&(_, speed, tie_break)| {
            (std::cmp::Reverse(speed), std::cmp::Reverse(tie_break))
        });
        for (target, ..) in targets {
            if self.state.pokemon(target).is_none_or(|p| p.is_fainted()) {
                continue;
            }
            // The ability's `onWeather` first: the weather's own handler has no speed.
            self.run_weather_reactions(target);
            let Some(immune_types) = immune_types else {
                continue;
            };
            let Some(pokemon) = self.state.pokemon(target) else {
                continue;
            };
            if pokemon.is_fainted() {
                continue;
            }
            let ability = crate::engine::ability_handlers(&self.state, &self.rules, pokemon);
            let weather_proof = ability.weather_and_powder_immune
                || ability.weather_immunity == Some(weather.kind)
                || crate::engine::held_item(&self.state, &self.rules, target)
                    .is_some_and(|(_, item)| item.weather_and_powder_immune);
            let shielded = immune_types
                .iter()
                .any(|t| crate::engine::types_of(&self.state, &self.rules, pokemon).contains(*t))
                || weather_proof
                || crate::engine::two_turn::hiding(pokemon)
                    .is_some_and(|kind| kind.sheltered_from_weather());
            if shielded {
                self.trace.push(TraceEvent::WeatherImmune {
                    target,
                    weather: weather.kind,
                });
                continue;
            }
            // Showdown's sandstorm / hail chip is `pokemon.baseMaxhp / 16`: under a Dynamax it
            // reads the HP before the doubling (DESIGN.md §44.5).
            let damage = (pokemon.base_max_hp() / rules.damage_divisor.max(1)).max(1);
            self.trace.push(TraceEvent::WeatherDamage {
                target,
                weather: weather.kind,
                damage,
            });
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            ctx.apply_damage(crate::engine::DamageRequest {
                target,
                amount: crate::engine::DamageAmount(damage),
                kind: crate::engine::DamageKind::Weather,
                source: EffectSource::Weather(weather.kind),
                flags: crate::engine::DamageFlags::default(),
            });
        }
    }

    /// The holder's ability's `onWeather` (`AbilityBehavior::weather_reactions`) in the
    /// weather's upkeep: a heal, a loss or a cure, as the holder sees the weather.
    fn run_weather_reactions(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let (ability, max_hp) = (crate::engine::raw_ability(pokemon), pokemon.base_max_hp());
        let reactions = crate::engine::ability_handler(&self.state, &self.rules, pokemon, |b| {
            b.weather_reactions
        });
        for reaction in reactions {
            if crate::engine::weather_for(&self.state, &self.rules, holder)
                != Some(reaction.weather)
            {
                continue;
            }
            let source = EffectSource::Ability {
                pokemon: holder,
                ability,
            };
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            match reaction.effect {
                crate::effects::abilities::WeatherEffect::Heal(divisor) => {
                    ctx.apply_heal(crate::engine::HealRequest {
                        target: holder,
                        amount: (max_hp / divisor.max(1)).max(1),
                        source,
                        direct: false,
                    });
                }
                crate::effects::abilities::WeatherEffect::Damage(divisor) => {
                    ctx.apply_damage(crate::engine::DamageRequest {
                        target: holder,
                        amount: crate::engine::DamageAmount((max_hp / divisor.max(1)).max(1)),
                        kind: crate::engine::DamageKind::Residual,
                        source,
                        flags: crate::engine::DamageFlags::default(),
                    });
                }
                crate::effects::abilities::WeatherEffect::CureStatus => {
                    if ctx.has_status(holder) {
                        ctx.ability_activated(holder, ability);
                        ctx.cure_status(holder, crate::engine::CureReason::Ability(ability));
                    }
                }
            }
        }
    }

    /// Bind / Wrap / Fire Spin … (Showdown `partiallytrapped`): the duration ticks first (the
    /// last turn ends it without a chip); then, if the trapper has left the field, fainted or
    /// just came back (`activeTurns` 0), the bind ends silently; otherwise the chip
    /// (`ConditionRules::bind_damage_divisor`) is a residual `DamageRequest`.
    fn run_partially_trapped(&mut self, holder: PokemonId) {
        let left = self.state.pokemon_mut(holder).and_then(|p| {
            p.volatiles
                .tick_timed(VolatileConditionKind::PartiallyTrapped)
        });
        if left == Some(0) {
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind: VolatileConditionKind::PartiallyTrapped,
                seen: true,
            });
            return;
        }
        let Some((source, _)) = self
            .state
            .pokemon(holder)
            .and_then(|p| p.volatiles.partially_trapped_by())
        else {
            return;
        };
        let trapper_gone = self
            .state
            .active_position(source, &self.rules.topology)
            .is_none()
            || self
                .state
                .pokemon(source)
                .is_none_or(|p| p.is_fainted() || p.active_turns == 0);
        if trapper_gone {
            if let Some(pokemon) = self.state.pokemon_mut(holder) {
                pokemon
                    .volatiles
                    .remove(VolatileConditionKind::PartiallyTrapped);
            }
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind: VolatileConditionKind::PartiallyTrapped,
                seen: true,
            });
            return;
        }
        let Some(max_hp) = self.state.pokemon(holder).map(|p| p.base_max_hp()) else {
            return;
        };
        let banded = self
            .state
            .pokemon(holder)
            .is_some_and(|p| p.volatiles.partially_trapped_banded());
        let divisor = if banded {
            self.rules.conditions.binding_band_damage_divisor
        } else {
            self.rules.conditions.bind_damage_divisor
        };
        let amount = (max_hp / divisor.max(1)).max(1);
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount(amount),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Volatile(VolatileConditionKind::PartiallyTrapped),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// One residual tick of a timed volatile (Taunt / Encore / Disable / Heal Block): Showdown's
    /// `duration--`, ending the condition at 0.
    fn tick_volatile(&mut self, holder: PokemonId, kind: VolatileConditionKind) {
        let left = self
            .state
            .pokemon_mut(holder)
            .and_then(|pokemon| pokemon.volatiles.tick_timed(kind));
        if left == Some(0) {
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind,
                seen: true,
            });
        }
    }

    /// Leech Seed (Showdown `leechseed` `onResidual`, Gen 2 `onAfterMoveSelf`): nothing
    /// happens unless someone stands at the seeder's position; then the holder loses
    /// `max HP / 8` and that Pokémon heals what was taken.
    pub(super) fn run_leech_seed(&mut self, holder: PokemonId) {
        let Some((max_hp, source_position)) = self.state.pokemon(holder).and_then(|pokemon| {
            pokemon
                .volatiles
                .leech_seed_source_position()
                .map(|position| (pokemon.base_max_hp(), position))
        }) else {
            return;
        };
        let receiver = self
            .state
            .pokemon_at_position(source_position, &self.rules.topology)
            .filter(|receiver| {
                self.state
                    .pokemon(*receiver)
                    .is_some_and(|pokemon| !pokemon.is_fainted())
            });
        let Some(receiver) = receiver else {
            return;
        };
        let mut amount = (max_hp / self.rules.conditions.leech_seed_hp_divisor.max(1)).max(1);
        // Gen 1: the `residualdmg` counter advances and multiplies the drain.
        let gen1 = self.rules.conditions.leech_seed_gen1;
        if gen1
            && let Some(pokemon) = self.state.pokemon_mut(holder)
            && pokemon.volatiles.residual_damage_counter().is_some()
        {
            pokemon.volatiles.advance_residual_damage();
            let counter = pokemon.volatiles.residual_damage_counter().unwrap_or(1);
            amount = amount.saturating_mul(u16::from(counter));
        }
        let holder_down = self.state.pokemon(holder).is_none_or(|p| p.is_fainted());
        let drained = if holder_down {
            0
        } else {
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            match ctx.apply_damage(crate::engine::DamageRequest {
                target: holder,
                amount: crate::engine::DamageAmount(amount),
                kind: crate::engine::DamageKind::Residual,
                source: EffectSource::Volatile(VolatileConditionKind::LeechSeed),
                flags: crate::engine::DamageFlags::default(),
            }) {
                DamageApplication::Hp { amount, .. } => amount.0,
                _ => 0,
            }
        };
        // Gen 1 heals the full amount, however little HP the seeded Pokémon had left.
        let healed = if gen1 { amount } else { drained };
        if healed == 0 {
            return;
        }
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.drain_heal(
            crate::engine::HealRequest {
                target: receiver,
                amount: healed,
                source: EffectSource::Volatile(VolatileConditionKind::LeechSeed),
                direct: false,
            },
            holder,
            crate::engine::HealDrain::LeechSeed,
        );
    }

    /// Gen 2's thaw (`frz` `onResidualOrder` 7): a frozen Pokémon thaws with the generation's
    /// chance at the end of the turn.
    pub(super) fn run_freeze_thaw(&mut self, holder: PokemonId) {
        if !self
            .state
            .pokemon(holder)
            .is_some_and(|p| p.has_status(NonVolatileStatus::Freeze))
        {
            return;
        }
        let chance = self.rules.status.lifecycle.thaw_chance;
        if self.state.rng.roll_ratio_for(
            RandomRequestKind::Thaw,
            chance.numerator,
            chance.denominator,
        ) {
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            ctx.cure_status(holder, crate::engine::CureReason::Thawed);
        }
    }

    /// Gen 1-2 (`StatusResidualTiming::AfterOwnAction`, Showdown `onAfterMoveSelf`): after the
    /// Pokémon's own action — also one stopped at BeforeMove (`stopped`), but not a move after
    /// which the foe in front of it has fainted — the `residualdmg` counter of a statused
    /// holder advances, then poison / burn / bad poison take their HP, then Leech Seed, then
    /// Gen 2's Nightmare. The actor's own HP does not matter (its confusion, recoil or
    /// Explosion may have left it at 0): damage to it then does nothing, but Gen 1's Leech Seed
    /// still heals.
    pub(super) fn run_after_own_action(
        &mut self,
        actor: PokemonId,
        stopped: bool,
        aimed_at_self: bool,
    ) {
        if self.rules.status.residual_timing != StatusResidualTiming::AfterOwnAction {
            return;
        }
        let Some((side, down)) = self.state.pokemon(actor).map(|p| (p.side, p.is_fainted())) else {
            return;
        };
        // Gen 1: a move aimed at its user asks the user itself to stand
        // (`ConditionRules::after_own_action_reads_the_target`).
        if !stopped
            && aimed_at_self
            && down
            && self.rules.conditions.after_own_action_reads_the_target
        {
            return;
        }
        let foe_standing = self.state.side(side.other()).active[0]
            .and_then(|foe| self.state.pokemon(foe))
            .is_some_and(|foe| !foe.is_fainted());
        if !stopped && !foe_standing {
            return;
        }
        self.advance_residual_damage(actor);
        if !down {
            self.run_status_residual(actor);
        }
        // Gen 1's Leech Seed still runs on a holder at 0 HP (Showdown gen1 `onAfterMoveSelf`
        // has no HP check; Gen 2's returns at 0 HP): the seeder heals in full.
        if self.state.pokemon(actor).is_some_and(|p| {
            (!p.is_fainted() || self.rules.conditions.leech_seed_gen1)
                && p.volatiles.contains(VolatileConditionKind::LeechSeed)
        }) {
            self.run_leech_seed(actor);
        }
        // Gen 2 Nightmare (`onAfterMoveSelfPriority` 1, after Leech Seed's 2); it ends when
        // the holder wakes, so a holder that still has it is asleep.
        if self.state.pokemon(actor).is_some_and(|p| {
            !p.is_fainted() && p.volatiles.contains(VolatileConditionKind::Nightmare)
        }) {
            self.run_nightmare(actor);
        }
        // Gen 2 Curse (`onAfterMoveSelfPriority` 0: last; pokecrystal `ResidualDamage` too).
        if self
            .state
            .pokemon(actor)
            .is_some_and(|p| !p.is_fainted() && p.volatiles.contains(VolatileConditionKind::Curse))
        {
            self.run_curse(actor);
        }
    }

    /// Gen 2 on switch-in (Showdown `tox` `onSwitchIn`, then `onAfterSwitchInSelf` unless the
    /// side already lost a Pokémon this turn): bad poison becomes poison, and poison / burn
    /// take their HP at once — the status's handler first (no priority), so a `residualdmg`
    /// counter handed on by Baton Pass scales this damage before it advances.
    fn run_after_switch_in_status(&mut self, pokemon_id: PokemonId) {
        if self.rules.status.residual_timing != StatusResidualTiming::AfterOwnAction {
            return;
        }
        if self.rules.status.toxic_model == ToxicModel::ResidualVolatile
            && let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
            && let Some(status) = &mut pokemon.status
            && status.kind == NonVolatileStatus::Toxic
        {
            status.kind = NonVolatileStatus::Poison;
        }
        let Some(side) = self.state.pokemon(pokemon_id).map(|p| p.side) else {
            return;
        };
        // Gen 2: not for a Pokémon a drag brought in (`draggedIn === turn`).
        let dragged = self.state.dragged_in.contains(&pokemon_id);
        self.state
            .dragged_in
            .retain(|pokemon| *pokemon != pokemon_id);
        if self.state.side(side).fainted_this_turn || dragged {
            return;
        }
        self.run_status_residual(pokemon_id);
        self.advance_residual_damage(pokemon_id);
    }

    /// Gen 2 `residualdmg` `onAfterMoveSelf` / `onAfterSwitchInSelf`: a burned, poisoned or
    /// badly poisoned holder's counter advances.
    fn advance_residual_damage(&mut self, pokemon_id: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
            && pokemon.status.is_some_and(|status| {
                matches!(
                    status.kind,
                    NonVolatileStatus::Burn | NonVolatileStatus::Poison | NonVolatileStatus::Toxic
                )
            })
        {
            pokemon.volatiles.advance_residual_damage();
        }
    }

    pub(super) fn run_status_residual(&mut self, pokemon_id: PokemonId) {
        let Some((status_state, max_hp)) = self
            .state
            .pokemon(pokemon_id)
            .and_then(|pokemon| pokemon.status.map(|status| (status, pokemon.base_max_hp())))
        else {
            return;
        };
        let status = status_state.kind;
        // Gen 2: the `residualdmg` counter, if any, scales every status's damage.
        let residual_counter = self
            .state
            .pokemon(pokemon_id)
            .and_then(|pokemon| pokemon.volatiles.residual_damage_counter())
            .filter(|_| self.rules.status.toxic_model == ToxicModel::ResidualVolatile);
        let damage = match (status, residual_counter) {
            (
                NonVolatileStatus::Burn | NonVolatileStatus::Poison | NonVolatileStatus::Toxic,
                Some(counter),
            ) => {
                let sixteenth = max_hp / self.rules.status.toxic_residual_divisor.max(1);
                let damage = if status == NonVolatileStatus::Toxic {
                    u32::from(sixteenth.max(1)) * u32::from(counter)
                } else {
                    (u32::from(sixteenth) * u32::from(counter)).max(1)
                };
                damage.min(u32::from(u16::MAX)) as u16
            }
            (NonVolatileStatus::Burn, _) => {
                let damage = (max_hp / self.rules.status.burn_residual_divisor.max(1)).max(1);
                // Heatproof (`onDamage`): half of it, still at least 1.
                let halved = self.state.pokemon(pokemon_id).is_some_and(|p| {
                    crate::engine::ability_handler(&self.state, &self.rules, p, |b| {
                        b.halves_status_damage
                    }) == Some(NonVolatileStatus::Burn)
                });
                if halved { (damage / 2).max(1) } else { damage }
            }
            (NonVolatileStatus::Poison, _) => {
                (max_hp / self.rules.status.poison_residual_divisor.max(1)).max(1)
            }
            (NonVolatileStatus::Toxic, _) => {
                // Showdown: `stage++` (capped at 15), then `clampIntRange(maxhp / 16, 1) *
                // stage` — the sixteenth is truncated first, then multiplied.
                let stage = status_state.toxic_counter.saturating_add(1).min(15);
                if let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
                    && let Some(status) = &mut pokemon.status
                {
                    status.toxic_counter = stage;
                }
                let divisor = self.rules.status.toxic_residual_divisor.max(1);
                let sixteenth = u32::from((max_hp / divisor).max(1));
                sixteenth
                    .saturating_mul(u32::from(stage))
                    .min(u32::from(u16::MAX)) as u16
            }
            _ => return,
        };
        self.trace.push(TraceEvent::StatusResidual {
            target: pokemon_id,
            status,
            damage,
        });
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: pokemon_id,
            amount: crate::engine::DamageAmount(damage),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Status(status),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// Entry hazards in the order they were laid (Showdown runs the side's SwitchIn handlers in
    /// effect order). Spikes / Toxic Spikes / Sticky Web need a grounded target; a grounded
    /// Poison type absorbs Toxic Spikes, Heavy-Duty Boots or not (Toxic Spikes checks the type
    /// before the boots); otherwise the boots skip every hazard. From Gen 5 a hazard that
    /// knocks the Pokémon out stops the rest; Gen 1-4 run them all — damage and status then do
    /// nothing to the fainted Pokémon, but a Poison type still absorbs Toxic Spikes
    /// (`ConditionRules::hazards_stop_at_knock_out`).
    pub(super) fn apply_entry_hazards(&mut self, pokemon_id: PokemonId) {
        let Some((side, max_hp, types, grounded, boots)) =
            self.state.pokemon(pokemon_id).map(|pokemon| {
                (
                    pokemon.side,
                    pokemon.max_hp,
                    crate::engine::types_of(&self.state, &self.rules, pokemon),
                    crate::engine::environment::grounded(&self.state, &self.rules, pokemon),
                    crate::engine::held_item_id(&self.state, &self.rules, pokemon_id)
                        == Some(ItemId::HEAVY_DUTY_BOOTS),
                )
            })
        else {
            return;
        };

        let hazards: Vec<SideCondition> = self.state.side(side).conditions.entries().to_vec();
        for hazard in hazards {
            let fainted = self
                .state
                .pokemon(pokemon_id)
                .is_none_or(|pokemon| pokemon.is_fainted());
            if fainted && self.rules.conditions.hazards_stop_at_knock_out {
                return;
            }
            match hazard {
                SideCondition::StealthRock { .. } => {
                    if boots {
                        continue;
                    }
                    let modifier = self
                        .rules
                        .type_chart
                        .combined_effectiveness(PokemonType::Rock, types.iter());
                    if modifier.is_zero() {
                        continue;
                    }
                    let divisor = u32::from(self.rules.conditions.stealth_rock_hp_divisor.max(1));
                    let denominator =
                        divisor.saturating_mul(u32::from(modifier.denominator.max(1)));
                    let raw = u32::from(max_hp).saturating_mul(u32::from(modifier.numerator))
                        / denominator.max(1);
                    let damage = raw.max(1).min(u32::from(u16::MAX)) as u16;
                    self.hazard_damage(side, pokemon_id, SideConditionKind::StealthRock, damage);
                }
                SideCondition::Spikes { layers, .. } => {
                    if !grounded || boots {
                        continue;
                    }
                    let table = self.rules.conditions.spikes_damage_24ths;
                    let index = usize::from(layers.clamp(1, table.len() as u8)) - 1;
                    let damage = (u32::from(max_hp) * u32::from(table[index]) / 24)
                        .max(1)
                        .min(u32::from(u16::MAX)) as u16;
                    self.hazard_damage(side, pokemon_id, SideConditionKind::Spikes, damage);
                }
                SideCondition::ToxicSpikes { layers, .. } => {
                    let absorbs = types.contains(PokemonType::Poison);
                    if !grounded || (boots && !absorbs) {
                        continue;
                    }
                    self.trace.push(TraceEvent::EntryHazardTriggered {
                        side,
                        target: pokemon_id,
                        kind: SideConditionKind::ToxicSpikes,
                        damage: 0,
                    });
                    let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                    if absorbs {
                        ctx.remove_side_condition(side, SideConditionKind::ToxicSpikes);
                    } else if !types.contains(PokemonType::Steel) {
                        let status = if layers >= 2 {
                            NonVolatileStatus::Toxic
                        } else {
                            NonVolatileStatus::Poison
                        };
                        ctx.try_set_status(
                            pokemon_id,
                            status,
                            EffectSource::SideCondition(SideConditionKind::ToxicSpikes),
                        );
                    }
                }
                SideCondition::StickyWeb { source } => {
                    if !grounded || boots {
                        continue;
                    }
                    self.trace.push(TraceEvent::EntryHazardTriggered {
                        side,
                        target: pokemon_id,
                        kind: SideConditionKind::StickyWeb,
                        damage: 0,
                    });
                    // Gen 9: from the entrant's foe in the first slot (an empty slot — its
                    // Pokémon down — leaves no one to send the drop back to), Gen 6-8 from
                    // the setter.
                    let from = if self.rules.conditions.sticky_web_drop_from_foe_lead {
                        self.state.side(side.other()).active[0]
                    } else {
                        Some(source)
                    };
                    let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                    ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
                        target: pokemon_id,
                        stat: crate::state::Stat::Speed,
                        stages: -1,
                        source: from.map_or(
                            EffectSource::SideCondition(SideConditionKind::StickyWeb),
                            |pokemon| EffectSource::Move {
                                pokemon,
                                move_id: MoveId::STICKY_WEB,
                            },
                        ),
                        cause: crate::engine::StatChangeCause::Move,
                    });
                }
                SideCondition::Tailwind { .. }
                | SideCondition::WideGuard { .. }
                | SideCondition::QuickGuard { .. }
                | SideCondition::CraftyShield { .. }
                | SideCondition::MatBlock { .. }
                | SideCondition::Timed { .. } => {}
            }
        }
    }

    pub(super) fn hazard_damage(
        &mut self,
        side: SideId,
        target: PokemonId,
        kind: SideConditionKind,
        damage: u16,
    ) {
        self.trace.push(TraceEvent::EntryHazardTriggered {
            side,
            target,
            kind,
            damage,
        });
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target,
            amount: crate::engine::DamageAmount(damage),
            kind: crate::engine::DamageKind::EntryHazard,
            source: EffectSource::SideCondition(kind),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    pub(super) fn clear_on_switch(&mut self, pokemon_id: PokemonId) {
        // Mean Look's links (`removeLinkedVolatiles`), before the volatiles go; Sky Drop lets go
        // of the Pokémon it carries as its user leaves (its condition ends with it).
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.release_trap_links(pokemon_id);
        ctx.end_sky_drop(pokemon_id);
        let expired = self
            .state
            .pokemon_mut(pokemon_id)
            .map(|pokemon| {
                pokemon.stages = StatStages::default();
                if let Some(status) = &mut pokemon.status
                    && status.kind == NonVolatileStatus::Toxic
                {
                    status.toxic_counter = 0;
                }
                pokemon.last_move = None;
                pokemon.last_move_encore = None;
                pokemon.last_move_base = None;
                pokemon.last_move_z = false;
                pokemon.last_move_used = None;
                pokemon.last_move_target = None;
                pokemon.active_turns = 0;
                pokemon.active_move_actions = 0;
                pokemon.history.leave_field();
                // `clearVolatile`: Transform's copy goes (DESIGN.md §41.7), before the stats,
                // types and ability fall back to the Pokémon's own — and with it Mimic's
                // overlay of the move slots (§42.3; Sketch's is permanent).
                pokemon.transform = None;
                // `setSpecies(baseSpecies)`: a forme it changed into for the stint goes
                // (DESIGN.md §43.1).
                pokemon.forme = None;
                pokemon.drop_move_overlay();
                // Gen 1: the modified stats go back to the stored ones (status cuts re-applied).
                pokemon.restore_stored_stats();
                pokemon.reset_modified_stats();
                // Showdown `setSpecies(baseSpecies)`: a Forecast forme or Mimicry's type goes;
                // `clearVolatile`: the ability it came in with comes back (its `End` ran first).
                pokemon.restore_types();
                pokemon.restore_ability();
                // `volatileStaleness = undefined` (DESIGN.md §45.7).
                pokemon.volatile_staleness = None;
                pokemon.volatiles.clear_on_switch()
            })
            .unwrap_or_default();
        for kind in expired {
            self.trace.push(TraceEvent::VolatileEnded {
                target: pokemon_id,
                kind,
                seen: true,
            });
        }
    }

    /// Gen 1 `runSwitch`'s Metronome glitch: a foe of the entrant locked into a binding move
    /// while its side's last selected slot holds Metronome (the bind Metronome called) goes back
    /// to Metronome — the side's last selected move, and its action this turn if it still has
    /// one. The lock's `onBeforeMove` would end it there (its target is gone); it ends here, so
    /// the action runs Metronome rather than the lock's move (R-M8-115).
    fn metronome_binding_glitch(&mut self, entrant: PokemonId) {
        if !self.rules.conditions.switching.binding_locks_user {
            return;
        }
        let Some(side) = self.state.pokemon(entrant).map(|p| p.side.other()) else {
            return;
        };
        let foes: Vec<PokemonId> = self
            .state
            .side(side)
            .active
            .iter()
            .flatten()
            .copied()
            .collect();
        let slot = self.state.side(side).last_selected_move_slot;
        for foe in foes {
            let glitched = self.state.pokemon(foe).is_some_and(|pokemon| {
                pokemon.volatiles.partial_trapping_lock().is_some()
                    && pokemon
                        .move_slots()
                        .get(slot)
                        .is_some_and(|m| m.move_id == crate::ids::MoveId::METRONOME)
            });
            if !glitched {
                continue;
            }
            self.state.side_mut(side).last_selected_move = Some(crate::state::SelectedMove::Move(
                crate::ids::MoveId::METRONOME,
            ));
            let PendingExecution::Turn { actions, .. } = &mut self.pending else {
                continue;
            };
            let Some(action) = actions.iter_mut().find(|action| {
                action.actor == foe
                    && matches!(action.choice, crate::engine::ActionChoice::Move { .. })
            }) else {
                continue;
            };
            if let crate::engine::ActionChoice::Move { slot: queued, .. } = &mut action.choice
                && let Some(metronome) = crate::ids::MoveSlot::new(slot as u8)
            {
                *queued = metronome;
            }
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .end_volatile(foe, VolatileConditionKind::PartialTrappingLock);
        }
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

    /// Gen 3 Follow Me's slot condition lasts the turn: it ends on whoever stands there (seen on
    /// the user, `VolatileStarted`).
    fn clear_follow_me_slots(&mut self, side: SideId) {
        for slot in std::mem::take(&mut self.state.side_mut(side).follow_me_slots) {
            if let Some(target) = self.state.side(side).active.get(slot).copied().flatten() {
                self.trace.push(TraceEvent::VolatileEnded {
                    target,
                    kind: VolatileConditionKind::FollowMe,
                    seen: true,
                });
            }
        }
    }

    /// The end of the turn's expiry; `positioned` (Gen 2+) leaves the volatiles', sides' and
    /// field's durations to their residual handlers (`collect_durations`).
    pub(super) fn expire_conditions_end_turn(&mut self, positioned: bool) {
        // Gen 1 binding's `duration`s (§30.9).
        self.tick_binding();
        if positioned {
            for side in [SideId::P1, SideId::P2] {
                self.clear_follow_me_slots(side);
            }
            return;
        }
        let turn = self.state.turn;
        let pokemon_ids = self.state.pokemon_ids().collect::<Vec<_>>();
        // Showdown `twoturnmove` (duration 2): ticked at the end of the charging turn and of
        // the attacking turn, ending after the latter.
        for pokemon_id in pokemon_ids.iter().copied() {
            // Gen 1's charge has no duration (`ChargeModel::Gen1`).
            if !self.rules.two_turn.gen1() {
                self.tick_volatile(pokemon_id, VolatileConditionKind::TwoTurnMove);
            }
            // Showdown `stall` (duration 2): Protect's counter survives exactly one turn.
            self.tick_volatile(pokemon_id, VolatileConditionKind::Stall);
            // Showdown `mustrecharge` (duration 2): gone at BeforeMove, or after the next turn.
            self.tick_volatile(pokemon_id, VolatileConditionKind::MustRecharge);
            // Showdown `toxic` (duration 1): a Poison type's sure-hit Toxic, for the turn.
            self.tick_volatile(pokemon_id, VolatileConditionKind::SureHit);
            // Showdown `magiccoat` (duration 1): the coat lasts the turn (Gen 4 ended it as it
            // reflected, §42.8).
            self.tick_volatile(pokemon_id, VolatileConditionKind::MagicCoat);
            // Showdown `snatch` and `mefirst` (duration 1) too.
            self.tick_volatile(pokemon_id, VolatileConditionKind::Snatch);
            self.tick_volatile(pokemon_id, VolatileConditionKind::MeFirst);
            // Showdown `allyswitch` (duration 2): the counter survives a turn.
            self.tick_volatile(pokemon_id, VolatileConditionKind::AllySwitch);
            // Chilly Reception's announcement (duration 1).
            self.tick_volatile(pokemon_id, VolatileConditionKind::ChillyReception);
            // Focus Punch's, Beak Blast's, Shell Trap's (duration 1).
            self.tick_volatile(pokemon_id, VolatileConditionKind::FocusPunch);
            self.tick_volatile(pokemon_id, VolatileConditionKind::BeakBlast);
            self.tick_volatile(pokemon_id, VolatileConditionKind::ShellTrap);
            self.tick_volatile(pokemon_id, VolatileConditionKind::Counter);
            self.tick_volatile(pokemon_id, VolatileConditionKind::MirrorCoat);
            // Gen 2+ Bide's `duration`: past its release turn it just ends.
            self.tick_volatile(pokemon_id, VolatileConditionKind::Bide);
            // Rollout / Ice Ball (`duration` 1, back to 2 by each of the first four hits).
            self.tick_volatile(pokemon_id, VolatileConditionKind::Rolling);
        }
        for pokemon_id in pokemon_ids {
            let expired = self
                .state
                .pokemon_mut(pokemon_id)
                .map(|pokemon| pokemon.volatiles.expire_end_turn(turn))
                .unwrap_or_default();
            for kind in expired {
                self.trace.push(TraceEvent::VolatileEnded {
                    target: pokemon_id,
                    kind,
                    seen: true,
                });
            }
        }

        for side in [SideId::P1, SideId::P2] {
            self.clear_follow_me_slots(side);
            let expired = self.state.side_mut(side).conditions.tick_end_turn(
                !self.rules.conditions.tailwind_ticks_first,
                !self.rules.conditions.side_protections.tick_first,
            );
            for kind in expired {
                self.trace
                    .push(TraceEvent::SideConditionEnded { side, kind });
            }
        }

        let expiry = self.state.field.tick_end_turn();
        for kind in expiry.conditions {
            self.trace.push(TraceEvent::FieldConditionEnded { kind });
        }
        if let Some(terrain) = expiry.terrain {
            self.trace.push(TraceEvent::TerrainChanged {
                from: Some(terrain),
                to: None,
            });
            // Showdown `clearTerrain` → `eachEvent('TerrainChange')` (Quark Drive).
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace).terrain_changed();
        }
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

#[derive(Debug, Clone, Copy)]
pub(super) struct ResidualInstance {
    /// `None` for a field-scoped residual (the weather).
    holder: Option<PokemonId>,
    kind: ResidualKind,
    order: u8,
    sub_order: u8,
    speed: u32,
    slot: usize,
    /// A slot condition's place (Wish, Future Sight): it counts down there even once its holder
    /// has fainted (Showdown `fieldEvent` keeps a fainted holder's `isSlotCondition` handlers).
    position: Option<(SideId, usize)>,
}

/// A handler of the residual: one with a callback, or one that only counts an effect down.
#[derive(Debug, Clone, Copy)]
enum ResidualHandler {
    Instance(ResidualInstance),
    Duration(super::durations::DurationInstance),
}

impl ResidualHandler {
    /// `(order, speed, sub-order, slot)`.
    fn key(&self) -> (u8, u32, u8, usize) {
        match self {
            Self::Instance(instance) => (
                instance.order,
                instance.speed,
                instance.sub_order,
                instance.slot,
            ),
            Self::Duration(duration) => (
                duration.order,
                duration.speed,
                duration.sub_order,
                usize::MAX,
            ),
        }
    }
}

/// Showdown `speedSort` on the residual handlers: the ones equal in order, speed and sub-order
/// (the field's weather ties nothing) are shuffled — the duration-only ones too, among
/// themselves and with the others (two sides' Safeguards, a Pokémon's Protect counter and its
/// Endure). Returns (index, tie group) in collection order — the instances, then the durations
/// past them — the order their `SpeedTie` decisions are drawn.
fn residual_tie_groups(
    instances: &[ResidualInstance],
    durations: &[super::durations::DurationInstance],
) -> Vec<(usize, usize)> {
    let keys: Vec<Option<(u8, u32, u8)>> = instances
        .iter()
        .map(|instance| {
            instance
                .holder
                .map(|_| (instance.order, instance.speed, instance.sub_order))
        })
        .chain(
            durations
                .iter()
                .map(|duration| Some((duration.order, duration.speed, duration.sub_order))),
        )
        .collect();
    let mut groups: Vec<(u8, u32, u8)> = Vec::new();
    let mut tied = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let Some(key) = key else {
            continue;
        };
        if !keys
            .iter()
            .enumerate()
            .any(|(other, k)| other != index && k.as_ref() == Some(key))
        {
            continue;
        }
        let group = groups.iter().position(|g| g == key).unwrap_or_else(|| {
            groups.push(*key);
            groups.len() - 1
        });
        tied.push((index, group));
    }
    tied
}

/// Indices of equal values, each with its group (first-seen order).
fn tie_groups(speeds: &[u32]) -> Vec<(usize, usize)> {
    let mut groups: Vec<u32> = Vec::new();
    let mut tied = Vec::new();
    for (index, speed) in speeds.iter().enumerate() {
        if speeds.iter().filter(|s| *s == speed).count() < 2 {
            continue;
        }
        let group = groups.iter().position(|g| g == speed).unwrap_or_else(|| {
            groups.push(*speed);
            groups.len() - 1
        });
        tied.push((index, group));
    }
    tied
}

/// How a residual handler ran (Showdown `fieldEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResidualRun {
    /// Its callback ran; the faints it caused are settled right after it.
    Ran,
    /// Its count ran out and its `end` ran instead: the faints it caused wait for the next
    /// handler's settling (or the residual's end).
    Ended,
}
