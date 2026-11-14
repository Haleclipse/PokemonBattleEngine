//! `BattleContext`'s stat stages (DESIGN.md §22): a stage set or cleared, a stored stat, a stage
//! change tried and resolved — the refusals, Mirror Armor, Contrary, Simple, Defiant and its kin
//! — Intimidate's shields.

use crate::engine::{
    StatChangeBlockReason, StatChangeCause, StatChangeRequest, StatChangeResult, TraceEvent,
    TryStatChangeResult,
};
use crate::ids::{MoveId, PokemonId};
use crate::state::{Stat, VolatileConditionKind};

use super::*;

impl<'a> BattleContext<'a> {
    /// Showdown `clearBoosts()`: every stat stage of `target` back to 0, directly (no TryBoost);
    /// each change is a `StatChanged`. Gen 1's (the gen1 `clearBoosts`) also puts the modified
    /// stats back to the stored ones — a paralysis or burn cut included.
    pub fn clear_stages(&mut self, target: PokemonId, source: EffectSource) {
        for stat in STAGED_STATS {
            self.set_stage(target, stat, 0, source);
        }
        if let Some(pokemon) = self.state.pokemon_mut(target)
            && pokemon.modified_stats.is_some()
        {
            pokemon.modified_stats = Some(*pokemon.battle_stats());
        }
    }

