//! Showdown's `Update` (`eachEvent('Update')`, DESIGN.md §40): every Pokémon on the field,
//! fastest first by handler speed, runs its `onUpdate` participants — its volatiles', then its
//! ability's (Trace), then its item's (Showdown's `subOrder` by effect type). The generation's
//! `UpdateRules` decide where the engine calls it; this is the one dispatcher every call site
//! shares.

use crate::effects::abilities::AbilityUpdate;
use crate::effects::items::{EatEffect, ItemUpdate};
use crate::ids::PokemonId;

use super::BattleContext;
use crate::rules::HpShare;

/// Whether an `Update` comes before or after Showdown processes the faints of the step it
/// follows (`faintMessages`): a Pokémon at 0 HP whose faint is still pending is on the field
/// (`isActive`), one whose faint was processed is not. The engine processes faints once the
/// action is over (`resolve_faints`, which empties the slot), so inside an action the caller
/// says which Showdown has done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingFaints {
    /// The step's faints are still pending (after a hit, after the weather's chip).
    Pending,
    /// Showdown has processed them (after an action).
    Processed,
    /// Showdown has processed the faints a move's hit loop reached (`announce_knocked_out`); one
    /// that came after it — the user's recoil — is still pending (R-M8-166).
    Announced,
}

impl<'a> BattleContext<'a> {
    /// Showdown `eachEvent('Update')`.
    pub(crate) fn run_update(&mut self, faints: PendingFaints) {
        let on_field = self.update_field(faints);
        // Only a Pokémon with HP left runs its handlers (`getAllActive`), fastest first;
        // Showdown shuffles a tie; no participant so far changes what another one does (a
        // volatile ends silently, a berry cures its own holder), so the order shows in no frame
        // and a tie keeps position order, drawing nothing.
        let mut order: Vec<(PokemonId, u32)> = on_field
            .iter()
            .filter(|id| self.state.pokemon(**id).is_some_and(|p| !p.is_fainted()))
            .map(|id| (*id, crate::engine::stats::handler_speed(self.state, *id)))
            .collect();
        order.sort_by_key(|(_, speed)| std::cmp::Reverse(*speed));
        for (pokemon, _) in order {
            self.update_volatiles(pokemon, &on_field);
            self.update_ability(pokemon);
            self.update_item(pokemon);
        }
    }

    /// The Pokémon on the field for an `Update` (`isActive`): in an active slot and, once
    /// Showdown has processed the step's faints, with HP left.
    fn update_field(&self, faints: PendingFaints) -> Vec<PokemonId> {
        self.state
            .sides
            .iter()
            .flat_map(|side| side.active.iter().flatten().copied())
            .filter(|id| match faints {
                PendingFaints::Pending => true,
                PendingFaints::Processed => {
                    self.state.pokemon(*id).is_some_and(|p| !p.is_fainted())
                }
                PendingFaints::Announced => {
                    self.state.pokemon(*id).is_some_and(|p| !p.is_fainted())
                        || !self.state.announced_faints.contains(id)
                }
            })
            .collect()
    }

