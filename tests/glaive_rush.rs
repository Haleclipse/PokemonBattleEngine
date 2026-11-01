//! L1 test for Glaive Rush (DESIGN.md §15.20): what the replay cannot see. Its drawback makes
//! every move at its user hit (the condition's `onAccuracy`) — the replay plans the accuracy
//! decisions with the engine's own `accuracy_roll`, so a check the drawback should skip would
//! still replay; only the check itself tells.

use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::accuracy_roll;
use pokemon_battle_engine::ids::{PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, VolatileConditionKind};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// A's Focus Blast on B, B open from its Glaive Rush or not: the accuracy roll.
fn focus_blast_on(open: bool) -> Option<(u16, u16)> {
    let generation = Generation::Gen9;
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let user = PokemonSet::new("Corviknight", ["Focus Blast"]);
    let target = PokemonSet::new("Baxcalibur", ["Glaive Rush"]);
    let mut b = build_pokemon(&dex, B, SideId::P2, &target).unwrap();
    if open {
        b.volatiles.set_marker(VolatileConditionKind::GlaiveRush);
    }
    let pokemon = vec![build_pokemon(&dex, A, SideId::P1, &user).unwrap(), b];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(A), side(B)], 1);
    let move_id = state.pokemon(A).unwrap().moves[0].move_id;
    let move_def = dex.move_by_id(move_id).unwrap().clone();
    accuracy_roll(&state, &rules, A, B, &move_def)
}

#[test]
fn glaive_rush_drawback_lets_every_move_hit() {
    assert_eq!(focus_blast_on(false), Some((70, 100)));
    assert_eq!(focus_blast_on(true), None);
}
