//! L1 rule tests for Mega Evolution and Ultra Burst (DESIGN.md §44.3): the stone's forme change
//! and the ability it brings, the once-a-side use, Mega Rayquaza's Dragon Ascent, the Ultra
//! Burst as a mechanic of its own, the stones' `onTakeItem` and the AfterMega participants.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, ChoiceError, EngineStep, MechanicChoice, SideChoice, SlotChoice,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId, SpeciesId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, MechanicUsage, SideState};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

fn rules(generation: Generation) -> Arc<pokemon_battle_engine::rules::ResolvedRules> {
    Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    )
}

fn engine_with(generation: Generation, user: PokemonSet, target: PokemonSet) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(generation));
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
        rules(generation),
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

fn with_item(mut set: PokemonSet, item: &str) -> PokemonSet {
    set.item = Some(item.into());
    set
}

fn mega(actor: PokemonId, slot: u8) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(slot).unwrap(),
            target: None,
            mechanic: Some(MechanicChoice::MegaEvolve),
        },
    }
}

fn ultra(actor: PokemonId, slot: u8) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(slot).unwrap(),
            target: None,
            mechanic: Some(MechanicChoice::UltraBurst),
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

/// The stone's forme change: the mega forme, its own ability (started), and the spent mechanic.
#[test]
fn a_mega_stone_changes_the_forme_and_starts_its_ability() {
    use pokemon_battle_engine::engine::raw_ability;
    use pokemon_battle_engine::ids::AbilityId;
    let mut engine = engine_with(
        Generation::Gen6,
        with_item(
            set("Venusaur", "Overgrow", &["Harden", "Tackle"]),
            "Venusaurite",
        ),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    turn(&mut engine, mega(USER, 0), plain(TARGET, 0));
    let holder = engine.state().pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::species_of(holder),
        Some(SpeciesId::VENUSAUR_MEGA)
    );
    assert_eq!(raw_ability(holder), AbilityId::THICK_FAT);
    assert_eq!(
        holder.held_item,
        Some(pokemon_battle_engine::ids::ItemId::VENUSAURITE)
    );
    assert_eq!(
        engine.state().side(SideId::P1).mechanics.mega,
        MechanicUsage::Consumed
    );
}

/// The stone evolves its holder once a side, and only the species it belongs to.
#[test]
fn a_side_mega_evolves_once_and_only_its_own_species() {
    let mut engine = engine_with(
        Generation::Gen6,
        with_item(
            set("Venusaur", "Overgrow", &["Harden", "Tackle"]),
            "Venusaurite",
        ),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    turn(&mut engine, mega(USER, 0), plain(TARGET, 0));
    let second = SideChoice {
        side: SideId::P1,
        actions: vec![mega(USER, 1)],
    };
    assert_eq!(
        engine.validate_choice(SideId::P1, &second),
        Err(ChoiceError::MechanicUnavailable(MechanicChoice::MegaEvolve))
    );

    // The stone on another species offers nothing.
    let engine = engine_with(
        Generation::Gen6,
        with_item(set("Snorlax", "Thick Fat", &["Harden"]), "Venusaurite"),
        set("Blissey", "Natural Cure", &["Harden"]),
    );
    let choice = SideChoice {
        side: SideId::P1,
        actions: vec![mega(USER, 0)],
    };
    assert_eq!(
        engine.validate_choice(SideId::P1, &choice),
        Err(ChoiceError::MechanicNotApplicable {
            actor: USER,
            mechanic: MechanicChoice::MegaEvolve,
        })
    );
}

/// Mega Rayquaza: a Dragon Ascent in the moves is the whole condition — and a Z-Crystal held
/// takes it away.
#[test]
fn mega_rayquaza_needs_dragon_ascent() {
    use pokemon_battle_engine::engine::mechanic_options;
    let options = |rayquaza: PokemonSet| {
        let engine = engine_with(
            Generation::Gen7,
            rayquaza,
            set("Snorlax", "Thick Fat", &["Harden"]),
        );
        mechanic_options(engine.state(), &rules(Generation::Gen7), engine.dex(), USER)
    };
    assert_eq!(
        options(set("Rayquaza", "Air Lock", &["Harden", "Tackle"])).mega,
        None,
        "no Dragon Ascent in the moves"
    );
    assert_eq!(
        options(set("Rayquaza", "Air Lock", &["Dragon Ascent", "Harden"])).mega,
        Some(SpeciesId::RAYQUAZA_MEGA)
    );
    assert_eq!(
        options(with_item(
            set("Rayquaza", "Air Lock", &["Dragon Ascent", "Harden"]),
            "Normalium Z"
        ))
        .mega,
        None,
        "a Z-Crystal held takes the Dragon Ascent path away"
    );
}

/// Ultra Burst: its own mechanic (Q1), separate from Mega Evolution's count.
#[test]
fn ultra_burst_is_a_mechanic_of_its_own() {
    use pokemon_battle_engine::engine::raw_ability;
    use pokemon_battle_engine::ids::AbilityId;
    let mut engine = engine_with(
        Generation::Gen7,
        with_item(
            set("Necrozma-Dusk-Mane", "Prism Armor", &["Harden", "Tackle"]),
            "Ultranecrozium Z",
        ),
        set("Snorlax", "Thick Fat", &["Harden"]),
    );
    turn(&mut engine, ultra(USER, 0), plain(TARGET, 0));
    let holder = engine.state().pokemon(USER).unwrap();
    assert_eq!(
        pokemon_battle_engine::engine::species_of(holder),
        Some(SpeciesId::NECROZMA_ULTRA)
    );
    assert_eq!(raw_ability(holder), AbilityId::NEUROFORCE);
    assert_eq!(
        engine.state().side(SideId::P1).mechanics.ultra_burst,
        MechanicUsage::Consumed
    );
    assert_eq!(
        engine.state().side(SideId::P1).mechanics.mega,
        MechanicUsage::Available,
        "Mega Evolution is another mechanic's"
    );
}

/// A Mega Stone stays with its species (any forme), and is taken from another.
#[test]
fn a_mega_stone_cannot_be_taken_from_its_species() {
    use pokemon_battle_engine::engine::item_refuses_take;
    use pokemon_battle_engine::ids::{ItemId, MoveId};
    let engine = engine_with(
        Generation::Gen7,
        set("Snorlax", "Thick Fat", &["Knock Off"]),
        set("Venusaur", "Overgrow", &["Harden"]),
    );
    let refuses = |holder: PokemonId| {
        item_refuses_take(
            engine.state(),
            &rules(Generation::Gen7),
            ItemId::VENUSAURITE,
            holder,
            USER,
            Some(MoveId::KNOCK_OFF),
        )
    };
    assert!(refuses(TARGET), "the stone stays with the Venusaur");
    assert!(!refuses(USER), "a Snorlax's stone is taken");
}

// --- the AfterMega event and the step's ordering ---------------------------------------------

const ALLY: PokemonId = PokemonId(3);
const FOE_ALLY: PokemonId = PokemonId(4);
const BENCH: PokemonId = PokemonId(5);
const FOE_BENCH: PokemonId = PokemonId(6);

fn doubles_rules(generation: Generation) -> Arc<pokemon_battle_engine::rules::ResolvedRules> {
    Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    )
}

