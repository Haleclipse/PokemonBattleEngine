//! `BattleContext`'s HP (DESIGN.md §15, §22): an HP loss committed, a damage applied — through
//! a Substitute, a Disguise or an Ice Face, Focus Sash and its kin — a damage resolved, a heal
//! applied or refused, Ripen's double.

use crate::engine::{
    DamageAmount, DamageApplication, DamageDecision, DamageKind, DamageRequest, HealBlockReason,
    HealRequest, HealResult, TraceEvent,
};
use crate::ids::{ItemId, MoveId, PokemonId};
use crate::state::VolatileConditionKind;

use super::*;

impl<'a> BattleContext<'a> {
    /// Showdown `sethp`: `target`'s HP becomes `hp` (at least 1, at most its max); nothing for a
    /// fainted Pokémon. Traced as `HpSet`.
    pub fn set_hp(&mut self, target: PokemonId, hp: u16, source: EffectSource) {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return;
        };
        if pokemon.is_fainted() {
            return;
        }
        let before = pokemon.hp;
        pokemon.hp = hp.clamp(1, pokemon.max_hp);
        let after = pokemon.hp;
        self.trace.push(TraceEvent::HpSet {
            target,
            hp_before: before,
            hp_after: after,
            source,
        });
    }

    fn commit_hp_loss(
        &mut self,
        target: PokemonId,
        amount: u16,
        source: EffectSource,
        kind: DamageKind,
        future_move: bool,
    ) -> u16 {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return 0;
        };

        let before = pokemon.hp;
        let applied = amount.min(before);
        pokemon.hp -= applied;
        let after = pokemon.hp;
        // Showdown `hurtThisTurn` (Assurance), set by every `spreadDamage` that took HP — not by
        // `directDamage` (Struggle's recoil, a Substitute's HP).
        if applied > 0 && !kind.is_direct() {
            pokemon.history.hurt_this_turn = true;
        }
        if after == 0 && before > 0 && !self.state.faint_queue.contains(&target) {
            self.state.faint_queue.push(target);
            self.state.faint_causes.push(crate::state::FaintCause {
                target,
                source: source.pokemon(),
                by_move: kind == DamageKind::Move,
                future_move,
                listeners_run: false,
                bonds_run: false,
                after_faint_run: false,
                counted: false,
            });
        }

        // A move's price is traced apart: the replay leaves it out of the move's HP outcome.
        self.trace.push(if kind == DamageKind::Cost {
            TraceEvent::HpCostPaid {
                target,
                amount: applied,
                hp_before: before,
                hp_after: after,
                source,
            }
        } else {
            TraceEvent::DamageApplied {
                target,
                amount: applied,
                hp_before: before,
                hp_after: after,
                source,
            }
        });

        applied
    }

    /// Request → Resolve → Commit for HP loss (DESIGN.md §20). Move damage is routed through
    /// the Substitute first (a Substitute hit resolves nothing: Showdown's Substitute condition
    /// takes it before `onDamage`); everything that reaches HP passes the target's damage
    /// resolution behavior (Magic Guard, Sturdy, Focus Sash) before the commit.
    pub fn apply_damage(&mut self, request: DamageRequest) -> DamageApplication {
        // Showdown `spreadDamage`: a Pokémon off the field takes nothing (`!target.isActive`) —
        // a benched attacker a Jaboca or Rowap Berry answers, its foreseen attack having struck
        // after it left (DESIGN.md R277). (`directDamage`, a self-hit's or a cost's, is
        // always on the field.)
        if self
            .state
            .active_position(request.target, &self.rules.topology)
            .is_none()
        {
            return DamageApplication::Prevented {
                target: request.target,
            };
        }
        // Gen 1 `battle.lastDamage` (§15.8): a move's damage, a confusion self-hit or a crash —
        // not recoil, drain or a residual. A move's damage on the body is capped at the target's
        // HP first (the gen1 `moveHit`'s `damage > target.hp`; pokered caps `wDamage` as the HP
        // reaches 0); a Substitute's takes it uncapped, and so do the self-hit and the crash
        // (`directDamage` of `confusion` and Jump Kick — not a Substitute's price).
        if self.rules.conditions.retaliation.gen1_last_damage
            && matches!(
                request.kind,
                DamageKind::Move | DamageKind::SelfHit | DamageKind::Direct
            )
        {
            let behind_substitute = !request.flags.bypass_substitute
                && self.state.pokemon(request.target).is_some_and(|p| {
                    p.volatiles
                        .contains(crate::state::VolatileConditionKind::Substitute)
                });
            let hp = self.state.pokemon(request.target).map_or(0, |p| p.hp);
            self.state.last_damage = if request.kind == DamageKind::Move && !behind_substitute {
                request.amount.0.min(hp)
            } else {
                request.amount.0
            };
        }
        if request.kind == DamageKind::Move
            && !request.flags.bypass_substitute
            && let Some(application) = self.absorb_with_substitute(request.target, request.amount)
        {
            if let DamageApplication::Substitute { broken: true, .. } = application
                && self.rules.conditions.substitute_broken_marker
                && let EffectSource::Move { move_id, .. } = request.source
                && let Some(pokemon) = self.state.pokemon_mut(request.target)
            {
                pokemon.volatiles.mark_substitute_broken(move_id);
            }
            return application;
        }
        match self.resolve_damage(request) {
            Some(amount) => {
                // Bide's `onDamage` (§15.10), the last handler before the HP cap.
                if request.kind == DamageKind::Move
                    && let Some(source) = request.source.pokemon()
                {
                    self.store_bide_damage(request.target, amount.0, source);
                }
                let applied = self.commit_hp_loss(
                    request.target,
                    amount.0,
                    request.source,
                    request.kind,
                    request.flags.future_move,
                );
                // `damage(…, instafaint)` knocking its target out: `faintMessages(true)` at once —
                // the target first (`lastFirst`), then the faints already waiting, each announced
                // (no longer a Pokémon left for `foePokemonLeft`), counted and heard as the hit
                // loop's end announces them, before anything after the damage (Mind Blown's
                // recoil before the `Update` that eats a foe's Apicot Berry: with no foe left, it
                // raises nothing, R-M10-80).
                if request.flags.instafaint
                    && self.is_fainted(request.target)
                    && let Some(index) = self
                        .state
                        .faint_queue
                        .iter()
                        .position(|pokemon| *pokemon == request.target)
                {
                    let target = self.state.faint_queue.remove(index);
                    self.state.faint_queue.insert(0, target);
                    self.announce_knocked_out(None);
                }
                DamageApplication::Hp {
                    target: request.target,
                    amount: DamageAmount(applied),
                }
            }
            None => DamageApplication::Prevented {
                target: request.target,
            },
        }
    }

    fn absorb_with_substitute(
        &mut self,
        target: PokemonId,
        amount: DamageAmount,
    ) -> Option<DamageApplication> {
        let (applied, before, after, broken) = self
            .state
            .pokemon_mut(target)
            .and_then(|pokemon| pokemon.volatiles.damage_substitute(amount.0))?;
        self.trace.push(TraceEvent::SubstituteDamaged {
            target,
            amount: applied,
            hp_before: before,
            hp_after: after,
            broken,
        });
        if broken {
            self.trace.push(TraceEvent::VolatileEnded {
                target,
                kind: VolatileConditionKind::Substitute,
                seen: true,
            });
        }
        Some(DamageApplication::Substitute {
            target,
            amount: DamageAmount(applied),
            broken,
        })
    }

    /// The Resolve stage of a DamageRequest: the target's ability and item behavior in Showdown's
    /// `onDamage` order (Magic Guard, then Sturdy, then Focus Sash). Each handler sees the amount
    /// the previous one left, so Sturdy's "leave 1 HP" keeps Focus Sash from being spent.
    fn resolve_damage(&mut self, request: DamageRequest) -> Option<DamageAmount> {
        // Berserk / Anger Shell's `onDamage` (priority 0: before Endure, Sturdy and Focus Sash
        // decide anything) only notes the damage.
        if let Some(ability) = self.running_ability(request.target) {
            let behavior = crate::engine::ability_behavior(self.rules, ability);
            crate::effects::abilities::damaged(self, behavior, &request);
        }
        let Some(pokemon) = self.state.pokemon(request.target) else {
            return Some(request.amount);
        };
        let mut behavior: Vec<(i16, DamageResolutionHandler, EffectSource)> = Vec::new();
        if let Some(ordered) =
            crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.damage_resolution)
        {
            behavior.push((
                ordered.priority,
                ordered.handler,
                EffectSource::Ability {
                    pokemon: request.target,
                    ability: crate::engine::raw_ability(pokemon),
                },
            ));
        }
        if let Some((item, item_behavior)) =
            crate::engine::held_item(self.state, self.rules, request.target)
            && let Some(ordered) = item_behavior.damage_resolution
        {
            behavior.push((
                ordered.priority,
                ordered.handler,
                EffectSource::Item {
                    pokemon: request.target,
                    item,
                },
            ));
        }
        // Endure (`onDamage` priority −10): after Magic Guard, before Sturdy and Focus Sash.
        if pokemon.volatiles.contains(VolatileConditionKind::Endure) {
            behavior.push((
                10,
                crate::effects::moves::endure_damage as DamageResolutionHandler,
                EffectSource::Move {
                    pokemon: request.target,
                    move_id: MoveId::ENDURE,
                },
            ));
        }
        // The move's own `onDamage` (False Swipe, Hold Back, priority -20).
        if request.flags.spare_one_hp {
            behavior.push((
                20,
                crate::effects::moves::leave_one_hp as DamageResolutionHandler,
                request.source,
            ));
        }
        behavior.sort_by_key(|(priority, _, _)| *priority);

        let mut current = request;
        for (_, handler, by) in behavior {
            match handler(self, &current) {
                DamageDecision::Allow => {}
                DamageDecision::Reduce(amount) => {
                    self.trace.push(TraceEvent::DamageReduced {
                        target: current.target,
                        requested: current.amount.0,
                        applied: amount.0,
                        by,
                    });
                    current.amount = amount;
                }
                DamageDecision::Prevent => {
                    self.trace.push(TraceEvent::DamagePrevented {
                        target: current.target,
                        amount: current.amount.0,
                        source: current.source,
                        by,
                    });
                    return None;
                }
            }
        }
        Some(current.amount)
    }

    /// Showdown `runEvent('TryHeal')` refusing a heal: Heal Block (`onTryHeal`, before the
    /// full-HP check). From Gen 5 every heal fails; the gen4 mod stops only a drain's, Leech
    /// Seed's and Wish's. Under Heal Block a Pokémon's own move heals it only by draining (the
    /// moves that heal are held at BeforeMove); a Wish is stopped by its id, whoever it heals
    /// (the wisher's replacement too).
    pub fn heal_refused(&self, target: PokemonId, source: EffectSource) -> bool {
        self.state.pokemon(target).is_some_and(|pokemon| {
            pokemon.volatiles.contains(VolatileConditionKind::HealBlock)
                && (self.rules.conditions.heal_block_stops_every_heal
                    || match source {
                        EffectSource::Volatile(VolatileConditionKind::LeechSeed) => true,
                        EffectSource::Move { pokemon, move_id } => {
                            pokemon == target || move_id == crate::ids::MoveId::WISH
                        }
                        _ => false,
                    })
        })
    }

    pub fn apply_heal(&mut self, request: HealRequest) -> HealResult {
        let Some(pokemon) = self.state.pokemon(request.target) else {
            return HealResult::Blocked(HealBlockReason::InvalidTarget);
        };
        if pokemon.is_fainted() {
            return HealResult::Blocked(HealBlockReason::Fainted);
        }
        // The `TryHeal` event, which a direct heal never runs.
        if !request.direct {
            if self.heal_refused(request.target, request.source) {
                return HealResult::Blocked(HealBlockReason::HealBlock);
            }
            // At full HP too: `TryHeal` comes before `heal` finds nothing to do.
            self.ripen_try_heal(request.target, request.source);
        }
        let Some(pokemon) = self.state.pokemon_mut(request.target) else {
            return HealResult::Blocked(HealBlockReason::InvalidTarget);
        };
        if pokemon.hp >= pokemon.max_hp {
            return HealResult::Blocked(HealBlockReason::FullHp);
        }
        let from = pokemon.hp;
        let to = pokemon
            .hp
            .saturating_add(request.amount)
            .min(pokemon.max_hp);
        pokemon.hp = to;
        self.trace.push(TraceEvent::HealApplied {
            target: request.target,
            amount: to - from,
            hp_before: from,
            hp_after: to,
            source: request.source,
        });
        HealResult::Applied { from, to }
    }

    /// Ripen's `onTryHeal` announces itself for Leftovers and Berry Juice (a berry's heal it
    /// doubles: `berry_heal`).
    pub(crate) fn ripen_try_heal(&mut self, target: PokemonId, source: EffectSource) {
        if let EffectSource::Item {
            item: ItemId::LEFTOVERS | ItemId::BERRY_JUICE,
            ..
        } = source
            && let Some(ability) = self.running_ability(target)
            && crate::engine::ability_behavior(self.rules, ability).ripen
        {
            self.ability_activated(target, ability);
        }
    }
}
