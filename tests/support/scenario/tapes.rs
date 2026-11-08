//! The other random plans: the entrants' and the leads' (Frisk, Forewarn, Trace, speed ties), the
//! residual's, a revival's, the moves with their own draws (Psywave, Present, Acupressure,
//! Conversion, Moody).

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

/// The scheduler's sort key: order, faster first, sub-order, position.
pub(super) type ResidualScheduleKey = (u8, std::cmp::Reverse<u32>, u8, usize, usize);

/// `outside_berry_confusions` with each draw's decision (a confusion's duration, Starf Berry's
/// stat).
pub(super) fn outside_berry_confusion_hints(
    compiled: &Compiled,
    oracle_frame: &Value,
    decisions: &[ActionOracle],
) -> Vec<(Who, RandomOverride, BerryConfusionAt)> {
    let reading = LogReading::for_rules(&compiled.rules, &compiled.dex);
    let events = oracle_frame["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    outside_berry_confusions(&events, compiled.rules.update().after_weather)
        .into_iter()
        .filter_map(|(position, who, at, draw)| {
            let before = extract_decisions(&events[..position], &reading).len();
            let later = decisions.get(before..).unwrap_or(&[]);
            berry_decision(draw, || {
                confusion_hint_after(
                    &compiled.rules,
                    oracle_frame,
                    later,
                    &who,
                    frame_member_at_slot(oracle_frame, &who),
                )
            })
            .map(|decision| (who, decision, at))
        })
        .collect()
}

/// The Pokémon a step brought in, in order.
pub(super) fn entrants(events: &[TraceEvent]) -> Vec<PokemonId> {
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::Switched { incoming, .. } => Some(*incoming),
            _ => None,
        })
        .collect()
}

/// The leads in the order their Frisk lines come (Showdown runs the leads' switch-ins fastest
/// first); the ones without a line last.
pub(super) fn frisk_order(
    compiled: &Compiled,
    state: &BattleState,
    initial: &Value,
) -> Vec<PokemonId> {
    let named: Vec<PokemonId> = frisk_reveals(compiled, state, initial)
        .into_iter()
        .filter_map(|(holder, _)| holder)
        .collect();
    let mut leads: Vec<PokemonId> = [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().flatten().copied())
        .collect();
    leads.sort_by_key(|lead| named.iter().position(|n| n == lead).unwrap_or(usize::MAX));
    leads
}

/// The oracle's Frisk lines in a frame: who frisked (`[of]`) and the item named.
pub(super) fn frisk_reveals(
    compiled: &Compiled,
    state: &BattleState,
    oracle_frame: &Value,
) -> Vec<(
    Option<PokemonId>,
    Option<pokemon_battle_engine::ids::ItemId>,
)> {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["kind"] == "item")
        .filter_map(|event| {
            let args: Vec<&str> = event["args"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            if !args.contains(&"[from] ability: Frisk") {
                return None;
            }
            let holder = args
                .iter()
                .find_map(|arg| arg.strip_prefix("[of] "))
                .and_then(|text| who_from_text(&Value::from(text)))
                .and_then(|who| label_to_id(compiled, &who).or_else(|| who_to_id(state, &who)));
            let item = args
                .get(1)
                .and_then(|name| compiled.dex.item_by_name(name))
                .map(|item| item.id);
            Some((holder, item))
        })
        .collect()
}

/// The ability draws the initial frame's leads make, in the order the engine asks them: the
/// frame's Trace copies, Forewarn warnings (R-M8-103) and Frisk reveals in their log order — a
/// lead whose Trace copied a Frisk draws twice, the copy's `Start` running at once (Gen 4-5,
/// DESIGN.md R251). All are `Sample`s, so planning them in the wrong order would silently pick
/// another candidate. A Gen 4-5 Frisk that found nothing shows no line but still draws
/// (`side.randomFoe()`): a foe holding nothing, in its holder's turn among the leads (DESIGN.md
/// R-M8-84).
pub(super) fn lead_ability_tape(
    compiled: &Compiled,
    state: &BattleState,
    leads: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    let id = |text: &str| {
        who_from_text(&Value::from(text))
            .and_then(|who| label_to_id(compiled, &who).or_else(|| who_to_id(state, &who)))
    };
    let frisk_draws = compiled.rules.conditions().frisk_one_random_foe;
    let foes_of = |seen: &BattleState, holder: PokemonId| -> Vec<PokemonId> {
        let Some(side) = seen.pokemon(holder).map(|pokemon| pokemon.side) else {
            return Vec::new();
        };
        seen.side(side.other())
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|foe| seen.pokemon(*foe).is_some_and(|p| !p.is_fainted()))
            .collect()
    };
    let mut seen = state.clone();
    // Each draw with the lead that makes it, in log order.
    let mut draws: Vec<(PokemonId, RandomOverride)> = Vec::new();
    let mut frisked: Vec<PokemonId> = Vec::new();
    let mut traced_frisk: Vec<PokemonId> = Vec::new();
    for event in oracle_frame["events"].as_array().into_iter().flatten() {
        let kind = event["kind"].as_str().unwrap_or("");
        let args: Vec<&str> = event["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let trace = args.contains(&"[from] ability: Trace");
        let frisk = args.contains(&"[from] ability: Frisk");
        if kind == "ability" && trace {
            let (Some(holder), Some(foe)) = (
                args.first().and_then(|text| id(text)),
                args.iter()
                    .find_map(|arg| arg.strip_prefix("[of] "))
                    .and_then(id),
            ) else {
                continue;
            };
            if leads.contains(&holder) {
                let candidates =
                    pokemon_battle_engine::engine::trace_candidates(&seen, &compiled.rules, holder);
                if candidates.len() >= 2
                    && let Some(index) = candidates.iter().position(|c| *c == foe)
                {
                    draws.push((
                        holder,
                        RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
                    ));
                }
                if args.get(1) == Some(&"Frisk") {
                    traced_frisk.push(holder);
                }
            }
            // The copy is made however the draw went: a foe the next lead's Trace may copy.
            if let Some(copied) = args
                .get(1)
                .and_then(|name| compiled.dex.ability_by_name(name))
                && let Some(pokemon) = seen.pokemon_mut(holder)
            {
                pokemon.change_ability(copied.id);
            }
        } else if kind == "transform" {
            // Imposter's copy as the leads come in (`-transform|USER|TARGET`): a lead whose
            // Trace looks after it sees the ability the user took (DESIGN.md R-M9-23).
            if let (Some(user), Some(target)) = (
                args.first().and_then(|text| id(text)),
                args.get(1).and_then(|text| id(text)),
            ) && let Some(ability) = seen
                .pokemon(target)
                .map(pokemon_battle_engine::engine::raw_ability)
                && let Some(pokemon) = seen.pokemon_mut(user)
            {
                pokemon.change_ability(ability);
            }
        } else if kind == "activate"
            && let Some((holder, draw)) = forewarn_draw(compiled, &seen, &args)
            && leads.contains(&holder)
        {
            draws.push((holder, draw));
        } else if kind == "item" && frisk && frisk_draws {
            let Some(holder) = args
                .iter()
                .find_map(|arg| arg.strip_prefix("[of] "))
                .and_then(id)
            else {
                continue;
            };
            if !leads.contains(&holder) {
                continue;
            }
            frisked.push(holder);
            let foes = foes_of(&seen, holder);
            if foes.len() < 2 {
                continue;
            }
            let named = args
                .get(1)
                .and_then(|name| compiled.dex.item_by_name(name))
                .map(|item| item.id);
            let index = foes
                .iter()
                .position(|foe| seen.pokemon(*foe).and_then(|p| p.held_item) == named)
                .unwrap_or(0);
            draws.push((
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ));
        }
    }
    if frisk_draws {
        for &holder in leads {
            let native = state.pokemon(holder).is_some_and(|pokemon| {
                pokemon_battle_engine::engine::ability_active(state, &compiled.rules, pokemon)
                    == Some(pokemon_battle_engine::ids::AbilityId::FRISK)
            });
            if !(native || traced_frisk.contains(&holder)) || frisked.contains(&holder) {
                continue;
            }
            let foes = foes_of(&seen, holder);
            if foes.len() < 2 {
                continue;
            }
            let index = foes
                .iter()
                .position(|foe| seen.pokemon(*foe).is_some_and(|p| p.held_item.is_none()))
                .unwrap_or(0);
            draws.push((
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ));
        }
    }
    // The leads start fastest first, the ties as Showdown reported them (`lead_tie_tape`); a
    // lead's own draws keep their order.
    let order = lead_order(compiled, Some(oracle_frame));
    draws.sort_by_key(|(holder, _)| order.iter().position(|lead| lead == holder));
    draws.into_iter().map(|(_, draw)| draw).collect()
}

/// Forewarn's draw (`sample` among the foes' strongest moves, two or more) that the frame's
/// `-activate|holder|ability: Forewarn|<move>|[of] <foe>` (`args`) shows: the holder and the
/// candidate it names, optional like Trace's (R-M8-103). The gen4 mod names no foe: the first
/// candidate with that move stands for it — they warn alike (R-M8-138).
pub(super) fn forewarn_draw(
    compiled: &Compiled,
    state: &BattleState,
    args: &[&str],
) -> Option<(PokemonId, RandomOverride)> {
    let id = |text: &str| {
        who_from_text(&Value::from(text))
            .and_then(|who| label_to_id(compiled, &who).or_else(|| who_to_id(state, &who)))
    };
    forewarn_pick(&compiled.rules, &compiled.dex, state, args, id)
}

