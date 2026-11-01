//! L1 rule tests for Terastallization (DESIGN.md §44.1, §44.6): the types it reads, the STAB it
//! gives, the 60-power floor, the two type-changing moves and what a Terastallized Pokémon
//! refuses. The frames show the types and the damage, so these pin the parts a replay cannot
//! tell apart (which of two equal modifiers applied, whether the added type went).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, MoveCategory, PokemonSet, PokemonType, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, MechanicChoice, SideChoice, SlotChoice,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{Fraction, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, Stat};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

fn rules() -> Arc<pokemon_battle_engine::rules::ResolvedRules> {
    Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    )
}

fn engine_with(user: PokemonSet, target: PokemonSet) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
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
        rules(),
        dex,
        BattleState::new(pokemon, [side(USER), side(TARGET)], 1),
    )
    .unwrap();
    engine.run_lead_switch_ins();
    engine
}

fn set(species: &str, ability: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.to_vec());
    set.ability = Some(ability.into());
    set.level = 50;
    set
}

#[test]
fn the_types_a_terastallized_pokemon_reads() {
    let rules = rules();
    let engine = engine_with(
        set("Charizard", "Blaze", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let snapshot = engine.snapshot();
    let mut state = snapshot.state.clone();
    // A Charizard (Fire/Flying) with an added Ghost type, Terastallized into Electric.
    {
        let pokemon = state.pokemon_mut(USER).unwrap();
        pokemon.added_type = Some(PokemonType::Ghost);
        pokemon.terastallized = Some(PokemonType::Electric);
    }
    let charizard = state.pokemon(USER).unwrap();
    // `getTypes`: the Tera type alone — no base type, no added type.
    let types = pokemon_battle_engine::engine::types_of(&state, &rules, charizard);
    assert_eq!(types.primary, PokemonType::Electric);
    assert_eq!(types.secondary, None);
    assert_eq!(types.added, None);
    assert_eq!(
        pokemon_battle_engine::engine::pre_tera_types_of(&state, &rules, charizard)
            .iter()
            .collect::<Vec<_>>(),
        vec![PokemonType::Fire, PokemonType::Flying, PokemonType::Ghost]
    );
    // Stellar is not a type of its own: the base types and the added one stay.
    {
        let pokemon = state.pokemon_mut(USER).unwrap();
        pokemon.terastallized = Some(PokemonType::Stellar);
    }
    let charizard = state.pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::types_of(&state, &rules, charizard)
            .iter()
            .collect::<Vec<_>>(),
        vec![PokemonType::Fire, PokemonType::Flying, PokemonType::Ghost]
    );
}

#[test]
fn stab_of_a_terastallized_pokemon() {
    let rules = rules();
    let engine = engine_with(
        set("Charizard", "Blaze", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let snapshot = engine.snapshot();
    let mut state = snapshot.state.clone();
    let stab = |state: &BattleState, move_type| {
        pokemon_battle_engine::engine::stab_modifier(
            state,
            &rules,
            state.pokemon(USER).unwrap(),
            move_type,
            false,
        )
        .modifier
    };
    let plain = rules.damage().stab_modifier;
    // Fire (a base type) and Flying (the other): the generation's STAB.
    assert_eq!(stab(&state, PokemonType::Fire), plain);
    assert_eq!(stab(&state, PokemonType::Flying), plain);
    // Water: not a type it has.
    assert_eq!(stab(&state, PokemonType::Water), Fraction::ONE);
    // Terastallized into Electric: Electric has the plain STAB (`hasType`), not the doubled one
    // — `terastallized === type` alone is not enough, the type must also be one of its own
    // (`getTypes(false, true)`). Fire and Flying keep theirs, Water has none.
    state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Electric);
    assert_eq!(stab(&state, PokemonType::Electric), plain);
    assert_eq!(stab(&state, PokemonType::Fire), plain);
    assert_eq!(stab(&state, PokemonType::Flying), plain);
    assert_eq!(stab(&state, PokemonType::Water), Fraction::ONE);
    // Terastallized into Fire (a base type): 2, as `terastallized === type` with the type also
    // in `getTypes(false, true)`.
    state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Fire);
    assert_eq!(stab(&state, PokemonType::Fire), Fraction::new(2, 1));
    assert_eq!(stab(&state, PokemonType::Flying), plain);
    // Stellar: 2 for a base type, 4915/4096 for another, once per type; then the plain STAB
    // again for that type.
    state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Stellar);
    assert_eq!(stab(&state, PokemonType::Fire), Fraction::new(2, 1));
    assert_eq!(
        stab(&state, PokemonType::Electric),
        Fraction::new(4915, 4096)
    );
    state.pokemon_mut(USER).unwrap().stellar_boosted_types =
        vec![PokemonType::Fire, PokemonType::Electric];
    assert_eq!(stab(&state, PokemonType::Fire), plain);
    assert_eq!(stab(&state, PokemonType::Electric), Fraction::ONE);
}

