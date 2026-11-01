//! L1 test for Gen 1 Roar and Whirlwind (DESIGN.md §15.17): what the replay cannot see — they
//! check no accuracy (pokered `ResidualEffects1`, issues.md I74). Showdown's Whirlwind may
//! `-miss`, which changes nothing a frame shows.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::accuracy_roll;
use pokemon_battle_engine::ids::{MoveId, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

#[test]
fn gen1_roar_and_whirlwind_check_no_accuracy() {
    let generation = Generation::Gen1;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let a = build_pokemon(
        &dex,
        PokemonId(1),
        SideId::P1,
        &PokemonSet::new("Pidgeot", ["Roar", "Whirlwind", "Gust"]),
    )
    .unwrap();
    let b = build_pokemon(
        &dex,
        PokemonId(2),
        SideId::P2,
        &PokemonSet::new("Snorlax", ["Harden"]),
    )
    .unwrap();
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![a, b], [side(PokemonId(1)), side(PokemonId(2))], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    for id in [MoveId::ROAR, MoveId::WHIRLWIND] {
        let m = dex.move_by_id(id).unwrap();
        assert_eq!(
            accuracy_roll(engine.state(), &rules, PokemonId(1), PokemonId(2), m),
            None,
            "{}",
            m.name
        );
    }
    // A Gen 1 move with an accuracy of its own still checks it.
    let gust = dex.move_by_id(MoveId::GUST).unwrap();
    assert!(accuracy_roll(engine.state(), &rules, PokemonId(1), PokemonId(2), gust).is_some());
}
