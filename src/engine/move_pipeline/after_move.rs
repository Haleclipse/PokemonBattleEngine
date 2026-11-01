//! After the primary effect and the once-per-move secondaries (DESIGN.md §15, §30): the
//! self-switch order, the struck targets' item reactions and the user's own item. (Damaging
//! phazing is part of the hit, `hit_sequence::force_switch_step`.)

use crate::dex::{MoveCategory, ResolvedMove, SecondaryTrigger};
use crate::ids::PokemonId;
use crate::rules::DamageModifierKind;

use crate::engine::{BattleContext, TraceEvent};

use super::{HitOutcome, MovePipeline};

impl<'a> MovePipeline<'a> {
    /// `pokemon` has been ordered out for a replacement (Showdown `switchFlag`).
    fn leaving(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|p| self.state.side(p.side).switch_out == Some(pokemon))
    }

    /// `hitStepMoveHitLoop`'s close for the Pokémon the move reached: their
    /// `AfterMoveSecondary` reactions, then Emergency Exit on each it hurt. A foreseen attack's
    /// strike closes the same way (DESIGN.md R277).
    pub(super) fn after_move_secondary(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        hits: &[HitOutcome],
    ) {
        let move_id = move_def.id;
        // Targets a damaging move struck (HP or Substitute), in hit order, once each.
        let mut struck: Vec<PokemonId> = Vec::new();
        for hit in hits {
            if let HitOutcome::Damaged { target, .. } | HitOutcome::SubstituteDamaged { target, .. } =
                hit
                && !struck.contains(target)
            {
                struck.push(*target);
            }
        }

        // Sheer Force keeps every `AfterMoveSecondary` event from running, the targets' and the
        // user's (`move.hasSheerForce && pokemon.hasAbility('sheerforce')`). Fell Stinger's own
        // has no secondaries to lose.
        let secondary_events = !move_def.flags.sheer_force;

        // Showdown `onAfterMoveSecondary` on every Pokémon the move reached, once per move: the
        // item reactions to a damaging move's hit on the HP (Eject Button, Red Card, Kee Berry),
        // and the abilities (Berserk, Anger Shell, Pickpocket) — Eject Button (priority 2) before
        // the ability, Red Card and the berries (0) after it, the ability's effect type sorting
        // first.
        if secondary_events {
            let damage_to = |target: PokemonId| {
                hits.iter()
                    .map(|hit| match hit {
                        HitOutcome::Damaged { target: t, amount } if *t == target => amount.0,
                        _ => 0,
                    })
                    .fold(0u16, u16::saturating_add)
            };
            // A target whose last hit struck its Substitute is not among them (Showdown
            // `spreadMoveHit` nulls it in `targetsCopy`): Color Change, Berserk and the items
            // never see a hit their Substitute took (DESIGN.md R263).
            let mut reached: Vec<PokemonId> = Vec::new();
            for hit in hits {
                if let HitOutcome::Damaged { target, .. }
                | HitOutcome::SubstituteDamaged { target, .. }
                | HitOutcome::Effect { target } = hit
                    && !reached.contains(target)
                {
                    reached.push(*target);
                }
            }
            reached.retain(|target| {
                !matches!(
                    hits.iter().rev().find(|hit| matches!(
                        hit,
                        HitOutcome::Damaged { target: t, .. }
                            | HitOutcome::SubstituteDamaged { target: t, .. }
                            | HitOutcome::Effect { target: t } if t == target
                    )),
                    Some(HitOutcome::SubstituteDamaged { .. })
                )
            });
            let dealt = crate::effects::abilities::MoveDamage {
                attacker: actor,
                move_type: move_def.move_type,
                category: move_def.category,
                to_holder: 0,
                total: reached
                    .iter()
                    .map(|t| damage_to(*t))
                    .fold(0, u16::saturating_add),
                multi_hit: move_def.multi_strike(),
            };
            for target in reached {
                // Gen 4 Focus Sash's condition (`onAfterMoveSecondary`, a condition's handler
                // first): the sash that held a hit goes now.
                if self.focus_items.iter().any(|armed| {
                    armed.holder == target
                        && armed.held
                        && armed.how == crate::effects::items::HoldsForTheMove::FromFullHp
                }) {
                    BattleContext::new(self.state, self.rules, self.trace).consume_item(target);
                }
                let took_hp = hits.iter().any(
                    |hit| matches!(hit, HitOutcome::Damaged { target: t, .. } if *t == target),
                );
                let item_reaction = crate::engine::held_item(self.state, self.rules, target)
                    .and_then(|(_, item)| item.after_hit)
                    .filter(|_| move_def.category != MoveCategory::Status && took_hp);
                let ability = self
                    .state
                    .pokemon(target)
                    .and_then(|p| crate::engine::ability_running(self.state, self.rules, p));
                let run_item = |pipeline: &mut Self| {
                    if let Some(ordered) = item_reaction {
                        let mut ctx =
                            BattleContext::new(pipeline.state, pipeline.rules, pipeline.trace);
                        (ordered.handler)(
                            &mut ctx,
                            crate::engine::AfterHit {
                                holder: target,
                                attacker: actor,
                                move_id,
                                category: move_def.category,
                            },
                        );
                    }
                };
                let item_first = item_reaction.is_some_and(|ordered| ordered.priority == 0);
                if item_first {
                    run_item(self);
                }
                if let Some(ability) = ability {
                    let behavior = crate::engine::ability_behavior(self.rules, ability);
                    // Pickpocket: not while the holder holds something, has fallen (it left the
                    // field as the faint was processed, before this event), is being dragged
                    // (`forceSwitchFlag`) or leaves by its own Eject Button, which went off first
                    // (`switchFlag`, R-M8-107), nor when the attacker leaves by its own Emergency
                    // Exit (`switchFlag === true`; a U-turn's is the move's id). The move's own
                    // contact flag (`move.flags['contact']`; Long Reach takes it away): the
                    // attacker's Protective Pads can be stolen (Sword / Shield research, R-M8-112).
                    let pickpocket = behavior.pickpocket
                        && move_def.flags.contact
                        && target != actor
                        && self
                            .state
                            .pokemon(target)
                            .is_some_and(|p| p.held_item.is_none() && !p.is_fainted())
                        && !self.state.drag_queue.contains(&target)
                        && !self.leaving(target)
                        && !(move_def.self_switch.is_none() && self.leaving(actor));
                    let mut ctx =
                        BattleContext::new(self.state, self.rules, self.trace).with_dex(self.dex);
                    crate::effects::abilities::after_move_secondary(
                        &mut ctx,
                        behavior,
                        target,
                        crate::effects::abilities::MoveDamage {
                            to_holder: damage_to(target),
                            ..dealt
                        },
                    );
                    if pickpocket {
                        ctx.pickpocket(target, actor, move_id);
                    }
                }
                if !item_first {
                    run_item(self);
                }
            }
        }

        // Emergency Exit / Wimp Out on each target the move hurt, once the `AfterMoveSecondary`
        // events are over — not after a move Sheer Force boosted (`hitStepMoveHitLoop`).
        if secondary_events {
            for target in struck {
                let hurt = hits.iter().any(
                    |hit| matches!(hit, HitOutcome::Damaged { target: t, .. } if *t == target),
                );
                if hurt
                    && let Some(&(_, before)) =
                        self.hp_before_hits.iter().find(|(id, _)| *id == target)
                {
                    let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                    ctx.emergency_exit(target, before);
                }
            }
        }
    }

    /// The after-move steps of one move, in Showdown's order: U-turn's switch order, Dragon
    /// Tail's drag, Eject Button / Red Card on each struck target, then the user's Life Orb.
    /// Gen 4 Bide's release, which `after_move` never sees (the action stops): a held
    /// `ModifyDamagePhase2` item (Life Orb) still takes its after-move turn when the release took
    /// real HP — pokeplatinum's after-hit check follows it as any hit (issues.md I193).
    pub(super) fn phase2_item_after_release(
        &mut self,
        actor: PokemonId,
        move_id: crate::ids::MoveId,
        hits: &[HitOutcome],
    ) {
        if !self
            .rules
            .damage
            .modifier_order
            .contains(&DamageModifierKind::ModifyDamagePhase2)
            || !hits
                .iter()
                .any(|hit| matches!(hit, HitOutcome::Damaged { amount, .. } if amount.0 > 0))
        {
            return;
        }
        let Some(handler) = crate::engine::held_item(self.state, self.rules, actor)
            .filter(|(_, item)| item.damage_phase2.is_some())
            .and_then(|(_, item)| item.after_move_self)
        else {
            return;
        };
        let Some(move_def) = self.dex.move_by_id(move_id).cloned() else {
            return;
        };
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        handler(&mut ctx, actor, &move_def);
    }

    pub(super) fn after_move(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        hits: &[HitOutcome],
        any_landed_hit: bool,
    ) {
        let move_id = move_def.id;
        // U-turn / Volt Switch / Baton Pass: a landed hit orders the user out. Whether anyone
        // can come in is decided at the request boundary (`BattleEngine::step`), not here.
        if let Some(self_switch) = move_def.self_switch
            && any_landed_hit
            && let Some(side) = self.state.pokemon(actor).map(|p| p.side)
            && self.state.pokemon(actor).is_some_and(|p| !p.is_fainted())
        {
            let side = self.state.side_mut(side);
            side.switch_out = Some(actor);
            side.switch_out_by_move = true;
            side.passing = match self_switch {
                crate::dex::SelfSwitch::Pass(kind) => Some(kind),
                crate::dex::SelfSwitch::Plain => None,
            };
            self.trace
                .push(TraceEvent::SwitchOutRequested { actor, move_id });
        }

        self.after_move_secondary(actor, move_def, hits);
        let secondary_events = !move_def.flags.sheer_force;

        // Showdown `useMoveInner`: `selfBoost` once the hit steps are over, if the move did
        // something (Clanging Scales, Scale Shot).
        if any_landed_hit {
            self.apply_secondary_effects(
                actor,
                actor,
                None,
                move_def,
                SecondaryTrigger::AfterMove,
                false,
            );
        }

        // `onAfterMoveSecondarySelf`, the move's own before the user's item's: Fell Stinger
        // raises the user's stat when its target is down.
        if any_landed_hit
            && let Some(stat) = move_def.behavior.knock_out_boost
            && self.state.pokemon(actor).is_some_and(|p| !p.is_fainted())
            && hits
                .iter()
                .find_map(|hit| match hit {
                    HitOutcome::Damaged { target, .. }
                    | HitOutcome::SubstituteDamaged { target, .. } => Some(*target),
                    _ => None,
                })
                .and_then(|target| self.state.pokemon(target))
                .is_some_and(|target| target.is_fainted())
        {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
                target: actor,
                stat,
                stages: self.rules.hits.knock_out_boost_stages,
                source: crate::engine::EffectSource::Move {
                    pokemon: actor,
                    move_id,
                },
                cause: crate::engine::StatChangeCause::Move,
            });
        }

        // Order Up (its own `onAfterMoveSecondarySelf`): the commanded Dondozo raises a stat by
        // the forme of the Tatsugiri in its mouth (DESIGN.md §43.4).
        if any_landed_hit && move_def.behavior.order_up {
            BattleContext::new(self.state, self.rules, self.trace).order_up(actor);
        }

        // Relic Song (its own `onAfterMoveSecondarySelf`, which Sheer Force keeps from running):
        // Meloetta changes forme once the move did something (DESIGN.md §43.4).
        if any_landed_hit
            && secondary_events
            && let Some(formes) = move_def.behavior.forme_toggle
        {
            BattleContext::new(self.state, self.rules, self.trace).toggle_forme(
                actor,
                formes,
                crate::engine::EffectSource::Move {
                    pokemon: actor,
                    move_id,
                },
            );
        }

        // The user's item after a damaging move that landed (Showdown
        // `onAfterMoveSecondarySelf`: Life Orb's recoil), once per move.
        // Gen 4 (a `ModifyDamagePhase2` item; pokeplatinum `AFTER_MOVE_HIT_STATE_LIFE_ORB`): the
        // item held once the move is over, when a hit of it took real HP (`SYSCTL_MOVE_HIT`, which
        // a Substitute's hit never sets) — fixed damage and one-hit KOs too (issues.md I193).
        let body_hit = hits
            .iter()
            .any(|hit| matches!(hit, HitOutcome::Damaged { amount, .. } if amount.0 > 0));
        let phase2_item =
            crate::engine::held_item(self.state, self.rules, actor).is_some_and(|(_, item)| {
                item.damage_phase2.is_some()
                    && self
                        .rules
                        .damage
                        .modifier_order
                        .contains(&DamageModifierKind::ModifyDamagePhase2)
            });
        let user_hp = self.state.pokemon(actor).map(|p| p.hp);
        // The event's handlers are gathered before any runs: an item Magician takes now has
        // none this time (its Life Orb, its Shell Bell, its Throat Spray).
        let held = crate::engine::held_item(self.state, self.rules, actor);
        let item_after_self = held.and_then(|(_, item)| item.after_move_self);
        let shell_bell = held.is_some_and(|(_, item)| item.heals_off_damage_dealt.is_some());
        let throat_spray = held.is_some_and(|(_, item)| item.used_after_a_sound_move.is_some());
        // Magician (the ability's `onAfterMoveSecondarySelf`, before the item's at the same
        // priority): an empty-handed user still standing that used no gem takes an item from the
        // Pokémon its damaging move hit — not after a Fling, whose thrower is empty-handed by
        // then (`move.id === 'fling'`, R-M8-102). (Showdown also stops for a user with
        // `switchFlag === true`, which only an Emergency Exit or an Eject Pack of its own would
        // set.)
        if move_def.category != MoveCategory::Status
            && any_landed_hit
            && secondary_events
            && self.gem.is_none()
            && !move_def.behavior.fling
            && self.state.pokemon(actor).is_some_and(|user| {
                user.held_item.is_none()
                    && !user.is_fainted()
                    && crate::engine::ability_handler(self.state, self.rules, user, |b| b.magician)
            })
        {
            let mut hit: Vec<PokemonId> = Vec::new();
            for outcome in hits {
                if let HitOutcome::Damaged { target, .. }
                | HitOutcome::SubstituteDamaged { target, .. } = outcome
                    && !hit.contains(target)
                {
                    hit.push(*target);
                }
            }
            // Dragon Darts split over two targets weighs both, faster first — the user's decision
            // (issues.md I200, R-M8-170; Showdown's `move.hitTargets` keeps only the second dart's,
            // which the oracle patches).
            BattleContext::new(self.state, self.rules, self.trace).magician(actor, &hit, move_id);
        }
        // Not a move aimed at its user (`source !== target`: Bide's first turn, R-M8-123).
        if move_def.category != MoveCategory::Status
            && move_def.target != crate::rules::TargetPattern::SelfOnly
            && any_landed_hit
            && secondary_events
            && (!phase2_item || body_hit)
            && let Some(handler) = item_after_self
        {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            handler(&mut ctx, actor, move_def);
        }
        // Shell Bell (`onAfterMoveSecondarySelf`, priority −1), unless the user is being dragged
        // out (Showdown `forceSwitchFlag`: Red Card) — issues.md I47 for what it counts.
        if shell_bell
            && any_landed_hit
            && secondary_events
            && !self.state.drag_queue.contains(&actor)
        {
            let multi_hit = move_def.multi_strike();
            let mut to_hp = 0u16;
            let mut each = Vec::new();
            for hit in hits {
                match *hit {
                    HitOutcome::Damaged { target, amount } if target != actor => {
                        to_hp = to_hp.saturating_add(amount.0);
                        each.push(amount.0);
                    }
                    HitOutcome::SubstituteDamaged { target, amount, .. } if target != actor => {
                        each.push(amount.0);
                    }
                    _ => {}
                }
            }
            let dealt = match self.rules.conditions.shell_bell {
                crate::rules::ShellBellCount::Move => vec![to_hp],
                crate::rules::ShellBellCount::MoveWithSubstitutes => {
                    vec![each.iter().fold(0u16, |sum, d| sum.saturating_add(*d))]
                }
                // A multi-hit move's hits were healed one by one as they landed.
                crate::rules::ShellBellCount::EachHit if multi_hit => Vec::new(),
                crate::rules::ShellBellCount::EachHit => each,
            };
            self.shell_bell(actor, &dealt);
        }
        // Throat Spray (`onAfterMoveSecondarySelf`): after any move of the user's that did
        // something, a status move included, the event Sheer Force did not cancel.
        if throat_spray && any_landed_hit && secondary_events && move_def.flags.sound {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            crate::effects::items::hit_items::use_after_a_sound_move(&mut ctx, actor);
        }
        // The user's own Emergency Exit after `AfterMoveSecondarySelf` (Life Orb's recoil).
        if move_def.category != MoveCategory::Status
            && any_landed_hit
            && secondary_events
            && let Some(before) = user_hp
        {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            ctx.emergency_exit(actor, before);
        }
    }
}