#[test]
fn adaptability_only_boosts_a_type_it_has_now() {
    let rules = rules();
    let engine = engine_with(
        set("Charizard", "Adaptability", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let snapshot = engine.snapshot();
    let mut state = snapshot.state.clone();
    let stab = |state: &BattleState, move_type| {
        pokemon_battle_engine::engine::stab_modifier(
            state,
            &rules,
            state.pokemon(USER).unwrap(),
            move_type,
            false,
        )
        .modifier
    };
    // Not Terastallized: a type it has → 2 (Adaptability's `onModifySTAB`).
    assert_eq!(stab(&state, PokemonType::Fire), Fraction::new(2, 1));
    // Terastallized into Fire: the Tera doubling (2) becomes 2.25.
    state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Fire);
    assert_eq!(stab(&state, PokemonType::Fire), Fraction::new(9, 4));
    state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Electric);
    // Electric: the plain STAB, which Adaptability takes to 2 (`hasType` is true).
    assert_eq!(stab(&state, PokemonType::Electric), Fraction::new(2, 1));
    // Fire: still a base-type STAB, but `hasType` is false now, so Adaptability leaves it.
    assert_eq!(
        stab(&state, PokemonType::Fire),
        rules.damage().stab_modifier
    );
}

#[test]
fn the_sixty_power_floor_of_a_terastallized_move() {
    // Tackle (Normal, 40 power) and Metal Claw (Steel, 50) become 60 under a matching
    // Terastallization, so their base damage is the same; without it, they differ. Quick
    // Attack (priority +1) is exempt from the floor.
    let base_damage = |tera: Option<PokemonType>, moves: &[&str], slot: u8| -> u16 {
        let engine = engine_with(
            set("Snorlax", "Thick Fat", moves),
            set("Blissey", "Natural Cure", &["Harden"]),
        );
        let mut snapshot = engine.snapshot();
        snapshot.state.pokemon_mut(USER).unwrap().terastallized = tera;
        let engine =
            BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
        fight_once(engine, MoveSlot::new(slot).unwrap())
    };
    let tackle = |tera| base_damage(tera, &["Tackle"], 0);
    let metal_claw = |tera| base_damage(tera, &["Metal Claw"], 0);
    assert!(
        tackle(None) < metal_claw(None),
        "40 power deals less than 50"
    );
    assert_eq!(
        tackle(Some(PokemonType::Normal)),
        metal_claw(Some(PokemonType::Steel)),
        "both floored to 60"
    );
    assert!(
        tackle(Some(PokemonType::Normal)) > tackle(None),
        "the floor raised it"
    );
    // A non-matching Tera type leaves the power alone, and so does a priority move.
    assert_eq!(tackle(Some(PokemonType::Electric)), tackle(None));
    assert_eq!(
        base_damage(Some(PokemonType::Normal), &["Quick Attack"], 0),
        base_damage(None, &["Quick Attack"], 0)
    );
    // A multi-hit move is exempt (`!dexMove.multihit`): Fury Swipes stays at 18.
    assert_eq!(
        base_damage(Some(PokemonType::Normal), &["Fury Swipes"], 0),
        base_damage(None, &["Fury Swipes"], 0)
    );
}

