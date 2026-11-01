//! L1 rule tests for Z-Moves (DESIGN.md §44.4): which crystal converts which move, what the
//! converted move is made of, what the side spends, and the exclusions the frames cannot tell
//! apart. The scenarios cover the damage; these pin the decisions.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, MoveCategory, PokemonSet, PokemonType, ZConversion, build_pokemon, z_conversion,
};
use pokemon_battle_engine::engine::{
    ActionChoice, ChoiceError, EngineStep, MechanicChoice, SideChoice, SlotChoice, z_move_execution,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleState, FieldConditionKind, MechanicUsage, SideState, VolatileConditionKind, WeatherKind,
};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

fn rules(generation: Generation) -> Arc<pokemon_battle_engine::rules::ResolvedRules> {
    Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    )
}

fn engine_with(user: PokemonSet, target: PokemonSet) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(Generation::Gen7));
    let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
    let pokemon = vec![
        build(USER, SideId::P1, user),
        build(TARGET, SideId::P2, target),
    ];
    let side = |id| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut engine = BattleEngine::try_new(
        rules(Generation::Gen7),
        dex,
        BattleState::new(pokemon, [side(USER), side(TARGET)], 1),
    )
    .unwrap();
    engine.run_lead_switch_ins();
    engine
}

fn set(species: &str, ability: &str, moves: &[&str], item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.to_vec());
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

#[test]
fn a_crystal_converts_what_the_data_says() {
    let dex = DexResolver::resolve(Generation::Gen7);
    let generic = |move_type| dex.z_move_of_type(move_type);
    let conversion = |item_name: &str, move_name: &str| {
        let item = dex.item_by_name(item_name).expect(item_name);
        let base = dex.move_by_name(move_name).expect(move_name);
        z_conversion(item, base, generic).map(|conversion| match conversion {
            ZConversion::Status => "self",
            ZConversion::Generic(id) => dex.move_by_id(id).unwrap().name.as_str(),
            ZConversion::Signature(id) => dex.move_by_id(id).unwrap().name.as_str(),
        })
    };
    assert_eq!(
        conversion("Electrium Z", "Thunderbolt"),
        Some("Gigavolt Havoc")
    );
    // A type crystal converts only its own type, and a status move into itself.
    assert_eq!(conversion("Electrium Z", "Tackle"), None);
    assert_eq!(conversion("Normalium Z", "Harden"), Some("self"));
    assert_eq!(conversion("Normalium Z", "Tackle"), Some("Breakneck Blitz"));
    // A signature crystal converts its one move.
    assert_eq!(
        conversion("Pikanium Z", "Volt Tackle"),
        Some("Catastropika")
    );
    assert_eq!(conversion("Pikanium Z", "Thunderbolt"), None);
    assert_eq!(
        conversion("Pikashunium Z", "Thunderbolt"),
        Some("10,000,000 Volt Thunderbolt")
    );
    // A Pokémon's own Z-Move is not converted again.
    assert_eq!(conversion("Electrium Z", "Gigavolt Havoc"), None);
}

