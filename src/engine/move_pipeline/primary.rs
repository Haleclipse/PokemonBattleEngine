//! Declarative status primaries (DESIGN.md §15.2): one mutation request per target.

use crate::dex::{FollowUp, ResolvedMove, StatusMoveEffect};
use crate::ids::PokemonId;
use crate::state::VolatileConditionKind;

use crate::engine::{BattleContext, EffectSource, MoveFailReason, TraceEvent};

use super::{HitOutcome, MovePipeline};

impl<'a> MovePipeline<'a> {
    /// Status-move primary for one target that passed the hit gate. Every branch goes through
    /// the corresponding mutation request, so immunities and "no change" are reported by the
    /// request's own pipeline (StatusBlocked, NoChange, FullHp).
    ///
    /// The flag is whether the move reached the target even though it did nothing there: a
    /// Heal Block on a target already under one fails (`-fail`), yet its `onRestart` returns
    /// nothing, so `addVolatile` succeeds and the target counts as hit (`gotAttacked`,
    /// which Gen 4's Mirror Move answers; pokeplatinum's `moveCopied` likewise records any move
    /// whose attack message was shown, R-M8-108).
    pub(super) fn apply_status_primary(
        &mut self,
        actor: PokemonId,
        target: PokemonId,
        move_def: &ResolvedMove,
        effect: &StatusMoveEffect,
    ) -> (HitOutcome, bool) {
        let source = EffectSource::Move {
            pokemon: actor,
            move_id: move_def.id,
        };
        // A heal's share of the target (Showdown `moveData.heal`, Heal Pulse, Floral Healing):
        // of its `baseMaxhp`, which a Dynamax does not double.
        let max_hp = self.state.pokemon(target).map_or(0, |p| p.base_max_hp());
        let sun = crate::engine::weather_for(self.state, self.rules, actor)
            == Some(crate::state::WeatherKind::Sun);
        let field_terrain = self.state.field.terrain.map(|terrain| terrain.kind);
        let weather_heal = match effect {
            StatusMoveEffect::WeatherHeal(heal) => {
                crate::engine::weather_heal_amount(self.state, self.rules, target, *heal)
            }
            _ => 0,
        };
        // Gen 1's Hyper Beam sleep bug: the sleep overwrites any other status (the gen1
        // `setStatus`, without undoing its stat cut) and, once it takes, clears the recharge.
        let recharge_bug =
            crate::engine::recharge_sleep_bug(self.state, self.rules, target, move_def);
        let overwrites = recharge_bug
            && self.state.pokemon(target).is_some_and(|pokemon| {
                pokemon
                    .status
                    .is_some_and(|s| s.kind != crate::state::NonVolatileStatus::Sleep)
            });
        // Showdown `failencore` on the target's last move (Encore itself, Transform …), and its
        // `move.isZ || move.isMax` (a Z-Move or Max Move cannot be encored, DESIGN.md §44.4).
        // A Max Move is read through its `baseMove` first (`encore`'s `move.isMax &&
        // move.baseMove`, §44.5), so a Dynamaxed target's base move is what is encored.
        let last_move_fails_encore = self
            .state
            .pokemon(target)
            .and_then(|p| p.last_move_encore_resolved())
            .and_then(|last| self.dex.move_by_id(last))
            .is_some_and(|last| last.flags.fail_encore || last.z_crystal.is_some() || last.is_max)
            // A status move used as a Z-Move (`isZ` on its own id, R-M9-36).
            || self.state.pokemon(target).is_some_and(|p| p.last_move_z);
        // Gen 2's Substitute lets Swagger's Attack boost through and drops its confusion (the
        // gen2 `substitute` deletes `volatileStatus` for Swagger by id).
        let follow_up_dropped = self.rules.conditions.substitute_gen2_quirks
            && move_def.behavior.boosts_through_gen2_substitute
            && self
                .state
                .pokemon(target)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Substitute));
        let failed_stages_end_the_move = self.rules.stats.failed_stages_end_the_move;
        let mut restarted = false;
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace).with_dex(self.dex);
        let applied = match effect {
            // Gen 1 Roar / Whirlwind: the move goes through and does nothing (no `-fail`).
            StatusMoveEffect::Inert => true,
            StatusMoveEffect::SetStatus(status) => {
                let result = if overwrites {
                    ctx.replace_status(target, *status, source)
                } else {
                    ctx.try_set_carried_status(target, *status, source)
                };
                let applied = matches!(result, crate::engine::StatusApplyResult::Applied { .. });
                if applied && recharge_bug {
                    ctx.end_volatile(target, VolatileConditionKind::MustRecharge);
                }
                applied
            }
            StatusMoveEffect::StatStages(stages) => {
                // Growth (Showdown `onModifyMove`): doubled in sun.
                let factor = if move_def.behavior.sun_doubles_boosts && sun {
                    2
                } else {
                    1
                };
                let changed = stages.iter().fold(false, |any, (stat, delta)| {
                    let result = ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
                        target,
                        stat: *stat,
                        stages: *delta * factor,
                        source,
                        cause: crate::engine::StatChangeCause::Move,
                    });
                    matches!(result, crate::engine::StatChangeResult::Applied { .. }) || any
                });
                if changed {
                    self.cut_foe_status_stats_again(actor);
                }
                changed
            }
            StatusMoveEffect::StagesThen(stages, follow_up) => {
                let changed = stages.iter().fold(false, |any, (stat, delta)| {
                    let result = ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
                        target,
                        stat: *stat,
                        stages: *delta,
                        source,
                        cause: crate::engine::StatChangeCause::Move,
                    });
                    matches!(result, crate::engine::StatChangeResult::Applied { .. }) || any
                });
                let followed = match follow_up {
                    // Gen 1: stages that changed nothing fail the move before what follows them
                    // (`StatRules::failed_stages_end_the_move`, R-M11-106).
                    _ if !changed && failed_stages_end_the_move => false,
                    // A status that does not take fails the move on this target, the stages
                    // staying changed.
                    FollowUp::Status(status) => matches!(
                        ctx.try_set_carried_status(target, *status, source),
                        crate::engine::StatusApplyResult::Applied { .. }
                    ),
                    FollowUp::Volatile(VolatileConditionKind::Confusion) if !follow_up_dropped => {
                        let confused = ctx.try_set_confusion(target, actor).is_ok();
                        ctx.confusion_move_hit(target);
                        confused || changed
                    }
                    FollowUp::Volatile(
                        kind @ (VolatileConditionKind::TarShot
                        | VolatileConditionKind::DefenseCurl
                        | VolatileConditionKind::Minimize),
                    ) => ctx.set_marker_volatile(target, *kind).is_ok() || changed,
                    FollowUp::Volatile(VolatileConditionKind::Charge) => {
                        ctx.charge(target);
                        true
                    }
                    FollowUp::Volatile(_) => changed,
                };
                // Gen 1 (Showdown `moveHit`): the user's foe, if paralysed or burned, is cut
                // again — after the boost, as the follow-up never gives it a status (Swagger's
                // confusion is not one), so its place does not change what it reads.
                if changed {
                    self.cut_foe_status_stats_again(actor);
                }
                followed
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::Confusion) => {
                let confused = ctx.try_set_confusion(target, actor).is_ok();
                ctx.confusion_move_hit(target);
                confused
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::AquaRing) => ctx
                .set_marker_volatile(target, VolatileConditionKind::AquaRing)
                .is_ok(),
            StatusMoveEffect::Volatile(VolatileConditionKind::FocusEnergy) => {
                ctx.set_focus_energy(target).is_ok()
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::DragonCheer) => {
                ctx.set_dragon_cheer(target).is_ok()
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::Charge) => {
                ctx.charge(target);
                true
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::Powder) => ctx
                .set_marker_volatile(target, VolatileConditionKind::Powder)
                .is_ok(),
            StatusMoveEffect::Volatile(VolatileConditionKind::LaserFocus) => {
                ctx.set_laser_focus(target);
                true
            }
            StatusMoveEffect::Volatile(kind @ VolatileConditionKind::Taunt) => {
                let result = ctx.set_taunt(target);
                set_or_refused(&mut ctx, target, *kind, result)
            }
            StatusMoveEffect::Volatile(kind @ VolatileConditionKind::Torment) => {
                let result = ctx.set_torment(target);
                set_or_refused(&mut ctx, target, *kind, result)
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::Imprison) => ctx
                .set_imprison(target, move_def.behavior.imprison_needs_shared_move)
                .is_ok(),
            StatusMoveEffect::Volatile(kind @ VolatileConditionKind::Encore) => {
                let result = ctx.set_encore(target, last_move_fails_encore);
                set_or_refused(&mut ctx, target, *kind, result)
            }
            StatusMoveEffect::Volatile(kind @ VolatileConditionKind::Attract) => {
                let result = ctx.set_attract(target, actor);
                set_or_refused(&mut ctx, target, *kind, result)
            }
            StatusMoveEffect::Volatile(kind @ VolatileConditionKind::Yawn) => {
                let result = ctx.set_yawn(target, actor);
                set_or_refused(&mut ctx, target, *kind, result)
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::Nightmare) => {
                ctx.set_nightmare(target).is_ok()
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::Embargo) => {
                ctx.set_embargo(target).is_ok()
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::GastroAcid) => {
                ctx.set_gastro_acid(target).is_ok()
            }
            StatusMoveEffect::Volatile(VolatileConditionKind::HealBlock) => match ctx
                .set_heal_block(
                    target,
                    crate::effects::moves::heal_block_turns(&move_def.behavior),
                ) {
                Ok(()) => true,
                Err(crate::engine::ConditionApplyError::AlreadyActive) => {
                    restarted = true;
                    false
                }
                Err(crate::engine::ConditionApplyError::Immune) => {
                    ctx.announce_volatile_refusal(target, VolatileConditionKind::HealBlock);
                    false
                }
                Err(_) => false,
            },
            // Psycho Shift: the user's status goes to the target (without one, its `onTryHit`
            // fails the move); once it took there, the user is cured (`self`: `selfDrops` skip
            // a target the move failed on). Its `onTryHit` makes the status the move's own
            // (`move.status`): an ability keeping it off says so (R-M11-31).
            StatusMoveEffect::TransferStatus => ctx.status_of(actor).is_some_and(|status| {
                let took = matches!(
                    ctx.try_set_carried_status(target, status, source),
                    crate::engine::StatusApplyResult::Applied { .. }
                );
                if took {
                    ctx.cure_status(actor, crate::engine::CureReason::Move(move_def.id));
                }
                took
            }),
            // Purify: the target's status is cured, then the user heals (rounded up).
            StatusMoveEffect::CureTarget(heal) => {
                let cured = ctx.cure_status(target, crate::engine::CureReason::Move(move_def.id));
                if cured && let Some(user_max_hp) = ctx.max_hp(actor) {
                    let numerator = u32::from(user_max_hp) * u32::from(heal.numerator);
                    let amount = numerator.div_ceil(u32::from(heal.denominator.max(1)));
                    ctx.apply_heal(crate::engine::HealRequest {
                        target: actor,
                        amount: amount.min(u32::from(u16::MAX)) as u16,
                        source,
                        direct: false,
                    });
                }
                cured
            }
            StatusMoveEffect::TargetHeal(heal) => {
                let more = match heal.more_when {
                    crate::effects::moves::TargetHealWhen::UserAbility(ability) => {
                        ctx.ability_of(actor) == Some(ability)
                    }
                    crate::effects::moves::TargetHealWhen::Terrain(terrain) => {
                        field_terrain == Some(terrain)
                    }
                };
                let amount = if more {
                    crate::engine::showdown_modify(u32::from(max_hp), heal.modifier)
                } else {
                    u32::from(max_hp).div_ceil(2)
                };
                let healed = matches!(
                    ctx.apply_heal(crate::engine::HealRequest {
                        target,
                        amount: amount.min(u32::from(u16::MAX)) as u16,
                        source,
                        direct: false,
                    }),
                    crate::engine::HealResult::Applied { .. }
                );
                // `if (success && !target.isAlly(source)) target.staleness = 'external'`.
                if healed {
                    ctx.make_stale(target, Some(actor));
                }
                healed
            }
            StatusMoveEffect::Volatile(_) => false,
            // Weather targets the field, never a Pokémon (dispatched in `execute`).
            StatusMoveEffect::Weather(_) => false,
            // Roar / Whirlwind: refused (Showdown `-fail`) when the target's side has no bench.
            StatusMoveEffect::ForceSwitch => ctx.request_drag_out(target, source),
            StatusMoveEffect::WeatherHeal(_) => {
                matches!(
                    ctx.apply_heal(crate::engine::HealRequest {
                        target,
                        amount: weather_heal,
                        source,
                        direct: false,
                    }),
                    crate::engine::HealResult::Applied { .. }
                )
            }
            StatusMoveEffect::Heal(_)
                if ctx.hp(target).is_some_and(|hp| {
                    crate::engine::heal_fails_255_short(&self.rules.conditions, hp, max_hp)
                }) =>
            {
                false
            }
            StatusMoveEffect::Heal(fraction) => {
                let numerator = u32::from(max_hp) * u32::from(fraction.numerator);
                let denominator = u32::from(fraction.denominator.max(1));
                let amount = if self.rules.conditions.heal_rounds_half_up {
                    (numerator + denominator / 2) / denominator
                } else {
                    numerator / denominator
                } as u16;
                matches!(
                    ctx.apply_heal(crate::engine::HealRequest {
                        target,
                        amount,
                        source,
                        direct: false,
                    }),
                    crate::engine::HealResult::Applied { .. }
                )
            }
        };
        if applied {
            (HitOutcome::Effect { target }, true)
        } else {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id: move_def.id,
                reason: MoveFailReason::NoEffect,
            });
            (HitOutcome::NoEffect { target }, restarted)
        }
    }
}

/// A volatile a status move sets (`result`): an ability's refusal says so
/// (`BattleContext::announce_volatile_refusal`).
fn set_or_refused(
    ctx: &mut BattleContext<'_>,
    target: crate::ids::PokemonId,
    kind: VolatileConditionKind,
    result: Result<(), crate::engine::ConditionApplyError>,
) -> bool {
    if result == Err(crate::engine::ConditionApplyError::Immune) {
        ctx.announce_volatile_refusal(target, kind);
    }
    result.is_ok()
}