/// `forewarn_draw`'s reading, the line's Pokémon named through `id`.
pub(super) fn forewarn_pick(
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    state: &BattleState,
    args: &[&str],
    id: impl Fn(&str) -> Option<PokemonId>,
) -> Option<(PokemonId, RandomOverride)> {
    if args.get(1) != Some(&"ability: Forewarn") {
        return None;
    }
    let holder = args.first().and_then(|text| id(text))?;
    let foe = args
        .iter()
        .find_map(|arg| arg.strip_prefix("[of] "))
        .and_then(&id);
    let named = dex.move_by_name(args.get(2)?)?.id;
    let candidates = pokemon_battle_engine::engine::forewarn_candidates(state, rules, holder);
    if candidates.len() < 2 {
        return None;
    }
    let index = candidates
        .iter()
        .position(|c| c.0 == named && foe.is_none_or(|foe| c.1 == foe))?;
    Some((
        holder,
        RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
    ))
}

/// Forewarn's draws for the holders among `entrants` (`forewarn_draw`), `state` the engine's
/// once they are in.
pub(super) fn forewarn_draws(
    compiled: &Compiled,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<(PokemonId, RandomOverride)> {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["kind"] == "activate")
        .filter_map(|event| {
            let args: Vec<&str> = event["args"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            forewarn_draw(compiled, state, &args)
        })
        .filter(|(holder, _)| entrants.contains(holder))
        .collect()
}

/// Gen 4-5 Frisk's foe (`side.randomFoe()`: a `Sample` among two foes or more) for each Frisk
/// holder among `entrants`: the first foe holding the item the oracle's `-item||<item>|[from]
/// ability: Frisk|[of] <holder>` names — or, without that line, one holding nothing. `state`
/// is the engine's once they are in. Optional, and put before the entry berries: a berry eaten
/// on the hazards comes before the ability, one eaten at the item's turn after it.
pub(super) fn frisk_draws(
    compiled: &Compiled,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<(PokemonId, RandomOverride)> {
    if !compiled.rules.conditions().frisk_one_random_foe {
        return Vec::new();
    }
    let revealed = frisk_reveals(compiled, state, oracle_frame);
    entrants
        .iter()
        .filter_map(|&holder| {
            let pokemon = state.pokemon(holder)?;
            if pokemon_battle_engine::engine::ability_active(state, &compiled.rules, pokemon)
                != Some(pokemon_battle_engine::ids::AbilityId::FRISK)
            {
                return None;
            }
            let foes: Vec<PokemonId> = state
                .side(pokemon.side.other())
                .active
                .iter()
                .flatten()
                .copied()
                .filter(|foe| state.pokemon(*foe).is_some_and(|p| !p.is_fainted()))
                .collect();
            if foes.len() < 2 {
                return None;
            }
            let named = revealed
                .iter()
                .find(|(by, _)| *by == Some(holder))
                .and_then(|(_, item)| *item);
            let index = foes
                .iter()
                .position(|foe| state.pokemon(*foe).and_then(|p| p.held_item) == named)
                .unwrap_or(0);
            Some((
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ))
        })
        .collect()
}

/// The entrants' switch-in ability draws (Trace's foe, Frisk's), in the order the engine runs
/// their switch-ins: fastest first (`engine::entry_speed`), one entrant's own draws in their
/// order (a Trace that copied Frisk draws for the copy after its own). Both are `Sample`s, so
/// a fixed Frisk-then-Trace order picked the other's foe whenever the Trace holder was the
/// faster (DESIGN.md R-M8-92). A Trace already on the field that seeks as a foe comes in looks
/// after the switch-ins, in the log's order.
pub(super) fn entrant_ability_tape(
    compiled: &Compiled,
    before: &BattleState,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    let (mut entering, seeking): (Vec<_>, Vec<_>) =
        trace_draws(compiled, before, state, entrants, oracle_frame)
            .into_iter()
            .partition(|(holder, _)| entrants.contains(holder));
    entering.extend(frisk_draws(compiled, state, entrants, oracle_frame));
    entering.extend(forewarn_draws(compiled, state, entrants, oracle_frame));
    entering.sort_by_key(|(holder, _)| {
        std::cmp::Reverse(pokemon_battle_engine::engine::entry_speed(
            state,
            &compiled.rules,
            *holder,
        ))
    });
    entering
        .into_iter()
        .chain(seeking)
        .map(|(_, draw)| draw)
        .collect()
}

/// Whether the frame shows Forewarn naming a move.
pub(super) fn forewarns(oracle_frame: &Value) -> bool {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|event| event["kind"] == "activate" && event["args"][1] == "ability: Forewarn")
}

/// Whether the frame shows Trace copying an ability.
pub(super) fn traces(oracle_frame: &Value) -> bool {
    oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|event| {
            event["kind"] == "ability"
                && event["args"]
                    .as_array()
                    .is_some_and(|args| args.iter().any(|arg| arg == "[from] ability: Trace"))
        })
}

/// Trace's foe (`this.sample(possibleTargets)`: a `Sample` among two foes or more, R237) for
/// each of the frame's `-ability|<holder>|<ability>|…|[from] ability: Trace|[of] <foe>` lines,
/// in order, whose holder looks in this step — one of its `entrants`, or a holder still seeking
/// in `before` (a foe came in): the foe's place among `engine::trace_candidates` on the field
/// as that Trace saw it — `state` (the engine's once the step's entrants are in) with the
/// earlier lines' copies made, since a Trace that copied an ability is a foe the next Trace may
/// copy (DESIGN.md R251). A line an earlier step already settled draws nothing here.
/// Optional: the step may not be the one Trace looks in.
pub(super) fn trace_tape(
    compiled: &Compiled,
    before: &BattleState,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    trace_draws(compiled, before, state, entrants, oracle_frame)
        .into_iter()
        .map(|(_, draw)| draw)
        .collect()
}

/// `trace_tape`'s draws, each with its Trace holder.
pub(super) fn trace_draws(
    compiled: &Compiled,
    before: &BattleState,
    state: &BattleState,
    entrants: &[PokemonId],
    oracle_frame: &Value,
) -> Vec<(PokemonId, RandomOverride)> {
    let id = |text: &str| {
        who_from_text(&Value::from(text))
            .and_then(|who| label_to_id(compiled, &who).or_else(|| who_to_id(state, &who)))
    };
    let mut seen = state.clone();
    let mut tape = Vec::new();
    for event in oracle_frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["kind"] == "ability")
    {
        let args: Vec<&str> = event["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !args.contains(&"[from] ability: Trace") {
            continue;
        }
        let (Some(holder), Some(foe)) = (
            args.first().and_then(|text| id(text)),
            args.iter()
                .find_map(|arg| arg.strip_prefix("[of] "))
                .and_then(id),
        ) else {
            continue;
        };
        let looks = entrants.contains(&holder)
            || before
                .pokemon(holder)
                .is_some_and(|pokemon| pokemon.ability_state.trace_seek);
        let candidates =
            pokemon_battle_engine::engine::trace_candidates(&seen, &compiled.rules, holder);
        // The field as an entrant's Trace first looked at it: a foe that comes in later in the
        // step is not there yet, the Pokémon it replaces still is.
        let mut at_entry = seen.clone();
        if let Some(at) = entrants.iter().position(|entrant| *entrant == holder) {
            for later in &entrants[at + 1..] {
                let Some(side) = at_entry.pokemon(*later).map(|pokemon| pokemon.side) else {
                    continue;
                };
                let Some(slot) = at_entry
                    .side(side)
                    .active
                    .iter()
                    .position(|occupant| *occupant == Some(*later))
                else {
                    continue;
                };
                let previous = before.side(side).active.get(slot).copied().flatten();
                at_entry.side_mut(side).active[slot] = previous;
            }
        }
        let first_look =
            pokemon_battle_engine::engine::trace_candidates(&at_entry, &compiled.rules, holder);
        // The line names a foe not there at that first look: Trace drew a foe whose ability
        // `setAbility` refuses (Gen 9 Gulp Missile: `cantsuppress` without `notrace`) and kept
        // looking, to copy the named foe at an `Update` once it had come in. It looked twice
        // before: as it started, and at the `Update` that closes its switch — the same foe
        // each time.
        if looks && entrants.contains(&holder) && !first_look.contains(&foe) {
            if first_look.len() >= 2
                && let Some(index) = first_look.iter().position(|candidate| {
                    at_entry.pokemon(*candidate).is_some_and(|pokemon| {
                        compiled
                            .rules
                            .conditions()
                            .cant_suppress
                            .contains(&pokemon_battle_engine::engine::raw_ability(pokemon))
                    })
                })
            {
                for _ in 0..2 {
                    tape.push((
                        holder,
                        RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
                    ));
                }
            }
            // The foe came in later in the step: the copy is this step's too, from the field
            // the step leaves (otherwise a later step's, where the holder still seeks).
            if !candidates.contains(&foe) {
                continue;
            }
        }
        if looks
            && candidates.len() >= 2
            && let Some(index) = candidates.iter().position(|c| *c == foe)
        {
            tape.push((
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16).optional(),
            ));
        }
        if let Some(copied) = args
            .get(1)
            .and_then(|name| compiled.dex.ability_by_name(name))
            && let Some(pokemon) = seen.pokemon_mut(holder)
        {
            pokemon.change_ability(copied.id);
        }
    }
    tape
}

