//! The random plan of one action (`tape_template`): every draw it makes, in the engine's order,
//! read off the oracle's log.

use std::collections::BTreeMap;
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, FollowUp, MoveCategory, MultiHit, Nature, PokemonSet, PrimaryEffect, ResolvedDex,
    ResolvedMove, SecondaryEffect, SecondaryTarget, SecondaryTrigger, StatSpread, StatusMoveEffect,
    build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, BattleSnapshot, EngineStep, ForcedReplacement, ForcedSwitchChoice,
    MechanicChoice, PriorityChance, ReplacementRequirement, ReviveChoice, SideChoice, SlotChoice,
    TraceEvent, two_turn,
};
use pokemon_battle_engine::format::{FormatLibrary, Stats, TeamSet};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, HitStepOrder, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::session::{OpenTeamSheets, SessionFormat, TeamChoice, TeamSlot};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    ActiveMove, BattleRng, BattleState, Gender, NonVolatileStatus, RandomOverride,
    RandomRequestKind, SideState, VolatileCondition, VolatileConditionKind,
};
use serde::Deserialize;
use serde_json::Value;

use crate::support::{capture, compare_json};

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn tape_template(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    actor: PokemonId,
    move_def: Option<&ResolvedMove>,
    fates: &[(Option<PokemonId>, TargetFate)],
    reflections: &[Reflection],
    oracle: &ActionOracle,
    duration_hints: &[(PokemonId, DurationKind, u16)],
    substitutes: &[(PokemonId, Option<u16>)],
) -> Vec<Option<RandomOverride>> {
    // The duration recovered for a victim; a target the replay cannot name takes the first.
    // One nothing shows (the victim was dragged out, cured or knocked out before the frame) is
    // still rolled: any legal value fits what was observed, so it takes the shortest.
    let duration_for = |target: Option<PokemonId>, class: DurationKind| {
        duration_hints
            .iter()
            .find(|(victim, c, _)| *c == class && target.is_none_or(|t| t == *victim))
            .map(|(_, _, hint)| *hint)
            .or(match class {
                DurationKind::Confusion => {
                    Some(u16::from(rules.status().lifecycle.confusion_time_min))
                }
                DurationKind::Sleep => Some(u16::from(rules.status().lifecycle.sleep_time_min)),
                DurationKind::Bind => Some(u16::from(rules.conditions().bind_turns_min)),
                DurationKind::BindingLock => Some(2),
                DurationKind::Bide => Some(u16::from(rules.conditions().retaliation.bide.turns.0)),
                DurationKind::Rampage => Some(2),
                DurationKind::Uproar => Some(u16::from(rules.conditions().uproar.turns.0)),
                DurationKind::Timed => None,
            })
    };
    // Gen 3+ Thrash's lock ending with its move: the fatigue's confusion duration.
    let fatigue_time = || {
        (oracle.fatigue && !rules.conditions().rampage.counter.counts_uses())
            .then(|| duration_for(Some(actor), DurationKind::Confusion))
            .flatten()
    };
    // A Z status move's effect comes before its hit steps (Showdown `runZPower` in
    // `useMoveInner`): Z-Attract's `clearnegativeboost` puts the user's lowered stages back to
    // 0, and the accuracy the move then checks reads them (R-M11-81).
    let z_cleared;
    let state = if oracle.z_status
        && move_def.is_some_and(|m| {
            m.z_status == Some(pokemon_battle_engine::dex::ZStatusEffect::ClearNegativeBoost)
        }) {
        let mut cleared = state.clone();
        if let Some(user) = cleared.pokemon_mut(actor) {
            for stat in pokemon_battle_engine::engine::STAGED_STATS {
                if user.stages.get(stat) < 0 {
                    user.stages.set(stat, 0);
                }
            }
        }
        z_cleared = cleared;
        &z_cleared
    } else {
        state
    };
    let mut tape = Vec::new();
    let Some(pokemon) = state.pokemon(actor) else {
        return tape;
    };
    let cant = oracle.cant.as_deref();
    // The state the move's own checks see (`move_under_way`), with the status the actor shook
    // off as its action began gone (a Gen 2-3 Beat Up counts it a member again, R-M8-134).
    let in_move_state = move_def.map(|m| {
        let mut in_move = move_under_way(state, rules, actor, m);
        if (oracle.cured.is_some() || oracle.defrosted)
            && let Some(user) = in_move.pokemon_mut(actor)
        {
            user.status = None;
        }
        in_move
    });
    let in_move = in_move_state.as_ref().unwrap_or(state);
    // The eaters whose berry draw a hit already planned.
    let mut berry_confusions_planned: Vec<Option<PokemonId>> = Vec::new();

    // Gen 3-4 Pursuit striking a Pokémon as it switches out runs without BeforeMove (§15.11):
    // Showdown's gen3 / gen4 Pursuit condition calls `useMove`, not `runMove`. Read from the
    // generation, not the engine's rule table, so that the replay checks the rule.
    let skips_before_move = oracle.intercepted
        && matches!(
            rules.generation(),
            pokemon_battle_engine::spec::Generation::Gen3
                | pokemon_battle_engine::spec::Generation::Gen4
        );
    if !skips_before_move {
        // 0. The recharge turn comes before any status check (`mustrecharge` `onBeforeMovePriority`
        //    11 over `frz`'s 10; Gen 1's freeze never thaws on its own): it asks nothing. Gen 3-4
        //    on the cartridge check it after sleep and freeze (issues.md I66): step 2.
        if cant == Some("recharge")
            && rules.conditions().recharge_check
                != pokemon_battle_engine::rules::RechargeCheck::AfterSleepAndFreeze
        {
            return tape;
        }
        // 1. Major status. Sleep asks nothing; freeze asks Thaw. Waking with no move and no other
        //    reason (Gen 1) spends the turn; a Pokémon that wakes then meets its confusion check.
        if let Some(status) = pokemon.status {
            match status.kind {
                NonVolatileStatus::Sleep => {
                    if cant == Some("slp")
                        || (oracle.cured.as_deref() == Some("slp")
                            && oracle.move_name.is_none()
                            && cant.is_none())
                    {
                        return tape;
                    }
                }
                NonVolatileStatus::Freeze => {
                    let thawed = oracle.cured.as_deref() == Some("frz") && !oracle.defrosted;
                    // The engine says whether a thaw is rolled here (Gen 2: in the residual; a
                    // `defrost` move from Gen 5: no roll).
                    let rolls = match move_def {
                        Some(m) => {
                            pokemon_battle_engine::engine::freeze_thaw_rolls(state, rules, actor, m)
                        }
                        None => {
                            rules.status().lifecycle.thaw_timing
                                == pokemon_battle_engine::rules::ThawTiming::BeforeMove
                        }
                    };
                    if rolls {
                        tape.push(Some(RandomOverride::bool(RandomRequestKind::Thaw, thawed)));
                    }
                    let defrost_moves = move_def.is_some_and(|m| {
                        pokemon_battle_engine::engine::defrost_move_for(state, rules, actor, m)
                    }) && rules.status().lifecycle.defrost_user
                        != pokemon_battle_engine::rules::DefrostUser::Never;
                    if !thawed && !defrost_moves {
                        return tape;
                    }
                }
                _ => {}
            }
        }
        // 2. Flinch asks nothing; neither do the recharge turn, Disable, Taunt or a Choice lock
        //    (`cant … recharge`, `cant … Disable`, `cant … move: Taunt`). A move with no PP left
        //    (`cant … nopp`) passed them all first.
        // An ability that stops the move at TryMove (Damp) comes after BeforeMove and after a
        // call (R-M8-70), and so does Powder blowing up a Fire move (a Metronome's Heat Wave,
        // R-M8-108): below.
        if cant.is_some_and(|reason| {
            !matches!(
                reason,
                "par" | "confusion" | "slp" | "frz" | "Attract" | "nopp" | "move: Powder"
            ) && !TRY_MOVE_STOPPERS.contains(&reason)
        }) {
            return tape;
        }
        // 3. Confusion: after the tick, a remaining counter rolls for a self-hit.
        if pokemon
            .volatiles
            .confusion_time()
            .is_some_and(|time| time > 1)
        {
            let self_hit = cant == Some("confusion");
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::ConfusionSelfHit,
                self_hit,
            )));
            if self_hit {
                if rules.status().lifecycle.confusion_self_hit_varies {
                    tape.push(None);
                }
                // Gen 1-2's self-hit is a `directDamage` no Focus Band sees (read from the
                // generation, so that the replay checks `confusion_self_hit_direct`).
                if let Some(decision) = self_focus_band(state, rules, actor, oracle)
                    && rules.generation() >= Generation::Gen3
                {
                    tape.push(Some(decision));
                }
                // A berry the self-hit's `Update` fed it: Starf Berry's stat (DESIGN.md R281).
                for (who, _, draw) in &oracle.berry_draws {
                    if who_to_id(state, who) == Some(actor)
                        && let Some(decision) = berry_decision(*draw, || {
                            duration_for(Some(actor), DurationKind::Confusion)
                        })
                    {
                        tape.push(Some(decision));
                    }
                }
                return tape;
            }
        }
        // 3a. Attract: an infatuated Pokémon draws before paralysis (Gen 2: after it); `cant …
        //     Attract` is the one that took.
        let infatuated = pokemon.volatiles.contains(VolatileConditionKind::Attract);
        let late_attract = rules.conditions().attract_after_paralysis;
        let love = cant == Some("Attract");
        if infatuated && !late_attract {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::Infatuation,
                love,
            )));
            if love {
                return tape;
            }
        }
        // 4. Full paralysis.
        if pokemon_battle_engine::engine::rolls_full_paralysis(state, rules, pokemon) {
            let fully = cant == Some("par");
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::FullParalysis,
                fully,
            )));
            if fully {
                return tape;
            }
        }
        if infatuated && late_attract {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::Infatuation,
                love,
            )));
            if love {
                return tape;
            }
        }
    }
    // The moves this action called, one after another (Metronome, or Copycat copying a
    // Metronome, §42.1): each caller's draw among its candidates, in the callers' order, whose
    // place the log's called move names.
    tape.extend(called_draws(state, rules, dex, actor, oracle));
    // The called move's target: a caller that names none (Metronome, Assist, Copycat …:
    // Showdown `useMove` without a target, `getRandomTarget`) has it drawn among the adjacent
    // foes, after the draw that picked the move; the `move` line names the one it went to. A
    // caller that names one (a reflection, Me First, Gen 5+ Mirror Move) draws nothing
    // (`engine::called_move_aims_at_random`): an entry planned for it anyway was taken by the
    // next RandomTarget the action drew — a Roar it called dragging in another Pokémon
    // (R-M8-144). The entry stays optional (DESIGN.md R-M8-55). The draw is the last move the
    // action's own user was called to use: a reflection after it (another user's, §42.8) aims
    // where it was told, and its line names another target (R-M8-67). A snatched move is the
    // snatcher's, and Snatch names no target either: the snatcher draws it among its own
    // adjacent foes (Gen 3-4 Psych Up, R-M9-33). The draw is for the move as called, before
    // `ModifyMove` (the dex's: a Metronome's Expanding Force aims at one foe as it is drawn,
    // all of them only once Psychic Terrain spreads it, R-M10-56).
    if let Some(called) = oracle.called.iter().rev().find(|called| {
        called.user == oracle.user || called.from.as_deref() == Some("[from] Snatch")
    }) && called_at_random(rules, dex, called.from.as_deref())
        && let Some(def) = move_def
        && let Some(user) = who_to_id(state, &called.user)
        && let Some(candidates) = pokemon_battle_engine::engine::random_retarget_candidates(
            state,
            rules,
            user,
            dex.move_by_id(def.id).unwrap_or(def),
            None,
        )
        && candidates.len() > 1
    {
        let picked = drawn_target(
            state,
            &candidates,
            called.target.as_ref(),
            called.redirected_by.as_ref(),
        )
        .unwrap_or(0);
        tape.push(Some(
            RandomOverride::range(RandomRequestKind::RandomTarget, picked as u16).optional(),
        ));
    }
    // A move with no PP left as its turn came (§15.15) stops right after BeforeMove; one an
    // ability or Powder stops at TryMove (Damp: a called Explosion too) after its call.
    if cant.is_some_and(|reason| {
        reason == "nopp" || reason == "move: Powder" || TRY_MOVE_STOPPERS.contains(&reason)
    }) {
        return tape;
    }
    // Snatch (`onAnyPrepareHit`): the holders sharing a handler speed draw a SpeedTie each, in
    // party order (`snatch_steals`); the one whose steal took the move — the first called move's
    // other user — draws the larger (DESIGN.md R-M8-93).
    if let Some(def) = move_def
        && def.flags.snatch
        && let Some(snatcher) = oracle
            .called
            .iter()
            .find(|called| called.user != oracle.user)
            .and_then(|called| who_to_id(state, &called.user))
    {
        let holders: Vec<(PokemonId, u32)> = state
            .all_pokemon()
            .iter()
            .filter(|pokemon| {
                pokemon.id() != actor
                    && !pokemon.is_fainted()
                    && state
                        .active_position(pokemon.id(), rules.topology())
                        .is_some()
                    && pokemon
                        .volatiles
                        .contains(pokemon_battle_engine::state::VolatileConditionKind::Snatch)
            })
            .map(|pokemon| (pokemon.id(), state.handler_speed(pokemon.id())))
            .collect();
        if holders.iter().any(|(id, _)| *id == snatcher) {
            for &(id, speed) in &holders {
                if holders.iter().filter(|(_, other)| *other == speed).count() > 1 {
                    let rank = if id == snatcher { 2 } else { 1 };
                    tape.push(Some(
                        RandomOverride::raw(RandomRequestKind::SpeedTie, rank).optional(),
                    ));
                }
            }
        }
    }
    // A reflection (the last called move another Pokémon used, §42.8): where the generation
    // rolls accuracy before TryHit (Gen 3-4, `HitStepOrder`), the move it sends back rolled its
    // accuracy at the reflector first, and hit (R-M8-69).
    // A spread move some of whose targets reflected it (`reflections`): the user's check at each
    // reflector comes in its turn among the targets, right before the reflection's (the status
    // moves' accuracy loop, R-M8-173, R-M8-176).
    let accuracy_before_try_hit = matches!(
        rules.hits().step_order,
        HitStepOrder::AccuracyFirst | HitStepOrder::AccuracyBeforeTryHit
    );
    let check_at = |reflector: PokemonId| -> Option<Option<RandomOverride>> {
        let def = move_def?;
        (accuracy_before_try_hit && accuracy_in_move(state, rules, actor, reflector, def) < 10_000)
            .then_some(Some(RandomOverride::bool(
                RandomRequestKind::Accuracy,
                true,
            )))
    };
    if reflections.is_empty()
        && let Some(reflection) = oracle
            .called
            .last()
            .filter(|called| called.user != oracle.user)
        && let Some(reflector) = who_to_id(state, &reflection.user)
        && let Some(check) = check_at(reflector)
    {
        tape.push(check);
    }
    // Each reflection strikes with the move as its reflector's (`modified_move`, as below).
    let reflected_moves: Vec<Option<ResolvedMove>> = reflections
        .iter()
        .map(|reflection| {
            move_def.and_then(|def| dex.move_by_id(def.id)).map(|own| {
                pokemon_battle_engine::engine::modified_move(
                    state,
                    rules,
                    reflection.reflector,
                    own,
                    false,
                )
                .0
            })
        })
        .collect();
    let user = actor;
    let own_move_def = move_def;
    // A move another Pokémon took over (a reflection, a snatch: the last called move has another
    // user) is that Pokémon's: its accuracy and the rest read its user's stages (R-M8-81), and
    // the move is the new user's (Showdown `getActiveMove(move.id)` that its own `ModifyMove`
    // shapes: a Turboblaze reflector's move ignores the target's Wonder Skin, R-M8-92).
    let taker = oracle
        .called
        .last()
        .filter(|called| called.user != oracle.user)
        .and_then(|called| who_to_id(state, &called.user));
    let taken_over = taker.zip(move_def).and_then(|(taker, def)| {
        dex.move_by_id(def.id).map(|own| {
            pokemon_battle_engine::engine::modified_move(state, rules, taker, own, false).0
        })
    });
    let move_def = taken_over.as_ref().or(move_def);
    let actor = taker.unwrap_or(actor);
    // 5. The move itself. A charging turn (Showdown `-prepare`) asks nothing more, nor does a
    //    damaging move that failed before hitting (`-fail|user`).
    // Bide's later turns (§15.10): storing asks nothing; the release draws its random target
    // when the Pokémon that damaged the holder last is gone (Gen 1: always), then — in Gen 2-3,
    // through the hit steps — its accuracy.
    if let Some(bide) = pokemon.volatiles.bide().copied() {
        let bide_rules = rules.conditions().retaliation.bide;
        let releases = if bide_rules.gen1 {
            bide.time <= 1
        } else {
            bide.turns == Some(1)
        };
        let stored = if bide_rules.gen1 {
            bide.damage + u32::from(state.last_damage)
        } else {
            bide.damage
        };
        if !releases || stored == 0 || (!bide_rules.gen1 && bide.source.is_none()) {
            return tape;
        }
        let Some(mut release) = dex.move_by_id(MoveId::BIDE).cloned() else {
            return tape;
        };
        release.target = pokemon_battle_engine::rules::TargetPattern::SelectedAdjacent;
        // Whom the release went at: the Pokémon it damaged, missed, or that was immune to it
        // (Wonder Guard on a Gen 3 target the random retarget drew, R-M8-105) — or whose
        // Substitute took it, no HP commit (R-M10-3).
        let struck = oracle
            .damages
            .iter()
            .map(|(who, _)| who)
            .chain(oracle.missed.iter())
            .chain(oracle.immune.iter())
            .chain(oracle.hit_targets.iter().flatten())
            .find_map(|who| who_to_id(state, who));
        let source = bide.source.filter(|source| {
            !bide_rules.gen1
                && state.active_position(*source, rules.topology()).is_some()
                && state.pokemon(*source).is_some_and(|p| !p.is_fainted())
        });
        let target = match source {
            Some(source) => Some(source),
            None => {
                let candidates = pokemon_battle_engine::engine::random_retarget_candidates(
                    state, rules, actor, &release, None,
                )
                .unwrap_or_default();
                if candidates.len() > 1 {
                    let index = struck
                        .and_then(|target| candidates.iter().position(|c| *c == target))
                        .unwrap_or(0);
                    tape.push(Some(RandomOverride::range(
                        RandomRequestKind::RandomTarget,
                        index as u16,
                    )));
                }
                struck.or_else(|| candidates.first().copied())
            }
        };
        // (A type immunity stops the release before its accuracy check, and so does a target
        // out of reach — underwater, underground: the invulnerability step, R-M10-49; Gen 2
        // reports a miss with no target.)
        if bide_rules.release_hit_steps
            && oracle.immune.is_empty()
            && let Some(target) = target
            && two_turn::dodged_by(state, rules, actor, target, &release).is_none()
            && accuracy_in_move(state, rules, actor, target, &release) < 10_000
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::Accuracy,
                oracle.hit,
            )));
        }
        // The release is a contact hit of fixed damage (no critical hit, no roll): the struck
        // Pokémon's reactions to it roll as after any hit — Static and its kind, then Effect
        // Spore, Cute Charm … (R190). Not off a Substitute.
        if oracle.hit
            && oracle.immune.is_empty()
            && let Some(target) = target
            && state
                .pokemon(target)
                .is_some_and(|p| !p.volatiles.contains(VolatileConditionKind::Substitute))
        {
            let mut staged = StagedTape::new(false);
            // Focus Band against the release, as against any hit (R196, I38): Gen 3-4 roll it
            // on the hit; elsewhere only a release that would knock the holder out does.
            if pokemon_battle_engine::engine::held_item_id(state, rules, target)
                == Some(pokemon_battle_engine::ids::ItemId::FOCUS_BAND)
            {
                let held = oracle
                    .focus_band_held
                    .iter()
                    .any(|(who, hits)| *hits == 0 && who_to_id(state, who) == Some(target));
                let decision = RandomOverride::bool(RandomRequestKind::FocusBand, held);
                if matches!(rules.generation(), Generation::Gen3 | Generation::Gen4) {
                    staged.push(Stage::Land, decision);
                } else if held || oracle.hit_hp.first() == Some(&Some(0)) {
                    staged.push(Stage::Land, decision.optional());
                }
            }
            if release.flags.contact
                && state.pokemon(target).is_some_and(|p| {
                    pokemon_battle_engine::engine::ability_handlers(state, rules, p)
                        .contact_status
                        .is_some()
                })
            {
                let took = oracle.contact_status.is_some()
                    && oracle
                        .contact_status_of
                        .as_ref()
                        .is_none_or(|who| who_to_id(state, who) == Some(target));
                staged.push(
                    Stage::Reaction,
                    RandomOverride::bool(RandomRequestKind::ContactReaction, took),
                );
            }
            chance_reactions(
                &mut staged,
                ChanceReactionHit {
                    state: in_move,
                    rules,
                    actor,
                    target,
                    move_def: &release,
                    oracle,
                    behind_substitute: false,
                    dealt: oracle.hit_hp.first().copied().flatten().map_or(1, |after| {
                        state
                            .pokemon(target)
                            .map_or(after, |p| p.hp)
                            .saturating_sub(after)
                    }),
                    reported_after: &|_| true,
                    sleep_duration: duration_for(Some(actor), DurationKind::Sleep),
                    user_keeps_ability: true,
                },
                &mut false,
            );
            // A berry the hit's `Update` fed the struck Pokémon (or the holder, hurt by its
            // reaction): Starf Berry's stat, a confusion berry's duration.
            for (who, _, draw) in &oracle.berry_draws {
                let eater = who_to_id(state, who);
                if (eater == Some(target) || eater == Some(actor))
                    && let Some(decision) =
                        berry_decision(*draw, || duration_for(eater, DurationKind::Confusion))
                {
                    staged.push(Stage::Reaction, decision);
                }
            }
            tape.extend(staged.finish());
        }
        return tape;
    }
    if let Some(move_def) = move_def {
        // A power drawn as the move starts (Magnitude), announced before it hits.
        // Shell Side Arm's coin flip on a tie, as its hint names the category — not with
        // nobody to aim at (`[notarget]`: its `onModifyMove` returns without a target, R-M8-155).
        if move_def.behavior.category_by_estimate
            && !oracle.no_target
            && let Some(target) = fates.first().and_then(|(target, _)| *target)
            && pokemon_battle_engine::engine::physical_by_estimate(
                state, rules, actor, target, move_def,
            )
            .is_none()
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::CategoryTie,
                oracle.physical_hint == Some(true),
            )));
        }
        if let Some(random) = move_def.behavior.random_power {
            tape.push(Some(RandomOverride::range(
                RandomRequestKind::Sample,
                drawn_value(random, oracle, behind_substitute(state, fates)),
            )));
        }
        // Gen 1-2 Thrash's lock as the move starts (§15.12): its uses drawn on the first, the
        // fatigue's confusion on the last.
        if move_def.behavior.rampage && rules.conditions().rampage.counter.counts_uses() {
            if pokemon.volatiles.rampage().is_none() {
                if let Some(uses) = duration_for(Some(actor), DurationKind::Rampage) {
                    tape.push(Some(RandomOverride::range(
                        RandomRequestKind::Duration,
                        uses,
                    )));
                }
            } else if oracle.fatigue
                && let Some(time) = duration_for(Some(actor), DurationKind::Confusion)
            {
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::Duration,
                    time,
                )));
            }
        }
        if two_turn::will_charge(state, rules, actor, move_def) {
            return tape;
        }
        if (oracle.failed && move_def.category != MoveCategory::Status) || oracle.no_target {
            // A lock's turn with nobody to aim at (`[notarget]`) or that failed still ends the
            // lock as the move is over (Gen 5+ `onAfterMove`): its fatigue's confusion.
            if let Some(time) = fatigue_time() {
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::Duration,
                    time,
                )));
            }
            return tape;
        }
        // A move its `onTry` stops (Dark Void for anyone but Darkrai) draws nothing more — as
        // the user is once its BeforeMove is over: one that woke up there no longer snores; nor
        // does one a Gen 3-4 uproar wakes there (the cartridge's `CANCELLER_ASLEEP`, its
        // `-curestatus` printed with the uproar's start, R-M8-160).
        let uproar_woke =
            pokemon_battle_engine::engine::uproar_wakes_before_move(state, rules, actor);
        let woke = (oracle.cured.as_deref() == Some("slp") || uproar_woke).then(|| {
            let mut woke = in_move.clone();
            if let Some(pokemon) = woke.pokemon_mut(actor) {
                pokemon.status = None;
            }
            woke
        });
        // The `onTry` runs once the action spent its PP, which marked the chosen move's slot
        // used (`deductPP`): Copycat's Last Resort counts Copycat as used (R-M8-167).
        let mut tried = woke.clone().unwrap_or_else(|| state.clone());
        if let Some(chosen) = oracle
            .move_name
            .as_deref()
            .and_then(|name| dex.move_by_name(name))
            && let Some(user) = tried.pokemon_mut(actor)
        {
            for slot in user.move_slots_mut() {
                if slot.move_id == chosen.id {
                    slot.used = true;
                }
            }
        }
        if let Some(try_use) = move_def.behavior.try_use
            && !pokemon_battle_engine::engine::try_use_holds(
                &tried,
                rules,
                actor,
                fates.first().and_then(|(target, _)| *target),
                try_use,
            )
        {
            return tape;
        }
        // Sky Drop's carrying turn (§15.23): sure to hit, no damage — nothing to draw.
        if move_def.behavior.sky_drop && !pokemon.volatiles.contains(VolatileConditionKind::SkyDrop)
        {
            return tape;
        }
        // Future Sight, Doom Desire (§15.22): the use only foresees; Gen 2-4 compute the damage
        // then (planned below from the strike's), Gen 5+ at the strike (`residual_tape`).
        if move_def.behavior.future_attack {
            // Gen 4 draws the computed damage's random factor at use: an open roll, settled
            // from the strike's damage (`future_damage_hint`).
            if rules.conditions().future_attack.damage_at_use
                == Some(pokemon_battle_engine::rules::ForeseenRoll::AtUse)
                && !oracle.failed
            {
                tape.push(None);
            }
            return tape;
        }
        // Whether an accuracy decision is drawn against a target: the engine's own chance with
        // the current accuracy / evasion stages (Muddy Water's accuracy drop makes a 100 %
        // move miss-able), not the move's base accuracy.
        let rolls_accuracy = |target: Option<PokemonId>| match target {
            Some(id) => accuracy_in_move(state, rules, actor, id, move_def) < 10_000,
            None => {
                !pokemon_battle_engine::engine::sure_hit(state, rules, actor, move_def)
                    && move_def.accuracy.is_some_and(|accuracy| accuracy < 100)
            }
        };
        // A `multiaccuracy` move's checks after the first: on the state the first check left
        // (Micle Berry's boost and a Lock-On on the target used up, `engine::spend_accuracy_check`).
        let rolls_later_accuracy = |target: Option<PokemonId>| match target {
            Some(id) => {
                let mut after = state.clone();
                pokemon_battle_engine::engine::spend_accuracy_check(
                    &mut after, rules, actor, id, move_def,
                );
                accuracy_in_move(&after, rules, actor, id, move_def) < 10_000
            }
            None => rolls_accuracy(None),
        };
        // A target that did not take the move (`Immune`: a type immunity, or a TryHit block such
        // as Protect or an absorbing ability) may still have drawn an accuracy decision, by the
        // generation's hit-step order (engine `HitStepOrder`): Gen 4 rolls accuracy before
        // TryHit, Gen 3 even before reporting a type immunity (a hit then meets TryHit, so a
        // TryHit block means the roll hit). Returns the decision's value.
        let accuracy_before_block = |target: Option<PokemonId>| -> Option<bool> {
            let id = target?;
            if pokemon_battle_engine::engine::ohko_blocked(state, rules, actor, id, move_def)
                || !rolls_accuracy(target)
            {
                return None;
            }
            // Gen 7+'s Dark-type immunity to a Prankster status move is the type check's too.
            let type_immune =
                pokemon_battle_engine::engine::type_immunity(state, rules, id, move_def)
                    .1
                    .is_some()
                    || pokemon_battle_engine::engine::prankster_immune(
                        state,
                        rules,
                        actor,
                        id,
                        move_def.category == MoveCategory::Status,
                    );
            // A type-immune target reports `-immune` whether the roll hit or missed; one that
            // TryHit stopped (Volt Absorb on a Ground type) was hit.
            let stopped_at_try_hit = oracle
                .try_hit_blocked
                .iter()
                .any(|who| who_to_id(state, who) == Some(id));
            // A status move's own status immunity (Poison Gas on a Steel type; Toxic Thread's
            // after its Speed drop) only shows when the status lands, after the accuracy check —
            // unless the gate stopped it first (a Grass type against a powder move from Gen 6).
            let powder_blocked = move_def.flags.powder
                && pokemon_battle_engine::engine::powder_immunity(state, rules, id).is_some();
            if !type_immune
                && !powder_blocked
                && let PrimaryEffect::Status(
                    StatusMoveEffect::SetStatus(status)
                    | StatusMoveEffect::StagesThen(_, FollowUp::Status(status)),
                ) = move_def.primary_effect
                && ((!stopped_at_try_hit
                    && pokemon_battle_engine::engine::status_type_immune(state, rules, id, status))
                    // Or an ability refusing the status as it is set (Limber, Pastel Veil):
                    // its `-immune … [from] ability:` comes once the move landed (R277) —
                    // unless a guard (Protect, Max Guard) stopped the move first (R-M8-109).
                    || (!oracle
                        .guarded
                        .iter()
                        .any(|who| who_to_id(state, who) == Some(id))
                        && pokemon_battle_engine::engine::ability_refuses_status(
                            state,
                            rules,
                            id,
                            status,
                            pokemon_battle_engine::engine::EffectSource::Move {
                                pokemon: actor,
                                move_id: move_def.id,
                            },
                        )))
            {
                return Some(true);
            }
            // Gen 3 reports the move's own immunity (Dream Eater on an awake target) with the
            // type's; Gen 4 checks it right after a hit.
            let move_immune =
                pokemon_battle_engine::engine::move_immune(state, rules, actor, id, move_def);
            match rules.hits().step_order {
                // Gen 3 Rage against a type immunity: its flag is the roll's (§15.19).
                HitStepOrder::AccuracyFirst
                    if type_immune
                        && !stopped_at_try_hit
                        && move_def.behavior.rage
                        && rules.conditions().rage.accuracy_sets =>
                {
                    Some(oracle.raging_after.unwrap_or(false))
                }
                HitStepOrder::AccuracyFirst => {
                    Some(!(type_immune || move_immune) || stopped_at_try_hit)
                }
                HitStepOrder::AccuracyBeforeTryHit if !type_immune => Some(true),
                _ => None,
            }
        };
        let targets: Vec<(Option<PokemonId>, TargetFate)> = if fates.is_empty() {
            vec![(
                None,
                if oracle.hit {
                    TargetFate::Hit
                } else {
                    TargetFate::Miss
                },
            )]
        } else {
            fates.to_vec()
        };
        // A status move checks accuracy once per Pokémon target (immune targets ask nothing),
        // and sleep, confusion, Taunt and Encore roll their duration on each target they land
        // on, right after its check.
        if move_def.primary_effect != PrimaryEffect::Damage {
            let rolled = |rule: pokemon_battle_engine::rules::TimedVolatileRules| {
                matches!(
                    rule.duration,
                    pokemon_battle_engine::rules::TimedDuration::Rolled { .. }
                )
            };
            let duration_roll = match &move_def.primary_effect {
                PrimaryEffect::Status(StatusMoveEffect::SetStatus(NonVolatileStatus::Sleep)) => {
                    Some(("status:slp", DurationKind::Sleep))
                }
                // Psycho Shift carrying a sleep (a sleeper's Sleep Talk called it): the sleep
                // it gives rolls its own duration (`setStatus`).
                PrimaryEffect::Status(StatusMoveEffect::TransferStatus)
                    if state
                        .pokemon(actor)
                        .is_some_and(|p| p.has_status(NonVolatileStatus::Sleep)) =>
                {
                    Some(("status:slp", DurationKind::Sleep))
                }
                PrimaryEffect::Status(
                    StatusMoveEffect::Volatile(VolatileConditionKind::Confusion)
                    | StatusMoveEffect::StagesThen(
                        _,
                        FollowUp::Volatile(VolatileConditionKind::Confusion),
                    ),
                ) => Some(("start:confusion", DurationKind::Confusion)),
                // Gen 3-4 roll Taunt / Encore durations when they take.
                PrimaryEffect::Status(StatusMoveEffect::Volatile(VolatileConditionKind::Taunt))
                    if rolled(rules.conditions().taunt) =>
                {
                    Some(("start:move: Taunt", DurationKind::Timed))
                }
                PrimaryEffect::Status(StatusMoveEffect::Volatile(
                    VolatileConditionKind::Encore,
                )) if rolled(rules.conditions().encore) => {
                    Some(("start:Encore", DurationKind::Timed))
                }
                _ => None,
            };
            // A spread move checks each target's accuracy in turn on the state the earlier ones
            // left: the first check spends the user's Micle Berry, so a later target rolls the
            // plain accuracy (Showdown `hitStepAccuracy`'s loop, DESIGN.md R-M8-44).
            let mut accuracy_state = state.clone();
            for (index, (target, fate)) in targets.iter().enumerate() {
                // A spread move some of its targets sent back: the targets the move strikes
                // itself are the user's, checked by it with its move; a reflection's are its
                // reflector's, with the reflector's move, after the user's check at the reflector
                // (R-M8-173), each reported in its own stretch of the action (R-M8-176).
                let reflection = reflections
                    .iter()
                    .position(|reflection| reflection.back.contains(&index));
                if let Some(at) = reflection
                    && reflections[at].back.start == index
                    && let Some(check) = check_at(reflections[at].reflector)
                {
                    tape.push(check);
                }
                let (actor, move_def) = match reflection {
                    Some(at) => (
                        reflections[at].reflector,
                        reflected_moves[at].as_ref().unwrap_or(move_def),
                    ),
                    None if !reflections.is_empty() => (user, own_move_def.unwrap_or(move_def)),
                    None => (actor, move_def),
                };
                let applied = match reflection {
                    Some(at) => called_outcomes(oracle, reflections[at].called).applied,
                    None => &oracle.applied[..],
                };
                if *fate == TargetFate::Immune {
                    if let Some(hit) = accuracy_before_block(*target) {
                        tape.push(Some(RandomOverride::bool(RandomRequestKind::Accuracy, hit)));
                    }
                } else if *fate != TargetFate::Dodged
                    && match target {
                        // No target the replay can name: the move's own accuracy decides (a
                        // sure-hit move and one of 100 never draw).
                        None => {
                            !pokemon_battle_engine::engine::sure_hit(
                                &accuracy_state,
                                rules,
                                actor,
                                move_def,
                            ) && move_def.accuracy.is_some_and(|accuracy| accuracy < 100)
                        }
                        Some(id) => {
                            accuracy_in_move(&accuracy_state, rules, actor, *id, move_def) < 10_000
                        }
                    }
                {
                    tape.push(Some(RandomOverride::bool(
                        RandomRequestKind::Accuracy,
                        *fate == TargetFate::Hit,
                    )));
                }
                if let Some(id) = target
                    && matches!(*fate, TargetFate::Hit | TargetFate::Miss)
                {
                    pokemon_battle_engine::engine::spend_accuracy_check(
                        &mut accuracy_state,
                        rules,
                        actor,
                        *id,
                        move_def,
                    );
                }
                if *fate == TargetFate::Hit
                    && let Some((label, class)) = duration_roll
                    && applied.iter().any(|(who, kind)| {
                        kind == label && target.is_none_or(|id| who_to_id(state, who) == Some(id))
                    })
                    && let Some(duration) = duration_for(*target, class)
                {
                    tape.push(Some(RandomOverride::range(
                        RandomRequestKind::Duration,
                        duration,
                    )));
                }
                // Poison Puppeteer: the poison confuses the target too, rolling its duration —
                // only the holder's (a confusion berry the poison fed it rolls its own, R-M9-35).
                if *fate == TargetFate::Hit
                    && pokemon_battle_engine::engine::poison_puppeteer_confuses(state, rules, actor)
                    && matches!(
                        move_def.primary_effect,
                        PrimaryEffect::Status(StatusMoveEffect::SetStatus(
                            NonVolatileStatus::Poison | NonVolatileStatus::Toxic
                        ))
                    )
                    && applied.iter().any(|(who, kind)| {
                        kind == "start:confusion"
                            && target.is_none_or(|id| who_to_id(state, who) == Some(id))
                    })
                    && let Some(duration) = duration_for(*target, DurationKind::Confusion)
                {
                    tape.push(Some(RandomOverride::range(
                        RandomRequestKind::Duration,
                        duration,
                    )));
                }
            }
        }
        // Gen 1 Mimic draws among the target's move slots as it learns (`this.sample(target.moves)`,
        // `copyable`): the position of the move it learned, read off the log's `-start|…|Mimic|…`
        // line. The draw comes after the move's own accuracy check (Gen 1 rolls the byte even for
        // a move of 100), and a Mimic that failed draws nothing — the entry is optional, like any
        // decision an action that never got there does not ask for.
        // The move the action actually ends up using: Mimic itself, or the Mimic a caller
        // (Mirror Move, Metronome) called — both draw it.
        let innermost_move = oracle
            .called
            .last()
            .map(|called| called.name.as_str())
            .or(oracle.move_name.as_deref());
        if innermost_move == Some("Mimic")
            && rules.conditions().calls.mimic_draws_from_the_slots
            && let Some(learned) = oracle.mimicked.as_deref()
            && let Some(target) = oracle
                .move_target
                .as_ref()
                .and_then(|who| who_to_id(state, who))
            && let Some(target_state) = state.pokemon(target)
            && let Some(learned_def) = dex.moves().iter().find(|def| def.name == learned)
            && let Some(index) = target_state
                .move_slots()
                .iter()
                .position(|slot| slot.move_id == learned_def.id)
        {
            tape.push(Some(
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ));
        }
        // Gen 1 Disable picks one of the target's moves with PP (drawn when there is a choice),
        // then its `time`.
        if move_def.id == MoveId::DISABLE
            && rules.conditions().disable_gen1
            && let Some(sealed) = oracle.disabled_move.as_deref()
            && let Some(victim) = targets
                .first()
                .and_then(|(target, _)| *target)
                .and_then(|id| state.pokemon(id))
        {
            let candidates: Vec<(usize, &str)> = victim
                .move_slots()
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.pp > 0)
                .filter_map(|(index, slot)| {
                    dex.move_by_id(slot.move_id)
                        .map(|m| (index, m.name.as_str()))
                })
                .collect();
            if candidates.len() > 1 {
                // The slot it drew where the oracle names it (a Mimic's copy beside the
                // original, R-M8-161); otherwise the first slot holding the move.
                let index = oracle
                    .disabled_slot
                    .and_then(|drawn| candidates.iter().position(|(slot, _)| *slot == drawn))
                    .or_else(|| candidates.iter().position(|(_, name)| *name == sealed))
                    .unwrap_or(0);
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::DisableSlot,
                    index as u16,
                )));
            }
            if let Some(duration) = duration_for(Some(victim.id()), DurationKind::Timed) {
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::Duration,
                    duration,
                )));
            }
        }
        // Gen 2-3 Spite (§15.15): the PP it took, drawn once its checks passed — the number its
        // `-activate` shows.
        if move_def.id == MoveId::SPITE
            && matches!(
                rules.conditions().pp.spite.amount,
                pokemon_battle_engine::rules::SpiteAmount::Rolled { .. }
            )
            && let Some(drop) = oracle.spite_drop
        {
            tape.push(Some(RandomOverride::range(RandomRequestKind::PpDrop, drop)));
        }
        // Disable (a Custom move) rolls its duration when it takes, before Gen 5.
        if move_def.id == MoveId::DISABLE
            && !rules.conditions().disable_gen1
            && oracle
                .applied
                .iter()
                .any(|(_, kind)| kind.starts_with("start:Disable"))
            && matches!(
                rules.conditions().disable.duration,
                pokemon_battle_engine::rules::TimedDuration::Rolled { .. }
            )
            && let Some(duration) = duration_for(
                targets.first().and_then(|(target, _)| *target),
                DurationKind::Timed,
            )
        {
            tape.push(Some(RandomOverride::range(
                RandomRequestKind::Duration,
                duration,
            )));
        }
        // Bide (a Custom move) as it starts: Gen 2's duration, Gen 1's time (§15.10).
        let bide_turns = rules.conditions().retaliation.bide.turns;
        if move_def.id == MoveId::BIDE
            && bide_turns.0 != bide_turns.1
            && oracle
                .applied
                .iter()
                .any(|(_, kind)| matches!(kind.as_str(), "start:move: Bide" | "start:Bide"))
            && let Some(duration) = duration_for(Some(actor), DurationKind::Bide)
        {
            tape.push(Some(RandomOverride::range(
                RandomRequestKind::Duration,
                duration,
            )));
        }
        // Acupressure: the stat `sample` drew (R166) — on its snatcher, when another Pokémon
        // took it over (Snatch uses it anew, at its own default target: itself).
        if move_def.id == MoveId::ACUPRESSURE
            && let Some(victim) = taker.or_else(|| {
                targets.first().and_then(|(target, _)| *target).or_else(|| {
                    oracle
                        .move_target
                        .as_ref()
                        .and_then(|who| who_to_id(state, who))
                })
            })
        {
            tape.extend(acupressure_tape(state, victim, oracle));
        }
        // Conversion, Conversion 2: the type `sample` drew (R254) — once the move reached its
        // `onHit`: not on a target that dodged, was missed or was not affected (Good as Gold,
        // Protect; DESIGN.md R263).
        if matches!(move_def.id, MoveId::CONVERSION | MoveId::CONVERSION_2)
            && targets
                .first()
                .is_none_or(|(_, fate)| *fate == TargetFate::Hit)
        {
            let target = targets.first().and_then(|(target, _)| *target).or_else(|| {
                oracle
                    .move_target
                    .as_ref()
                    .and_then(|who| who_to_id(state, who))
            });
            tape.extend(conversion_tape(
                in_move,
                rules,
                dex,
                actor,
                target,
                move_def.id,
                oracle,
            ));
        }
        // Gen 9 Ally Switch (`onPrepareHit`): 1 / its own counter while one stands, rolled
        // before anything else the move does. A failed roll is `PrepareHit`'s bare `-fail`; a
        // roll that went through can still fail the swap itself (no partner: a single battle),
        // which names the move (`-fail|p|move: Ally Switch`, R-M8-58).
        if rules.conditions().calls.ally_switch_consecutive_chance
            && move_def.id == MoveId::ALLY_SWITCH
            && pokemon.volatiles.ally_switch_counter().is_some()
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::ProtectSuccess,
                !oracle.failed || oracle.fail_named_the_move,
            )));
        }
        // A stalling move right after a use (Protect and its variants, Endure; Gen 5's guards —
        // `rolls_stall`): 1 / stall counter, only rolled when somebody else still acts
        // this turn (otherwise the move just fails).
        // The roll held when the move went on: its `-singleturn`, or — a second use in the
        // turn (Instruct), the protection already up — no `-fail` (R-M8-84).
        // An instructed use (`queue.willAct()`: any action left) counts its user's own action
        // still to come (R-M8-149).
        if rolls_stall(rules, move_def)
            && pokemon.volatiles.stall_counter().is_some()
            && state
                .pending_movers
                .iter()
                .any(|mover| *mover != actor || oracle.instructed)
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::ProtectSuccess,
                oracle.protected || !oracle.failed,
            )));
        }
        // A stalling move Snatch took (a Gen 5 guard): the snatcher's own roll, 1 / its stall
        // counter, while somebody else — not the action's user, whose action this is — still
        // acts; it held when the snatcher's `-singleturn` came (R-M11-113).
        if rolls_stall(rules, move_def)
            && let Some(snatcher) = oracle
                .snatched_by
                .as_ref()
                .and_then(|who| who_to_id(state, who))
            && state
                .pokemon(snatcher)
                .is_some_and(|p| p.volatiles.stall_counter().is_some())
            && state
                .pending_movers
                .iter()
                .any(|mover| *mover != snatcher && *mover != actor)
        {
            tape.push(Some(RandomOverride::bool(
                RandomRequestKind::ProtectSuccess,
                oracle.snatcher_protected,
            )));
        }
        if move_def.primary_effect == PrimaryEffect::Damage {
            // Hit sequence per target, in target order: the count is fixed to what Showdown
            // reported (a sequence cut short by a faint reports the hits that landed, which
            // reproduces the same outcome), then every hit rolls critical + damage (Gen 1: once
            // for the whole sequence) and its per-hit secondaries. `-crit` events are consumed
            // in order across targets.
            let landed = oracle.hit_count.unwrap_or(1).max(1);
            let same_damage = rules.hits().same_damage_each_hit;
            // The same per-target accuracy chain as above: the first check spends the Micle
            // Berry (DESIGN.md R-M8-44).
            let mut accuracy_state = state.clone();
            // Each target's hits, in order: a hit reported for a Pokémon matches that target;
            // one the replay cannot place is taken in order.
            let mut taken = vec![false; oracle.crits.len()];
            // Gen 3-4 Focus Bands this move armed (they roll no more).
            let mut bands_armed: Vec<PokemonId> = Vec::new();
            // A spread move resolves stage by stage (engine `apply_spread_hit`): every target's
            // accuracy, then the Substitutes' damage, the other targets' damage, the
            // secondaries and the reactions, each stage in target order. Gen 1-3 hit each
            // target in full before the next (`staged_spread_hits`).
            let spread = rules.hits().staged_spread_hits
                && move_def.multi_hit == MultiHit::Single
                && targets
                    .iter()
                    .filter(|(target, _)| target.is_some())
                    .count()
                    > 1;
            let mut staged = StagedTape::new(spread);
            staged.gates_first = move_def.smart_target && targets.len() > 1;
            // A user-directed secondary rolls once per target and hit; the user shows one
            // effect per success (Gen 6 Diamond Storm's Defense +1 on each foe it rolls for):
            // that many rolls landed, the first ones — which ones cannot be told, nor matters.
            let mut user_effects_left: Vec<Option<usize>> = vec![None; move_def.secondaries.len()];
            // Thrash's or Uproar's lock is drawn as the first hit starts it; a later hit (Parental
            // Bond's second) adds the volatile again, which draws nothing (DESIGN.md R281).
            let mut lock_drawn = false;
            // A single-hit move's own `self` lands with its first hit that went through
            // (Showdown `selfDropped`).
            let mut self_planned = false;
            // Cursed Body rolls until it disables the user's move, on whichever target.
            let mut cursed_body_done = false;
            for (target, fate) in &targets {
                // Dragon Darts shows no `-hitcount`: its darts per target are the hits the log
                // names on it.
                let landed = if move_def.smart_target {
                    let on_target = oracle
                        .hit_targets
                        .iter()
                        .filter(|who| who.as_ref().and_then(|w| who_to_id(state, w)) == *target)
                        .count() as u16;
                    on_target.max(1)
                } else {
                    landed
                };
                // Static / Flame Body / Poison Point roll after every hit that reaches the
                // target's HP on a contact move (a Substitute takes the hit instead).
                // Per hit, only a hit on the body reacts: a multi-hit move that breaks a
                // Substitute meets the ability on the hits after it. A move counts as contact as
                // the engine has it (`checkMoveMakesContact`: an attacker's Protective Pads take
                // the contact away, R-M8-153).
                let contact_reaction = target.is_some_and(|id| {
                    pokemon_battle_engine::engine::move_makes_contact(state, rules, actor, move_def)
                        && state.pokemon(id).is_some_and(|p| {
                            pokemon_battle_engine::engine::ability_handlers(state, rules, p)
                                .contact_status
                                .is_some()
                        })
                });
                match fate {
                    TargetFate::Immune => {
                        if let Some(hit) = accuracy_before_block(*target) {
                            staged.push(
                                Stage::Gate,
                                RandomOverride::bool(RandomRequestKind::Accuracy, hit),
                            );
                        }
                        continue;
                    }
                    TargetFate::Dodged => continue,
                    TargetFate::Miss => {
                        if accuracy_draws(&accuracy_state, rules, actor, *target, move_def) {
                            staged.push(
                                Stage::Gate,
                                RandomOverride::bool(RandomRequestKind::Accuracy, false),
                            );
                        }
                        continue;
                    }
                    TargetFate::Hit => {
                        if accuracy_draws(&accuracy_state, rules, actor, *target, move_def) {
                            staged.push(
                                Stage::Gate,
                                RandomOverride::bool(RandomRequestKind::Accuracy, true),
                            );
                        }
                    }
                }
                if let Some(id) = target
                    && matches!(fate, TargetFate::Hit | TargetFate::Miss)
                {
                    pokemon_battle_engine::engine::spend_accuracy_check(
                        &mut accuracy_state,
                        rules,
                        actor,
                        *id,
                        move_def,
                    );
                }
                // Present's heal, Pollen Puff on an ally: no critical hit, no damage roll.
                let drawn = move_def.behavior.random_power.map(|random| {
                    random.effect(drawn_value(random, oracle, behind_substitute(state, fates)))
                });
                if target.is_some_and(|id| {
                    pokemon_battle_engine::engine::heals_instead(
                        state, rules, actor, id, move_def, drawn,
                    )
                    .is_some()
                }) {
                    continue;
                }
                if let MultiHit::Range { min, .. } = move_def.multi_hit {
                    // A sequence cut short by a faint after fewer hits than the minimum: any
                    // legal count stops at the same hit.
                    staged.push(
                        Stage::Gate,
                        RandomOverride::range(
                            RandomRequestKind::MultiHitCount,
                            landed.max(u16::from(min)),
                        ),
                    );
                }
                // A power of nothing (Gen 2-4 Beat Up without members, a Frustration of full
                // happiness in Gen 2) stops before the hit: no critical hit, no roll, no
                // secondary (not even an item's flinch).
                if target.is_some_and(|id| {
                    pokemon_battle_engine::engine::zero_power_does_nothing(
                        in_move, rules, dex, actor, id, move_def,
                    )
                }) {
                    continue;
                }
                // Fixed damage and one-hit KOs roll neither a critical hit nor damage; the
                // engine says whether a critical-hit decision is drawn (`willCrit`, a ratio of 0,
                // Gen 1 without a species ask nothing).
                let formula = target.is_none_or(|id| {
                    pokemon_battle_engine::engine::fixed_damage(state, rules, actor, id, move_def)
                        .is_none()
                        && pokemon_battle_engine::engine::binding_damage(
                            state, rules, actor, id, move_def,
                        )
                        .is_none()
                        && move_def.fixed_damage.is_none()
                        && !pokemon_battle_engine::engine::zero_power_does_nothing(
                            in_move, rules, dex, actor, id, move_def,
                        )
                });
                let crit_rolled = matches!(
                    pokemon_battle_engine::engine::critical_roll(
                        in_move,
                        rules,
                        dex,
                        actor,
                        target.unwrap_or(actor),
                        move_def
                    ),
                    pokemon_battle_engine::engine::CriticalRoll::Chance(..)
                );
                // The hits the sequence was decided to have (Beat Up: one per member, as the
                // engine fixes them when the move starts); Gen 1 plans its one damage roll on
                // the first hit and its secondaries with it, as of the hit that rolls them.
                let planned = match move_def.multi_hit {
                    _ if move_def.behavior.beat_up => {
                        pokemon_battle_engine::engine::beat_up_members(in_move, rules, actor).len()
                            as u16
                    }
                    MultiHit::Single => 1,
                    MultiHit::Fixed(count) => u16::from(count),
                    MultiHit::Range { min, .. } => landed.max(u16::from(min)),
                };
                for hit in 0..usize::from(landed) {
                    if hit > 0 && same_damage {
                        break;
                    }
                    let hit_number = if same_damage {
                        usize::from(landed)
                    } else {
                        hit + 1
                    };
                    // Showdown `multiaccuracy` (Triple Kick): every hit after the first checks
                    // accuracy again.
                    if hit > 0 && move_def.multi_accuracy && rolls_later_accuracy(*target) {
                        staged.push(
                            Stage::Gate,
                            RandomOverride::bool(RandomRequestKind::Accuracy, true),
                        );
                    }
                    let reached = |index: usize| {
                        oracle
                            .hit_targets
                            .get(index)
                            .cloned()
                            .flatten()
                            .and_then(|who| who_to_id(state, &who))
                    };
                    let index = (0..taken.len())
                        .find(|&i| !taken[i] && target.is_some() && reached(i) == *target)
                        .or_else(|| (0..taken.len()).find(|&i| !taken[i]));
                    if let Some(index) = index {
                        taken[index] = true;
                    }
                    let crit = index.is_some_and(|i| oracle.crits[i]);
                    let behind_substitute =
                        index.is_some_and(|i| oracle.substitute_hits.get(i) == Some(&true));
                    // The HP this hit took off its target: from the HP the target's previous
                    // hit left it (or it had before the move) to this hit's. A Focus Band holding
                    // a target already at 1 HP takes nothing (`damage` 0 for the Gen 3-4 contact
                    // reactions).
                    let before_after = match (*target, index) {
                        (Some(id), Some(i)) => oracle.hit_hp[i].map(|after| {
                            // A berry the `Update` after an earlier hit fed the target counts
                            // (a Mago Berry between a Fury Attack's hits, R-M11-99).
                            let before = oracle
                                .hit_hp_before
                                .get(i)
                                .copied()
                                .flatten()
                                .or_else(|| {
                                    (0..i)
                                        .rev()
                                        .find(|&j| {
                                            reached(j) == Some(id) && oracle.hit_hp[j].is_some()
                                        })
                                        .and_then(|j| oracle.hit_hp[j])
                                })
                                .or_else(|| state.pokemon(id).map(|p| p.hp))
                                .unwrap_or(after);
                            (before, after)
                        }),
                        _ => None,
                    };
                    let dealt =
                        before_after.map_or(1, |(before, after)| before.saturating_sub(after));
                    // The hit left its target's HP exactly as it was (not healed in between).
                    let took_nothing = before_after.is_some_and(|(before, after)| before == after);
                    let damage_stage = if behind_substitute {
                        Stage::SubstituteDamage
                    } else {
                        Stage::Damage
                    };
                    if formula {
                        if crit_rolled {
                            staged.push(
                                damage_stage,
                                RandomOverride::bool(RandomRequestKind::Critical, crit),
                            );
                        }
                        // A BasePower participant's own chance (Fickle Beam), after the
                        // critical-hit check.
                        if move_def.behavior.power_chance.is_some() {
                            staged.push(
                                damage_stage,
                                RandomOverride::bool(
                                    RandomRequestKind::PowerChance,
                                    oracle.power_chance,
                                ),
                            );
                        }
                        // A hit that took nothing off a target in the open may draw no roll: a
                        // BasePower event that left 0 stops Showdown's `getDamage` before it
                        // (R-M8-84). Its roll is searched as any other, but optional.
                        let open = !move_def.no_damage_variance;
                        if open && took_nothing && !behind_substitute {
                            staged.push(damage_stage, open_optional_roll());
                        } else if open {
                            staged.push_roll(damage_stage);
                        }
                    } else if move_def.fixed_damage
                        == Some(pokemon_battle_engine::dex::FixedDamage::Psywave)
                        && let Some(id) = *target
                    {
                        // Psywave's damage is the roll: read back from the HP it took — or, a
                        // hold on 1 HP the log names (a Focus Band, a Focus Sash, Sturdy), the
                        // smallest roll that would have knocked the target out (R-M10-48).
                        let named = |who: &Who| who_to_id(state, who) == Some(id);
                        let held = oracle
                            .focus_band_held
                            .iter()
                            .any(|(who, hits)| Some(*hits) == index && named(who))
                            || oracle.focus_sash_held.iter().any(named)
                            || oracle.sturdy_held.iter().any(named);
                        let (dealt, after) = if held {
                            (state.pokemon(id).map_or(0, |p| p.hp), Some(0))
                        } else {
                            (dealt, index.and_then(|i| oracle.hit_hp[i]))
                        };
                        staged.push(
                            damage_stage,
                            RandomOverride::range(
                                RandomRequestKind::DamageRoll,
                                psywave_roll(state, rules, actor, id, dealt, after, substitutes),
                            ),
                        );
                    }
                    // Focus Band (DESIGN.md R196): where it lasts the move (Gen 3-4, issues.md
                    // I38) every hit rolls until one succeeds — the success nothing shows is
                    // planned on the hit the band held (a Substitute's hit rolls with its damage);
                    // elsewhere only a hit that would knock the holder out rolls: one it held or
                    // one that left it fainted.
                    if let (Some(id), Some(i)) = (*target, index)
                        && pokemon_battle_engine::engine::held_item_id(state, rules, id)
                            == Some(pokemon_battle_engine::ids::ItemId::FOCUS_BAND)
                    {
                        let held = oracle
                            .focus_band_held
                            .iter()
                            .any(|(who, hits)| *hits == i && who_to_id(state, who) == Some(id));
                        let decision = RandomOverride::bool(RandomRequestKind::FocusBand, held);
                        // Read from the generation (I38), not the engine's rule table, so that
                        // the replay checks the rule.
                        if matches!(rules.generation(), Generation::Gen3 | Generation::Gen4) {
                            if !bands_armed.contains(&id) {
                                staged.push(
                                    if behind_substitute {
                                        Stage::SubstituteDamage
                                    } else {
                                        Stage::Land
                                    },
                                    decision,
                                );
                                if held {
                                    bands_armed.push(id);
                                }
                            }
                        } else if !behind_substitute {
                            // Optional, on every hit: only a damage that would knock the holder
                            // out draws it, and the damage-roll search tries rolls that would or
                            // would not — the engine then goes on to the hit's next draw (a
                            // secondary's chance, the next hit's critical-hit check). A hit the
                            // log shows the holder standing through without the band held it
                            // with no draw; a roll that would draw there is a wrong roll, and
                            // the planned failure says so (DESIGN.md R251).
                            staged.push(Stage::Land, decision.optional());
                        }
                    }
                    // Thrash's lock starts with the hit (Gen 3+ `selfDrops`, before the
                    // secondaries, §15.12): its length drawn.
                    if move_def.behavior.rampage
                        && !rules.conditions().rampage.counter.counts_uses()
                        && pokemon.volatiles.rampage().is_none()
                        && !std::mem::replace(&mut lock_drawn, true)
                        && let Some(length) = duration_for(Some(actor), DurationKind::Rampage)
                    {
                        staged.push(
                            Stage::Effect,
                            RandomOverride::range(RandomRequestKind::Duration, length),
                        );
                    }
                    // So does Uproar (Gen 3-4: its turns drawn).
                    let uproar_turns = rules.conditions().uproar.turns;
                    if move_def.behavior.uproar
                        && uproar_turns.0 != uproar_turns.1
                        && pokemon.volatiles.uproar().is_none()
                        && !std::mem::replace(&mut lock_drawn, true)
                        && let Some(turns) = duration_for(Some(actor), DurationKind::Uproar)
                    {
                        staged.push(
                            Stage::Effect,
                            RandomOverride::range(RandomRequestKind::Duration, turns),
                        );
                    }
                    let multi_hit = move_def.multi_hit != MultiHit::Single;
                    // A bare `-fail|target` on a target the move dealt nothing: the hit came to
                    // nothing there (Gen 2's `moveHit` returns false — a Counter with nothing to
                    // answer, Present's heal at full HP) before any secondary, King's Rock's flinch
                    // included. (A target the move damaged may show one after its Synchronize
                    // failed to send a status back: its secondaries rolled.)
                    // So does a hit on a doll that shows nothing at all — no strike on it, no
                    // damage: Gen 2's Substitute (`onTryPrimaryHit`) returns null without a word
                    // when the move's damage is nothing (a Mirror Coat with nothing to answer,
                    // R-M8-179).
                    let came_to_nothing = target.is_some_and(|id| {
                        let on_target = |who: &Who| who_to_id(state, who) == Some(id);
                        let undamaged = !oracle.damages.iter().any(|(who, _)| on_target(who));
                        let failed = oracle
                            .applied
                            .iter()
                            .any(|(who, kind)| kind == "fail" && on_target(who));
                        let doll_untouched = substitute_hp(state, id).is_some()
                            && !oracle
                                .substitute_struck
                                .iter()
                                .any(|(who, _)| on_target(who));
                        undamaged && (failed || doll_untouched)
                    });
                    // Gen 1: a confusion secondary reaches past a doll the hit left standing
                    // (`ConditionRules::confusion_secondary_through_substitute`, R-M8-93).
                    let doll_stands = target.is_some_and(|id| {
                        substitutes
                            .iter()
                            .any(|(holder, hp)| *holder == id && hp.is_some_and(|hp| hp > 0))
                    });
                    // The berries this hit fed the target or the user (`berry_draws`): one the
                    // move itself stole and ate (Pluck, R-M9-48) or threw (Fling, R-M10-3) draws
                    // in its `onHit` — ahead of the move's `self` and secondaries (King's Rock's
                    // flinch, R-M11-119) as of the hit's reactions; any other after the reactions.
                    let mut berries_after_reactions = Vec::new();
                    for (who, hits, draw) in &oracle.berry_draws {
                        let eater = who_to_id(state, who);
                        if index.is_some_and(|i| i + 1 == *hits)
                            && (eater == *target || eater == Some(actor))
                            && !berry_confusions_planned.contains(&eater)
                            && let Some(decision) = berry_decision(*draw, || {
                                duration_for(eater, DurationKind::Confusion)
                            })
                        {
                            berry_confusions_planned.push(eater);
                            if oracle.stolen_berry.as_ref() == Some(who)
                                || oracle.flung_berry.as_ref() == Some(who)
                            {
                                staged.push(Stage::Effect, decision);
                            } else {
                                berries_after_reactions.push(decision);
                            }
                        }
                    }
                    // The move's own `self` lands with the hit, before its secondaries (Showdown
                    // `selfDrops`): a multi-hit move's on every hit, a single-hit move's at the
                    // first that went through — a spread move's before any target's secondaries
                    // (R-M11-102). `selfBoost` never per hit.
                    let move_self = multi_hit
                        || (!came_to_nothing && !std::mem::replace(&mut self_planned, true));
                    let per_hit = move_def
                        .secondaries
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| move_self && s.trigger == SecondaryTrigger::OnceAfterMove)
                        .chain(
                            move_def
                                .secondaries
                                .iter()
                                .enumerate()
                                .filter(|(_, s)| s.trigger == SecondaryTrigger::PerTargetHit),
                        );
                    for (secondary_index, secondary) in per_hit {
                        let past_the_doll =
                            rules.conditions().confusion_secondary_through_substitute
                                && doll_stands
                                && secondary.effect
                                    == SecondaryEffect::Volatile(VolatileConditionKind::Confusion);
                        // A hit on a Substitute rolls nothing for the target behind it.
                        if came_to_nothing
                            || (behind_substitute
                                && secondary.target == SecondaryTarget::Target
                                && !past_the_doll)
                            || pokemon_battle_engine::engine::secondary_withheld(
                                rules,
                                move_def,
                                hit_number as u8,
                                planned as u8,
                                secondary,
                            )
                        {
                            continue;
                        }
                        // Did this secondary land on this target? Per target for a
                        // target-directed one (spread moves roll each target separately); on the
                        // user for a self-directed one.
                        // What this secondary shows when it lands on its target. A stage
                        // change goes the secondary's way (the other with Contrary): a Rage
                        // user's Attack boost from the hit is not Aurora Beam's drop (R-M8-151).
                        let shows = |label: &str, contrary: bool| match secondary.effect {
                            SecondaryEffect::Status(status) => {
                                label == format!("status:{}", status_id(status))
                            }
                            SecondaryEffect::StatStage { stat, delta } => {
                                (stat_changed(label, stat)
                                    && label.starts_with(if (delta < 0) != contrary {
                                        "unboost:"
                                    } else {
                                        "boost:"
                                    }))
                                    || (delta < 0 && label == "reflected")
                            }
                            SecondaryEffect::Volatile(VolatileConditionKind::Flinch) => {
                                label == "flinch"
                            }
                            SecondaryEffect::Volatile(VolatileConditionKind::Confusion) => {
                                label == "start:confusion"
                            }
                            SecondaryEffect::Volatile(VolatileConditionKind::PartiallyTrapped) => {
                                label == "start:partiallytrapped"
                            }
                            SecondaryEffect::Volatile(_) => label.starts_with("start:"),
                            SecondaryEffect::RandomStatus(pool) => pool
                                .iter()
                                .any(|status| label == format!("status:{}", status_id(*status))),
                            SecondaryEffect::StealItem => label == "stolen",
                            // A certain one (Eerie Spell's): nothing is planned from it.
                            SecondaryEffect::EerieSpell => false,
                        };
                        let landed = match (secondary.target, target) {
                            // A multi-hit move's roll after this hit: what showed before the next.
                            (SecondaryTarget::Target, Some(id)) => {
                                // The target's Contrary as the move meets it: a Mold Breaker
                                // user's move ignores it (`ability_running` in the move, R-M10-57).
                                let contrary = in_move.pokemon(*id).is_some_and(|pokemon| {
                                    pokemon_battle_engine::engine::ability_running(
                                        in_move, rules, pokemon,
                                    ) == Some(pokemon_battle_engine::ids::AbilityId::CONTRARY)
                                });
                                oracle.secondary_on.iter().any(|(who, label, hits)| {
                                    who_to_id(state, who) == Some(*id)
                                        && shows(label, contrary)
                                        && (!multi_hit
                                            || *hits == usize::MAX
                                            || *hits == hit_number)
                                })
                            }
                            // Only the effect this secondary causes counts: a contact
                            // reaction's paralysis on the user is not Metal Claw's boost.
                            (SecondaryTarget::User, _) => {
                                // (either direction: Contrary turns a boost into a drop).
                                let caused = |kind: &str| match secondary.effect {
                                    SecondaryEffect::StatStage { stat, .. } => {
                                        stat_changed(kind, stat)
                                    }
                                    SecondaryEffect::Status(_)
                                    | SecondaryEffect::RandomStatus(_) => {
                                        kind.starts_with("status")
                                    }
                                    SecondaryEffect::Volatile(_) => kind.starts_with("start"),
                                    SecondaryEffect::StealItem | SecondaryEffect::EerieSpell => {
                                        false
                                    }
                                };
                                if multi_hit {
                                    // A multi-hit move's roll after this hit: the effect shown
                                    // before the next (Parental Bond Fiery Dance's second).
                                    oracle
                                        .user_applied_hits
                                        .iter()
                                        .any(|(kind, hits)| *hits == hit_number && caused(kind))
                                } else {
                                    let left = user_effects_left[secondary_index]
                                        .get_or_insert_with(|| {
                                            oracle
                                                .applied
                                                .iter()
                                                .filter(|(who, kind)| {
                                                    *who == oracle.user && caused(kind)
                                                })
                                                .count()
                                        });
                                    let landed = *left > 0;
                                    *left = left.saturating_sub(1);
                                    landed
                                }
                            }
                            (SecondaryTarget::Target, None) => oracle.secondary_applied,
                        };
                        // Serene Grace doubles the chance; a certain one asks nothing, nor does
                        // one Gen 2 skips before the roll.
                        let affected = match secondary.target {
                            SecondaryTarget::Target => target.unwrap_or(actor),
                            SecondaryTarget::User => actor,
                        };
                        // A random status (Tri Attack, Dire Claw): which one was drawn. Showdown
                        // shows it when it lands; otherwise the draw is one that changes nothing
                        // — Gen 2 Tri Attack on a frozen target draws a burn exactly when its hit
                        // thaws the target.
                        let random_pick = match (secondary.effect, target) {
                            (SecondaryEffect::RandomStatus(pool), Some(id)) => {
                                let shown = pool.iter().position(|status| {
                                    oracle.secondary_on.iter().any(|(who, label, _)| {
                                        who_to_id(state, who) == Some(*id)
                                            && *label == format!("status:{}", status_id(*status))
                                    })
                                });
                                let refused = || {
                                    let pokemon = state.pokemon(*id)?;
                                    if pokemon.has_status(NonVolatileStatus::Freeze)
                                        && move_def.behavior.status_pick_before_chance
                                    {
                                        let thawed = oracle
                                            .thawed
                                            .iter()
                                            .any(|who| who_to_id(state, who) == Some(*id));
                                        return pool.iter().position(|status| {
                                            (*status == NonVolatileStatus::Burn) == thawed
                                        });
                                    }
                                    if pokemon.status.is_some() {
                                        return Some(0);
                                    }
                                    pool.iter().position(|status| {
                                        pokemon_battle_engine::engine::status_type_immune(
                                            state, rules, *id, *status,
                                        )
                                    })
                                };
                                Some((shown.or_else(refused).unwrap_or(0) as u16, pool, shown))
                            }
                            _ => None,
                        };
                        // Gen 2 Tri Attack's `onHit` draws before the chance, every hit.
                        if let Some((pick, ..)) = random_pick
                            && move_def.behavior.status_pick_before_chance
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Sample, pick),
                            );
                        }
                        let roll = pokemon_battle_engine::engine::secondary_roll(
                            in_move, rules, actor, affected, move_def, secondary,
                        );
                        // A roll the next secondaries share (Silver Wind's five boosts) landed if
                        // any of them shows: a stat already at +6 shows nothing.
                        let group_landed = landed
                            || (secondary.target == SecondaryTarget::User
                                && move_def.secondaries[secondary_index + 1..]
                                    .iter()
                                    .take_while(|next| next.shares_roll)
                                    .any(|next| match next.effect {
                                        // A multi-hit move's: shown after this hit (a Parental
                                        // Bond Ancient Power's boosts after its second hit are
                                        // the second's roll, R-M8-108).
                                        SecondaryEffect::StatStage { stat, .. } if multi_hit => {
                                            oracle.user_applied_hits.iter().any(|(kind, hits)| {
                                                *hits == hit_number && stat_changed(kind, stat)
                                            })
                                        }
                                        SecondaryEffect::StatStage { stat, .. } => {
                                            oracle.applied.iter().any(|(who, kind)| {
                                                *who == oracle.user && stat_changed(kind, stat)
                                            })
                                        }
                                        _ => false,
                                    }));
                        if !secondary.shares_roll
                            && let pokemon_battle_engine::engine::SecondaryRoll::Chance(..) = roll
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::bool(RandomRequestKind::Secondary, group_landed),
                            );
                        }
                        if let Some((pick, pool, shown)) = random_pick {
                            // Otherwise the draw follows a chance that succeeded.
                            if !move_def.behavior.status_pick_before_chance
                                && (landed
                                    || roll
                                        == pokemon_battle_engine::engine::SecondaryRoll::Certain)
                            {
                                staged.push(
                                    Stage::Effect,
                                    RandomOverride::range(RandomRequestKind::Sample, pick),
                                );
                            }
                            // A sleep it put the target to rolls its duration — not one drawn and
                            // stopped (Sweet Veil's `-block` shows it, but no `-status`, R-M11-124).
                            if shown.is_some_and(|index| pool[index] == NonVolatileStatus::Sleep)
                                && oracle.applied.iter().any(|(who, kind)| {
                                    kind == "status:slp" && who_to_id(state, who) == Some(affected)
                                })
                                && let Some(duration) =
                                    duration_for(Some(affected), DurationKind::Sleep)
                            {
                                staged.push(
                                    Stage::Effect,
                                    RandomOverride::range(RandomRequestKind::Duration, duration),
                                );
                            }
                        }
                        // A sleep secondary that took (Secret Power in Grassy Terrain) rolls its
                        // duration — not one a clause stopped after its chance (Sleep Clause Mod:
                        // no `-status`, R-M10-83).
                        if landed
                            && secondary.effect == SecondaryEffect::Status(NonVolatileStatus::Sleep)
                            && oracle.applied.iter().any(|(who, kind)| {
                                kind == "status:slp" && who_to_id(state, who) == Some(affected)
                            })
                            && let Some(duration) =
                                duration_for(Some(affected), DurationKind::Sleep)
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Duration, duration),
                            );
                        }
                        // Poison Puppeteer: a poison secondary that took confuses the target too —
                        // only the holder's (a confusion berry the poison fed it rolls its own,
                        // R-M9-35).
                        if landed
                            && pokemon_battle_engine::engine::poison_puppeteer_confuses(
                                state, rules, actor,
                            )
                            && matches!(
                                secondary.effect,
                                SecondaryEffect::Status(
                                    NonVolatileStatus::Poison | NonVolatileStatus::Toxic
                                )
                            )
                            && target.is_some_and(|id| {
                                oracle.applied.iter().any(|(who, kind)| {
                                    kind == "start:confusion" && who_to_id(state, who) == Some(id)
                                })
                            })
                            && let Some(duration) =
                                duration_for(Some(affected), DurationKind::Confusion)
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Duration, duration),
                            );
                        }
                        // Applying confusion rolls its duration right after the secondary lands.
                        let confuses = matches!(
                            secondary.effect,
                            SecondaryEffect::Volatile(VolatileConditionKind::Confusion)
                        );
                        if landed
                            && confuses
                            && let Some(duration) =
                                duration_for(Some(affected), DurationKind::Confusion)
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Duration, duration),
                            );
                        } else if confuses
                            && !landed
                            && oracle.damages.iter().any(|(who, hp)| {
                                *hp == 0
                                    && target.is_some_and(|id| who_to_id(state, who) == Some(id))
                            })
                            && pokemon_battle_engine::engine::secondary_roll(
                                in_move, rules, actor, affected, move_def, secondary,
                            ) == pokemon_battle_engine::engine::SecondaryRoll::Certain
                        {
                            // A certain confusion the knock-out kept off (Dynamic Punch): a
                            // smaller damage roll the search tries leaves the target standing and
                            // confused, asking for a duration — optional.
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(
                                    RandomRequestKind::Duration,
                                    u16::from(rules.status().lifecycle.confusion_time_min),
                                )
                                .optional(),
                            );
                        }
                        // A bind (Bind / Wrap / Fire Spin …) rolls its duration when it takes.
                        // One that did not because the hit knocked the target out may still
                        // be asked for by a smaller damage roll the search tries: optional.
                        let binds = matches!(
                            secondary.effect,
                            SecondaryEffect::Volatile(VolatileConditionKind::PartiallyTrapped)
                        );
                        let knocked_out = oracle.damages.iter().any(|(who, hp)| {
                            *hp == 0 && target.is_some_and(|id| who_to_id(state, who) == Some(id))
                        });
                        // A multi-hit move binds on the hit that shows it; the later hits find
                        // the target bound and roll nothing (Parental Bond Infestation).
                        let bound = if multi_hit {
                            oracle.secondary_on.iter().any(|(who, label, hits)| {
                                label == "start:partiallytrapped"
                                    && *hits == hit_number
                                    && target.is_some_and(|id| who_to_id(state, who) == Some(id))
                            })
                        } else {
                            oracle
                                .applied
                                .iter()
                                .any(|(_, kind)| kind == "start:partiallytrapped")
                        };
                        // A trapper holding Grip Claw rolls no duration (its fixed turns).
                        let grip_claw =
                            pokemon_battle_engine::engine::held_item_id(state, rules, actor)
                                == Some(pokemon_battle_engine::ids::ItemId::GRIP_CLAW);
                        if binds
                            && bound
                            && !grip_claw
                            && let Some(duration) = duration_for(*target, DurationKind::Bind)
                        {
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(RandomRequestKind::Duration, duration),
                            );
                        } else if binds
                            && knocked_out
                            && !rules.conditions().switching.binding_locks_user
                        {
                            // (Gen 1's bind rolls nothing of its own: the lock's turns are
                            // rolled after the hit, KO or not.)
                            staged.push(
                                Stage::Effect,
                                RandomOverride::range(
                                    RandomRequestKind::Duration,
                                    u16::from(rules.conditions().bind_turns_min),
                                )
                                .optional(),
                            );
                        }
                    }
                    // Wandering Spirit traded on an earlier hit: the target now holds the user's
                    // ability, whose own contact reaction answers the later hits (R-M9-35).
                    let contact_reaction = match oracle.ability_taken_after {
                        Some(after) if oracle.ability_traded && hit >= after => {
                            pokemon_battle_engine::engine::move_makes_contact(
                                state, rules, actor, move_def,
                            ) && state.pokemon(actor).is_some_and(|user| {
                                pokemon_battle_engine::engine::ability_behavior(
                                    rules,
                                    pokemon_battle_engine::engine::raw_ability(user),
                                )
                                .contact_status
                                .is_some()
                            })
                        }
                        _ => contact_reaction,
                    };
                    // A hit on the Substitute asks nothing; but a candidate damage roll that
                    // breaks it sooner than Showdown did — on an earlier hit of the move — reaches
                    // the body here and rolls, so the search meets an optional failed roll instead
                    // of a desynchronized plan. The move's first hit on the target has no earlier
                    // one: it plans nothing, or its optional roll would take the next target's
                    // draw of the same kind (R-M11-88).
                    let body_reachable = !behind_substitute || hit_number > 1;
                    if contact_reaction && behind_substitute && body_reachable {
                        staged.push(
                            Stage::Reaction,
                            RandomOverride::bool(RandomRequestKind::ContactReaction, false)
                                .optional(),
                        );
                    }
                    if contact_reaction
                        && !behind_substitute
                        && !(rules.hits().contact_reaction_needs_damage && dealt == 0)
                    {
                        // The reaction that took is the one after the hit it followed; the
                        // other hits' rolls failed — a later one's changes nothing while the
                        // status stays, and one something cured in between (Lum Berry) would
                        // show a second report (R-M8-134). A spread move reports every hit
                        // before the reactions: the one that took is the holder's.
                        let holder = oracle.contact_status_of.as_ref().is_none_or(|who| {
                            target.is_none_or(|id| who_to_id(state, who) == Some(id))
                        });
                        let took = oracle.contact_status.is_some()
                            && holder
                            && (spread
                                || index
                                    .is_none_or(|i| oracle.contact_status_hits.contains(&(i + 1))));
                        staged.push(
                            Stage::Reaction,
                            RandomOverride::bool(RandomRequestKind::ContactReaction, took),
                        );
                    }
                    if let Some(id) = *target
                        && body_reachable
                    {
                        chance_reactions(
                            &mut staged,
                            ChanceReactionHit {
                                // The user as the hit lands: Protean's type already its own (a
                                // Grass type shrugs off Effect Spore's powder, R-M8-180).
                                state: in_move,
                                rules,
                                actor,
                                target: id,
                                move_def,
                                oracle,
                                behind_substitute,
                                dealt,
                                // Which hit the reported reaction followed: a spread move reports
                                // every hit before the reactions.
                                reported_after: &|after| {
                                    spread || index.is_none_or(|i| i + 1 == after)
                                },
                                sleep_duration: duration_for(Some(actor), DurationKind::Sleep),
                                user_keeps_ability: spread
                                    || oracle
                                        .ability_taken_after
                                        .is_none_or(|after| index.is_none_or(|i| i < after)),
                            },
                            &mut cursed_body_done,
                        );
                    }
                    // A berry the `Update` after this hit fed the target (or the user, hurt by the
                    // hit's reaction) draws after the reactions: a confusion berry's duration,
                    // Starf Berry's stat.
                    for decision in berries_after_reactions {
                        staged.push(Stage::Reaction, decision);
                    }
                    staged.end_hit();
                }
                // A `multiaccuracy` sequence cut short with both sides standing missed its next
                // hit (nothing shows it but the hit count) — unless a hit's reaction put the user
                // to sleep, which stops the sequence before the check (DESIGN.md R263).
                if move_def.multi_accuracy
                    && let MultiHit::Fixed(count) = move_def.multi_hit
                    && landed < u16::from(count)
                    && oracle.contact_status.as_deref() != Some("slp")
                    && rolls_later_accuracy(*target)
                    && !oracle.damages.iter().any(|(who, hp)| {
                        *hp == 0
                            && (*who == oracle.user
                                || target.is_some_and(|id| who_to_id(state, who) == Some(id)))
                    })
                {
                    staged.push(
                        Stage::Gate,
                        RandomOverride::bool(RandomRequestKind::Accuracy, false),
                    );
                }
                staged.end_hit();
            }
            tape.extend(staged.finish());
            // Gen 1: a binding move that reached its target locks a user not locked onto it yet
            // (§30.9), rolling the lock's turns last (a knocked-out target too: the roll comes
            // before the lock's `onStart` finds no foe).
            if rules.conditions().switching.binding_locks_user
                && move_def.behavior.binding_lock
                && let Some((Some(target), TargetFate::Hit)) = targets.first().copied()
                && (oracle
                    .damages
                    .iter()
                    .any(|(who, _)| who_to_id(state, who) == Some(target))
                    || oracle.substitute_hits.iter().any(|hit| *hit))
                && pokemon
                    .volatiles
                    .partial_trapping_lock()
                    .is_none_or(|(locked, ..)| locked != target)
                && let Some(duration) = duration_for(Some(actor), DurationKind::BindingLock)
            {
                tape.push(Some(RandomOverride::range(
                    RandomRequestKind::Duration,
                    duration,
                )));
            }
            // Gen 2-4 crash (`HitRules::crash`): what the move would have dealt its target is
            // computed afresh — a critical-hit check nothing shows (planned as not, the search
            // retries it as one) and a damage roll the crash's HP settles. A target immune to
            // the move's type computes nothing.
            let first_target = targets.iter().find_map(|(target, _)| *target);
            if oracle.crashed
                && matches!(
                    rules.hits().crash,
                    pokemon_battle_engine::rules::CrashDamage::ShareOfDamage { .. }
                )
                && let Some(target) = first_target
                && pokemon_battle_engine::engine::type_immunity(state, rules, target, move_def)
                    .1
                    .is_none()
            {
                if matches!(
                    pokemon_battle_engine::engine::critical_roll(
                        in_move, rules, dex, actor, target, move_def
                    ),
                    pokemon_battle_engine::engine::CriticalRoll::Chance(..)
                ) {
                    tape.push(Some(
                        RandomOverride::bool(RandomRequestKind::Critical, false).optional(),
                    ));
                }
                if !move_def.no_damage_variance {
                    tape.push(None);
                }
                if let Some(decision) = self_focus_band(state, rules, actor, oracle) {
                    tape.push(Some(decision));
                }
            }
        }
    }
    // Forewarn starting as the move hands it over (`ActionOracle::forewarned`): its draw among
    // the holder's foes' strongest moves, which the hand-over leaves as they were.
    for args in &oracle.forewarned {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let id =
            |text: &str| who_from_text(&Value::from(text)).and_then(|who| who_to_id(state, &who));
        if let Some((_, decision)) = forewarn_pick(rules, dex, state, &args, id) {
            tape.push(Some(decision));
        }
    }
    // Gen 4-5 Frisk starting as the move hands it over: its foe's draw (R-M11-123).
    for decision in handed_over_frisk_draws(rules, dex, state, actor, move_def, fates, oracle) {
        tape.push(Some(decision));
    }
    // Magician (the user's `onAfterMoveSecondarySelf`, once the move is over): the Pokémon it hit
    // that share a handler speed each draw a SpeedTie decision (Showdown `speedSort`); the one
    // the item came from goes first. Optional: the ability may not run (an item in hand, a gem).
    if let Some(move_def) = move_def
        && move_def.category != MoveCategory::Status
        && state.pokemon(actor).is_some_and(|user| {
            user.held_item.is_none()
                && pokemon_battle_engine::engine::raw_ability(user)
                    == pokemon_battle_engine::ids::AbilityId::MAGICIAN
        })
    {
        let hit: Vec<PokemonId> = fates
            .iter()
            .filter(|(_, fate)| *fate == TargetFate::Hit)
            .filter_map(|(target, _)| *target)
            .filter(|target| *target != actor)
            .collect();
        let from = oracle
            .magician_from
            .as_ref()
            .and_then(|who| who_to_id(state, who));
        for target in &hit {
            let speed = state.handler_speed(*target);
            if hit
                .iter()
                .any(|other| other != target && state.handler_speed(*other) == speed)
            {
                let rank = if Some(*target) == from { 2 } else { 1 };
                tape.push(Some(
                    RandomOverride::raw(RandomRequestKind::SpeedTie, rank).optional(),
                ));
            }
        }
    }
    // Gen 3+ Thrash's lock ending with its move (Gen 5+ `onAfterMove`, §15.12): the fatigue's
    // confusion. (Gen 1's came as the move started; Gen 3-4's and a stopped Gen 5+ move's come
    // in the residual.)
    if let Some(time) = fatigue_time() {
        tape.push(Some(RandomOverride::range(
            RandomRequestKind::Duration,
            time,
        )));
    }
    // The rest of the action's berry draws (a status move's target; the user, hurt by its
    // recoil; a Pokémon it dragged in onto hazards): drawn at the move's or the action's
    // closing `Update`. Those after the action's drag come after its draw
    // (`berry_draws_after_drag`).
    let before_drag = oracle
        .berry_draws_at_drag
        .unwrap_or(oracle.berry_draws.len());
    for (who, _, draw) in &oracle.berry_draws[..before_drag] {
        let eater = who_to_id(state, who);
        if !berry_confusions_planned.contains(&eater)
            && let Some(decision) =
                berry_decision(*draw, || duration_for(eater, DurationKind::Confusion))
        {
            berry_confusions_planned.push(eater);
            tape.push(Some(decision));
        }
    }
    tape
}

