//! L1 rule tests for Dynamax (DESIGN.md §44.5): the HP it doubles, the Max Moves it makes of the
//! slots, what ends it, and the traits a frame cannot tell apart.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, MoveCategory, PokemonSet, PokemonType, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, MechanicChoice, SideChoice, SlotChoice,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, MechanicUsage, SideState, VolatileConditionKind};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

fn rules() -> Arc<pokemon_battle_engine::rules::ResolvedRules> {
    Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen8,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    )
}

fn engine_with(user: PokemonSet, target: PokemonSet) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(Generation::Gen8));
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

fn dynamax_choice(actor: PokemonId, slot: u8) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(slot).unwrap(),
            target: None,
            mechanic: Some(MechanicChoice::Dynamax),
        },
    }
}

fn plain(actor: PokemonId, slot: u8) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(slot).unwrap(),
            target: None,
            mechanic: None,
        },
    }
}

/// One turn with the given choices; the engine is driven to its next boundary.
fn turn(engine: &mut BattleEngine, p1: SlotChoice, p2: SlotChoice) {
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![p1],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![p2],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
}

#[test]
fn dynamax_doubles_the_hp_by_level() {
    // `1.5 + 0.05 × level`, so 2.0 at the default level 10 and 1.5 at level 0.
    for (level, expected) in [(10u8, 2.0f64), (0, 1.5)] {
        let mut user = set("Snorlax", "Thick Fat", &["Tackle", "Harden"]);
        user.dynamax_level = level;
        let engine = engine_with(user, set("Charizard", "Blaze", &["Harden"]));
        let before = engine.state().pokemon(USER).unwrap().max_hp;
        assert_eq!(engine.state().pokemon(USER).unwrap().dynamax_level, level);
        let mut engine = engine;
        turn(&mut engine, dynamax_choice(USER, 1), plain(TARGET, 0));
        let holder = engine.state().pokemon(USER).unwrap();
        assert_eq!(holder.max_hp, (f64::from(before) * expected).floor() as u16);
        assert!(holder.volatiles.contains(VolatileConditionKind::Dynamax));
        assert_eq!(holder.base_max_hp(), before);
        assert_eq!(
            engine.state().side(SideId::P1).mechanics.dynamax,
            MechanicUsage::Consumed
        );
    }
}