/// The berry draws of the residual's closing `Update` (no berry handler of its own: Gen 4+), after
/// every residual handler — Moody's included (DESIGN.md R263) — in the active order.
pub(super) fn residual_end_berry_tape(
    state: &BattleState,
    rules: &ResolvedRules,
    berry_confusions: &[(Who, RandomOverride, BerryConfusionAt)],
) -> Vec<RandomOverride> {
    if rules.residual().key(ResidualKind::Berry).is_some() {
        return Vec::new();
    }
    [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().copied().flatten())
        .flat_map(|id| {
            berry_confusions
                .iter()
                .filter(move |(who, _, at)| {
                    *at == BerryConfusionAt::ResidualEnd && who_to_id(state, who) == Some(id)
                })
                .map(|(_, decision, _)| *decision)
        })
        .collect()
}

/// The switch-ins' berry draws, optional (whichever switch-in asks takes one).
pub(super) fn entry_berry_durations(
    berry_confusions: &[(Who, RandomOverride, BerryConfusionAt)],
) -> Vec<RandomOverride> {
    berry_confusions
        .iter()
        .filter(|(_, _, at)| *at == BerryConfusionAt::Entry)
        .map(|(_, decision, _)| decision.optional())
        .collect()
}

/// The end-of-turn decisions in schedule order (order, faster first, sub-order, position — as
/// the scheduler orders them): Shed Skin for every statused holder on the field, and Gen 2's
/// thaw roll for every frozen one.
/// The end of the turn's `SpeedTie` decisions (DESIGN.md §12.4): for every group of tied
/// residual handlers, then of Pokémon the weather chips at the same speed, the one the closing
/// events show first gets the larger draw. A handler that shows nothing (a Taunt still
/// running) goes last; its place changes nothing observable.
/// The leads in the order `run_lead_switch_ins` starts them: fastest first, the ties in the
/// order Showdown first names them (as `lead_tie_tape` plans).
pub(super) fn lead_order(compiled: &Compiled, initial: Option<&Value>) -> Vec<PokemonId> {
    let shown_at = first_named_at(compiled, initial);
    let mut leads = compiled.engine.lead_speeds();
    leads.sort_by_key(|(id, speed)| (std::cmp::Reverse(*speed), shown_at(*id)));
    leads.into_iter().map(|(id, _)| id).collect()
}

/// Where each Pokémon is first named among a frame's events (switches aside): the leads in the
/// initial frame, simultaneous entrants in their replacements' frame.
pub(super) fn first_named_at(
    compiled: &Compiled,
    initial: Option<&Value>,
) -> impl Fn(PokemonId) -> usize {
    let events = initial
        .and_then(|frame| frame["events"].as_array())
        .cloned()
        .unwrap_or_default();
    let state = &compiled.engine.state();
    let mut named: Vec<PokemonId> = Vec::new();
    for event in events.iter().filter(|event| event["kind"] != "switch") {
        let args = event["args"].as_array().cloned().unwrap_or_default();
        let whos = std::iter::once(serde_json::from_value::<Who>(event["pokemon"].clone()).ok())
            .chain(args.iter().map(|arg| {
                let text = arg.as_str()?;
                who_from_text(&Value::from(text.strip_prefix("[of] ").unwrap_or(text)))
            }))
            .flatten();
        for who in whos {
            if let Some(id) = label_to_id(compiled, &who).or_else(|| who_to_id(state, &who))
                && !named.contains(&id)
            {
                named.push(id);
            }
        }
    }
    move |id: PokemonId| named.iter().position(|n| *n == id).unwrap_or(usize::MAX)
}

/// The SpeedTie decisions of the leads' switch-in batch (`BattleEngine::lead_speed_ties`): tied
/// leads go in the order the initial frame first names them (the source of a weather, the
/// Intimidate user …); one it never names comes last.
pub(super) fn lead_tie_tape(compiled: &Compiled, initial: Option<&Value>) -> Vec<RandomOverride> {
    let ties = compiled.engine.lead_speed_ties();
    if ties.is_empty() {
        return Vec::new();
    }
    let shown_at = first_named_at(compiled, initial);
    ties.iter()
        .map(|(lead, speed)| {
            let mut members: Vec<PokemonId> = ties
                .iter()
                .filter(|(_, s)| s == speed)
                .map(|(id, _)| *id)
                .collect();
            members.sort_by_key(|id| shown_at(*id));
            let rank = members.iter().position(|id| id == lead).unwrap_or(0);
            RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
        })
        .collect()
}

/// The SpeedTie decisions of the order simultaneous replacements come in (a probe's
/// `ReplacementTiesDrawn`): tied entrants go in the order the frame's `switch` events name them.
pub(super) fn replacement_tie_tape(
    compiled: &Compiled,
    events: &[TraceEvent],
    frame: &Value,
) -> Vec<RandomOverride> {
    let state = compiled.engine.state();
    let switched: Vec<PokemonId> = frame["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["kind"] == "switch")
        .filter_map(|event| {
            let who = serde_json::from_value::<Who>(event["pokemon"].clone()).ok()?;
            label_to_id(compiled, &who).or_else(|| who_to_id(state, &who))
        })
        .collect();
    let shown_at = |id: PokemonId| switched.iter().position(|s| *s == id).unwrap_or(usize::MAX);
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::ReplacementTiesDrawn { tied } => Some(tied),
            _ => None,
        })
        .flat_map(|tied| {
            tied.iter().map(|(entrant, speed)| {
                let mut members: Vec<PokemonId> = tied
                    .iter()
                    .filter(|(_, s)| s == speed)
                    .map(|(id, _)| *id)
                    .collect();
                members.sort_by_key(|id| shown_at(*id));
                let rank = members.iter().position(|id| id == entrant).unwrap_or(0);
                RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
            })
        })
        .collect()
}

/// The SpeedTie decisions of simultaneous replacements (a probe's `SwitchInTiesDrawn`): tied
/// entrants go in the order the frame first names them, as the leads do. Unplanned, the
/// decisions fell to the PRNG — and a Trace entrant's optional foe draw, planned next, was
/// passed over by the first of them (R-M8-140).
pub(super) fn entry_tie_tape(
    compiled: &Compiled,
    events: &[TraceEvent],
    frame: &Value,
) -> Vec<RandomOverride> {
    let shown_at = first_named_at(compiled, Some(frame));
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::SwitchInTiesDrawn { tied } => Some(tied),
            _ => None,
        })
        .flat_map(|tied| {
            tied.iter().map(|(entrant, speed)| {
                let mut members: Vec<PokemonId> = tied
                    .iter()
                    .filter(|(_, s)| s == speed)
                    .map(|(id, _)| *id)
                    .collect();
                members.sort_by_key(|id| shown_at(*id));
                let rank = members.iter().position(|id| id == entrant).unwrap_or(0);
                RandomOverride::raw(RandomRequestKind::SpeedTie, (members.len() - rank) as u64)
            })
        })
        .collect()
}