/// The Focus Band roll of damage the user's own action dealt it (a confusion self-hit, a crash):
/// drawn when it would knock the user out — the band held it, or the user fainted.
pub(super) fn self_focus_band(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    oracle: &ActionOracle,
) -> Option<RandomOverride> {
    if pokemon_battle_engine::engine::held_item_id(state, rules, actor)
        != Some(pokemon_battle_engine::ids::ItemId::FOCUS_BAND)
    {
        return None;
    }
    let held = oracle
        .focus_band_held
        .iter()
        .any(|(who, _)| *who == oracle.user);
    let fainted = oracle
        .damages
        .iter()
        .any(|(who, hp)| *who == oracle.user && *hp == 0);
    // Optional, as a hit's: a crash's damage follows the move's damage roll, and the rolls the
    // search tries need not knock the user out.
    (held || fainted).then(|| RandomOverride::bool(RandomRequestKind::FocusBand, held).optional())
}

/// The decision a berry's eating draws: a confusion berry's duration (as `confusion` finds it),
/// Starf Berry's stat.
/// The berries eaten at the `Update` after the action's drag (`berry_draws_at_drag`): a Roar
/// that dragged out the foe whose Unnerve held a confusion berry back (R-M11-60). Showdown drags
/// at the action's end, then runs `Update`: these come after the drag's RandomTarget.
pub(super) fn berry_draws_after_drag(
    state: &BattleState,
    rules: &ResolvedRules,
    oracle: &ActionOracle,
    duration_hints: &[(PokemonId, DurationKind, u16)],
) -> Vec<Option<RandomOverride>> {
    let Some(at) = oracle.berry_draws_at_drag else {
        return Vec::new();
    };
    let mut planned: Vec<Option<PokemonId>> = Vec::new();
    let mut tape = Vec::new();
    for (who, _, draw) in &oracle.berry_draws[at..] {
        let eater = who_to_id(state, who);
        if planned.contains(&eater) {
            continue;
        }
        let duration = || {
            duration_hints
                .iter()
                .find(|(victim, class, _)| {
                    *class == DurationKind::Confusion && eater.is_none_or(|e| e == *victim)
                })
                .map(|(_, _, hint)| *hint)
                .or(Some(u16::from(rules.status().lifecycle.confusion_time_min)))
        };
        if let Some(decision) = berry_decision(*draw, duration) {
            planned.push(eater);
            tape.push(Some(decision));
        }
    }
    tape
}