/// One turn of the user's move against a Harden-ing target; the base damage the damage was
/// computed from (before STAB, effectiveness and the roll).
fn fight_once(mut engine: BattleEngine, slot: MoveSlot) -> u16 {
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
                actions: vec![move_of(USER, slot)],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![move_of(TARGET, MoveSlot::new(0).unwrap())],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    engine
        .trace()
        .events()
        .iter()
        .find_map(|event| match event {
            pokemon_battle_engine::engine::TraceEvent::DamageCalculated { base_damage, .. } => {
                Some(*base_damage)
            }
            _ => None,
        })
        .expect("the move dealt damage")
}

#[test]
fn a_variable_power_move_is_exempt_from_the_floor() {
    // Eruption's data leaves 150 power open (`basePowerCallback` scales it by HP), which
    // Showdown exempts by name: at 1 HP its power is far below 60 and the floor must not raise
    // it back.
    let base_damage = |tera: Option<PokemonType>| -> u16 {
        let engine = engine_with(
            set("Snorlax", "Thick Fat", &["Eruption"]),
            set("Blissey", "Natural Cure", &["Harden"]),
        );
        let mut snapshot = engine.snapshot();
        {
            let user = snapshot.state.pokemon_mut(USER).unwrap();
            user.terastallized = tera;
            user.hp = 1;
        }
        let engine =
            BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
        fight_once(engine, MoveSlot::new(0).unwrap())
    };
    assert_eq!(base_damage(Some(PokemonType::Fire)), base_damage(None));
}

