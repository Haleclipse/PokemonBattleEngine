//! `BattleContext`'s pseudo-weathers (DESIGN.md §23.4): Gravity, Wonder Room, Ion Deluge, Mud
//! Sport, Water Sport, Fairy Lock.

use crate::ids::{MoveId, PokemonId};
use crate::state::{FieldConditionKind, VolatileConditionKind};

use super::{BattleContext, ConditionApplyError};
use crate::engine::TraceEvent;

impl<'a> BattleContext<'a> {
    /// Showdown `addPseudoWeather` for a condition without `onFieldRestart`: false when it is
    /// already up.
    pub fn start_pseudo_weather(&mut self, kind: FieldConditionKind, turns: u8) -> bool {
        if !self.state.field.start_timed(kind, turns) {
            return false;
        }
        self.trace.push(TraceEvent::FieldConditionStarted { kind });
        true
    }

    /// Gravity (`onFieldStart`): every Pokémon in the air mid-Fly or mid-Bounce comes down —
    /// its charge ends and its move this turn, if it has not moved yet, is cancelled (Showdown
    /// `queue.cancelMove`; by id, as Showdown's `removeVolatile('bounce') ||
    /// removeVolatile('fly')`) — and Magnet Rise and Telekinesis end. False when Gravity is
    /// already up.
    pub fn start_gravity(&mut self) -> bool {
        let turns = self.rules.conditions.field.gravity_turns;
        if !self.start_pseudo_weather(FieldConditionKind::Gravity, turns) {
            return false;
        }
        for pokemon in self.active_pokemon() {
            let flying = self.state.pokemon(pokemon).is_some_and(|p| {
                matches!(
                    p.volatiles.charging_move(),
                    Some(MoveId::FLY | MoveId::BOUNCE)
                )
            });
            if flying {
                self.end_volatile(pokemon, VolatileConditionKind::TwoTurnMove);
                self.cancel_move(pokemon);
            }
            // Sky Drop's user brings itself and its catch down (`-end … Sky Drop|[interrupt]`).
            if self
                .state
                .pokemon(pokemon)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::SkyDrop))
            {
                self.cancel_move(pokemon);
                self.end_sky_drop(pokemon);
                self.end_volatile(pokemon, VolatileConditionKind::TwoTurnMove);
            }
            for lift in [
                VolatileConditionKind::MagnetRise,
                VolatileConditionKind::Telekinesis,
            ] {
                self.end_volatile(pokemon, lift);
            }
        }
        true
    }

    /// Showdown `queue.cancelMove`: `pokemon`'s move this turn, if it has not moved yet, does
    /// not happen (the turn loop skips a move whose actor is no longer pending).
    pub fn cancel_move(&mut self, pokemon: PokemonId) {
        let before = self.state.pending_movers.len();
        self.state.pending_movers.retain(|mover| *mover != pokemon);
        if self.state.pending_movers.len() != before {
            self.trace
                .push(TraceEvent::MoveCancelled { actor: pokemon });
        }
    }

    /// Gen 1 Haze on a sleeping or frozen Pokémon (Showdown `lastSelectedMove = 'cannotmove'`,
    /// `queue.changeAction(… 'cannotmove')`): its side's last selected move is cleared, and its
    /// action this turn, if still to come, is lost (only its after-action residual runs).
    pub fn lose_action(&mut self, pokemon: PokemonId) {
        let Some(side) = self.side_of(pokemon) else {
            return;
        };
        self.state.side_mut(side).last_selected_move = Some(crate::state::SelectedMove::CannotMove);
        if self.state.pending_movers.contains(&pokemon) {
            self.state.cannot_move.push(pokemon);
        }
    }

    /// Ion Deluge (the move, or Plasma Fists' hit): Normal moves are Electric for the rest of
    /// the turn (`engine::modified_move`). False when it is already up.
    pub fn start_ion_deluge(&mut self) -> bool {
        let turns = self.rules.conditions.field.ion_deluge_turns;
        self.start_pseudo_weather(FieldConditionKind::IonDeluge, turns)
    }

    /// Wish (Showdown `slotCondition`): pending on the user's slot until the end of the next
    /// turn; fails while one is pending there.
    pub fn make_wish(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(position) = self.state.active_position(user, &self.rules.topology) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        let Some(spec) = self.rules.topology.spec(position) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        let slot = usize::from(spec.slot);
        let amount = self.state.pokemon(user).map_or(0, |p| p.max_hp / 2);
        let side = self.state.side_mut(spec.side);
        if side.wishes.iter().any(|wish| wish.slot == slot) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        side.wishes.push(crate::state::WishState {
            slot,
            source: user,
            amount,
            turns_remaining: 2,
        });
        Ok(())
    }

    /// Healing Wish / Lunar Dance (`onTryHit`: the side must have a Pokémon to switch in): the
    /// heal waits on the user's slot (`SideState::slot_heals`, `SelfFaintRules::slot_heal`);
    /// Gen 4 fills that slot at once (`refill_mid_turn`). The user faints as the move lands
    /// (`selfdestruct: 'ifHit'`, the Shell's).
    pub fn wish_for_the_next(
        &mut self,
        user: PokemonId,
        kind: crate::state::SlotHealKind,
    ) -> Result<(), ConditionApplyError> {
        let Some(position) = self.state.active_position(user, &self.rules.topology) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        let Some(spec) = self.rules.topology.spec(position) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if self.state.healthy_bench(spec.side).is_empty() {
            return Err(ConditionApplyError::NothingToDo);
        }
        let rule = self.rules.conditions.self_faint.slot_heal;
        let turns_remaining = match rule {
            crate::rules::SlotHealRules::Immediate => Some(1),
            crate::rules::SlotHealRules::NextEntrant => Some(2),
            crate::rules::SlotHealRules::WhenNeeded => None,
        };
        let slot = usize::from(spec.slot);
        let side = self.state.side_mut(spec.side);
        // One of its kind already waiting at the slot (Gen 8+: no entrant has needed it yet)
        // stays as it was, and the move adds nothing (Showdown `addSlotCondition`, no
        // `onRestart`): it fails, its user fainting all the same (R-M8-177).
        if side
            .slot_heals
            .iter()
            .any(|heal| heal.slot == slot && heal.kind == kind)
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        side.slot_heals.push(crate::state::SlotHeal {
            slot,
            kind,
            user,
            turns_remaining,
        });
        if rule == crate::rules::SlotHealRules::Immediate {
            side.refill_mid_turn = Some(slot);
        }
        Ok(())
    }

    /// Healing Wish / Lunar Dance on `pokemon`'s slot, as it comes in or (Gen 8+ `onSwap`) is
    /// moved into it: Gen 4-7 heal whoever comes in standing; Gen 8+ only one that needs it (HP,
    /// status; Lunar Dance also PP), the wish waiting otherwise.
    pub fn apply_slot_heals(&mut self, pokemon: PokemonId) {
        let Some(position) = self.state.active_position(pokemon, &self.rules.topology) else {
            return;
        };
        let Some(spec) = self.rules.topology.spec(position) else {
            return;
        };
        let slot = usize::from(spec.slot);
        let rule = self.rules.conditions.self_faint.slot_heal;
        // Every condition of the slot is asked in the order it was laid (Showdown keeps one
        // handler per condition id and each asks `onSwap` for itself): one that needs nothing
        // is kept without stopping the others.
        let mut index = 0;
        while index < self.state.side(spec.side).slot_heals.len() {
            if self.state.side(spec.side).slot_heals[index].slot != slot {
                index += 1;
                continue;
            }
            let kind = self.state.side(spec.side).slot_heals[index].kind;
            let Some(entrant) = self.state.pokemon(pokemon).filter(|p| !p.is_fainted()) else {
                // Gen 4 (the gen4 mod's `onSwitchIn`: `target.switchFlag = true`): a newcomer the
                // hazards knocked out leaves the heal to one more replacement, asked for at once
                // (R-M8-136).
                if rule == crate::rules::SlotHealRules::Immediate {
                    self.state.side_mut(spec.side).refill_mid_turn = Some(slot);
                }
                return;
            };
            let needs = entrant.hp < entrant.max_hp
                || entrant.status.is_some()
                || (kind == crate::state::SlotHealKind::LunarDance
                    && entrant
                        .move_slots()
                        .iter()
                        .any(|slot| slot.pp < slot.max_pp));
            if rule == crate::rules::SlotHealRules::WhenNeeded && !needs {
                index += 1;
                continue;
            }
            let amount = entrant.max_hp - entrant.hp;
            self.state.side_mut(spec.side).slot_heals.remove(index);
            // The condition is gone: the next one now sits at this index (no step).
            // `target.heal(target.maxhp)`: `pokemon.heal`, no `TryHeal` (DESIGN.md R-M8-91).
            if amount > 0 {
                self.apply_heal(crate::engine::HealRequest {
                    target: pokemon,
                    amount,
                    source: crate::engine::EffectSource::Move {
                        pokemon,
                        move_id: match kind {
                            crate::state::SlotHealKind::HealingWish => {
                                crate::ids::MoveId::HEALING_WISH
                            }
                            crate::state::SlotHealKind::LunarDance => {
                                crate::ids::MoveId::LUNAR_DANCE
                            }
                        },
                    },
                    direct: true,
                });
            } else {
                // The wish's `-heal` line at full HP (Lunar Dance giving back PP alone):
                // Showdown shows it, the HP as it was (DESIGN.md R-M8-87).
                let hp = self.state.pokemon(pokemon).map_or(0, |p| p.hp);
                self.trace.push(crate::engine::TraceEvent::HealApplied {
                    target: pokemon,
                    amount: 0,
                    hp_before: hp,
                    hp_after: hp,
                    source: crate::engine::EffectSource::Move {
                        pokemon,
                        move_id: match kind {
                            crate::state::SlotHealKind::HealingWish => {
                                crate::ids::MoveId::HEALING_WISH
                            }
                            crate::state::SlotHealKind::LunarDance => {
                                crate::ids::MoveId::LUNAR_DANCE
                            }
                        },
                    },
                });
            }
            // Gen 4 (the gen4 slot condition's `onSwitchIn`: `target.lastMove = this.lastMove`):
            // the entrant takes the battle's last move as its own — the healer's action, so
            // Metronome when Metronome called the wish (pokeplatinum writes
            // `movePrevByBattler[attacker] = moveTemp`, the action's move, once the entrant
            // stands in the healer's place). Spite, Encore, Disable and Mimic then act on it.
            let passed = self
                .rules
                .conditions
                .self_switch_passes_last_move
                .then_some((self.state.last_move, self.state.last_move_base));
            if let Some(entrant) = self.state.pokemon_mut(pokemon) {
                entrant.status = None;
                if kind == crate::state::SlotHealKind::LunarDance {
                    for slot in entrant.move_slots_mut() {
                        slot.pp = slot.max_pp;
                    }
                }
                if let Some((last_move, last_move_base)) = passed {
                    entrant.last_move = last_move;
                    entrant.last_move_encore = last_move;
                    entrant.last_move_base = last_move_base;
                }
            }
        }
    }

    /// Revival Blessing (`onTryHit`: the side must have a fainted Pokémon): the side is asked
    /// which one comes back (`BattleRequest::Revive`) once the move is over.
    pub fn bless_revival(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(side) = self.side_of(user) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        let any_fainted = self
            .state
            .side(side)
            .team
            .iter()
            .any(|pokemon| self.state.pokemon(*pokemon).is_some_and(|p| p.is_fainted()));
        if !any_fainted {
            return Err(ConditionApplyError::NothingToDo);
        }
        self.state.side_mut(side).reviving = Some(user);
        Ok(())
    }

    /// Wonder Room: started for five turns, or ended when used while up (`onFieldRestart`).
    /// Returns whether it is up afterwards.
    /// Magic Room (`pseudoWeather`, `onFieldRestart` → removed): used while up, it ends. Its
    /// `onFieldStart` sends `End` to every active Pokémon's item, which no implemented item
    /// answers (Utility Umbrella's tells Forecast, which no Gen 8+ Pokémon has).
    pub fn toggle_magic_room(&mut self) -> bool {
        if self.state.field.remove(FieldConditionKind::MagicRoom) {
            self.trace.push(TraceEvent::FieldConditionEnded {
                kind: FieldConditionKind::MagicRoom,
            });
            return false;
        }
        let turns = self.rules.conditions.field.magic_room_turns;
        self.start_pseudo_weather(FieldConditionKind::MagicRoom, turns)
    }

    pub fn toggle_wonder_room(&mut self) -> bool {
        if self.state.field.remove(FieldConditionKind::WonderRoom) {
            self.trace.push(TraceEvent::FieldConditionEnded {
                kind: FieldConditionKind::WonderRoom,
            });
            return false;
        }
        let turns = self.rules.conditions.field.wonder_room_turns;
        self.start_pseudo_weather(FieldConditionKind::WonderRoom, turns)
    }

    /// Mud Sport / Water Sport: Gen 6+ a pseudo-weather for five turns, Gen 3-5 a volatile on
    /// the user (`FieldRules::sports_on_field`); either fails when already there.
    pub fn start_sport(
        &mut self,
        user: PokemonId,
        kind: FieldConditionKind,
    ) -> Result<(), ConditionApplyError> {
        let rules = self.rules.conditions.field;
        if rules.sports_on_field {
            return if self.start_pseudo_weather(kind, rules.sport_turns) {
                Ok(())
            } else {
                Err(ConditionApplyError::AlreadyActive)
            };
        }
        let volatile = match kind {
            FieldConditionKind::MudSport => VolatileConditionKind::MudSport,
            FieldConditionKind::WaterSport => VolatileConditionKind::WaterSport,
            _ => return Err(ConditionApplyError::InvalidTarget),
        };
        let pokemon = self
            .state
            .pokemon_mut(user)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if !pokemon.volatiles.set_user_guard(volatile) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target: user,
            kind: volatile,
        });
        Ok(())
    }
}
