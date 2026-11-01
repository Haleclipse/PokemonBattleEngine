//! L1 tests for Rage (DESIGN.md §15.19): what the replay cannot see. Gen 3's flag is the
//! accuracy check's (issues.md I77), and Showdown reports `-immune` against a Ghost whether the
//! check passed or not, so only the roll tells whether Rage leaves its user raging; Gen 2's count
//! is no frame's, and a damage of 1 it multiplies draws the random factor; the count stops at 255;
//! Gen 1's lock refuses every other choice, which no script can try.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, PokemonState, RandomOverride, RandomRequestKind, SideState,
    VolatileState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// One turn of `a` using its first move on `b`, which uses its first move too, under `tape`.
/// Returns the engine's state afterwards and whether the tape was used up.
fn turn(
    generation: Generation,
    a: PokemonState,
    b: PokemonState,
    tape: Vec<RandomOverride>,
) -> (PokemonState, bool) {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(vec![a, b], [side(A), side(B)], 1);
    state.rng = BattleRng::with_semantic_tape(1, tape);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let choice = |side: SideId, actor: PokemonId, target: SideId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: Some(DeclaredTarget(
                    rules.topology().position(target, 0).unwrap(),
                )),
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, A, SideId::P2),
            choice(SideId::P2, B, SideId::P1),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    (
        engine.state().pokemon(A).unwrap().clone(),
        engine.state().rng.semantic_tape_exhausted(),
    )
}

fn pokemon(
    generation: Generation,
    id: PokemonId,
    species: &str,
    moves: &[&str],
    level: u8,
) -> PokemonState {
    let dex = DexResolver::resolve(generation);
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = level;
    let side = if id == A { SideId::P1 } else { SideId::P2 };
    build_pokemon(&dex, id, side, &set).unwrap()
}

/// Gen 3: a Primeape (raging or not) uses Rage on a slower, evasive Gengar, behind a Substitute
/// or not, its accuracy roll `hit`. Returns whether it is raging afterwards.
fn gen3_rage_on_a_ghost(raging: bool, hit: bool, substitute: bool) -> bool {
    let generation = Generation::Gen3;
    let mut a = pokemon(generation, A, "Primeape", &["Rage"], 50);
    if raging {
        a.volatiles.start_rage(false);
    }
    let mut b = pokemon(generation, B, "Gengar", &["Harden"], 20);
    // Evasive: Rage's check is a roll (a 100 % move against no stages draws none).
    b.stages.evasion = 1;
    if substitute {
        b.volatiles.set_substitute(B, 10);
    }
    let (a, _) = turn(
        generation,
        a,
        b,
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, hit)],
    );
    a.volatiles.rage().is_some()
}

#[test]
fn gen3_rage_against_a_ghost_is_the_accuracy_checks() {
    // The check passed: raging, though the Ghost is immune …
    assert!(gen3_rage_on_a_ghost(false, true, false));
    assert!(gen3_rage_on_a_ghost(true, true, false));
    // … unless a Substitute stands in front of it (it does not end the rage either).
    assert!(!gen3_rage_on_a_ghost(false, true, true));
    assert!(gen3_rage_on_a_ghost(true, true, true));
    // Missed: the rage ends (Showdown's `-immune` all the same).
    assert!(!gen3_rage_on_a_ghost(true, false, false));
    assert!(!gen3_rage_on_a_ghost(false, false, false));
}

/// Gen 2: a level 5 Primeape with a rage count of `count` uses Rage on a Steelix — the damage
/// after the resisted type matchup is 1. Returns whether the random factor was drawn.
fn gen2_rage_draws_its_random_factor(count: u8) -> bool {
    let generation = Generation::Gen2;
    let mut a = pokemon(generation, A, "Primeape", &["Rage"], 5);
    a.volatiles.start_rage(false);
    for _ in 0..count {
        a.volatiles.count_rage();
    }
    let b = pokemon(generation, B, "Steelix", &["Harden"], 100);
    let (_, exhausted) = turn(
        generation,
        a,
        b,
        vec![
            RandomOverride::bool(RandomRequestKind::Accuracy, true).optional(),
            RandomOverride::bool(RandomRequestKind::Critical, false).optional(),
            RandomOverride::range(RandomRequestKind::DamageRoll, 255),
        ],
    );
    exhausted
}

