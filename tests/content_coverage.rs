//! Content coverage (ROADMAP §11.4, DESIGN.md §25.1): every behaviour the engine claims must be
//! exercised against Showdown. The behavior registry says what the engine implements — a move with
//! data behavior or a custom primary, an ability or item with behavior components — and the pinned
//! Showdown scenarios say what has been played: a move Showdown's log shows being used, an
//! ability or item some Pokémon in a scenario carries. A with_behavior entry must appear in at least
//! one scenario of a generation where it is implemented, or be listed below as known debt.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use pokemon_battle_engine::dex::{DexResolver, PrimaryEffect};
use pokemon_battle_engine::effects;
use pokemon_battle_engine::effects::moves::MoveBehavior;
use pokemon_battle_engine::spec::Generation;
use serde_json::Value;

/// Entries with behavior no scenario exercises yet, as (kind, name). Adding a behavior without a
/// scenario means adding it here, in review.
const NOT_YET_EXERCISED: &[(&str, &str)] = &[];

const GENERATIONS: [Generation; 9] = [
    Generation::Gen1,
    Generation::Gen2,
    Generation::Gen3,
    Generation::Gen4,
    Generation::Gen5,
    Generation::Gen6,
    Generation::Gen7,
    Generation::Gen8,
    Generation::Gen9,
];

#[derive(Default)]
struct Exercised {
    moves: BTreeMap<u8, BTreeSet<String>>,
    abilities: BTreeMap<u8, BTreeSet<String>>,
    items: BTreeMap<u8, BTreeSet<String>>,
}

fn exercised() -> Exercised {
    let cases = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cases");
    let mut out = Exercised::default();
    let read = |path: &PathBuf| -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("json")
    };
    for entry in std::fs::read_dir(&cases).expect("cases") {
        let path = entry.expect("entry").path();
        let Some(file) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        let Some(stem) = file.strip_suffix(".showdown.json") else {
            continue;
        };
        let scenario = read(&cases.join(format!("{stem}.json")));
        let generation = scenario["battle"]["generation"]
            .as_u64()
            .expect("generation") as u8;
        for side in ["p1", "p2"] {
            for set in scenario["teams"][side].as_array().expect("team") {
                if let Some(ability) = set["ability"].as_str() {
                    out.abilities
                        .entry(generation)
                        .or_default()
                        .insert(ability.to_owned());
                }
                if let Some(item) = set["item"].as_str() {
                    out.items
                        .entry(generation)
                        .or_default()
                        .insert(item.to_owned());
                }
            }
        }
        let oracle = read(&path);
        for line in oracle["debug_log"].as_str().expect("log").lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() > 3 && parts[1] == "move" {
                out.moves
                    .entry(generation)
                    .or_default()
                    .insert(parts[3].to_owned());
            }
        }
    }
    out
}

#[test]
fn every_move_ability_and_item_with_behavior_is_exercised_against_showdown() {
    let played = exercised();
    // (kind, name) → generations where the engine implements it with behavior.
    let mut with_behavior: BTreeMap<(&str, String), Vec<u8>> = BTreeMap::new();
    for generation in GENERATIONS {
        let number = generation.number();
        let dex = DexResolver::resolve(generation);
        for resolved in dex.moves() {
            let has_behavior = resolved.behavior != MoveBehavior::default()
                || effects::moves::custom(resolved.id).is_some();
            if has_behavior && resolved.primary_effect != PrimaryEffect::Unsupported {
                with_behavior
                    .entry(("move", resolved.name.clone()))
                    .or_default()
                    .push(number);
            }
        }
        for resolved in dex.abilities() {
            let behavior = effects::abilities::definition(resolved.id);
            if resolved.implemented && !behavior.covered_callbacks().is_empty() {
                with_behavior
                    .entry(("ability", resolved.name.clone()))
                    .or_default()
                    .push(number);
            }
        }
        for resolved in dex.items() {
            let behavior = effects::items::definition(resolved.id);
            if resolved.implemented && !behavior.covered_callbacks().is_empty() {
                with_behavior
                    .entry(("item", resolved.name.clone()))
                    .or_default()
                    .push(number);
            }
        }
    }

    let played_in = |kind: &str, generation: u8| match kind {
        "move" => played.moves.get(&generation),
        "ability" => played.abilities.get(&generation),
        _ => played.items.get(&generation),
    };
    let mut uncovered = Vec::new();
    for ((kind, name), generations) in &with_behavior {
        let exercised = generations.iter().any(|generation| {
            played_in(kind, *generation).is_some_and(|names| names.contains(name))
        });
        let listed = NOT_YET_EXERCISED.contains(&(*kind, name.as_str()));
        if !exercised && !listed {
            uncovered.push(format!(
                "{kind} {name} (implemented in gen {generations:?})"
            ));
        }
        if exercised && listed {
            uncovered.push(format!(
                "{kind} {name} is exercised now: drop it from NOT_YET_EXERCISED"
            ));
        }
    }
    for (kind, name) in NOT_YET_EXERCISED {
        if !with_behavior.contains_key(&(*kind, (*name).to_owned())) {
            uncovered.push(format!(
                "{kind} {name} in NOT_YET_EXERCISED has no behavior"
            ));
        }
    }
    let count = |kind: &str| with_behavior.keys().filter(|(k, _)| *k == kind).count();
    assert!(
        count("move") > 50 && count("ability") > 30 && count("item") > 30,
        "the behavior registry looks empty"
    );
    assert!(
        uncovered.is_empty(),
        "with_behavior content no Showdown scenario exercises:\n{}",
        uncovered.join("\n")
    );
}

