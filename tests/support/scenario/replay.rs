//! The replay against the oracle (`replay_against_oracle`, `replay_pinned`): each boundary driven
//! (`drive_to_boundary`) and compared.

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

/// Replays `scenario` against Showdown's recording, boundary by boundary. A replay that
/// diverges after open choices (`OpenChoices`) runs again with each other alternative of one
/// choice taken, the latest choice first (R-M8-102); one that gets further keeps it, and the
/// choices met after it are tried on top — a choice may only show its fit once an earlier one
/// is taken right (R-M8-164). The first run that diverges nowhere is the report, else the
/// first run's.
pub fn replay_against_oracle(scenario: &Scenario, oracle: &Value) -> ReplayReport {
    let report = replay_pinned(scenario, oracle, BTreeMap::new());
    if report.divergences.is_empty() {
        return report;
    }
    let progress = |report: &ReplayReport| (report.frames_compared, report.actions_replayed);
    let mut pins: BTreeMap<usize, usize> = BTreeMap::new();
    let mut furthest = progress(&report);
    let mut met = report.open_choices.met.clone();
    'refine: loop {
        for (choice, &count) in met.iter().enumerate().rev() {
            for pick in 0..count {
                if pins.get(&choice) == Some(&pick) {
                    continue;
                }
                // The choices after this one may come in another order once it changes.
                let mut tried: BTreeMap<usize, usize> =
                    pins.range(..choice).map(|(&c, &p)| (c, p)).collect();
                tried.insert(choice, pick);
                let again = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    replay_pinned(scenario, oracle, tried.clone())
                }));
                let Ok(again) = again else {
                    continue;
                };
                if again.divergences.is_empty() {
                    return again;
                }
                if progress(&again) > furthest {
                    furthest = progress(&again);
                    pins = tried;
                    met = again.open_choices.met.clone();
                    continue 'refine;
                }
            }
        }
        break;
    }
    report
}

