//! Searching the rolls that reproduce Showdown's HP outcome (`search_rolls`), and the replay's
//! steps.

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

/// Recovers the damage rolls of one action. Each open slot (`None`) in the template is a hit's
/// roll; a hit's roll only affects the HP commits from that hit on, so the slots are settled
/// front to back (in three sweeps), keeping for each the roll that extends the matching prefix
/// of Showdown's HP sequence the furthest. Returns the engine that reproduced the whole
/// sequence, or the attempts of the slot that could not be matched.
#[allow(clippy::too_many_arguments)]
pub(super) fn search_rolls(
    compiled: &Compiled,
    before: &BattleSnapshot,
    trace_len: usize,
    template: &[Option<RandomOverride>],
    expected: &[(PokemonId, u16)],
    hits: &[(PokemonId, bool)],
    substitutes: &[(PokemonId, Option<u16>)],
    deferred: &[PokemonId],
    holds: &ShownHolds,
    unused: &mut Option<String>,
) -> RollSearch {
    let damage = compiled.rules.damage();
    // An open Psywave roll ranges over Psywave's own rolls (R-M11-151); any other, the damage
    // formula's random factor.
    let (low, high) = template
        .iter()
        .find_map(open_psywave_level)
        .map_or((damage.random_min, damage.random_max), |level| {
            pokemon_battle_engine::engine::psywave_range(&compiled.rules, level)
        });
    let candidates: Vec<u16> = (low..=high).collect();
    let open = template
        .iter()
        .filter(|entry| {
            entry.is_none() || is_open_optional_roll(entry) || open_psywave_level(entry).is_some()
        })
        .count();
    let mut rolls = vec![high; open];
    let mut seen = Vec::new();

    // Gen 1-2 roll no variance for a damage of 1 (Showdown `damage > 1`): the plan cannot know
    // before the damage is computed, so their rolls are optional decisions.
    let optional_roll = matches!(
        compiled.rules.damage().algorithm,
        pokemon_battle_engine::rules::DamageAlgorithm::Gen1Legacy
            | pokemon_battle_engine::rules::DamageAlgorithm::Gen2
    );
    // A roll set can make the engine ask for a decision out of the plan's order — a berry the
    // HP threshold feeds one hit early because the rolls are too high (DESIGN.md R269) — and
    // the plan's tape panics: such a set is only a candidate that does not reproduce the action.
    // The first such panic is raised again when no roll set ran at all.
    let first_panic: std::cell::RefCell<Option<Box<dyn std::any::Any + Send>>> =
        std::cell::RefCell::new(None);
    let ran = std::cell::Cell::new(false);
    let run = |rolls: &[u16]| {
        let mut next = 0;
        let tape = template
            .iter()
            .map(|entry| {
                let optional = is_open_optional_roll(entry);
                let psywave = open_psywave_level(entry).is_some();
                match entry {
                    Some(entry) if !optional && !psywave => *entry,
                    _ => {
                        let roll = rolls[next];
                        next += 1;
                        let decision = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
                        if optional_roll || optional {
                            decision.optional()
                        } else {
                            decision
                        }
                    }
                }
            })
            .collect::<Vec<_>>();
        let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut candidate = restore_with_tape(compiled, before, tape.clone());
            let step = candidate.step().expect("step");
            (candidate, step)
        }));
        match stepped {
            Ok((candidate, step)) => {
                ran.set(true);
                let dealt = damages_in(&candidate, trace_len);
                Some((candidate, step, dealt, tape))
            }
            Err(payload) => {
                first_panic.borrow_mut().get_or_insert(payload);
                None
            }
        }
    };
    let raise_if_none_ran = || {
        if !ran.get()
            && let Some(payload) = first_panic.borrow_mut().take()
        {
            std::panic::resume_unwind(payload);
        }
    };
    // A hit on a Substitute commits no HP: the boundary frame's Substitute HP settles its roll.
    // So does a hold on 1 HP the log names (`ShownHolds`: a sash's hold, not its loss — a sash
    // flung goes too).
    let substitutes_match = |candidate: &BattleEngine| {
        substitutes
            .iter()
            .all(|(id, hp)| substitute_hp(candidate.state(), *id) == *hp)
            && holds.admits(&ShownHolds::of(&candidate.trace().events()[trace_len..]))
            // A deferred doll stood after this action (the log shows it struck, not broken):
            // a roll set that breaks it here leaves the later action that strikes it again
            // hitting the body (R-M8-183).
            && deferred
                .iter()
                .all(|id| substitute_hp(candidate.state(), *id).is_some())
    };
    let substitutes_matched = |candidate: &BattleEngine| {
        substitutes
            .iter()
            .filter(|(id, hp)| substitute_hp(candidate.state(), *id) == *hp)
            .count()
    };
    // How far the Substitutes' HP is from the frame's: several hits on one doll settle their
    // rolls together (Spike Cannon), so a roll that brings the sum closer is progress.
    let substitute_distance = |candidate: &BattleEngine| {
        substitutes
            .iter()
            .map(
                |(id, hp)| match (substitute_hp(candidate.state(), *id), *hp) {
                    (Some(actual), Some(want)) => u32::from(actual.abs_diff(want)),
                    (None, None) => 0,
                    _ => u32::from(u16::MAX),
                },
            )
            .sum::<u32>()
    };
    let prefix = |dealt: &[(PokemonId, u16)]| {
        dealt
            .iter()
            .zip(expected)
            .take_while(|(a, b)| a == b)
            .count()
    };
    // Where each of the move's hits landed, Substitute (`true`) or body, as the log shows it:
    // a roll that breaks a Substitute one hit too late can still match an HP commit of the
    // hit after, and must not win over the one that breaks it where Showdown did.
    let hits_matched = |candidate: &BattleEngine| {
        candidate.trace().events()[trace_len..]
            .iter()
            .filter_map(|event| match event {
                TraceEvent::SubstituteDamaged { target, .. } => Some((*target, true)),
                TraceEvent::DamageApplied {
                    target,
                    source: pokemon_battle_engine::engine::EffectSource::Move { pokemon, .. },
                    ..
                } if target != pokemon => Some((*target, false)),
                _ => None,
            })
            .zip(hits)
            .take_while(|(a, b)| a == *b)
            .count()
    };

    // A tape the engine did not use up planned a decision the engine never asked for: the
    // plan and the engine disagree about the rules, which the HP alone may not show.
    let mut used_up = |candidate: &BattleEngine| {
        let rng = &candidate.state().rng;
        if rng.semantic_tape_exhausted() {
            return true;
        }
        let asked = rng.semantic_tape_cursor();
        let left: Vec<String> = template[asked.min(template.len())..]
            .iter()
            .map(|entry| match entry {
                Some(decision) => format!("{:?}={:?}", decision.kind, decision.value),
                None => "DamageRoll".into(),
            })
            .collect();
        *unused = Some(format!(
            "asked {asked} of {} planned decisions; left {left:?}",
            template.len()
        ));
        if std::env::var_os("REPLAY_DEBUG").is_some() {
            for pokemon in before.state.all_pokemon() {
                if pokemon.status.is_some() {
                    eprintln!("    before: {:?} {:?}", pokemon.id(), pokemon.status);
                }
            }
            for event in &candidate.trace().events()[trace_len..] {
                eprintln!("    trace: {event:?}");
            }
        }
        false
    };

    // Every other roll set that reproduces the action as well, one per HP it leaves on the
    // deferred Substitutes. Each slot's rolls that change that HP on their own stand for its
    // outcomes (a hit's 4 or 5), and every combination of them is tried: three hits that must
    // all deal their least leave an HP no single roll reaches (R-M8-178).
    let alternatives = |rolls: &[u16], chosen: &BattleEngine| {
        let key = |engine: &BattleEngine| {
            deferred
                .iter()
                .map(|id| substitute_hp(engine.state(), *id))
                .collect::<Vec<_>>()
        };
        let mut keys = vec![key(chosen)];
        let mut found = Vec::new();
        if deferred.is_empty() {
            return found;
        }
        let outcomes: Vec<Vec<u16>> = (0..rolls.len())
            .map(|slot| {
                let mut seen = vec![key(chosen)];
                let mut kept = vec![rolls[slot]];
                for &roll in &candidates {
                    let mut other = rolls.to_vec();
                    other[slot] = roll;
                    if let Some((candidate, ..)) = run(&other)
                        && !seen.contains(&key(&candidate))
                    {
                        seen.push(key(&candidate));
                        kept.push(roll);
                    }
                }
                kept
            })
            .collect();
        // The combinations, as indices into each slot's outcomes (a bounded number).
        let combinations = outcomes
            .iter()
            .try_fold(1usize, |count, kept| count.checked_mul(kept.len()))
            .filter(|count| *count <= 4096)
            .unwrap_or(0);
        for mut index in 1..combinations {
            let mut other = Vec::with_capacity(rolls.len());
            for kept in &outcomes {
                other.push(kept[index % kept.len()]);
                index /= kept.len();
            }
            let Some((candidate, _, dealt, tape)) = run(&other) else {
                continue;
            };
            if dealt == expected
                && substitutes_match(&candidate)
                && candidate.state().rng.semantic_tape_exhausted()
                && !keys.contains(&key(&candidate))
            {
                keys.push(key(&candidate));
                found.push(tape);
            }
        }
        found
    };

    if open == 0 {
        let Some((candidate, step, dealt, tape)) = run(&rolls) else {
            raise_if_none_ran();
            unreachable!("a roll set that panicked");
        };
        if dealt == expected && substitutes_match(&candidate) && used_up(&candidate) {
            return RollSearch {
                chosen: Some((candidate, step, tape)),
                seen,
                alternatives: Vec::new(),
            };
        }
        seen.push((0, dealt));
        return RollSearch {
            chosen: None,
            seen,
            alternatives: Vec::new(),
        };
    }

    let score_of = |candidate: &BattleEngine, dealt: &[(PokemonId, u16)]| {
        (
            hits_matched(candidate),
            prefix(dealt),
            dealt.len() == expected.len(),
            substitutes_matched(candidate),
            std::cmp::Reverse(substitute_distance(candidate)),
        )
    };
    // Slot by slot, keep the roll that reproduces the longest prefix of Showdown's HP commits;
    // among equals, one that makes as many commits (a roll on a Substitute commits nothing, but
    // decides whether the next hit still lands on it), then one that leaves the Substitutes as
    // the boundary frame shows them, or closest to it. Hits whose rolls only add up on a doll
    // settle together, so the slots are swept again from the rolls the last sweep kept. The
    // sweeps start from the highest rolls; when they settle nowhere, they start again from the
    // uniform roll set that comes closest: hits on a doll can need every slot to move at once
    // (two hits of 12 where a hit's roll gives 10, 12 or 13), out of reach one slot at a time.
    for pass in 0..2 {
        if pass == 1 {
            if open < 2 {
                break;
            }
            let mut best: Option<(RollScore, u16)> = None;
            for &roll in &candidates {
                let uniform = vec![roll; open];
                let Some((candidate, step, dealt, tape)) = run(&uniform) else {
                    continue;
                };
                if dealt == expected && substitutes_match(&candidate) && used_up(&candidate) {
                    let alternatives = alternatives(&uniform, &candidate);
                    return RollSearch {
                        chosen: Some((candidate, step, tape)),
                        seen,
                        alternatives,
                    };
                }
                let score = score_of(&candidate, &dealt);
                if best.is_none_or(|(best_score, _)| score > best_score) {
                    best = Some((score, roll));
                }
            }
            if let Some((_, roll)) = best {
                rolls = vec![roll; open];
            }
        }
        for slot in (0..3).flat_map(|_| 0..open) {
            let mut best: Option<(RollScore, u16)> = None;
            seen.clear();
            for &roll in &candidates {
                rolls[slot] = roll;
                let Some((candidate, step, dealt, tape)) = run(&rolls) else {
                    seen.push((roll, Vec::new()));
                    continue;
                };
                if dealt == expected && substitutes_match(&candidate) && used_up(&candidate) {
                    let alternatives = alternatives(&rolls, &candidate);
                    return RollSearch {
                        chosen: Some((candidate, step, tape)),
                        seen,
                        alternatives,
                    };
                }
                if std::env::var_os("REPLAY_DEBUG_SUB").is_some() {
                    eprintln!(
                        "    slot {slot} rolls {rolls:?}: dealt {dealt:?}, substitutes {:?}",
                        substitutes
                            .iter()
                            .map(|(id, _)| substitute_hp(candidate.state(), *id))
                            .collect::<Vec<_>>()
                    );
                }
                if std::env::var_os("REPLAY_DEBUG").is_some() && dealt == expected {
                    eprintln!(
                        "    slot {slot} roll {roll}: HP matches, substitutes {} (want {substitutes:?})",
                        substitutes_match(&candidate)
                    );
                }
                // Substitutes count one by one: a spread move breaking two dolls settles each
                // target's roll in its own slot.
                let score = score_of(&candidate, &dealt);
                if best.is_none_or(|(best_score, _)| score > best_score) {
                    best = Some((score, roll));
                }
                seen.push((roll, dealt));
            }
            rolls[slot] = best.map_or(high, |(_, roll)| roll);
        }
    }
    // Hits on a doll that only their sum settles can need several slots to move at once from
    // wherever the sweeps settled: two hits of 33 and 36 where, beside the other's kept roll,
    // each slot alone deals 31, 34 or 37 (a Rock Blast leaving a Substitute at 6, R-M10-79).
    // Every combination of the outcomes each slot has on its own, from the rolls the sweeps
    // kept — the rolls that leave the Substitutes a different HP stand for them — is tried (a
    // bounded number).
    if !substitutes.is_empty() && open >= 2 {
        let outcomes: Vec<Vec<u16>> = (0..open)
            .map(|slot| {
                let mut dolls: Vec<Vec<Option<u16>>> = Vec::new();
                let mut kept = Vec::new();
                for &roll in &candidates {
                    let mut other = rolls.clone();
                    other[slot] = roll;
                    if let Some((candidate, ..)) = run(&other) {
                        let doll: Vec<Option<u16>> = substitutes
                            .iter()
                            .map(|(id, _)| substitute_hp(candidate.state(), *id))
                            .collect();
                        if !dolls.contains(&doll) {
                            dolls.push(doll);
                            kept.push(roll);
                        }
                    }
                }
                kept
            })
            .collect();
        let combinations = outcomes
            .iter()
            .try_fold(1usize, |count, kept| count.checked_mul(kept.len()))
            .filter(|count| *count <= 4096)
            .unwrap_or(0);
        for mut index in 0..combinations {
            let mut other = Vec::with_capacity(open);
            for kept in &outcomes {
                other.push(kept[index % kept.len()]);
                index /= kept.len();
            }
            let Some((candidate, step, dealt, tape)) = run(&other) else {
                continue;
            };
            if dealt == expected && substitutes_match(&candidate) && used_up(&candidate) {
                let alternatives = alternatives(&other, &candidate);
                return RollSearch {
                    chosen: Some((candidate, step, tape)),
                    seen,
                    alternatives,
                };
            }
        }
    }
    raise_if_none_ran();
    RollSearch {
        chosen: None,
        seen,
        alternatives: Vec::new(),
    }
}

