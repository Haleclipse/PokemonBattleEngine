//! `run_move`'s hits (DESIGN.md §15, §48.3): every target through the hit gate and the primary
//! effect — target by target, or a spread move's hit steps over all of them first (Showdown
//! `trySpreadMoveHit`, `HitRules::staged_spread_hits`), Dragon Darts split between two, a
//! custom primary's targets — and what the hits did.

use super::*;

impl MovePipeline<'_> {
    /// The move's hits on its targets, in the generation's order of hit steps: Dragon Darts
    /// split between two targets; the hit steps of a move with several targets over all of
    /// them first (Gen 4+) or target by target; the custom primary's targets at the end.
    pub(super) fn hit_batch(
        &mut self,
        actor: PokemonId,
        move_id: MoveId,
        move_def: &ResolvedMove,
        targets: &crate::rules::ResolvedTargets,
        smart: bool,
        pokemon_target_count: usize,
    ) -> Result<HitBatch, BattleError> {
        // Unified hit loop. Every move, damaging or custom, sends each foreign Pokémon target
        // through the same gate (immunity pre-check for damaging primaries, TryHit, accuracy)
        // and then dispatches its primary effect. Custom primaries collect the targets that
        // survived the gate and run once after the loop, because Side / Field semantics are
        // naturally whole-move operations. No move gets to re-implement any part of this.
        let mut hits = Vec::with_capacity(targets.targets.len());
        let mut custom_targets = Vec::new();
        // The move's own `self` is yet to land (Showdown's `selfDropped`, a property of the move).
        self.self_dropped = false;
        // A spread move's hit runs stage by stage over every target that passed the gate
        // (`apply_spread_hit`), after all the gates (Showdown checks every target's accuracy
        // before any damage). Gen 1-3 hit each target in full before the next one's gate.
        let spread = self.rules.hits.staged_spread_hits
            && move_def.primary_effect == PrimaryEffect::Damage
            && move_def.multi_hit == crate::dex::MultiHit::Single
            && targets
                .targets
                .iter()
                .filter(|t| matches!(t, ResolvedTarget::Pokemon(p) if *p != actor))
                .count()
                > 1;
        let mut spread_targets = Vec::new();
        let mut any_landed_hit = false;
        // A whole-move operation (a custom primary, a weather move) that changed nothing.
        let mut move_failed = false;
        // A custom primary that went through on a Pokémon (Showdown runs a side or field move
        // without the hit loop, so without its `Update`s).
        let mut custom_hit_pokemon = false;

        // Dragon Darts split between two targets: both targets' hit steps first; one failing
        // ends the split (`smartTarget = false`) and both darts go to the other, one dart
        // each when both pass.
        if smart {
            // The split's first protection or Wonder Guard refusal is silent (`smart_quiet`) —
            // unless a target out of reach failed the first hit step already (Showdown runs each
            // step over both targets, so that comes before either target's TryHit).
            self.smart_quiet = !targets.pokemon_ids().any(|target| {
                crate::engine::two_turn::dodged_by(self.state, self.rules, actor, target, move_def)
                    .is_some()
            });
            let mut passed = Vec::new();
            for target in targets.pokemon_ids() {
                match self.resolve_hit_gate(actor, target, move_def) {
                    Ok(modifier) => passed.push((target, modifier)),
                    Err(outcome) => hits.push(outcome),
                }
            }
            self.smart_quiet = false;
            self.smart_hit = passed.len() > 1;
            for (target, type_effectiveness) in passed {
                let outcomes =
                    self.apply_damage_sequence(actor, target, move_def, 1, type_effectiveness);
                any_landed_hit |= outcomes
                    .iter()
                    .any(|outcome| !matches!(outcome, HitOutcome::NoEffect { .. }));
                self.record_attack(actor, target, move_def, &outcomes);
                hits.extend(outcomes);
            }
            self.smart_hit = false;
        }
        let unsplit: &[ResolvedTarget] = if smart { &[] } else { &targets.targets };
        let foreign =
            |target: &ResolvedTarget| matches!(target, ResolvedTarget::Pokemon(p) if *p != actor);
        // Showdown's `trySpreadMoveHit` (`HitRules::staged_spread_hits`) takes each hit step over
        // all of a move's targets: where TryHit comes before the accuracy roll (Gen 5+), every
        // target's steps up to TryHit — a reflection with them — come before any target's
        // accuracy (R-M8-77). Gen 4 rolls the accuracy first: target by target here (R-M8-78).
        let try_hit_first = matches!(
            self.rules.hits.step_order,
            crate::rules::HitStepOrder::ImmunityFirst | crate::rules::HitStepOrder::TryHitFirst
        );
        let mut past_try_hit = vec![None; unsplit.len()];
        // Any move with several targets, the user among them too (Life Dew: its ally's Water
        // Absorb takes the move at TryHit before the user's own heal, R-M9-37).
        if self.rules.hits.staged_spread_hits
            && try_hit_first
            && unsplit.len() > 1
            && unsplit.iter().any(foreign)
        {
            for (index, target) in unsplit.iter().enumerate() {
                let ResolvedTarget::Pokemon(pokemon) = *target else {
                    continue;
                };
                if pokemon == actor {
                    continue;
                }
                let past = self.hit_gate_through_try_hit(actor, pokemon, move_def);
                // Showdown runs the reflection inside the target's `TryHit` (§42.8): the move
                // goes on to its other targets once the reflected one is over (R-M8-68). A
                // reflection with no other target left takes the action over as its called move.
                if matches!(past, Err(HitOutcome::Reflected { .. }))
                    && (past_try_hit[..index]
                        .iter()
                        .any(|p| matches!(p, Some(Ok(_))))
                        || unsplit[index + 1..].iter().any(foreign))
                {
                    self.run_reflection_now()?;
                }
                past_try_hit[index] = Some(past);
            }
        }
        for (index, target) in unsplit.iter().enumerate() {
            let type_effectiveness = match *target {
                ResolvedTarget::Pokemon(pokemon) if pokemon != actor => {
                    let gate = match past_try_hit[index].take() {
                        Some(past) => past.and_then(|past| {
                            self.hit_gate_after_try_hit(actor, pokemon, move_def, past)
                        }),
                        None => self.resolve_hit_gate(actor, pokemon, move_def),
                    };
                    match gate {
                        Ok(modifier) => modifier,
                        Err(outcome) => {
                            let reflected = matches!(outcome, HitOutcome::Reflected { .. });
                            hits.push(outcome);
                            // A reflection still waiting (the target by target gate, or the
                            // staged one's last) takes the action over as its called move, or
                            // runs now when another target is left to hit.
                            if reflected && self.state.pending_call.is_some() {
                                if unsplit[index + 1..].iter().any(foreign) {
                                    self.run_reflection_now()?;
                                } else {
                                    break;
                                }
                            }
                            continue;
                        }
                    }
                }
                // The user's own body, its side and the field never protect against the user, so
                // these targets skip the gate — but for a hiding user missing itself (Gen 2+,
                // `TwoTurnRules::hides_from_itself`: a Metronome Encore keeps on the turn of the
                // Bounce it called, R-M10-50), its side's Quick Guard (`own_quick_guard`), and
                // `hitStepAccuracy` still runs the `Accuracy` event for a move on its user unless
                // it is a status move aimed at the user alone (`move.target === 'self'`, the user
                // not semi-invulnerable): Bide, and Acupressure (`adjacentAllyOrSelf`) or Howl
                // (`allies`) on the user, spend its Micle Berry (R-M8-105).
                ResolvedTarget::Pokemon(pokemon) => {
                    if self.rules.two_turn.hides_from_itself
                        && let Err(outcome) = self.invulnerability_step(actor, pokemon, move_def)
                    {
                        hits.push(outcome);
                        continue;
                    }
                    if let Some(outcome) = self.own_quick_guard(pokemon, move_def) {
                        hits.push(outcome);
                        continue;
                    }
                    let skips_accuracy = move_def.category == crate::dex::MoveCategory::Status
                        && move_def.target == crate::rules::TargetPattern::SelfOnly
                        && self
                            .state
                            .pokemon(pokemon)
                            .is_none_or(|user| crate::engine::two_turn::hiding(user).is_none());
                    if !skips_accuracy {
                        self.accuracy_event(actor, pokemon, move_def);
                    }
                    self.type_effectiveness(pokemon, move_def)
                }
                // `TryHitSide` (Gen 5+): a move at the foes' side that one of them reflects
                // becomes the action's called move, used back at the user's side (R-M8-77).
                ResolvedTarget::Side(side) => {
                    if let Some(reflector) = self.reflect_side_move(actor, side, move_def) {
                        hits.push(HitOutcome::Reflected { target: reflector });
                        break;
                    }
                    Fraction::ONE
                }
                ResolvedTarget::Field => Fraction::ONE,
            };

            match (&move_def.primary_effect, *target) {
                (PrimaryEffect::Status(effect), ResolvedTarget::Pokemon(pokemon)) => {
                    let (outcome, reached) =
                        self.apply_status_primary(actor, pokemon, move_def, effect);
                    let landed = matches!(outcome, HitOutcome::Effect { .. });
                    any_landed_hit |= landed;
                    // Showdown `gotAttacked` where the move did something to the target, or reached
                    // it with nothing to do (a Heal Block restarted, R-M8-108). A move that failed
                    // there records nothing — Gen 3 too (pokeemerald's `MOVEEND_MIRROR_MOVE`
                    // skips `MOVE_RESULT_NO_EFFECT`, a Growl at -6 or a failed Mimic; issues.md
                    // I173, R-M8-143).
                    if reached {
                        self.record_attack(
                            actor,
                            pokemon,
                            move_def,
                            &[HitOutcome::Effect { target: pokemon }],
                        );
                    }
                    hits.push(outcome);
                    // Memento (`selfdestruct: 'ifHit'`): the target was reached, whatever its
                    // stats did.
                    self.self_destruct_if_hit(actor, move_def, true);
                }
                (PrimaryEffect::Damage, ResolvedTarget::Pokemon(pokemon)) if spread => {
                    spread_targets.push((pokemon, type_effectiveness));
                }
                (PrimaryEffect::Damage, ResolvedTarget::Pokemon(pokemon)) => {
                    let outcomes = self.apply_damage_sequence(
                        actor,
                        pokemon,
                        move_def,
                        pokemon_target_count,
                        type_effectiveness,
                    );
                    // A hit that did nothing (Present healing a full-HP target) lands nothing,
                    // nor does one the type chart took to 0 (Gen 1's miss, issues.md I221).
                    let landed = outcomes.iter().any(|outcome| {
                        !matches!(
                            outcome,
                            HitOutcome::NoEffect { .. } | HitOutcome::Missed { .. }
                        )
                    });
                    any_landed_hit |= landed;
                    self.record_attack(actor, pokemon, move_def, &outcomes);
                    hits.extend(outcomes);
                    self.self_destruct_if_hit(actor, move_def, landed);
                }
                // A weather move is a field-targeted status primary: one request on the field.
                (PrimaryEffect::Status(StatusMoveEffect::Weather(kind)), ResolvedTarget::Field) => {
                    let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
                    if ctx.set_weather_by_move(*kind, actor) {
                        any_landed_hit = true;
                    } else {
                        move_failed = true;
                        self.trace.push(TraceEvent::MoveFailed {
                            actor,
                            move_id,
                            reason: MoveFailReason::NoEffect,
                        });
                    }
                }
                // Other damage / status primaries have no meaning for a side or the field.
                (PrimaryEffect::Damage | PrimaryEffect::Status(_), _) => {}
                (PrimaryEffect::Custom, _) => custom_targets.push(*target),
                // Rejected above; kept exhaustive so a new variant cannot fall through silently.
                (PrimaryEffect::Unsupported, _) => {}
            }
        }

        if !spread_targets.is_empty() {
            let outcomes =
                self.apply_spread_hit(actor, &spread_targets, move_def, pokemon_target_count);
            any_landed_hit = true;
            self.self_destruct_if_hit(actor, move_def, true);
            for (target, outcome) in outcomes {
                self.record_attack(actor, target, move_def, &[outcome]);
                hits.push(outcome);
            }
        }

        if move_def.primary_effect == PrimaryEffect::Custom && !custom_targets.is_empty() {
            if let Some(user_side) = self.state.pokemon(actor).map(|pokemon| pokemon.side)
                && custom_targets.contains(&ResolvedTarget::Side(user_side))
            {
                BattleContext::new(self.state, self.rules, self.trace)
                    .ally_try_hit_side(actor, move_def);
            }
            let result = {
                let mut ctx =
                    BattleContext::new(self.state, self.rules, self.trace).with_dex(self.dex);
                effects::moves::execute_custom(move_id, &mut ctx, actor, &custom_targets)
            };
            // `gotAttacked` for the Pokémon it was used on where it worked (a failed Mimic or
            // Sketch records nothing, Gen 3 too: issues.md I173, R-M8-143).
            if result.is_ok() {
                for target in custom_targets.iter() {
                    if let ResolvedTarget::Pokemon(pokemon) = *target {
                        self.record_attack(
                            actor,
                            pokemon,
                            move_def,
                            &[HitOutcome::Effect { target: pokemon }],
                        );
                    }
                }
            }
            match result {
                Ok(()) => {
                    any_landed_hit = true;
                    custom_hit_pokemon = custom_targets
                        .iter()
                        .any(|target| matches!(target, ResolvedTarget::Pokemon(_)));
                    // Healing Wish, Lunar Dance (`selfdestruct: 'ifHit'`).
                    self.self_destruct_if_hit(actor, move_def, true);
                }
                Err(error) => {
                    move_failed = true;
                    self.trace.push(TraceEvent::MoveFailed {
                        actor,
                        move_id,
                        reason: MoveFailReason::Condition(error),
                    });
                    // A wish already waiting at the slot: the move reached its user and did
                    // nothing, so it failed (no Dancer), but `selfdestruct: 'ifHit'` asks only
                    // for the hit (Showdown `runMoveEffects`, R-M8-177).
                    if error == crate::engine::ConditionApplyError::AlreadyActive {
                        self.self_destruct_if_hit(actor, move_def, true);
                    }
                }
            }
        }
        Ok(HitBatch {
            hits,
            any_landed_hit,
            move_failed,
            custom_hit_pokemon,
        })
    }
}
