//! L1 tests for Commander (DESIGN.md §43.4, R-N.5): what the replay cannot see. Showdown never
//! offers a commanded Pokémon's switch (its request keeps the slot but the action is
//! cancelled), so the trap and the action cancellation are pinned here.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BeforeMoveBlockReason, CanMoveResult, EngineStep, SideChoice, SlotChoice,
    trapped_by,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, VolatileConditionKind};

const DONDOZO: PokemonId = PokemonId(1);
const TATSUGIRI: PokemonId = PokemonId(2);
const FOE: PokemonId = PokemonId(3);
const FOE2: PokemonId = PokemonId(4);

fn rules() -> Arc<ResolvedRules> {
    Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Doubles,
            "custom",
        ))
        .unwrap(),
    )
}

fn set(species: &str, ability: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set.ability = Some(ability.to_string());
    set
}

fn doubles_engine() -> BattleEngine {
    let rules = rules();
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let pokemon = vec![
        build_pokemon(
            &dex,
            DONDOZO,
            SideId::P1,
            &set("Dondozo", "Unaware", &["Order Up"]),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            TATSUGIRI,
            SideId::P1,
            &set("Tatsugiri-Droopy", "Commander", &["Harden"]),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            FOE,
            SideId::P2,
            &set("Snorlax", "Thick Fat", &["Harden"]),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            FOE2,
            SideId::P2,
            &set("Blissey", "Natural Cure", &["Harden"]),
        )
        .unwrap(),
    ];
    let side = |slots: [PokemonId; 2], team: Vec<PokemonId>| {
        let mut side = SideState::new(team, 2);
        side.active = slots.map(Some).to_vec();
        side
    };
    let mut engine = BattleEngine::try_new(
        rules,
        dex,
        BattleState::new(
            pokemon,
            [
                side([DONDOZO, TATSUGIRI], vec![DONDOZO, TATSUGIRI]),
                side([FOE, FOE2], vec![FOE, FOE2]),
            ],
            1,
        ),
    )
    .unwrap();
    engine.run_lead_switch_ins();
    engine
}

/// Commander takes hold as the pair comes in: the Dondozo's five stats rise two stages and
/// both carry their volatile.
#[test]
fn commander_pairs_the_two_as_they_come_in() {
    let engine = doubles_engine();
    let dondozo = engine.state().pokemon(DONDOZO).unwrap();
    let tatsugiri = engine.state().pokemon(TATSUGIRI).unwrap();
    assert!(
        tatsugiri
            .volatiles
            .contains(VolatileConditionKind::Commanding),
        "the Tatsugiri took the Dondozo into its mouth"
    );
    assert!(dondozo.volatiles.contains(VolatileConditionKind::Commanded));
    assert_eq!(dondozo.stages.attack, 2);
    assert_eq!(dondozo.stages.special_attack, 2);
    assert_eq!(dondozo.stages.speed, 2);
    assert_eq!(dondozo.stages.defense, 2);
    assert_eq!(dondozo.stages.special_defense, 2);
}

/// Neither half of the pair may switch out (`onTrapPokemon` priority −11, ahead of Shed Shell's
/// −10), and the commanding Tatsugiri's action comes to nothing.
#[test]
fn commander_traps_both_and_cancels_the_tatsugiris_action() {
    let mut engine = doubles_engine();
    assert_eq!(
        trapped_by(engine.state(), engine.rules(), TATSUGIRI),
        Some(VolatileConditionKind::Commanding)
    );
    assert_eq!(
        trapped_by(engine.state(), engine.rules(), DONDOZO),
        Some(VolatileConditionKind::Commanding)
    );
    // The Tatsugiri still gets a choice, and taking one costs it nothing: its action is
    // cancelled before the turn (`onBeforeTurn`).
    let topology = engine.rules().topology();
    let choice = |side: SideId, actions: Vec<SlotChoice>| SideChoice { side, actions };
    let slot = |actor: PokemonId, index: u8, target: SideId| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(index).unwrap(),
            target: Some(DeclaredTarget(topology.position(target, 0).unwrap())),
            mechanic: None,
        },
    };
    engine
        .submit_turn_choices([
            choice(
                SideId::P1,
                vec![slot(DONDOZO, 0, SideId::P2), slot(TATSUGIRI, 0, SideId::P1)],
            ),
            choice(
                SideId::P2,
                vec![slot(FOE, 0, SideId::P2), slot(FOE2, 0, SideId::P2)],
            ),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let blocked = engine.trace().events().iter().any(|event| {
        matches!(
            event,
            pokemon_battle_engine::engine::TraceEvent::MoveFailed {
                actor,
                reason: pokemon_battle_engine::engine::MoveFailReason::BeforeMove(
                    BeforeMoveBlockReason::Commanding
                ),
                ..
            } if *actor == TATSUGIRI
        )
    });
    assert!(blocked, "the commanding Tatsugiri's action is cancelled");
    // Its own stats never moved: only the Dondozo's Order Up did.
    assert_eq!(engine.state().pokemon(TATSUGIRI).unwrap().stages.attack, 0);
    assert_eq!(
        engine.state().pokemon(DONDOZO).unwrap().stages.defense,
        3,
        "Order Up with the Droopy Tatsugiri raises Defense"
    );
    let _ = CanMoveResult::Allow;
}