#[test]
fn tera_blast_takes_the_tera_type_and_the_better_stat() {
    let rules = rules();
    // Charizard's Attack is far below its Special Attack; a Terastallized Tera Blast stays
    // special and takes the Tera type.
    let engine = engine_with(
        set("Charizard", "Blaze", &["Tera Blast"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let snapshot = engine.snapshot();
    let mut state = snapshot.state.clone();
    let dex = engine.dex().clone();
    state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Electric);
    let modified = pokemon_battle_engine::engine::modified_move(
        &state,
        &rules,
        USER,
        dex.move_by_name("Tera Blast").unwrap(),
        false,
    )
    .0;
    assert_eq!(modified.move_type, PokemonType::Electric);
    assert_eq!(modified.category, MoveCategory::Special);
    assert_eq!(modified.base_power, 80);
    // With the Attack stat higher (a hand-made state), it is physical.
    let mut state = state.clone();
    state.pokemon_mut(USER).unwrap().stats.attack = 999;
    let modified = pokemon_battle_engine::engine::modified_move(
        &state,
        &rules,
        USER,
        dex.move_by_name("Tera Blast").unwrap(),
        false,
    )
    .0;
    assert_eq!(modified.category, MoveCategory::Physical);
    // Stellar: 100 power, and the user's Attack and Special Attack each drop by one.
    state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Stellar);
    let modified = pokemon_battle_engine::engine::modified_move(
        &state,
        &rules,
        USER,
        dex.move_by_name("Tera Blast").unwrap(),
        false,
    )
    .0;
    assert_eq!(modified.base_power, 100);
    let drops: Vec<(Stat, i8)> = modified
        .secondaries
        .iter()
        .filter_map(|secondary| match secondary.effect {
            pokemon_battle_engine::dex::SecondaryEffect::StatStage { stat, delta } => {
                Some((stat, delta))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        drops,
        vec![(Stat::Attack, -1), (Stat::SpecialAttack, -1)],
        "both drops, in order"
    );
    // Not Terastallized: a plain Normal special move.
    state.pokemon_mut(USER).unwrap().terastallized = None;
    let modified = pokemon_battle_engine::engine::modified_move(
        &state,
        &rules,
        USER,
        dex.move_by_name("Tera Blast").unwrap(),
        false,
    )
    .0;
    assert_eq!(modified.move_type, PokemonType::Normal);
    assert_eq!(modified.base_power, 80);
    assert!(modified.secondaries.is_empty());
}

#[test]
fn tera_blast_keeps_its_type_from_a_normal_type_ability() {
    // Pixilate (`onModifyType`) leaves a Terastallized Tera Blast alone.
    let rules = rules();
    let engine = engine_with(
        set("Charizard", "Pixilate", &["Tera Blast"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let dex = engine.dex().clone();
    let snapshot = engine.snapshot();
    let mut state = snapshot.state.clone();
    let move_def = dex.move_by_name("Tera Blast").unwrap();
    let plain =
        pokemon_battle_engine::engine::modified_move(&state, &rules, USER, move_def, false).0;
    assert_eq!(plain.move_type, PokemonType::Fairy, "Pixilate changes it");
    state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Electric);
    let tera =
        pokemon_battle_engine::engine::modified_move(&state, &rules, USER, move_def, false).0;
    assert_eq!(tera.move_type, PokemonType::Electric);
}

#[test]
fn a_terastallized_pokemon_refuses_a_type_change_and_clears_the_added_type() {
    // Soak (Water) on a Terastallized target changes nothing; Terastallizing clears the added
    // type Trick-or-Treat left (no Gen 9 species learns it, so the state is set by hand).
    let engine = engine_with(
        set("Slowbro", "Oblivious", &["Soak"]),
        set("Charizard", "Blaze", &["Harden"]),
    );
    let mut snapshot = engine.snapshot();
    {
        let target = snapshot.state.pokemon_mut(TARGET).unwrap();
        target.terastallized = Some(PokemonType::Electric);
        target.added_type = Some(PokemonType::Ghost);
    }
    let mut engine =
        BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
    let start = engine.trace().events().len();
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
        !engine.trace().events()[start..]
            .iter()
            .any(|event| matches!(
                event,
                pokemon_battle_engine::engine::TraceEvent::TypesChanged {
                    pokemon: TARGET,
                    ..
                }
            )),
        "setType refuses a Terastallized Pokémon"
    );

    // Terastallizing itself: the added type goes, the Tera type is the set's.
    let engine = engine_with(
        set("Charizard", "Blaze", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let mut snapshot = engine.snapshot();
    {
        let user = snapshot.state.pokemon_mut(USER).unwrap();
        user.added_type = Some(PokemonType::Ghost);
        user.tera_type = PokemonType::Water;
    }
    let mut engine =
        BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
    let tera = |actor| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: Some(MechanicChoice::Terastallize),
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![tera(USER)],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: TARGET,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: None,
                    },
                }],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let user = engine.state().pokemon(USER).unwrap();
    assert_eq!(user.terastallized, Some(PokemonType::Water));
    assert_eq!(user.added_type, None);
    assert_eq!(
        pokemon_battle_engine::engine::types_of(engine.state(), &rules(), user)
            .iter()
            .collect::<Vec<_>>(),
        vec![PokemonType::Water]
    );
    assert_eq!(
        engine.state().side(SideId::P1).mechanics.tera,
        pokemon_battle_engine::state::MechanicUsage::Consumed
    );
}

#[test]
fn the_tera_type_defaults_to_the_first_type() {
    let engine = engine_with(
        set("Charizard", "Blaze", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    assert_eq!(
        engine.state().pokemon(USER).unwrap().tera_type,
        PokemonType::Fire
    );
    let mut blazing = set("Charizard", "Blaze", &["Harden"]);
    blazing.tera_type = Some(PokemonType::Stellar);
    let engine = engine_with(blazing, set("Snorlax", "Thick Fat", &["Harden"]));
    assert_eq!(
        engine.state().pokemon(USER).unwrap().tera_type,
        PokemonType::Stellar
    );
}

#[test]
fn a_terastallized_pokemon_is_hit_super_effectively_by_a_stellar_move() {
    // Tera Starstorm's Stellar form is super effective against a Terastallized target whatever
    // its types (`runEffectiveness`); against anyone else it is neutral.
    let engine = engine_with(
        set("Terapagos-Stellar", "Teraform Zero", &["Tera Starstorm"]),
        set("Charizard", "Blaze", &["Harden"]),
    );
    let mut snapshot = engine.snapshot();
    snapshot.state.pokemon_mut(TARGET).unwrap().terastallized = Some(PokemonType::Fire);
    let mut engine =
        BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
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
    let effectiveness = engine
        .trace()
        .events()
        .iter()
        .find_map(|event| match event {
            pokemon_battle_engine::engine::TraceEvent::TypeEffectivenessResolved {
                modifier,
                ..
            } => Some(*modifier),
            _ => None,
        })
        .expect("the move's effectiveness");
    assert_eq!(effectiveness, Fraction::new(2, 1));
}

#[test]
fn tera_starstorm_leaves_another_species_alone() {
    // The Stellar type and the all-adjacent-foes target belong to Terapagos-Stellar; anyone
    // else's Tera Starstorm stays a Normal single-target special move.
    let rules = rules();
    let engine = engine_with(
        set("Dragonite", "Inner Focus", &["Tera Starstorm"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let dex = engine.dex().clone();
    let mut snapshot = engine.snapshot();
    snapshot.state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Stellar);
    let modified = pokemon_battle_engine::engine::modified_move(
        &snapshot.state,
        &rules,
        USER,
        dex.move_by_name("Tera Starstorm").unwrap(),
        false,
    )
    .0;
    assert_eq!(modified.move_type, PokemonType::Normal);
    assert!(!modified.is_spread);
}

#[test]
fn a_second_terastallization_is_refused() {
    let engine = engine_with(
        set("Charizard", "Blaze", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let mut snapshot = engine.snapshot();
    snapshot.state.pokemon_mut(USER).unwrap().terastallized = Some(PokemonType::Fire);
    let engine = BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
    let choice = SideChoice {
        side: SideId::P1,
        actions: vec![SlotChoice {
            actor: USER,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: None,
                mechanic: Some(MechanicChoice::Terastallize),
            },
        }],
    };
    assert!(matches!(
        engine.validate_choice(SideId::P1, &choice),
        Err(pokemon_battle_engine::engine::ChoiceError::MechanicNotApplicable { actor: USER, .. })
    ));
}

#[test]
fn the_tera_step_orders_before_every_move() {
    // The slower Snorlax's Terastallization still runs before the faster Charizard's move: the
    // step's own action priority, whatever the speeds (Showdown `resolveAction`'s order 106).
    let mut engine = engine_with(
        set("Snorlax", "Thick Fat", &["Tackle"]),
        set("Charizard", "Blaze", &["Quick Attack"]),
    );
    let tera = SlotChoice {
        actor: USER,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: Some(MechanicChoice::Terastallize),
        },
    };
    let attack = SlotChoice {
        actor: TARGET,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![tera],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![attack],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let order: Vec<PokemonId> = engine
        .trace()
        .events()
        .iter()
        .filter_map(|event| match event {
            pokemon_battle_engine::engine::TraceEvent::BeforeMoveResolved { actor, .. } => {
                Some(*actor)
            }
            _ => None,
        })
        .collect();
    // The Tera step itself resolves no BeforeMove; the move order is still by speed.
    assert_eq!(order, vec![TARGET, USER]);
    let tera_at = engine
        .trace()
        .events()
        .iter()
        .position(|event| {
            matches!(
                event,
                pokemon_battle_engine::engine::TraceEvent::Terastallized { pokemon: USER, .. }
            )
        })
        .expect("Terastallization ran");
    let first_move_at = engine
        .trace()
        .events()
        .iter()
        .position(|event| {
            matches!(
                event,
                pokemon_battle_engine::engine::TraceEvent::MoveUsed { actor, .. } if *actor == TARGET
            )
        })
        .expect("the faster Pokémon's move ran");
    assert!(tera_at < first_move_at, "the step comes first");
    // A Charizard's Quick Attack is not a same-type move for a Normal Terastallization.
    assert_eq!(
        engine.state().pokemon(USER).unwrap().terastallized,
        Some(PokemonType::Normal)
    );
}

#[test]
fn mechanic_options_offer_only_what_the_rules_allow() {
    // The user's Q3 answer: a read-only view of the gimmicks a Pokémon may ask for. Terastallize
    // is implemented; the rest wait for their batches, so their fields stay empty.
    let mut terastaller = set("Charizard", "Blaze", &["Harden"]);
    terastaller.tera_type = Some(PokemonType::Electric);
    let engine = engine_with(terastaller, set("Snorlax", "Thick Fat", &["Harden"]));
    let options = engine.mechanic_options(USER);
    assert_eq!(options.tera, Some(PokemonType::Electric));
    assert!(options.mega.is_none() && options.ultra_burst.is_none());
    assert!(!options.dynamax && options.z_moves.iter().all(Option::is_none));
    assert!(!options.is_empty());

    // Gen 8 has no Terastallization: nothing is offered.
    let engine = engine_with(
        set("Charizard", "Blaze", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let mut snapshot = engine.snapshot();
    snapshot.rules_fingerprint = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen8,
        BattleMode::Singles,
        "custom",
    ))
    .unwrap()
    .fingerprint()
    .0
    .clone();
    let gen8 = BattleEngine::restore(
        Arc::new(
            RulesResolver::resolve(&BattleSpec::new(
                Generation::Gen8,
                BattleMode::Singles,
                "custom",
            ))
            .unwrap(),
        ),
        Arc::new(DexResolver::resolve(Generation::Gen8)),
        snapshot,
    );
    if let Ok(gen8) = gen8 {
        assert!(gen8.mechanic_options(USER).tera.is_none());
    }

    // A Pokémon that is not on the field is offered nothing.
    let engine = engine_with(
        set("Charizard", "Blaze", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    assert_eq!(
        engine.mechanic_options(pokemon_battle_engine::ids::PokemonId(99)),
        pokemon_battle_engine::engine::MechanicOptions::default()
    );
    assert!(!engine.mechanic_options(USER).is_empty());

    // Once the side has Terastallized, nothing more is offered.
    let mut engine = engine_with(
        set("Charizard", "Blaze", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let tera = SlotChoice {
        actor: USER,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: Some(MechanicChoice::Terastallize),
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![tera],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: TARGET,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: None,
                    },
                }],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert!(engine.mechanic_options(USER).tera.is_none());
}

// --- O.2c: Tera Shift, the Ogerpon `-Tera` formes and Embody Aspect (DESIGN.md §44.6) ---------

/// Tera Shift changes a Terapagos that comes in into its Terastal forme (and only a Terapagos).
#[test]
fn tera_shift_turns_terapagos_terastal_on_entry() {
    use pokemon_battle_engine::engine::{TraceEvent, raw_ability};
    use pokemon_battle_engine::ids::{AbilityId, SpeciesId};
    let engine = engine_with(
        set("Terapagos", "Tera Shift", &["Harden"]),
        set("Snorlax", "Tera Shift", &["Harden"]),
    );
    // The leads already switched in (`engine_with`), so the ability has run.
    let holder = engine.state().pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::species_of(holder),
        Some(SpeciesId::TERAPAGOS_TERASTAL)
    );
    assert_eq!(raw_ability(holder), AbilityId::TERA_SHELL);
    assert!(engine.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::AbilityActivated {
            source,
            ability: AbilityId::TERA_SHIFT
        } if *source == USER
    )));
    // The Snorlax with the same ability is no Terapagos: nothing changes, and it does not
    // announce the ability either.
    let other = engine.state().pokemon(TARGET).unwrap();
    assert_eq!(raw_ability(other), AbilityId::TERA_SHIFT);
    assert_eq!(
        pokemon_battle_engine::engine::species_of(other),
        Some(SpeciesId::SNORLAX)
    );
    assert!(!engine.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::AbilityActivated {
            source,
            ability: AbilityId::TERA_SHIFT
        } if *source != USER
    )));

    // A Pokémon that *is* in its Terastal forme already (a set may name the forme) announces
    // nothing either.
    let engine = engine_with(
        set("Terapagos-Terastal", "Tera Shift", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let holder = engine.state().pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::species_of(holder),
        Some(SpeciesId::TERAPAGOS_TERASTAL)
    );
    assert!(!engine.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::AbilityActivated {
            ability: AbilityId::TERA_SHIFT,
            ..
        }
    )));
}

/// An Ogerpon of a Tera type that is none of its masks' does nothing at all — its side's
/// Terastallization is not spent either.
#[test]
fn an_ogerpon_of_another_tera_type_refuses_terastallization() {
    use pokemon_battle_engine::ids::SpeciesId;
    use pokemon_battle_engine::state::MechanicUsage;
    let engine = engine_with(
        set("Ogerpon", "Defiant", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let mut snapshot = engine.snapshot();
    snapshot.state.pokemon_mut(USER).unwrap().tera_type = PokemonType::Electric;
    let mut engine =
        BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
    let tera = SlotChoice {
        actor: USER,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: Some(MechanicChoice::Terastallize),
        },
    };
    let plain = SlotChoice {
        actor: TARGET,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![tera],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![plain],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let holder = engine.state().pokemon(USER).unwrap();
    assert!(holder.terastallized.is_none(), "nothing happened");
    assert_eq!(
        pokemon_battle_engine::engine::species_of(holder),
        Some(SpeciesId::OGERPON)
    );
    assert_eq!(
        engine.state().side(SideId::P1).mechanics.tera,
        MechanicUsage::Available,
        "the side's Terastallization is not spent"
    );
}

/// Terastallizing an Ogerpon: the `-Tera` forme, its Embody Aspect, and the stat it raises.
#[test]
fn ogerpon_takes_its_tera_forme_and_embody_aspect_raises() {
    use pokemon_battle_engine::engine::raw_ability;
    use pokemon_battle_engine::ids::{AbilityId, SpeciesId};
    let mut engine = engine_with(
        set("Ogerpon-Wellspring", "Water Absorb", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: USER,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: Some(MechanicChoice::Terastallize),
                    },
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: TARGET,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: None,
                    },
                }],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let holder = engine.state().pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::species_of(holder),
        Some(SpeciesId::OGERPON_WELLSPRING_TERA)
    );
    assert_eq!(raw_ability(holder), AbilityId::EMBODY_ASPECT_WELLSPRING);
    assert_eq!(
        holder.stages.special_defense, 1,
        "Embody Aspect's own raise"
    );
    assert_eq!(holder.stages.attack, 0);
}

/// Embody Aspect's two guards: the forme and the Terastallization (and its `embodied` mark).
#[test]
fn embody_aspect_needs_the_forme_and_the_terastallization() {
    use pokemon_battle_engine::ids::SpeciesId;
    let tera_speed = |engine: BattleEngine, terastallized: Option<PokemonType>| {
        let mut snapshot = engine.snapshot();
        {
            let holder = snapshot.state.pokemon_mut(USER).unwrap();
            holder.species = Some(SpeciesId::OGERPON_TEAL_TERA);
            holder.ability = pokemon_battle_engine::ids::AbilityId::EMBODY_ASPECT_TEAL;
            holder.terastallized = terastallized;
        }
        let mut engine =
            BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
        // The ability starts as the lead does: its `onStart` is this entry.
        engine.run_lead_switch_ins();
        engine.state().pokemon(USER).unwrap().stages.speed
    };
    let build = || {
        engine_with(
            set("Ogerpon", "Defiant", &["Harden"]),
            set("Snorlax", "Thick Fat", &["Harden"]),
        )
    };
    assert_eq!(
        tera_speed(build(), Some(PokemonType::Grass)),
        1,
        "the Tera forme raises its stat as it stands"
    );
    assert_eq!(
        tera_speed(build(), None),
        0,
        "not Terastallized: nothing is raised"
    );
    let tera_speed_wrong_forme = |engine: BattleEngine| {
        let mut snapshot = engine.snapshot();
        {
            let holder = snapshot.state.pokemon_mut(USER).unwrap();
            holder.ability = pokemon_battle_engine::ids::AbilityId::EMBODY_ASPECT_TEAL;
            holder.terastallized = Some(PokemonType::Grass);
        }
        let mut engine =
            BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
        engine.run_lead_switch_ins();
        engine.state().pokemon(USER).unwrap().stages.speed
    };
    assert_eq!(
        tera_speed_wrong_forme(build()),
        0,
        "the base Ogerpon forme is not the one Embody Aspect belongs to"
    );
}

/// Embody Aspect's `embodied` mark: an ability start that is not a new `setAbility` (how
/// Neutralizing Gas clears) raises nothing again.
#[test]
fn embody_aspect_raises_once_per_ability_start() {
    use pokemon_battle_engine::ids::SpeciesId;
    let engine = engine_with(
        set("Ogerpon", "Defiant", &["Harden"]),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    let mut snapshot = engine.snapshot();
    {
        let holder = snapshot.state.pokemon_mut(USER).unwrap();
        holder.species = Some(SpeciesId::OGERPON_TEAL_TERA);
        holder.ability = pokemon_battle_engine::ids::AbilityId::EMBODY_ASPECT_TEAL;
        holder.terastallized = Some(PokemonType::Grass);
        holder.ability_state.embodied = true;
    }
    let mut engine =
        BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
    engine.run_lead_switch_ins();
    assert_eq!(
        engine.state().pokemon(USER).unwrap().stages.speed,
        0,
        "the mark holds the raise back"
    );
}

/// An Illusion wearing an Ogerpon's or Terapagos's face ends as the holder Terastallizes.
#[test]
fn an_illusion_of_an_ogerpon_ends_at_the_terastallization() {
    use pokemon_battle_engine::engine::{TraceEvent, raw_ability};
    use pokemon_battle_engine::ids::{AbilityId, SpeciesId};
    const BENCH: PokemonId = PokemonId(3);
    let engine = {
        let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
        let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
        let pokemon = vec![
            build(
                USER,
                SideId::P1,
                set("Zoroark", "Illusion", &["Harden", "Tackle"]),
            ),
            build(TARGET, SideId::P2, set("Snorlax", "Thick Fat", &["Harden"])),
            build(BENCH, SideId::P1, set("Ogerpon", "Defiant", &["Harden"])),
        ];
        let mut side = SideState::new(vec![USER, BENCH], 1);
        side.active[0] = Some(USER);
        let mut foe = SideState::new(vec![TARGET], 1);
        foe.active[0] = Some(TARGET);
        let mut engine =
            BattleEngine::try_new(rules(), dex, BattleState::new(pokemon, [side, foe], 1)).unwrap();
        engine.run_lead_switch_ins();
        engine
    };
    // The disguise is the Ogerpon on the bench.
    assert_eq!(
        engine.state().pokemon(USER).unwrap().illusion,
        Some(BENCH),
        "Zoroark wears the Ogerpon's face"
    );
    assert_eq!(
        pokemon_battle_engine::engine::species_of(engine.state().pokemon(BENCH).unwrap()),
        Some(SpeciesId::OGERPON)
    );
    let mut engine = engine;
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: USER,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: Some(MechanicChoice::Terastallize),
                    },
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: TARGET,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: None,
                    },
                }],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert!(engine.state().pokemon(USER).unwrap().illusion.is_none());
    assert!(engine.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::IllusionEnded { holder } if *holder == USER
    )));
    assert_eq!(
        raw_ability(engine.state().pokemon(USER).unwrap()),
        AbilityId::ILLUSION
    );
}

