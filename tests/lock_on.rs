//! L1 test for Lock-On (DESIGN.md §15.14): what the replay cannot see — a move that would have
//! hit anyway draws no accuracy at all while the lock holds (the replay plans its decisions
//! from the same `accuracy_roll`).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::accuracy_roll;
use pokemon_battle_engine::ids::{MoveId, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, VolatileConditionKind};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

#[test]
fn a_lock_skips_the_accuracy_roll_of_its_holders_moves_against_the_target() {
    for (generation, on_target) in [
        (Generation::Gen2, true),
        (Generation::Gen4, false),
        (Generation::Gen9, false),
    ] {
        let rules = Arc::new(
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .unwrap(),
        );
        let dex = Arc::new(DexResolver::resolve(generation));
        let a = build_pokemon(
            &dex,
            A,
            SideId::P1,
            &PokemonSet::new("Magneton", ["Zap Cannon"]),
        )
        .unwrap();
        let b =
            build_pokemon(&dex, B, SideId::P2, &PokemonSet::new("Chansey", ["Harden"])).unwrap();
        let side = |id: PokemonId| {
            let mut side = SideState::new(vec![id], 1);
            side.active[0] = Some(id);
            side
        };
        let state = BattleState::new(vec![a, b], [side(A), side(B)], 1);
        let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
        let zap = dex.move_by_id(MoveId::ZAP_CANNON).unwrap().clone();
        assert!(
            accuracy_roll(engine.state(), &rules, A, B, &zap).is_some(),
            "{generation:?}"
        );
        let mut snapshot = engine.snapshot();
        if on_target {
            assert!(
                snapshot
                    .state
                    .pokemon_mut(B)
                    .unwrap()
                    .volatiles
                    .set_marker(VolatileConditionKind::LockedOn)
            );
        } else {
            snapshot
                .state
                .pokemon_mut(A)
                .unwrap()
                .volatiles
                .set_lock_on(B, 2);
        }
        let engine = BattleEngine::restore(rules.clone(), dex.clone(), snapshot).unwrap();
        assert_eq!(
            accuracy_roll(engine.state(), &rules, A, B, &zap),
            None,
            "{generation:?}"
        );
        // Not the other way round.
        assert!(
            accuracy_roll(engine.state(), &rules, B, A, &zap).is_some(),
            "{generation:?}"
        );
    }
}
