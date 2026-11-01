//! L1 tests for the order of the BeforeMove checks where the replay cannot tell them apart
//! (DESIGN.md §15.1): Gen 3-4 on the cartridge (issues.md I66) refuse a taunted status move
//! before Heal Block and Gravity; from Gen 5 Heal Block and Gravity (Showdown priority 6) come
//! before Taunt (5).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BeforeMoveBlockReason, CanMoveResult, EngineStep, SideChoice, SlotChoice,
    TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleRng, BattleState, SideState};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// A (Snorlax, `Recover`, taunted and under Heal Block) against B (Chansey, `Harden`): what
/// stops A's Recover.
fn stopped_by(generation: Generation) -> Option<CanMoveResult> {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut user = build_pokemon(
        &dex,
        A,
        SideId::P1,
        &PokemonSet::new("Snorlax", ["Recover"]),
    )
    .unwrap();
    user.hp = user.max_hp / 2;
    let foe = build_pokemon(&dex, B, SideId::P2, &PokemonSet::new("Chansey", ["Harden"])).unwrap();
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![user, foe], [side(A), side(B)], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let action = |actor: PokemonId| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::FIRST,
            target: None,
            mechanic: None,
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![action(A)],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![action(B)],
            },
        ])
        .unwrap();
    // Taunted and under Heal Block once the move is chosen (a request would refuse it).
    let mut snapshot = engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(1, Vec::new());
    let user = snapshot.state.pokemon_mut(A).unwrap();
    assert!(user.volatiles.set_taunt(3));
    assert!(user.volatiles.set_heal_block(5));
    let mut engine = BattleEngine::restore(rules, dex, snapshot).unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    engine
        .trace()
        .events()
        .iter()
        .find_map(|event| match event {
            TraceEvent::BeforeMoveResolved {
                actor: A, result, ..
            } => Some(*result),
            _ => None,
        })
}

#[test]
fn gen_3_4_taunt_comes_before_heal_block_and_gen_5_after() {
    for (generation, reason) in [
        (Generation::Gen4, BeforeMoveBlockReason::Taunt),
        (Generation::Gen5, BeforeMoveBlockReason::HealBlock),
        (Generation::Gen9, BeforeMoveBlockReason::HealBlock),
    ] {
        assert_eq!(
            stopped_by(generation),
            Some(CanMoveResult::Blocked(reason)),
            "{generation:?}"
        );
    }
}
