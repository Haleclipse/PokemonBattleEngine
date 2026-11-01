//! Items changing hands (DESIGN.md §39.3, R211): Showdown's `takeItem`, the `TakeItem` event,
//! `setItem`, and the moves built on them — Knock Off, Thief, Covet.

use crate::ids::{AbilityId, ItemId, MoveId, PokemonId};
use crate::state::VolatileConditionKind;

use super::{BattleContext, TraceEvent};

/// What Showdown's `takeItem` returns (`BattleContext::try_take_item`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Taken {
    Item(ItemId),
    /// `false`: something refused.
    Refused,
    /// `undefined`: there was nothing to take.
    Nothing,
}

impl<'a> BattleContext<'a> {
    /// Showdown `holder.takeItem(taker)` while `move_id` is the active move: Gen 4 nothing when
    /// either was marked by a Knock Off (`ItemTransferRules::knocked_off_blocks_taking`), nothing
    /// without an item, nothing when the `TakeItem` event refuses; otherwise the item is gone.
    pub fn take_item(
        &mut self,
        holder: PokemonId,
        taker: PokemonId,
        move_id: Option<MoveId>,
    ) -> Option<ItemId> {
        match self.try_take_item(holder, taker, move_id, true) {
            Taken::Item(item) => Some(item),
            Taken::Refused | Taken::Nothing => None,
        }
    }

    /// `take_item`, telling a refusal from an empty hand (Trick fails on either side's refusal).
    fn try_take_item(
        &mut self,
        holder: PokemonId,
        taker: PokemonId,
        move_id: Option<MoveId>,
        seen: bool,
    ) -> Taken {
        let taken = self.may_take_item(holder, taker, move_id);
        if let Taken::Item(item) = taken {
            self.remove_item(holder, item, taker, move_id, seen);
        }
        taken
    }

    /// `holder.takeItem(taker)` up to the item leaving its holder: the item it would take.
    fn may_take_item(
        &mut self,
        holder: PokemonId,
        taker: PokemonId,
        move_id: Option<MoveId>,
    ) -> Taken {
        let marked = |id: PokemonId| {
            self.state
                .pokemon(id)
                .is_some_and(|p| p.history.item_knocked_off)
        };
        if self.rules.conditions.items.knocked_off_blocks_taking
            && (marked(holder) || marked(taker))
        {
            return Taken::Refused;
        }
        let Some(item) = self.state.pokemon(holder).and_then(|p| p.held_item) else {
            return Taken::Nothing;
        };
        if !self.take_item_event(holder, taker, move_id, item) {
            return Taken::Refused;
        }
        Taken::Item(item)
    }

