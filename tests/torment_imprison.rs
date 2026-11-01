//! L1 tests for Torment and Imprison at the request (DESIGN.md §15.13): what the replay cannot
//! see — its scripts only ever choose a move the request allows.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, ChoiceError, SideChoice, SlotChoice, must_struggle,
};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, VolatileConditionKind};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// A (Snorlax, `moves`) against B (Blissey, `foe`), with `edit` applied to the state.
fn arena(moves: &[&str], foe: &[&str], edit: impl FnOnce(&mut BattleState)) -> BattleEngine {
    let generation = Generation::Gen9;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let a = build_pokemon(
        &dex,
        A,
        SideId::P1,
        &PokemonSet::new("Snorlax", moves.iter().copied()),
    )
    .unwrap();
    let b = build_pokemon(
        &dex,
        B,
        SideId::P2,
        &PokemonSet::new("Blissey", foe.iter().copied()),
    )
    .unwrap();
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![a, b], [side(A), side(B)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let mut snapshot = engine.snapshot();
    edit(&mut snapshot.state);
    BattleEngine::restore(rules, dex, snapshot).unwrap()
}

fn choose(engine: &BattleEngine, slot: u8) -> Result<(), ChoiceError> {
    engine.validate_choice(
        SideId::P1,
        &SideChoice {
            side: SideId::P1,
            actions: vec![SlotChoice {
                actor: A,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot).unwrap(),
                    target: None,
                    mechanic: None,
                },
            }],
        },
    )
}

#[test]
fn torment_takes_the_last_move_off_the_request() {
    let engine = arena(&["Tackle", "Growl"], &["Harden"], |state| {
        let a = state.pokemon_mut(A).unwrap();
        assert!(a.volatiles.set_marker(VolatileConditionKind::Torment));
        a.last_move = Some(MoveId::TACKLE);
    });
    assert!(matches!(
        choose(&engine, 0),
        Err(ChoiceError::MoveDisabled {
            by: VolatileConditionKind::Torment,
            ..
        })
    ));
    assert_eq!(choose(&engine, 1), Ok(()));
}

#[test]
fn imprison_takes_the_holders_moves_off_the_foes_request_down_to_struggle() {
    let engine = arena(&["Tackle", "Growl"], &["Tackle", "Harden"], |state| {
        assert!(
            state
                .pokemon_mut(B)
                .unwrap()
                .volatiles
                .set_marker(VolatileConditionKind::Imprison)
        );
    });
    assert!(matches!(
        choose(&engine, 0),
        Err(ChoiceError::MoveDisabled {
            by: VolatileConditionKind::Imprison,
            ..
        })
    ));
    assert_eq!(choose(&engine, 1), Ok(()));
    // Every move of A's imprisoned: Struggle.
    let engine = arena(&["Tackle"], &["Tackle", "Harden"], |state| {
        assert!(
            state
                .pokemon_mut(B)
                .unwrap()
                .volatiles
                .set_marker(VolatileConditionKind::Imprison)
        );
    });
    assert!(must_struggle(
        engine.state(),
        engine.rules(),
        engine.dex(),
        A
    ));
}