/// Replays `steps[from]` with `tape` and every later step with the tape it used, returning the
/// engine after the last one and the steps as rebuilt (their starting snapshots updated).
pub(super) fn replay_steps(
    compiled: &Compiled,
    steps: &[ReplayedStep],
    from: usize,
    tape: &[RandomOverride],
) -> (BattleEngine, Vec<ReplayedStep>, Vec<TraceEvent>) {
    // What the replayed steps traced, in order: the log from `steps[from].log_len` on (R-M10-6).
    let mut events: Vec<TraceEvent> = Vec::new();
    let mut rebuilt = steps[..from].to_vec();
    let mut engine = restore_with_tape(compiled, &steps[from].before, tape.to_vec());
    engine.step().expect("step");
    rebuilt.push(ReplayedStep {
        before: steps[from].before.clone(),
        log_len: steps[from].log_len,
        tape: tape.to_vec(),
        alternatives: Vec::new(),
    });
    for step in &steps[from + 1..] {
        events.extend(engine.take_trace_events());
        let before = engine.snapshot();
        engine = restore_with_tape(compiled, &before, step.tape.clone());
        engine.step().expect("step");
        rebuilt.push(ReplayedStep {
            before,
            log_len: steps[from].log_len + events.len(),
            tape: step.tape.clone(),
            alternatives: step.alternatives.clone(),
        });
    }
    events.extend(engine.take_trace_events());
    (engine, rebuilt, events)
}

