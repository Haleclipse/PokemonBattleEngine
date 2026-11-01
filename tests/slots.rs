//! L1 tests for the slot conditions of DESIGN.md §30.5: what Revival Blessing's Revive request
//! asks for and what an answer may name (the replay only ever sends Showdown's legal answer), and
//! Lunar Dance's PP, which no frame shows.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, BattleRequest, ChoiceError, EngineStep, ReviveChoice, SideChoice,
    SlotChoice,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattlePhase, BattleState, SideState, SlotHeal, SlotHealKind};

const BLESSER: PokemonId = PokemonId(1);
const FOE: PokemonId = PokemonId(2);
const FAINTED: PokemonId = PokemonId(3);
const HEALTHY: PokemonId = PokemonId(4);

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set
}

/// P1 = [BLESSER, FAINTED (0 HP), HEALTHY]; P2 = [FOE]. BLESSER uses Revival Blessing.
fn blessed() -> BattleEngine {
    let generation = Generation::Gen9;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut fainted = build_pokemon(&dex, FAINTED, SideId::P1, &set("Mew", &["Harden"])).unwrap();
    fainted.hp = 0;
    let pokemon = vec![
        build_pokemon(
            &dex,
            BLESSER,
            SideId::P1,
            &set("Mew", &["Revival Blessing"]),
        )
        .unwrap(),
        build_pokemon(&dex, FOE, SideId::P2, &set("Snorlax", &["Harden"])).unwrap(),
        fainted,
        build_pokemon(&dex, HEALTHY, SideId::P1, &set("Mew", &["Harden"])).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(
        pokemon,
        [side(vec![BLESSER, FAINTED, HEALTHY]), side(vec![FOE])],
        1,
    );
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    let choice = |side, actor| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: None,
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([choice(SideId::P1, BLESSER), choice(SideId::P2, FOE)])
        .unwrap();
    engine
}

fn run_to_request(engine: &mut BattleEngine) -> EngineStep {
    loop {
        match engine.step().unwrap() {
            EngineStep::Progressed { .. } => continue,
            other => return other,
        }
    }
}

#[test]
fn revival_blessing_asks_for_a_fainted_pokemon() {
    let mut engine = blessed();
    assert_eq!(
        run_to_request(&mut engine),
        EngineStep::Awaiting(BattleRequest::Revive {
            side: SideId::P1,
            candidates: vec![FAINTED],
        })
    );
    assert_eq!(engine.state().phase, BattlePhase::Revive);
    // A healthy Pokémon, the other side, or a turn's choices are not answers.
    assert!(matches!(
        engine.resolve_revive(ReviveChoice {
            side: SideId::P1,
            pokemon: HEALTHY,
        }),
        Err(BattleError::Choice(ChoiceError::NotRevivable(HEALTHY)))
    ));
    assert!(matches!(
        engine.resolve_revive(ReviveChoice {
            side: SideId::P2,
            pokemon: FAINTED,
        }),
        Err(BattleError::Choice(ChoiceError::WrongSide { .. }))
    ));
    assert!(matches!(
        engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::Revive { .. })
    ));
    engine
        .resolve_revive(ReviveChoice {
            side: SideId::P1,
            pokemon: FAINTED,
        })
        .unwrap();
    let revived = engine.state().pokemon(FAINTED).unwrap();
    assert_eq!(revived.hp, revived.max_hp / 2);
    // It stays on the bench (singles); the turn goes on to its end.
    assert!(!engine.state().side(SideId::P1).is_active(FAINTED));
    assert!(matches!(
        run_to_request(&mut engine),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
    // Answered, the request is gone.
    assert!(matches!(
        engine.resolve_revive(ReviveChoice {
            side: SideId::P1,
            pokemon: FAINTED,
        }),
        Err(BattleError::WrongPhase { .. })
    ));
}

/// Lunar Dance waiting on P1's slot; P1 switches in a Pokémon whose only need is PP.
fn lunar_dance_entry(generation: Generation) -> BattleEngine {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut tired = build_pokemon(&dex, HEALTHY, SideId::P1, &set("Mew", &["Harden"])).unwrap();
    tired.moves[0].pp = 1;
    let pokemon = vec![
        build_pokemon(&dex, BLESSER, SideId::P1, &set("Mew", &["Harden"])).unwrap(),
        build_pokemon(&dex, FOE, SideId::P2, &set("Snorlax", &["Harden"])).unwrap(),
        tired,
    ];
    let mut p1 = SideState::new(vec![BLESSER, HEALTHY], 1);
    p1.active[0] = Some(BLESSER);
    p1.slot_heals.push(SlotHeal {
        slot: 0,
        kind: SlotHealKind::LunarDance,
        user: BLESSER,
        turns_remaining: None,
    });
    let mut p2 = SideState::new(vec![FOE], 1);
    p2.active[0] = Some(FOE);
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    let foe = SideChoice {
        side: SideId::P2,
        actions: vec![SlotChoice {
            actor: FOE,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: None,
                mechanic: None,
            },
        }],
    };
    let switch = SideChoice {
        side: SideId::P1,
        actions: vec![SlotChoice {
            actor: BLESSER,
            action: ActionChoice::Switch { pokemon: HEALTHY },
        }],
    };
    engine.resolve_turn([switch, foe]).unwrap();
    engine
}

#[test]
fn lunar_dance_restores_pp_and_counts_it_as_a_need() {
    // Gen 9: PP alone is a need; the dance is used up and the PP restored.
    let engine = lunar_dance_entry(Generation::Gen9);
    let entrant = engine.state().pokemon(HEALTHY).unwrap();
    assert_eq!(entrant.moves[0].pp, entrant.moves[0].max_pp);
    assert!(engine.state().side(SideId::P1).slot_heals.is_empty());
}

/// Two conditions on one slot: one that needs nothing is kept without stopping the other
/// (Showdown keeps one handler per condition and each asks `onSwap` for itself, §30.5). A
/// Healing Wish in front of a Lunar Dance must not swallow the dance's PP refill.
#[test]
fn a_slot_keeps_a_condition_that_needs_nothing_and_still_runs_the_next() {
    let generation = Generation::Gen9;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut tired = build_pokemon(&dex, HEALTHY, SideId::P1, &set("Mew", &["Harden"])).unwrap();
    tired.moves[0].pp = 1;
    let pokemon = vec![
        build_pokemon(&dex, BLESSER, SideId::P1, &set("Mew", &["Harden"])).unwrap(),
        build_pokemon(&dex, FOE, SideId::P2, &set("Snorlax", &["Harden"])).unwrap(),
        build_pokemon(&dex, FAINTED, SideId::P1, &set("Mew", &["Harden"])).unwrap(),
        tired,
    ];
    let mut p1 = SideState::new(vec![BLESSER, FAINTED, HEALTHY], 1);
    p1.active[0] = Some(BLESSER);
    // Healing Wish first (HP and status only), Lunar Dance behind it (PP too).
    p1.slot_heals.push(SlotHeal {
        slot: 0,
        kind: SlotHealKind::HealingWish,
        user: BLESSER,
        turns_remaining: None,
    });
    p1.slot_heals.push(SlotHeal {
        slot: 0,
        kind: SlotHealKind::LunarDance,
        user: BLESSER,
        turns_remaining: None,
    });
    let mut p2 = SideState::new(vec![FOE], 1);
    p2.active[0] = Some(FOE);
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    let foe = SideChoice {
        side: SideId::P2,
        actions: vec![SlotChoice {
            actor: FOE,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: None,
                mechanic: None,
            },
        }],
    };
    let switch = SideChoice {
        side: SideId::P1,
        actions: vec![SlotChoice {
            actor: BLESSER,
            action: ActionChoice::Switch { pokemon: HEALTHY },
        }],
    };
    engine.resolve_turn([switch, foe]).unwrap();
    let entrant = engine.state().pokemon(HEALTHY).unwrap();
    assert_eq!(
        entrant.moves[0].pp, entrant.moves[0].max_pp,
        "the Lunar Dance behind the Healing Wish did not refill the PP"
    );
    // The Healing Wish needed nothing (HP full, no status) and is still waiting.
    assert_eq!(
        engine.state().side(SideId::P1).slot_heals.len(),
        1,
        "the Healing Wish was consumed by an entrant that needed nothing"
    );
}
