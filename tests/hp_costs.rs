//! L1 rule tests for HP a move makes its own user pay (DESIGN.md R-M8-91): the price goes through
//! `apply_damage` as a `DamageKind::Cost`, which the frames cannot tell from a direct HP write.
//! - Gen 1's `lastDamage` (Counter, Bide) is not set by a Substitute's price: Showdown's gen1
//!   `directDamage` sets it for confusion and Jump Kick's crash alone (`DamageKind::Direct`).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const USER: PokemonId = PokemonId(1);
const FOE: PokemonId = PokemonId(2);

fn set(species: &str, moves: [&str; 1]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves);
    set.level = 50;
    set
}

/// One turn in which both Pokémon use their one move; the engine after it.
fn one_turn(generation: Generation, user: PokemonSet, foe: PokemonSet) -> BattleEngine {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
    let pokemon = vec![build(USER, SideId::P1, user), build(FOE, SideId::P2, foe)];
    let side = |id| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(USER), side(FOE)], 1);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine.run_lead_switch_ins();
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
                actions: vec![first_move(FOE)],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    engine
}

#[test]
fn a_gen_1_substitutes_price_is_no_last_damage() {
    let engine = one_turn(
        Generation::Gen1,
        set("Snorlax", ["Substitute"]),
        set("Slowbro", ["Splash"]),
    );
    let paid = engine.trace().events().iter().any(|event| {
        matches!(
            event,
            TraceEvent::HpCostPaid {
                target: USER,
                amount,
                ..
            } if *amount > 0
        )
    });
    assert!(paid, "the Substitute's price is traced as a cost");
    assert_eq!(engine.state().last_damage, 0);
}

#[test]
fn a_gen_1_crash_is_last_damage() {
    let engine = one_turn(
        Generation::Gen1,
        set("Hitmonlee", ["High Jump Kick"]),
        set("Gengar", ["Splash"]),
    );
    assert_eq!(engine.state().last_damage, 1, "the crash's 1 HP");
}