#[test]
fn gen2_rage_multiplies_a_damage_of_1_into_a_random_factor() {
    assert!(!gen2_rage_draws_its_random_factor(0));
    assert!(gen2_rage_draws_its_random_factor(1));
}

#[test]
fn gen2_rage_count_stops_at_255() {
    let mut volatiles = VolatileState::default();
    assert!(!volatiles.count_rage());
    volatiles.start_rage(false);
    for _ in 0..300 {
        volatiles.count_rage();
    }
    assert_eq!(volatiles.rage(), Some(255));
}

/// Gen 1: a Primeape locked into Rage (`RageEnd::Locked`), with a Snorlax on its bench, faces a
/// Gengar. Returns whether the engine takes Rage, Harden, a switch.
fn gen1_locked_choices() -> (bool, bool, bool) {
    let generation = Generation::Gen1;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut a = pokemon(generation, A, "Primeape", &["Rage", "Harden"], 50);
    a.volatiles.start_rage(true);
    let bench = PokemonId(3);
    let reserve = {
        let set = PokemonSet::new("Snorlax", ["Harden"]);
        build_pokemon(&dex, bench, SideId::P1, &set).unwrap()
    };
    let b = pokemon(generation, B, "Gengar", &["Harden"], 50);
    let mut p1 = SideState::new(vec![A, bench], 1);
    p1.active[0] = Some(A);
    let mut p2 = SideState::new(vec![B], 1);
    p2.active[0] = Some(B);
    let state = BattleState::new(vec![a, reserve, b], [p1, p2], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let foe = Some(DeclaredTarget(
        rules.topology().position(SideId::P2, 0).unwrap(),
    ));
    let takes = |action: ActionChoice| {
        engine
            .validate_choice(
                SideId::P1,
                &SideChoice {
                    side: SideId::P1,
                    actions: vec![SlotChoice { actor: A, action }],
                },
            )
            .is_ok()
    };
    let slot = |n: u8| ActionChoice::Move {
        slot: MoveSlot::new(n).unwrap(),
        target: foe,
        mechanic: None,
    };
    (
        takes(slot(0)),
        takes(slot(1)),
        takes(ActionChoice::Switch { pokemon: bench }),
    )
}

#[test]
fn gen1_rage_locks_out_every_other_choice() {
    assert_eq!(gen1_locked_choices(), (true, false, false));
}

/// Gen 1: a Primeape locked into Rage at accuracy −1 uses it on a Snorlax `turns` times. Returns
/// its lock's accuracy byte afterwards.
fn gen1_rage_accuracy_after(turns: usize) -> Option<u8> {
    let generation = Generation::Gen1;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut a = pokemon(generation, A, "Primeape", &["Rage"], 50);
    a.volatiles.start_rage(true);
    a.stages.accuracy = -1;
    let b = pokemon(generation, B, "Snorlax", &["Harden"], 50);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(vec![a, b], [side(A), side(B)], 1);
    let mut tape = Vec::new();
    for _ in 0..turns {
        tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, true));
        tape.push(RandomOverride::bool(RandomRequestKind::Critical, false).optional());
        tape.push(RandomOverride::range(RandomRequestKind::DamageRoll, 255).optional());
    }
    state.rng = BattleRng::with_semantic_tape(1, tape);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let foe = Some(DeclaredTarget(
        rules.topology().position(SideId::P2, 0).unwrap(),
    ));
    for _ in 0..turns {
        let choice = |side: SideId, actor: PokemonId, target| SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(0).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        };
        engine
            .submit_turn_choices([choice(SideId::P1, A, foe), choice(SideId::P2, B, None)])
            .unwrap();
        while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    }
    engine.state().pokemon(A).unwrap().volatiles.rage_lock()
}

#[test]
fn gen1_rage_carries_its_accuracy_byte_from_use_to_use() {
    // 255 as the rage starts; each use applies the −1 stage again (× 66 / 100, floored).
    assert_eq!(gen1_rage_accuracy_after(1), Some(168));
    assert_eq!(gen1_rage_accuracy_after(2), Some(110));
}
