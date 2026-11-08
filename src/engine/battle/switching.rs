//! `BattleEngine`'s switches (DESIGN.md §30.1): the switch-in stage — Showdown `runSwitch`, each
//! entrant's steps, the entry hazards — the switch out and its cleanup, a placement, the drags,
//! the leads.

use super::BattleEngine;
use crate::dex::PokemonType;
use crate::effects;
use crate::engine::stats::action_speed;
use crate::engine::{
    BattleContext, BattleError, EffectSource, PendingExecution, PendingFaints, TraceEvent,
};
use crate::ids::{ItemId, MoveId, PokemonId, PositionId, SideId};
use crate::rules::{StatusResidualTiming, SwitchInOrder, ToxicModel};
use crate::state::{
    BattlePhase, NonVolatileStatus, RandomRequestKind, SideCondition, SideConditionKind,
    StatStages, VolatileConditionKind,
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
        // Showdown `switchIn`: the Pokémon chosen is already in — an Eject Pack's, an Eject
        // Button's or a self-switch's replacement earlier in the turn — and the switch fails
        // before anything of it (no `BeforeSwitchOut`): the Pokémon stays (R-M11-4).
        if self
            .state
            .active_position(incoming, &self.rules.topology)
            .is_some()
        {
            return Ok(());
        }
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
                seen_as: previous,
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
    /// switch-in fastest first — in Gen 3-4 as the queued `runSwitch` action, after the dragging
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
            // The start-of-battle `endTurn`: the first turn's top (`turn_start_items`), then its
            // rules (Gen 1: two teams that only Transform).
            self.turn_start_items();
            self.end_turn_rules();
        }
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
}