pub(super) fn berry_decision(
    draw: BerryDraw,
    confusion: impl FnOnce() -> Option<u16>,
) -> Option<RandomOverride> {
    match draw {
        BerryDraw::Confusion => {
            confusion().map(|duration| RandomOverride::range(RandomRequestKind::Duration, duration))
        }
        BerryDraw::Stat(code) => Some(RandomOverride::range(RandomRequestKind::Sample, code)),
    }
}

/// Recovers the confusion `time` Showdown rolled when `decisions[index]` confused `victim`:
/// the value shown in the boundary frame plus one for every BeforeMove tick the victim went
/// through afterwards this turn (a tick happens unless sleep, freeze or flinch stopped the
/// action first). If the confusion already ended within the turn, the ticks alone give it.
/// `member`: the victim's party index in the frame (`frame_member`).
pub(super) fn confusion_duration_hint(
    rules: &ResolvedRules,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    index: usize,
    victim: &Who,
    member: Option<usize>,
) -> Option<u16> {
    confusion_hint_after(rules, oracle_frame, &decisions[index + 1..], victim, member)
}

/// The confusion `time` rolled for `victim` before the actions `later`: the frame's, plus a tick
/// for each of them that reached its BeforeMove confusion check. A later lock's fatigue that
/// confuses the victim afresh (Gen 1's `-start|p|confusion|[silent]` replaces the time) leaves
/// the frame to the new confusion: this one only had to outlast the checks up to it.
/// The victim is read in the frame at `member`, its party index there (`None`: off the field);
/// its actions and events are matched by side and name, as an Ally Switch may move it to the
/// other slot within the turn (R-M8-175).
pub(super) fn confusion_hint_after(
    rules: &ResolvedRules,
    oracle_frame: &Value,
    later: &[ActionOracle],
    victim: &Who,
    member: Option<usize>,
) -> Option<u16> {
    if let Some(replaced) = later
        .iter()
        .position(|action| same_pokemon(&action.user, victim) && action.fatigue)
    {
        let ticks = confusion_ticks(&later[..=replaced], victim);
        return Some((ticks + 1).max(u16::from(rules.status().lifecycle.confusion_time_min)));
    }
    let later_ticks = confusion_ticks(later, victim);
    // Off the field (the victim fell, no one in its place yet) shows no confusion.
    let at_boundary = member.and_then(|member| {
        oracle_frame["sides"][victim.side]["party"][member]["volatiles"]["confusion"].as_u64()
    });
    match at_boundary {
        Some(time) => Some(time as u16 + later_ticks),
        None => {
            let events = oracle_frame["events"].as_array()?;
            let mut ended = None;
            for (index, event) in events.iter().enumerate() {
                if event["kind"] == "end"
                    && event["args"][1] == "confusion"
                    && who_from_text(&event["args"][0])
                        .is_some_and(|who| same_pokemon(&who, victim))
                {
                    ended = Some(index);
                }
            }
            // Ended by a berry the victim ate (Persim; Lum after its status cure): any time
            // behaves the same.
            let eaten = |index: usize| {
                events[..index]
                    .iter()
                    .rev()
                    .take(2)
                    .find(|event| event["kind"] != "curestatus")
                    .is_some_and(|event| {
                        berry_eaten(event).is_some_and(|(eater, _)| eater == *victim)
                    })
            };
            // Ended by none of the victim's own checks (Gen 1 Haze, before it moved again):
            // any time behaves the same.
            // (An ability that cures it at once, Own Tempo behind a Mold Breaker's Teeter Dance:
            // no check ever ticked it — any time in the range behaves the same, R-M8-127.)
            let shortest = u16::from(rules.status().lifecycle.confusion_time_min);
            match ended {
                Some(index) => (!eaten(index))
                    .then(|| later_ticks.max(shortest))
                    .filter(|ticks| *ticks > 0),
                // Never ended: still confused as it left the field (fainted, switched or dragged
                // out — the volatile went with it), so longer than the checks it went through —
                // a dance's and its own action's (R-M8-156).
                None => Some((later_ticks + 1).max(shortest)),
            }
        }
    }
}