/// `replay_against_oracle` with the open choices in `pins` taken.
pub(super) fn replay_pinned(
    scenario: &Scenario,
    oracle: &Value,
    pins: BTreeMap<usize, usize>,
) -> ReplayReport {
    assert_eq!(
        scenario.schema, 1,
        "{}: unsupported scenario schema",
        scenario.id
    );
    let mut report = ReplayReport::default();
    report.open_choices.pins = pins;
    let declared = scenario;
    let scenario = &with_oracle_genders(scenario, oracle);
    let mut compiled = compile(scenario);
    report
        .divergences
        .extend(fixed_gender_mismatches(declared, scenario, &compiled.dex));
    report.provenance = serde_json::json!({
        "scenario": scenario.id,
        "generation": scenario.battle.generation,
        "rules_fingerprint": compiled.rules.fingerprint().0,
        "dex_fingerprint": compiled.dex.fingerprint(),
    });
    let frames = oracle["frames"].as_array().expect("frames");
    let frame_named = |name: &str| frames.iter().find(|f| f["checkpoint"] == name);
    let reading = LogReading::for_rules(&compiled.rules, &compiled.dex);
    let all_decisions: Vec<Vec<ActionOracle>> = (0..scenario.script.len())
        .map(|i| {
            frame_named(&format!("boundary_{}", i + 1))
                .and_then(|f| f["events"].as_array())
                .map(|events| extract_decisions(events, &reading))
                .unwrap_or_default()
        })
        .collect();
    let all_entries: Vec<Vec<(usize, Who)>> = (0..scenario.script.len())
        .map(|i| {
            frame_named(&format!("boundary_{}", i + 1))
                .and_then(|f| f["events"].as_array())
                .map(|events| extract_entries(events, &reading))
                .unwrap_or_default()
        })
        .collect();

    // What each boundary's residual did to whom (Yawn's sleep, Healer's and Shed Skin's cures):
    // a sleep that ends there has no wake-up of its own to read its time from.
    let all_residuals: Vec<Vec<(Who, ResidualKind)>> = (0..scenario.script.len())
        .map(|i| {
            frame_named(&format!("boundary_{}", i + 1))
                .and_then(|f| f["events"].as_array())
                .map(|events| extract_residual_cures(events))
                .unwrap_or_default()
        })
        .collect();

    // Every boundary's frame, for the decisions only a later turn tells (a lock's length).
    let boundary_frames: Vec<Option<&Value>> = (0..scenario.script.len())
        .map(|i| frame_named(&format!("boundary_{}", i + 1)))
        .collect();

    // A real format's Team Preview, before any battle (M9.4).
    if let (Some(state), Some(frame)) = (&compiled.preview, frame_named("team_preview")) {
        let rust = crate::support::capture_state(
            state,
            &compiled.rules,
            &compiled.dex,
            "team_preview",
            Some("team_preview".into()),
        );
        compare_frame(&mut report, "team_preview", &rust, frame);
    }

    // Showdown runs the leads' switch-in (Intimidate, a weather …) before the first request;
    // the initial frame is captured after it. Tied leads go in the order it reports them.
    let mut lead_tape = lead_tie_tape(&compiled, frame_named("initial"));
    if let Some(initial) = frame_named("initial") {
        let leads = frisk_order(&compiled, compiled.engine.state(), initial);
        lead_tape.extend(lead_ability_tape(
            &compiled,
            compiled.engine.state(),
            &leads,
            initial,
        ));
    }
    flush_trace(&mut compiled);
    let snapshot = compiled.engine.snapshot();
    compiled.engine = restore_with_tape(&compiled, &snapshot, lead_tape);
    compiled.engine.run_lead_switch_ins();

    let step = compiled.engine.step().expect("initial step");
    if let Some(initial) = frame_named("initial") {
        let rust = capture(&compiled.engine, "initial", Some(&step));
        compare_frame(&mut report, "initial", &rust, initial);
        let frame = report.rust_frames.last().cloned().unwrap_or_default();
        crate::support::views::compare_views(&mut report, "initial", &compiled, &frame, initial);
    }

    // What each actor intends to use this turn, so the tape can be planned even when the
    // action ends up blocked before the move is announced. Kept across a mid-turn suspension:
    // the actions still pending after a replacement were declared by the earlier entry.
    let mut intended: BTreeMap<PokemonId, (MoveId, Option<DeclaredTarget>)> = BTreeMap::new();
    // The gimmick each choice asked for: a Z-Move runs in place of the slot's move, so the plan
    // must be built on the move the engine will actually use (DESIGN.md §44.4).
    let mut intended_mechanics: BTreeMap<PokemonId, MechanicChoice> = BTreeMap::new();
    let mut last_step = step;
    for (entry_index, entry) in scenario.script.iter().enumerate() {
        let checkpoint = format!("boundary_{}", entry_index + 1);
        let mut turn_retry: Option<TurnRetry> = None;
        flush_trace(&mut compiled);
        let boundary_trace = compiled.log.len();
        let Some(oracle_frame) = frame_named(&checkpoint) else {
            report
                .divergences
                .push(format!("{checkpoint}: missing from oracle output"));
            break;
        };
        let events = oracle_frame["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // Whether this move can flinch as its user would use it, the battle as it stands: the
        // move as the engine edits it carries a flinch — its own, or one the user's item or
        // ability adds (King's Rock, Razor Fang; Gen 5+ Stench, R-M8-102; Gen 2-4 the items only
        // on their list, R-M9-34) — unless Sheer Force took it away (R-M11-71).
        let flinches = |who: &Who, move_name: Option<&str>| {
            let state = compiled.engine.state();
            let id = who_to_id(state, who)?;
            let def = move_name.and_then(|name| compiled.dex.move_by_name(name))?;
            Some(
                pokemon_battle_engine::engine::modified_move(
                    state,
                    &compiled.rules,
                    id,
                    def,
                    false,
                )
                .0
                .secondaries
                .iter()
                .any(|secondary| {
                    secondary.effect
                        == pokemon_battle_engine::dex::SecondaryEffect::Volatile(
                            VolatileConditionKind::Flinch,
                        )
                }),
            )
        };
        // Gen 1 stats at their bounds as the turn begins (Showdown `modifiedStats`): 999 to raise,
        // 1 to drop (R-M9-42).
        let stat_stuck = |who: &Who, stat: &str, raised: bool| {
            use pokemon_battle_engine::state::Stat;
            let state = compiled.engine.state();
            let stat = match stat {
                "atk" => Stat::Attack,
                "def" => Stat::Defense,
                "spa" => Stat::SpecialAttack,
                "spd" => Stat::SpecialDefense,
                "spe" => Stat::Speed,
                _ => return false,
            };
            who_to_id(state, who)
                .and_then(|id| state.pokemon(id))
                .and_then(|pokemon| pokemon.modified_stats)
                .and_then(|stats| stats.get(stat))
                .is_some_and(|value| if raised { value == 999 } else { value == 1 })
        };
        let mut decisions = extract_decisions_with(&events, &reading, &flinches, &stat_stuck);
        attribute_frame_flinches(&mut decisions, oracle_frame, &reading, &flinches);
        attribute_frame_rage(&mut decisions, oracle_frame);

        // A script entry answers whatever the engine is waiting for: the turn's choices, or
        // the replacements of a turn suspended mid-way (U-turn / Volt Switch) or ended by a
        // faint. Both engines read the same entry, so the boundaries stay aligned.
        if let EngineStep::Awaiting(BattleRequest::Revive { side, .. }) = &last_step {
            // Revival Blessing: the asked side names a fainted Pokémon with Showdown's `switch N`
            // (`pass` for its other slots); the party index names it, no swap follows.
            let side = *side;
            let text = if side == SideId::P1 {
                entry.p1.as_deref()
            } else {
                entry.p2.as_deref()
            };
            let Some(named) = revive_answer(&compiled, side, text) else {
                report.divergences.push(format!(
                    "{checkpoint}: rust asks {side:?} to name a Pokémon to revive but the script answers {text:?}"
                ));
                break;
            };
            if let Err(error) = answer_revive(&mut compiled, side, &named, oracle_frame) {
                report.divergences.push(format!(
                    "{checkpoint}: rust refused the script's revive: {error:?}"
                ));
                break;
            }
        } else if let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) =
            &last_step
        {
            // The engine wants replacements; the script must answer with switches. Anything
            // else means the request boundaries drifted apart: report where and stop.
            let answers_switch = |text: Option<&str>| {
                text.is_none_or(|t| {
                    t.split(',').map(str::trim).all(|part| {
                        part.is_empty() || part == "pass" || part.starts_with("switch ")
                    })
                })
            };
            if !answers_switch(entry.p1.as_deref()) || !answers_switch(entry.p2.as_deref()) {
                report.divergences.push(format!(
                    "{checkpoint}: rust asks for a forced switch ({:?}) but the script answers a turn ({:?} / {:?})",
                    replacements
                        .iter()
                        .map(|r| (r.side, r.required_count))
                        .collect::<Vec<_>>(),
                    entry.p1,
                    entry.p2
                ));
                break;
            }
            let choices = [
                parse_replacement(&mut compiled, replacements, SideId::P1, entry.p1.as_deref()),
                parse_replacement(&mut compiled, replacements, SideId::P2, entry.p2.as_deref()),
            ];
            // The entrants' confusion berries (hazards on the way in) draw their durations; a
            // Gen 4-5 Frisk entrant its foe first, Trace its foe (probed: which entrants came in).
            // Entrants sharing a speed draw their order first (R-M8-140), after the replacements
            // that replace Pokémon sharing a speed draw the order they come in (R-M11-68).
            let berries = entry_berry_durations(&outside_berry_confusion_hints(
                &compiled,
                oracle_frame,
                &decisions,
            ));
            let mut entry_tape = Vec::new();
            let ability_draws = compiled.rules.conditions().frisk_one_random_foe
                || traces(oracle_frame)
                || forewarns(oracle_frame);
            let together = choices
                .iter()
                .map(|choice| choice.replacements.len())
                .sum::<usize>()
                > 1;
            if ability_draws || together {
                flush_trace(&mut compiled);
                let snapshot = compiled.engine.snapshot();
                let trace_len = compiled.engine.trace().events().len();
                let mut probe = restore_with_tape(&compiled, &snapshot, berries.clone());
                if probe.resolve_forced_switches(choices.clone()).is_ok() {
                    let events = &probe.trace().events()[trace_len..];
                    entry_tape = replacement_tie_tape(&compiled, events, oracle_frame);
                    entry_tape.extend(entry_tie_tape(&compiled, events, oracle_frame));
                    if ability_draws {
                        entry_tape.extend(entrant_ability_tape(
                            &compiled,
                            compiled.engine.state(),
                            probe.state(),
                            &entrants(events),
                            oracle_frame,
                        ));
                    }
                }
            }
            entry_tape.extend(berries);
            if !entry_tape.is_empty() {
                flush_trace(&mut compiled);
                let snapshot = compiled.engine.snapshot();
                compiled.engine = restore_with_tape(&compiled, &snapshot, entry_tape);
            }
            let resolved = compiled.engine.resolve_forced_switches(choices);
            // Showdown asks a mid-turn switch and a Revival Blessing of one side in one request
            // (one part per slot) and runs the switch first; the engine asks the revive next, and
            // this entry's part for the reviving user's slot answers it (R-M8-171).
            let resolved = resolved.and_then(|result| {
                if compiled.engine.state().phase
                    != pokemon_battle_engine::state::BattlePhase::Revive
                {
                    return Ok(result);
                }
                for side in [SideId::P1, SideId::P2] {
                    let Some(user) = compiled.engine.state().side(side).reviving else {
                        continue;
                    };
                    let text = if side == SideId::P1 {
                        entry.p1.as_deref()
                    } else {
                        entry.p2.as_deref()
                    };
                    let slot = reviving_slot(&compiled, user);
                    let named = text.and_then(|t| {
                        t.split(',')
                            .map(str::trim)
                            .nth(slot?)
                            .and_then(|part| part.strip_prefix("switch "))
                            .map(str::to_string)
                    });
                    if let Some(named) = named {
                        answer_revive(&mut compiled, side, &named, oracle_frame)?;
                    }
                }
                Ok(result)
            });
            if let Err(error) = resolved {
                report.divergences.push(format!(
                    "{checkpoint}: rust refused the script's replacements: {error:?} (rust asked {:?})",
                    replacements
                        .iter()
                        .map(|r| (r.side, r.required_count))
                        .collect::<Vec<_>>()
                ));
                let rust = capture(&compiled.engine, &checkpoint, Some(&last_step));
                report
                    .rust_frames
                    .push(serde_json::to_value(rust).expect("frame"));
                break;
            }
        } else {
            let choices = [
                parse_choice(&mut compiled, SideId::P1, entry.p1.as_deref()),
                parse_choice(&mut compiled, SideId::P2, entry.p2.as_deref()),
            ];
            intended.clear();
            intended_mechanics.clear();
            for choice in &choices {
                for slot in &choice.actions {
                    if let ActionChoice::Move {
                        slot: move_slot,
                        target,
                        mechanic,
                    } = slot.action
                        && let Some(pokemon) = compiled.engine.state().pokemon(slot.actor)
                        // A hard lock onto a move the slots do not hold (a charged move a
                        // Metronome called): the named slot means the lock, as the engine's
                        // choice validation reads it.
                        && let Some(id) = pokemon
                            .hard_lock()
                            .map(|lock| lock.move_id)
                            .filter(|locked| {
                                pokemon.move_slots().iter().all(|m| m.move_id != *locked)
                            })
                            .or_else(|| pokemon.move_slot(move_slot).map(|m| m.move_id))
                    {
                        intended.insert(slot.actor, (id, target));
                        if let Some(mechanic) = mechanic {
                            intended_mechanics.insert(slot.actor, mechanic);
                        }
                    }
                }
            }

            // Speed ties: the engine says which actions will draw a SpeedTie decision; the
            // oracle's action order says who won each tie. Earlier movers get the larger draw.
            // The turn's actions run on past a mid-turn replacement (Gen 1-3 after a faint,
            // U-turn): the later boundaries of the same turn carry them, named by label since
            // the replacement changed who stands in a slot.
            flush_trace(&mut compiled);
            let snapshot = compiled.engine.snapshot();
            let frame_turn = |boundary: usize| {
                let name = if boundary == 0 {
                    "initial".to_owned()
                } else {
                    format!("boundary_{boundary}")
                };
                frame_named(&name).and_then(|f| f["turn"].as_u64())
            };
            let turn = frame_turn(entry_index);
            let turn_decisions: Vec<&ActionOracle> = decisions
                .iter()
                .chain(
                    (entry_index + 1..all_decisions.len())
                        .take_while(|&later| frame_turn(later) == turn)
                        .flat_map(|later| all_decisions[later].iter()),
                )
                .collect();
            // Where each declared action shows in Showdown's log: a move by its user's first
            // action, a switch by its entrant's `switch` line (tied switches go in that order).
            let entrants: Vec<PokemonId> = all_entries
                .get(entry_index)
                .into_iter()
                .flatten()
                .filter_map(|(_, who)| label_to_id(&compiled, who))
                .collect();
            let switching_in = |actor: PokemonId| {
                choices
                    .iter()
                    .flat_map(|c| &c.actions)
                    .find_map(|slot| match slot.action {
                        ActionChoice::Switch { pokemon } if slot.actor == actor => Some(pokemon),
                        _ => None,
                    })
            };
            let first_move = |actor: PokemonId| {
                if let Some(entrant) = switching_in(actor) {
                    return entrants
                        .iter()
                        .position(|id| *id == entrant)
                        .unwrap_or(usize::MAX);
                }
                let is_actor = |who: &Who| {
                    label_to_id(&compiled, who).or_else(|| who_to_id(&snapshot.state, who))
                        == Some(actor)
                };
                // A move by its user's first action — a dance is another's (R-M8-78); a Pokémon
                // knocked out before it could act comes right after the action that knocked it out.
                turn_decisions
                    .iter()
                    .position(|d| !d.danced && !d.instructed && is_actor(&d.user))
                    .map(|index| 2 * index + 2)
                    .or_else(|| {
                        turn_decisions
                            .iter()
                            .position(|d| {
                                d.damages.iter().any(|(who, hp)| *hp == 0 && is_actor(who))
                            })
                            .map(|index| 2 * index + 3)
                    })
                    .or_else(|| {
                        // Sky Drop's carried Pokémon shows nothing: its action came to nothing
                        // before its carrier let it go — after, it would have acted (DESIGN.md
                        // R281).
                        let carrier = snapshot.state.pokemon(actor)?.volatiles.sky_dropped_by()?;
                        turn_decisions
                            .iter()
                            .position(|d| {
                                label_to_id(&compiled, &d.user)
                                    .or_else(|| who_to_id(&snapshot.state, &d.user))
                                    == Some(carrier)
                            })
                            .map(|index| 2 * index + 1)
                    })
                    .unwrap_or(usize::MAX)
            };
            // Priority chances (DESIGN.md §12.4): a Quick Claw or Quick Draw that went off
            // announces itself as the turn starts; Gen 2-3's turn roll is silent, so it is read
            // from the order — it went off when the order without it contradicts the oracle's.
            // One Instruct's `resolveAction` rolled for its target (right after the
            // `-singleturn`) is that action's, not the turn's (R-M9-32).
            let instructed_at = |index: usize| {
                events[..index]
                    .iter()
                    .rev()
                    .find(|event| {
                        event["kind"] != "activate"
                            || event["args"]
                                .get(1)
                                .and_then(Value::as_str)
                                .and_then(announced_chance)
                                .is_none()
                    })
                    .is_some_and(|event| {
                        event["kind"] == "singleturn" && event["move"] == "move: Instruct"
                    })
            };
            let announced: Vec<(PokemonId, PriorityChance)> = events
                .iter()
                .enumerate()
                .filter(|(index, event)| event["kind"] == "activate" && !instructed_at(*index))
                .filter_map(|(_, event)| {
                    let args = event["args"].as_array()?;
                    let chance = announced_chance(args.get(1).and_then(Value::as_str)?)?;
                    let who = args.first().and_then(who_from_text)?;
                    let id = label_to_id(&compiled, &who)
                        .or_else(|| who_to_id(&snapshot.state, &who))?;
                    Some((id, chance))
                })
                .collect();
            let preview = |turn_roll: bool| {
                compiled
                    .engine
                    .preview_action_order(&choices, &|actor, chance| match chance {
                        PriorityChance::QuickClawTurn => turn_roll,
                        _ => announced.contains(&(actor, chance)),
                    })
                    .unwrap_or_else(|error| panic!("{}: {checkpoint}: {error:?}", scenario.id))
            };
            // Whether Showdown's order fits the brackets: every actor of an earlier bracket could
            // have come before every actor of a later one. An actor that acted sits at its
            // action; one knocked out before acting only after the action that knocked it out.
            let consistent = |order: &[Vec<PokemonId>]| {
                let spans: Vec<Vec<(usize, usize)>> = order
                    .iter()
                    .map(|bracket| {
                        bracket
                            .iter()
                            .filter(|actor| switching_in(**actor).is_none())
                            .map(|actor| first_move(*actor))
                            .filter(|position| *position != usize::MAX)
                            .map(|position| {
                                if position % 2 == 0 {
                                    (position, position)
                                } else {
                                    (position, usize::MAX)
                                }
                            })
                            .collect()
                    })
                    .collect();
                spans.iter().enumerate().all(|(index, earlier)| {
                    spans[index + 1..]
                        .iter()
                        .flatten()
                        .all(|later| earlier.iter().all(|(earliest, _)| *earliest < later.1))
                })
            };
            // The turn roll also sets the holders' Speed for the residual: when the action order
            // cannot tell, the other outcome is kept to retry the boundary with.
            let mut plan = preview(false);
            let mut undecided = None;
            if plan
                .chances
                .iter()
                .any(|(_, chance, _)| *chance == PriorityChance::QuickClawTurn)
            {
                let rolled = preview(true);
                match (consistent(&plan.order), consistent(&rolled.order)) {
                    (false, true) => plan = rolled,
                    (true, true) => undecided = Some(rolled),
                    _ => {}
                }
            }
            let tape_for = |plan: &pokemon_battle_engine::engine::ActionOrderPreview| {
                let ties = plan.ties.iter().map(|(actor, group)| {
                    let mut members: Vec<PokemonId> = plan
                        .ties
                        .iter()
                        .filter(|(_, g)| g == group)
                        .map(|(a, _)| *a)
                        .collect();
                    members.sort_by_key(|a| first_move(*a));
                    let rank = members.iter().position(|a| a == actor).unwrap_or(0);
                    RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
                });
                plan.chances
                    .iter()
                    .map(|(_, _, rolled)| {
                        RandomOverride::bool(RandomRequestKind::PriorityChance, *rolled)
                    })
                    .chain(ties)
                    .collect::<Vec<RandomOverride>>()
            };
            let tie_tape = tape_for(&plan);
            if std::env::var_os("REPLAY_DEBUG").is_some() {
                eprintln!(
                    "  {checkpoint}: priority chances {:?}, speed ties {:?} → {tie_tape:?}",
                    plan.chances, plan.ties
                );
            }
            if let Some(other) = &undecided {
                turn_retry = Some((
                    snapshot.clone(),
                    choices.clone(),
                    tie_tape.clone(),
                    tape_for(other),
                ));
            }
            compiled.engine = restore_with_tape(&compiled, &snapshot, tie_tape);
            if let Err(error) = compiled.engine.submit_turn_choices(choices) {
                report.divergences.push(format!(
                    "{checkpoint}: rust refused the script's turn choices: {error:?}"
                ));
                break;
            }
        }

        let marks = (
            compiled.party.clone(),
            compiled.pending_swaps.clone(),
            report.divergences.len(),
            report.actions_replayed,
            report.frames_compared,
            report.rust_frames.len(),
            // The log as the turn's choices went in: a retry from the turn's snapshot drops what
            // the failed drive traced (R-M10-6).
            compiled.log.len(),
        );
        let drive = |compiled: &mut Compiled, report: &mut ReplayReport| {
            drive_and_compare(
                compiled,
                report,
                &checkpoint,
                oracle_frame,
                &decisions,
                &all_decisions,
                &all_entries,
                &all_residuals,
                &boundary_frames,
                entry_index,
                &intended,
                &intended_mechanics,
                boundary_trace,
            )
        };
        let mut step = drive(&mut compiled, &mut report);
        // The other Quick Claw outcome, then (both failing) the first again, for its report.
        if report.divergences.len() > marks.2
            && let Some((snapshot, choices, first, other)) = turn_retry
        {
            for tape in [other, first] {
                compiled.party = marks.0.clone();
                compiled.pending_swaps = marks.1.clone();
                report.divergences.truncate(marks.2);
                report.actions_replayed = marks.3;
                report.frames_compared = marks.4;
                report.rust_frames.truncate(marks.5);
                compiled.log.truncate(marks.6);
                compiled.engine = restore_with_tape(&compiled, &snapshot, tape);
                compiled
                    .engine
                    .submit_turn_choices(choices.clone())
                    .expect("the same choices were accepted");
                step = drive(&mut compiled, &mut report);
                if report.divergences.len() == marks.2 {
                    break;
                }
            }
        }
        let finished = matches!(step, EngineStep::Finished(_));
        last_step = step;
        // A diverged battle stops at that boundary: the later ones replay a different battle,
        // and a panic there would hide the divergence that explains it.
        if finished || report.divergences.len() > marks.2 {
            break;
        }
    }
    flush_trace(&mut compiled);
    report.trace = compiled
        .log
        .iter()
        .map(|event| format!("{event:?}"))
        .collect();
    report
}

