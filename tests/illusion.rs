//! L1 tests for Illusion (DESIGN.md R-K.10c): the disguise is what one side sees
//! (`BattleEngine::view_for`), which no frame shows — the replay only meets the Transform it
//! makes fail.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{AbilityId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, PokemonState, SideState};

const ZOROARK: PokemonId = PokemonId(1);
const CHANSEY: PokemonId = PokemonId(2);
const SNORLAX: PokemonId = PokemonId(3);
const FOE: PokemonId = PokemonId(4);

fn set(species: &str, moves: &[&str], ability: &str) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.level = 50;
    set
}

/// Zoroark (Illusion, Harden) leads Chansey and Snorlax against `foe`.
fn battle(
    foe: PokemonSet,
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> (Arc<ResolvedRules>, BattleEngine) {
    let generation = Generation::Gen9;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let sets = [
        (ZOROARK, SideId::P1, set("Zoroark", &["Harden"], "Illusion")),
        (
            CHANSEY,
            SideId::P1,
            set("Chansey", &["Harden"], "Natural Cure"),
        ),
        (
            SNORLAX,
            SideId::P1,
            set("Snorlax", &["Harden"], "Thick Fat"),
        ),
        (FOE, SideId::P2, foe),
    ];
    let mut pokemon: Vec<PokemonState> = sets
        .iter()
        .map(|(id, side, set)| build_pokemon(&dex, *id, *side, set).unwrap())
        .collect();
    prepare(&mut pokemon);
    let mut p1 = SideState::new(vec![ZOROARK, CHANSEY, SNORLAX], 1);
    p1.active[0] = Some(ZOROARK);
    let mut p2 = SideState::new(vec![FOE], 1);
    p2.active[0] = Some(FOE);
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    engine.run_lead_switch_ins();
    (rules, engine)
}

/// Whom the foe sees in Zoroark's place, and whether it sees Zoroark itself.
fn seen_by_foe(engine: &BattleEngine) -> (PokemonId, Option<PokemonId>) {
    let view = engine.view_for(SideId::P2);
    let seen = view
        .active
        .iter()
        .filter_map(|(_, seen)| *seen)
        .find(|seen| {
            seen.hp == engine.state().pokemon(ZOROARK).unwrap().hp && seen.pokemon != Some(FOE)
        })
        .unwrap();
    (seen.appears_as, seen.pokemon)
}

fn turn(rules: &ResolvedRules, engine: &mut BattleEngine) {
    let choice = |side: SideId, actor: PokemonId, target: SideId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: Some(DeclaredTarget(
                    rules.topology().position(target, 0).unwrap(),
                )),
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, ZOROARK, SideId::P2),
            choice(SideId::P2, FOE, SideId::P1),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
}

fn lifted(engine: &BattleEngine) -> bool {
    engine
        .trace()
        .events()
        .iter()
        .any(|event| matches!(event, TraceEvent::IllusionEnded { holder } if *holder == ZOROARK))
}

#[test]
fn the_foe_sees_the_last_party_member_standing_until_a_hit() {
    let (rules, mut engine) = battle(set("Blissey", &["Tackle"], "Natural Cure"), |_| {});
    assert_eq!(seen_by_foe(&engine), (SNORLAX, None));
    // Its own side sees it as it is.
    let own = engine.view_for(SideId::P1);
    let mine = own.active.iter().find_map(|(_, seen)| *seen).unwrap();
    assert_eq!((mine.appears_as, mine.pokemon), (ZOROARK, Some(ZOROARK)));
    turn(&rules, &mut engine);
    assert!(lifted(&engine));
    assert_eq!(seen_by_foe(&engine), (ZOROARK, Some(ZOROARK)));
}

#[test]
fn a_fainted_last_member_is_passed_over() {
    let (_, engine) = battle(set("Blissey", &["Harden"], "Natural Cure"), |pokemon| {
        pokemon[2].hp = 0;
    });
    assert_eq!(seen_by_foe(&engine), (CHANSEY, None));
}

#[test]
fn neutralizing_gas_lifts_it_and_keeps_it_off() {
    // The gas holder leads after Zoroark took its disguise: its switch-in lifts it.
    let (_, engine) = battle(set("Weezing", &["Harden"], "Neutralizing Gas"), |_| {});
    assert!(lifted(&engine));
    assert_eq!(seen_by_foe(&engine), (ZOROARK, Some(ZOROARK)));
}

