//! L1 test for Curse (DESIGN.md §15.16): what the replay cannot see — the request asks a target
//! of a Ghost type's Curse in doubles, none of anyone else's (Showdown `getMoves` reports
//! `nonGhostTarget`); the replay's harness fills targets in itself.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, ChoiceError, SideChoice, SlotChoice,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

fn doubles(user: &str) -> BattleEngine {
    let generation = Generation::Gen9;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let set = |species: &str, moves: &[&str]| {
        let mut set = PokemonSet::new(species, moves.iter().copied());
        set.level = 50;
        set
    };
    let pokemon = vec![
        build_pokemon(&dex, PokemonId(1), SideId::P1, &set(user, &["Curse"])).unwrap(),
        build_pokemon(&dex, PokemonId(2), SideId::P1, &set("Chansey", &["Harden"])).unwrap(),
        build_pokemon(&dex, PokemonId(3), SideId::P2, &set("Chansey", &["Harden"])).unwrap(),
        build_pokemon(&dex, PokemonId(4), SideId::P2, &set("Chansey", &["Harden"])).unwrap(),
    ];
    let side = |team: [u8; 2]| {
        let team: Vec<PokemonId> = team.iter().map(|id| PokemonId(*id)).collect();
        let mut side = SideState::new(team.clone(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let state = BattleState::new(pokemon, [side([1, 2]), side([3, 4])], 2);
    BattleEngine::try_new(rules, dex, state).unwrap()
}

fn untargeted_curse(engine: &mut BattleEngine) -> Result<(), BattleError> {
    let slot = |actor: u8| SlotChoice {
        actor: PokemonId(actor),
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    };
    let choice = |side: SideId, actors: [u8; 2]| SideChoice {
        side,
        actions: actors.into_iter().map(slot).collect(),
    };
    engine
        .submit_turn_choices([choice(SideId::P1, [1, 2]), choice(SideId::P2, [3, 4])])
        .map(|_| ())
}

#[test]
fn a_ghost_types_curse_asks_a_target_and_anyone_elses_does_not() {
    assert!(untargeted_curse(&mut doubles("Snorlax")).is_ok());
    assert!(matches!(
        untargeted_curse(&mut doubles("Gengar")),
        Err(BattleError::Choice(ChoiceError::InvalidTarget(_)))
    ));
}