/// The actions of `victim` among `actions` that reached its BeforeMove confusion check.
pub(super) fn confusion_ticks(actions: &[ActionOracle], victim: &Who) -> u16 {
    actions
        .iter()
        .filter(|action| same_pokemon(&action.user, victim))
        // A turn stopped before confusion's own BeforeMove step (Showdown priority 3: recharge,
        // sleep, freeze, Truant, flinch, Disable, Heal Block, Throat Chop, Gravity, Taunt,
        // Imprison; the cartridges' order alike, issues.md I66, I105) does not tick it — nor does
        // a Gen 1 turn spent waking up.
        .filter(|action| {
            let before_confusion = matches!(
                action.cant.as_deref(),
                Some(
                    "recharge"
                        | "slp"
                        | "frz"
                        | "ability: Truant"
                        | "flinch"
                        | "Disable"
                        | "move: Heal Block"
                        | "move: Throat Chop"
                        | "move: Gravity"
                        | "move: Taunt"
                        | "move: Imprison"
                )
            );
            !before_confusion && !woke_without_acting(action)
        })
        .count() as u16
}

/// A Gen 1 turn spent waking up: the gen1 sleep check refuses the move even as the Pokémon
/// wakes (no `cant`, a `-curestatus`, no move), before any later BeforeMove check counts.
pub(super) fn woke_without_acting(action: &ActionOracle) -> bool {
    action.cured.as_deref() == Some("slp") && action.move_name.is_none() && action.cant.is_none()
}