#[test]
fn the_converted_move_takes_the_base_moves_power_category_and_priority() {
    let engine = engine_with(
        set("Pikachu", "Static", &["Thunderbolt"], Some("Electrium Z")),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let dex = engine.dex().clone();
    let base = dex.move_by_name("Thunderbolt").unwrap().clone();
    let z = z_move_execution(engine.state(), engine.rules(), &dex, USER, &base)
        .expect("Electrium Z converts Thunderbolt");
    let def = &z.move_def;
    assert_eq!(dex.move_by_id(def.id).unwrap().name, "Gigavolt Havoc");
    assert_eq!(def.base_power, 175);
    assert_eq!(def.category, MoveCategory::Special);
    assert!(z.powered);
    assert_eq!(z.base, base.id);
    assert!(z.status_effect.is_none());

    // A status move's Z version is the move itself, with the Z effect to run first.
    let engine = engine_with(
        set("Snorlax", "Thick Fat", &["Harden"], Some("Normalium Z")),
        set("Pikachu", "Static", &["Harden"], None),
    );
    let dex = engine.dex().clone();
    let base = dex.move_by_name("Harden").unwrap().clone();
    let z = z_move_execution(engine.state(), engine.rules(), &dex, USER, &base).unwrap();
    assert_eq!(z.move_def.id, base.id);
    assert!(z.status_effect.is_some());
    // An item the crystal does not belong to: none.
    let engine = engine_with(
        set("Snorlax", "Thick Fat", &["Harden"], Some("Normalium Z")),
        set("Pikachu", "Static", &["Harden"], None),
    );
    let dex = engine.dex().clone();
    let tackle = dex.move_by_name("Tackle").unwrap().clone();
    assert!(z_move_execution(engine.state(), engine.rules(), &dex, USER, &tackle).is_some());
    let none = dex.move_by_name("Thunderbolt").unwrap().clone();
    assert!(z_move_execution(engine.state(), engine.rules(), &dex, USER, &none).is_none());
}

#[test]
fn a_signature_crystal_belongs_to_its_species() {
    // Pikashunium Z lists the cosplay formes alone: a plain Pikachu cannot use it.
    let engine = engine_with(
        set("Pikachu", "Static", &["Thunderbolt"], Some("Pikashunium Z")),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let dex = engine.dex().clone();
    let base = dex.move_by_name("Thunderbolt").unwrap().clone();
    assert!(
        z_move_execution(engine.state(), engine.rules(), &dex, USER, &base).is_none(),
        "Pikachu is not a Pikachu-Original"
    );
    let engine = engine_with(
        set(
            "Pikachu-Original",
            "Static",
            &["Thunderbolt"],
            Some("Pikashunium Z"),
        ),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let dex = engine.dex().clone();
    let base = dex.move_by_name("Thunderbolt").unwrap().clone();
    assert!(z_move_execution(engine.state(), engine.rules(), &dex, USER, &base).is_some());
}

#[test]
fn mechanic_options_offer_the_z_moves_of_the_slots() {
    let engine = engine_with(
        set(
            "Pikachu",
            "Static",
            &["Thunderbolt", "Harden", "Tackle"],
            Some("Electrium Z"),
        ),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let options = engine.mechanic_options(USER);
    assert_eq!(options.z_moves.len(), 3);
    let name = |option: Option<pokemon_battle_engine::engine::ZMoveOption>| {
        option.map(|option| {
            engine
                .dex()
                .move_by_id(option.move_id)
                .unwrap()
                .name
                .clone()
        })
    };
    assert_eq!(name(options.z_moves[0]).as_deref(), Some("Gigavolt Havoc"));
    // A move of another type has none (Harden is Normal, the crystal Electric; Tackle too).
    assert_eq!(name(options.z_moves[1]), None);
    assert_eq!(name(options.z_moves[2]), None);

    // A Normalium Z: the Normal status move is its own Z version (Z-Harden).
    let engine = engine_with(
        set(
            "Pikachu",
            "Static",
            &["Harden", "Thunderbolt"],
            Some("Normalium Z"),
        ),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let options = engine.mechanic_options(USER);
    let harden = options.z_moves[0].expect("Z-Harden");
    assert_eq!(
        engine.dex().move_by_id(harden.move_id).unwrap().name,
        "Harden"
    );
    assert!(options.z_moves[1].is_none(), "Thunderbolt is not Normal");
    assert!(!options.is_empty());
}

#[test]
fn the_side_spends_its_z_move_once() {
    let mut engine = engine_with(
        set("Pikachu", "Static", &["Thunderbolt"], Some("Electrium Z")),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let z_choice = |actor| SideChoice {
        side: SideId::P1,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: None,
                mechanic: Some(MechanicChoice::ZMove),
            },
        }],
    };
    assert!(engine.validate_choice(SideId::P1, &z_choice(USER)).is_ok());
    let plain = SideChoice {
        side: SideId::P2,
        actions: vec![SlotChoice {
            actor: TARGET,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: None,
                mechanic: None,
            },
        }],
    };
    engine.submit_turn_choices([z_choice(USER), plain]).unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert_eq!(
        engine.state().side(SideId::P1).mechanics.z_move,
        MechanicUsage::Consumed
    );
    // A second Z-Move in the same battle is refused, and so is it once offered no longer.
    assert!(matches!(
        engine.validate_choice(SideId::P1, &z_choice(USER)),
        Err(ChoiceError::MechanicUnavailable(MechanicChoice::ZMove))
    ));
    assert!(
        engine
            .mechanic_options(USER)
            .z_moves
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn a_move_the_crystal_does_not_convert_is_refused() {
    let engine = engine_with(
        set("Pikachu", "Static", &["Tackle"], Some("Electrium Z")),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let choice = SideChoice {
        side: SideId::P1,
        actions: vec![SlotChoice {
            actor: USER,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: None,
                mechanic: Some(MechanicChoice::ZMove),
            },
        }],
    };
    assert!(matches!(
        engine.validate_choice(SideId::P1, &choice),
        Err(ChoiceError::MechanicNotApplicable {
            actor: USER,
            mechanic: MechanicChoice::ZMove
        })
    ));
}

#[test]
fn a_z_crystal_is_never_taken() {
    // `onTakeItem: false`: Knock Off takes nothing off the holder.
    let mut engine = engine_with(
        set("Pikachu", "Static", &["Thunderbolt"], Some("Electrium Z")),
        set("Snorlax", "Thick Fat", &["Knock Off"], None),
    );
    let move_of = |actor, slot| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot,
            target: None,
            mechanic: None,
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![move_of(USER, MoveSlot::new(0).unwrap())],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![move_of(TARGET, MoveSlot::new(0).unwrap())],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert!(
        engine.state().pokemon(USER).unwrap().held_item.is_some(),
        "the crystal stays"
    );
    assert!(engine.state().pokemon(TARGET).unwrap().held_item.is_none());
}

#[test]
fn a_z_move_is_not_retyped_and_not_doubled() {
    // The `-ate` abilities leave a Z-Move's type alone, and Parental Bond strikes once
    // (`move.isZOrMaxPowered`, both read as the move is prepared).
    let engine = engine_with(
        set("Sylveon", "Pixilate", &["Breakneck Blitz"], None),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let dex = engine.dex().clone();
    let base = dex.move_by_name("Breakneck Blitz").unwrap().clone();
    let modified = pokemon_battle_engine::engine::modified_move(
        engine.state(),
        engine.rules(),
        USER,
        &base,
        false,
    )
    .0;
    assert_eq!(
        modified.move_type,
        PokemonType::Normal,
        "Pixilate leaves it"
    );

    // Parental Bond: with a plain move it strikes twice, with a Z-powered one once.
    let engine = engine_with(
        set("Kangaskhan", "Parental Bond", &["Tackle"], None),
        set("Snorlax", "Thick Fat", &["Harden"], None),
    );
    let dex = engine.dex().clone();
    let tackle = dex.move_by_name("Tackle").unwrap().clone();
    let once = |state: &BattleState| {
        pokemon_battle_engine::engine::prepared_move(
            state,
            engine.rules(),
            USER,
            tackle.clone(),
            1,
            Some(TARGET),
        )
        .flags
        .parental_bond
    };
    assert!(once(engine.state()), "a plain move strikes twice");
    let mut snapshot = engine.snapshot();
    snapshot.state.active_move = Some(pokemon_battle_engine::state::ActiveMove {
        user: USER,
        move_id: tackle.id,
        category: tackle.category,
        ignores_ability: false,
        stellar_boosted: false,
        z_powered: true,
        base_move: None,
        prankster_boosted: false,
    });
    let z_powered_engine = BattleEngine::restore(
        rules(Generation::Gen7),
        Arc::new(engine.dex().clone()),
        snapshot,
    )
    .unwrap();
    assert!(
        !once(z_powered_engine.state()),
        "a Z-powered move strikes once"
    );
}

/// One turn: the user's first move, as its Z-Move when `z`; the target's first move.
fn one_turn(engine: &mut BattleEngine, z: bool) {
    let choice = |side, actor, mechanic| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: None,
                mechanic,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, USER, z.then_some(MechanicChoice::ZMove)),
            choice(SideId::P2, TARGET, None),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
}

#[test]
fn a_taunted_pokemon_uses_a_status_moves_z_version() {
    // Showdown's Taunt leaves a Z-Move alone (`!(move.isZ && move.isZOrMaxPowered)`), as Disable,
    // Imprison, Heal Block and Throat Chop do: a status move's Z version, which keeps the move's
    // own data, goes ahead (R-M10-41); the move itself does not.
    for z in [true, false] {
        let mut engine = engine_with(
            set("Snivy", "Overgrow", &["Sunny Day"], Some("Firium Z")),
            set("Pyroar", "Unnerve", &["Taunt"], None),
        );
        one_turn(&mut engine, z);
        let user = engine.state().pokemon(USER).unwrap();
        assert!(user.volatiles.contains(VolatileConditionKind::Taunt));
        assert_eq!(
            engine.state().field.weather.map(|weather| weather.kind),
            z.then_some(WeatherKind::Sun),
            "Z: {z}"
        );
        assert_eq!(user.stages.speed, i8::from(z), "Z: {z}");
    }
}

#[test]
fn a_z_move_goes_ahead_under_gravity() {
    // Gravity refuses a move with its flag but a Z-Move (`move.flags['gravity'] && !move.isZ`,
    // R-M10-41): Z-Splash's +3 Attack, Splash itself refused.
    for z in [true, false] {
        let mut engine = engine_with(
            set("Snorlax", "Thick Fat", &["Splash"], Some("Normalium Z")),
            set("Alakazam", "Synchronize", &["Gravity"], None),
        );
        one_turn(&mut engine, z);
        assert!(engine.state().field.contains(FieldConditionKind::Gravity));
        assert_eq!(
            engine.state().pokemon(USER).unwrap().stages.attack,
            if z { 3 } else { 0 },
            "Z: {z}"
        );
    }
}

#[test]
fn snatch_leaves_a_z_move_alone() {
    // Snatch's `onAnyPrepareHit` passes over a Z-Move (`move.isZ || move.isMax`, R-M10-43):
    // Z-Tailwind blows on its user's side; Tailwind itself is snatched.
    for z in [true, false] {
        let mut engine = engine_with(
            set("Pidgeot", "Keen Eye", &["Tailwind"], Some("Flyinium Z")),
            set("Murkrow", "Insomnia", &["Snatch"], None),
        );
        one_turn(&mut engine, z);
        let blows = |side| {
            engine
                .state()
                .side(side)
                .conditions
                .tailwind_turns()
                .is_some()
        };
        assert_eq!((blows(SideId::P1), blows(SideId::P2)), (z, !z), "Z: {z}");
    }
}