/// Drives a boundary from the answered request and compares its frame (DESIGN.md §31.5): a
/// Taunt / Encore / Disable duration is read off the frame as if the end of the turn ticked it;
/// a battle decided within the residual may stop before that tick — the engine says whether it
/// got there, and the boundary is driven again with those durations read as unticked.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_and_compare(
    compiled: &mut Compiled,
    report: &mut ReplayReport,
    checkpoint: &str,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    all_decisions: &[Vec<ActionOracle>],
    all_entries: &[Vec<(usize, Who)>],
    all_residuals: &[Vec<(Who, ResidualKind)>],
    boundary_frames: &[Option<&Value>],
    entry_index: usize,
    intended: &BTreeMap<PokemonId, (MoveId, Option<DeclaredTarget>)>,
    intended_mechanics: &BTreeMap<PokemonId, MechanicChoice>,
    boundary_trace: usize,
) -> EngineStep {
    flush_trace(compiled);
    let driven_from = (
        compiled.engine.snapshot(),
        compiled.party.clone(),
        compiled.pending_swaps.clone(),
        report.divergences.len(),
        report.actions_replayed,
        compiled.log.len(),
    );
    let mut step = drive_to_boundary(
        compiled,
        report,
        checkpoint,
        oracle_frame,
        decisions,
        all_decisions,
        all_entries,
        all_residuals,
        boundary_frames,
        entry_index,
        intended,
        intended_mechanics,
        &[],
    );
    let untick = if matches!(step, EngineStep::Finished(_)) {
        flush_trace(compiled);
        let events = &compiled.log[boundary_trace..];
        let mut untick = timed_volatiles_left_unticked(events);
        untick.extend(bides_left_unticked(
            compiled.engine.state(),
            &compiled.rules,
            oracle_frame,
            events,
        ));
        untick
    } else {
        Vec::new()
    };
    if !untick.is_empty() {
        let (snapshot, party, pending_swaps, divergences, actions, log) = driven_from;
        compiled.log.truncate(log);
        compiled.engine =
            BattleEngine::restore(compiled.rules.clone(), compiled.dex.clone(), snapshot)
                .expect("restore");
        compiled.party = party;
        compiled.pending_swaps = pending_swaps;
        report.divergences.truncate(divergences);
        report.actions_replayed = actions;
        step = drive_to_boundary(
            compiled,
            report,
            checkpoint,
            oracle_frame,
            decisions,
            all_decisions,
            all_entries,
            all_residuals,
            boundary_frames,
            entry_index,
            intended,
            intended_mechanics,
            &untick,
        );
    }
    settle_party_swaps(compiled, boundary_trace);
    let rust = capture(&compiled.engine, checkpoint, Some(&step));
    compare_frame(report, checkpoint, &rust, oracle_frame);
    let frame = report.rust_frames.last().cloned().unwrap_or_default();
    crate::support::views::compare_views(report, checkpoint, compiled, &frame, oracle_frame);
    step
}

thread_local! {
    /// The boundary and action being replayed, so a panic inside the engine can be reported
    /// where it happened (`tests/campaign.rs`).
    pub static REPLAY_CONTEXT: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
}

