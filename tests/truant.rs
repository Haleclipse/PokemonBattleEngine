//! L1 tests for Truant (DESIGN.md §15.21): where its check stands against the recharge turn's.
//! A loafing turn and a recharge turn after Hyper Beam pass the same turns either way round — Gen 3
//! flips its flag at the turn's end whatever stopped the move, Gen 5+'s recharge clears the
//! `truant` volatile (Showdown `mustrecharge`) — so only the `cant` reason tells the order, and the
//! replay reads it as a refusal that drew nothing.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BeforeMoveBlockReason, EngineStep, MoveFailReason, SideChoice, SlotChoice,
    TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, PokemonState, SideState, VolatileConditionKind};

const SLAKING: PokemonId = PokemonId(1);
const WALL: PokemonId = PokemonId(2);

/// Slaking, its recharge turn pending and its next attempt a loafing one (`loafing`), facing a
/// Blissey that hardens: why its move fails, and whether the `truant` volatile is left.
fn recharge_turn(
    generation: Generation,
    loafing: impl FnOnce(&mut PokemonState),
) -> (BeforeMoveBlockReason, bool) {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut slaking = PokemonSet::new("Slaking", ["Tackle"]);
    slaking.ability = Some("Truant".into());
    let mut wall = PokemonSet::new("Blissey", ["Harden"]);
    wall.ability = Some("Natural Cure".into());
    let mut slaking = build_pokemon(&dex, SLAKING, SideId::P1, &slaking).unwrap();
    slaking.volatiles.set_must_recharge(Some(2));
    loafing(&mut slaking);
    let pokemon = vec![
        slaking,
        build_pokemon(&dex, WALL, SideId::P2, &wall).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(SLAKING), side(WALL)], 1);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    let choice = |side: SideId, actor: PokemonId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: None,
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([choice(SideId::P1, SLAKING), choice(SideId::P2, WALL)])
        .unwrap();
    let mut reason = None;
    let mut truant_left = false;
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {
        if reason.is_none() {
            reason = engine
                .trace()
                .events()
                .iter()
                .find_map(|event| match event {
                    TraceEvent::MoveFailed {
                        actor: SLAKING,
                        reason: MoveFailReason::BeforeMove(reason),
                        ..
                    } => Some(*reason),
                    _ => None,
                });
            truant_left = engine
                .state()
                .pokemon(SLAKING)
                .unwrap()
                .volatiles
                .contains(VolatileConditionKind::Truant);
        }
    }
    (reason.expect("Slaking's move failed"), truant_left)
}

#[test]
fn gen3_4_truant_comes_before_the_recharge() {
    // Gen 3: the flag set; Gen 4: odd turns, and turn 1 is odd.
    for generation in [Generation::Gen3, Generation::Gen4] {
        let (reason, _) = recharge_turn(generation, |slaking| slaking.truant_turn = true);
        assert_eq!(reason, BeforeMoveBlockReason::Truant, "{generation:?}");
    }
}

#[test]
fn gen5_recharge_comes_first_and_clears_the_truant_volatile() {
    let (reason, truant_left) = recharge_turn(Generation::Gen9, |slaking| {
        slaking.volatiles.set_marker(VolatileConditionKind::Truant);
    });
    assert_eq!(reason, BeforeMoveBlockReason::Recharge);
    assert!(!truant_left);
}
