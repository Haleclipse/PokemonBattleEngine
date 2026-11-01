//! L1 rule tests for Neutralizing Gas where the frames cannot tell (DESIGN.md R203): with
//! another holder still on the field, the one leaving starts nothing again — the other gas keeps
//! everything it would start suppressed, so only the trace shows its `onEnd` returning early.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{AbilityId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const WEEZING: PokemonId = PokemonId(1);
const GALARIAN: PokemonId = PokemonId(2);
const BENCH: PokemonId = PokemonId(3);
const TYRANITAR: PokemonId = PokemonId(4);
const GYARADOS: PokemonId = PokemonId(5);

fn set(species: &str, ability: &str) -> PokemonSet {
    let mut set = PokemonSet::new(species, ["Harden"]);
    set.ability = Some(ability.into());
    set.level = 50;
    set
}

fn harden(actor: PokemonId) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    }
}

#[test]
fn the_first_of_two_gases_to_leave_starts_nothing_again() {
    let generation = Generation::Gen8;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
    let pokemon = vec![
        build(WEEZING, SideId::P1, set("Weezing", "Neutralizing Gas")),
        build(
            GALARIAN,
            SideId::P1,
            set("Weezing-Galar", "Neutralizing Gas"),
        ),
        build(BENCH, SideId::P1, set("Snorlax", "Thick Fat")),
        build(TYRANITAR, SideId::P2, set("Tyranitar", "Sand Stream")),
        build(GYARADOS, SideId::P2, set("Gyarados", "Intimidate")),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let state = BattleState::new(
        pokemon,
        [
            side(vec![WEEZING, GALARIAN, BENCH]),
            side(vec![TYRANITAR, GYARADOS]),
        ],
        1,
    );
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine.run_lead_switch_ins();
    let start = engine.trace().events().len();
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![
                    SlotChoice {
                        actor: WEEZING,
                        action: ActionChoice::Switch { pokemon: BENCH },
                    },
                    harden(GALARIAN),
                ],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![harden(TYRANITAR), harden(GYARADOS)],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let restarted: Vec<(PokemonId, AbilityId)> = engine.trace().events()[start..]
        .iter()
        .filter_map(|event| match event {
            TraceEvent::AbilityActivated { source, ability } => Some((*source, *ability)),
            _ => None,
        })
        .collect();
    // Showdown's `onEnd` returns before its `-end`: the other holder's gas is not started again
    // (no second `-ability|Neutralizing Gas`), nor anything else.
    assert_eq!(restarted, Vec::new());
    assert_eq!(engine.state().field.weather, None);
}
