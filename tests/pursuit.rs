//! L1 test for Pursuit (DESIGN.md §15.11): what the replay cannot step through. Two pursuers
//! strike the same switching Pokémon inside one switch action (the replay reads one action per
//! step), each in marking order, before it leaves.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EffectSource, EngineStep, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const FOE: PokemonId = PokemonId(3);
const LEAVING: PokemonId = PokemonId(4);
const BENCH: PokemonId = PokemonId(5);

fn set(species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some("Run Away".into());
    set.level = level;
    set
}

fn slot_move(actor: PokemonId, target: Option<DeclaredTarget>) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target,
            mechanic: None,
        },
    }
}

#[test]
fn two_pursuers_strike_the_same_switching_pokemon_before_it_leaves() {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen5,
            BattleMode::Doubles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen5));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &set("Tyranitar", &["Pursuit"], 40)).unwrap(),
        build_pokemon(&dex, B, SideId::P1, &set("Snorlax", &["Pursuit"], 40)).unwrap(),
        build_pokemon(&dex, FOE, SideId::P2, &set("Snorlax", &["Harden"], 50)).unwrap(),
        build_pokemon(&dex, LEAVING, SideId::P2, &set("Vaporeon", &["Harden"], 70)).unwrap(),
        build_pokemon(&dex, BENCH, SideId::P2, &set("Jolteon", &["Harden"], 50)).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 2);
        side.active = vec![Some(team[0]), Some(team[1])];
        side
    };
    let state = BattleState::new(
        pokemon,
        [side(vec![A, B]), side(vec![FOE, LEAVING, BENCH])],
        5,
    );
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let foe_position = rules.topology().position(SideId::P2, 0).unwrap();
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![
                    slot_move(A, Some(DeclaredTarget(foe_position))),
                    slot_move(B, Some(DeclaredTarget(foe_position))),
                ],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![
                    slot_move(FOE, None),
                    SlotChoice {
                        actor: LEAVING,
                        action: ActionChoice::Switch { pokemon: BENCH },
                    },
                ],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let events = engine.trace().events();
    let struck: Vec<PokemonId> = events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::DamageApplied {
                target: LEAVING,
                source: EffectSource::Move { pokemon, .. },
                ..
            } => Some(*pokemon),
            _ => None,
        })
        .collect();
    assert_eq!(
        struck,
        vec![A, B],
        "both pursuers struck the leaving Pokémon, faster first"
    );
    let switched = events
        .iter()
        .position(|event| {
            matches!(
                event,
                TraceEvent::Switched {
                    incoming: BENCH,
                    ..
                }
            )
        })
        .expect("the switch went through");
    let last_hit = events
        .iter()
        .rposition(|event| {
            matches!(
                event,
                TraceEvent::DamageApplied {
                    target: LEAVING,
                    ..
                }
            )
        })
        .unwrap();
    assert!(last_hit < switched, "the hits came before the switch");
    // Neither pursuer acted again: FOE took no Pursuit.
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, TraceEvent::DamageApplied { target: FOE, .. }))
    );
}
