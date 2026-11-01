//! L1 tests for the Memories and Drives (DESIGN.md §43.4, R-N.7): what the replay cannot see —
//! the species condition reads the item as it is held (a benched or Embargoed Silvally keeps
//! the Memory's type) while Multi-Attack / Techno Blast go without it, the generation's read
//! of a holder without the ability, and Gen 4's Arceus entering Normal.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, PokemonType, build_pokemon};
use pokemon_battle_engine::ids::{ItemId, MoveId, PokemonId, SideId, SpeciesId};
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

fn set(species: &str, ability: &str, moves: &[&str], item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set.ability = Some(ability.to_string());
    set.item = item.map(str::to_string);
    set
}

/// A lead `user` (active) against a Snorlax, switched in. `edit` changes the state after it is
/// built.
fn engine_for(
    generation: Generation,
    rules: &Arc<ResolvedRules>,
    user: &PokemonSet,
    edit: impl FnOnce(&mut BattleState),
) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, USER, SideId::P1, user).unwrap(),
        build_pokemon(
            &dex,
            FOE,
            SideId::P2,
            &set("Snorlax", "Thick Fat", &["Harden"], None),
        )
        .unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(pokemon, [side(USER), side(FOE)], 1);
    edit(&mut state);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    engine.run_lead_switch_ins();
    engine
}

fn type_of(engine: &BattleEngine, rules: &ResolvedRules) -> PokemonType {
    let pokemon = engine.state().pokemon(USER).unwrap();
    pokemon_battle_engine::engine::types_of(engine.state(), rules, pokemon).primary
}

/// The type a Memory gives: the Memory's with RKS System, Normal without one, and the species'
/// own for a Silvally built as another forme (Gen 8 keeps it; Gen 7's condition says Normal).
#[test]
fn a_memory_fixes_silvallys_type() {
    for generation in [Generation::Gen7, Generation::Gen8] {
        let rules = rules(generation);
        let engine = engine_for(
            generation,
            &rules,
            &set("Silvally", "RKS System", &["Harden"], Some("Bug Memory")),
            |_| {},
        );
        assert_eq!(
            type_of(&engine, &rules),
            PokemonType::Bug,
            "{generation:?}: a Bug Memory makes Silvally Bug"
        );
        let bare = engine_for(
            generation,
            &rules,
            &set("Silvally", "RKS System", &["Harden"], None),
            |_| {},
        );
        assert_eq!(
            type_of(&bare, &rules),
            PokemonType::Normal,
            "{generation:?}: no Memory is Normal"
        );
        // Built as Silvally-Bug with no ability of its own.
        let foreign = engine_for(
            generation,
            &rules,
            &set("Silvally-Bug", "Intimidate", &["Harden"], None),
            |_| {},
        );
        let expected = if generation == Generation::Gen7 {
            PokemonType::Normal
        } else {
            PokemonType::Bug
        };
        assert_eq!(
            type_of(&foreign, &rules),
            expected,
            "{generation:?}: without RKS System the condition says Normal before Gen 8 \
             (`this.gen >= 8`), the forme's own type from Gen 8"
        );
    }
}

/// Arceus's condition is the same shape: Normal without Multitype before Gen 8 (Gen 4 its own
/// entry rule), the forme's own type from Gen 8.
#[test]
fn a_foreign_ability_gives_normal_to_arceus_before_gen_8() {
    for (generation, expected) in [
        (Generation::Gen5, PokemonType::Normal),
        (Generation::Gen7, PokemonType::Normal),
        (Generation::Gen9, PokemonType::Fire),
    ] {
        let rules = rules(generation);
        let engine = engine_for(
            generation,
            &rules,
            &set("Arceus-Fire", "Intimidate", &["Harden"], None),
            |_| {},
        );
        assert_eq!(type_of(&engine, &rules), expected, "{generation:?}");
    }
}