/// Recovers Gen 1 Disable's `time` (1-8), counted down at every BeforeMove of the victim that
/// gets past sleep (the waking turn included), freeze, flinch, a partial trap and the recharge
/// (Showdown priorities 10-7 over Disable's 6): the frame's time plus the victim's later actions in the boundary. Ended by
/// the frame (`-end`): the actions that counted it out; gone without ending (the victim left):
/// one more, as any larger time behaves the same.
pub(super) fn gen1_disable_hint(
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    index: usize,
    victim: &Who,
) -> Option<u16> {
    let later_ticks = decisions[index + 1..]
        .iter()
        .filter(|action| action.user == *victim)
        .filter(|action| {
            !matches!(
                action.cant.as_deref(),
                Some("slp" | "frz" | "flinch" | "partiallytrapped" | "recharge")
            ) && !woke_without_acting(action)
        })
        .count() as u16;
    let side = &oracle_frame["sides"][victim.side];
    let at_boundary = victim
        .slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|party_index| {
            side["party"][party_index as usize]["volatiles"]["disable"].as_u64()
        });
    let ended = oracle_frame["events"].as_array().is_some_and(|events| {
        events.iter().any(|event| {
            event["kind"] == "end"
                && event["args"][1] == "Disable"
                && who_from_text(&event["args"][0]).as_ref() == Some(victim)
        })
    });
    Some(match at_boundary {
        Some(time) => time as u16 + later_ticks,
        None if ended => later_ticks,
        None => later_ticks + 1,
    })
}

