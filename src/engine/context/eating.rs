//! Eating a berry (Showdown `eatItem`, DESIGN.md §40.4) and what can refuse it (`TryEatItem`): a
//! foe's Unnerve, the holder's Berserk / Anger Shell, Heal Block for a healing berry.

use crate::effects::items::{BerryHeal, EatEffect, ItemUpdate};
use crate::engine::{StatChangeCause, StatChangeRequest};
use crate::ids::{AbilityId, ItemId, PokemonId};
use crate::rules::{HpShare, StatusBerryTiming};
use crate::state::{MoveSlotState, RandomRequestKind, Stat, VolatileConditionKind};

use super::{BattleContext, CureReason, EffectSource, HealRequest, TraceEvent};

impl<'a> BattleContext<'a> {
    /// Showdown `eatItem()`: the holder, standing and on the field, eats its held berry unless a
    /// foe's Unnerve refuses it (`TryEatItem`). The berry's `onEat` runs while it is still held
    /// (`ItemBehavior::eat_effect`); then it is gone, remembered as the holder's last item, and
    /// the holder has used an item this turn and eaten a berry (§39.1). Returns the berry eaten.
    /// `EatItem` follows the effect (Cheek Pouch, Ripen's note, Cud Chew); `AfterUseItem` after
    /// the bookkeeping (Unburden, Symbiosis).
    pub fn eat_item(&mut self, holder: PokemonId) -> Option<ItemId> {
        self.eat_held_item(holder, false)
    }

    /// Showdown `eatItem(true)` (Stuff Cheeks, Teatime): nothing that `TryEatItem` asks — a
    /// foe's Unnerve, Berserk's wait, Heal Block on a healing berry, nor Ripen's announcement,
    /// which is its `onTryEatItem` — stands in the way.
    pub fn force_eat_item(&mut self, holder: PokemonId) -> Option<ItemId> {
        self.eat_held_item(holder, true)
    }