/// The residual's `SpeedTie` decisions: the tapes to try, the first the plan. A tied handler
/// the log shows goes where it shows; one it does not — a duration-only handler, counting down
/// in silence — keeps its collection order in the plan, and the other tapes put it everywhere
/// else among its group. No frame shows that order unless the battle ends between them (the
/// residual stops there): the replay then takes the tape whose frame is the oracle's.
pub(super) fn residual_tie_tapes(
    compiled: &Compiled,
    state: &BattleState,
    oracle_frame: &Value,
) -> (Vec<Vec<RandomOverride>>, Vec<RandomOverride>) {
    let ties = compiled.engine.pending_residual_ties();
    if ties.handlers.is_empty() && ties.durations.is_empty() && ties.weather.is_empty() {
        return (vec![Vec::new()], Vec::new());
    }
    // What each closing event stands for: a residual kind on a Pokémon, or (`None`) the
    // weather's chip on it.
    let events = oracle_frame["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let start = events
        .iter()
        .position(|event| event["kind"] == "residual")
        .map_or(events.len(), |marker| marker + 1);
    let id_of = |who: &Who| label_to_id(compiled, who).or_else(|| who_to_id(state, who));
    let mut shown: Vec<(PokemonId, Option<ResidualKind>)> = Vec::new();
    for event in &events[start..] {
        let from = event["from"].as_str().unwrap_or("");
        let args = event["args"].as_array().cloned().unwrap_or_default();
        let arg = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or("");
        let pokemon = serde_json::from_value::<Who>(event["pokemon"].clone()).ok();
        let (who, what) = match event["kind"].as_str().unwrap_or("") {
            "damage" | "heal" => {
                let what = if from.contains("Leech Seed") {
                    Some(ResidualKind::LeechSeed)
                } else if from == "[from] psn" {
                    Some(ResidualKind::Poison)
                } else if from == "[from] brn" {
                    Some(ResidualKind::Burn)
                } else if from == "[from] item: Leftovers" {
                    Some(ResidualKind::Leftovers)
                } else if from == "[from] item: Black Sludge" {
                    Some(ResidualKind::BlackSludge)
                } else if from == "[from] move: Ingrain" {
                    Some(ResidualKind::Ingrain)
                } else if from == "[from] Grassy Terrain" {
                    Some(ResidualKind::GrassyTerrain)
                } else if from == "[from] Salt Cure" {
                    Some(ResidualKind::SaltCure)
                } else if event["of"].as_str() == Some("[partiallytrapped]") {
                    Some(ResidualKind::PartiallyTrapped)
                } else if from == "[from] Sandstorm" || from == "[from] Hail" {
                    None
                } else {
                    continue;
                };
                (pokemon, what)
            }
            "status" if from.contains("item: Toxic Orb") || from.contains("item: Flame Orb") => {
                (pokemon, Some(ResidualKind::StatusOrb))
            }
            "boost" if event["stat"] == "spe" => (pokemon, Some(ResidualKind::SpeedBoost)),
            "unboost" if event["stat"] == "spe" => (pokemon, Some(ResidualKind::SyrupBomb)),
            "curestatus" if event["status"] == "frz" => (pokemon, Some(ResidualKind::FreezeThaw)),
            "activate" if arg(1) == "ability: Shed Skin" => {
                (who_from_text(&args[0]), Some(ResidualKind::ShedSkin))
            }
            "activate" if arg(1) == "ability: Cud Chew" => {
                (who_from_text(&args[0]), Some(ResidualKind::CudChew))
            }
            // Moody (`-ability|p|Moody|boost`, then its `-boost` / `-unboost`).
            "ability" if arg(1) == "Moody" && arg(2) == "boost" => (
                args.first().and_then(who_from_text),
                Some(ResidualKind::Moody),
            ),
            "item" if arg(2) == "[from] ability: Harvest" => (
                args.first().and_then(who_from_text),
                Some(ResidualKind::Harvest),
            ),
            // Pickup's find (`-item|p|X|[from] ability: Pickup`): two holders that tie pick up in
            // the order shown — the first's find may be the second's to pick up (R-M8-84).
            "item" if arg(2) == "[from] ability: Pickup" => (
                args.first().and_then(who_from_text),
                Some(ResidualKind::Pickup),
            ),
            // A lock's fatigue (Thrash and its kind) ending in the residual: its `-start … confusion
            // [fatigue]` orders the tied handlers — two locks may end on the same turn
            // (DESIGN.md R-M8-44).
            "start"
                if arg(1) == "confusion"
                    && args.iter().any(|a| matches!(a.as_str(), Some("[fatigue]"))) =>
            {
                // A `-start` names its Pokémon in its first argument (R-M8-79).
                (
                    args.first().and_then(who_from_text),
                    Some(ResidualKind::Rampage),
                )
            }
            // Perish Song's count (`-start|p|perishN`), its end at 0 included.
            "start" if arg(1).starts_with("perish") => (
                args.first().and_then(who_from_text),
                Some(ResidualKind::PerishSong),
            ),
            "end" => {
                let what = match arg(1) {
                    // A foreseen attack striking (DESIGN.md R277).
                    "move: Future Sight" | "move: Doom Desire" => ResidualKind::FutureAttack,
                    "move: Yawn" => ResidualKind::Yawn,
                    "move: Taunt" => ResidualKind::Taunt,
                    "Encore" => ResidualKind::Encore,
                    "Disable" => ResidualKind::Disable,
                    _ => continue,
                };
                (args.first().and_then(who_from_text), Some(what))
            }
            _ => continue,
        };
        if let Some(id) = who.as_ref().and_then(id_of)
            && !shown.contains(&(id, what))
        {
            shown.push((id, what));
        }
    }
    let same = |kind: ResidualKind| match kind {
        ResidualKind::Toxic => ResidualKind::Poison,
        other => other,
    };
    let position = |id: PokemonId, what: Option<ResidualKind>| {
        shown
            .iter()
            .position(|(who, w)| *who == id && w.map(same) == what.map(same))
            .unwrap_or(usize::MAX)
    };
    let ranked = |members: Vec<(usize, usize)>, of: usize| {
        let mut ordered = members;
        ordered.sort_by_key(|(_, shown_at)| *shown_at);
        let rank = ordered.iter().position(|(i, _)| *i == of).unwrap_or(0);
        RandomOverride::raw(RandomRequestKind::SpeedTie, (ordered.len() - rank) as u64)
    };
    // The handlers' decisions in draw order — the ones with a callback, then the duration-only
    // ones — each with its group and where the log shows it (`usize::MAX`: nowhere).
    let drawn: Vec<(usize, usize)> = ties
        .handlers
        .iter()
        .map(|(holder, kind, group)| (*group, position(*holder, Some(*kind))))
        .chain(ties.durations.iter().map(|(_, group)| (*group, usize::MAX)))
        .collect();
    let mut weather = Vec::new();
    for (index, (_, group)) in ties.weather.iter().enumerate() {
        let members = ties
            .weather
            .iter()
            .enumerate()
            .filter(|(_, (_, g))| g == group)
            .map(|(i, (pokemon, _))| (i, position(*pokemon, None)))
            .collect();
        weather.push(ranked(members, index));
    }
    // Each group's orders: the plan's (shown first, the rest in draw order), then — for a
    // group with a handler the log does not show — every other order that keeps the shown ones
    // as shown. An order lists the group's members (indices into `drawn`) first to last.
    let mut groups: Vec<usize> = drawn.iter().map(|(group, _)| *group).collect();
    groups.dedup();
    groups.sort_unstable();
    groups.dedup();
    let orders: Vec<Vec<Vec<usize>>> = groups
        .iter()
        .map(|group| {
            let mut members: Vec<usize> =
                (0..drawn.len()).filter(|i| drawn[*i].0 == *group).collect();
            members.sort_by_key(|i| (drawn[*i].1, *i));
            let mut orders = vec![members.clone()];
            if members.iter().any(|i| drawn[*i].1 == usize::MAX) && members.len() <= 5 {
                let shown: Vec<usize> = members
                    .iter()
                    .copied()
                    .filter(|i| drawn[*i].1 != usize::MAX)
                    .collect();
                for order in permutations(&members) {
                    let keeps_shown = order
                        .iter()
                        .copied()
                        .filter(|i| drawn[*i].1 != usize::MAX)
                        .eq(shown.iter().copied());
                    if keeps_shown && order != members {
                        orders.push(order);
                    }
                }
            }
            orders
        })
        .collect();
    let tape_for = |choice: &[usize]| {
        let mut values = vec![0u64; drawn.len()];
        for (group, &pick) in choice.iter().enumerate() {
            let order = &orders[group][pick];
            for (rank, member) in order.iter().enumerate() {
                values[*member] = (order.len() - rank) as u64;
            }
        }
        values
            .into_iter()
            .map(|value| RandomOverride::raw(RandomRequestKind::SpeedTie, value))
            .collect::<Vec<RandomOverride>>()
    };
    // The plan, then each group's other orders one group at a time, then their combinations
    // (a handful at most: tied silent handlers come in twos and threes).
    let mut choice = vec![0usize; orders.len()];
    let mut tapes = vec![tape_for(&choice)];
    'combinations: loop {
        let mut digit = 0;
        loop {
            if digit == orders.len() {
                break 'combinations;
            }
            choice[digit] += 1;
            if choice[digit] < orders[digit].len() {
                break;
            }
            choice[digit] = 0;
            digit += 1;
        }
        tapes.push(tape_for(&choice));
        if tapes.len() >= 64 {
            break;
        }
    }
    (tapes, weather)
}

/// Every order of `items`.
pub(super) fn permutations(items: &[usize]) -> Vec<Vec<usize>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut all = Vec::new();
    for (index, first) in items.iter().enumerate() {
        let mut rest = items.to_vec();
        rest.remove(index);
        for mut tail in permutations(&rest) {
            tail.insert(0, *first);
            all.push(tail);
        }
    }
    all
}