/// Recovers the Taunt / Encore / Disable duration Showdown rolled (Gen 3-4): the boundary
/// frame's turns plus the end-of-turn tick already taken (none at a mid-turn request, nor when
/// `ticked` says the battle ended in the residual before it), minus the `onStart` adjustment the
/// engine computes for the target at that moment. Gone by the frame (the victim left): the
/// rule's minimum reproduces the same frames.
pub(super) fn timed_duration_hint(
    oracle_frame: &Value,
    before: &BattleState,
    victim: &Who,
    volatile: &str,
    rule: pokemon_battle_engine::rules::TimedVolatileRules,
    ticked: bool,
) -> Option<u16> {
    let pokemon_battle_engine::rules::TimedDuration::Rolled { min, max } = rule.duration else {
        return None;
    };
    let side = &oracle_frame["sides"][victim.side];
    let party_index = side["active"][victim.slot?].as_u64()? as usize;
    let at_boundary = side["party"][party_index]["volatiles"][volatile].as_u64();
    let target = who_to_id(before, victim)?;
    let adjust = pokemon_battle_engine::engine::duration_adjustment(before, rule.adjust, target);
    let ticked = i16::from(ticked && residual_passed(oracle_frame));
    let rolled = match at_boundary {
        Some(turns) => turns as i16 + ticked - adjust,
        None => i16::from(min),
    };
    Some(rolled.clamp(i16::from(min), i16::from(max)) as u16)
}

/// A turn-end tick the battle, decided in the residual, never got to (R-M8-121).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MissedTick {
    /// A residual handler's: Taunt, Encore, Disable, a bind, a Gen 3-4 rampage.
    Residual(ResidualKind),
    /// Gen 2+ Bide's `duration`, a duration-only handler: Showdown runs the one handler after the
    /// knock-out that decided it (Perish Song's `onEnd` prints no faint until the next handler is
    /// over), so whether it ticked depends on the order — read from the engine (R-M8-158).
    Bide,
}

/// The Gen 2+ Bide users whose Bide started in these trace events but whose `duration` the
/// engine left as the plan drew it (`bide_duration_hint` with its tick): the residual that
/// decided the battle stopped before their turn-end tick.
pub(super) fn bides_left_unticked(
    state: &BattleState,
    rules: &ResolvedRules,
    oracle_frame: &Value,
    events: &[TraceEvent],
) -> Vec<(PokemonId, MissedTick)> {
    use pokemon_battle_engine::state::VolatileConditionKind as V;
    if rules.conditions().retaliation.bide.gen1 {
        return Vec::new();
    }
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::VolatileStarted {
                target,
                kind: V::Bide,
            } => Some(*target),
            _ => None,
        })
        .filter(|target| {
            let Some(pokemon) = state.pokemon(*target) else {
                return false;
            };
            let Some(spec) = state
                .active_position(*target, rules.topology())
                .and_then(|position| rules.topology().spec(position))
            else {
                return false;
            };
            let who = Who {
                side: spec.side.index(),
                slot: Some(usize::from(spec.slot)),
                name: String::new(),
            };
            let drawn = bide_duration_hint(oracle_frame, rules, &who, true);
            pokemon
                .volatiles
                .bide()
                .and_then(|bide| bide.turns)
                .is_some_and(|turns| u16::from(turns) == drawn)
        })
        .map(|target| (target, MissedTick::Bide))
        .collect()
}

/// The Pokémon whose Taunt / Encore / Disable, bind, Gen 3-4 rampage or Uproar started in these trace
/// events but never reached its end-of-turn tick (the battle was decided in the residual before
/// it), with the tick it missed.
pub(super) fn timed_volatiles_left_unticked(events: &[TraceEvent]) -> Vec<(PokemonId, MissedTick)> {
    use pokemon_battle_engine::rules::ResidualKind;
    use pokemon_battle_engine::state::VolatileConditionKind as V;
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::VolatileStarted { target, kind } => match kind {
                V::Taunt => Some((*target, ResidualKind::Taunt)),
                V::Encore => Some((*target, ResidualKind::Encore)),
                V::Disable => Some((*target, ResidualKind::Disable)),
                V::PartiallyTrapped => Some((*target, ResidualKind::PartiallyTrapped)),
                V::Rampage => Some((*target, ResidualKind::Rampage)),
                V::Uproar => Some((*target, ResidualKind::Uproar)),
                _ => None,
            },
            _ => None,
        })
        .filter(|(target, residual)| {
            !events.iter().any(|event| {
                matches!(event, TraceEvent::ResidualEffect { holder, kind, .. }
                    if holder == target && kind == residual)
            })
        })
        .map(|(target, residual)| (target, MissedTick::Residual(residual)))
        .collect()
}

/// Whether the boundary frame comes after the end-of-turn residual (its events carry the next
/// `turn`, or the `residual` marker; fixtures recorded before the marker existed have only the
/// former): a frame taken at a mid-turn request (U-turn, Volt Switch), or when the battle was
/// won before the end of the turn, comes before the duration ticks.
pub(super) fn residual_passed(oracle_frame: &Value) -> bool {
    oracle_frame["events"].as_array().is_some_and(|events| {
        let turn = events.iter().any(|event| event["kind"] == "turn");
        // A battle won before the end of the turn prints the marker (`win`'s blank line) and
        // nothing after it; a residual that printed nothing before a replacement request is
        // followed by nothing either, but the battle goes on.
        let decided = !oracle_frame["outcome"].is_null();
        let residual_ran = events
            .iter()
            .position(|event| event["kind"] == "residual")
            .is_some_and(|marker| marker + 1 < events.len() || !decided);
        turn || residual_ran
    })
}

/// Recovers the bind duration Showdown rolled: the boundary frame's `partiallytrapped` plus the
/// tick the end of that turn already took (none yet at a mid-turn request, nor when `ticked`
/// says a battle decided in the residual stopped before it). A bind that ended within the turn
/// (the trapper left) leaves no trace; the generation's minimum then reproduces the same frames.
/// The victim is read at `member`, its party index in the frame (`frame_member`).
pub(super) fn bind_duration_hint(
    oracle_frame: &Value,
    rules: &ResolvedRules,
    victim: &Who,
    member: Option<usize>,
    ticked: bool,
) -> Option<u16> {
    // The victim may be gone from the field by the boundary (fainted to the bind itself).
    let at_boundary = member.and_then(|member| {
        oracle_frame["sides"][victim.side]["party"][member]["volatiles"]["partiallytrapped"]
            .as_u64()
    });
    let ticked = u16::from(ticked && residual_passed(oracle_frame));
    let (min, max) = (
        u16::from(rules.conditions().bind_turns_min),
        u16::from(rules.conditions().bind_turns_max),
    );
    // A first reading that assumed a tick the residual never reached may overshoot the range;
    // the boundary is then read again unticked.
    Some(match at_boundary {
        Some(turns) => (turns as u16 + ticked).clamp(min, max),
        None => min,
    })
}

/// Recovers the turns Gen 1's binding lock rolled: the user's `partialtrappinglock` at the
/// boundary plus the end-of-turn tick (none when a faint cut the turn short). A lock gone by the
/// boundary (its target knocked out) leaves no trace; the shortest then reproduces the frames.
pub(super) fn binding_lock_hint(oracle_frame: &Value, user: &Who) -> u16 {
    let side = &oracle_frame["sides"][user.side];
    let left = user
        .slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|index| {
            side["party"][index as usize]["volatiles"]["partialtrappinglock"].as_u64()
        });
    match left {
        Some(turns) => (turns as u16 + u16::from(residual_passed(oracle_frame))).clamp(2, 5),
        None => 2,
    }
}

/// The `volatiles` of `who` in a boundary frame.
pub(super) fn frame_volatiles<'a>(frame: &'a Value, who: &Who) -> Option<&'a Value> {
    frame_pokemon(frame, who).map(|pokemon| &pokemon["volatiles"])
}

/// The active Pokémon `who` names, as the frame shows it.
pub(super) fn frame_pokemon<'a>(frame: &'a Value, who: &Who) -> Option<&'a Value> {
    let side = &frame["sides"][who.side];
    let index = side["active"][who.slot?].as_u64()? as usize;
    Some(&side["party"][index])
}