/// Drives the engine from a freshly answered request to the next one, action by action: each
/// step is probed to learn whose action it is, the oracle's matching decision fixes the
/// semantic random calls, and the damage roll is searched. Returns the step that stopped the
/// drive (a request, or the end of the battle).
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_to_boundary(
    compiled: &mut Compiled,
    report: &mut ReplayReport,
    checkpoint: &str,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
    all_decisions: &[Vec<ActionOracle>],
    all_entries: &[Vec<(usize, Who)>],
    all_residuals: &[Vec<(Who, ResidualKind)>],
    boundary_frames: &[Option<&Value>],
    turn_index: usize,
    intended: &BTreeMap<PokemonId, (MoveId, Option<DeclaredTarget>)>,
    intended_mechanics: &BTreeMap<PokemonId, MechanicChoice>,
    untick: &[(PokemonId, MissedTick)],
) -> EngineStep {
    let residual_cures = extract_residual_cures(
        &oracle_frame["events"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    let residual_faints = extract_residual_faints(
        &oracle_frame["events"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    // The sleep a Yawn brought on in this residual: its duration, recovered from the turns
    // after this one like a move's.
    let lifecycle = compiled.rules.status().lifecycle;
    let yawn_sleeps: Vec<(Who, u16)> = residual_cures
        .iter()
        .filter(|(_, kind)| *kind == ResidualKind::Yawn)
        .filter_map(|(who, _)| {
            let after_last_action = all_decisions.get(turn_index).map_or(0, Vec::len);
            sleep_duration_hint(
                all_decisions,
                all_entries,
                all_residuals,
                turn_index,
                after_last_action,
                who,
                lifecycle.sleep_resets_on_switch_in,
                lifecycle.sleep_usable_turns_come_back,
                u16::from(lifecycle.sleep_time_min),
                u16::from(lifecycle.sleep_time_max),
                &sleep_steps(
                    compiled.engine.state(),
                    &compiled.rules,
                    who,
                    turn_index,
                    boundary_frames,
                ),
            )
            .map(|time| (who.clone(), time))
        })
        .collect();
    // The locks (Thrash and its kind) the residual ended with their fatigue: the confusion's
    // duration from the frame.
    let fatigues: Vec<(Who, u16)> = residual_cures
        .iter()
        .filter(|(_, kind)| *kind == ResidualKind::Rampage)
        .map(|(who, _)| {
            let member = frame_member_at_slot(oracle_frame, who);
            let time = confusion_hint_after(&compiled.rules, oracle_frame, &[], who, member)
                .unwrap_or(u16::from(lifecycle.confusion_time_min));
            (who.clone(), time)
        })
        .collect();
    // The confusion berries eaten outside the actions, with their durations: in the residual
    // (the frame shows the time as rolled) and in a switch-in (plus the entrant's later ticks).
    let berry_confusions = outside_berry_confusion_hints(compiled, oracle_frame, decisions);
    let mut decision_index = 0;
    // Every step of the boundary so far, for the Substitute backtracking below.
    let mut steps: Vec<ReplayedStep> = Vec::new();
    let step = loop {
        flush_trace(compiled);
        let before = compiled.engine.snapshot();
        let trace_len = compiled.engine.trace().events().len();

        // A Gen 8+ step re-sorts the queue first and reshuffles its speed ties: the engine says
        // which pending actions tie, the oracle's remaining order says who won (earlier movers
        // get the larger draw, as at the start of the turn).
        let resort_ties: Vec<RandomOverride> = {
            let tied = compiled.engine.pending_speed_ties();
            let next_move = |actor: PokemonId| {
                decisions[decision_index.min(decisions.len())..]
                    .iter()
                    .filter(|d| !d.danced && !d.instructed)
                    .position(|d| {
                        label_to_id(compiled, &d.user).or_else(|| who_to_id(&before.state, &d.user))
                            == Some(actor)
                    })
                    .unwrap_or(usize::MAX)
            };
            tied.iter()
                .map(|(actor, group)| {
                    let mut members: Vec<PokemonId> = tied
                        .iter()
                        .filter(|(_, g)| g == group)
                        .map(|(a, _)| *a)
                        .collect();
                    members.sort_by_key(|a| next_move(*a));
                    let rank = members.iter().position(|a| a == actor).unwrap_or(0);
                    RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
                })
                .collect()
        };

        // Probe with only those decisions (PRNG fallback after) to learn whose action this
        // step is.
        let mut probe = restore_with_tape(compiled, &before, resort_ties.clone());

        let probe_step = probe.step().expect("step");
        let acting = probe.trace().events()[trace_len..]
            .iter()
            .find_map(|event| match event {
                TraceEvent::BeforeMoveResolved { actor, .. } => Some(*actor),
                _ => None,
            });

        // Terastallization's own queue step (DESIGN.md §44.2): Showdown logs `-terastallize`
        // before the move it belongs to, and no `move` line of its own, so it consumes no
        // oracle action — the move that follows does. Mega Evolution's and Ultra Burst's steps
        // are the same (`-mega` / `-burst`), and in Gen 7 they re-queue the Pokémon's move.
        let gimmick = probe.trace().events()[trace_len..]
            .iter()
            .find_map(|event| match event {
                TraceEvent::Terastallized { .. } | TraceEvent::MegaEvolved { .. } => Some(()),
                _ => None,
            });
        if gimmick.is_some() {
            steps.push(ReplayedStep {
                before,
                log_len: compiled.log.len(),
                tape: resort_ties,
                alternatives: Vec::new(),
            });
            compiled.engine = probe;
            if !matches!(probe_step, EngineStep::Progressed { .. }) {
                break probe_step;
            }
            continue;
        }

        let Some(actor) = acting else {
            // Gen 1's Pokémon held by a trapping move that goes on (`cannotmove`): its action is
            // its own after-action residual (poison, burn, Leech Seed), which the log shows as a
            // silent action of its own — first in the turn when it won a tie (R-M8-108).
            if let Some(next) = decisions.get(decision_index)
                && next.move_name.is_none()
                && next.cant.is_none()
                && let Some(silent) = who_to_id(&before.state, &next.user)
                && probe.trace().events()[trace_len..].iter().any(|event| {
                    matches!(event, TraceEvent::DamageApplied { target, .. } if *target == silent)
                })
            {
                decision_index += 1;
            }
            // Not an action: the end of the turn (or a switch). Its speed ties come first
            // (Showdown shuffles tied residual handlers, then the weather's tied targets: the
            // closing events show who went first), then its decisions (Shed Skin, per statused
            // holder in schedule order).
            let ends_turn = probe.trace().events()[trace_len..].iter().any(|event| {
                matches!(
                    event,
                    TraceEvent::PhaseChanged {
                        to: pokemon_battle_engine::state::BattlePhase::EndTurn,
                        ..
                    }
                )
            });
            // The foreseen attacks the residual strikes with (§15.22): their damage rolls,
            // searched below against the log's HP.
            let strikes = if ends_turn {
                extract_future_strikes(
                    oracle_frame["events"]
                        .as_array()
                        .map_or(&[][..], Vec::as_slice),
                )
            } else {
                Vec::new()
            };
            // The entrants this step brings in (a switch-only turn: the switches run inside the
            // same step as the end of turn).
            let entrants_now = entrants(&probe.trace().events()[trace_len..]);
            // The entrants' switch-in draws (Frisk's reveal, Trace's copy): they run before the
            // step's other handlers — the switches a turn ends with come before its residual
            // (DESIGN.md R-M8-48) — and are optional, so a step that brings nobody only passes
            // over them when its own ask is of another kind.
            let entrant_tape = entrant_ability_tape(
                compiled,
                &before.state,
                probe.state(),
                &entrants_now,
                oracle_frame,
            );
            // The residual's tie decisions: the plan first, then the other orders of the ties the
            // log does not show.
            let (tie_tapes, weather_ties) = if ends_turn {
                residual_tie_tapes(compiled, &before.state, oracle_frame)
            } else {
                (vec![Vec::new()], Vec::new())
            };
            let build_residual = |rolls: &[Option<u16>], ties: &[RandomOverride]| {
                let mut residual = resort_ties.clone();
                residual.extend(entrant_tape.iter().copied());
                // Neutralizing Gas clearing (a switch: at once; the end of the turn: as its holder
                // faints, after the residual's own ties).
                let gas = neutralizing_gas_tape(compiled, &before, oracle_frame);
                if !ends_turn {
                    residual.extend(gas.iter().copied());
                }
                residual.extend(ties.iter().copied());
                let moody = moody_tape(&before.state, &compiled.rules, &compiled.dex, oracle_frame);
                residual.extend(residual_tape(
                    &before.state,
                    &compiled.rules,
                    &compiled.dex,
                    &residual_cures,
                    &residual_faints,
                    &yawn_sleeps,
                    &berry_confusions,
                    &fatigues,
                    &strikes,
                    rolls,
                    &moody,
                    &weather_ties,
                ));
                residual.extend(residual_end_berry_tape(
                    &before.state,
                    &compiled.rules,
                    &berry_confusions,
                ));
                if ends_turn {
                    residual.extend(residual_drag_tape(compiled, &before.state, oracle_frame));
                    residual.extend(gas);
                }
                // A switch action: the confusion berry (optional: the step may be another's).
                if !ends_turn {
                    residual.extend(entry_berry_durations(&berry_confusions));
                }
                residual
            };
            let mut rolls: Vec<Option<u16>> = vec![None; strikes.len()];
            // Each strike's roll in turn, the one that leaves its target the HP the log shows —
            // or, a strike on a Substitute, the Substitute HP the frame shows (none: broken),
            // the turn's last word on it (R-M8-84).
            for index in 0..strikes.len() {
                let strike = &strikes[index];
                let Some(target) = who_to_id(&before.state, &strike.target) else {
                    continue;
                };
                let on_substitute = strike.hp_after.is_none() && !strike.missed && !strike.immune;
                if on_substitute {
                    let expected = frame_substitute_hp(oracle_frame, &strike.target);
                    let damage = compiled.rules.damage();
                    for roll in (damage.random_min..=damage.random_max).rev() {
                        rolls[index] = Some(roll);
                        let mut candidate = restore_with_tape(
                            compiled,
                            &before,
                            build_residual(&rolls, &tie_tapes[0]),
                        );
                        if candidate.step().is_ok()
                            && substitute_hp(candidate.state(), target) == expected
                        {
                            break;
                        }
                    }
                    continue;
                }
                let Some(expected) = strike.hp_after else {
                    continue;
                };
                let damage = compiled.rules.damage();
                for roll in (damage.random_min..=damage.random_max).rev() {
                    rolls[index] = Some(roll);
                    let mut candidate =
                        restore_with_tape(compiled, &before, build_residual(&rolls, &tie_tapes[0]));
                    if candidate.step().is_err() {
                        continue;
                    }
                    let struck = candidate.trace().events()[trace_len..]
                        .iter()
                        .find_map(|event| match event {
                            TraceEvent::DamageApplied {
                                target: struck,
                                hp_after,
                                source:
                                    pokemon_battle_engine::engine::EffectSource::Move {
                                        move_id, ..
                                    },
                                ..
                            } if *struck == target
                                && matches!(
                                    *move_id,
                                    MoveId::FUTURE_SIGHT | MoveId::DOOM_DESIRE
                                ) =>
                            {
                                Some(*hp_after)
                            }
                            _ => None,
                        });
                    if struck == Some(expected) {
                        break;
                    }
                }
            }
            let mut residual = build_residual(&rolls, &tie_tapes[0]);
            // A battle the residual decides stops it where it is decided: whether a silent
            // handler tied with others counted down before shows in the last frame alone. Take
            // the tie order whose frame is the oracle's.
            if tie_tapes.len() > 1 {
                let fits = |tape: Vec<RandomOverride>| {
                    let mut candidate = restore_with_tape(compiled, &before, tape);
                    let Ok(step) = candidate.step() else {
                        return None;
                    };
                    candidate.state().outcome?;
                    let frame = capture(&candidate, checkpoint, Some(&step));
                    Some(frame_diffs(&frame, oracle_frame).1.is_empty())
                };
                if fits(residual.clone()) == Some(false)
                    && let Some(found) = tie_tapes[1..]
                        .iter()
                        .map(|ties| build_residual(&rolls, ties))
                        .find(|tape| fits(tape.clone()) == Some(true))
                {
                    residual = found;
                }
            }
            let (engine, step, tape) = if residual == resort_ties {
                (probe, probe_step, resort_ties)
            } else {
                let mut engine = restore_with_tape(compiled, &before, residual.clone());
                let step = engine.step().expect("step");
                (engine, step, residual)
            };

            steps.push(ReplayedStep {
                before,
                log_len: compiled.log.len(),
                tape,
                alternatives: Vec::new(),
            });
            compiled.engine = engine;
            follow_drags(compiled, trace_len);
            if !matches!(step, EngineStep::Progressed { .. }) {
                break step;
            }
            continue;
        };

        // Sky Drop's carried Pokémon (§15.23): its action comes to nothing without a word
        // (`onFoeBeforeMove` → `null`), so the log has no action for it.
        // Gen 1's Desync Clause Mod ends an action the same way, the log showing only its
        // `-hint` lines (R-M8-150).
        let sky_dropped = probe.trace().events()[trace_len..].iter().any(|event| {
            matches!(
                event,
                TraceEvent::MoveFailed {
                    actor: failed,
                    reason: pokemon_battle_engine::engine::MoveFailReason::BeforeMove(
                        pokemon_battle_engine::engine::BeforeMoveBlockReason::SkyDropped
                            | pokemon_battle_engine::engine::BeforeMoveBlockReason::Commanding
                    ) | pokemon_battle_engine::engine::MoveFailReason::Desync,
                    ..
                } if *failed == actor
            )
        });
        if sky_dropped {
            steps.push(ReplayedStep {
                before,
                log_len: compiled.log.len(),
                tape: resort_ties,
                alternatives: Vec::new(),
            });
            compiled.engine = probe;
            if !matches!(probe_step, EngineStep::Progressed { .. }) {
                break probe_step;
            }
            continue;
        }

        report.actions_replayed += 1;
        let Some(decision) = decisions.get(decision_index).cloned() else {
            report.divergences.push(format!(
                "{checkpoint}: rust executed an extra action ({actor:?}) the oracle did not report"
            ));
            compiled.engine = probe;
            continue;
        };
        // This action's place in the oracle's list; the dances that follow it are the engine's
        // inside it (Dancer's `runMove`, R-M8-78).
        let index = decision_index;
        decision_index += 1;
        let mut dances = Vec::new();
        while let Some(next) = decisions.get(decision_index).filter(|next| next.danced) {
            dances.push(next.clone());
            decision_index += 1;
        }
        REPLAY_CONTEXT.with(|context| {
            *context.borrow_mut() = format!(
                "{checkpoint}: {} by {}",
                decision.move_name.as_deref().unwrap_or("-"),
                decision.user.name
            )
        });
        if who_to_id(&before.state, &decision.user) != Some(actor) {
            report.divergences.push(format!(
                "{checkpoint}: action order: rust actor {actor:?} but oracle's next action is {:?} by {:?}",
                decision.move_name, decision.user
            ));
        }

        // An Encore that landed before the actor moved overrides its choice (Showdown
        // `OverrideAction`); the engine uses the Encored move, so the plan does too. Gen 3 keeps
        // a lock a caller brought in, which the slots do not hold, and its target with it
        // (`CallRules::encore_overrides_a_called_lock`, R-M8-137).
        let encored = before
            .state
            .pokemon(actor)
            .filter(|p| {
                compiled
                    .rules
                    .conditions()
                    .calls
                    .encore_overrides_a_called_lock
                    || p.hard_lock()
                        .is_none_or(|lock| p.move_slots().iter().any(|m| m.move_id == lock.move_id))
            })
            .and_then(|p| p.volatiles.encored_move())
            .filter(|encored| {
                intended
                    .get(&actor)
                    .is_some_and(|(chosen, _)| chosen != encored)
            });
        // No move was left to choose: the engine's action is Struggle (`BattleState::struggling`,
        // decided at the choice) — the user's own, not an action Instruct injected (R-M11-108).
        let struggling = !decision.instructed && before.state.struggling.contains(&actor);
        let encored = encored.filter(|_| !struggling);
        let move_def = (if struggling {
            Some(MoveId::STRUGGLE)
        } else {
            encored.or_else(|| intended.get(&actor).map(|(id, _)| *id))
        })
        // Gen 1: a user locked into its binding move uses it, whatever it chose (§30.9).
        .map(|id| {
            pokemon_battle_engine::engine::binding_locked_move(&before.state, actor).unwrap_or(id)
        })
        .and_then(|id| compiled.dex.move_by_id(id));
        // An action the engine runs with another move than the one chosen — Instruct's injected
        // action repeats the target's last move (DESIGN.md §42.9): the move the probe used at the
        // top level, when the log's action names it too (R-M8-57).
        let engine_used = probe.trace().events()[trace_len..]
            .iter()
            .find_map(|event| match event {
                TraceEvent::MoveUsed {
                    actor: user,
                    move_id,
                    declared_target,
                    called_by: None,
                    ..
                } if *user == actor => Some((*move_id, *declared_target)),
                _ => None,
            })
            .and_then(|(id, target)| compiled.dex.move_by_id(id).map(|def| (def, target)))
            .filter(|(used, _)| {
                move_def.is_none_or(|chosen| chosen.id != used.id)
                    && decision.move_name.as_deref() == Some(used.name.as_str())
            });
        let move_def = engine_used.map(|(used, _)| used).or(move_def);
        // Where the action aims: the choice's target, or the injected action's own (Instruct's
        // `lastMoveTargetLoc`).
        let chosen_target = match engine_used {
            Some((_, target)) => target,
            None => intended.get(&actor).and_then(|(_, target)| *target),
        };
        // The move the action chose (the caller, when it called another).
        let chosen_move = move_def;
        // The move the action called (Showdown's `[from] move: …` line, DESIGN.md §42.1): the
        // engine runs it instead, so the plan, the fates and the HP search are its.
        let move_def = decision
            .called
            .last()
            .and_then(|called| compiled.dex.moves().iter().find(|m| m.name == called.name))
            .or(move_def);
        let dex_move = move_def;
        // As the user will use it (Weather Ball's type, an -ate ability): the engine's own
        // `modified_move`, on the state before the action (Shell Side Arm's category comes once
        // its target is known, below).
        // A Z-Move replaces the slot's move (Showdown `getActiveZMove` before `ModifyMove`):
        // the engine's own resolution, so the plan sees the move it will run.
        let z_move = intended_mechanics
            .get(&actor)
            .filter(|mechanic| **mechanic == MechanicChoice::ZMove)
            .and_then(|_| {
                pokemon_battle_engine::engine::z_move_execution(
                    &before.state,
                    &compiled.rules,
                    &compiled.dex,
                    actor,
                    dex_move?,
                )
                .map(|z| z.move_def)
            });
        // A Dynamaxed Pokémon's move is its Max Move (DESIGN.md §44.5), the engine's own
        // resolution again.
        let max_move = {
            let modified = z_move.clone().or(dex_move.cloned()).map(|def| {
                pokemon_battle_engine::engine::modified_move(
                    &before.state,
                    &compiled.rules,
                    actor,
                    &def,
                    false,
                )
                .0
            });
            modified.and_then(|modified| {
                pokemon_battle_engine::engine::max_move_execution(
                    &before.state,
                    &compiled.dex,
                    actor,
                    &modified,
                )
                .map(|max| max.move_def)
            })
        };
        // Showdown `runMove` aims a Max or Z action by the move it runs (`getTarget(pokemon,
        // maxMove || zMove || move, targetLoc)`): Mirror Coat's Max Mindstorm keeps the chosen
        // foe, where the base move (`scripted`) would have one drawn (R-M9-45). An action that
        // called another aims by its own move, the caller.
        let aimed_move = if decision.called.is_empty() {
            max_move.clone().or_else(|| z_move.clone())
        } else {
            None
        };
        let move_def = max_move.or(z_move).or(dex_move.cloned()).map(|def| {
            pokemon_battle_engine::engine::modified_move(
                &before.state,
                &compiled.rules,
                actor,
                &def,
                false,
            )
            .0
        });
        // The Pokémon the move will strike, in the engine's target order, each with what
        // Showdown reported for it (a spread move misses or is immune per target).
        let (fates, reflections) = move_def
            .as_ref()
            .and_then(|def| {
                // An overriding Encore re-chooses the target (Showdown: `getRandomTarget`).
                let declared = chosen_target.filter(|_| encored.is_none() && !struggling);
                compiled
                    .rules
                    .targeting()
                    .resolve(
                        compiled.rules.topology(),
                        &before.state,
                        actor,
                        def.target,
                        declared,
                    )
                    .ok()
                    // The declared target is gone (fainted earlier this turn): the engine
                    // retargets, and Showdown's `move` event below names where it went.
                    .or_else(|| {
                        Some(pokemon_battle_engine::rules::ResolvedTargets {
                            targets: Vec::new(),
                        })
                    })
            })
            .map(|targets| {
                let mut ids: Vec<PokemonId> = targets
                    .targets
                    .iter()
                    .filter_map(|t| match t {
                        pokemon_battle_engine::rules::ResolvedTarget::Pokemon(p) if *p != actor => {
                            Some(*p)
                        }
                        _ => None,
                    })
                    .collect();
                // A single-target move goes where Showdown's `move` event says (Follow Me,
                // Lightning Rod and a fainted target change it at execution time).
                let single = matches!(
                    move_def.as_ref().map(|def| def.target),
                    Some(
                        TargetPattern::SelectedAny
                            | TargetPattern::SelectedAdjacent
                            | TargetPattern::SelectedFoe
                            | TargetPattern::SelectedAdjacentFoe
                            | TargetPattern::RandomAdjacentFoe
                    )
                );
                // A reflection (Magic Coat, Magic Bounce: the last called move another Pokémon
                // used) goes back at the actor itself (R-M8-68).
                let reflected_back = decision
                    .called
                    .last()
                    .is_some_and(|called| called.user != decision.user);
                if single
                    && let Some(actual) = decision.move_target.as_ref().and_then(|who| {
                        label_to_id(compiled, who).or(who_to_id(&before.state, who))
                    })
                    && (actual != actor || reflected_back)
                {
                    ids = vec![actual];
                }
                // A move with several targets some of them reflected: each reflection — aimed
                // back from its reflector, at the actor's side — takes the reflector's place.
                // Where every target's TryHit comes before any accuracy (Gen 5+, R-M8-77) they
                // strike first, in turn, then the move its other targets; Gen 3-4 roll each
                // target's accuracy before its TryHit, target by target, so each strikes in its
                // reflector's turn (R-M8-152). A reflection and the move never share a target, so
                // the action's lists tell their fates apart; two reflections strike the same
                // Pokémon, each reported in its own stretch (`called_outcomes`, R-M8-176).
                let mut reflections: Vec<Reflection> = Vec::new();
                // Each id with the reflection that strikes it (`None`: the move itself).
                let mut entries: Vec<(PokemonId, Option<usize>)> =
                    ids.iter().map(|id| (*id, None)).collect();
                if reflected_back
                    && !single
                    && let Some(def) = move_def.as_ref()
                {
                    let sent_back: Vec<(usize, PokemonId)> = decision
                        .called
                        .iter()
                        .enumerate()
                        .filter(|(_, called)| called.user != decision.user)
                        .filter_map(|(index, called)| {
                            who_to_id(&before.state, &called.user).map(|id| (index, id))
                        })
                        .filter(|(_, reflector)| ids.contains(reflector))
                        .collect();
                    let back_from = |reflector: PokemonId| -> Vec<PokemonId> {
                        compiled
                            .rules
                            .targeting()
                            .resolve(
                                compiled.rules.topology(),
                                &before.state,
                                reflector,
                                def.target,
                                before
                                    .state
                                    .active_position(actor, compiled.rules.topology())
                                    .map(DeclaredTarget),
                            )
                            .map(|targets| {
                                targets.pokemon_ids().filter(|p| *p != reflector).collect()
                            })
                            .unwrap_or_default()
                    };
                    let accuracy_before_try_hit = matches!(
                        compiled.rules.hits().step_order,
                        HitStepOrder::AccuracyFirst | HitStepOrder::AccuracyBeforeTryHit
                    );
                    let reflection_of = |id: PokemonId| {
                        sent_back.iter().position(|(_, reflector)| *reflector == id)
                    };
                    let strikes = |at: usize| {
                        let (_, reflector) = sent_back[at];
                        back_from(reflector)
                            .into_iter()
                            .map(move |id| (id, Some(at)))
                    };
                    entries = if accuracy_before_try_hit {
                        ids.iter()
                            .flat_map(|id| match reflection_of(*id) {
                                Some(at) => strikes(at).collect::<Vec<_>>(),
                                None => vec![(*id, None)],
                            })
                            .collect()
                    } else {
                        (0..sent_back.len())
                            .flat_map(strikes)
                            .chain(
                                ids.iter()
                                    .filter(|id| reflection_of(**id).is_none())
                                    .map(|id| (*id, None)),
                            )
                            .collect()
                    };
                    reflections = sent_back
                        .iter()
                        .enumerate()
                        .map(|(at, (called, reflector))| {
                            let first = entries.iter().position(|(_, by)| *by == Some(at));
                            let len = entries.iter().filter(|(_, by)| *by == Some(at)).count();
                            let first = first.unwrap_or(entries.len());
                            Reflection {
                                reflector: *reflector,
                                called: *called,
                                back: first..first + len,
                            }
                        })
                        .collect();
                }
                let listed = |whos: &[Who], id: PokemonId| {
                    whos.iter()
                        .any(|who| who_to_id(&before.state, who) == Some(id))
                };
                let mut fates: Vec<(Option<PokemonId>, TargetFate)> = entries
                    .iter()
                    .map(|(id, by)| {
                        // A reflection's target: by its reflector, in its own stretch.
                        let (attacker, missed, immune) = match by {
                            Some(at) => {
                                let outcomes = called_outcomes(&decision, reflections[*at].called);
                                (reflections[*at].reflector, outcomes.missed, outcomes.immune)
                            }
                            None => (actor, &decision.missed[..], &decision.immune[..]),
                        };
                        // The hit gate's own question: a sure hit, No Guard or a Lock-On
                        // reaches a hiding target.
                        let dodged = move_def.as_ref().is_some_and(|def| {
                            two_turn::dodged_by(&before.state, &compiled.rules, attacker, *id, def)
                                .is_some()
                        });
                        let fate = if dodged {
                            TargetFate::Dodged
                        } else if listed(immune, *id) {
                            TargetFate::Immune
                        } else if listed(missed, *id) {
                            TargetFate::Miss
                        } else {
                            TargetFate::Hit
                        };
                        (Some(*id), fate)
                    })
                    .collect();
                // A miss reported without a target applies to the first.
                if !decision.hit
                    && decision.cant.is_none()
                    && fates.iter().all(|(_, f)| *f == TargetFate::Hit)
                    && let Some((_, first)) = fates.first_mut()
                {
                    *first = TargetFate::Miss;
                }
                (fates, reflections)
            })
            .unwrap_or_default();
        // Dragon Darts (`smartTarget`) in doubles: the declared target and its partner, as the
        // engine splits them; one that took no dart failed its hit steps without a word —
        // immune, behind Protect, or else missed.
        let fates = match move_def.as_ref() {
            Some(def) if def.smart_target => {
                let declared = chosen_target
                    .and_then(|position| {
                        before
                            .state
                            .pokemon_at_position(position.0, compiled.rules.topology())
                    })
                    .or_else(|| fates.first().and_then(|(target, _)| *target));
                // A center of attention (Follow Me …) or a drawing ability holds both darts
                // (`engine::redirector_cancels_smart_target`, R-M8-169).
                let held = declared
                    .and_then(|target| {
                        pokemon_battle_engine::engine::deciding_redirector(
                            &before.state,
                            &compiled.rules,
                            &compiled.dex,
                            actor,
                            def,
                            Some(target),
                        )
                    })
                    .is_some_and(|(_, by)| {
                        pokemon_battle_engine::engine::redirector_cancels_smart_target(by)
                    });
                let split = declared
                    .filter(|_| !held)
                    .map(|target| {
                        pokemon_battle_engine::engine::smart_targets(
                            &before.state,
                            &compiled.rules,
                            actor,
                            target,
                        )
                    })
                    .unwrap_or_default();
                if split.len() > 1 {
                    split
                        .into_iter()
                        .map(|id| {
                            let struck = decision.hit_targets.iter().any(|who| {
                                who.as_ref().and_then(|w| who_to_id(&before.state, w)) == Some(id)
                            });
                            let blocked = pokemon_battle_engine::engine::type_immunity(
                                &before.state,
                                &compiled.rules,
                                id,
                                def,
                            )
                            .1
                            .is_some()
                                || before.state.pokemon(id).is_some_and(|p| {
                                    p.volatiles.contains(
                                        pokemon_battle_engine::state::VolatileConditionKind::Protect,
                                    )
                                });
                            let fate = if struck {
                                TargetFate::Hit
                            } else if blocked {
                                TargetFate::Immune
                            } else {
                                TargetFate::Miss
                            };
                            (Some(id), fate)
                        })
                        .collect()
                } else {
                    fates
                }
            }
            _ => fates,
        };
        // Shell Side Arm as it will strike: physical by its estimates against the first target,
        // a tie as its hint says (the plan reads contact reactions off the category's contact).
        let move_def = move_def.map(|def| match fates.first().and_then(|(target, _)| *target) {
            Some(target) if def.behavior.category_by_estimate => {
                let physical = pokemon_battle_engine::engine::physical_by_estimate(
                    &before.state,
                    &compiled.rules,
                    actor,
                    target,
                    &def,
                )
                .unwrap_or(decision.physical_hint == Some(true));
                match dex_move {
                    Some(dex_def) if physical => {
                        pokemon_battle_engine::engine::modified_move(
                            &before.state,
                            &compiled.rules,
                            actor,
                            dex_def,
                            true,
                        )
                        .0
                    }
                    _ => def,
                }
            }
            _ => def,
        });
        // The move as its `onPrepareHit` handlers leave it: Fling's and Natural Gift's power
        // and Fling's secondary from the item, Parental Bond's two strikes unless it hits
        // several targets.
        let move_def = move_def.map(|def| {
            pokemon_battle_engine::engine::prepared_move(
                &before.state,
                &compiled.rules,
                actor,
                def,
                fates.len(),
                fates.iter().find_map(|(target, _)| *target),
            )
        });
        // Duration hints, per victim (Teeter Dance confuses every Pokémon it reaches, each with
        // its own roll): confusion from the boundary frame, sleep from the later turns, binds
        // and Taunt / Encore / Disable (rolled before Gen 5) from the boundary frame.
        let mut duration_hints: Vec<(PokemonId, DurationKind, u16)> = Vec::new();
        for (who, kind) in &decision.applied {
            let Some(victim) = who_to_id(&before.state, who) else {
                continue;
            };
            let (class, hint) = if kind == "start:confusion" {
                (
                    DurationKind::Confusion,
                    confusion_duration_hint(
                        &compiled.rules,
                        oracle_frame,
                        decisions,
                        index,
                        who,
                        frame_member(&before.state, oracle_frame, who),
                    ),
                )
            } else if kind == "status:slp" {
                (
                    DurationKind::Sleep,
                    sleep_duration_hint(
                        all_decisions,
                        all_entries,
                        all_residuals,
                        turn_index,
                        index,
                        who,
                        compiled.rules.status().lifecycle.sleep_resets_on_switch_in,
                        compiled
                            .rules
                            .status()
                            .lifecycle
                            .sleep_usable_turns_come_back,
                        u16::from(compiled.rules.status().lifecycle.sleep_time_min),
                        u16::from(compiled.rules.status().lifecycle.sleep_time_max),
                        &sleep_steps(
                            &before.state,
                            &compiled.rules,
                            who,
                            turn_index,
                            boundary_frames,
                        ),
                    ),
                )
            } else if kind == "start:partiallytrapped" {
                let ticked = !who_to_id(&before.state, who).is_some_and(|id| {
                    untick.contains(&(id, MissedTick::Residual(ResidualKind::PartiallyTrapped)))
                });
                (
                    DurationKind::Bind,
                    bind_duration_hint(
                        oracle_frame,
                        &compiled.rules,
                        who,
                        frame_member(&before.state, oracle_frame, who),
                        ticked,
                    ),
                )
            } else {
                let conditions = compiled.rules.conditions();
                let (volatile, rule, residual) = if kind == "start:move: Taunt" {
                    ("taunt", conditions.taunt, ResidualKind::Taunt)
                } else if kind == "start:Encore" {
                    ("encore", conditions.encore, ResidualKind::Encore)
                } else if kind.starts_with("start:Disable") {
                    ("disable", conditions.disable, ResidualKind::Disable)
                } else {
                    continue;
                };
                let ticked = !who_to_id(&before.state, who)
                    .is_some_and(|id| untick.contains(&(id, MissedTick::Residual(residual))));
                let hint = if volatile == "disable" && conditions.disable_gen1 {
                    gen1_disable_hint(oracle_frame, decisions, index, who)
                } else {
                    timed_duration_hint(oracle_frame, &before.state, who, volatile, rule, ticked)
                };
                (DurationKind::Timed, hint)
            };
            if let Some(hint) = hint
                && !duration_hints
                    .iter()
                    .any(|(v, c, _)| *v == victim && *c == class)
            {
                duration_hints.push((victim, class, hint));
            }
        }
        // Gen 1's binding lock (§30.9): its turns from the user's `partialtrappinglock` at the
        // boundary, plus the tick the end of the turn already took.
        if compiled.rules.conditions().switching.binding_locks_user
            && move_def
                .as_ref()
                .is_some_and(|def| def.behavior.binding_lock)
        {
            let hint = binding_lock_hint(oracle_frame, &decision.user);
            duration_hints.push((actor, DurationKind::BindingLock, hint));
        }
        // Bide's duration (Gen 2) or time (Gen 1): the user's `bide` at the boundary.
        if move_def.as_ref().is_some_and(|def| def.id == MoveId::BIDE) {
            let ticked = !untick.contains(&(actor, MissedTick::Bide));
            let hint = bide_duration_hint(oracle_frame, &compiled.rules, &decision.user, ticked);
            duration_hints.push((actor, DurationKind::Bide, hint));
        }
        // Uproar (§15.12): Gen 3-4's turns, from the user's `uproar` at the boundary.
        if move_def.as_ref().is_some_and(|def| def.behavior.uproar) {
            let ticked = !untick.contains(&(actor, MissedTick::Residual(ResidualKind::Uproar)));
            let hint = uproar_turns_hint(oracle_frame, &compiled.rules, &decision.user, ticked);
            duration_hints.push((actor, DurationKind::Uproar, hint));
        }
        // Thrash and its kind (§15.12): the lock's length, and the fatigue's confusion.
        if move_def.as_ref().is_some_and(|def| def.behavior.rampage) {
            let hint = rampage_length_hint(
                &compiled.rules,
                &boundary_frames[turn_index..],
                &all_decisions[turn_index..],
                &all_residuals[turn_index..],
                &decision.user,
                !untick.contains(&(actor, MissedTick::Residual(ResidualKind::Rampage))),
            );
            duration_hints.push((actor, DurationKind::Rampage, hint));
        }
        if decision.fatigue
            && !duration_hints
                .iter()
                .any(|(v, c, _)| *v == actor && *c == DurationKind::Confusion)
            && let Some(hint) = confusion_duration_hint(
                &compiled.rules,
                oracle_frame,
                decisions,
                index,
                &decision.user,
                frame_member(&before.state, oracle_frame, &decision.user),
            )
        {
            duration_hints.push((actor, DurationKind::Confusion, hint));
        }
        let substitutes = substitute_hint(
            &before.state,
            &compiled.rules,
            oracle_frame,
            decisions,
            index,
        );
        // An intercepting Pursuit (§15.11) runs while its target is being called back: the
        // engine's accuracy (Gen 3+ never misses it) reads `called_back`.
        let intercept_state;
        let plan_state = if decision.intercepted {
            let mut state = before.state.clone();
            state.called_back = fates.first().and_then(|(target, _)| *target);
            intercept_state = state;
            &intercept_state
        } else if before.state.pending_movers.contains(&actor) && !decision.instructed {
            // The actor's action is under way: it no longer waits to move (Showdown's queue
            // shifted it out) — a move reflected back at it reads that (Zoom Lens:
            // `!queue.willMove(target)`, R-M8-125). An instructed use is an extra action: the
            // actor's own still waits (R-M8-149).
            let mut state = before.state.clone();
            state.pending_movers.retain(|pokemon| *pokemon != actor);
            intercept_state = state;
            &intercept_state
        } else {
            &before.state
        };
        let template = tape_template(
            plan_state,
            &compiled.rules,
            &compiled.dex,
            actor,
            move_def.as_ref(),
            &fates,
            &reflections,
            &decision,
            &duration_hints,
            &substitutes,
        );
        // A single-target move whose declared target is gone, or overridden by Encore, picks a
        // random adjacent foe first (Showdown `getRandomTarget`): the engine says whether it
        // draws, among whom; Showdown's `move` event names the one it picked.
        // The move as chosen, before `ModifyMove` (a non-Ghost's Curse aims at a random foe
        // there, its `self` target comes after: DESIGN.md R281) — the caller's, when it called
        // another: a called move's own target is drawn after the call (`tape_template`).
        let mut template = template;
        // The Curse glitch aims the action at its side's first slot as it starts
        // (`BattleState::curse_glitched`): no foe is drawn for it (R-M9-37).
        let curse_glitched = before.state.curse_glitched.contains(&actor);
        if let Some(def) = aimed_move
            .as_ref()
            .or(chosen_move)
            .filter(|_| !curse_glitched)
        {
            let declared = chosen_target.filter(|_| encored.is_none() && !struggling);
            if let Some(candidates) = pokemon_battle_engine::engine::random_retarget_candidates(
                &before.state,
                &compiled.rules,
                actor,
                def,
                declared,
            ) && candidates.len() > 1
                // The draw comes before BeforeMove: a move that never came (full paralysis, a
                // confusion self-hit) shows no target, and any pick behaves the same.
                // A target outside the candidates was redirected afterwards (Lightning Rod,
                // Follow Me): the draw still came, and any pick ends the same.
                // The action's own line names the pick: a reflection's names its own user
                // (R-M10-73).
                && let Some(picked) = match decision.move_name {
                    None => Some(0),
                    Some(_) => decision
                        .own_target
                        .as_ref()
                        .and_then(|who| who_to_id(&before.state, who))
                        .map(|_| {
                            drawn_target(
                                &before.state,
                                &candidates,
                                decision.own_target.as_ref(),
                                decision.redirected_by.as_ref(),
                            )
                            .unwrap_or(0)
                        }),
                }
            {
                template.insert(
                    0,
                    Some(RandomOverride::range(
                        RandomRequestKind::RandomTarget,
                        picked as u16,
                    )),
                );
            }
        }
        // Gen 9 Curse from a Ghost type aimed at an ally (§15.16): the foe it goes to, drawn
        // after BeforeMove — the last of its decisions (it has no others).
        if let Some(def) = move_def.as_ref()
            && decision.move_name.is_some()
            && let Some(candidates) = pokemon_battle_engine::engine::curse_redirect_candidates(
                &before.state,
                &compiled.rules,
                actor,
                def,
                chosen_target,
            )
            && candidates.len() > 1
        {
            let picked = decision
                .move_target
                .as_ref()
                .and_then(|who| who_to_id(&before.state, who))
                .and_then(|id| candidates.iter().position(|c| *c == id))
                .unwrap_or(0);
            template.push(Some(RandomOverride::range(
                RandomRequestKind::RandomTarget,
                picked as u16,
            )));
        }
        if std::env::var_os("REPLAY_DEBUG").is_some() {
            eprintln!("  {checkpoint}: {actor:?} plans {template:?}");
            eprintln!(
                "    oracle: move {:?} → {:?}, hit {}, immune {:?}, missed {:?}; fates {fates:?}",
                decision.move_name,
                decision.move_target,
                decision.hit,
                decision.immune,
                decision.missed
            );
        }
        // By slot, unless the Pokémon in that slot before the action is not the one named (a
        // Pokémon dragged in during the action): then by label.
        let expected: Vec<(PokemonId, u16)> = decision
            .damages
            .iter()
            .filter_map(|(who, hp)| {
                let by_slot = who_to_id(&before.state, who);
                let by_label = label_to_id(compiled, who);
                let id = match (by_slot, by_label) {
                    (Some(slot), Some(label)) if slot != label => label,
                    (Some(slot), _) => slot,
                    (None, label) => label?,
                };
                Some((id, *hp))
            })
            .collect();
        // The dances that followed it (Dancer's `runMove` inside this action, R-M8-78): each
        // dancer, the actor of its own move, uses it at the target its line names — planned as
        // an action's move is, on the state before the action — and its HP commits and hits
        // come after the action's own.
        let mut expected = expected;
        let mut dance_hits: Vec<(PokemonId, bool)> = Vec::new();
        for (offset, dance) in dances.iter().enumerate() {
            let Some(dancer) = who_to_id(&before.state, &dance.user) else {
                continue;
            };
            let modified = |name: &str| {
                compiled
                    .dex
                    .moves()
                    .iter()
                    .find(|m| m.name == name)
                    .map(|def| {
                        pokemon_battle_engine::engine::modified_move(
                            &before.state,
                            &compiled.rules,
                            dancer,
                            def,
                            false,
                        )
                        .0
                    })
            };
            let dance_def = dance.move_name.as_deref().and_then(modified);
            // A dance BeforeMove stopped (a confusion self-hit) prints no move line: it was the
            // action's move all the same, its random target drawn before the check (R-M8-122) —
            // nothing past BeforeMove (R-M8-126).
            let drawn_def = dance_def
                .clone()
                .or_else(|| decision.move_name.as_deref().and_then(modified));
            let listed = |whos: &[Who], id: PokemonId| {
                whos.iter()
                    .any(|who| who_to_id(&before.state, who) == Some(id))
            };
            let aimed = dance
                .move_target
                .as_ref()
                .and_then(|who| who_to_id(&before.state, who));
            // A dance BeforeMove stopped shows no move line; it aimed where Dancer sends it all the
            // same (`dancersTarget`, the engine's `dancer_target`), so a move that takes a target
            // has none drawn — only a `randomNormal` one draws (R-M9-47).
            let declared = aimed
                .or_else(|| {
                    let original = decision
                        .move_target
                        .as_ref()
                        .and_then(|who| who_to_id(&before.state, who));
                    Some(pokemon_battle_engine::engine::dancer_target(
                        &before.state,
                        actor,
                        dancer,
                        original,
                    ))
                })
                .and_then(|id| before.state.active_position(id, compiled.rules.topology()))
                .map(DeclaredTarget);
            let dance_fates: Vec<(Option<PokemonId>, TargetFate)> = dance_def
                .as_ref()
                .map(|def| {
                    let mut ids: Vec<PokemonId> = compiled
                        .rules
                        .targeting()
                        .resolve(
                            compiled.rules.topology(),
                            &before.state,
                            dancer,
                            def.target,
                            declared,
                        )
                        .map(|targets| targets.pokemon_ids().filter(|p| *p != dancer).collect())
                        .unwrap_or_default();
                    if matches!(
                        def.target,
                        TargetPattern::SelectedAny
                            | TargetPattern::SelectedAdjacent
                            | TargetPattern::SelectedFoe
                            | TargetPattern::SelectedAdjacentFoe
                            | TargetPattern::RandomAdjacentFoe
                    ) && let Some(actual) = aimed.filter(|id| *id != dancer)
                    {
                        ids = vec![actual];
                    }
                    ids.into_iter()
                        .map(|id| {
                            let fate = if listed(&dance.immune, id) {
                                TargetFate::Immune
                            } else if listed(&dance.missed, id) {
                                TargetFate::Miss
                            } else {
                                TargetFate::Hit
                            };
                            (Some(id), fate)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let dance_index = index + 1 + offset;
            let mut dance_hints: Vec<(PokemonId, DurationKind, u16)> = Vec::new();
            for (who, kind) in &dance.applied {
                if kind == "start:confusion"
                    && let Some(victim) = who_to_id(&before.state, who)
                    && let Some(hint) = confusion_duration_hint(
                        &compiled.rules,
                        oracle_frame,
                        decisions,
                        dance_index,
                        who,
                        frame_member(&before.state, oracle_frame, who),
                    )
                {
                    dance_hints.push((victim, DurationKind::Confusion, hint));
                }
            }
            // A dancer's lock (Petal Dance) is drawn, then dropped after the dance (`noLock`).
            if dance_def.as_ref().is_some_and(|def| def.behavior.rampage) {
                let hint = rampage_length_hint(
                    &compiled.rules,
                    &boundary_frames[turn_index..],
                    &all_decisions[turn_index..],
                    &all_residuals[turn_index..],
                    &dance.user,
                    true,
                );
                dance_hints.push((dancer, DurationKind::Rampage, hint));
            }
            let dance_substitutes = substitute_hint(
                &before.state,
                &compiled.rules,
                oracle_frame,
                decisions,
                dance_index,
            );
            // A `randomNormal` dance (Petal Dance) draws its target among the dancer's adjacent
            // foes first (`getRandomTarget`, before BeforeMove, R-M8-78); its line names the pick.
            // The candidates are those still standing as it dances: the action's move (or an
            // earlier dance) may have knocked one out, leaving one alone and nothing to draw
            // (R-M11-58).
            let fallen: Vec<PokemonId> = expected
                .iter()
                .filter(|(_, hp)| *hp == 0)
                .map(|(id, _)| *id)
                .collect();
            if let Some(def) = drawn_def.as_ref()
                && let Some(mut candidates) =
                    pokemon_battle_engine::engine::random_retarget_candidates(
                        &before.state,
                        &compiled.rules,
                        dancer,
                        def,
                        declared,
                    )
                && {
                    candidates.retain(|candidate| !fallen.contains(candidate));
                    candidates.len() > 1
                }
            {
                let picked = aimed
                    .and_then(|id| candidates.iter().position(|c| *c == id))
                    .unwrap_or(0);
                template.push(Some(RandomOverride::range(
                    RandomRequestKind::RandomTarget,
                    picked as u16,
                )));
            }
            // A dancer the move it dances confused (Teeter Dance) meets that confusion at its
            // BeforeMove: the dance is planned on a state where it has it, as long as the action
            // gave it (R-M8-126).
            let confused_by_the_move;
            let dance_state = match duration_hints
                .iter()
                .find(|(victim, kind, _)| *victim == dancer && *kind == DurationKind::Confusion)
            {
                Some((_, _, time))
                    if before
                        .state
                        .pokemon(dancer)
                        .is_some_and(|p| p.volatiles.confusion_time().is_none()) =>
                {
                    let mut state = before.state.clone();
                    if let Some(pokemon) = state.pokemon_mut(dancer) {
                        pokemon.volatiles.set_confusion(actor, *time);
                    }
                    confused_by_the_move = state;
                    &confused_by_the_move
                }
                _ => &before.state,
            };
            template.extend(tape_template(
                dance_state,
                &compiled.rules,
                &compiled.dex,
                dancer,
                dance_def.as_ref(),
                &dance_fates,
                &[],
                dance,
                &dance_hints,
                &dance_substitutes,
            ));
            expected.extend(dance.damages.iter().filter_map(|(who, hp)| {
                let id = who_to_id(&before.state, who).or_else(|| label_to_id(compiled, who))?;
                Some((id, *hp))
            }));
            dance_hits.extend(
                dance
                    .hit_targets
                    .iter()
                    .zip(&dance.substitute_hits)
                    .filter_map(|(who, on_substitute)| {
                        Some((who_to_id(&before.state, who.as_ref()?)?, *on_substitute))
                    }),
            );
        }
        // Forced entries (Roar / Dragon Tail / Red Card): the engine picks from the healthy
        // bench in team order with a RandomTarget decision (none when the bench has one
        // Pokémon); the oracle's `drag` event names who came in. The drag also swaps the
        // Showdown party order like any switch. A Red Card drags as the action ends, after the
        // dances it set off: the log shows the `drag` after them, so it is read off the last
        // dance — the action's all the same, after the dances' draws (R-M9-53).
        let drags: Vec<&Who> = decision
            .dragged_in
            .iter()
            .chain(dances.iter().flat_map(|dance| dance.dragged_in.iter()))
            .collect();
        for entrant_who in drags {
            let Some(entrant) = label_to_id(compiled, entrant_who) else {
                continue;
            };
            let side = if entrant_who.side == 0 {
                SideId::P1
            } else {
                SideId::P2
            };
            let bench = before.state.healthy_bench(side);
            if bench.len() > 1
                && let Some(index) = bench.iter().position(|id| *id == entrant)
            {
                template.push(Some(RandomOverride::range(
                    RandomRequestKind::RandomTarget,
                    index as u16,
                )));
            }
            let slot = entrant_who.slot.unwrap_or(0);
            let outgoing = before.state.side(side).active.get(slot).copied().flatten();
            let party = &mut compiled.party[side.index()];
            if let (Some(from), Some(to)) = (
                outgoing.and_then(|o| party.iter().position(|p| *p == o)),
                party.iter().position(|p| *p == entrant),
            ) {
                party.swap(from, to);
            }
        }
        // The berries eaten at the `Update` after the drag (R-M11-60).
        template.extend(berry_draws_after_drag(
            &before.state,
            &compiled.rules,
            &decision,
            &duration_hints,
        ));
        for dance in &dances {
            template.extend(berry_draws_after_drag(
                &before.state,
                &compiled.rules,
                dance,
                &duration_hints,
            ));
        }
        // The re-sort's tie decisions come before anything the action itself draws; the ties of
        // Neutralizing Gas clearing (Gastro Acid on its holder, or the holder fainting) after.
        template.splice(0..0, resort_ties.iter().copied().map(Some));
        template.extend(
            neutralizing_gas_tape(compiled, &before, oracle_frame)
                .into_iter()
                .map(Some),
        );
        // Instruct's target's use, built as this action ends (`resolveAction`): its priority
        // chances roll then, the last of this action's decisions (R-M9-32).
        if let Some(next) = decisions.get(decision_index).filter(|next| next.instructed) {
            template.extend(instructed_chance_tape(compiled, &before.state, next));
        }
        let hits: Vec<(PokemonId, bool)> = decision
            .hit_targets
            .iter()
            .zip(&decision.substitute_hits)
            .filter_map(|(who, on_substitute)| {
                Some((who_to_id(&before.state, who.as_ref()?)?, *on_substitute))
            })
            .chain(dance_hits)
            .collect();
        // A Substitute this action strikes and leaves standing, and a later action strikes
        // again: the boundary frame only shows it after both, so this action's roll on it stays
        // open. One a later hit of this action breaks (Rock Blast's second, the holder making a
        // new doll later) is not: it does not stand after the action (R-M8-183).
        let broken_here = |holder: &Who| {
            decision
                .substitute_struck
                .iter()
                .any(|(who, broken)| who == holder && *broken)
        };
        let deferred: Vec<PokemonId> = decision
            .substitute_struck
            .iter()
            .filter(|(holder, broken)| {
                !broken
                    && !broken_here(holder)
                    && decisions[decision_index..]
                        .iter()
                        .any(|later| later.substitute_struck.iter().any(|(who, _)| who == holder))
            })
            .filter_map(|(holder, _)| who_to_id(&before.state, holder))
            .collect();
        let ids = |whos: Vec<&Who>| -> Vec<PokemonId> {
            whos.into_iter()
                .filter_map(|who| who_to_id(&before.state, who))
                .collect()
        };
        let holds = ShownHolds {
            sashes: ids(decision.focus_sash_held.iter().collect()),
            bands: ids(decision
                .focus_band_held
                .iter()
                .map(|(holder, _)| holder)
                .collect()),
            sturdy: ids(decision.sturdy_held.iter().collect()),
        }
        .settled();
        let mut unused = None;
        let mut search = search_rolls(
            compiled,
            &before,
            trace_len,
            &template,
            &expected,
            &hits,
            &substitutes,
            &deferred,
            &holds,
            &mut unused,
        );
        // Present's drawn power shows only through the damage: its other bands.
        if search.chosen.is_none() {
            for variant in present_bands(&template, move_def.as_ref()) {
                let retry = search_rolls(
                    compiled,
                    &before,
                    trace_len,
                    &variant,
                    &expected,
                    &hits,
                    &substitutes,
                    &deferred,
                    &holds,
                    &mut unused,
                );
                if retry.chosen.is_some() {
                    search = retry;
                    break;
                }
            }
        }
        // Nothing fits: an earlier action of the boundary may have left a Substitute this one
        // strikes with the wrong HP. Retry it with its other HP values (latest first),
        // replaying the steps after it with the tapes they used, then search this one again.
        if search.chosen.is_none() {
            'backtrack: for from in (0..steps.len()).rev() {
                for tape in steps[from].alternatives.clone() {
                    let (engine, rebuilt, events) = replay_steps(compiled, &steps, from, &tape);
                    // Candidates restored from the rebuilt step's snapshot trace from 0.
                    let again = search_rolls(
                        compiled,
                        &engine.snapshot(),
                        0,
                        &template,
                        &expected,
                        &hits,
                        &substitutes,
                        &deferred,
                        &holds,
                        &mut None,
                    );
                    if again.chosen.is_some() {
                        // The log takes the rebuilt steps' events in place of the old ones.
                        compiled.log.truncate(steps[from].log_len);
                        compiled.log.extend(events);
                        steps = rebuilt;
                        search = again;
                        break 'backtrack;
                    }
                }
            }
        }

        // Nothing fits and the plan holds a crash's hidden critical-hit check (planned as not):
        // try it as one.
        if search.chosen.is_none()
            && let Some(index) = template.iter().position(|entry| {
                entry.is_some_and(|decision| {
                    decision.kind == RandomRequestKind::Critical && decision.optional
                })
            })
        {
            let mut critical = template.clone();
            critical[index] =
                Some(RandomOverride::bool(RandomRequestKind::Critical, true).optional());
            let again = search_rolls(
                compiled,
                &before,
                trace_len,
                &critical,
                &expected,
                &hits,
                &substitutes,
                &deferred,
                &holds,
                &mut None,
            );
            if again.chosen.is_some() {
                search = again;
            }
        }

        // Gen 1: a roll that broke a Substitute leaves `lastDamage` uncapped, which only a later
        // Bide release, Counter or binding lock's next turn shows: the roll sets leaving another
        // value are an open choice, taken where the replay was told to (`OpenChoices`, R-M8-102).
        if compiled.rules.conditions().retaliation.gen1_last_damage
            && let Some((engine, _, tape)) = &search.chosen
            && broke_a_doll(&before.state, engine.state())
            && last_damage_read_later(boundary_frames, turn_index)
        {
            let others = other_last_damages(compiled, &before, trace_len, tape, engine);
            if !others.is_empty() {
                let choice = report.open_choices.met.len();
                report.open_choices.met.push(others.len());
                if let Some(other) = report
                    .open_choices
                    .pins
                    .get(&choice)
                    .and_then(|pick| others.get(*pick))
                {
                    let mut again = restore_with_tape(compiled, &before, other.clone());
                    if let Ok(again_step) = again.step() {
                        search.chosen = Some((again, again_step, other.clone()));
                    }
                }
            }
        }

        // Gen 4 Future Sight / Doom Desire (§15.22): the damage computed at use drew its random
        // factor, which only the strike two turns later shows: the roll that stores that
        // damage (a strike that knocked its target out: any that stores enough).
        let foresight = compiled.rules.conditions().future_attack;
        if let Some(move_def) = move_def.as_ref()
            && move_def.behavior.future_attack
            && foresight.damage_at_use == Some(pokemon_battle_engine::rules::ForeseenRoll::AtUse)
            && let Some((_, _, tape)) = &search.chosen
            && let Some(roll_at) = tape
                .iter()
                .position(|decision| decision.kind == RandomRequestKind::DamageRoll)
            && let Some(target) = decision.move_target.as_ref()
            && let Some(foreseen) = future_damage_hint(boundary_frames, turn_index, target)
        {
            let damage = compiled.rules.damage();
            let tape = tape.clone();
            // A doll another move also struck that turn: every roll whose stored damage fits under
            // the turn's loss is an open choice, the highest first (`OpenChoices`, R-M8-164).
            let mut open: Vec<(BattleEngine, EngineStep, Vec<RandomOverride>)> = Vec::new();
            for roll in (damage.random_min..=damage.random_max).rev() {
                let mut again_tape = tape.clone();
                again_tape[roll_at] = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
                let mut again = restore_with_tape(compiled, &before, again_tape.clone());
                let Ok(again_step) = again.step() else {
                    continue;
                };
                // The attack this use foresaw: the user's on the target's slot (an earlier one
                // of the user's may still be waiting elsewhere, DESIGN.md R277).
                let side = if target.side == 0 {
                    SideId::P1
                } else {
                    SideId::P2
                };
                let stored = again
                    .state()
                    .side(side)
                    .future_attacks
                    .iter()
                    .find(|attack| {
                        attack.source == actor && Some(attack.slot) == target.slot.or(Some(0))
                    })
                    .and_then(|attack| attack.damage);
                match foreseen {
                    ForeseenDamage::Dealt(dealt, knocked_out) => {
                        let fits = stored.is_some_and(|stored| {
                            if knocked_out {
                                stored >= dealt
                            } else {
                                stored == dealt
                            }
                        });
                        if fits {
                            search.chosen = Some((again, again_step, again_tape));
                            break;
                        }
                    }
                    ForeseenDamage::AtMost(lost, broken) => {
                        if stored.is_some_and(|stored| broken || stored <= lost) {
                            open.push((again, again_step, again_tape));
                        }
                    }
                }
            }
            if !open.is_empty() {
                let choice = report.open_choices.met.len();
                report.open_choices.met.push(open.len() - 1);
                let pick = report
                    .open_choices
                    .pins
                    .get(&choice)
                    .map_or(0, |pick| pick + 1)
                    .min(open.len() - 1);
                search.chosen = Some(open.swap_remove(pick));
            }
        }

        // From Gen 5 a foreseen attack draws its random factor as it strikes (§15.22): one that
        // strikes, in the residual, a doll this action struck leaves the boundary frame showing
        // the doll after both, so the action's roll on the doll is open — every roll that leaves
        // the doll standing above the frame's HP, the highest first (`OpenChoices`, R-M10-21).
        // The roll on the doll is the one that moves its HP: a spread move rolls once a target
        // (an Incinerate on the doll and its partner, R-M11-40). Gen 4's strike deals what it
        // stored at use, which `substitute_hint` adds.
        if foresight.damage_at_use != Some(pokemon_battle_engine::rules::ForeseenRoll::AtUse)
            && let Some((_, _, tape)) = &search.chosen
            && let [(holder, false)] = &decision.substitute_struck[..]
            && residual_substitute_strike(oracle_frame, holder)
            && let Some(at_boundary) = frame_substitute_hp(oracle_frame, holder)
            && let Some(id) = who_to_id(&before.state, holder)
            && let Some(roll_at) = doll_roll(compiled, &before, tape, id)
        {
            let damage = compiled.rules.damage();
            let tape = tape.clone();
            let mut open: Vec<(BattleEngine, EngineStep, Vec<RandomOverride>)> = Vec::new();
            let mut dolls: Vec<u16> = Vec::new();
            for roll in (damage.random_min..=damage.random_max).rev() {
                let mut again_tape = tape.clone();
                again_tape[roll_at] = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
                let mut again = restore_with_tape(compiled, &before, again_tape.clone());
                let Ok(again_step) = again.step() else {
                    continue;
                };
                // Rolls that leave the doll the same HP are one alternative.
                if let Some(doll) = substitute_hp(again.state(), id)
                    && doll > at_boundary
                    && !dolls.contains(&doll)
                {
                    dolls.push(doll);
                    open.push((again, again_step, again_tape));
                }
            }
            if !open.is_empty() {
                let choice = report.open_choices.met.len();
                report.open_choices.met.push(open.len() - 1);
                let pick = report
                    .open_choices
                    .pins
                    .get(&choice)
                    .map_or(0, |pick| pick + 1)
                    .min(open.len() - 1);
                search.chosen = Some(open.swap_remove(pick));
            }
        }

        // A Pokémon the action brings in (a Roar's drag) draws its switch-in ability's foe —
        // Trace's, Gen 4-5 Frisk's, Forewarn's — and so does a Trace holder that starts looking
        // during the action (its Neutralizing Gas holder fell), as the action's last draws
        // (DESIGN.md R263, R-M10-42): read from the chosen run, then run again.
        let search_chosen = search.chosen.map(|(engine, step, tape)| {
            let events = &engine.trace().events()[trace_len..];
            let mut lookers = entrants(events);
            if lookers.is_empty() && !traces(oracle_frame) {
                return (engine, step, tape);
            }
            for event in events {
                if let TraceEvent::AbilityChanged { pokemon, from, .. } = event
                    && *from == pokemon_battle_engine::ids::AbilityId::TRACE
                    && !lookers.contains(pokemon)
                {
                    lookers.push(*pokemon);
                }
            }
            let traced = entrant_ability_tape(
                compiled,
                &before.state,
                engine.state(),
                &lookers,
                oracle_frame,
            );
            if traced.is_empty() {
                return (engine, step, tape);
            }
            let mut again_tape = tape.clone();
            again_tape.extend(traced);
            let mut again = restore_with_tape(compiled, &before, again_tape.clone());
            match again.step() {
                Ok(again_step) => (again, again_step, again_tape),
                Err(_) => (engine, step, tape),
            }
        });
        match search_chosen {
            Some((engine, step, tape)) => {
                steps.push(ReplayedStep {
                    before,
                    log_len: compiled.log.len(),
                    tape,
                    alternatives: search.alternatives,
                });
                compiled.engine = engine;
                follow_position_swaps(compiled, trace_len);
                if !matches!(step, EngineStep::Progressed { .. }) {
                    break step;
                }
            }
            None => {
                report.divergences.push(match unused {
                    Some(note) => format!(
                        "{checkpoint}: {} by {:?} (cant={:?}): HP matched but the plan is not the engine's: {note}",
                        decision.move_name.as_deref().unwrap_or("-"),
                        decision.user,
                        decision.cant,
                    ),
                    None => format!(
                        "{checkpoint}: {} by {:?} (cant={:?}): no tape reproduces showdown's HP outcome {:?}; rust gave {:?}",
                        decision.move_name.as_deref().unwrap_or("-"),
                        decision.user,
                        decision.cant,
                        expected,
                        search.seen
                    ),
                });
                compiled.engine = probe;
                follow_position_swaps(compiled, trace_len);
                if !matches!(probe_step, EngineStep::Progressed { .. }) {
                    break probe_step;
                }
            }
        }
    };
    // An action the oracle reported that the engine never ran (a turn Gen 1 lost to
    // `cannotmove` where Showdown's Pokémon acted and was stopped): the boundary is over, the
    // log still has it (R190). Only a move or a `cant` counts: a bare cure of sleep is
    // read as a waking Pokémon's action even when another's move cured it (Haze, Wake-Up
    // Slap).
    if let Some(decision) = decisions[decision_index.min(decisions.len())..]
        .iter()
        .find(|decision| decision.move_name.is_some() || decision.cant.is_some())
    {
        report.divergences.push(format!(
            "{checkpoint}: the oracle's action {:?} by {:?} (cant={:?}) never ran in rust",
            decision.move_name, decision.user, decision.cant
        ));
    }
    step
}

/// Which of `tape`'s damage rolls struck the doll `holder` stands behind: the only one, or the
/// one whose lowest and highest values leave the doll different HP (R-M11-40).
fn doll_roll(
    compiled: &Compiled,
    before: &BattleSnapshot,
    tape: &[RandomOverride],
    holder: PokemonId,
) -> Option<usize> {
    let rolls: Vec<usize> = tape
        .iter()
        .enumerate()
        .filter(|(_, decision)| decision.kind == RandomRequestKind::DamageRoll)
        .map(|(at, _)| at)
        .collect();
    if let [only] = rolls[..] {
        return Some(only);
    }
    let damage = compiled.rules.damage();
    let doll_with = |at: usize, roll: u16| {
        let mut again = tape.to_vec();
        again[at] = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
        let mut engine = restore_with_tape(compiled, before, again);
        engine.step().ok()?;
        substitute_hp(engine.state(), holder)
    };
    rolls.into_iter().find(|&at| {
        let low = doll_with(at, damage.random_min);
        low.is_some() && low != doll_with(at, damage.random_max)
    })
}