/// Gen 4's own condition: the entrant takes Normal first (`onBeforeSwitchIn`), which Multitype's
/// lock refuses, so a Multitype Arceus still comes in as its Plate's forme and type.
#[test]
fn gen_4_arceus_enters_normal_unless_multitype() {
    let rules = rules(Generation::Gen4);
    let engine = engine_for(
        Generation::Gen4,
        &rules,
        &set("Arceus-Fire", "Intimidate", &["Harden"], None),
        |_| {},
    );
    assert_eq!(type_of(&engine, &rules), PokemonType::Normal);
    let with_plate = engine_for(
        Generation::Gen4,
        &rules,
        &set(
            "Arceus-Fire",
            "Multitype",
            &["Judgment"],
            Some("Flame Plate"),
        ),
        |_| {},
    );
    assert_eq!(
        with_plate.state().pokemon(USER).unwrap().species,
        Some(SpeciesId::ARCEUS_FIRE),
        "Multitype refused the entry Normal and the Plate's forme stands"
    );
    assert_eq!(type_of(&with_plate, &rules), PokemonType::Fire);
}

/// The species condition reads the item as it is held (Showdown `pokemon.getItem()`): Embargo
/// does not take the type away. Multi-Attack, whose Memory event *is* an item handler, goes
/// back to Normal with it.
#[test]
fn embargo_leaves_the_type_but_not_the_move() {
    let generation = Generation::Gen7;
    let rules = rules(generation);
    let engine = engine_for(
        generation,
        &rules,
        &set(
            "Silvally",
            "RKS System",
            &["Multi-Attack"],
            Some("Bug Memory"),
        ),
        |state| {
            state.pokemon_mut(USER).unwrap().volatiles.set_embargo(3);
        },
    );
    assert_eq!(
        type_of(&engine, &rules),
        PokemonType::Bug,
        "the condition is the species', not the item's"
    );
    let dex = DexResolver::resolve(generation);
    let move_def = dex.move_by_id(MoveId::MULTI_ATTACK).unwrap().clone();
    let (modified, _) = pokemon_battle_engine::engine::modified_move(
        engine.state(),
        &rules,
        USER,
        &move_def,
        false,
    );
    assert_eq!(
        modified.move_type,
        PokemonType::Normal,
        "the Memory handler is suppressed with the item"
    );
}

/// A benched Pokémon has no ability, item or species handlers at all (Showdown
/// `findEventHandlers`): its types stay the species' own.
#[test]
fn the_bench_reads_no_memory() {
    let generation = Generation::Gen7;
    let rules = rules(generation);
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(
            &dex,
            USER,
            SideId::P1,
            &set("Silvally", "RKS System", &["Harden"], Some("Bug Memory")),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            FOE,
            SideId::P2,
            &set("Snorlax", "Thick Fat", &["Harden"], None),
        )
        .unwrap(),
    ];
    let mut p1 = SideState::new(vec![USER], 1);
    p1.active[0] = None;
    let mut p2 = SideState::new(vec![FOE], 1);
    p2.active[0] = Some(FOE);
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let holder = engine.state().pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::types_of(engine.state(), &rules, holder).primary,
        PokemonType::Normal,
        "off the field the base species' types stand"
    );
}

/// Multi-Attack and Techno Blast read the held Memory / Drive, on any holder (Showdown's event
/// asks the item, not the species), and keep their own type without one.
#[test]
fn the_moves_read_the_gift_on_any_holder() {
    let generation = Generation::Gen8;
    let rules = rules(generation);
    let dex = Arc::new(DexResolver::resolve(generation));
    let cases: &[(&str, &str, Option<&str>, MoveId, PokemonType)] = &[
        (
            "Silvally",
            "RKS System",
            Some("Fire Memory"),
            MoveId::MULTI_ATTACK,
            PokemonType::Fire,
        ),
        (
            "Snorlax",
            "Thick Fat",
            Some("Fire Memory"),
            MoveId::MULTI_ATTACK,
            PokemonType::Fire,
        ),
        (
            "Snorlax",
            "Thick Fat",
            None,
            MoveId::MULTI_ATTACK,
            PokemonType::Normal,
        ),
        (
            "Genesect",
            "Download",
            Some("Shock Drive"),
            MoveId::TECHNO_BLAST,
            PokemonType::Electric,
        ),
        (
            "Snorlax",
            "Thick Fat",
            Some("Shock Drive"),
            MoveId::TECHNO_BLAST,
            PokemonType::Electric,
        ),
        (
            "Genesect",
            "Download",
            None,
            MoveId::TECHNO_BLAST,
            PokemonType::Normal,
        ),
    ];
    for (species, ability, item, move_id, expected) in cases {
        let engine = engine_for(
            generation,
            &rules,
            &set(species, ability, &["Harden"], *item),
            |_| {},
        );
        let move_def = dex.move_by_id(*move_id).unwrap().clone();
        let (modified, _) = pokemon_battle_engine::engine::modified_move(
            engine.state(),
            &rules,
            USER,
            &move_def,
            false,
        );
        assert_eq!(
            modified.move_type, *expected,
            "{species} {item:?} {move_id:?}"
        );
    }
}