/// Recovers the length a lock (Thrash and its kind, §15.12) drew as it started this turn, from
/// `frames` / `decisions` / `residuals` (this boundary's first). Gen 1: the uses left in the
/// frame (`time`, not yet counted down); Gen 2 one more. Gen 3-4 (the cartridge's count): the frame's count plus
/// the end of the turn's, if it ran (not at a replacement request mid-turn). Gen 5+ (the hidden length): only a later turn tells — a fatigue there
/// (in the user's action or the residual) means 2, a lock still there after it 3, a lock that
/// ended silently after a hit 2 (the confusion did not take) unless the user fell asleep
/// unconfused (the sleep ended it in the residual: 3), otherwise 3; a lock gone with no action
/// of its user's and no fatigue ran out on a 3. Where nothing tells, any length behaves the
/// same: 2.
pub(super) fn rampage_length_hint(
    rules: &ResolvedRules,
    frames: &[Option<&Value>],
    decisions: &[Vec<ActionOracle>],
    residuals: &[Vec<(Who, ResidualKind)>],
    who: &Who,
    ticked: bool,
) -> u16 {
    use pokemon_battle_engine::rules::RampageCounter;
    let locked = |frame: Option<&Value>| {
        frame
            .and_then(|frame| frame_volatiles(frame, who))
            .and_then(|volatiles| volatiles["lockedmove"].as_u64())
    };
    match rules.conditions().rampage.counter {
        RampageCounter::Gen1Uses => {
            locked(frames.first().copied().flatten()).map_or(2, |uses| uses.clamp(2, 3) as u16)
        }
        // Gen 2: the uses after the first (the drawn length less one).
        RampageCounter::Gen2Uses => locked(frames.first().copied().flatten())
            .map_or(2, |more| (more as u16 + 1).clamp(2, 3)),
        RampageCounter::Cartridge => {
            let frame = frames.first().copied().flatten();
            // The end of the turn's count-down has not run at a replacement request the
            // turn's faint made (Showdown: before the residual), nor when the battle was decided
            // in the residual before it (`ticked`: `timed_volatiles_left_unticked`, R-M8-121).
            let ticked = ticked && frame.is_some_and(residual_passed);
            locked(frame).map_or(2, |count| (count as u16 + u16::from(ticked)).clamp(2, 3))
        }
        RampageCounter::HiddenLength => {
            // The user by its place in the party: an Ally Switch moves it to the other slot,
            // where the later turns' actions, residuals and frames name it (R-M8-152).
            let member = frames
                .first()
                .copied()
                .flatten()
                .and_then(|frame| frame["sides"][who.side]["active"][who.slot?].as_u64());
            fn slot_of(frame: &Value, who: &Who, member: Option<u64>) -> Option<usize> {
                match member {
                    Some(member) => frame["sides"][who.side]["active"]
                        .as_array()?
                        .iter()
                        .position(|index| index.as_u64() == Some(member)),
                    None => who.slot,
                }
            }
            // The user as `frame` shows it: none once it has left the field.
            fn user_in<'f>(frame: &'f Value, who: &Who, member: Option<u64>) -> Option<&'f Value> {
                match member {
                    Some(member) => slot_of(frame, who, Some(member))
                        .map(|_| &frame["sides"][who.side]["party"][member as usize]),
                    None => frame_pokemon(frame, who),
                }
            }
            let slot_in = |frame: &Value| slot_of(frame, who, member);
            let locked = |frame: Option<&Value>| {
                frame
                    .and_then(|frame| user_in(frame, who, member))
                    .and_then(|pokemon| pokemon["volatiles"]["lockedmove"].as_u64())
            };
            // The user's stopped action at a replacement request mid-turn (a Baton Pass after it):
            // the end of that turn, in the next boundary, still has to tell.
            let mut carried: Option<&ActionOracle> = None;
            for (index, frame) in frames.iter().enumerate().skip(1) {
                // Where it stood as the turn ran: its slot at the boundary, or — dragged out or
                // gone by then — the one it had at the boundary before.
                let user = Who {
                    side: who.side,
                    slot: frame
                        .and_then(slot_in)
                        .or_else(|| frames[index - 1].and_then(slot_in))
                        .or(who.slot),
                    name: who.name.clone(),
                };
                if residuals.get(index).is_some_and(|r| {
                    r.iter()
                        .any(|(w, k)| *w == user && *k == ResidualKind::Rampage)
                }) {
                    return 2;
                }
                // Its action that turn, by the slot it had then: an Ally Switch later in the turn
                // (its partner's) has it in the other slot at the boundary (R-M11-96).
                let before = Who {
                    slot: frames[index - 1].and_then(slot_in).or(user.slot),
                    ..user.clone()
                };
                let action = decisions
                    .get(index)
                    .and_then(|actions| {
                        actions
                            .iter()
                            .find(|action| action.user == user)
                            .or_else(|| actions.iter().find(|action| action.user == before))
                    })
                    .or(carried.take());
                if let Some(action) = action {
                    if action.fatigue {
                        return 2;
                    }
                    if locked(*frame).is_some() {
                        // A turn the lock was stopped (Disable's `cant`) ends it with the turn's
                        // residual, not in the action (no `onRestart`, no `onAfterMove`): before
                        // that, the lock is still there whatever its length — a 2 shows its
                        // fatigue there (R-M8-105). A use that went on decided in its AfterMove.
                        if action.cant.is_some()
                            && frame.is_some_and(|frame| !residual_passed(frame))
                        {
                            carried = Some(action);
                            continue;
                        }
                        return 3;
                    }
                    // Off the field by the boundary after it moved (a Dancer's dance knocked it
                    // out; a Red Card dragged it out, another standing in its slot, R-M8-136): its
                    // lock went through AfterMove, where a 2 shows its fatigue — unless it was
                    // confused already, when either length leaves the same (R-M8-78).
                    let in_slot = |frame: Option<&Value>| {
                        frame.and_then(|frame| {
                            frame["sides"][who.side]["active"][who.slot?].as_u64()
                        })
                    };
                    let gone = frame.is_some_and(|frame| {
                        user_in(frame, who, member).is_none_or(|p| p["hp"] == 0)
                    }) || (member.is_none()
                        && in_slot(*frame) != in_slot(frames[index - 1]));
                    if gone {
                        let confused_before = frames[index - 1].is_some_and(|frame| {
                            user_in(frame, who, member)
                                .is_some_and(|p| p["volatiles"].get("confusion").is_some())
                        });
                        return if confused_before { 2 } else { 3 };
                    }
                    let landed = action.damages.iter().any(|(target, _)| *target != user)
                        || action.substitute_hits.iter().any(|hit| *hit);
                    // A lock gone silently from a user asleep and not confused at the
                    // boundary went on after its hit and lost to the sleep at the end of the
                    // turn (`lockedmove.onResidual`): 3. (Confused already, the user would
                    // show a 2's fatigue no more — either length leaves the same then.)
                    let asleep_unconfused = frame.is_some_and(|frame| {
                        user_in(frame, who, member).is_some_and(|pokemon| {
                            pokemon["status"]["kind"] == "slp"
                                && pokemon["volatiles"].get("confusion").is_none()
                        })
                    });
                    return if landed && !asleep_unconfused { 2 } else { 3 };
                }
                // Gone with no action of its user's this turn — one Sky Drop carries cannot move,
                // and its `onFoeBeforeMove` says nothing — and no fatigue in the residual (above):
                // its `duration` ran out on a hidden length still above 1, so 3 (R-M10-76). A
                // fatigue the confusion could not take, or a lock the user lost leaving the field,
                // leaves the same with either.
                if locked(*frame).is_none() {
                    return 3;
                }
            }
            2
        }
    }
}

/// Recovers Bide's draw (§15.10): the user's `bide` at the boundary — Gen 1's `time`, which only
/// its later turns count down, or Gen 2's `duration` plus the end-of-turn tick. Gone by the
/// boundary (the user switched out or fainted), the shortest reproduces the frames.
pub(super) fn bide_duration_hint(
    oracle_frame: &Value,
    rules: &ResolvedRules,
    user: &Who,
    ticked: bool,
) -> u16 {
    let bide = rules.conditions().retaliation.bide;
    let side = &oracle_frame["sides"][user.side];
    let left = user
        .slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|index| side["party"][index as usize]["volatiles"]["bide"].as_u64());
    // `ticked`: not when a battle the residual decided stopped before Bide's `duration`
    // (`MissedTick::Bide`, R-M8-158).
    let ticked = u16::from(!bide.gen1 && ticked && residual_passed(oracle_frame));
    match left {
        Some(turns) => {
            (turns as u16 + ticked).clamp(u16::from(bide.turns.0), u16::from(bide.turns.1))
        }
        None => u16::from(bide.turns.0),
    }
}

/// Recovers the turns Gen 3-4 Uproar drew as it started (`UproarRules::turns`): the user's
/// `uproar` at the boundary, plus the tick its turn-end step already took — none when a battle
/// the residual decided stopped before it (`ticked`: `timed_volatiles_left_unticked`,
/// R-M8-163). Gone by then (the user fainted, or Gen 3 cancelled it): any value behaves the
/// same, so the shortest.
pub(super) fn uproar_turns_hint(
    oracle_frame: &Value,
    rules: &ResolvedRules,
    user: &Who,
    ticked: bool,
) -> u16 {
    let (low, high) = rules.conditions().uproar.turns;
    let side = &oracle_frame["sides"][user.side];
    let left = user
        .slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|index| side["party"][index as usize]["volatiles"]["uproar"].as_u64());
    let ticked = u16::from(ticked && residual_passed(oracle_frame));
    match left {
        Some(turns) => (turns as u16 + ticked).clamp(u16::from(low), u16::from(high)),
        None => u16::from(low),
    }
}

/// Recovers the sleep `time` Showdown rolled when `all[turn][index]` put `victim` to sleep:
/// one more than the number of `cant slp` actions before the `-curestatus slp`. If the victim
/// is still asleep when the scenario ends, any time of at least that many counts behaves
/// identically within the scenario, so the smallest such value (capped by the generation's
/// maximum) is used. Each `cant slp` took its turn's `step_at` off the time (Early Bird: 2 —
/// a Skill Swap may give it or take it away during the sleep), so a wake-up after those says
/// the time was above what they took and at most one more step; the top of that range is used.
#[allow(clippy::too_many_arguments)]
pub(super) fn sleep_duration_hint(
    all: &[Vec<ActionOracle>],
    entries: &[Vec<(usize, Who)>],
    residuals: &[Vec<(Who, ResidualKind)>],
    turn: usize,
    index: usize,
    victim: &Who,
    resets_on_switch_in: bool,
    sleep_usable_turns_come_back: bool,
    min_time: u16,
    max_time: u16,
    step_at: &dyn Fn(usize) -> u16,
) -> Option<u16> {
    // The same Pokémon by side and name: it may come back into another slot after a switch.
    let same = |who: &Who| who.side == victim.side && who.name == victim.name;
    let mut cants: u16 = 0;
    // What the `cant slp`s took off the time so far.
    let mut taken: u16 = 0;
    // Gen 5 restarts the count at each entry; a stretch that ended without waking up still
    // says the rolled time is longer than it.
    let mut at_least: u16 = 0;
    // Gen 3: the turns spent on Snore / Sleep Talk since sleep last stopped it, given back at
    // the next entry (`skippedTime`, R-M8-79).
    let mut skipped: u16 = 0;
    for (t, decisions) in all.iter().enumerate().skip(turn) {
        let start = if t == turn { index + 1 } else { 0 };
        let entered = entries.get(t).map_or(&[][..], Vec::as_slice);
        for position in start..=decisions.len() {
            let comes_in = entered
                .iter()
                .any(|(before, who)| *before == position && same(who));
            // Gen 5: the counter goes back to the rolled time as the sleeper comes in.
            if resets_on_switch_in && comes_in {
                at_least = at_least.max(taken + 1);
                cants = 0;
                taken = 0;
            }
            if sleep_usable_turns_come_back && comes_in {
                // The sleep the turns given back had not ended still says the time is longer
                // than what was taken before them (R-M8-152).
                at_least = at_least.max(taken + 1);
                taken = taken.saturating_sub(skipped);
                skipped = 0;
            }
            let Some(action) = decisions.get(position) else {
                // The turn's residual cured it (Healer, Shed Skin, an uproar) or put it to sleep
                // anew (Yawn): the sleep ended without a wake-up of its own, so any time above
                // the turns seen behaves the same.
                if residuals.get(t).is_some_and(|done| {
                    done.iter().any(|(who, kind)| {
                        same(who)
                            && matches!(
                                kind,
                                ResidualKind::Healer
                                    | ResidualKind::ShedSkin
                                    | ResidualKind::Yawn
                                    | ResidualKind::Uproar
                            )
                    })
                }) {
                    return Some((taken + 1).max(at_least).clamp(min_time, max_time));
                }
                break;
            };
            // A turn it slept through counts before what the action then did: a Sleep Talk
            // that calls Rest sleeps anew after its BeforeMove took the turn (R-M11-32).
            let slept_here = same(&action.user)
                && (action.cant.as_deref() == Some("slp") || action.slept_through);
            if slept_here {
                cants += 1;
                taken += step_at(t);
                skipped = if action.slept_through { skipped + 1 } else { 0 };
            }
            // Put to sleep again, or cured by another's move (Purify): the first sleep ended
            // without a wake-up of its own (Shed Skin, a switch-out cure …), so any time above
            // the turns seen behaves the same.
            if action
                .applied
                .iter()
                .any(|(who, kind)| same(who) && (kind == "status:slp" || kind == "curestatus:slp"))
            {
                return Some((taken + 1).max(at_least).clamp(min_time, max_time));
            }
            if !same(&action.user) || slept_here {
                continue;
            }
            // Gen 3-4 (issues.md I63): an uproar woke it as it was about to move, before this
            // BeforeMove counted: any time that does not run out here behaves the same.
            if action
                .applied
                .iter()
                .any(|(who, kind)| same(who) && kind == "uproar:slp")
            {
                return Some(
                    (taken + step_at(t) + 1)
                        .max(at_least)
                        .clamp(min_time, max_time),
                );
            }
            if action.cured.as_deref() == Some("slp") {
                // Early Bird's range may reach past the generation's maximum.
                return Some((taken + step_at(t)).min(max_time.max(cants + 1)));
            }
        }
    }
    // Never woke within the scenario (switched out, knocked out, battle over): any time above
    // the sleeping turns seen behaves the same; keep it inside the generation's range.
    Some((taken + 1).max(at_least).clamp(min_time, max_time))
}

/// How much each BeforeMove takes off `victim`'s sleep (Early Bird: 2), turn by turn: as the
/// engine has it now for this turn, and for a later one as the frame before it
/// (`boundary_frames[t - 1]`) shows the Pokémon — its ability (a Skill Swap during the sleep),
/// Gastro Acid, and a Neutralizing Gas on the field.
pub(super) fn sleep_steps<'a>(
    state: &'a BattleState,
    rules: &'a ResolvedRules,
    victim: &'a Who,
    turn: usize,
    boundary_frames: &'a [Option<&'a Value>],
) -> impl Fn(usize) -> u16 + 'a {
    let id = who_to_id(state, victim);
    let pokemon = id.and_then(|id| state.pokemon(id));
    let now = pokemon.is_some_and(|p| {
        pokemon_battle_engine::engine::ability_active(state, rules, p)
            == Some(pokemon_battle_engine::ids::AbilityId::EARLY_BIRD)
    });
    let party = id.and_then(|id| {
        state
            .side(if victim.side == 0 {
                SideId::P1
            } else {
                SideId::P2
            })
            .team
            .iter()
            .position(|member| *member == id)
    });
    move |t: usize| {
        let frame = (t > turn)
            .then(|| t.checked_sub(1))
            .flatten()
            .and_then(|before| boundary_frames.get(before).copied().flatten());
        let shown = frame
            .zip(party)
            .map(|(frame, party)| &frame["sides"][victim.side]["party"][party]);
        let early = match (frame, shown) {
            // As the frame before that turn has it: the ability it shows (a Skill Swap during
            // the sleep), not set aside by Gastro Acid nor by a Neutralizing Gas on the field
            // (one that has left since the sleep began counts no more, R-M8-126) — save behind
            // Ability Shield.
            (Some(frame), Some(entry)) if entry["ability"].as_str().is_some() => {
                entry["ability"] == "earlybird"
                    && entry["volatiles"].get("gastroacid").is_none()
                    && (entry["item"] == "abilityshield" || !neutralizing_gas_shown(frame))
            }
            _ => now,
        };
        if early { 2 } else { 1 }
    }
}

/// Whether a boundary frame shows a Neutralizing Gas at work: an active Pokémon standing with the
/// ability, not under Gastro Acid.
pub(super) fn neutralizing_gas_shown(frame: &Value) -> bool {
    frame["sides"].as_array().is_some_and(|sides| {
        sides.iter().any(|side| {
            side["active"].as_array().is_some_and(|active| {
                active.iter().filter_map(Value::as_u64).any(|index| {
                    let pokemon = &side["party"][index as usize];
                    pokemon["ability"] == "neutralizinggas"
                        && pokemon["hp"].as_u64().is_some_and(|hp| hp > 0)
                        // (A volatile with no value shows as `null`: its key is what counts.)
                        && pokemon["volatiles"].get("gastroacid").is_none()
                })
            })
        })
    })
}

/// One hit that reached a target's body, for the chance reactions that follow it.
pub(super) struct ChanceReactionHit<'a> {
    pub(super) state: &'a BattleState,
    pub(super) rules: &'a ResolvedRules,
    pub(super) actor: PokemonId,
    pub(super) target: PokemonId,
    pub(super) move_def: &'a ResolvedMove,
    pub(super) oracle: &'a ActionOracle,
    pub(super) behind_substitute: bool,
    /// The HP the hit took off its target.
    pub(super) dealt: u16,
    /// Whether a reaction reported after this many hits belongs to this hit (R-M8-134: not a
    /// later one, whose roll failed — Mental Herb may have cured what an earlier one took).
    pub(super) reported_after: &'a dyn Fn(usize) -> bool,
    pub(super) sleep_duration: Option<u16>,
    /// The user still has the ability it attacked with: no Mummy (or its kind) took it on an
    /// earlier hit (`ActionOracle::ability_taken_after`).
    pub(super) user_keeps_ability: bool,
}