    /// The holder's ability's `onUpdate`: Trace looks again while it found nothing to copy;
    /// Immunity, Insomnia, Limber … cure the holder of a status they keep off, Own Tempo and
    /// Oblivious end their volatiles (`AbilityUpdate`, DESIGN.md R270).
    fn update_ability(&mut self, holder: PokemonId) {
        // Whether the ability has an `onUpdate` comes first, whether it runs only then (R-M10-10).
        let updates = self.state.pokemon(holder).is_some_and(|pokemon| {
            let behavior =
                crate::engine::ability_behavior(self.rules, crate::engine::raw_ability(pokemon));
            behavior.trace || behavior.update.is_some()
        });
        if !updates {
            return;
        }
        let Some(ability) = self.running_ability(holder) else {
            return;
        };
        let behavior = crate::engine::ability_behavior(self.rules, ability);
        if behavior.trace {
            self.trace_ability(holder);
        }
        match behavior.update {
            // Disguise / Ice Face: the broken forme is taken (and Disguise's cost paid).
            Some(AbilityUpdate::BustedForme(absorbing)) => absorbing.update(self, holder),
            Some(AbilityUpdate::Commander) => {
                crate::effects::abilities::forme_ability::commander(self, holder)
            }
            Some(AbilityUpdate::CureStatus(statuses)) => {
                if self
                    .status_of(holder)
                    .is_some_and(|status| statuses.contains(&status))
                {
                    self.ability_activated(holder, ability);
                    self.cure_status(holder, crate::engine::CureReason::Ability(ability));
                }
            }
            Some(AbilityUpdate::EndVolatiles(kinds)) => {
                for &kind in kinds {
                    if kind == crate::state::VolatileConditionKind::Taunt
                        && !self.rules.conditions.oblivious_covers_taunt
                    {
                        continue;
                    }
                    if self
                        .state
                        .pokemon(holder)
                        .is_some_and(|p| p.volatiles.contains(kind))
                    {
                        self.ability_activated(holder, ability);
                        self.end_volatile(holder, kind);
                    }
                }
            }
            None => {}
        }
    }

    /// The holder's volatiles' `onUpdate`: an infatuation (`attract`) or a Syrup Bomb whose
    /// source is no longer on the field ends (`!effectState.source.isActive`); a Fling's thrower
    /// loses its item.
    fn update_volatiles(&mut self, holder: PokemonId, on_field: &[PokemonId]) {
        let stale: Vec<_> = self
            .state
            .pokemon(holder)
            .map(|p| p.volatiles.bound())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, source)| !on_field.contains(source))
            .map(|(kind, _)| kind)
            .collect();
        for kind in stale {
            self.end_volatile(holder, kind);
        }
        // Fling's `condition.onUpdate`: the item thrown goes.
        if self.state.pokemon(holder).is_some_and(|p| {
            p.volatiles
                .contains(crate::state::VolatileConditionKind::Fling)
        }) {
            self.fling_lands(holder);
        }
    }

    /// The holder's item's `onUpdate`: a berry whose trigger holds is eaten (Showdown
    /// `eatItem()`; a foe's Unnerve can still refuse it), Berry Juice and Mental Herb used up. Gen 2's status
    /// berries are the cartridge's instead (`StatusBerryTiming::InflictionOrTurnEnd`).
    fn update_item(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let Some((_, behavior)) = crate::engine::held_item(self.state, self.rules, holder) else {
            return;
        };
        let gluttony = crate::engine::gluttony(self.state, self.rules, pokemon);
        let berries = &self.rules.conditions.berries;
        match behavior.update {
            Some(ItemUpdate::Eat(trigger))
                if !trigger.at_turn_end(berries)
                    && trigger.holds(pokemon, berries, gluttony)
                    && self.berry_has_room(holder) =>
            {
                self.eat_item(holder);
            }
            Some(ItemUpdate::UseToHeal { at, amount }) => self.use_to_heal(holder, at, amount),
            Some(ItemUpdate::CureVolatiles) => self.use_mental_herb(holder),
            Some(ItemUpdate::BoosterEnergy) => self.use_booster_energy(holder),
            Some(ItemUpdate::BerserkGene) => self.use_berserk_gene(holder),
            // Gen 4's Micle Berry is eaten like the other pinch berries (issues.md I42), though
            // its item's handler is an `onResidual` (`BerryRules::micle`).
            None if berries.micle.eaten_at_update
                && behavior.eat_effect == Some(EatEffect::NextMoveAccuracy)
                && berries.reached(HpShare::Pinch, pokemon.hp, pokemon.max_hp, gluttony) =>
            {
                self.eat_item(holder);
            }
            Some(ItemUpdate::Eat(_)) | None => {}
        }
    }
}