#[allow(clippy::too_many_arguments)]
pub(super) fn residual_tape(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    cures: &[(Who, ResidualKind)],
    faints: &[(Who, ResidualKind, Who)],
    yawn_sleeps: &[(Who, u16)],
    berry_confusions: &[(Who, RandomOverride, BerryConfusionAt)],
    fatigues: &[(Who, u16)],
    strikes: &[FutureStrikeOracle],
    rolls: &[Option<u16>],
    moody: &[(PokemonId, ResidualKind, RandomOverride)],
    weather_ties: &[RandomOverride],
) -> Vec<RandomOverride> {
    let mut decisions: Vec<(ResidualScheduleKey, RandomOverride)> = Vec::new();
    // The weather's targets tied in speed (`residual_tie_tapes`): shuffled as the weather's own
    // handler runs (its chip's `eachEvent('Weather')`), at its place in the residual — after a
    // Gen 2 foreseen attack's strike, before its chip (R-M11-114). The engine keys the field's
    // handler at speed 0.
    if let Some((order, sub_order)) = rules.residual().key(ResidualKind::Weather) {
        for (rank, tie) in weather_ties.iter().enumerate() {
            decisions.push(((order, std::cmp::Reverse(0), sub_order, 0, rank), *tie));
        }
    }
    // Moody's and Pickup's draws, by their handler's place: the log's order breaks a tie in speed
    // (`residual_tie_tape` planned it), and one holder's draws keep their own order.
    for (rank, (holder, kind, draw)) in moody.iter().enumerate() {
        let (order, sub_order) = rules.residual().key(*kind).unwrap_or((u8::MAX, u8::MAX));
        let speed = pokemon_battle_engine::engine::residual_speed(state, rules, *holder);
        decisions.push((
            (order, std::cmp::Reverse(speed), sub_order, rank, usize::MAX),
            *draw,
        ));
    }
    // Handlers tied in order, speed and sub-order run in the order Showdown's shuffle gave them
    // (the tie decisions planned from the log): the order the log shows their outcomes in.
    let shown_at = |id: PokemonId| {
        cures
            .iter()
            .map(|(who, _)| who)
            .chain(yawn_sleeps.iter().map(|(who, _)| who))
            .position(|who| who_to_id(state, who) == Some(id))
            .unwrap_or(usize::MAX)
    };
    // A berry the residual fed its holder draws (a confusion's duration, Starf Berry's stat) at
    // the `Update` after the weather (Gen 7+) or at the residual's closing one — or, where the
    // berries are residual handlers of their own (Gen 2-3 `ResidualKind::Berry`), at the
    // holder's berry.
    let weather = rules
        .residual()
        .key(ResidualKind::Weather)
        .map_or(0, |(order, _)| order);
    let berry = rules.residual().key(ResidualKind::Berry);
    let active = [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().copied().flatten());
    for (index, id) in active.clone().enumerate() {
        for (who, decision, at) in berry_confusions {
            if who_to_id(state, who) != Some(id) {
                continue;
            }
            let speed = pokemon_battle_engine::engine::residual_speed(state, rules, id);
            let key = match (at, berry) {
                (BerryConfusionAt::AfterWeather, _) => (
                    weather,
                    std::cmp::Reverse(0),
                    u8::MAX,
                    usize::MAX,
                    usize::MAX,
                ),
                (BerryConfusionAt::ResidualEnd, Some((order, sub_order))) => (
                    order,
                    std::cmp::Reverse(speed),
                    sub_order,
                    shown_at(id),
                    index,
                ),
                // At the residual's closing `Update`: after every handler, Moody's too
                // (`residual_end_berry_tape`).
                (BerryConfusionAt::ResidualEnd, None) | (BerryConfusionAt::Entry, _) => continue,
            };
            decisions.push((key, *decision));
        }
    }
    // Where in the residual a Shed Skin or a Healer cured a Pokémon (the log): a status roll for it
    // that comes later never comes — the status is gone (Showdown's Healer asks
    // `allyActive.status`, Shed Skin `pokemon.status`; R-M9-48).
    let actives: Vec<PokemonId> = active.clone().collect();
    let key_of = |id: PokemonId, kind: ResidualKind| -> Option<ResidualScheduleKey> {
        let (order, sub_order) = rules.residual().key(kind)?;
        let index = actives.iter().position(|active| *active == id)?;
        let speed = pokemon_battle_engine::engine::residual_speed(state, rules, id);
        Some((
            order,
            std::cmp::Reverse(speed),
            sub_order,
            shown_at(id),
            index,
        ))
    };
    let healer_of = |id: PokemonId| {
        state.pokemon(id).is_some_and(|pokemon| {
            pokemon_battle_engine::engine::ability_handlers(state, rules, pokemon)
                .residual
                .is_some_and(|participant| participant.kind == ResidualKind::Healer)
        })
    };
    let cured_at = |target: PokemonId| -> Option<ResidualScheduleKey> {
        cures
            .iter()
            .filter(|(who, _)| who_to_id(state, who) == Some(target))
            .filter_map(|(_, kind)| match kind {
                ResidualKind::ShedSkin => key_of(target, ResidualKind::ShedSkin),
                ResidualKind::Healer => {
                    let position = state.active_position(target, rules.topology())?;
                    rules
                        .topology()
                        .adjacent_allies(position)
                        .into_iter()
                        .filter_map(|p| state.pokemon_at_position(p, rules.topology()))
                        .filter(|ally| healer_of(*ally))
                        .filter_map(|ally| key_of(ally, ResidualKind::Healer))
                        .min()
                }
                _ => None,
            })
            .min()
    };
    let cured_before = |target: PokemonId, key: Option<ResidualScheduleKey>| matches!((cured_at(target), key), (Some(at), Some(key)) if at < key);
    // Where in the residual a Pokémon fell (the log): a roll of a handler of its own, or one
    // aimed at it, that comes later never comes — Showdown's `fieldEvent` skips a fainted
    // holder's handlers, and Healer's `adjacentAllies()` leaves a fainted ally out (R-M9-51).
    let fell_before = |target: PokemonId, key: Option<ResidualScheduleKey>| {
        let fell_at = faints
            .iter()
            .filter(|(who, _, _)| who_to_id(state, who) == Some(target))
            .filter_map(|(_, kind, holder)| key_of(who_to_id(state, holder)?, *kind))
            .min();
        matches!((fell_at, key), (Some(at), Some(key)) if at < key)
    };
    for (index, id) in active.enumerate() {
        let Some(pokemon) = state.pokemon(id) else {
            continue;
        };
        if pokemon.is_fainted() {
            continue;
        }
        let speed = pokemon_battle_engine::engine::residual_speed(state, rules, id);
        let cured = |kind: ResidualKind| {
            cures
                .iter()
                .any(|(who, k)| *k == kind && who_to_id(state, who) == Some(id))
        };
        let mut push = |kind: ResidualKind, request: RandomRequestKind| {
            if let Some((order, sub_order)) = rules.residual().key(kind) {
                decisions.push((
                    (
                        order,
                        std::cmp::Reverse(speed),
                        sub_order,
                        shown_at(id),
                        index,
                    ),
                    RandomOverride::bool(request, cured(kind)),
                ));
            }
        };
        let sheds = pokemon_battle_engine::engine::ability_handlers(state, rules, pokemon)
            .residual
            .is_some_and(|participant| participant.kind == ResidualKind::ShedSkin);
        if sheds
            && pokemon.status.is_some()
            && !cured_before(id, key_of(id, ResidualKind::ShedSkin))
            && !fell_before(id, key_of(id, ResidualKind::ShedSkin))
        {
            push(ResidualKind::ShedSkin, RandomRequestKind::ShedSkin);
        }
        if pokemon.has_status(NonVolatileStatus::Freeze)
            && !fell_before(id, key_of(id, ResidualKind::FreezeThaw))
        {
            push(ResidualKind::FreezeThaw, RandomRequestKind::Thaw);
        }
        // Harvest rolls at every turn's end out of the sun, with a berry to bring back or not:
        // it succeeded when the berry came back. Optional: in the sun it rolls nothing, and a
        // sun that runs out in this residual leaves it rolling after all.
        let harvests = pokemon_battle_engine::engine::ability_handlers(state, rules, pokemon)
            .residual
            .is_some_and(|participant| participant.kind == ResidualKind::Harvest);
        if harvests
            && !fell_before(id, key_of(id, ResidualKind::Harvest))
            && let Some((order, sub_order)) = rules.residual().key(ResidualKind::Harvest)
        {
            decisions.push((
                (
                    order,
                    std::cmp::Reverse(speed),
                    sub_order,
                    shown_at(id),
                    index,
                ),
                RandomOverride::bool(RandomRequestKind::Harvest, cured(ResidualKind::Harvest))
                    .optional(),
            ));
        }
        // Healer: one roll per adjacent ally with a status, in adjacency order.
        let healer = pokemon_battle_engine::engine::ability_handlers(state, rules, pokemon)
            .residual
            .is_some_and(|participant| participant.kind == ResidualKind::Healer);
        if healer
            && !fell_before(id, key_of(id, ResidualKind::Healer))
            && let Some((order, sub_order)) = rules.residual().key(ResidualKind::Healer)
            && let Some(position) = state.active_position(id, rules.topology())
        {
            for ally in rules
                .topology()
                .adjacent_allies(position)
                .into_iter()
                .filter_map(|p| state.pokemon_at_position(p, rules.topology()))
                .filter_map(|ally| state.pokemon(ally).map(|p| (ally, p)))
                .filter(|(_, p)| !p.is_fainted() && p.status.is_some())
                .map(|(ally, _)| ally)
                .filter(|ally| !cured_before(*ally, key_of(id, ResidualKind::Healer)))
                .filter(|ally| !fell_before(*ally, key_of(id, ResidualKind::Healer)))
            {
                let cured = cures.iter().any(|(who, k)| {
                    *k == ResidualKind::Healer && who_to_id(state, who) == Some(ally)
                });
                decisions.push((
                    (
                        order,
                        std::cmp::Reverse(speed),
                        sub_order,
                        shown_at(id),
                        index,
                    ),
                    RandomOverride::bool(RandomRequestKind::Healer, cured),
                ));
            }
        }
        // A Yawn that ran out and put the holder to sleep draws the sleep's duration.
        if pokemon.volatiles.contains(VolatileConditionKind::Yawn)
            && let Some((_, time)) = yawn_sleeps
                .iter()
                .find(|(who, _)| who_to_id(state, who) == Some(id))
            && let Some((order, sub_order)) = rules.residual().key(ResidualKind::Yawn)
        {
            decisions.push((
                (
                    order,
                    std::cmp::Reverse(speed),
                    sub_order,
                    shown_at(id),
                    index,
                ),
                RandomOverride::range(RandomRequestKind::Duration, *time),
            ));
        }
    }
    // A lock (Thrash and its kind, §15.12) that ended in the residual with its fatigue draws the
    // confusion's duration at the lock's place: Gen 3-4 the cartridge's (`ResidualKind::Rampage`),
    // Gen 5+ a duration with no order, at the holder's speed.
    for (index, id) in [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().copied().flatten())
        .enumerate()
    {
        let Some((_, time)) = fatigues
            .iter()
            .find(|(who, _)| who_to_id(state, who) == Some(id))
        else {
            continue;
        };
        let speed = pokemon_battle_engine::engine::residual_speed(state, rules, id);
        let (order, sub_order) = rules.residual().key(ResidualKind::Rampage).unwrap_or((
            u8::MAX,
            pokemon_battle_engine::rules::UNORDERED_VOLATILE_SUB_ORDER,
        ));
        // Two locks ending in one residual at a speed tie: in the order the log shows their
        // fatigue (R-M8-79).
        decisions.push((
            (
                order,
                std::cmp::Reverse(speed),
                sub_order,
                shown_at(id),
                index,
            ),
            RandomOverride::range(RandomRequestKind::Duration, *time),
        ));
    }
    // The foreseen attacks' strikes (§15.22), at their place in the residual and the target's
    // speed: the hit steps' accuracy, then (Gen 5+) the critical hit and the damage roll, or
    // (Gen 2-3 on the cartridge) the random factor of the damage computed at use.
    if let Some((order, sub_order)) = rules.residual().key(ResidualKind::FutureAttack) {
        for (index, strike) in strikes.iter().enumerate() {
            let Some(target) = who_to_id(state, &strike.target) else {
                continue;
            };
            let Some(position) = state.active_position(target, rules.topology()) else {
                continue;
            };
            let Some(spec) = rules.topology().spec(position) else {
                continue;
            };
            let Some(attack) = state
                .side(spec.side)
                .future_attacks
                .iter()
                .find(|attack| attack.slot == usize::from(spec.slot))
            else {
                continue;
            };
            let Some(strike_def) = pokemon_battle_engine::engine::future_strike(rules, dex, attack)
            else {
                continue;
            };
            let source = attack.source;
            // Gen 3 on the cartridge (issues.md I111): by battler, not by speed.
            let speed = if rules.conditions().future_attack.battler_order {
                u32::MAX - (u32::from(spec.slot) * 2 + u32::from(spec.side == SideId::P2))
            } else {
                pokemon_battle_engine::engine::residual_speed(state, rules, target)
            };
            let key = (
                order,
                std::cmp::Reverse(speed),
                sub_order,
                usize::MAX,
                index,
            );
            let roll = rolls
                .get(index)
                .copied()
                .flatten()
                .unwrap_or(rules.damage().random_max);
            // Gen 2 on the cartridge: the random factor first, hit, miss or dodge (none for 1).
            if rules.conditions().future_attack.damage_at_use
                == Some(pokemon_battle_engine::rules::ForeseenRoll::BeforeAccuracy)
                && attack.damage.is_some_and(|damage| damage > 1)
            {
                decisions.push((
                    key,
                    RandomOverride::range(RandomRequestKind::DamageRoll, roll),
                ));
            }
            if strike.immune
                || pokemon_battle_engine::engine::two_turn::dodged_by(
                    state,
                    rules,
                    source,
                    target,
                    &strike_def,
                )
                .is_some()
            {
                continue;
            }
            // Optional when it hit: the check is the strike's own, on the field the earlier
            // strikes left (a user they knocked out has lost its Lock-On, DESIGN.md R277).
            let accuracy = RandomOverride::bool(RandomRequestKind::Accuracy, !strike.missed);
            decisions.push((
                key,
                if strike.missed {
                    accuracy
                } else {
                    accuracy.optional()
                },
            ));
            if strike.missed {
                continue;
            }
            if strike_def.fixed_damage.is_none()
                && matches!(
                    pokemon_battle_engine::engine::critical_roll(
                        state,
                        rules,
                        dex,
                        source,
                        target,
                        &strike_def
                    ),
                    pokemon_battle_engine::engine::CriticalRoll::Chance(..)
                )
            {
                decisions.push((
                    key,
                    RandomOverride::bool(RandomRequestKind::Critical, strike.critical),
                ));
            }
            match strike_def.fixed_damage {
                None => decisions.push((
                    key,
                    RandomOverride::range(RandomRequestKind::DamageRoll, roll),
                )),
                Some(pokemon_battle_engine::dex::FixedDamage::Foreseen(_)) => decisions.push((
                    key,
                    RandomOverride::range(RandomRequestKind::DamageRoll, roll).optional(),
                )),
                Some(_) => {}
            }
            // The target's Focus Band, as any hit's (DESIGN.md R277): Gen 3-4 on every hit,
            // later only on one that would knock it out — optional, as the damage search tries
            // rolls that would or would not.
            if pokemon_battle_engine::engine::held_item_id(state, rules, target)
                == Some(pokemon_battle_engine::ids::ItemId::FOCUS_BAND)
            {
                let decision =
                    RandomOverride::bool(RandomRequestKind::FocusBand, strike.focus_band_held);
                decisions.push((
                    key,
                    if matches!(rules.generation(), Generation::Gen3 | Generation::Gen4) {
                        decision
                    } else {
                        decision.optional()
                    },
                ));
            }
            // The user's Toxic Chain (`onSourceDamagingHit`, any move but contact-only Poison
            // Touch): it rolls on a strike that reached the target's HP, as the engine's hit
            // reactions have it (R-M11-33).
            if strike.hp_after.is_some()
                && pokemon_battle_engine::engine::dealt_hit_status(
                    state,
                    rules,
                    pokemon_battle_engine::engine::chance_reactors(state, rules, source, target),
                    target,
                    &strike_def,
                )
                .is_some()
            {
                decisions.push((
                    key,
                    RandomOverride::bool(RandomRequestKind::HitReaction, strike.dealt_status),
                ));
            }
            // A Starf Berry the strike's `Update` fed the target: its stat.
            if let Some(draw @ BerryDraw::Stat(_)) = strike.berry_draw
                && let Some(decision) = berry_decision(draw, || None)
            {
                decisions.push((key, decision));
            }
        }
    }
    decisions.sort_by_key(|(key, _)| *key);
    decisions
        .into_iter()
        .map(|(_, decision)| decision)
        .collect()
}