/// One engine step of a boundary as the replay took it.
#[derive(Clone)]
pub(super) struct ReplayedStep {
    pub(super) before: BattleSnapshot,
    /// The battle's log (`Compiled::log`) at `before`: a backtrack cuts it back there.
    pub(super) log_len: usize,
    pub(super) tape: Vec<RandomOverride>,
    pub(super) alternatives: Vec<Vec<RandomOverride>>,
}

/// The scenario with the gender Showdown drew (as it built the teams) for every set that names
/// none, read off the details of the Pokémon's `switch` / `drag` lines (`Snorlax, L50, M`; no
/// gender for a genderless one), and of the member an Illusion's disguise copies off the details
/// it shows. A Pokémon that never shows keeps none: nothing can tell.
pub(super) fn with_oracle_genders(scenario: &Scenario, oracle: &Value) -> Scenario {
    let mut scenario = scenario.clone();
    let events = oracle["frames"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|frame| frame["events"].as_array().into_iter().flatten());
    for event in events {
        if !matches!(event["kind"].as_str(), Some("switch" | "drag")) {
            continue;
        }
        // A `switch` into an Illusion shows the disguise's details; `illusion` holds the
        // entrant's own (DESIGN.md R-K.10c).
        let (Some(side), Some(name), Some(details)) = (
            event["pokemon"]["side"].as_u64(),
            event["pokemon"]["name"].as_str(),
            event["illusion"].as_str().or(event["details"].as_str()),
        ) else {
            continue;
        };
        let gender = details
            .split(", ")
            .find(|part| matches!(*part, "M" | "F"))
            .unwrap_or("N");
        let sets = if side == 0 {
            &mut scenario.teams.p1
        } else {
            &mut scenario.teams.p2
        };
        if let Some(set) = sets
            .iter_mut()
            .find(|set| set.label.as_deref().unwrap_or(&set.species) == name)
            && set.gender.is_none()
        {
            set.gender = Some(gender.to_string());
        }
        // The disguise's details show the gender Showdown drew for the member it copies (the
        // one of its species), which may never come in itself.
        if let (Some(_), Some(details)) = (event["illusion"].as_str(), event["details"].as_str()) {
            let species = details.split(", ").next().unwrap_or("");
            let gender = details
                .split(", ")
                .find(|part| matches!(*part, "M" | "F"))
                .unwrap_or("N");
            if let Some(set) = sets
                .iter_mut()
                .find(|set| set.species == species && set.gender.is_none())
            {
                set.gender = Some(gender.to_string());
            }
        }
    }
    scenario
}

/// Showdown gives a set that names no gender its species' fixed gender (`species.gender`), as
/// `build_pokemon` does: the gender the log shows for such a set must be that one.
pub(super) fn fixed_gender_mismatches(
    declared: &Scenario,
    read: &Scenario,
    dex: &ResolvedDex,
) -> Vec<String> {
    let mut out = Vec::new();
    for (declared, read) in [
        (&declared.teams.p1, &read.teams.p1),
        (&declared.teams.p2, &read.teams.p2),
    ] {
        for (declared, read) in declared.iter().zip(read) {
            if declared.gender.is_none()
                && let Some(fixed) = dex.species_by_name(&read.species).and_then(|s| s.gender)
                && let Some(logged) = read.to_set().gender
                && logged != fixed
            {
                out.push(format!(
                    "gender: the log shows {} as {logged:?}, but the species is always {fixed:?}",
                    read.species
                ));
            }
        }
    }
    out
}
