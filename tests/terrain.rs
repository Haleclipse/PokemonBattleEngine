//! L1 rule tests for M7 E/F batch 3 (DESIGN.md §23.2): which stat Quark Drive raises. Showdown's
//! `getBestStat` keeps the first of Attack, Defense, Special Attack, Special Defense and Speed
//! among equals (`>`, not `>=`); the scenarios' species never tie, so the replay cannot tell.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::ids::{PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, BattleStats, SideState, Stat, TerrainKind};

const HOLDER: PokemonId = PokemonId(1);
const FOE: PokemonId = PokemonId(2);

/// The stat Quark Drive picks as its holder comes in under Electric Terrain with `stats`.
fn quark_drive_pick(stats: BattleStats) -> Option<Stat> {
    let generation = Generation::Gen9;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let set = |species: &str, ability: &str| {
        let mut set = PokemonSet::new(species, ["Harden"]);
        set.ability = Some(ability.into());
        set.level = 50;
        set
    };
    let mut holder =
        build_pokemon(&dex, HOLDER, SideId::P1, &set("Iron Moth", "Quark Drive")).unwrap();
    holder.stats = stats;
    let pokemon = vec![
        holder,
        build_pokemon(&dex, FOE, SideId::P2, &set("Snorlax", "Run Away")).unwrap(),
    ];
    let side = |id| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(pokemon, [side(HOLDER), side(FOE)], 1);
    state.field.set_terrain(TerrainKind::Electric, FOE, 5);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine.run_lead_switch_ins();
    engine
        .state()
        .pokemon(HOLDER)
        .unwrap()
        .volatiles
        .paradox_boost()
        .map(|(_, stat)| stat)
}

#[test]
fn quark_drive_keeps_the_first_of_equal_stats() {
    let stats = |attack, defense, special_attack, special_defense, speed| BattleStats {
        attack,
        defense,
        special_attack,
        special_defense,
        speed,
    };
    assert_eq!(
        quark_drive_pick(stats(100, 90, 130, 90, 120)),
        Some(Stat::SpecialAttack)
    );
    // Attack ties Special Attack: Attack, the first.
    assert_eq!(
        quark_drive_pick(stats(130, 90, 130, 90, 120)),
        Some(Stat::Attack)
    );
    // Special Defense ties Speed: Special Defense.
    assert_eq!(
        quark_drive_pick(stats(100, 90, 110, 140, 140)),
        Some(Stat::SpecialDefense)
    );
}