#[test]
fn gastro_acid_lifts_it() {
    let (rules, mut engine) = battle(set("Blissey", &["Gastro Acid"], "Natural Cure"), |_| {});
    assert_eq!(seen_by_foe(&engine), (SNORLAX, None));
    turn(&rules, &mut engine);
    assert!(lifted(&engine));
}

#[test]
fn it_comes_back_in_disguised() {
    let (rules, mut engine) = battle(set("Blissey", &["Tackle"], "Natural Cure"), |_| {});
    turn(&rules, &mut engine);
    assert_eq!(seen_by_foe(&engine), (ZOROARK, Some(ZOROARK)));
    let switch = |engine: &mut BattleEngine, actor: PokemonId, to: PokemonId| {
        let tackle = SideChoice {
            side: SideId::P2,
            actions: vec![SlotChoice {
                actor: FOE,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(0).unwrap(),
                    target: Some(DeclaredTarget(
                        rules.topology().position(SideId::P1, 0).unwrap(),
                    )),
                    mechanic: None,
                },
            }],
        };
        let out = SideChoice {
            side: SideId::P1,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Switch { pokemon: to },
            }],
        };
        engine.submit_turn_choices([out, tackle]).unwrap();
        while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    };
    switch(&mut engine, ZOROARK, CHANSEY);
    switch(&mut engine, CHANSEY, ZOROARK);
    // In again, it takes a disguise again before the Tackle lifts it once more.
    let disguised_again = engine
        .trace()
        .events()
        .iter()
        .filter(|event| matches!(event, TraceEvent::IllusionEnded { holder } if *holder == ZOROARK))
        .count();
    assert_eq!(disguised_again, 2);
}

#[test]
fn without_the_ability_nothing_is_disguised() {
    let (_, engine) = battle(set("Blissey", &["Harden"], "Natural Cure"), |pokemon| {
        pokemon[0].ability = AbilityId::THICK_FAT;
    });
    assert_eq!(seen_by_foe(&engine), (ZOROARK, Some(ZOROARK)));
}

/// Who stands on P1's side for `observer` (the stable API's view, DESIGN.md §47.0 item 9): the
/// id and species it appears with, and its HP as shown.
fn appears(
    engine: &BattleEngine,
    observer: Option<pokemon_battle_engine::view::Observer>,
) -> (
    PokemonId,
    Option<pokemon_battle_engine::ids::SpeciesId>,
    pokemon_battle_engine::view::Hp,
) {
    let view = match observer {
        Some(observer) => engine.view(observer),
        None => engine.omniscient_view(),
    };
    let standing = view
        .side(SideId::P1)
        .active()
        .find_map(|(_, standing)| standing)
        .unwrap();
    (standing.id(), standing.species(), standing.hp())
}

#[test]
fn the_view_shows_the_disguise_to_whom_it_fools() {
    use pokemon_battle_engine::view::{Hp, Observer};
    let (rules, mut engine) = battle(set("Blissey", &["Tackle"], "Natural Cure"), |_| {});
    let species =
        |id| pokemon_battle_engine::engine::species_of(engine.state().pokemon(id).unwrap());
    let (snorlax, zoroark) = (species(SNORLAX), species(ZOROARK));
    // Its own side and the omniscient eye see Zoroark; the foe and a spectator, Snorlax — with
    // Zoroark's own HP.
    for observer in [Some(Observer::Side(SideId::P1)), None] {
        let (id, seen, hp) = appears(&engine, observer);
        assert_eq!((id, seen), (ZOROARK, zoroark));
        assert!(matches!(hp, Hp::Exact { .. }));
    }
    // A custom game shares every HP exactly (Showdown's `debug` formats): Zoroark's own.
    let zoroark_hp = {
        let zoroark = engine.state().pokemon(ZOROARK).unwrap();
        Hp::Exact {
            hp: zoroark.hp,
            max: zoroark.max_hp,
        }
    };
    for observer in [Observer::Side(SideId::P2), Observer::Spectator] {
        assert_eq!(
            appears(&engine, Some(observer)),
            (SNORLAX, snorlax, zoroark_hp)
        );
    }
    turn(&rules, &mut engine);
    assert!(lifted(&engine));
    // Lifted: everyone sees Zoroark.
    let (id, seen, _) = appears(&engine, Some(Observer::Side(SideId::P2)));
    assert_eq!((id, seen), (ZOROARK, zoroark));
}