fn doubles_engine(
    generation: Generation,
    p1: [PokemonSet; 3],
    p2: [PokemonSet; 3],
) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(generation));
    let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
    let pokemon = vec![
        build(USER, SideId::P1, p1[0].clone()),
        build(TARGET, SideId::P2, p2[0].clone()),
        build(ALLY, SideId::P1, p1[1].clone()),
        build(FOE_ALLY, SideId::P2, p2[1].clone()),
        build(BENCH, SideId::P1, p1[2].clone()),
        build(FOE_BENCH, SideId::P2, p2[2].clone()),
    ];
    let side = |team: [PokemonId; 3]| {
        let mut side = SideState::new(team.to_vec(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let mut engine = BattleEngine::try_new(
        doubles_rules(generation),
        dex,
        BattleState::new(
            pokemon,
            [
                side([USER, ALLY, BENCH]),
                side([TARGET, FOE_ALLY, FOE_BENCH]),
            ],
            1,
        ),
    )
    .unwrap();
    engine.run_lead_switch_ins();
    engine
}

fn edit(engine: BattleEngine, edit: impl FnOnce(&mut BattleState)) -> BattleEngine {
    let mut snapshot = engine.snapshot();
    edit(&mut snapshot.state);
    let dex = Arc::new(engine.dex().clone());
    BattleEngine::restore(doubles_rules(Generation::Gen7), dex, snapshot).unwrap()
}

/// Showdown `runEvent('AfterMega')`: the White Herb a foe's Mega Evolution finds ready restores
/// its holder's lowered stats (the handlers are the ones their `onAnyAfterMove` runs).
#[test]
fn the_after_mega_event_runs_the_reused_handlers() {
    // White Herb: the holder's lowered stat is restored as the foe Mega Evolves. (The other
    // `onAnyAfterMega` handlers cannot run beside a Mega Evolution: Mirror Herb is Gen 9, Eject
    // Pack Gen 8, Opportunist Gen 9, and no Mega Stone exists in those generations —
    // DESIGN.md §44.3.)
    let engine = doubles_engine(
        Generation::Gen7,
        [
            with_item(set("Snorlax", "Thick Fat", &["Harden"]), "White Herb"),
            set("Blissey", "Natural Cure", &["Harden"]),
            set("Chansey", "Natural Cure", &["Harden"]),
        ],
        [
            with_item(set("Lucario", "Inner Focus", &["Harden"]), "Lucarionite"),
            set("Snorlax", "Thick Fat", &["Harden"]),
            set("Blissey", "Natural Cure", &["Harden"]),
        ],
    );
    let mut engine = edit(engine, |state| {
        state.pokemon_mut(USER).unwrap().stages.attack = -1;
    });
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![plain(USER, 0), plain(ALLY, 0)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![mega(TARGET, 0), plain(FOE_ALLY, 0)],
        },
    ];
    engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert_eq!(
        engine.state().pokemon(USER).unwrap().stages.attack,
        0,
        "the herb restored the lowered stat at AfterMega"
    );
    assert_eq!(
        engine.state().pokemon(USER).unwrap().held_item,
        None,
        "the herb is used up"
    );
}

/// A switch runs before a Mega Evolution's step of the same turn (Showdown's 103 < 104), even
/// at the same speed: no `SpeedTie` decision is drawn for the pair.
#[test]
fn a_switch_orders_before_a_mega_step() {
    use pokemon_battle_engine::engine::TraceEvent;
    let mut engine = doubles_engine(
        Generation::Gen7,
        [
            set("Venusaur", "Overgrow", &["Harden"]),
            with_item(set("Slowbro", "Oblivious", &["Harden"]), "Slowbronite"),
            set("Chansey", "Natural Cure", &["Harden"]),
        ],
        [
            set("Blissey", "Natural Cure", &["Harden"]),
            set("Charizard", "Blaze", &["Harden"]),
            set("Blissey", "Natural Cure", &["Harden"]),
        ],
    );
    // P1's Venusaur switches out while the *slower* Slowbro Mega Evolves (Blissey is faster):
    // the switch goes first (order 103 < 104) and the Mega step still before every move.
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![
                SlotChoice {
                    actor: USER,
                    action: ActionChoice::Switch { pokemon: BENCH },
                },
                mega(ALLY, 0),
            ],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![plain(TARGET, 0), plain(FOE_ALLY, 0)],
        },
    ];
    engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let events = engine.trace().events();
    let switch_first = events
        .iter()
        .position(|event| {
            matches!(
                event,
                TraceEvent::ActionOrderResolved { actor, action_priority: 8, .. } if *actor == USER
            )
        })
        .expect("the switch's order");
    let mega_step = events
        .iter()
        .position(|event| {
            matches!(
                event,
                TraceEvent::ActionOrderResolved { actor, action_priority: 7, .. } if *actor == ALLY
            )
        })
        .expect("the mega step's order");
    assert!(switch_first < mega_step, "the switch goes first");
    let foe_move = events
        .iter()
        .position(|event| {
            matches!(
                event,
                TraceEvent::ActionOrderResolved { actor, action_priority: 0, .. } if *actor == TARGET
            )
        })
        .expect("the faster foe's move order");
    assert!(
        mega_step < foe_move,
        "the mega step runs before a faster foe's move"
    );
    let drawn = events
        .iter()
        .filter(|event| {
            matches!(event, TraceEvent::ActionOrderResolved { tie_break, .. } if *tie_break != 0)
        })
        .count();
    assert_eq!(drawn, 0, "distinct priorities: nothing to draw");
    assert_eq!(
        pokemon_battle_engine::engine::species_of(engine.state().pokemon(ALLY).unwrap()),
        Some(SpeciesId::SLOWBRO_MEGA)
    );
}

