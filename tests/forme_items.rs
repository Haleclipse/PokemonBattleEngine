//! L1 tests for the items that give a forme or a type (DESIGN.md §43.4, R-N.4): what the
//! replay cannot see — which species each one asks, the generation's take rules, and the
//! battle-start forme a Rusted Sword gives before anyone is on the field.
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

fn set(species: &str, moves: &[&str], item: &str) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set.item = (!item.is_empty()).then(|| item.to_string());
    set
}

fn engine_for(
    generation: Generation,
    rules: &Arc<ResolvedRules>,
    user: &PokemonSet,
    foe: &PokemonSet,
) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, USER, SideId::P1, user).unwrap(),
        build_pokemon(&dex, FOE, SideId::P2, foe).unwrap(),
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

/// The creation trio's orbs ask the species the holder battles as in Gen 2-4 and its own from
/// Gen 5 (`StatRules::species_items_read_battling`): a Ditto that transformed into Dialga is a
/// Dialga for the Orb in Gen 4 only.
#[test]
fn the_orbs_ask_the_battling_species_by_generation() {
    for (generation, battling) in [(Generation::Gen4, true), (Generation::Gen5, false)] {
        let rules = rules(generation);
        let engine = engine_for(
            generation,
            &rules,
            &set("Dialga", &["Harden"], "Adamant Orb"),
            &set("Snorlax", &["Harden"], ""),
        );
        // A hand-made overlay: what a Transform into Dialga would leave.
        {
            let pokemon = engine.state().pokemon(USER).unwrap().clone();
            let mut ditto = pokemon.clone();
            ditto.species = Some(SpeciesId::DITTO);
            ditto.forme = None;
            ditto.transform = Some(Box::new(pokemon_battle_engine::state::TransformState {
                into: FOE,
                species: Some(SpeciesId::DIALGA),
                types: pokemon.types,
                stats: pokemon.stats,
                ability: pokemon.ability,
                moves: pokemon.moves.clone(),
                hidden_power: pokemon.hidden_power,
            }));
            let species = pokemon_battle_engine::engine::item_species(&rules, &ditto);
            assert_eq!(
                species == Some(SpeciesId::DIALGA),
                battling,
                "{generation:?}: the orb asks the battling species only in Gen 4"
            );
        }
    }
}

/// Gen 4's Griseous Orb is never taken (`onTakeItem: false`); from Gen 5 it is only refused
/// from a Giratina, and Gen 9's Orb carries no check at all (its Core does).
#[test]
fn the_griseous_orb_follows_the_generations_take_rule() {
    for (generation, holder, taker, refused) in [
        (Generation::Gen4, "Snorlax", "Snorlax", true),
        (Generation::Gen5, "Giratina", "Snorlax", true),
        (Generation::Gen5, "Snorlax", "Snorlax", false),
        (Generation::Gen9, "Giratina", "Snorlax", false),
    ] {
        let rules = rules(generation);
        let engine = engine_for(
            generation,
            &rules,
            &set(holder, &["Harden"], "Griseous Orb"),
            &set(taker, &["Harden"], ""),
        );
        assert_eq!(
            pokemon_battle_engine::engine::item_refuses_take(
                engine.state(),
                &rules,
                ItemId::GRISEOUS_ORB,
                USER,
                FOE,
                None,
            ),
            refused,
            "{generation:?}: {holder} holding the Orb, {taker} taking it"
        );
    }
}

/// Ogerpon's masks answer their own formes (`baseSpecies.name.startsWith('Ogerpon-Cornerstone')`,
/// which covers the Terastal forme too) and nothing else — the base Ogerpon included.
#[test]
fn a_mask_answers_its_own_formes_only() {
    let rules = rules(Generation::Gen9);
    let dex = DexResolver::resolve(Generation::Gen9);
    let behavior = pokemon_battle_engine::engine::item_behavior(&rules, ItemId::CORNERSTONE_MASK);
    let move_def = dex.move_by_name("Ivy Cudgel").unwrap();
    let admits = |species| {
        behavior
            .base_power_boost
            .unwrap()
            .filter
            .admits(Some(species), move_def)
    };
    assert!(admits(SpeciesId::OGERPON_CORNERSTONE));
    assert!(admits(SpeciesId::OGERPON_CORNERSTONE_TERA));
    assert!(!admits(SpeciesId::OGERPON));
    assert!(!admits(SpeciesId::OGERPON_WELLSPRING));
    assert!(!admits(SpeciesId::DIALGA));
}

/// The Rusted Sword gives its crowned forme before anyone is on the field (Showdown
/// `onBattleStart`), which is the Pokémon's own species from then on, and renames Iron Head —
/// with the new move's PP. The ability starts as the holder enters, not at the forme change.
#[test]
fn the_rusted_sword_crowns_its_holder_at_battle_start() {
    let rules = rules(Generation::Gen8);
    let engine = engine_for(
        Generation::Gen8,
        &rules,
        &set("Zacian", &["Iron Head", "Harden"], "Rusted Sword"),
        &set("Snorlax", &["Harden"], ""),
    );
    let pokemon = engine.state().pokemon(USER).unwrap();
    assert_eq!(pokemon.species, Some(SpeciesId::ZACIAN_CROWNED));
    assert_eq!(pokemon.forme, None, "the crowned forme is its own");
    assert_eq!(
        pokemon_battle_engine::engine::raw_ability(pokemon),
        pokemon_battle_engine::ids::AbilityId::INTREPID_SWORD
    );
    assert_eq!(
        pokemon.move_slots()[0].move_id,
        pokemon_battle_engine::ids::MoveId::BEHEMOTH_BLADE
    );
    // Three PP Ups of Behemoth Blade's 5 PP (Gen 8's `calculatePP`).
    assert_eq!(pokemon.move_slots()[0].max_pp, 8);
    // The leads have not entered yet, so Intrepid Sword has not started: the boost comes with
    // the switch-in (the frames show it).
    assert_eq!(pokemon.stages.attack, 0);
}