/// A replacement answer in Showdown choice syntax (`switch N`); a side that was not asked
/// sends nothing.
/// The slot of the Pokémon whose Revival Blessing waits for an answer (its side's
/// `reviving`), in its side's slot order.
pub(super) fn reviving_slot(compiled: &Compiled, user: PokemonId) -> Option<usize> {
    let position = compiled
        .engine
        .state()
        .active_position(user, compiled.rules.topology())?;
    compiled
        .rules
        .topology()
        .spec(position)
        .map(|spec| usize::from(spec.slot))
}

/// The Pokémon a side's entry names for its Revival Blessing (`switch N`): the part for the
/// reviving user's slot — Showdown's request gives one part per slot, `pass` for one it does
/// not ask (R-M8-171) — else the first `switch`.
pub(super) fn revive_answer(
    compiled: &Compiled,
    side: SideId,
    text: Option<&str>,
) -> Option<String> {
    let parts: Vec<&str> = text?.split(',').map(str::trim).collect();
    let slot = compiled
        .engine
        .state()
        .side(side)
        .reviving
        .and_then(|user| reviving_slot(compiled, user));
    slot.and_then(|slot| parts.get(slot))
        .and_then(|part| part.strip_prefix("switch "))
        .or_else(|| parts.iter().find_map(|part| part.strip_prefix("switch ")))
        .map(|named| named.trim().to_string())
}

/// Answers a side's Revival Blessing with the Pokémon `named` (`switch N`'s target): its party
/// index names it, no swap follows. A revived Pokémon that goes back into its slot at once (Gen
/// 8+'s re-sort) runs its switch-in there: a Trace holder draws its foe (probed: who came in;
/// DESIGN.md R263).
pub(super) fn answer_revive(
    compiled: &mut Compiled,
    side: SideId,
    named: &str,
    oracle_frame: &Value,
) -> Result<pokemon_battle_engine::engine::TurnResult, pokemon_battle_engine::engine::BattleError> {
    let n = switch_index(compiled, side, named.trim());
    let pokemon = compiled.party[side.index()][n - 1];
    if traces(oracle_frame) {
        flush_trace(compiled);
        let snapshot = compiled.engine.snapshot();
        let trace_len = compiled.engine.trace().events().len();
        let mut probe = restore_with_tape(compiled, &snapshot, Vec::new());
        if probe.resolve_revive(ReviveChoice { side, pokemon }).is_ok() {
            let came_in = entrants(&probe.trace().events()[trace_len..]);
            let tape = trace_tape(
                compiled,
                compiled.engine.state(),
                probe.state(),
                &came_in,
                oracle_frame,
            );
            if !tape.is_empty() {
                compiled.engine = restore_with_tape(compiled, &snapshot, tape);
            }
        }
    }
    compiled
        .engine
        .resolve_revive(ReviveChoice { side, pokemon })
}

