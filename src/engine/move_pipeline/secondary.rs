//! Secondary effects (DESIGN.md §21): per-hit and once-per-move secondaries through the same
//! mutation requests as primaries, and Gen 1's re-applied paralysis / burn stat cuts.

use crate::dex::{ResolvedMove, SecondaryEffect, SecondaryTarget, SecondaryTrigger};
use crate::ids::PokemonId;
use crate::state::{RandomRequestKind, VolatileConditionKind};

use super::MovePipeline;
use crate::engine::{BattleContext, EffectSource, TraceEvent};

impl<'a> MovePipeline<'a> {
    /// `move_target`: the Pokémon the move was aimed at (Gen 1's recharge is skipped when it
    /// fainted). Returns the status a random-status secondary drew (Gen 2 Tri Attack's
    /// `statusRoll` thaws a frozen target when it is a burn).
    pub(super) fn apply_secondary_effects(
        &mut self,
        actor: PokemonId,
        hit_target: PokemonId,
        move_target: Option<PokemonId>,
        move_def: &ResolvedMove,
        trigger: SecondaryTrigger,
        behind_substitute: bool,
    ) -> Option<crate::state::NonVolatileStatus> {
        let mut picked = None;
        // One Showdown secondary with several effects rolls once (`shares_roll`).
        let mut previous: Option<bool> = None;
        let mut stages_changed = false;
        // A Substitute keeps the target-directed secondaries from the body behind it; the
        // user-directed ones (Power-Up Punch) still land, and so does a Gen 1 confusion past a
        // doll still standing.
        let doll_up = self.state.pokemon(hit_target).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .contains(VolatileConditionKind::Substitute)
        });
        let past_the_doll = |secondary: &crate::dex::SecondaryEffectSpec| {
            self.rules.conditions.confusion_secondary_through_substitute
                && doll_up
                && secondary.effect == SecondaryEffect::Volatile(VolatileConditionKind::Confusion)
        };
        for secondary in move_def.secondaries.iter().copied().filter(|secondary| {
            secondary.trigger == trigger
                && !(behind_substitute
                    && secondary.target == SecondaryTarget::Target
                    && !past_the_doll(secondary))
                && !(trigger == SecondaryTrigger::PerTargetHit
                    && crate::engine::secondary_withheld(
                        self.rules,
                        move_def,
                        self.hit,
                        self.hit_count,
                        secondary,
                    ))
        }) {
            let affected = match secondary.target {
                SecondaryTarget::Target => hit_target,
                SecondaryTarget::User => actor,
            };
            // A random status (Showdown `this.sample`): Gen 2 Tri Attack's `onHit` draws it on
            // every hit before the secondary's chance; otherwise it is drawn once the chance
            // succeeds.
            let pick = |state: &mut crate::state::BattleState,
                        pool: [crate::state::NonVolatileStatus; 3]| {
                let index = state
                    .rng
                    .range_inclusive_for(RandomRequestKind::Sample, 0, 2);
                pool[usize::from(index)]
            };
            let early_pick = match secondary.effect {
                SecondaryEffect::RandomStatus(pool)
                    if move_def.behavior.status_pick_before_chance =>
                {
                    Some(pick(self.state, pool))
                }
                _ => None,
            };
            picked = picked.or(early_pick);

            let success = match previous.filter(|_| secondary.shares_roll) {
                Some(shared) => shared,
                None => match crate::engine::secondary_roll(
                    self.state, self.rules, actor, affected, move_def, &secondary,
                ) {
                    crate::engine::SecondaryRoll::Certain => true,
                    crate::engine::SecondaryRoll::Skipped => false,
                    crate::engine::SecondaryRoll::Chance(numerator, denominator) => self
                        .state
                        .rng
                        .roll_ratio_for(RandomRequestKind::Secondary, numerator, denominator),
                },
            };
            previous = Some(success);
            self.trace.push(TraceEvent::SecondaryEffectRolled {
                actor,
                affected,
                move_id: move_def.id,
                effect: secondary.effect,
                chance_basis_points: secondary.chance.basis_points(),
                success,
            });
            if !success {
                continue;
            }

            let source = EffectSource::Move {
                pokemon: actor,
                move_id: move_def.id,
            };
            let affected_alive = self
                .state
                .pokemon(affected)
                .is_some_and(|pokemon| !pokemon.is_fainted());
            let recharge_skipped = self.rules.conditions.recharge_skipped_on_knock_out
                && move_target
                    .and_then(|target| self.state.pokemon(target))
                    .is_some_and(|pokemon| pokemon.is_fainted());
            let random_status = match secondary.effect {
                SecondaryEffect::RandomStatus(pool) => {
                    let status = early_pick.unwrap_or_else(|| pick(self.state, pool));
                    picked = Some(status);
                    Some(status)
                }
                _ => None,
            };
            let used_gem = self.gem.is_some();
            let applied = {
                let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                match secondary.effect {
                    SecondaryEffect::StealItem => {
                        ctx.steal_item(actor, affected, move_def.id, used_gem)
                    }
                    SecondaryEffect::EerieSpell => ctx.eerie_spell(actor, affected, move_def.id),
                    SecondaryEffect::Status(status) => matches!(
                        ctx.try_set_status(affected, status, source),
                        crate::engine::StatusApplyResult::Applied { .. }
                    ),
                    SecondaryEffect::RandomStatus(_) => random_status.is_some_and(|status| {
                        matches!(
                            ctx.try_set_status(affected, status, source),
                            crate::engine::StatusApplyResult::Applied { .. }
                        )
                    }),
                    SecondaryEffect::Volatile(kind) => {
                        affected_alive
                            && match kind {
                                VolatileConditionKind::Flinch => {
                                    ctx.set_flinch(affected, actor).is_ok()
                                }
                                VolatileConditionKind::Confusion => {
                                    ctx.try_set_confusion(affected, actor).is_ok()
                                }
                                // Gen 1: knocking the target out skips the recharge.
                                VolatileConditionKind::MustRecharge => {
                                    !recharge_skipped && ctx.set_must_recharge(affected).is_ok()
                                }
                                VolatileConditionKind::PartiallyTrapped => ctx
                                    .set_partially_trapped(affected, actor, move_def.id)
                                    .is_ok(),
                                VolatileConditionKind::SaltCure
                                | VolatileConditionKind::SparklingAria
                                | VolatileConditionKind::Roost => {
                                    ctx.set_marker_volatile(affected, kind).is_ok()
                                }
                                VolatileConditionKind::SyrupBomb => {
                                    ctx.set_syrup_bomb(affected, actor).is_ok()
                                }
                                VolatileConditionKind::ThroatChop => {
                                    ctx.set_throat_chop(affected).is_ok()
                                }
                                // Aroma Veil's refusal names it (R-M10-59).
                                VolatileConditionKind::HealBlock => {
                                    let result = ctx.set_heal_block(
                                        affected,
                                        crate::effects::moves::heal_block_turns(&move_def.behavior),
                                    );
                                    if result == Err(crate::engine::ConditionApplyError::Immune) {
                                        ctx.announce_secondary_volatile_refusal(affected, kind);
                                    }
                                    result.is_ok()
                                }
                                // Anchor Shot, Spirit Shackle (`if (source.isActive)`).
                                VolatileConditionKind::Trapped => ctx.trap(affected, actor).is_ok(),
                                VolatileConditionKind::SmackDown => {
                                    ctx.smack_down(affected).is_ok()
                                }
                                _ => false,
                            }
                    }
                    SecondaryEffect::StatStage { stat, delta } => {
                        if affected_alive {
                            matches!(
                                ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
                                    target: affected,
                                    stat,
                                    stages: delta,
                                    source,
                                    cause: crate::engine::StatChangeCause::Secondary,
                                }),
                                crate::engine::StatChangeResult::Applied { .. }
                            )
                        } else {
                            false
                        }
                    }
                }
            };

            if applied {
                stages_changed |= matches!(secondary.effect, SecondaryEffect::StatStage { .. });
                self.trace.push(TraceEvent::SecondaryEffectApplied {
                    actor,
                    affected,
                    move_id: move_def.id,
                    effect: secondary.effect,
                });
            }
        }
        if stages_changed {
            self.cut_foe_status_stats_again(actor);
        }
        picked
    }

    /// Gen 1 (Showdown gen1 `moveHit`): after a move changes stat stages, the user's foe, if
    /// paralysed or burned, has its Speed quartered / Attack halved again.
    pub(super) fn cut_foe_status_stats_again(&mut self, user: PokemonId) {
        if !self.rules.stats.modified_stats {
            return;
        }
        let Some(side) = self.state.pokemon(user).map(|p| p.side) else {
            return;
        };
        if let Some(foe) = self.state.side(side.other()).active[0]
            && let Some(pokemon) = self.state.pokemon_mut(foe)
        {
            pokemon.apply_status_stat_cut();
        }
    }
}