/// A Memory / Drive refuses the take from its species in any forme, and by its species in any
/// forme; anything else takes it.
#[test]
fn a_memory_or_drive_refuses_its_species() {
    let generation = Generation::Gen8;
    let rules = rules(generation);
    let dex = Arc::new(DexResolver::resolve(generation));
    let build = |id, species: &str| {
        build_pokemon(
            &dex,
            id,
            SideId::P1,
            &set(species, "Thick Fat", &["Harden"], Some("Bug Memory")),
        )
        .unwrap()
    };
    let state = BattleState::new(
        vec![
            build(PokemonId(1), "Silvally"),
            build(PokemonId(2), "Silvally-Bug"),
            build(PokemonId(3), "Snorlax"),
            build(PokemonId(4), "Blissey"),
        ],
        [
            SideState::new(vec![PokemonId(1)], 1),
            SideState::new(vec![PokemonId(3)], 1),
        ],
        1,
    );
    let refuses = |holder, taker| {
        pokemon_battle_engine::engine::item_refuses_take(
            &state,
            &rules,
            ItemId::BUG_MEMORY,
            holder,
            taker,
            None,
        )
    };
    assert!(refuses(PokemonId(1), PokemonId(3)), "off a Silvally");
    assert!(
        refuses(PokemonId(2), PokemonId(3)),
        "off a Silvally-Bug (any forme)"
    );
    assert!(refuses(PokemonId(3), PokemonId(1)), "by a Silvally");
    assert!(!refuses(PokemonId(3), PokemonId(4)), "not off a Snorlax");
    let drive = pokemon_battle_engine::engine::item_refuses_take(
        &state,
        &rules,
        ItemId::BURN_DRIVE,
        PokemonId(3),
        PokemonId(4),
        None,
    );
    assert!(!drive, "a Drive on a Snorlax is a plain item");
}

/// The snapshots' `memory` / `drive` data covers the callbacks (the dex resolves the items as
/// implemented), and the registry hands the type on.
#[test]
fn the_memories_and_drives_are_data_not_callbacks() {
    let generation = Generation::Gen7;
    let dex = DexResolver::resolve(generation);
    let memory = dex.item_by_name("Bug Memory").unwrap();
    assert_eq!(
        memory.memory,
        Some(PokemonType::Bug),
        "{:?}",
        memory.missing_callbacks
    );
    assert!(memory.implemented, "{:?}", memory.missing_callbacks);
    let drive = dex.item_by_name("Shock Drive").unwrap();
    assert_eq!(drive.drive, Some(PokemonType::Electric));
    assert!(drive.implemented, "{:?}", drive.missing_callbacks);
    let rules = rules(generation);
    let behavior = pokemon_battle_engine::engine::item_behavior(&rules, ItemId::BUG_MEMORY);
    assert_eq!(behavior.memory, Some(PokemonType::Bug));
    let behavior = pokemon_battle_engine::engine::item_behavior(&rules, ItemId::SHOCK_DRIVE);
    assert_eq!(behavior.drive, Some(PokemonType::Electric));
}