    /// Showdown `runEvent('TakeItem', holder, taker, move, item)`: the holder's ability first —
    /// Gen 4 Multitype keeps the item, Sticky Hold refuses another taker or a Knock Off (Gen 5+
    /// not for a fainted holder; its Sticky Barb exception comes with that item), Unburden
    /// starts its volatile whatever follows — then the item's own refusal
    /// (`engine::item_refuses_take`). Suppressed items answer too (`TakeItem` is exempt from
    /// `ignoringItem`).
    fn take_item_event(
        &mut self,
        holder: PokemonId,
        taker: PokemonId,
        move_id: Option<MoveId>,
        item: ItemId,
    ) -> bool {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return false;
        };
        let fainted = pokemon.is_fainted();
        let behavior = crate::engine::ability_handlers(self.state, self.rules, pokemon);
        if behavior.keeps_item {
            return false;
        }
        if behavior.sticky_hold {
            // Gen 5+ (`!pokemon.hp || pokemon.item === 'stickybarb'`): a fainted holder, or one
            // giving up a Sticky Barb, is not held back.
            let spared = self.rules.conditions.items.sticky_hold_spares_fainted
                && (fainted || item == ItemId::STICKY_BARB);
            if !spared && (taker != holder || move_id == Some(MoveId::KNOCK_OFF)) {
                self.ability_activated(holder, AbilityId::STICKY_HOLD);
                return false;
            }
        }
        if behavior.unburden {
            let _ = self.set_marker_volatile(holder, VolatileConditionKind::Unburden);
        }
        !crate::engine::item_refuses_take(self.state, self.rules, item, holder, taker, move_id)
    }

    /// The item leaves its holder (Showdown `item = ''`; the `End` and `AfterTakeItem` that
    /// follow have no implemented participant).
    fn remove_item(
        &mut self,
        holder: PokemonId,
        item: ItemId,
        taker: PokemonId,
        move_id: Option<MoveId>,
        seen: bool,
    ) {
        // `clearEffectState(itemState)`; `pendingStaleness = undefined`.
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.held_item = None;
            pokemon.item_state = crate::state::ItemState::default();
            pokemon.pending_staleness = false;
        }
        self.trace.push(TraceEvent::ItemTaken {
            holder,
            item,
            taker,
            move_id,
            seen,
        });
    }

    /// Showdown `receiver.setItem(item)` for a receiver standing on the field — every taker so
    /// far is: a move's `onAfterHit` asks for its user's HP, and a fallen Pickpocket or Magician
    /// holder has left the field before its handler would run (`faintMessages` comes first) —
    /// its `Start` runs (suppressed or not), as on entry.
    fn give_item(&mut self, receiver: PokemonId, item: ItemId, from: PokemonId, seen: bool) {
        // `initEffectState` for the new item's `itemState`; `pendingStaleness` (DESIGN.md
        // §45.7).
        if let Some(pokemon) = self.state.pokemon_mut(receiver) {
            pokemon.held_item = Some(item);
            pokemon.item_state = crate::state::ItemState::default();
            pokemon.pending_staleness = true;
        }
        if self.state.acting_slot.is_some() {
            self.state.items_received_in_action.push(receiver);
        }
        self.trace.push(TraceEvent::ItemReceived {
            receiver,
            item,
            from,
            seen,
        });
        if let Some(start) = crate::engine::item_behavior(self.rules, item).switch_in {
            start(self, receiver);
        }
    }

    /// Sticky Barb's `onHit`: the user of a contact move that holds nothing (`!source.item`) takes
    /// the holder's barb — the holder gives it up itself (`target.takeItem()`: Sticky Hold lets
    /// it, Gen 5+ even for Knock Off) — with no message. Gen 4
    /// (`ItemTransferRules::sticky_barb_after_contact_abilities`, issues.md I100): called after
    /// the contact abilities, and not for a user knocked out by then, a Knock Off, or a user a
    /// Knock Off marked. (A Gen 4 Knock Off has knocked the barb off by then — its `AfterHit`
    /// comes before `DamagingHit` — so its check only mirrors the cartridge's.)
    pub fn sticky_barb_transfer(&mut self, hit: crate::engine::DamagingHit) {
        let Some(attacker) = self.state.pokemon(hit.attacker) else {
            return;
        };
        if hit.attacker == hit.holder || attacker.held_item.is_some() || !hit.contact {
            return;
        }
        if self
            .rules
            .conditions
            .items
            .sticky_barb_after_contact_abilities
            && (attacker.is_fainted()
                || hit.move_id == MoveId::KNOCK_OFF
                || attacker.history.item_knocked_off)
        {
            return;
        }
        let Taken::Item(item) =
            self.try_take_item(hit.holder, hit.holder, Some(hit.move_id), false)
        else {
            return;
        };
        // `source.setItem(barb)` refuses a user with no HP left (Final Gambit's, Showdown
        // `setItem`: `!this.hp`), and nothing gives the barb back: it is gone (R-M8-106).
        if self
            .state
            .pokemon(hit.attacker)
            .is_some_and(|attacker| attacker.is_fainted())
        {
            return;
        }
        self.give_item(hit.attacker, item, hit.holder, false);
    }

    /// `taker` takes `holder`'s item into its own hands (`holder.takeItem(taker)`, then
    /// `taker.setItem`). Returns whether it took it.
    fn transfer_item(
        &mut self,
        holder: PokemonId,
        taker: PokemonId,
        move_id: Option<MoveId>,
    ) -> bool {
        let Some(item) = self.take_item(holder, taker, move_id) else {
            return false;
        };
        self.give_item(taker, item, holder, true);
        true
    }

    /// Knock Off's `onAfterHit` on a target its hit reached. Gen 5+ the target takes its own
    /// item (`target.takeItem()`); Gen 3-4 the `TakeItem` event alone decides, and the target is
    /// marked for the battle (`ItemTransferRules::knock_off_marks`) — Gen 3 not on a target the
    /// hit knocked out, Gen 4 not on one whose Quick Claw or Custap Berry went off this turn and
    /// that has not moved yet (the cartridges, issues.md I53).
    pub fn knock_off(&mut self, user: PokemonId, target: PokemonId) {
        let items = self.rules.conditions.items;
        if !items.knock_off_marks {
            let _ = self.take_item(target, target, Some(MoveId::KNOCK_OFF));
            return;
        }
        let Some(pokemon) = self.state.pokemon(target) else {
            return;
        };
        let Some(item) = pokemon.held_item else {
            return;
        };
        if (items.knock_off_spares_fainted && pokemon.is_fainted())
            || self.priority_item_guards(target)
            || !self.take_item_event(target, user, Some(MoveId::KNOCK_OFF), item)
        {
            return;
        }
        self.remove_item(target, item, user, Some(MoveId::KNOCK_OFF), true);
        if let Some(pokemon) = self.state.pokemon_mut(target) {
            pokemon.history.item_knocked_off = true;
        }
    }

    /// Thief / Covet (their `onAfterHit`; Gen 2 Thief's secondary): a user that holds nothing and
    /// used no gem with this move takes the target's item. Gen 4 not when either has Multitype,
    /// nor from a target whose Quick Claw or Custap Berry went off this turn and that has not
    /// moved yet (issues.md I53). Once the item is taken, the move asks its `TakeItem` again with
    /// the roles swapped (`singleEvent('TakeItem', item, …, user, target)`), a refusal putting it
    /// back unseen: a Mega Stone its thief would Mega Evolve with, Booster Energy for a Paradox
    /// thief (R-M10-43; Magician and Pickpocket do not ask). Returns whether it stole.
    pub fn steal_item(
        &mut self,
        user: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        used_gem: bool,
    ) -> bool {
        let items = self.rules.conditions.items;
        let Some(thief) = self.state.pokemon(user) else {
            return false;
        };
        if thief.held_item.is_some() || used_gem {
            return false;
        }
        let multitype = |id: PokemonId| {
            self.state
                .pokemon(id)
                .is_some_and(|p| crate::engine::raw_ability(p) == AbilityId::MULTITYPE)
        };
        if (items.steal_needs_no_multitype && (multitype(user) || multitype(target)))
            || self.priority_item_guards(target)
        {
            return false;
        }
        let Taken::Item(item) = self.may_take_item(target, user, Some(move_id)) else {
            return false;
        };
        if crate::engine::item_refuses_take(
            self.state,
            self.rules,
            item,
            user,
            target,
            Some(move_id),
        ) {
            return false;
        }
        self.remove_item(target, item, user, Some(move_id), true);
        self.give_item(user, item, target, true);
        true
    }

    /// Incinerate's `onHit` on a body it struck: the target's berry — a gem too where
    /// `ItemTransferRules::incinerate_burns_gems` says — is taken by the user and burnt
    /// (`-enditem … [from] move: Incinerate`), never remembered as the target's last item.
    pub fn burn_item(&mut self, user: PokemonId, target: PokemonId) {
        let Some(item) = self.state.pokemon(target).and_then(|p| p.held_item) else {
            return;
        };
        let behavior = crate::engine::item_behavior(self.rules, item);
        if behavior.is_berry
            || (behavior.is_gem && self.rules.conditions.items.incinerate_burns_gems)
        {
            let _ = self.take_item(target, user, Some(MoveId::INCINERATE));
        }
    }

    /// Fling's `onPrepareHit` once it goes ahead: `user` throws `item`, which it loses at the
    /// next `Update` (`addVolatile('fling')`, `fling_lands`).
    pub fn start_fling(&mut self, user: PokemonId, item: ItemId) {
        if let Some(pokemon) = self.state.pokemon_mut(user) {
            pokemon.volatiles.set_fling(item);
            self.trace.push(TraceEvent::VolatileStarted {
                target: user,
                kind: VolatileConditionKind::Fling,
            });
        }
    }

    /// Fling's `condition.onUpdate`: the thrower loses the item (`-enditem … [from] move:
    /// Fling`, remembered as its last item, `AfterUseItem`) and the mark ends.
    pub(super) fn fling_lands(&mut self, user: PokemonId) {
        self.consume_item(user);
        self.end_volatile(user, VolatileConditionKind::Fling);
    }

    /// Fling's `onHit` on a body it struck, for a berry (`move.onHit`, set by its
    /// `onPrepareHit`): the target eats it (`eat_flung_berry`). A non-berry's status or flinch
    /// is a secondary (`engine::prepared_move`).
    pub fn fling_hits(&mut self, user: PokemonId, target: PokemonId) {
        let Some(item) = self
            .state
            .pokemon(user)
            .and_then(|p| p.volatiles.flung_item())
        else {
            return;
        };
        // A Leppa Berry eaten makes the target stale (`foe.staleness = 'external'`).
        if crate::engine::item_behavior(self.rules, item).is_berry
            && self.eat_flung_berry(target, item)
            && item == ItemId::LEPPA_BERRY
        {
            self.make_stale(target, None);
        }
    }

    /// Recycle's `onHit`: a user holding nothing takes back the item it last used up or ate
    /// (`lastItem`, then cleared): `-item … [from] move: Recycle`, the item's `Start` as it is
    /// set. Returns whether it did (the move fails otherwise).
    pub fn recycle(&mut self, user: PokemonId) -> bool {
        let Some(pokemon) = self.state.pokemon_mut(user) else {
            return false;
        };
        if pokemon.held_item.is_some() {
            return false;
        }
        let Some(item) = pokemon.history.last_item.take() else {
            return false;
        };
        self.give_item(user, item, user, true);
        true
    }

    /// Pickup's `onResidual` (Gen 5+): a holder holding nothing takes the item an adjacent
    /// Pokémon used up or ate this turn (`lastItem` while `usedItemThisTurn`; that Pokémon's
    /// `lastItem` is cleared): `-item … [from] ability: Pickup`, the item's `Start` as it is set.
    /// Several such Pokémon, in `getAllActive` order, draw one (`sample`; Showdown draws for a
    /// single one too, which shows nowhere).
    pub fn pickup(&mut self, holder: PokemonId) {
        if self
            .state
            .pokemon(holder)
            .is_none_or(|pokemon| pokemon.held_item.is_some())
        {
            return;
        }
        let mut adjacent = self.adjacent_allies(holder);
        adjacent.extend(self.adjacent_foes(holder));
        let candidates: Vec<PokemonId> = self
            .active_pokemon()
            .into_iter()
            .filter(|pokemon| {
                adjacent.contains(pokemon)
                    && self.state.pokemon(*pokemon).is_some_and(|p| {
                        p.history.last_item.is_some() && p.history.used_item_this_turn
                    })
            })
            .collect();
        let source = match candidates.len() {
            0 => return,
            1 => candidates[0],
            count => candidates[self.sample_index(count)],
        };
        let Some(item) = self
            .state
            .pokemon_mut(source)
            .and_then(|pokemon| pokemon.history.last_item.take())
        else {
            return;
        };
        // The line names the ability (R-M10-41).
        self.ability_activated(holder, AbilityId::PICKUP);
        self.give_item(holder, item, source, true);
    }

    /// Whether `pokemon` holds a berry, set aside or not (Showdown `getItem().isBerry`).
    pub fn holds_berry(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .and_then(|p| p.held_item)
            .is_some_and(|item| crate::engine::item_behavior(self.rules, item).is_berry)
    }

    /// Bug Bite / Pluck's `onHit` on a body it struck: the user takes the target's berry
    /// (`takeItem`: Sticky Hold refuses, `-enditem … [from] stealeat`) and eats it
    /// (`eat_taken_berry`). Gen 4 leaves the berry of a target whose Custap Berry went off this
    /// turn and that has not moved yet (`priority_item_guards`, issues.md I56), and a berry it
    /// takes is the target's last item, which Recycle brings back
    /// (`ItemTransferRules::plucked_berry_recyclable`, I93).
    pub fn steal_and_eat_berry(&mut self, user: PokemonId, target: PokemonId, move_id: MoveId) {
        let Some(item) = self.state.pokemon(target).and_then(|p| p.held_item) else {
            return;
        };
        if !crate::engine::item_behavior(self.rules, item).is_berry
            || self.priority_item_guards(target)
        {
            return;
        }
        if let Some(berry) = self.take_item(target, user, Some(move_id)) {
            // Gen 4: the berry is the target's to Recycle (issues.md I93).
            if self.rules.conditions.items.plucked_berry_recyclable
                && let Some(pokemon) = self.state.pokemon_mut(target)
            {
                pokemon.history.last_item = Some(berry);
            }
            // The user ate the target's Leppa Berry: the target is stale
            // (`target.staleness = 'external'`).
            if self.eat_taken_berry(user, berry) && berry == ItemId::LEPPA_BERRY {
                self.make_stale(target, None);
            }
        }
    }

    /// Magician (`onAfterMoveSecondarySelf`) after `holder`'s damaging move: the Pokémon it hit
    /// (`move.hitTargets`, Substitutes too) fastest first by their handler speed — Pokémon that
    /// share one draw a SpeedTie decision each (Showdown `speedSort`) — and the first whose item
    /// it can take gives it up. Returns whether it took one.
    pub fn magician(&mut self, holder: PokemonId, hit: &[PokemonId], move_id: MoveId) -> bool {
        let mut order: Vec<(PokemonId, u32, u64)> = hit
            .iter()
            .filter(|target| **target != holder)
            .map(|target| (*target, self.state.handler_speed(*target), 0))
            .collect();
        for index in 0..order.len() {
            let speed = order[index].1;
            if order
                .iter()
                .enumerate()
                .any(|(other, entry)| other != index && entry.1 == speed)
            {
                order[index].2 = self
                    .state
                    .rng
                    .next_u64_for(crate::state::RandomRequestKind::SpeedTie);
            }
        }
        order.sort_by_key(|entry| std::cmp::Reverse((entry.1, entry.2)));
        let took = order
            .into_iter()
            .any(|(target, ..)| self.transfer_item(target, holder, Some(move_id)));
        // `-item|holder|item|[from] ability: Magician|[of] target` names it (R-M10-40).
        if took {
            self.ability_activated(holder, AbilityId::MAGICIAN);
        }
        took
    }

    /// Pickpocket (`onAfterMoveSecondary`) on `holder`, struck by `attacker`'s contact move:
    /// it takes the attacker's item, its lines naming it (`-item|holder|item|[from] ability:
    /// Pickpocket|[of] attacker`, R-M10-40).
    pub fn pickpocket(&mut self, holder: PokemonId, attacker: PokemonId, move_id: MoveId) -> bool {
        let took = self.transfer_item(attacker, holder, Some(move_id));
        if took {
            self.ability_activated(holder, AbilityId::PICKPOCKET);
        }
        took
    }

    /// Gen 4 (`ItemTransferRules::priority_item_keeps_item`): `target`'s Quick Claw or Custap
    /// Berry went off as the turn was ordered and it has not moved yet
    /// (`BattleState::priority_item_pending`, issues.md I53).
    fn priority_item_guards(&self, target: PokemonId) -> bool {
        self.rules.conditions.items.priority_item_keeps_item
            && self.state.priority_item_pending(target)
    }

    /// Showdown `runEvent('AfterUseItem')` after the holder used or ate its item: Unburden's
    /// `onAfterUseItem` starts its volatile; a partner's Symbiosis (`onAllyAfterUseItem`) hands
    /// over its own item — Gen 7+ not to a holder that is leaving.
    pub(super) fn after_use_item(&mut self, holder: PokemonId) {
        if let Some(ability) = self.running_ability(holder)
            && crate::engine::ability_behavior(self.rules, ability).unburden
        {
            let _ = self.set_marker_volatile(holder, VolatileConditionKind::Unburden);
        }
        let leaving = self
            .side_of(holder)
            .is_some_and(|side| self.state.side(side).switch_out == Some(holder));
        if leaving
            && self
                .rules
                .conditions
                .items
                .symbiosis_skips_a_leaving_partner
        {
            return;
        }
        for partner in self.adjacent_allies(holder) {
            let symbiosis = self.state.pokemon(partner).is_some_and(|p| {
                crate::engine::ability_handler(self.state, self.rules, p, |b| b.symbiosis)
            });
            if symbiosis && self.give_own_item(partner, holder, None) {
                self.ability_activated(partner, AbilityId::SYMBIOSIS);
                return;
            }
        }
    }

    /// `giver` hands its own item to `receiver` (Bestow, Symbiosis): `giver.takeItem()`, the
    /// item's `TakeItem` again for the receiver, then `receiver.setItem` — a refusal puts the
    /// item back. Returns whether it changed hands.
    fn give_own_item(
        &mut self,
        giver: PokemonId,
        receiver: PokemonId,
        move_id: Option<MoveId>,
    ) -> bool {
        let Some(item) = self.take_item(giver, giver, move_id) else {
            return false;
        };
        // `receiver.setItem` fails for a receiver with no HP left (`!this.hp`): Symbiosis's
        // partner that ate a Jaboca or Rowap Berry as the hit knocked it out.
        if crate::engine::item_refuses_take(self.state, self.rules, item, receiver, giver, move_id)
            || self.is_fainted(receiver)
        {
            if let Some(pokemon) = self.state.pokemon_mut(giver) {
                pokemon.held_item = Some(item);
            }
            return false;
        }
        self.give_item(receiver, item, giver, true);
        true
    }

    /// Bestow's `onHit`: a target holding nothing receives the user's item. Returns whether it
    /// did (the move fails otherwise).
    pub fn bestow(&mut self, user: PokemonId, target: PokemonId) -> bool {
        if self
            .state
            .pokemon(target)
            .is_some_and(|p| p.held_item.is_some())
        {
            return false;
        }
        self.give_own_item(user, target, Some(MoveId::BESTOW))
    }

    /// Trick / Switcheroo's `onHit`: the target's item taken by the user, then the user's own;
    /// a refusal on either side, both hands empty, or either item refusing its receiver (its
    /// `TakeItem` again, the receiver as the holder: a Mega Stone its receiver would Mega Evolve
    /// with, DESIGN.md R-M8-86) puts both back and the move fails.
    /// Otherwise each takes the other's (`setItem`: from Gen 5 a Choice item's `Start` ends the
    /// receiver's lock; Gen 3 ends both battlers', I184). Gen 4 also fails on a target whose Quick Claw or Custap Berry went off this turn
    /// and that has not moved yet (`priority_item_guards`, issues.md I53).
    pub fn swap_items(&mut self, user: PokemonId, target: PokemonId, move_id: MoveId) -> bool {
        // Gen 4: a Knock Off's mark on either side fails the move before any item is asked about
        // (pokeplatinum `BtlCmd_TrySwapItems`, issues.md I94) — no `TakeItem`, so no Unburden.
        let marked = |id: PokemonId| {
            self.state
                .pokemon(id)
                .is_some_and(|p| p.history.item_knocked_off)
        };
        if self.priority_item_guards(target)
            || (self.rules.conditions.items.knocked_off_blocks_taking
                && (marked(user) || marked(target)))
        {
            return false;
        }
        // Showdown's `takeItem` says nothing; the `-item` lines of a swap that goes through show
        // what each holds after (a refused swap gives each its own back, unseen).
        let yours = self.try_take_item(target, user, Some(move_id), false);
        let mine = self.try_take_item(user, user, Some(move_id), false);
        let item = |taken: Taken| match taken {
            Taken::Item(item) => Some(item),
            Taken::Refused | Taken::Nothing => None,
        };
        let (your_item, my_item) = (item(yours), item(mine));
        let refused = yours == Taken::Refused
            || mine == Taken::Refused
            || (your_item.is_none() && my_item.is_none())
            || my_item.is_some_and(|my| {
                crate::engine::item_refuses_take(
                    self.state,
                    self.rules,
                    my,
                    target,
                    user,
                    Some(move_id),
                )
            })
            || your_item.is_some_and(|yours| {
                crate::engine::item_refuses_take(
                    self.state,
                    self.rules,
                    yours,
                    user,
                    target,
                    Some(move_id),
                )
            });
        if refused {
            for (holder, item) in [(target, your_item), (user, my_item)] {
                if let (Some(item), Some(pokemon)) = (item, self.state.pokemon_mut(holder)) {
                    pokemon.held_item = Some(item);
                }
            }
            return false;
        }
        // Each side's `-item` line shows what it got; one given nothing, its `-enditem|holder|
        // item|[silent]|[from] move: Trick` that it holds nothing.
        for (receiver, gets, from, had) in [
            (target, my_item, user, your_item),
            (user, your_item, target, my_item),
        ] {
            match (gets, had) {
                (Some(item), _) => self.give_item(receiver, item, from, true),
                (None, Some(item)) => self.trace.push(TraceEvent::ItemSwappedAway {
                    holder: receiver,
                    item,
                }),
                (None, None) => {}
            }
        }
        // Gen 3: both battlers' `choicedMove` cleared (`ItemTransferRules::trick_frees_choice_locks`,
        // issues.md I184).
        if self.rules.conditions.items.trick_frees_choice_locks {
            self.clear_choice_lock(target);
            self.clear_choice_lock(user);
        }
        true
    }
}