    /// Showdown `setBoost` / a direct `boosts[stat] = …`: `target`'s stage in `stat` becomes
    /// `to` with no TryBoost (Heart Swap, Topsy-Turvy); a change is a `StatChanged`.
    pub fn set_stage(&mut self, target: PokemonId, stat: Stat, to: i8, source: EffectSource) {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return;
        };
        let from = pokemon.stages.get(stat);
        if from != to {
            pokemon.stages.set(stat, to);
            self.trace.push(TraceEvent::StatChanged {
                target,
                stat,
                from,
                to,
                source,
            });
        }
    }

    /// A move rewrites `pokemon`'s stored stat until it leaves the field
    /// (`PokemonState::set_stored_stat`).
    pub fn set_stored_stat(&mut self, pokemon: PokemonId, stat: Stat, value: u16) {
        if let Some(p) = self.state.pokemon_mut(pokemon) {
            p.set_stored_stat(stat, value);
        }
    }

    /// Intimidate and similar: a Substitute shields the target (Showdown `-immune`); in Gen 4
    /// so does a Substitute U-turn just broke.
    pub fn shielded_from_intimidation(&self, target: PokemonId) -> bool {
        self.state.pokemon(target).is_some_and(|p| {
            p.volatiles.contains(VolatileConditionKind::Substitute)
                || (self
                    .rules
                    .conditions
                    .intimidate_blocked_by_u_turn_broken_substitute
                    && p.volatiles.substitute_broken_by() == Some(MoveId::U_TURN))
        })
    }

    /// Request → Resolve → Commit for a stat stage change, in Showdown's `boost()` order:
    /// ChangeBoost (Contrary / Simple rewrite the stages) → cap to ±6 → TryBoost (Clear Body
    /// refuses, Mirror Armor sends the drop back) → commit.
    pub fn try_change_stat_stage(&mut self, request: StatChangeRequest) -> StatChangeResult {
        let result = self.change_stat_stage_resolved(request);
        // Showdown `runEvent('AfterBoost')` of the holder's item, after its ability's (which
        // `change_stat_stage_resolved` runs only where the stage moved): whether or not it moved,
        // but not for a Pokémon down or off the field (`boost()` returns before).
        if matches!(
            result,
            StatChangeResult::Blocked(
                StatChangeBlockReason::InvalidTarget
                    | StatChangeBlockReason::Fainted
                    | StatChangeBlockReason::NoFoeLeft
            )
        ) {
            return result;
        }
        if let Some((_, item)) = crate::engine::held_item(self.state, self.rules, request.target)
            && let Some(handler) = item.after_boost
        {
            handler(self, request.target, request, result);
        }
        // Then the `onFoeAfterBoost` of the abilities and items of its foes on the field
        // (Opportunist, Mirror Herb), which only keep what they saw: their order does not
        // matter.
        for foe in self.active_foes(request.target) {
            if let Some(handler) = self.state.pokemon(foe).and_then(|p| {
                crate::engine::ability_handler(self.state, self.rules, p, |b| b.foe_after_boost)
            }) {
                handler(self, foe, request, result);
            }
            if let Some((_, item)) = crate::engine::held_item(self.state, self.rules, foe)
                && let Some(handler) = item.foe_after_boost
            {
                handler(self, foe, request, result);
            }
        }
        result
    }

    fn change_stat_stage_resolved(&mut self, request: StatChangeRequest) -> StatChangeResult {
        let Some(pokemon) = self.state.pokemon(request.target) else {
            return StatChangeResult::Blocked(StatChangeBlockReason::InvalidTarget);
        };
        if pokemon.is_fainted() {
            return StatChangeResult::Blocked(StatChangeBlockReason::Fainted);
        }
        // Gen 6+ `boost`: nothing once the side facing the Pokémon has none left.
        if self.rules.stats.boosts_need_foe_pokemon_left && !self.foe_pokemon_left(request.target) {
            return StatChangeResult::Blocked(StatChangeBlockReason::NoFoeLeft);
        }
        let ability = crate::engine::raw_ability(pokemon);
        let handlers = crate::engine::ability_handlers(self.state, self.rules, pokemon);
        let item = crate::engine::held_item_id(self.state, self.rules, request.target);
        let from = pokemon.stages.get(request.stat);

        // ChangeBoost.
        let mut request = request;
        if let Some(handler) = handlers.modify_stat_change {
            let resolved = handler(
                &StatChangeContext::new(self.state, self.rules, request),
                request.stages,
            );
            if resolved != request.stages {
                self.trace.push(TraceEvent::StatChangeModified {
                    target: request.target,
                    stat: request.stat,
                    requested: request.stages,
                    resolved,
                    by: EffectSource::Ability {
                        pokemon: request.target,
                        ability,
                    },
                });
                request.stages = resolved;
            }
        }

        // Cap: what can still move within ±6.
        let to = (from + request.stages).clamp(-6, 6);
        if from == to {
            return StatChangeResult::Blocked(StatChangeBlockReason::NoChange);
        }
        request.stages = to - from;

        // Guard Dog's `onTryBoost` (`onTryBoostPriority` 2, ahead of everything below) takes
        // the change over.
        if let Some(handler) = handlers.try_boost_first
            && handler(self, request)
        {
            // Its own rise (`-ability|holder|Guard Dog|boost`) shows it, where there is one.
            let reason = StatChangeBlockReason::Ability(ability);
            self.trace.push(TraceEvent::StatChangeBlocked {
                target: request.target,
                stat: request.stat,
                stages: request.stages,
                source: request.source,
                reason,
                shown_by: None,
            });
            return StatChangeResult::Blocked(reason);
        }

        // TryBoost, in Showdown's handler order: the item first (Clear Amulet's
        // `onTryBoostPriority` 1), then the abilities — Mirror Armor's return of a drop included —
        // and Mist, a side's handler, last: Showdown sorts a Pokémon's handlers before a side's at
        // the same priority, a side having no speed (issues.md I9).
        // Each refusal with the Pokémon whose ability or item refused it (a Flower Veil an ally's)
        // and whether its line shows.
        let refusal = {
            let ctx = StatChangeContext::new(self.state, self.rules, request);
            let refused = |result, holder| match result {
                TryStatChangeResult::Allow => None,
                TryStatChangeResult::Blocked { reason, shown } => Some((reason, shown, holder)),
            };
            let by_item = item
                .and_then(|item| crate::engine::item_behavior(self.rules, item).try_stat_change)
                .and_then(|handler| refused(handler(&ctx), request.target));
            // The target's own `onTryBoost` (Clear Body …) and every `onAllyTryBoost` (Flower
            // Veil, the target's own included) share one priority: the faster holder's first
            // (`speedSort`, R-M10-53) — a faster ally's Flower Veil before a Grass type's own Clear
            // Body (R-M11-166). A tie keeps the target's own first (Showdown shuffles it;
            // unmodelled).
            by_item.or_else(|| {
                let own = handlers
                    .try_stat_change
                    .map(|handler| (request.target, handler));
                let mut holders: Vec<_> = own
                    .into_iter()
                    .chain(
                        crate::engine::allied_ability_holders(
                            self.state,
                            self.rules,
                            request.target,
                        )
                        .into_iter()
                        .filter_map(|(holder, behavior)| {
                            behavior
                                .ally_try_stat_change
                                .map(|handler| (holder, handler))
                        }),
                    )
                    .collect();
                holders.sort_by_key(|(holder, _)| {
                    std::cmp::Reverse(crate::engine::stats::handler_speed(self.state, *holder))
                });
                holders
                    .into_iter()
                    .find_map(|(holder, handler)| refused(handler(&ctx), holder))
            })
        };
        if let Some((reason, shown, holder)) = refusal {
            let shown_by = shown
                .then_some(match reason {
                    StatChangeBlockReason::Ability(ability) => Some(EffectSource::Ability {
                        pokemon: holder,
                        ability,
                    }),
                    StatChangeBlockReason::Item(item) => Some(EffectSource::Item {
                        pokemon: holder,
                        item,
                    }),
                    _ => None,
                })
                .flatten();
            self.trace.push(TraceEvent::StatChangeBlocked {
                target: request.target,
                stat: request.stat,
                stages: request.stages,
                source: request.source,
                reason,
                shown_by,
            });
            return StatChangeResult::Blocked(reason);
        }
        if self.mirror_armor_refuses(request, handlers) {
            let reason = StatChangeBlockReason::Ability(ability);
            // `-ability|holder|Mirror Armor` while the drop's culprit has HP (`source.hp`),
            // whether the drop sent back lands or not.
            let shown_by = request
                .source
                .pokemon()
                .is_some_and(|culprit| !self.is_fainted(culprit))
                .then_some(EffectSource::Ability {
                    pokemon: request.target,
                    ability,
                });
            self.trace.push(TraceEvent::StatChangeBlocked {
                target: request.target,
                stat: request.stat,
                stages: request.stages,
                source: request.source,
                reason,
                shown_by,
            });
            // Sent back with Showdown `boost()`, which does nothing to a Pokémon down or off
            // the field.
            if let Some(culprit) = request.source.pokemon()
                && !self.is_fainted(culprit)
                && self.is_active(culprit)
            {
                self.try_change_stat_stage(StatChangeRequest {
                    target: culprit,
                    stat: request.stat,
                    stages: request.stages,
                    source: EffectSource::Ability {
                        pokemon: request.target,
                        ability,
                    },
                    cause: StatChangeCause::Ability,
                });
            }
            return StatChangeResult::Blocked(reason);
        }
        if crate::engine::protections::mist_refuses(self.state, self.rules, &request) {
            let reason = StatChangeBlockReason::Mist;
            self.trace.push(TraceEvent::StatChangeBlocked {
                target: request.target,
                stat: request.stat,
                stages: request.stages,
                source: request.source,
                reason,
                shown_by: None,
            });
            return StatChangeResult::Blocked(reason);
        }

        // Gen 2 (the gen2 `boostBy`): a raise of a stat that reads exactly 999 changes nothing
        // (a species item doubling it past the clamp reads more). Gen 2 has no Gen 1
        // `modified_stats`, so this comes before the commit below as it did after it.
        if self.rules.stats.raise_stops_at_cap
            && request.stages > 0
            && self.capped_stat_reading(request.target, request.stat, from) == 999
        {
            return StatChangeResult::Blocked(StatChangeBlockReason::NoChange);
        }

        // Commit.
        let stage_table = self.rules.stats.stage_table;
        let Some(pokemon) = self.state.pokemon_mut(request.target) else {
            return StatChangeResult::Blocked(StatChangeBlockReason::InvalidTarget);
        };
        // Gen 1 (Showdown gen1 `boostBy`): the modified stat is recomputed from the stored one
        // (a paralysis / burn cut on it is gone), capped at 999 when raised; raising a stat at
        // 999 (lowering one at 1) moves the stage one short and does not count as a change.
        if let Some(modified) = pokemon.modified_stats.and_then(|m| m.get(request.stat)) {
            let stuck =
                (request.stages > 0 && modified == 999) || (request.stages < 0 && modified == 1);
            if stuck {
                let back = if request.stages > 0 { to - 1 } else { to + 1 };
                pokemon.stages.set(request.stat, back);
                self.trace.push(TraceEvent::StatChanged {
                    target: request.target,
                    stat: request.stat,
                    from,
                    to: back,
                    source: request.source,
                });
                return StatChangeResult::Blocked(StatChangeBlockReason::NoChange);
            }
            let stored = pokemon.battle_stats().get(request.stat).unwrap_or(1);
            let mut next = crate::engine::stats::stage_value(stage_table, stored, to).max(1);
            if request.stages > 0 {
                next = next.min(999);
            }
            if let Some(stats) = &mut pokemon.modified_stats {
                stats.set(request.stat, next.min(u32::from(u16::MAX)) as u16);
            }
        }
        pokemon.stages.set(request.stat, to);
        // Showdown `statsLoweredThisTurn` (Lash Out) / `statsRaisedThisTurn` (Burning Jealousy).
        if to < from {
            pokemon.history.stats_lowered_this_turn = true;
        }
        if to > from {
            pokemon.history.stats_raised_this_turn = true;
        }

        self.trace.push(TraceEvent::StatChanged {
            target: request.target,
            stat: request.stat,
            from,
            to,
            source: request.source,
        });
        // Showdown `onAfterEachBoost` (Defiant, Competitive) and `onAfterBoost` (Rattled) of
        // the Pokémon whose stage moved.
        if let Some(handler) = handlers.after_each_boost {
            handler(self, request.target, request);
        }
        if let Some(handler) = handlers.after_boost {
            handler(self, request.target, request);
        }
        StatChangeResult::Applied { from, to }
    }

    /// Mirror Armor's `onTryBoost`: the holder refuses a drop (the capped stages: Showdown
    /// reflects after `getCappedBoost`) unless it caused the drop itself or Mirror Armor sent it
    /// (`effect.name === 'Mirror Armor'`); the drop goes back to whoever caused it.
    fn mirror_armor_refuses(
        &self,
        request: StatChangeRequest,
        handlers: &crate::effects::abilities::AbilityBehavior,
    ) -> bool {
        if request.stages >= 0
            || !handlers.reflects_stat_drops
            || request.source.pokemon() == Some(request.target)
        {
            return false;
        }
        !matches!(request.source, EffectSource::Ability { ability: sent, .. }
            if crate::engine::ability_behavior(self.rules, sent).reflects_stat_drops)
    }

    pub fn change_stat_stage(
        &mut self,
        target: PokemonId,
        stat: Stat,
        delta: i8,
        source: EffectSource,
    ) -> StatChangeResult {
        self.try_change_stat_stage(StatChangeRequest {
            target,
            stat,
            stages: delta,
            source,
            cause: StatChangeCause::Rule,
        })
    }
}