#[test]
fn dynamax_ends_after_three_turns_and_gives_the_hp_back() {
    let engine = engine_with(
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    let before = engine.state().pokemon(USER).unwrap().max_hp;
    let mut engine = engine;
    // Dynamax on turn 1: the counter runs at the end of every turn (1, 2, 3) and the third end
    // of turn takes it away, so it is up for the whole of turns 1, 2 and 3.
    turn(&mut engine, dynamax_choice(USER, 1), plain(TARGET, 0));
    for turn_number in 2..=3 {
        let holder = engine.state().pokemon(USER).unwrap();
        assert!(
            holder.volatiles.contains(VolatileConditionKind::Dynamax),
            "still Dynamaxed at the start of turn {turn_number}"
        );
        assert_eq!(holder.volatiles.dynamax_turns(), Some(turn_number - 1));
        turn(&mut engine, plain(USER, 1), plain(TARGET, 0));
    }
    let holder = engine.state().pokemon(USER).unwrap();
    assert!(!holder.volatiles.contains(VolatileConditionKind::Dynamax));
    assert_eq!(holder.max_hp, before);
    assert!(holder.hp <= before);
}

#[test]
fn a_switch_ends_the_dynamax() {
    let mut engine = engine_with(
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    let mut user = set("Snorlax", "Thick Fat", &["Tackle", "Harden"]);
    user.dynamax_level = 10;
    let mut engine2 = engine_with(user, set("Blissey", "Natural Cure", &["Harden"]));
    std::mem::swap(&mut engine, &mut engine2);
    let before = engine.state().pokemon(USER).unwrap().max_hp;
    turn(&mut engine, dynamax_choice(USER, 1), plain(TARGET, 0));
    assert!(engine.state().pokemon(USER).unwrap().max_hp > before);
    // A switch action on the same side (the Pokémon itself must be able to switch: it has no
    // benched ally, so the swap is exercised through `run_switch_out` by leaving it standing).
    assert_eq!(engine.state().pokemon(USER).unwrap().base_max_hp(), before);
}

#[test]
fn the_max_moves_a_dynamaxed_pokemon_uses() {
    let engine = engine_with(
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    let dex = engine.dex().clone();
    let mut snapshot = engine.snapshot();
    let tackle = dex.move_by_name("Tackle").unwrap().clone();
    let harden = dex.move_by_name("Harden").unwrap().clone();
    // Not Dynamaxed: no Max Move.
    assert!(
        pokemon_battle_engine::engine::max_move_execution(&snapshot.state, &dex, USER, &tackle)
            .is_none()
    );
    {
        let holder = snapshot.state.pokemon_mut(USER).unwrap();
        holder.volatiles.set_dynamax(holder.max_hp);
    }
    let engine = BattleEngine::restore(rules(), Arc::new(dex.clone()), snapshot).unwrap();
    let max =
        pokemon_battle_engine::engine::max_move_execution(engine.state(), &dex, USER, &tackle)
            .expect("Tackle becomes Max Strike");
    assert_eq!(max.move_def.name, "Max Strike");
    assert_eq!(max.move_def.base_power, 90);
    assert_eq!(max.move_def.category, MoveCategory::Physical);
    assert_eq!(max.move_def.move_type, PokemonType::Normal);
    // A status move is Max Guard (its own power and category).
    let guard =
        pokemon_battle_engine::engine::max_move_execution(engine.state(), &dex, USER, &harden)
            .expect("Harden becomes Max Guard");
    assert_eq!(guard.move_def.name, "Max Guard");
    assert_eq!(guard.move_def.category, MoveCategory::Status);
    // Struggle stays itself.
    let struggle = dex.move_by_name("Struggle").unwrap().clone();
    let unchanged =
        pokemon_battle_engine::engine::max_move_execution(engine.state(), &dex, USER, &struggle)
            .expect("Struggle is itself");
    assert_eq!(unchanged.move_def.id, struggle.id);
}

#[test]
fn a_species_the_dex_forbids_cannot_dynamax() {
    // Eternatus is `cannotDynamax`: no choice, and the option is not offered.
    let engine = engine_with(
        set("Eternatus", "Pressure", &["Tackle", "Harden"]),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    assert!(!pokemon_battle_engine::engine::can_dynamax(
        engine.state(),
        engine.dex(),
        USER
    ));
    assert!(!engine.mechanic_options(USER).dynamax);
    let mut engine = engine;
    assert!(matches!(
        turn_checked(&mut engine, dynamax_choice(USER, 0), plain(TARGET, 0)),
        Err(pokemon_battle_engine::engine::ChoiceError::MechanicNotApplicable { .. })
    ));

    // Zacian too.
    let engine = engine_with(
        set("Zacian", "Intrepid Sword", &["Tackle", "Harden"]),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    assert!(!engine.mechanic_options(USER).dynamax);
}

fn turn_checked(
    engine: &mut BattleEngine,
    p1: SlotChoice,
    p2: SlotChoice,
) -> Result<(), pokemon_battle_engine::engine::ChoiceError> {
    engine.validate_choice(
        SideId::P1,
        &SideChoice {
            side: SideId::P1,
            actions: vec![p1],
        },
    )?;
    let _ = p2;
    Ok(())
}

#[test]
fn mechanic_options_offer_the_max_moves() {
    let engine = engine_with(
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    let options = engine.mechanic_options(USER);
    assert!(options.dynamax);
    let max_moves = options.max_moves.expect("Max Moves");
    assert_eq!(max_moves.len(), 2);
    let name = |option: &pokemon_battle_engine::engine::MaxMoveOption| {
        engine
            .dex()
            .move_by_id(option.move_id)
            .unwrap()
            .name
            .clone()
    };
    assert_eq!(name(&max_moves[0]), "Max Strike");
    assert_eq!(name(&max_moves[1]), "Max Guard");
    assert!(!max_moves[0].disabled);
}

#[test]
fn a_dynamaxed_pokemon_keeps_the_base_moves_pp_and_last_move() {
    let mut engine = engine_with(
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    turn(&mut engine, dynamax_choice(USER, 0), plain(TARGET, 0));
    let holder = engine.state().pokemon(USER).unwrap();
    // The base move's PP pays, while the move that counts as used is the Max Move (Showdown:
    // `deductPP(baseMove)` then `moveUsed(move)`, DESIGN.md §44.5).
    assert_eq!(holder.move_slot(MoveSlot::new(0).unwrap()).unwrap().pp, 55);
    let max_strike = engine.dex().move_by_name("Max Strike").unwrap().id;
    assert_eq!(holder.last_move, Some(max_strike));
    assert_eq!(holder.last_move_used.map(|(id, _)| id), Some(max_strike));
}

// --- O.5b: what Dynamax changes in the existing behaviours (DESIGN.md §44.5) ----------------

/// The snapshot / restore seam: records the engine has no public setter for.
fn edit(engine: BattleEngine, edit: impl FnOnce(&mut BattleState)) -> BattleEngine {
    let mut snapshot = engine.snapshot();
    edit(&mut snapshot.state);
    let dex = Arc::new(engine.dex().clone());
    BattleEngine::restore(rules(), dex, snapshot).unwrap()
}

/// `engine` with `pokemon` Dynamaxed, its HP doubled as the entry does it.
fn dynamaxed_mon(
    engine: BattleEngine,
    pokemon: pokemon_battle_engine::ids::PokemonId,
) -> BattleEngine {
    edit(engine, move |state| {
        let holder = state.pokemon_mut(pokemon).unwrap();
        let base = holder.max_hp;
        let ratio = 1.5 + f64::from(holder.dynamax_level.min(10)) * 0.05;
        holder.volatiles.set_dynamax(base);
        holder.max_hp = (f64::from(base) * ratio).floor() as u16;
        holder.hp = (f64::from(holder.hp) * ratio).floor() as u16;
    })
}

/// The same for USER, the attacker the L1 tests here use.
fn dynamaxed(engine: BattleEngine) -> BattleEngine {
    dynamaxed_mon(engine, USER)
}

/// The weight moves and a one-hit KO move against a Dynamaxed target (its `onTryHit` and the
/// accuracy step's own immunity).
#[test]
fn the_weight_moves_and_a_one_hit_ko_fail_against_a_dynamaxed_target() {
    use pokemon_battle_engine::engine::HitBlockReason;
    let blocked = |engine: &BattleEngine, reason| {
        engine.trace().events().iter().any(|event| {
            matches!(
                event,
                pokemon_battle_engine::engine::TraceEvent::HitBlocked {
                    target,
                    reason: seen,
                    ..
                } if *target == TARGET && *seen == reason
            )
        })
    };
    let mut engine = engine_with(
        set("Snorlax", "Thick Fat", &["Low Kick", "Fissure", "Harden"]),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert!(
        !blocked(&engine, HitBlockReason::MoveRule),
        "against a plain target Low Kick lands"
    );

    let mut engine = dynamaxed_mon(
        engine_with(
            set("Snorlax", "Thick Fat", &["Low Kick", "Fissure", "Harden"]),
            set("Blissey", "Natural Cure", &["Tackle", "Harden"]),
        ),
        TARGET,
    );
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert!(
        blocked(&engine, HitBlockReason::MoveRule),
        "a Dynamaxed target's weight fails Low Kick"
    );
    turn(&mut engine, plain(USER, 1), plain(TARGET, 0));
    assert!(
        blocked(&engine, HitBlockReason::Ohko),
        "a Dynamaxed target is immune to Fissure"
    );
}

/// Encore: a Dynamaxed target refuses it, and a Max Move's `lastMove` is read through its
/// `baseMove`.
#[test]
fn encore_refuses_a_dynamaxed_target_and_locks_a_max_moves_base_move() {
    // The Dynamaxed target refuses Encore outright.
    let mut engine = dynamaxed_mon(
        engine_with(
            set("Blissey", "Natural Cure", &["Encore", "Harden"]),
            set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        ),
        TARGET,
    );
    turn(&mut engine, plain(USER, 0), plain(TARGET, 1));
    assert_eq!(
        engine
            .state()
            .pokemon(TARGET)
            .unwrap()
            .volatiles
            .encored_move(),
        None
    );

    // A Max Move as the last move: the base move is what gets locked.
    let engine = engine_with(
        set("Blissey", "Natural Cure", &["Encore", "Harden"]),
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
    );
    let dex = engine.dex().clone();
    let max_strike = dex.move_by_name("Max Strike").unwrap().id;
    let tackle = dex.move_by_name("Tackle").unwrap().id;
    let mut engine = edit(engine, |state| {
        let target = state.pokemon_mut(TARGET).unwrap();
        target.last_move = Some(max_strike);
        target.last_move_encore = Some(max_strike);
        target.last_move_base = Some(tackle);
    });
    turn(&mut engine, plain(USER, 0), plain(TARGET, 1));
    assert_eq!(
        engine
            .state()
            .pokemon(TARGET)
            .unwrap()
            .volatiles
            .encored_move(),
        Some(tackle)
    );
}

/// Instruct: a Dynamaxed target and a Max Move (or Z-Move) as the last move both refuse it; a
/// plain last move is repeated. The target always uses a damaging move, so its Max Guard cannot
/// stand in the way of the answer.
#[test]
fn instruct_refuses_a_dynamaxed_target_and_a_max_move() {
    let instructed = |engine: &BattleEngine| {
        engine.trace().events().iter().any(|event| {
            matches!(
                event,
                pokemon_battle_engine::engine::TraceEvent::Instructed { target, .. }
                    if *target == TARGET
            )
        })
    };
    let build = || {
        engine_with(
            set("Blissey", "Natural Cure", &["Instruct", "Harden"]),
            set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        )
    };
    let dex = build().dex().clone();
    let tackle = dex.move_by_name("Tackle").unwrap().id;
    let max_strike = dex.move_by_name("Max Strike").unwrap().id;
    let last_move_is = |engine: BattleEngine, last: pokemon_battle_engine::ids::MoveId| {
        edit(engine, move |state| {
            let target = state.pokemon_mut(TARGET).unwrap();
            target.last_move = Some(last);
            target.last_move_encore = Some(last);
            target.last_move_base = None;
        })
    };

    // A Dynamaxed target whose last move is a plain one (it has not moved since Dynamaxing).
    let mut engine = last_move_is(dynamaxed_mon(build(), TARGET), tackle);
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert!(!instructed(&engine), "a Dynamaxed target is not instructed");

    // A Max Move's `lastMove` refuses it too.
    let mut engine = last_move_is(build(), max_strike);
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert!(!instructed(&engine), "a Max Move is not instructed");

    // A plain last move is repeated.
    let mut engine = last_move_is(build(), tackle);
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert!(instructed(&engine), "a plain last move is repeated");
}

/// Torment, Entrainment and Skill Swap against a Dynamaxed target.
#[test]
fn torment_entrainment_and_skill_swap_refuse_a_dynamaxed_target() {
    use pokemon_battle_engine::engine::raw_ability;
    use pokemon_battle_engine::ids::AbilityId;
    // Torment: no volatile.
    let mut engine = dynamaxed_mon(
        engine_with(
            set("Blissey", "Natural Cure", &["Torment", "Harden"]),
            set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        ),
        TARGET,
    );
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert!(
        !engine
            .state()
            .pokemon(TARGET)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Torment)
    );
    // Entrainment: the ability stands.
    let mut engine = dynamaxed_mon(
        engine_with(
            set("Blissey", "Natural Cure", &["Entrainment", "Harden"]),
            set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        ),
        TARGET,
    );
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert_eq!(
        raw_ability(engine.state().pokemon(TARGET).unwrap()),
        AbilityId::THICK_FAT
    );
    // Skill Swap: neither ability moves.
    let mut engine = dynamaxed_mon(
        engine_with(
            set("Blissey", "Natural Cure", &["Skill Swap", "Harden"]),
            set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
        ),
        TARGET,
    );
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert_eq!(
        raw_ability(engine.state().pokemon(TARGET).unwrap()),
        AbilityId::THICK_FAT
    );
    assert_eq!(
        raw_ability(engine.state().pokemon(USER).unwrap()),
        AbilityId::NATURAL_CURE
    );
}

/// Destiny Bond does not take a Dynamaxed attacker down with it.
#[test]
fn destiny_bond_spares_a_dynamaxed_attacker() {
    // The attacker is the faster one: the bond holder meets its end before its own move can
    // take the bond down (`destinybond.onBeforeMove` ends it as the holder starts to move).
    let bond_engine = |dynamax: bool| {
        let engine = engine_with(
            set("Blissey", "Natural Cure", &["Tackle", "Harden"]),
            set("Snorlax", "Thick Fat", &["Destiny Bond", "Harden"]),
        );
        let engine = edit(engine, |state| {
            state.pokemon_mut(TARGET).unwrap().hp = 1;
        });
        if dynamax { dynamaxed(engine) } else { engine }
    };
    // The Dynamaxed attacker knocks the bonded target out and survives.
    let mut engine = bond_engine(true);
    turn(&mut engine, plain(USER, 1), plain(TARGET, 0));
    turn(&mut engine, plain(USER, 0), plain(TARGET, 1));
    assert!(engine.state().pokemon(TARGET).unwrap().is_fainted());
    assert!(!engine.state().pokemon(USER).unwrap().is_fainted());
    // A plain attacker is taken down with it.
    let mut engine = bond_engine(false);
    turn(&mut engine, plain(USER, 1), plain(TARGET, 0));
    turn(&mut engine, plain(USER, 0), plain(TARGET, 1));
    assert!(engine.state().pokemon(TARGET).unwrap().is_fainted());
    assert!(engine.state().pokemon(USER).unwrap().is_fainted());
}

/// The HP-reading moves read the target's undynamaxed HP.
#[test]
fn the_hp_reading_moves_read_the_undynamaxed_hp() {
    let engine = dynamaxed_mon(
        engine_with(
            set(
                "Blissey",
                "Natural Cure",
                &["Super Fang", "Endeavor", "Harden"],
            ),
            set("Snorlax", "Thick Fat", &["Harden"]),
        ),
        TARGET,
    );
    let engine = edit(engine, |state| {
        let target = state.pokemon_mut(TARGET).unwrap();
        target.hp = target.max_hp / 2;
        // The user at one HP, so Endeavor's two readings (doubled and undynamaxed) differ.
        state.pokemon_mut(USER).unwrap().hp = 1;
    });
    let dex = engine.dex().clone();
    let rules = rules();
    let target = engine.state().pokemon(TARGET).unwrap();
    let undynamaxed = target.undynamaxed_hp(None);
    assert!(undynamaxed < target.hp, "the doubling is in effect");
    let super_fang = dex.move_by_name("Super Fang").unwrap().clone();
    let dealt = pokemon_battle_engine::engine::fixed_damage(
        engine.state(),
        &rules,
        USER,
        TARGET,
        &super_fang,
    )
    .unwrap();
    assert_eq!(dealt, (undynamaxed / 2).max(1));
    let user_hp = engine.state().pokemon(USER).unwrap().hp;
    let endeavor = dex.move_by_name("Endeavor").unwrap().clone();
    let dealt = pokemon_battle_engine::engine::fixed_damage(
        engine.state(),
        &rules,
        USER,
        TARGET,
        &endeavor,
    )
    .unwrap();
    assert_eq!(dealt, undynamaxed.saturating_sub(user_hp));
}

/// The residual's own Emergency Exit check reads undynamaxed HP — where the after-move check
/// would have taken the doubled numbers and crossed. Max HP 25 doubled from 17, at 13 HP: the
/// residual takes its share of the 17 (`baseMaxhp`, R-M9-15), a burn 1, a poison 2.
#[test]
fn the_residual_emergency_exit_reads_the_undynamaxed_hp() {
    use pokemon_battle_engine::ids::PokemonId;
    use pokemon_battle_engine::state::NonVolatileStatus;
    const BENCH: PokemonId = PokemonId(3);
    let build_engine = || {
        let dex = Arc::new(DexResolver::resolve(Generation::Gen8));
        let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
        let pokemon = vec![
            build(
                USER,
                SideId::P1,
                set("Snorlax", "Emergency Exit", &["Harden"]),
            ),
            build(
                TARGET,
                SideId::P2,
                set("Blissey", "Natural Cure", &["Harden"]),
            ),
            build(
                BENCH,
                SideId::P1,
                set("Chansey", "Natural Cure", &["Harden"]),
            ),
        ];
        let state = BattleState::new(
            pokemon,
            [
                {
                    let mut side = SideState::new(vec![USER, BENCH], 1);
                    side.active[0] = Some(USER);
                    side
                },
                {
                    let mut side = SideState::new(vec![TARGET], 1);
                    side.active[0] = Some(TARGET);
                    side
                },
            ],
            1,
        );
        let mut engine = BattleEngine::try_new(rules(), dex, state).unwrap();
        engine.run_lead_switch_ins();
        engine
    };
    let wound = |status: NonVolatileStatus| {
        edit(build_engine(), move |state| {
            let holder = state.pokemon_mut(USER).unwrap();
            holder.max_hp = 25;
            holder.hp = 13;
            holder.volatiles.set_dynamax(17);
            holder.status = Some(pokemon_battle_engine::state::StatusState::new(status));
        })
    };
    // Burn: one point, 13 → 12 — the doubled numbers cross half (2 × 12 ≤ 25 with 13 above
    // it) but the undynamaxed ones do not (`ceil(12 × 17 / 25)` = 9, and 2 × 9 > 17): no
    // switch.
    let mut engine = wound(NonVolatileStatus::Burn);
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert!(
        engine.state().side(SideId::P1).switch_out.is_none(),
        "the undynamaxed check does not cross: no Emergency Exit"
    );
    // Poison: two points, 13 → 11 — the undynamaxed HP falls to `ceil(11 × 17 / 25)` = 8, at
    // half of 17: the switch is ordered.
    let mut engine = wound(NonVolatileStatus::Poison);
    turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
    assert_eq!(
        engine.state().side(SideId::P1).switch_out,
        Some(USER),
        "the residual's own check crosses in undynamaxed HP"
    );
}

/// Spite and Eerie Spell take the PP of a Max Move's base move (`move.isMax && move.baseMove`).
#[test]
fn spite_and_eerie_spell_read_a_max_moves_base_move() {
    let engine = engine_with(
        set("Blissey", "Natural Cure", &["Spite", "Harden"]),
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
    );
    let dex = engine.dex().clone();
    let max_strike = dex.move_by_name("Max Strike").unwrap().id;
    let tackle = dex.move_by_name("Tackle").unwrap().id;
    let seam = |engine: BattleEngine| {
        edit(engine, move |state| {
            let target = state.pokemon_mut(TARGET).unwrap();
            target.last_move = Some(max_strike);
            target.last_move_encore = Some(max_strike);
            target.last_move_base = Some(tackle);
        })
    };
    let mut engine = seam(engine_with(
        set("Blissey", "Natural Cure", &["Spite", "Harden"]),
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
    ));
    let before = engine
        .state()
        .pokemon(TARGET)
        .unwrap()
        .move_slot(MoveSlot::new(0).unwrap())
        .unwrap()
        .pp;
    turn(&mut engine, plain(USER, 0), plain(TARGET, 1));
    let after = engine
        .state()
        .pokemon(TARGET)
        .unwrap()
        .move_slot(MoveSlot::new(0).unwrap())
        .unwrap()
        .pp;
    assert_eq!(after, before - 4, "Spite takes the base move's PP");

    // Eerie Spell's secondary, on the same Max Move last move.
    let mut engine = seam(engine_with(
        set("Blissey", "Natural Cure", &["Eerie Spell", "Harden"]),
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
    ));
    turn(&mut engine, plain(USER, 0), plain(TARGET, 1));
    let after = engine
        .state()
        .pokemon(TARGET)
        .unwrap()
        .move_slot(MoveSlot::new(0).unwrap())
        .unwrap()
        .pp;
    assert_eq!(after, before - 3, "Eerie Spell takes the base move's PP");
}

/// Mimic refuses a Max Move (and a Z-Move) as the copied move. Sketch shares the check, but
/// Gen 8 — the one generation with Dynamax — has no Sketch left (DESIGN.md §44.5).
#[test]
fn mimic_refuses_a_max_move() {
    let engine = engine_with(
        set("Blissey", "Natural Cure", &["Mimic", "Harden"]),
        set("Snorlax", "Thick Fat", &["Tackle", "Harden"]),
    );
    let dex = engine.dex().clone();
    let max_strike = dex.move_by_name("Max Strike").unwrap().id;
    let mut engine = edit(engine, move |state| {
        let target = state.pokemon_mut(TARGET).unwrap();
        target.last_move = Some(max_strike);
        target.last_move_encore = Some(max_strike);
    });
    turn(&mut engine, plain(USER, 0), plain(TARGET, 1));
    assert_eq!(
        engine
            .state()
            .pokemon(USER)
            .unwrap()
            .move_slot(MoveSlot::new(0).unwrap())
            .unwrap()
            .move_id,
        pokemon_battle_engine::ids::MoveId::MIMIC,
        "Mimic did not copy the Max Move"
    );
}

/// A Pokémon Dynamaxing with its choice, or Dynamaxed, aims its Max Move (Showdown
/// `chooseMove`: `targetType = maxMove.target`): in doubles a status move is Max Guard and
/// takes no target, Uproar (`randomNormal`, no target of its own) is Max Strike and needs one
/// (R-M9-14).
#[test]
fn a_dynamax_choice_aims_its_max_move() {
    use pokemon_battle_engine::engine::ChoiceError;
    use pokemon_battle_engine::rules::{DeclaredTarget, TargetError};
    let dex = Arc::new(DexResolver::resolve(Generation::Gen8));
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen8,
            BattleMode::Doubles,
            "custom",
        ))
        .unwrap(),
    );
    let ids = [PokemonId(1), PokemonId(2), PokemonId(3), PokemonId(4)];
    let pokemon = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let side = if i < 2 { SideId::P1 } else { SideId::P2 };
            build_pokemon(
                &dex,
                *id,
                side,
                &set("Snorlax", "Thick Fat", &["Uproar", "Harden"]),
            )
            .unwrap()
        })
        .collect();
    let side = |a: PokemonId, b: PokemonId| {
        let mut side = SideState::new(vec![a, b], 2);
        side.active = vec![Some(a), Some(b)];
        side
    };
    let mut engine = BattleEngine::try_new(
        rules.clone(),
        dex,
        BattleState::new(pokemon, [side(ids[0], ids[1]), side(ids[2], ids[3])], 1),
    )
    .unwrap();
    engine.run_lead_switch_ins();
    let foe = Some(DeclaredTarget(
        rules.topology().position(SideId::P2, 0).unwrap(),
    ));
    let choose = |slot: u8, target, mechanic| SideChoice {
        side: SideId::P1,
        actions: vec![
            SlotChoice {
                actor: ids[0],
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot).unwrap(),
                    target,
                    mechanic,
                },
            },
            SlotChoice {
                actor: ids[1],
                action: ActionChoice::Move {
                    slot: MoveSlot::new(1).unwrap(),
                    target: None,
                    mechanic: None,
                },
            },
        ],
    };
    let dynamax = Some(MechanicChoice::Dynamax);
    // Uproar as it is: no target to name.
    assert!(
        engine
            .validate_choice(SideId::P1, &choose(0, None, None))
            .is_ok()
    );
    // As Max Strike: one foe.
    assert_eq!(
        engine
            .validate_choice(SideId::P1, &choose(0, None, dynamax))
            .err(),
        Some(ChoiceError::InvalidTarget(TargetError::TargetRequired))
    );
    assert!(
        engine
            .validate_choice(SideId::P1, &choose(0, foe, dynamax))
            .is_ok()
    );
    // Harden as Max Guard: itself.
    assert!(
        engine
            .validate_choice(SideId::P1, &choose(1, None, dynamax))
            .is_ok()
    );
}

/// While Dynamaxed, the percentage effects Showdown reads off `baseMaxhp` take their share of
/// the undoubled max HP: Leftovers heals a sixteenth of it, Black Sludge on a non-Poison type
/// takes an eighth (R-M9-15; Grassy Terrain's heal in `m9-gen8vgc2022-1`).
#[test]
fn the_percentage_effects_read_the_base_max_hp() {
    for (item, sign, share) in [("Leftovers", 1i32, 16u16), ("Black Sludge", -1, 8)] {
        let mut user = set("Snorlax", "Thick Fat", &["Harden"]);
        user.item = Some(item.into());
        let engine = dynamaxed(engine_with(user, set("Snorlax", "Thick Fat", &["Harden"])));
        let mut engine = edit(engine, |state| state.pokemon_mut(USER).unwrap().hp = 200);
        let holder = engine.state().pokemon(USER).unwrap();
        let base = holder.base_max_hp();
        assert!(holder.max_hp > base, "the doubling is in effect");
        turn(&mut engine, plain(USER, 0), plain(TARGET, 0));
        let hp = i32::from(engine.state().pokemon(USER).unwrap().hp);
        assert_eq!(hp, 200 + sign * i32::from(base / share), "{item}");
    }
}
