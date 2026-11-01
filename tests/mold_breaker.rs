//! L1 rule tests for the moves that ignore abilities (DESIGN.md R204): what no scenario can
//! show for every ability at once — while such a move is used (`BattleState::active_move`) every
//! breakable ability but the user's own (Gen 8+) runs no handler, and nothing else changes.

use std::sync::Arc;

use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::ability_running;
use pokemon_battle_engine::ids::{AbilityId, PokemonId, SideId};
use pokemon_battle_engine::rules::{ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{ActiveMove, BattleState, SideState};

const USER: PokemonId = PokemonId(1);
const HOLDER: PokemonId = PokemonId(2);

fn battle(
    generation: Generation,
    user_ability: AbilityId,
    holder_ability: AbilityId,
    holder_item: Option<&str>,
) -> (ResolvedRules, BattleState) {
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex: Arc<ResolvedDex> = Arc::new(DexResolver::resolve(generation));
    let build = |id, side, ability: AbilityId, item: Option<&str>| {
        let mut set = PokemonSet::new("Snorlax", ["Harden"]);
        set.ability = Some(dex.ability_by_id(ability).unwrap().name.clone());
        set.item = item.map(Into::into);
        build_pokemon(&dex, id, side, &set).unwrap()
    };
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(
        vec![
            build(USER, SideId::P1, user_ability, None),
            build(HOLDER, SideId::P2, holder_ability, holder_item),
        ],
        [side(USER), side(HOLDER)],
        1,
    );
    (rules, state)
}

fn ignoring(state: &mut BattleState) {
    state.active_move = Some(ActiveMove {
        user: USER,
        move_id: pokemon_battle_engine::ids::MoveId::TACKLE,
        category: pokemon_battle_engine::dex::MoveCategory::Physical,
        ignores_ability: true,
        stellar_boosted: false,
        z_powered: false,
        base_move: None,
        prankster_boosted: false,
    });
}

#[test]
fn every_breakable_ability_runs_no_handler_while_a_move_ignores_it() {
    for generation in [
        Generation::Gen4,
        Generation::Gen5,
        Generation::Gen6,
        Generation::Gen7,
        Generation::Gen8,
        Generation::Gen9,
    ] {
        let probe = battle(
            generation,
            AbilityId::MOLD_BREAKER,
            AbilityId::INTIMIDATE,
            None,
        )
        .0;
        for &ability in probe.conditions().breakable {
            let (rules, mut state) = battle(generation, AbilityId::MOLD_BREAKER, ability, None);
            let holder = |state: &BattleState| state.pokemon(HOLDER).unwrap().clone();
            assert_eq!(
                ability_running(&state, &rules, &holder(&state)),
                Some(ability),
                "{generation:?} {ability:?} before the move"
            );
            ignoring(&mut state);
            assert_eq!(
                ability_running(&state, &rules, &holder(&state)),
                None,
                "{generation:?} {ability:?} under the move"
            );
        }
        // An ability that is not breakable keeps running.
        let (rules, mut state) = battle(
            generation,
            AbilityId::MOLD_BREAKER,
            AbilityId::INTIMIDATE,
            None,
        );
        ignoring(&mut state);
        assert_eq!(
            ability_running(&state, &rules, state.pokemon(HOLDER).unwrap()),
            Some(AbilityId::INTIMIDATE),
            "{generation:?}"
        );
    }
}

#[test]
fn from_gen_8_the_user_keeps_its_own_breakable_ability() {
    for (generation, kept) in [(Generation::Gen7, false), (Generation::Gen8, true)] {
        let (rules, mut state) = battle(
            generation,
            AbilityId::THICK_FAT,
            AbilityId::INTIMIDATE,
            None,
        );
        ignoring(&mut state);
        assert_eq!(
            ability_running(&state, &rules, state.pokemon(USER).unwrap()),
            kept.then_some(AbilityId::THICK_FAT),
            "{generation:?}"
        );
    }
}

#[test]
fn ability_shield_keeps_a_breakable_ability_running() {
    let (rules, mut state) = battle(
        Generation::Gen9,
        AbilityId::MOLD_BREAKER,
        AbilityId::THICK_FAT,
        Some("Ability Shield"),
    );
    ignoring(&mut state);
    assert_eq!(
        ability_running(&state, &rules, state.pokemon(HOLDER).unwrap()),
        Some(AbilityId::THICK_FAT)
    );
}
