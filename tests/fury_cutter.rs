//! L1 test for Fury Cutter (DESIGN.md §15.18): what the replay cannot see — the Gen 2-4 count
//! lives in the oracle patch's `pokemon.m`, not in a frame, so put to sleep, the Gen 3-4 user
//! loses it (the cartridges' `CancelMultiTurnMoves` / `subscript_fall_asleep`), and Gen 2's
//! does not; Gen 5+ Showdown's volatile only runs out.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, RandomOverride, RandomRequestKind, SideState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// A Scizor whose Fury Cutter chain stands at `level` chooses Fury Cutter again; a faster
/// Jolteon's Spore puts it to sleep first. Returns the chain afterwards.
fn chain_after_sleep(generation: Generation, level: u8, turns: Option<u8>) -> Option<u8> {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut cutter = PokemonSet::new("Scizor", ["Fury Cutter", "Harden"]);
    cutter.level = 50;
    let mut sporer = PokemonSet::new("Jolteon", ["Spore"]);
    sporer.level = 100;
    let mut a = build_pokemon(&dex, A, SideId::P1, &cutter).unwrap();
    a.volatiles.set_fury_cutter(level, turns);
    let b = build_pokemon(&dex, B, SideId::P2, &sporer).unwrap();
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(vec![a, b], [side(A), side(B)], 1);
    // Spore's accuracy (where drawn), the sleep's duration.
    state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Accuracy, true).optional(),
            RandomOverride::range(RandomRequestKind::Duration, 3).optional(),
        ],
    );
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let choice =
        |side: SideId, actor: PokemonId, slot: u8, target: Option<DeclaredTarget>| SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        };
    let at = |side: SideId| Some(DeclaredTarget(rules.topology().position(side, 0).unwrap()));
    engine
        .submit_turn_choices([
            choice(SideId::P1, A, 0, at(SideId::P2)),
            choice(SideId::P2, B, 0, at(SideId::P1)),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    engine
        .state()
        .pokemon(A)
        .unwrap()
        .volatiles
        .fury_cutter()
        .map(|(level, _)| level)
}

#[test]
fn sleep_put_on_the_user_ends_the_gen3_4_count() {
    assert_eq!(chain_after_sleep(Generation::Gen2, 3, None), Some(3));
    assert_eq!(chain_after_sleep(Generation::Gen3, 3, None), None);
    assert_eq!(chain_after_sleep(Generation::Gen4, 3, None), None);
    // Gen 5+: the volatile only counts down (2 → 1 at the end of the turn).
    assert_eq!(chain_after_sleep(Generation::Gen9, 2, Some(2)), Some(2));
}