/// The chance reactions after a hit's ordered ones (engine `hit_reactions`), in the engine's
/// order: the target's Effect Spore or Cursed Body, then the user's Poison Touch / Toxic Chain.
/// The one that took is the one reported after this hit; the other hits' rolls failed (a later
/// one changes nothing while the status stays; Cursed Body stops rolling once the move is
/// disabled). A hit on a Substitute reacts to
/// nothing, but a candidate damage roll that breaks it sooner meets them: optional decisions.
pub(super) fn chance_reactions(
    staged: &mut StagedTape,
    hit: ChanceReactionHit<'_>,
    cursed_body_done: &mut bool,
) {
    use pokemon_battle_engine::rules::EffectSporeRoll;
    let ChanceReactionHit {
        state,
        rules,
        actor,
        target,
        move_def,
        oracle,
        behind_substitute,
        dealt,
        reported_after,
        sleep_duration,
        user_keeps_ability,
    } = hit;
    let named = |who: &Option<Who>, id: PokemonId| {
        who.as_ref()
            .is_none_or(|who| who_to_id(state, who) == Some(id))
    };
    let push = |staged: &mut StagedTape, decision: RandomOverride| {
        staged.push(
            Stage::Reaction,
            if behind_substitute {
                decision.optional()
            } else {
                decision
            },
        );
    };
    let mut reactors = pokemon_battle_engine::engine::chance_reactors(state, rules, actor, target);
    // Core Enforcer's `onHit` runs before the hit's `DamagingHit` (`runMoveEffects`): an
    // ability it suppresses reacts to nothing.
    if !behind_substitute
        && move_def.behavior.hit_or_sub
            == Some(pokemon_battle_engine::effects::moves::HitOrSubEffect::SuppressAbilityOfMoved)
        && pokemon_battle_engine::engine::core_enforcer_suppresses(state, rules, target)
    {
        reactors.holder = None;
    }
    if let Some(roll) = pokemon_battle_engine::engine::effect_spore_roll(
        state, rules, reactors, actor, move_def, dealt,
    ) {
        let shown = oracle
            .contact_status
            .as_deref()
            .filter(|_| {
                !behind_substitute
                    && named(&oracle.contact_status_of, target)
                    && oracle
                        .contact_status_hits
                        .iter()
                        .any(|after| reported_after(*after))
            })
            .and_then(|shown| {
                [
                    NonVolatileStatus::Sleep,
                    NonVolatileStatus::Paralysis,
                    NonVolatileStatus::Poison,
                ]
                .into_iter()
                .find(|status| status_id(*status) == shown)
            });
        // A sleep Sweet Veil stopped (`contact_sleep_blocked`) was drawn all the same: the draw
        // came to sleep, and no duration follows it (R-M11-15).
        let drawn = shown.or_else(|| {
            (!behind_substitute
                && oracle
                    .contact_sleep_blocked
                    .iter()
                    .any(|after| reported_after(*after)))
            .then_some(NonVolatileStatus::Sleep)
        });
        match roll {
            EffectSporeRoll::ChanceThenSample(_) => {
                push(
                    staged,
                    RandomOverride::bool(RandomRequestKind::HitReaction, drawn.is_some()),
                );
                if let Some(status) = drawn {
                    let index = pokemon_battle_engine::engine::EFFECT_SPORE_SAMPLE
                        .iter()
                        .position(|s| *s == status)
                        .unwrap_or(0);
                    push(
                        staged,
                        RandomOverride::range(RandomRequestKind::Sample, index as u16),
                    );
                }
            }
            EffectSporeRoll::Percent => {
                let value = (0..100)
                    .find(|roll| pokemon_battle_engine::engine::effect_spore_status(*roll) == drawn)
                    .unwrap_or(99);
                push(
                    staged,
                    RandomOverride::range(RandomRequestKind::EffectSpore, value),
                );
            }
        }
        if shown == Some(NonVolatileStatus::Sleep)
            && let Some(duration) = sleep_duration
        {
            push(
                staged,
                RandomOverride::range(RandomRequestKind::Duration, duration),
            );
        }
    }
    if !*cursed_body_done
        && pokemon_battle_engine::engine::cursed_body_rolls(state, reactors, actor, move_def)
    {
        let took = !behind_substitute
            && oracle
                .cursed_body
                .as_ref()
                .is_some_and(|(after, of)| named(of, target) && reported_after(*after));
        push(
            staged,
            RandomOverride::bool(RandomRequestKind::HitReaction, took),
        );
        *cursed_body_done = took;
    }
    if pokemon_battle_engine::engine::cute_charm_rolls(
        state, rules, reactors, actor, move_def, dealt,
    ) {
        let took = !behind_substitute
            && oracle.cute_charm.as_ref().is_some_and(|(_, of)| {
                named(of, target)
                    && oracle
                        .cute_charm_hits
                        .iter()
                        .any(|after| reported_after(*after))
            });
        push(
            staged,
            RandomOverride::bool(RandomRequestKind::HitReaction, took),
        );
    }
    if user_keeps_ability
        && pokemon_battle_engine::engine::dealt_hit_status(state, rules, reactors, target, move_def)
            .is_some()
    {
        let took = !behind_substitute
            && oracle.dealt_status.iter().any(|(who, after)| {
                who_to_id(state, who) == Some(target) && reported_after(*after)
            });
        push(
            staged,
            RandomOverride::bool(RandomRequestKind::HitReaction, took),
        );
    }
}

/// Whether an effect label shows `stat` changed, in either direction (Contrary turns a boost
/// into a drop): `boost:spd`, `unboost:spd`.
pub(super) fn stat_changed(label: &str, stat: pokemon_battle_engine::state::Stat) -> bool {
    use pokemon_battle_engine::state::Stat;
    let id = match stat {
        Stat::Attack => "atk",
        Stat::Defense => "def",
        Stat::SpecialAttack => "spa",
        Stat::SpecialDefense => "spd",
        Stat::Speed => "spe",
        Stat::Accuracy => "accuracy",
        Stat::Evasion => "evasion",
    };
    label
        .strip_prefix("boost:")
        .or_else(|| label.strip_prefix("unboost:"))
        == Some(id)
}

/// Showdown's id for a major status (`-status|p|brn`).
pub(crate) fn status_id(status: NonVolatileStatus) -> &'static str {
    match status {
        NonVolatileStatus::Burn => "brn",
        NonVolatileStatus::Freeze => "frz",
        NonVolatileStatus::Paralysis => "par",
        NonVolatileStatus::Poison => "psn",
        NonVolatileStatus::Toxic => "tox",
        NonVolatileStatus::Sleep => "slp",
    }
}

pub(super) fn normalise_oracle_frame(frame: &Value) -> Value {
    let mut frame = frame.clone();
    // Showdown reports one request kind per side (`wait|forced_switch`); the engine's request
    // is the side that is asked, so the first non-`wait` part is the comparable one.
    if let Some(request) = frame["request"].as_str() {
        let parts: Vec<&str> = request.split('|').collect();
        let asked = parts
            .iter()
            .find(|part| **part != "wait")
            .or(parts.first())
            .map(|part| part.to_string())
            .unwrap_or_default();
        frame["request"] = Value::String(asked);
    }
    // Each observer's knowledge (`views`, M10.4c) is compared on its own, against the
    // `BattleView`s.
    frame.as_object_mut().map(|o| {
        o.remove("events");
        o.remove("views")
    });
    for side in frame
        .get_mut("sides")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        for pokemon in side
            .get_mut("party")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(doll) = pokemon
                .get_mut("volatiles")
                .and_then(|volatiles| volatiles.get_mut("substitute"))
                && let Some(hp) = substitute_hp_value(doll)
            {
                *doll = Value::from(hp);
            }
        }
    }
    frame
}

/// A Substitute's HP in a Showdown frame, as the engine keeps it. Showdown's Gen 3+ Psywave
/// leaves a fraction on a doll: its `damageCallback` is not rounded (the HP damage is, in
/// `spreadDamage`), and `substitute.onTryPrimaryHit` subtracts it as it is. The cartridge's
/// damage is whole, and a doll at 1.79 breaks under the same whole hits as one at 2, so the
/// fraction (in hundredths: Psywave's `random(50, 151) × level / 100`) reads rounded up
/// (DESIGN.md R72).
pub(super) fn substitute_hp_value(doll: &Value) -> Option<u16> {
    let hundredths = (doll.as_f64()? * 100.0).round() as u32;
    Some(hundredths.div_ceil(100) as u16)
}

pub(super) fn compare_frame(
    report: &mut ReplayReport,
    checkpoint: &str,
    rust: &crate::support::SemanticFrame,
    oracle: &Value,
) {
    let (rust, diffs) = frame_diffs(rust, oracle);
    report.rust_frames.push(rust);
    report.frames_compared += 1;
    for diff in diffs.iter().take(10) {
        report.divergences.push(format!(
            "{checkpoint}: {} rust={} showdown={}",
            diff.path, diff.actual, diff.expected
        ));
    }
}

/// The engine's frame as compared, and where it differs from the oracle's.
pub(super) fn frame_diffs(
    rust: &crate::support::SemanticFrame,
    oracle: &Value,
) -> (Value, Vec<crate::support::compare::SemanticDiff>) {
    let mut rust = serde_json::to_value(rust).expect("serialize frame");
    let expected = normalise_oracle_frame(oracle);
    // A recording made before the frames carried `types` / `ability` / `item` (2026-10-01),
    // `pp` (2026-10-02) or `species` (2026-10-04) has none: those fields are not compared.
    for (side, sides) in expected["sides"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (index, recorded) in sides["party"].as_array().into_iter().flatten().enumerate() {
            for field in ["types", "ability", "item", "pp", "species"] {
                if recorded.get(field).is_none()
                    && let Some(pokemon) = rust["sides"][side]["party"][index].as_object_mut()
                {
                    pokemon.remove(field);
                }
            }
        }
    }
    // Showdown is the expected side, the engine the actual one.
    let diffs = compare_json(&expected, &rust, &["$.checkpoint"]);
    (rust, diffs)
}

/// Every HP change one step committed, in order (damage, recoil, drain, self heal).
pub(super) fn substitute_hp(state: &BattleState, pokemon: PokemonId) -> Option<u16> {
    state
        .pokemon(pokemon)?
        .volatiles
        .entries()
        .iter()
        .find_map(|volatile| match volatile {
            VolatileCondition::Substitute { hp, .. } => Some(*hp),
            _ => None,
        })
}

/// The Substitute HP each Pokémon whose Substitute `decisions[index]` struck must be left
/// Each Rage user's rage at the boundary (`ActionOracle::raging_after`): nothing a later action
/// of the turn does starts or ends a Gen 3 rage, so the frame tells what its Rage left —
/// raging after a passed accuracy check, not after a miss (a fainted user shows neither).
pub(super) fn attribute_frame_rage(decisions: &mut [ActionOracle], oracle_frame: &Value) {
    for decision in decisions.iter_mut() {
        if decision.move_name.as_deref() != Some("Rage") {
            continue;
        }
        let side = &oracle_frame["sides"][decision.user.side];
        decision.raging_after = decision
            .user
            .slot
            .and_then(|slot| side["active"][slot].as_u64())
            .map(|party| {
                side["party"][party as usize]["volatiles"]
                    .as_object()
                    .is_some_and(|volatiles| volatiles.contains_key("rage"))
            });
    }
}

/// with: gone if the hit broke it; otherwise, when no later action this turn strikes it
/// again, what the boundary frame shows (nothing to say if it is gone by then). A Gen 4
/// foreseen attack that strikes it in the residual deals the damage stored at use: the frame's
/// HP plus that, when the doll outlasts the strike (R-M8-164).
pub(super) fn substitute_hint(
    state: &BattleState,
    rules: &ResolvedRules,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    index: usize,
) -> Vec<(PokemonId, Option<u16>)> {
    let mut hint = Vec::new();
    for (holder, broken) in &decisions[index].substitute_struck {
        let Some(id) = who_to_id(state, holder) else {
            continue;
        };
        if hint.iter().any(|(seen, _)| *seen == id) {
            hint.retain(|(seen, _)| *seen != id);
        }
        if *broken {
            hint.push((id, None));
            continue;
        }
        if decisions[index + 1..].iter().any(|later| {
            later
                .substitute_struck
                .iter()
                .any(|(who, _)| same_pokemon(who, holder))
        }) {
            continue;
        }
        // The holder where it stands at the boundary (an Ally Switch after the hit moves it to
        // the other slot); one no longer in passed its doll on with Baton Pass, to its slot.
        let side = &oracle_frame["sides"][holder.side];
        let at_boundary = frame_member(state, oracle_frame, holder)
            .or_else(|| frame_member_at_slot(oracle_frame, holder))
            .and_then(|party| {
                substitute_hp_value(&side["party"][party]["volatiles"]["substitute"])
            });
        let foreseen = if residual_substitute_strike(oracle_frame, holder) {
            match foreseen_strike_damage(state, rules, holder) {
                Some(damage) => Some(damage),
                None => continue,
            }
        } else {
            None
        };
        if let Some(hp) = at_boundary {
            hint.push((id, Some(hp + foreseen.unwrap_or(0))));
        }
    }
    hint
}

/// The damage a Gen 4 foreseen attack (`ForeseenRoll::AtUse`) on `holder`'s slot stored at use
/// (§15.22): what its strike deals, a fixed amount. None in other generations, whose strike
/// draws its random factor (or computes the damage) as it strikes.
pub(super) fn foreseen_strike_damage(
    state: &BattleState,
    rules: &ResolvedRules,
    holder: &Who,
) -> Option<u16> {
    if rules.conditions().future_attack.damage_at_use
        != Some(pokemon_battle_engine::rules::ForeseenRoll::AtUse)
    {
        return None;
    }
    let side = if holder.side == 0 {
        SideId::P1
    } else {
        SideId::P2
    };
    state
        .side(side)
        .future_attacks
        .iter()
        .find(|attack| attack.slot == holder.slot.unwrap_or(0))
        .and_then(|attack| attack.damage)
}

/// The Substitute HP the frame shows on the Pokémon in `who`'s place, if it has one.
pub(super) fn frame_substitute_hp(oracle_frame: &Value, who: &Who) -> Option<u16> {
    let side = &oracle_frame["sides"][who.side];
    who.slot
        .and_then(|slot| side["active"][slot].as_u64())
        .and_then(|party| {
            substitute_hp_value(&side["party"][party as usize]["volatiles"]["substitute"])
        })
}

/// Whether a foreseen attack strikes `holder`'s Substitute in the boundary's residual
/// (`-activate|holder|move: Substitute|[damage]` or its `-end` after the `residual` marker): the
/// frame's Substitute HP is after that strike, not after the turn's last action (R-M8-84).
pub(super) fn residual_substitute_strike(oracle_frame: &Value, holder: &Who) -> bool {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .skip_while(|event| event["kind"] != "residual")
        .any(|event| {
            matches!(event["kind"].as_str(), Some("activate" | "end"))
                && event["args"][1]
                    .as_str()
                    .is_some_and(|what| what.ends_with("Substitute"))
                && who_from_text(&event["args"][0]).as_ref() == Some(holder)
        })
}

pub(super) fn damages_in(engine: &BattleEngine, from: usize) -> Vec<(PokemonId, u16)> {
    engine.trace().events()[from..]
        .iter()
        .filter_map(|event| match event {
            // Regenerator heals in silence (`pokemon.heal` with no message): a holder a move
            // dragged out (Circle Throw) shows no HP line for it (R-M8-106).
            TraceEvent::HealApplied {
                source:
                    pokemon_battle_engine::engine::EffectSource::Ability {
                        ability: pokemon_battle_engine::ids::AbilityId::REGENERATOR,
                        ..
                    },
                ..
            } => None,
            TraceEvent::DamageApplied {
                target, hp_after, ..
            }
            | TraceEvent::HealApplied {
                target, hp_after, ..
            } => Some((*target, *hp_after)),
            _ => None,
        })
        .collect()
}

/// What `search_rolls` found for one action.
pub(super) struct RollSearch {
    /// The engine after the action, its step, and the complete tape that produced it.
    pub(super) chosen: Option<(BattleEngine, EngineStep, Vec<RandomOverride>)>,
    /// The attempts of the slot that could not be matched (for the report).
    pub(super) seen: Vec<(u16, Vec<(PokemonId, u16)>)>,
    /// Other complete tapes that reproduce the action just as well but leave a deferred
    /// Substitute (struck again later in the boundary, so the frame says nothing about it
    /// yet) with other HP: one per HP value, for the backtracking in `drive_to_boundary`.
    pub(super) alternatives: Vec<Vec<RandomOverride>>,
}

/// How well a roll set reproduces an action: the hits whose landing on a Substitute or a body
/// matches the log, in order; the HP commits matched in order; whether the count matches; the
/// Substitutes matched; and how close the rest are.
pub(super) type RollScore = (usize, usize, bool, usize, std::cmp::Reverse<u32>);

/// A turn submitted with an undecided Gen 2-3 Quick Claw roll: the snapshot before it, the
/// choices, the tape used and the other outcome's.
pub(super) type TurnRetry = (
    BattleSnapshot,
    [SideChoice; 2],
    Vec<RandomOverride>,
    Vec<RandomOverride>,
);

/// A damage roll left open for the search, but optional: the hit may not draw it (DESIGN.md
/// R-M8-84). Marked by a value no roll takes; `search_rolls` fills it like an open slot.
pub(super) fn open_optional_roll() -> RandomOverride {
    RandomOverride::range(RandomRequestKind::DamageRoll, u16::MAX).optional()
}

pub(super) fn is_open_optional_roll(entry: &Option<RandomOverride>) -> bool {
    entry.is_some_and(|entry| entry == open_optional_roll())
}

/// The holds on 1 HP an action's log names: whose Focus Sash held (`-enditem|p|Focus Sash`), whose
/// Focus Band (`-activate|p|item: Focus Band`) and whose Sturdy (`-ability|p|Sturdy`). A roll that
/// leaves a Pokémon on 1 by itself shows the same HP; the search keeps the rolls under which each
/// held where the log says, nowhere else (R-M10-41: a hold names its item or ability).
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ShownHolds {
    pub(super) sashes: Vec<PokemonId>,
    pub(super) bands: Vec<PokemonId>,
    pub(super) sturdy: Vec<PokemonId>,
    /// The Gen 4 sashes the action's Knock Off took (`ActionOracle::focus_sash_knocked_off`):
    /// held or not, the log is the same (R-M11-129).
    pub(super) unseen_sashes: Vec<PokemonId>,
}

impl ShownHolds {
    /// Whether a candidate's holds (`of`) are the ones the log names, a sash whose hold cannot be
    /// seen either way.
    pub(super) fn admits(&self, shown: &ShownHolds) -> bool {
        let seen = |sashes: &[PokemonId]| -> Vec<PokemonId> {
            sashes
                .iter()
                .copied()
                .filter(|id| !self.unseen_sashes.contains(id))
                .collect()
        };
        seen(&shown.sashes) == seen(&self.sashes)
            && shown.bands == self.bands
            && shown.sturdy == self.sturdy
    }

    /// The holds a candidate's trace shows (its `DamageReduced` by each; Gen 4's sash holds
    /// every hit of the move).
    pub(super) fn of(events: &[TraceEvent]) -> Self {
        use pokemon_battle_engine::engine::EffectSource;
        use pokemon_battle_engine::ids::{AbilityId, ItemId};
        let mut holds = Self::default();
        for event in events {
            let TraceEvent::DamageReduced { target, by, .. } = event else {
                continue;
            };
            let list = match by {
                EffectSource::Item {
                    item: ItemId::FOCUS_SASH,
                    ..
                } => &mut holds.sashes,
                EffectSource::Item {
                    item: ItemId::FOCUS_BAND,
                    ..
                } => &mut holds.bands,
                EffectSource::Ability {
                    ability: AbilityId::STURDY,
                    ..
                } => &mut holds.sturdy,
                _ => continue,
            };
            list.push(*target);
        }
        holds.settled()
    }

    /// Each holder once, in order.
    pub(super) fn settled(mut self) -> Self {
        for list in [&mut self.sashes, &mut self.bands, &mut self.sturdy] {
            list.sort();
            list.dedup();
        }
        self
    }
}
