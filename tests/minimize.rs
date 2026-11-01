//! L1 rule test for R169 (DESIGN.md §20.1) where the replay cannot tell: it plans an accuracy
//! roll exactly where the engine's own `accuracy_roll` says there is one, so whether Minimize
//! makes a `minimize` move sure to hit only shows in the roll itself.

use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::ids::{PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, Stat, VolatileConditionKind};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

/// The accuracy check of the user's `move_name` on a target at +2 evasion, Minimized or not.
fn accuracy(generation: Generation, move_name: &str, minimized: bool) -> Option<(u16, u16)> {
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let user = build_pokemon(
        &dex,
        USER,
        SideId::P1,
        &PokemonSet::new("Snorlax", [move_name]),
    )
    .unwrap();
    let mut target = build_pokemon(
        &dex,
        TARGET,
        SideId::P2,
        &PokemonSet::new("Chansey", ["Minimize"]),
    )
    .unwrap();
    target.stages.set(Stat::Evasion, 2);
    if minimized {
        target.volatiles.set_marker(VolatileConditionKind::Minimize);
    }
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![user, target], [side(USER), side(TARGET)], 1);
    let move_id = state.pokemon(USER).unwrap().moves[0].move_id;
    let move_def = dex.move_by_id(move_id).unwrap().clone();
    pokemon_battle_engine::engine::accuracy_roll(&state, &rules, USER, TARGET, &move_def)
}

#[test]
fn minimize_makes_minimize_moves_sure_to_hit_from_gen_6() {
    for generation in [Generation::Gen6, Generation::Gen9] {
        assert_eq!(
            accuracy(generation, "Body Slam", true),
            None,
            "{generation:?}"
        );
        assert!(accuracy(generation, "Body Slam", false).is_some());
        // Not a `minimize` move: the evasion counts.
        assert!(accuracy(generation, "Tackle", true).is_some());
    }
    // The gen5 mod drops the condition's `onAccuracy`.
    assert!(accuracy(Generation::Gen5, "Stomp", true).is_some());
}
