//! L1 tests for forme changes (DESIGN.md §43, R-N.1): what the replay cannot see. The frames
//! carry a forme's species, types and ability and damage carries its stats, but not the values
//! it will go back to when it leaves — nor the `notransform` step of `ignoringAbility`, which
//! only shows through a suppressible ability's handler not running.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{AbilityId, MoveSlot, PokemonId, SideId, SpeciesId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, TransformState};

const USER: PokemonId = PokemonId(1);
const FOE: PokemonId = PokemonId(2);

fn set(species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = level;
    set
}

/// An engine with both Pokémon on the field, the user on the left.
fn engine_for(generation: Generation, user: &PokemonSet, foe: &PokemonSet) -> BattleEngine {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
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
    let state = BattleState::new(pokemon, [side(USER), side(FOE)], 1);
    BattleEngine::try_new(rules, dex, state).unwrap()
}

/// Both sides use their first move at each other, to the end of the turn.
fn play_one_turn(engine: &mut BattleEngine) {
    let topology = engine.rules().topology();
    let choice = |side: SideId, actor: PokemonId, target: SideId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: Some(DeclaredTarget(topology.position(target, 0).unwrap())),
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, USER, SideId::P2),
            choice(SideId::P2, FOE, SideId::P1),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
}

/// A Pokémon's forme book holds every forme of its species with the values the set gives it
/// (DESIGN.md §43.1): Shaymin-Sky's book has both its formes, each with its own types, stats,
/// ability and the one it goes back to.
#[test]
fn the_forme_book_holds_every_forme_with_the_sets_values() {
    let dex = DexResolver::resolve(Generation::Gen7);
    let shaymin =
        build_pokemon(&dex, USER, SideId::P1, &set("Shaymin-Sky", &["Harden"], 50)).unwrap();
    let sky = shaymin
        .forme_of(SpeciesId::SHAYMIN_SKY)
        .expect("its own forme");
    let land = shaymin
        .forme_of(SpeciesId::SHAYMIN)
        .expect("the other forme");
    assert_eq!(sky.first_ability, AbilityId::SERENE_GRACE);
    assert_eq!(land.first_ability, AbilityId::NATURAL_CURE);
    assert_eq!(land.changes_from, None);
    assert!(sky.types != land.types, "Grass/Flying against Grass");
    assert!(sky.stats.speed > land.stats.speed);
}

/// Stance Change: Sacred Sword writes the Blade Forme's stats and keeps the Shield Forme's for
/// the leaving (Showdown `setSpecies(baseSpecies)`).
#[test]
fn a_forme_change_writes_the_formes_stats_and_the_ones_it_returns_to() {
    let mut engine = engine_for(
        Generation::Gen7,
        &set("Aegislash", &["Sacred Sword"], 50),
        &set("Snorlax", &["Harden"], 50),
    );
    let own = *engine.state().pokemon(USER).unwrap().battle_stats();
    assert_eq!(
        engine.state().pokemon(USER).unwrap().species,
        Some(SpeciesId::AEGISLASH)
    );
    play_one_turn(&mut engine);
    let pokemon = engine.state().pokemon(USER).unwrap();
    assert_eq!(pokemon.forme, Some(SpeciesId::AEGISLASH_BLADE));
    assert_eq!(
        pokemon_battle_engine::engine::species_of(pokemon),
        Some(SpeciesId::AEGISLASH_BLADE)
    );
    // Its own species is still the one the item and the dex ask of it.
    assert_eq!(pokemon.species, Some(SpeciesId::AEGISLASH));
    let blade = *pokemon.battle_stats();
    assert!(blade.attack > own.attack && blade.defense < own.defense);
    assert_eq!(pokemon.unchanged_stats, Some(own));
}

/// `notransform` (DESIGN.md §43.3): a transformed Pokémon has none of these in effect, even
/// though they cannot be suppressed (`ignoringAbility` checks it before `cantsuppress`: Gulp
/// Missile is both).
#[test]
fn a_transformed_pokemon_has_no_notransform_ability() {
    let engine = engine_for(
        Generation::Gen9,
        &set("Mimikyu", &["Harden"], 50),
        &set("Snorlax", &["Harden"], 50),
    );
    let state = engine.state();
    let rules = engine.rules();
    let mimikyu = state.pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::raw_ability(mimikyu),
        AbilityId::DISGUISE
    );
    assert!(
        !pokemon_battle_engine::engine::ignoring_ability(state, rules, mimikyu),
        "Disguise is not the least suppressible on an untransformed Mimikyu"
    );
    let transformed = |ability| {
        let mut pokemon = state.pokemon(USER).unwrap().clone();
        pokemon.transform = Some(Box::new(TransformState {
            into: FOE,
            species: None,
            types: pokemon.types,
            stats: pokemon.stats,
            ability,
            moves: pokemon.moves.clone(),
            hidden_power: pokemon.hidden_power,
        }));
        pokemon
    };
    assert!(
        pokemon_battle_engine::engine::ignoring_ability(
            state,
            rules,
            &transformed(AbilityId::GULP_MISSILE)
        ),
        "the Gulp Missile it copied has no effect on a transformed Pokémon"
    );
    assert!(
        !pokemon_battle_engine::engine::ignoring_ability(
            state,
            rules,
            &transformed(AbilityId::THICK_FAT)
        ),
        "an ability without the flag counts as usual"
    );
}
