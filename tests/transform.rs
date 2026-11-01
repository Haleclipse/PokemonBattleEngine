//! L1 tests for Transform (DESIGN.md R280): what the replay cannot see. A copied move's
//! max PP shows in no frame (Gen 5+ 5, Gen 1-4 the user's own full max PP).
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const MEW: PokemonId = PokemonId(1);
const SNORLAX: PokemonId = PokemonId(2);

/// Mew transforms into Snorlax (Body Slam, Rest): the copied slots' (pp, max PP).
fn copied_pp(generation: Generation) -> Vec<(u8, u8)> {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let set = |species: &str, moves: &[&str]| {
        let mut set = PokemonSet::new(species, moves.iter().copied());
        set.level = 50;
        set
    };
    let pokemon = vec![
        build_pokemon(&dex, MEW, SideId::P1, &set("Mew", &["Transform"])).unwrap(),
        build_pokemon(
            &dex,
            SNORLAX,
            SideId::P2,
            &set("Snorlax", &["Body Slam", "Rest"]),
        )
        .unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(MEW), side(SNORLAX)], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let choice = |side: SideId, actor: PokemonId, target: SideId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(if actor == MEW { 0 } else { 1 }).unwrap(),
                target: Some(DeclaredTarget(
                    rules.topology().position(target, 0).unwrap(),
                )),
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, MEW, SideId::P2),
            choice(SideId::P2, SNORLAX, SideId::P1),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let mew = engine.state().pokemon(MEW).unwrap();
    assert!(mew.transform.is_some());
    mew.move_slots()
        .iter()
        .map(|slot| (slot.pp, slot.max_pp))
        .collect()
}

#[test]
fn transform_gives_5_pp_and_the_generations_max_pp() {
    // Body Slam 15 PP (24 with three PP Ups), Rest 10 (16).
    assert_eq!(copied_pp(Generation::Gen9), vec![(5, 5), (5, 5)]);
    assert_eq!(copied_pp(Generation::Gen4), vec![(5, 24), (5, 16)]);
}
