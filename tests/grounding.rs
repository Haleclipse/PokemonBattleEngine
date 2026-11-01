//! L1 tests for K.7b (DESIGN.md §41.5): what the replay cannot see. Telekinesis makes every move
//! at its holder but a one-hit KO hit (`onAccuracy`); a rate decides nothing a log shows.
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::ids::{PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, VolatileConditionKind};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// The accuracy check of Golem's `move_name` on Snorlax, lifted by Telekinesis or not.
fn accuracy(move_name: &str, lifted: bool) -> Option<(u16, u16)> {
    let generation = Generation::Gen5;
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let set = |species: &str, moves: &[&str]| {
        let mut set = PokemonSet::new(species, moves.iter().copied());
        set.level = 50;
        set
    };
    let mut b = build_pokemon(&dex, B, SideId::P2, &set("Snorlax", &["Harden"])).unwrap();
    if lifted {
        b.volatiles.set_telekinesis(3);
    }
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &set("Golem", &[move_name])).unwrap(),
        b,
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(A), side(B)], 1);
    assert!(
        !lifted
            || state
                .pokemon(B)
                .unwrap()
                .volatiles
                .contains(VolatileConditionKind::Telekinesis)
    );
    let move_id = state.pokemon(A).unwrap().moves[0].move_id;
    let move_def = dex.move_by_id(move_id).unwrap().clone();
    pokemon_battle_engine::engine::accuracy_roll(&state, &rules, A, B, &move_def)
}

#[test]
fn telekinesis_makes_every_move_but_a_one_hit_ko_hit() {
    assert!(accuracy("Dynamic Punch", false).is_some());
    assert_eq!(accuracy("Dynamic Punch", true), None);
    assert!(accuracy("Fissure", true).is_some());
}