/// Showdown `faint`'s `formeRegression`: a fainted Mega goes back to the species and ability
/// its set named (and only then clears its volatiles).
#[test]
fn a_fainted_mega_goes_back_to_the_sets_species() {
    use pokemon_battle_engine::engine::raw_ability;
    use pokemon_battle_engine::ids::AbilityId;
    let engine = engine_with(
        Generation::Gen6,
        with_item(
            set("Venusaur", "Overgrow", &["Harden", "Tackle"]),
            "Venusaurite",
        ),
        set("Snorlax", "Thick Fat", &["Tackle"]),
    );
    let base = engine.state().pokemon(USER).unwrap().max_hp;
    let mut snapshot = engine.snapshot();
    snapshot.state.pokemon_mut(USER).unwrap().hp = 1;
    let mut engine = BattleEngine::restore(
        rules(Generation::Gen6),
        Arc::new(engine.dex().clone()),
        snapshot,
    )
    .unwrap();
    turn(&mut engine, mega(USER, 0), plain(TARGET, 0));
    let holder = engine.state().pokemon(USER).unwrap();
    assert!(holder.is_fainted());
    assert_eq!(
        pokemon_battle_engine::engine::species_of(holder),
        Some(SpeciesId::VENUSAUR)
    );
    assert_eq!(raw_ability(holder), AbilityId::OVERGROW);
    assert_eq!(holder.max_hp, base, "the base forme's max HP");
    assert!(!holder.forme_regression, "the mark is spent");
}