/// Showdown `faint`'s `formeRegression` for a `-Tera` forme (DESIGN.md §44.6, R-O.9): a fainted
/// Ogerpon goes back to the species its set named.
#[test]
fn a_fainted_tera_forme_goes_back_to_the_sets_species() {
    use pokemon_battle_engine::engine::{raw_ability, species_of};
    use pokemon_battle_engine::ids::{AbilityId, SpeciesId};
    let engine = engine_with(
        set("Ogerpon", "Defiant", &["Harden", "Tackle"]),
        set("Snorlax", "Thick Fat", &["Tackle"]),
    );
    let mut snapshot = engine.snapshot();
    {
        let holder = snapshot.state.pokemon_mut(USER).unwrap();
        holder.hp = 1;
        holder.tera_type = PokemonType::Grass;
    }
    let mut engine =
        BattleEngine::restore(rules(), Arc::new(engine.dex().clone()), snapshot).unwrap();
    let tera = SlotChoice {
        actor: USER,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: Some(MechanicChoice::Terastallize),
        },
    };
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![tera],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: TARGET,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: None,
                    },
                }],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let holder = engine.state().pokemon(USER).unwrap();
    assert!(holder.is_fainted());
    assert_eq!(species_of(holder), Some(SpeciesId::OGERPON));
    assert_eq!(raw_ability(holder), AbilityId::DEFIANT);
    assert!(
        holder.terastallized.is_none(),
        "a fainted Pokémon is not Terastallized"
    );
}