/// Every behavior table entry is honoured by at least one generation: a component none of whose
/// callbacks any generation declares for that move, ability or item is dead code (the
/// per-generation restriction removes it everywhere).
#[test]
fn every_behavior_table_entry_is_honoured_by_some_generation() {
    use pokemon_battle_engine::data;
    use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveId};

    // (kind, Showdown id) → (has behavior, honoured somewhere).
    let mut entries: BTreeMap<(&str, String), (bool, bool)> = BTreeMap::new();
    for generation in GENERATIONS {
        let snapshot = data::snapshot(generation);
        for row in &snapshot.moves {
            let behavior = effects::moves::definition(MoveId(row.eid));
            let entry = entries.entry(("move", row.id.clone())).or_default();
            entry.0 |= behavior != MoveBehavior::default();
            entry.1 |= behavior.restricted_to(&row.declared_callbacks()) != MoveBehavior::default();
        }
        for row in &snapshot.abilities {
            let behavior = effects::abilities::definition(AbilityId(row.eid));
            let entry = entries.entry(("ability", row.id.clone())).or_default();
            entry.0 |= !behavior.covered_callbacks().is_empty();
            entry.1 |= !behavior
                .restricted_to(&row.callbacks)
                .covered_callbacks()
                .is_empty();
        }
        for row in &snapshot.items {
            let behavior = effects::items::definition(ItemId(row.eid));
            let entry = entries.entry(("item", row.id.clone())).or_default();
            entry.0 |= !behavior.covered_callbacks().is_empty();
            entry.1 |= !behavior
                .restricted_to(&row.callbacks)
                .covered_callbacks()
                .is_empty();
        }
    }
    let dead: Vec<String> = entries
        .iter()
        .filter(|(_, (with_behavior, honoured))| *with_behavior && !honoured)
        .map(|((kind, id), _)| format!("{kind} {id}"))
        .collect();
    assert!(
        dead.is_empty(),
        "behavior table entries no generation declares a callback for:\n{}",
        dead.join("\n")
    );
}

/// A custom primary claims only what some generation's snapshot declares for its move: every
/// callback and data key it names appears in at least one generation (DESIGN.md §25). Together
/// with the dex rule — anything a custom move declares that its primary does not name keeps it
/// Unsupported — the claim and the data match exactly.
#[test]
fn every_custom_primary_claim_is_declared_by_some_generation() {
    use pokemon_battle_engine::data;
    use pokemon_battle_engine::ids::MoveId;

    // Move id → (callbacks, keys) the snapshots declare across generations.
    let mut declared: BTreeMap<u16, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut names: BTreeMap<u16, String> = BTreeMap::new();
    for generation in GENERATIONS {
        for row in &data::snapshot(generation).moves {
            if effects::moves::custom(MoveId(row.eid)).is_none() {
                continue;
            }
            let entry = declared.entry(row.eid).or_default();
            // `self:<name>` for the move's `self` table (Baton Pass's `self.onHit`).
            entry.0.extend(row.declared_callbacks());
            entry.1.extend(row.other.keys().cloned());
            names.insert(row.eid, row.name.clone());
        }
    }
    assert!(declared.len() > 15, "the custom primaries look empty");
    let mut overclaimed = Vec::new();
    for (eid, (callbacks, keys)) in &declared {
        let primary = effects::moves::custom(MoveId(*eid)).expect("custom");
        for callback in primary.source_callbacks {
            if !callbacks.contains(*callback) {
                overclaimed.push(format!("{} callback {callback}", names[eid]));
            }
        }
        for key in primary.source_keys {
            if !keys.contains(*key) {
                overclaimed.push(format!("{} key {key}", names[eid]));
            }
        }
    }
    assert!(
        overclaimed.is_empty(),
        "custom primaries claim what no generation declares:\n{}",
        overclaimed.join("\n")
    );
}
