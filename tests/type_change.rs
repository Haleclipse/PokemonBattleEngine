//! L1 rule tests for type changes the frames cannot tell (DESIGN.md R238): the types stay the
//! same whether the move works or fails, so only the trace shows it.
//! - Gen 5 Soak on a pure Water target goes through (the gen5 `onHit` asks only `setType`);
//!   Gen 6+ fails (`TypeRules::soak_fails_if_same`).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

fn set(species: &str, ability: &str, moves: [&str; 1]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves);
    set.ability = Some(ability.into());
    set.level = 50;
    set
}

/// The trace of one turn: the user uses its one move on the target, which hardens.
fn one_turn(generation: Generation, user: PokemonSet, target: PokemonSet) -> Vec<TraceEvent> {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
    let pokemon = vec![
        build(USER, SideId::P1, user),
        build(TARGET, SideId::P2, target),
    ];
    let side = |id| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(USER), side(TARGET)], 1);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine.run_lead_switch_ins();
    let start = engine.trace().events().len();
    let first_move = |actor| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![first_move(USER)],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![first_move(TARGET)],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    engine.trace().events()[start..].to_vec()
}

#[test]
fn soak_on_pure_water_goes_through_in_gen_5_only() {
    for generation in [Generation::Gen5, Generation::Gen6, Generation::Gen9] {
        let changed = one_turn(
            generation,
            set("Slowbro", "Oblivious", ["Soak"]),
            set("Vaporeon", "Hydration", ["Harden"]),
        )
        .iter()
        .any(|event| {
            matches!(
                event,
                TraceEvent::TypesChanged {
                    pokemon: TARGET,
                    ..
                }
            )
        });
        assert_eq!(changed, generation == Generation::Gen5, "{generation:?}");
    }
}