pub(super) fn parse_replacement(
    compiled: &mut Compiled,
    requirements: &[ReplacementRequirement],
    side: SideId,
    text: Option<&str>,
) -> ForcedSwitchChoice {
    // A Pokémon ordered out mid-turn (an Eject Pack's) whose own switch is still queued: the
    // replacement's `switchIn` cancels that switch (`queue.cancelAction(oldActive)`), so its swap
    // never happens. Undo it before `switch N` counts in Showdown's party (DESIGN.md R-M9-23).
    if let Some(requirement) = requirements.iter().find(|r| r.side == side) {
        for slot in &requirement.positions {
            let slot_index = compiled
                .rules
                .topology()
                .spec(slot.position)
                .map_or(0, |spec| usize::from(spec.slot));
            let leaving_alive = compiled
                .engine
                .state()
                .side(side)
                .active
                .get(slot_index)
                .copied()
                .flatten()
                .and_then(|id| compiled.engine.state().pokemon(id))
                .is_some_and(|pokemon| !pokemon.is_fainted());
            if !leaving_alive {
                continue;
            }
            while let Some(at) = compiled
                .pending_swaps
                .iter()
                .rposition(|&(s, a, _, _)| s == side.index() && a == slot_index)
            {
                let (_, a, b, _) = compiled.pending_swaps.remove(at);
                compiled.party[side.index()].swap(a, b);
            }
        }
    }
    // `switch N[, switch M]`: one entry per slot that has to be filled, in slot order; `pass`
    // entries (Showdown's placeholder for slots that are not asked) are skipped.
    let switches: Vec<usize> = text
        .map(|t| {
            t.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty() && *part != "pass")
                .map(|part| {
                    let target = part
                        .strip_prefix("switch ")
                        .unwrap_or_else(|| panic!("unsupported replacement syntax: {part}"));
                    switch_index(compiled, side, target.trim())
                })
                .collect()
        })
        .unwrap_or_default();
    let mut replacements = Vec::new();
    if !switches.is_empty() {
        let requirement = requirements
            .iter()
            .find(|requirement| requirement.side == side)
            .expect("replacement requirement for the answering side");
        for (slot, n) in requirement.positions.iter().zip(switches) {
            let position = slot.position;
            let slot_index = compiled
                .rules
                .topology()
                .spec(position)
                .map_or(0, |spec| usize::from(spec.slot));
            replacements.push(ForcedReplacement {
                position,
                pokemon: take_switch_target(compiled, side, slot_index, n),
            });
        }
    }
    ForcedSwitchChoice { side, replacements }
}

/// The value a move drew as it started, as the log shows it: Magnitude's announced magnitude;
/// Present's heal (0) when its target was healed or refused the heal (`-fail|target|heal`, Gen
/// 2's bare `-fail|target`; a Heal Block's refusal fails the move, `-fail|user`: no damage band
/// fails so) or, behind a Substitute before Gen 6, when nothing struck the doll; else the first
/// 40-power value — its 80 and 120 bands (`present_bands`) are tried when that one does not
/// fit.
pub(super) fn drawn_value(
    random: pokemon_battle_engine::effects::moves::RandomPower,
    oracle: &ActionOracle,
    behind_substitute: bool,
) -> u16 {
    use pokemon_battle_engine::effects::moves::RandomPower;
    match random {
        RandomPower::Magnitude => oracle.magnitude.unwrap_or(random.table().0[0]),
        RandomPower::Present => {
            // Before Gen 6 a heal fails against a Substitute (Gen 3-5 `-fail|user`, Gen 2 with
            // no word): a doll nothing struck means the heal was drawn.
            let healed = oracle
                .applied
                .iter()
                .any(|(_, kind)| matches!(kind.as_str(), "heal" | "fail:heal" | "fail"))
                || oracle.failed
                || (behind_substitute
                    && oracle.substitute_struck.is_empty()
                    && oracle.missed.is_empty());
            if healed { 0 } else { 2 }
        }
    }
}

/// Whether the move's first target is behind a Substitute as the action starts.
pub(super) fn behind_substitute(
    state: &BattleState,
    fates: &[(Option<PokemonId>, TargetFate)],
) -> bool {
    fates
        .first()
        .and_then(|(target, _)| *target)
        .is_some_and(|target| substitute_hp(state, target).is_some())
}

/// Present's other power bands (80, 120), for a plan whose 40-power draw fits no damage roll:
/// the template with its draw replaced. The draw is the last `Sample` before the hit's damage
/// roll: a move that called Present (Sleep Talk, Metronome) drew its pick before it, which may be
/// a 2 as well (R-M8-166), and what the hit sets off draws after it — a Starf Berry's stat
/// (R-M8-173).
pub(super) fn present_bands(
    template: &[Option<RandomOverride>],
    move_def: Option<&ResolvedMove>,
) -> Vec<Vec<Option<RandomOverride>>> {
    let present = move_def.is_some_and(|def| {
        def.behavior.random_power
            == Some(pokemon_battle_engine::effects::moves::RandomPower::Present)
    });
    let Some(damage_roll) = template
        .iter()
        .position(|entry| entry.is_none() || is_open_optional_roll(entry))
    else {
        return Vec::new();
    };
    let Some(index) = template[..damage_roll].iter().rposition(|entry| {
        entry.is_some_and(|decision| decision.kind == RandomRequestKind::Sample)
    }) else {
        return Vec::new();
    };
    if !present
        || template[index].is_none_or(|decision| {
            decision.value != pokemon_battle_engine::state::RandomOverrideValue::Range(2)
        })
    {
        return Vec::new();
    }
    [6, 9]
        .into_iter()
        .map(|value| {
            let mut variant = template.to_vec();
            variant[index] = Some(RandomOverride::range(RandomRequestKind::Sample, value));
            variant
        })
        .collect()
}

/// The Psywave roll that deals what Showdown's log shows: the HP the target lost (nothing: a
/// Gen 1 roll of 0), read back through the generation's formula; a knock-out takes the smallest
/// roll that reaches it.
pub(super) fn psywave_roll(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    target: PokemonId,
    dealt: u16,
    after: Option<u16>,
    substitutes: &[(PokemonId, Option<u16>)],
) -> u16 {
    let level = state.pokemon(actor).map_or(1, |p| p.level);
    let (min, max) = pokemon_battle_engine::engine::psywave_range(rules, level);
    let damage = |roll: u16| pokemon_battle_engine::engine::psywave_damage(rules, level, roll);
    // A hit on the target's Substitute commits no HP: the boundary frame's Substitute settles
    // it — broken, the smallest roll that breaks it; left standing, the roll it lost.
    if let Some(doll) = substitute_hp(state, target)
        && let Some((_, after)) = substitutes.iter().find(|(id, _)| *id == target)
    {
        return (min..=max)
            .find(|roll| match after {
                None => damage(*roll) >= doll,
                Some(left) => damage(*roll) == doll.saturating_sub(*left),
            })
            .unwrap_or(max);
    }
    // Each hit's own roll, from that hit's HP (a Parental Bond second strike rolls again).
    let Some(after) = after else {
        return min;
    };
    let before = state.pokemon(target).map_or(0, |p| p.hp);
    (min..=max)
        .find(|roll| damage(*roll) == dealt || (after == 0 && damage(*roll) >= before))
        .unwrap_or(max)
}

// ---------------------------------------------------------------------------------------------
// Random stat picks (R166): Acupressure's and Moody's `sample`, read from the stats they named
// ---------------------------------------------------------------------------------------------

/// The stat a `boost:` / `unboost:` label names.
pub(super) fn labelled_stat(label: &str) -> Option<pokemon_battle_engine::state::Stat> {
    pokemon_battle_engine::engine::STAGED_STATS
        .into_iter()
        .find(|stat| stat_changed(label, *stat))
}

/// Acupressure's `sample` over the target's stats below +6 (drawn only with a choice): the
/// index of the stat its boost named.
pub(super) fn acupressure_tape(
    state: &BattleState,
    victim: PokemonId,
    oracle: &ActionOracle,
) -> Option<Option<RandomOverride>> {
    let pokemon = state.pokemon(victim)?;
    let candidates: Vec<_> = pokemon_battle_engine::engine::STAGED_STATS
        .into_iter()
        .filter(|stat| pokemon.stages.get(*stat) < 6)
        .collect();
    if candidates.len() < 2 {
        return None;
    }
    let picked = oracle.applied.iter().find_map(|(who, kind)| {
        (who_to_id(state, who) == Some(victim))
            .then(|| labelled_stat(kind))
            .flatten()
    })?;
    let index = candidates.iter().position(|stat| *stat == picked)?;
    Some(Some(RandomOverride::range(
        RandomRequestKind::Sample,
        index as u16,
    )))
}

