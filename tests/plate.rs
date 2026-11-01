//! L1 tests for Arceus's Plates (DESIGN.md §43.4, R-N.6): what the replay cannot see — the type
//! a Plate gives in each generation, the generation's read of the species, and the ability
//! change Multitype refuses.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::ids::{ItemId, PokemonId, SideId, SpeciesId};
use pokemon_battle_engine::rules::{ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const USER: PokemonId = PokemonId(1);
const FOE: PokemonId = PokemonId(2);

fn rules(generation: Generation) -> Arc<ResolvedRules> {
    Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    )
}

fn set(species: &str, moves: &[&str], item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set.ability = Some("Multitype".to_string());
    set.item = item.map(str::to_string);
    set
}

fn engine_for(
    generation: Generation,
    rules: &Arc<ResolvedRules>,
    user: &PokemonSet,
) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, USER, SideId::P1, user).unwrap(),
        build_pokemon(&dex, FOE, SideId::P2, &set("Snorlax", &["Harden"], None)).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    BattleEngine::try_new(
        rules.clone(),
        dex,
        BattleState::new(pokemon, [side(USER), side(FOE)], 1),
    )
    .unwrap()
}

/// The type the Plate gives: Fire with a Flame Plate, Normal without one (never the
/// generation's `???` filler), and from Gen 8 only with Multitype.
#[test]
fn a_plate_fixes_arceus_types() {
    for generation in [Generation::Gen4, Generation::Gen9] {
        let rules = rules(generation);
        let engine = engine_for(
            generation,
            &rules,
            &set("Arceus", &["Judgment"], Some("Flame Plate")),
        );
        let pokemon = engine.state().pokemon(USER).unwrap();
        let types = pokemon_battle_engine::engine::types_of(engine.state(), &rules, pokemon);
        assert_eq!(types.primary, pokemon_battle_engine::dex::PokemonType::Fire);
        assert_eq!(
            pokemon_battle_engine::engine::plate_type(engine.state(), &rules, pokemon),
            Some(pokemon_battle_engine::dex::PokemonType::Fire)
        );
        let bare = engine_for(generation, &rules, &set("Arceus", &["Judgment"], None));
        let pokemon = bare.state().pokemon(USER).unwrap();
        let types = pokemon_battle_engine::engine::types_of(bare.state(), &rules, pokemon);
        assert_eq!(
            types.primary,
            pokemon_battle_engine::dex::PokemonType::Normal,
            "{generation:?}: an Arceus with no Plate is Normal"
        );
        // A Plate on an Arceus that is not Multitype: Normal before Gen 8, its own types from
        // Gen 8 (`pokemon.ability !== 'multitype' && this.gen >= 8`).
        let other_rules = rules;
        let mut stranger = set("Arceus", &["Judgment"], Some("Flame Plate"));
        stranger.ability = Some("Intimidate".to_string());
        let engine = engine_for(generation, &other_rules, &stranger);
        let pokemon = engine.state().pokemon(USER).unwrap();
        let types = pokemon_battle_engine::engine::types_of(engine.state(), &other_rules, pokemon);
        assert_eq!(
            types.primary,
            pokemon_battle_engine::dex::PokemonType::Normal,
            "{generation:?}: an Arceus without Multitype is Normal (Fire only from Gen 8's own-type rule, which Arceus's Normal also is)"
        );
    }
}

/// Gen 4 switches the forme as the Arceus comes in, until it leaves the field (its own species
/// stays Arceus, R-M8-81); Gen 5+ keeps the species and only the type follows. Gen 4's Plate has
/// no refusal of its own — Multitype keeps it (`keeps_item`; pokeplatinum `BtlCmd_TryStealItem`,
/// R-M8-105) — where Gen 5+'s Plate stays with an Arceus.
#[test]
fn the_forme_follows_the_plate_in_gen_4_only() {
    let gen4_rules = rules(Generation::Gen4);
    let mut engine = engine_for(
        Generation::Gen4,
        &gen4_rules,
        &set("Arceus", &["Judgment"], Some("Flame Plate")),
    );
    engine.run_lead_switch_ins();
    let pokemon = engine.state().pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::species_of(pokemon),
        Some(SpeciesId::ARCEUS_FIRE)
    );
    assert_eq!(pokemon.species, Some(SpeciesId::ARCEUS), "its own species");
    assert!(
        !pokemon_battle_engine::engine::item_refuses_take(
            engine.state(),
            &gen4_rules,
            ItemId::FLAME_PLATE,
            USER,
            FOE,
            None,
        ),
        "Gen 4's Plate has no refusal of its own"
    );
    assert!(
        pokemon_battle_engine::engine::ability_behavior(
            &gen4_rules,
            pokemon_battle_engine::engine::raw_ability(pokemon)
        )
        .keeps_item,
        "Gen 4's Multitype keeps it"
    );
    let gen9_rules = rules(Generation::Gen9);
    let mut engine = engine_for(
        Generation::Gen9,
        &gen9_rules,
        &set("Arceus", &["Judgment"], Some("Flame Plate")),
    );
    engine.run_lead_switch_ins();
    assert_eq!(
        engine.state().pokemon(USER).unwrap().species,
        Some(SpeciesId::ARCEUS),
        "Gen 9 has no forme change"
    );
    assert!(
        pokemon_battle_engine::engine::item_refuses_take(
            engine.state(),
            &gen9_rules,
            ItemId::FLAME_PLATE,
            USER,
            FOE,
            None,
        ),
        "Gen 9's Plate stays on an Arceus"
    );
}

/// Gen 4's Multitype refuses an ability change (`onSetAbility: false`): Skill Swap aimed at its
/// holder does nothing.
#[test]
fn multitype_refuses_an_ability_change_in_gen_4() {
    let gen4_rules = rules(Generation::Gen4);
    let engine = engine_for(
        Generation::Gen4,
        &gen4_rules,
        &set("Arceus", &["Harden"], Some("Flame Plate")),
    );
    let user = engine.state().pokemon(USER).unwrap();
    assert!(
        pokemon_battle_engine::engine::ability_behavior(
            &gen4_rules,
            pokemon_battle_engine::engine::raw_ability(user)
        )
        .refuses_ability_change,
        "Gen 4's Multitype"
    );
    let rules9 = rules(Generation::Gen9);
    let engine9 = engine_for(
        Generation::Gen9,
        &rules9,
        &set("Arceus", &["Harden"], Some("Flame Plate")),
    );
    let user9 = engine9.state().pokemon(USER).unwrap();
    assert!(
        !pokemon_battle_engine::engine::ability_behavior(
            &rules9,
            pokemon_battle_engine::engine::raw_ability(user9)
        )
        .refuses_ability_change,
        "Gen 5+ does not declare it"
    );
}