    fn eat_held_item(&mut self, holder: PokemonId, force: bool) -> Option<ItemId> {
        let pokemon = self.state.pokemon(holder)?;
        let item = pokemon.held_item?;
        // Showdown `eatItem`: Jaboca and Rowap Berry go even off a holder the hit knocked out.
        let at_no_hp = crate::engine::item_behavior(self.rules, item)
            .recoil_berry
            .is_some();
        if (pokemon.hp == 0 && !at_no_hp) || !self.is_active(holder) {
            return None;
        }
        if !force {
            if self.eating_refused(holder, item) {
                return None;
            }
            // Ripen's `onTryEatItem` (priority −1: once nothing refused the berry) announces it.
            if let Some(ability) = self.running_ability(holder)
                && crate::engine::ability_behavior(self.rules, ability).ripen
            {
                self.ability_activated(holder, ability);
            }
        }
        self.trace.push(TraceEvent::ItemEaten { holder, item });
        if !self.sets_items_aside(holder, false) {
            self.eat_effect(holder, item, None);
        }
        // `EatItem`, the berry still held; a restorative berry settles the holder's pending
        // staleness (DESIGN.md §45.7).
        self.ate_item(holder, item, false);
        self.eat_settles_staleness(holder, item);
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.held_item = None;
            pokemon.item_state = crate::state::ItemState::default();
            pokemon.history.last_item = Some(item);
            pokemon.history.used_item_this_turn = true;
            pokemon.history.ate_berry = true;
        }
        self.after_use_item(holder);
        Some(item)
    }

    /// Bug Bite / Pluck's `onHit`, the target's berry taken (`takeItem`): `eater` eats it
    /// (`eat_from_another`). Gen 4 a confusion berry does nothing to an eater at full HP
    /// (`BerryRules::taken_confusion_berry_needs_damage`, issues.md I57), nor any berry to an
    /// eater with Klutz, whatever it holds (I59). Returns whether the eater ate it.
    pub fn eat_taken_berry(&mut self, eater: PokemonId, item: ItemId) -> bool {
        let unharmed = self
            .state
            .pokemon(eater)
            .is_some_and(|pokemon| pokemon.hp == pokemon.max_hp);
        let berries = &self.rules.conditions.berries;
        let spoilt = berries.taken_confusion_berry_needs_damage
            && matches!(
                crate::engine::item_behavior(self.rules, item).eat_effect,
                Some(EatEffect::ConfusionBerryHeal(_))
            )
            && unharmed;
        let effect = !spoilt && !self.sets_items_aside(eater, berries.klutz_spoils_taken_berries);
        self.eat_from_another(eater, item, effect, true, None)
    }

    /// Fling's `onHit` for a berry `thrower` flung: the target eats it (`eat_from_another`).
    /// Gen 4 only Embargo keeps its effect away — Klutz does not
    /// (`ItemTransferRules::flung_berry_ignores_klutz`, issues.md I91). Returns whether the
    /// target ate it.
    pub(super) fn eat_flung_berry(
        &mut self,
        thrower: PokemonId,
        target: PokemonId,
        item: ItemId,
    ) -> bool {
        let Some(pokemon) = self.state.pokemon(target) else {
            return false;
        };
        let set_aside = if self.rules.conditions.items.flung_berry_ignores_klutz {
            pokemon.volatiles.contains(VolatileConditionKind::Embargo)
        } else {
            self.sets_items_aside(target, false)
        };
        self.eat_from_another(target, item, !set_aside, false, Some(thrower))
    }

    /// A berry from another's hand (Bug Bite, Pluck, Fling): its `onEat` on `eater`
    /// (`singleEvent('Eat')`) where `effect` says, then `EatItem` — `taken` by Bug Bite or Pluck,
    /// which Cud Chew passes over, not thrown — and the eater has eaten a berry; nothing asks
    /// `TryEatItem`, and nobody remembers it as a last item. A berry whose `onEat` is `false`
    /// (`EatEffect::Inert`) stops it all. Returns whether the eater ate it (the `Eat` event's
    /// result). `thrower`: who flung it (Fling), the `Eat` event's source.
    fn eat_from_another(
        &mut self,
        eater: PokemonId,
        item: ItemId,
        effect: bool,
        taken: bool,
        thrower: Option<PokemonId>,
    ) -> bool {
        if crate::engine::item_behavior(self.rules, item).eat_effect == Some(EatEffect::Inert) {
            return false;
        }
        if effect {
            self.eat_effect(eater, item, thrower);
        }
        self.ate_item(eater, item, taken);
        if let Some(pokemon) = self.state.pokemon_mut(eater) {
            pokemon.history.ate_berry = true;
        }
        true
    }

    /// Whether `eater` gets nothing from a berry it eats (Showdown's `singleEvent('Eat')` skips
    /// the berry while `ignoringItem()`): Klutz, Embargo, Magic Room — Klutz unless what the
    /// eater holds is an item Klutz leaves alone (`engine::ignores_items`), or whatever it holds
    /// where `klutz_regardless` (Gen 4 Bug Bite and Pluck: `BerryRules::klutz_spoils_taken_berries`,
    /// issues.md I59).
    fn sets_items_aside(&self, eater: PokemonId, klutz_regardless: bool) -> bool {
        self.state.pokemon(eater).is_none_or(|pokemon| {
            if klutz_regardless {
                crate::engine::sets_any_item_aside(self.state, self.rules, pokemon)
            } else {
                crate::engine::ignores_items(self.state, self.rules, pokemon)
            }
        })
    }

    /// The berry's `onEat` on `eater` (Showdown `singleEvent('Eat')`); `thrower` flung it.
    fn eat_effect(&mut self, eater: PokemonId, item: ItemId, thrower: Option<PokemonId>) {
        match crate::engine::item_behavior(self.rules, item).eat_effect {
            Some(EatEffect::CureStatus(statuses)) => {
                if self.state.pokemon(eater).is_some_and(|pokemon| {
                    statuses.iter().any(|status| pokemon.has_status(*status))
                }) {
                    self.cure_status(eater, CureReason::Item(item));
                }
            }
            Some(EatEffect::CureConfusion) => {
                self.end_volatile(eater, VolatileConditionKind::Confusion);
            }
            Some(EatEffect::CureStatusAndConfusion) => {
                self.cure_status(eater, CureReason::Item(item));
                self.end_volatile(eater, VolatileConditionKind::Confusion);
            }
            Some(EatEffect::Heal(amount)) => {
                self.berry_heal(eater, item, amount);
            }
            Some(EatEffect::SitrusHeal) => {
                let amount = self.rules.conditions.berries.sitrus_heal;
                self.berry_heal(eater, item, amount);
            }
            Some(EatEffect::ConfusionBerryHeal(disliked)) => {
                let share = self.rules.conditions.berries.confusion_berry_heal;
                self.berry_heal(eater, item, BerryHeal::MaxHp(share));
                if self
                    .state
                    .pokemon(eater)
                    .is_some_and(|pokemon| pokemon.nature.minus() == Some(disliked))
                {
                    // The eater confuses itself; Safeguard stands in the way of a berry flung at
                    // it (the flinger is the event's source) and, in Gen 4, of any berry
                    // (`BerryRules::safeguard_stops_own_berry_confusion`, issues.md I214).
                    let culprit = if thrower.is_some()
                        || self
                            .rules
                            .conditions
                            .berries
                            .safeguard_stops_own_berry_confusion
                    {
                        crate::engine::protections::Culprit::Another {
                            infiltrating: false,
                        }
                    } else {
                        crate::engine::protections::Culprit::None
                    };
                    let _ = self.confuse_berry_eater(eater, culprit);
                }
            }
            Some(EatEffect::Boost(stat)) => {
                self.berry_boost(eater, item, stat, 1);
            }
            Some(EatEffect::FocusEnergy) => {
                let _ = self.set_focus_energy(eater);
            }
            Some(EatEffect::RandomBoost) => {
                if let Some(stat) = self.random_raisable_stat(eater) {
                    self.berry_boost(eater, item, stat, 2);
                }
            }
            Some(EatEffect::NextMoveAccuracy) => {
                let turns = self.rules.conditions.berries.micle.boost_turns;
                if self
                    .state
                    .pokemon_mut(eater)
                    .is_some_and(|pokemon| pokemon.volatiles.set_micle_berry(turns))
                {
                    self.trace.push(TraceEvent::VolatileStarted {
                        target: eater,
                        kind: VolatileConditionKind::MicleBerry,
                    });
                }
            }
            Some(EatEffect::RestorePp(amount)) => self.berry_restore_pp(eater, item, amount),
            Some(EatEffect::Nothing | EatEffect::Inert) | None => {}
        }
    }

    /// `runEvent('EatItem')` on the eater, the berry eaten: Cheek Pouch heals; Ripen notes
    /// whether it was a resist berry; Cud Chew remembers it, unless it was `taken` from another's
    /// hand (Bug Bite, Pluck: its `onEatItem` skips them).
    fn ate_item(&mut self, eater: PokemonId, item: ItemId, taken: bool) {
        if let Some(ability) = self.running_ability(eater) {
            let behavior = crate::engine::ability_behavior(self.rules, ability);
            crate::effects::abilities::ate_item(self, behavior, eater, item, taken);
        }
    }

    /// Cud Chew's `onEatItem`: the berry is remembered for two turn ends — one when the turn's
    /// queue is empty (Showdown `!queue.peek()`, `BattleState::turn_actions_done`: eaten in
    /// the residual or by an end-of-turn replacement). A berry eaten at the residual's closing
    /// `Update` therefore comes back at the next turn's end, as one eaten during the turn does.
    pub fn remember_for_cud_chew(&mut self, holder: PokemonId, item: ItemId) {
        let turns = if self.state.turn_actions_done { 1 } else { 2 };
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.cud_chew = Some((item, turns));
        }
    }

    /// Cud Chew's `onResidual`: the remembered berry counts one turn end down; at the last the
    /// holder eats it again (`-activate`, `-enditem … [eat]`) — its `onEat` alone (Showdown
    /// `singleEvent('Eat')`: no `TryEatItem`, so a foe's Unnerve does not stop it; Embargo and
    /// Magic Room set the berry aside) — has eaten
    /// a berry (`ateBerry`) and forgets it. Its `EatItem` reaches only Cud Chew, which
    /// remembers the berry to forget it at once.
    pub fn chew_the_cud(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon_mut(holder) else {
            return;
        };
        let Some((item, turns)) = pokemon.ability_state.cud_chew else {
            return;
        };
        if pokemon.hp == 0 {
            return;
        }
        if turns > 1 {
            pokemon.ability_state.cud_chew = Some((item, turns - 1));
            return;
        }
        pokemon.ability_state.cud_chew = None;
        self.ability_activated(holder, AbilityId::CUD_CHEW);
        self.trace.push(TraceEvent::ItemEaten { holder, item });
        if !self.sets_items_aside(holder, false) {
            self.eat_effect(holder, item, None);
        }
        if crate::engine::item_behavior(self.rules, item)
            .eat_effect
            .is_some()
            && let Some(pokemon) = self.state.pokemon_mut(holder)
        {
            pokemon.history.ate_berry = true;
        }
    }

    /// Harvest's `setItem(lastItem)`: a standing holder with no item gets back the item it last
    /// ate or used up if that is a berry (Showdown `isBerry`, from the dex) — `-item … [from]
    /// ability: Harvest` — and no longer has it as its last item.
    pub fn harvest(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let Some(item) = pokemon.history.last_item else {
            return;
        };
        if pokemon.hp == 0
            || pokemon.held_item.is_some()
            || !self
                .dex
                .and_then(|dex| dex.item_by_id(item))
                .is_some_and(|resolved| resolved.is_berry)
        {
            return;
        }
        // `setItem`: a fresh `itemState`, `pendingStaleness` (DESIGN.md §45.7).
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.held_item = Some(item);
            pokemon.item_state = crate::state::ItemState::default();
            pokemon.pending_staleness = true;
            pokemon.history.last_item = None;
        }
        self.trace.push(TraceEvent::ItemRestored {
            holder,
            item,
            source: EffectSource::Ability {
                pokemon: holder,
                ability: AbilityId::HARVEST,
            },
        });
    }

    /// Showdown `runEvent('TryEatItem')`: the holder's Berserk / Anger Shell refuses a healing
    /// berry while a move's hit keeps it waiting (`onTryEatItem`); a healing berry's own
    /// `onTryEatItem` refuses it while the heal would be refused (`TryHeal`: Heal Block); an
    /// opposing Pokémon on the field whose Unnerve (or As One) has started (`onFoeTryEatItem`:
    /// `!effectState.unnerved`) refuses any.
    fn eating_refused(&self, holder: PokemonId, item: ItemId) -> bool {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return false;
        };
        let behavior = crate::engine::item_behavior(self.rules, item);
        let heals = matches!(
            behavior.eat_effect,
            Some(EatEffect::Heal(_) | EatEffect::SitrusHeal | EatEffect::ConfusionBerryHeal(_))
        ) || behavior.super_effective_heal.is_some();
        if heals
            && pokemon.ability_state.hp_berries_wait
            && crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.half_hp_boosts)
                .is_some()
        {
            return true;
        }
        if heals
            && self.heal_refused(
                holder,
                EffectSource::Item {
                    pokemon: holder,
                    item,
                },
            )
        {
            return true;
        }
        unnerved_by_a_foe(self.state, self.rules, holder)
    }

    /// A berry's `this.boost(...)` on its holder, the berry as the effect.
    fn berry_boost(&mut self, holder: PokemonId, item: ItemId, stat: Stat, stages: i8) {
        self.try_change_stat_stage(StatChangeRequest {
            target: holder,
            stat,
            stages,
            source: EffectSource::Item {
                pokemon: holder,
                item,
            },
            cause: StatChangeCause::Item,
        });
    }

    /// Starf Berry's `this.sample(...)` over the holder's Attack, Defense, Special Attack,
    /// Special Defense and Speed below +6 (Showdown's `boosts` order). The draw names the stat
    /// by its place in `engine::STAGED_STATS`; none when all five are at +6.
    fn random_raisable_stat(&mut self, holder: PokemonId) -> Option<Stat> {
        let pokemon = self.state.pokemon(holder)?;
        let codes: Vec<u16> = (0u16..)
            .zip(super::STAGED_STATS)
            .take(5)
            .filter(|(_, stat)| pokemon.stages.get(*stat) < 6)
            .map(|(code, _)| code)
            .collect();
        if codes.is_empty() {
            return None;
        }
        let weights = vec![1; codes.len()];
        let code = self
            .state
            .rng
            .sample_weighted_for(RandomRequestKind::Sample, &codes, &weights);
        super::STAGED_STATS.get(usize::from(code)).copied()
    }

    /// A heal a berry's own handler gives once it is eaten (Enigma Berry's `this.heal`).
    pub fn heal_from_berry(&mut self, holder: PokemonId, item: ItemId, amount: BerryHeal) {
        self.berry_heal(holder, item, amount);
    }

    /// A berry's `heal(n)` (Showdown: truncated, at least 1; then `TryHeal`, where Ripen
    /// doubles a berry's heal).
    fn berry_heal(&mut self, holder: PokemonId, item: ItemId, amount: BerryHeal) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let mut amount = amount.of(pokemon.base_max_hp());
        if crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.ripen) {
            amount = amount.saturating_mul(2);
        }
        self.apply_heal(HealRequest {
            target: holder,
            amount,
            source: EffectSource::Item {
                pokemon: holder,
                item,
            },
            direct: false,
        });
    }

    /// Whether the holder's HP is down to `share` for a berry (the generation's half, Gluttony).
    pub fn berry_hp_reached(&self, holder: PokemonId, share: HpShare) -> bool {
        self.state.pokemon(holder).is_some_and(|pokemon| {
            let gluttony =
                crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.gluttony);
            self.rules
                .conditions
                .berries
                .reached(share, pokemon.hp, pokemon.max_hp, gluttony)
        })
    }

    /// Whether the holder's berry has room for its effect where the generation asks
    /// (`BerryRules::stat_berries_need_room`, Gen 3-4, issues.md I37): a stat berry is not
    /// eaten, and stays held, while it would change nothing.
    pub fn berry_has_room(&self, holder: PokemonId) -> bool {
        !self.rules.conditions.berries.stat_berries_need_room
            || self.state.pokemon(holder).is_none_or(|pokemon| {
                pokemon
                    .held_item
                    .and_then(|item| crate::engine::item_behavior(self.rules, item).eat_effect)
                    .is_none_or(|effect| effect.has_room(pokemon))
            })
    }

    /// Berry Juice: at `share` of its HP, unless the heal would be refused (`TryHeal`), the
    /// holder uses it up (`useItem()`: not eaten) and heals `amount`.
    pub fn use_to_heal(&mut self, holder: PokemonId, share: HpShare, amount: BerryHeal) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let Some(item) = pokemon.held_item else {
            return;
        };
        let amount = amount.of(pokemon.base_max_hp());
        let source = EffectSource::Item {
            pokemon: holder,
            item,
        };
        if !self.berry_hp_reached(holder, share) || self.heal_refused(holder, source) {
            return;
        }
        // Its own `runEvent('TryHeal')` before `useItem`, then `heal`'s.
        self.ripen_try_heal(holder, source);
        if self.consume_item(holder).is_some() {
            self.apply_heal(HealRequest {
                target: holder,
                amount,
                source,
                direct: false,
            });
        }
    }

    /// A status berry (an item whose `Update` trigger is a status or confusion) eaten if its
    /// trigger holds — where the generation feeds status berries outside an `Update`
    /// (`BerryRules::status_berries`).
    pub fn feed_status_berry(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let Some((_, item)) = crate::engine::held_item(self.state, self.rules, holder) else {
            return;
        };
        let berries = &self.rules.conditions.berries;
        if let Some(ItemUpdate::Eat(trigger)) = item.update
            && trigger.cures()
            && trigger.holds(pokemon, berries, false)
        {
            self.eat_item(holder);
        }
    }

    /// The end of the turn's check of a berry the generation feeds there rather than at an
    /// `Update` (`EatTrigger::at_turn_end`): eaten if its trigger holds.
    pub fn feed_turn_end_berry(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let Some((_, item)) = crate::engine::held_item(self.state, self.rules, holder) else {
            return;
        };
        let berries = &self.rules.conditions.berries;
        if let Some(ItemUpdate::Eat(trigger)) = item.update
            && trigger.at_turn_end(berries)
            && trigger.holds(pokemon, berries, false)
        {
            self.eat_item(holder);
        }
    }

    /// Leppa Berry, Mystery Berry (`onEat`): the first move with no PP left — none (a forced eat):
    /// the first below its max — gets `amount` back (Ripen: twice), up to its max.
    fn berry_restore_pp(&mut self, holder: PokemonId, item: ItemId, amount: u8) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let ripen = crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.ripen);
        // Gen 3: the own slots (`BerryRules::pp_berry_reads_own_slots`, issues.md I185).
        let own = self.rules.conditions.berries.pp_berry_reads_own_slots;
        let slots: Vec<MoveSlotState> = if own {
            pokemon.base_slots().copied().collect()
        } else {
            pokemon.move_slots().to_vec()
        };
        let Some(index) = slots
            .iter()
            .position(|slot| slot.pp == 0)
            .or_else(|| slots.iter().position(|slot| slot.pp < slot.max_pp))
        else {
            return;
        };
        let amount = if ripen {
            amount.saturating_mul(2)
        } else {
            amount
        };
        let Some(slot) = self.state.pokemon_mut(holder).and_then(|pokemon| {
            if own {
                pokemon.base_slot_mut(index)
            } else {
                pokemon.move_slots_mut().get_mut(index)
            }
        }) else {
            return;
        };
        slot.pp = slot.pp.saturating_add(amount).min(slot.max_pp);
        let (move_id, remaining) = (slot.move_id, slot.pp);
        self.trace.push(TraceEvent::PpRestored {
            pokemon: holder,
            move_id,
            remaining,
            source: EffectSource::Item {
                pokemon: holder,
                item,
            },
        });
    }

    /// Mental Herb's `onUpdate`: while the holder has one of the conditions it cures (Gen 3-4:
    /// an infatuation; Gen 5+: also Taunt, Encore, Torment, Disable, Heal Block), it uses the
    /// herb up and every one of them ends — the infatuation with `-end … [from] item: Mental
    /// Herb`. Torment has no volatile yet: it joins the list with its move.
    pub fn use_mental_herb(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        if pokemon.hp == 0
            || !self
                .mental_herb_cures()
                .iter()
                .any(|kind| pokemon.volatiles.contains(*kind))
        {
            return;
        }
        if self.consume_item(holder).is_none() {
            return;
        }
        self.cure_mental_herb_volatiles(holder);
    }

    /// What Mental Herb cures: Gen 4 infatuation alone, later the six (Showdown's
    /// `conditions` list).
    fn mental_herb_cures(&self) -> &'static [VolatileConditionKind] {
        if self.rules.conditions.mental_herb_cures_only_infatuation {
            &[VolatileConditionKind::Attract]
        } else {
            &[
                VolatileConditionKind::Attract,
                VolatileConditionKind::Taunt,
                VolatileConditionKind::Encore,
                VolatileConditionKind::Torment,
                VolatileConditionKind::Disable,
                VolatileConditionKind::HealBlock,
            ]
        }
    }

    /// White Herb's restoration, held or flung (`fling.effect`, DESIGN.md R-M8-86): every
    /// stat stage of `pokemon` below 0 goes back to 0 (`setBoost`, no stat change event).
    pub fn clear_lowered_stages(
        &mut self,
        pokemon: PokemonId,
        source: crate::engine::EffectSource,
    ) {
        for stat in crate::engine::STAGED_STATS {
            if self.stage(pokemon, stat) < 0 {
                self.set_stage(pokemon, stat, 0, source);
            }
        }
    }

    /// Mental Herb's cure, held or flung (`fling.effect`, DESIGN.md R-M8-86): when `pokemon` has
    /// one of the conditions, every one of them ends.
    pub fn cure_mental_herb_volatiles(&mut self, pokemon: PokemonId) {
        let cures = self.mental_herb_cures();
        if !self
            .state
            .pokemon(pokemon)
            .is_some_and(|p| cures.iter().any(|kind| p.volatiles.contains(*kind)))
        {
            return;
        }
        for kind in cures {
            self.end_volatile(pokemon, *kind);
        }
    }

    /// Gen 2 (issues.md I33, `StatusBerryTiming::InflictionOrTurnEnd`): another Pokémon's move
    /// gave the holder a status or confusion — pokecrystal's effect commands feed it its status
    /// berry at once (`UseHeldStatusHealingItem`, `UseConfusionHealingItem`); its own Rest does
    /// not.
    pub fn status_inflicted(&mut self, holder: PokemonId, by: Option<PokemonId>) {
        if self.rules.conditions.berries.status_berries == StatusBerryTiming::InflictionOrTurnEnd
            && by.is_some_and(|by| by != holder)
        {
            self.feed_status_berry(holder);
        }
    }

    /// Gen 3 (issues.md I34, `StatusBerryTiming::UpdateAndBeforeReplacement`): an `Update` a
    /// replacement request cut short still feeds the status berries, fastest first — pokeemerald
    /// checks them at every move's end, at the end of the turn and after the replacements, all
    /// before the request.
    pub(crate) fn feed_status_berries_before_request(&mut self) {
        if self.rules.conditions.berries.status_berries
            != StatusBerryTiming::UpdateAndBeforeReplacement
        {
            return;
        }
        let mut order: Vec<(PokemonId, u32)> = self
            .state
            .sides
            .iter()
            .flat_map(|side| side.active.iter().flatten().copied())
            .filter(|id| self.state.pokemon(*id).is_some_and(|p| !p.is_fainted()))
            .map(|id| (id, crate::engine::stats::handler_speed(self.state, id)))
            .collect();
        order.sort_by_key(|(_, speed)| std::cmp::Reverse(*speed));
        for (holder, _) in order {
            self.feed_status_berry(holder);
        }
    }

    /// Berserk / Anger Shell: whether the holder's healing berries wait for the move's
    /// `AfterMoveSecondary` (`AbilityState::hp_berries_wait`).
    pub fn set_hp_berries_wait(&mut self, holder: PokemonId, wait: bool) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.hp_berries_wait = wait;
        }
    }

    /// Ripen's `onEatItem`: whether the berry the holder just ate was a resist berry
    /// (`abilityState.berryWeaken`). While it was, the next hit the holder takes is halved again
    /// (its `onSourceModifyDamage`) — at once when the berry was eaten for that hit, later when
    /// it was eaten otherwise (Stuff Cheeks, Teatime, Bug Bite).
    pub fn note_berry_weaken(&mut self, holder: PokemonId, item: ItemId) {
        let weakening = crate::engine::item_behavior(self.rules, item)
            .weakening_berry
            .is_some();
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.berry_weaken = weakening;
        }
    }

    /// Unnerve's `onStart`: the first start since the holder came in (`effectState.unnerved`
    /// was unset) — the one that announces it.
    pub fn start_unnerve(&mut self, holder: PokemonId) -> bool {
        self.state
            .pokemon_mut(holder)
            .is_some_and(|pokemon| !std::mem::replace(&mut pokemon.ability_state.unnerved, true))
    }
}

/// An opposing Pokémon on the field whose Unnerve (or As One) has started refuses `holder` any
/// berry (`onFoeTryEatItem`: `!effectState.unnerved`).
pub(crate) fn unnerved_by_a_foe(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    holder: PokemonId,
) -> bool {
    let Some(side) = state.pokemon(holder).map(|pokemon| pokemon.side) else {
        return false;
    };
    state
        .side(side.other())
        .active
        .iter()
        .flatten()
        .filter_map(|foe| state.pokemon(*foe))
        .any(|foe| {
            !foe.is_fainted()
                && foe.ability_state.unnerved
                && crate::engine::ability_handler(state, rules, foe, |b| b.unnerve)
        })
}