/// Conversion's (Gen 2-5) or Conversion 2's `sample` over `engine::conversion_candidates` /
/// `conversion2_candidates` (drawn only with a choice): the place of the type the user's
/// `-start|user|typechange|T` names — the first slot with it, which draws the same type.
pub(super) fn conversion_tape(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    user: PokemonId,
    target: Option<PokemonId>,
    move_id: MoveId,
    oracle: &ActionOracle,
) -> Option<Option<RandomOverride>> {
    let candidates = if move_id == MoveId::CONVERSION {
        pokemon_battle_engine::engine::conversion_candidates(state, rules, dex, user)
    } else {
        pokemon_battle_engine::engine::conversion2_candidates(state, rules, dex, user, target?)?
    };
    if candidates.len() < 2 {
        return None;
    }
    let shown = oracle
        .typechanges
        .iter()
        .find(|(who, _)| who_to_id(state, who) == Some(user))
        .and_then(|(_, name)| pokemon_battle_engine::dex::PokemonType::from_name(name));
    let index = shown
        .and_then(|type_| candidates.iter().position(|c| *c == type_))
        .unwrap_or(0);
    Some(Some(RandomOverride::range(
        RandomRequestKind::Sample,
        index as u16,
    )))
}

/// Moody and Pickup in the end-of-turn residual (both order 28.2, after every other residual
/// that draws), in the log's order. Moody: each holder that showed `-ability|p|Moody|boost`,
/// fastest first, draws the stat it raised among those below +6, then the one it lowered among
/// those above -6 but that one — read from the two stat changes that follow, in order
/// (Contrary turns them around, the stats stay). Pickup: a holder that showed `-item|p|X|[from]
/// ability: Pickup` drew, among the adjacent Pokémon that used an item up this turn (in
/// `getAllActive` order), the first whose last item is X — when there were several; the one it
/// took no longer counts for a later Pickup.
/// Moody's and Pickup's draws in the log's order, each with its holder and its residual handler:
/// `residual_tape` puts them among the other residual decisions by the handlers' order and
/// speed (a faster Moody holder draws before a slower Harvest holder's roll, DESIGN.md R-M8-92).
pub(super) fn moody_tape(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &pokemon_battle_engine::dex::ResolvedDex,
    oracle_frame: &Value,
) -> Vec<(PokemonId, ResidualKind, RandomOverride)> {
    let events = oracle_frame["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // Each holder's stat changes in the log's order, with whether each was a rise (`boost`);
    // a Pickup holder's with the item it picked up instead. Moody's candidate lists are built
    // from the holder's stages **as its handler runs** (§20.2), so the walk below keeps the
    // residual's own `boost`/`unboost` events applied on top of the frame's state.
    type MoodyPick = (
        PokemonId,
        Vec<(bool, pokemon_battle_engine::state::Stat)>,
        pokemon_battle_engine::state::StatStages,
    );
    let mut live: Vec<(PokemonId, pokemon_battle_engine::state::StatStages)> = state
        .all_pokemon()
        .iter()
        .map(|pokemon| (pokemon.id(), pokemon.stages))
        .collect();
    let mut picks: Vec<MoodyPick> = Vec::new();
    let mut pickups: Vec<(usize, PokemonId, String)> = Vec::new();
    let mut in_residual = false;
    let mut reading: Option<usize> = None;
    for event in &events {
        if event["kind"] == "residual" {
            in_residual = true;
            continue;
        }
        if !in_residual {
            continue;
        }
        // The residual's own stat changes, in the order they happened, for every Pokémon (not
        // only the one being read): Moody's filters see them.
        let kind = event["kind"].as_str().unwrap_or("");
        if (kind == "boost" || kind == "unboost")
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
            && let Some(id) = who_to_id(state, &who)
            && let Some(stat) =
                labelled_stat(&format!("{kind}:{}", event["stat"].as_str().unwrap_or("")))
            && let Some(amount) = event["amount"].as_i64()
            && let Some(entry) = live.iter_mut().find(|(who, _)| *who == id)
        {
            let delta = amount as i8 * if kind == "boost" { 1 } else { -1 };
            entry.1.set(stat, (entry.1.get(stat) + delta).clamp(-6, 6));
        }
        if event["kind"] == "item"
            && let Some(args) = event["args"].as_array()
            && args.get(2).and_then(Value::as_str) == Some("[from] ability: Pickup")
            && let (Some(id), Some(item)) = (
                args.first()
                    .and_then(who_from_text)
                    .and_then(|who| who_to_id(state, &who)),
                args.get(1).and_then(Value::as_str),
            )
        {
            pickups.push((picks.len(), id, item.to_string()));
            reading = None;
            continue;
        }
        if event["kind"] == "ability"
            && let Some(args) = event["args"].as_array()
            && args.get(1).and_then(Value::as_str) == Some("Moody")
            && let Some(id) = args
                .first()
                .and_then(who_from_text)
                .and_then(|who| who_to_id(state, &who))
        {
            let stages = live
                .iter()
                .find(|(who, _)| *who == id)
                .map(|(_, stages)| *stages)
                .unwrap_or_default();
            picks.push((id, Vec::new(), stages));
            reading = Some(picks.len() - 1);
            continue;
        }
        let Some(current) = reading else {
            continue;
        };
        let kind = event["kind"].as_str().unwrap_or("");
        if (kind == "boost" || kind == "unboost")
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
            && who_to_id(state, &who) == Some(picks[current].0)
            && let Some(stat) =
                labelled_stat(&format!("{kind}:{}", event["stat"].as_str().unwrap_or("")))
        {
            picks[current].1.push((kind == "boost", stat));
        } else {
            reading = None;
        }
    }
    // Pickup's draws, each placed before the Moody pick that followed it in the log.
    let mut taken: Vec<PokemonId> = Vec::new();
    let mut pickup_draws: Vec<(usize, PokemonId, RandomOverride)> = Vec::new();
    for (before_pick, holder, item) in pickups {
        let Some(position) = state.active_position(holder, rules.topology()) else {
            continue;
        };
        let topology = rules.topology();
        let adjacent: Vec<PokemonId> = topology
            .adjacent_allies(position)
            .into_iter()
            .chain(topology.adjacent_foes(position))
            .filter_map(|position| state.pokemon_at_position(position, topology))
            .collect();
        let candidates: Vec<PokemonId> = [SideId::P1, SideId::P2]
            .into_iter()
            .flat_map(|side| state.side(side).active.iter().copied().flatten())
            .filter(|id| {
                adjacent.contains(id)
                    && !taken.contains(id)
                    && state.pokemon(*id).is_some_and(|p| {
                        !p.is_fainted()
                            && p.history.last_item.is_some()
                            && p.history.used_item_this_turn
                    })
            })
            .collect();
        let Some(index) = candidates.iter().position(|id| {
            state
                .pokemon(*id)
                .and_then(|p| p.history.last_item)
                .and_then(|last| dex.item_by_id(last))
                .is_some_and(|last| last.name == item)
        }) else {
            continue;
        };
        taken.push(candidates[index]);
        if candidates.len() > 1 {
            pickup_draws.push((
                before_pick,
                holder,
                RandomOverride::range(RandomRequestKind::Sample, index as u16),
            ));
        }
    }
    let accuracy_evasion = rules.conditions().moody_draws_accuracy_evasion;
    let stats = pokemon_battle_engine::effects::abilities::moody_stats(accuracy_evasion);
    // The picks in the log's order: the order the holders' residuals ran in (by their speed at
    // the time, ties already planned by `residual_tie_tape`).
    let mut tape = Vec::new();
    let pick_count = picks.len();
    for (pick, (id, named, stages)) in picks.into_iter().enumerate() {
        tape.extend(
            pickup_draws
                .iter()
                .filter(|(before_pick, _, _)| *before_pick == pick)
                .map(|(_, holder, draw)| (*holder, ResidualKind::Pickup, *draw)),
        );
        let Some(pokemon) = state.pokemon(id) else {
            continue;
        };
        let _ = stats;
        let raisable = pokemon_battle_engine::effects::abilities::raisable(
            |stat| stages.get(stat),
            accuracy_evasion,
        );
        // The rise and the drop, told apart by their direction (Contrary turns both around):
        // with nothing left to raise, the only change shown is the drop.
        let contrary = pokemon_battle_engine::engine::ability_active(state, rules, pokemon)
            == Some(pokemon_battle_engine::ids::AbilityId::CONTRARY);
        let raised = named
            .iter()
            .find(|(up, _)| *up != contrary)
            .map(|(_, stat)| *stat);
        let lowered = named
            .iter()
            .find(|(up, _)| *up == contrary)
            .map(|(_, stat)| *stat);
        if raisable.len() > 1
            && let Some(index) = raised.and_then(|r| raisable.iter().position(|s| *s == r))
        {
            tape.push((
                id,
                ResidualKind::Moody,
                RandomOverride::range(RandomRequestKind::Sample, index as u16),
            ));
        }
        let lowerable = pokemon_battle_engine::effects::abilities::lowerable(
            |stat| stages.get(stat),
            accuracy_evasion,
            raised,
        );
        if lowerable.len() > 1
            && let Some(index) = lowered.and_then(|l| lowerable.iter().position(|s| *s == l))
        {
            tape.push((
                id,
                ResidualKind::Moody,
                RandomOverride::range(RandomRequestKind::Sample, index as u16),
            ));
        }
    }
    // The Pickups after the last Moody pick.
    tape.extend(
        pickup_draws
            .iter()
            .filter(|(before_pick, _, _)| *before_pick == pick_count)
            .map(|(_, holder, draw)| (*holder, ResidualKind::Pickup, *draw)),
    );
    tape
}
