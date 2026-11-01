//! L1 tests for the hard lock (DESIGN.md §15.12): what the replay cannot submit. Showdown's
//! request offers a locked Pokémon nothing but the locked move, with no switch and no mechanic,
//! so a looser `choice::validate_move` would still replay every scenario; the frames show no PP
//! either.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, ChoiceError, EffectSource, EngineStep, MechanicChoice, SideChoice, SlotChoice,
    TraceEvent,
};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, PokemonState, SideState, VolatileConditionKind};

const A: PokemonId = PokemonId(1);
const C: PokemonId = PokemonId(2);
const B: PokemonId = PokemonId(3);
const D: PokemonId = PokemonId(4);

fn set(species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some("Run Away".into());
    set.level = level;
    set
}

/// A (Rollout, Tackle; accuracy +6 so that Rollout never misses) and C on P1, B and D on P2;
/// the first `slots` of each side active.
fn engine(mode: BattleMode, format: &str) -> (Arc<ResolvedRules>, Arc<ResolvedDex>, BattleEngine) {
    let rules =
        Arc::new(RulesResolver::resolve(&BattleSpec::new(Generation::Gen9, mode, format)).unwrap());
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let slots = usize::from(rules.topology().active_slots_per_side());
    let mut user =
        build_pokemon(&dex, A, SideId::P1, &set("Mew", &["Rollout", "Tackle"], 20)).unwrap();
    user.stages.accuracy = 6;
    let pokemon: Vec<PokemonState> = vec![
        user,
        build_pokemon(&dex, C, SideId::P1, &set("Snorlax", &["Harden"], 100)).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &set("Snorlax", &["Harden"], 100)).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &set("Snorlax", &["Harden"], 100)).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), slots);
        for (slot, id) in team.iter().take(slots).enumerate() {
            side.active[slot] = Some(*id);
        }
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B, D])], 4);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    (rules, dex, engine)
}

fn rollout(
    slot: u8,
    target: Option<DeclaredTarget>,
    mechanic: Option<MechanicChoice>,
) -> SlotChoice {
    SlotChoice {
        actor: A,
        action: ActionChoice::Move {
            slot: MoveSlot::new(slot).unwrap(),
            target,
            mechanic,
        },
    }
}

fn harden(actor: PokemonId) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    }
}

fn run_turn(engine: &mut BattleEngine, p1: Vec<SlotChoice>, p2: Vec<SlotChoice>) -> usize {
    let start = engine.trace().events().len();
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: p1,
            },
            SideChoice {
                side: SideId::P2,
                actions: p2,
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    start
}

fn validate(engine: &BattleEngine, action: SlotChoice) -> Result<(), ChoiceError> {
    engine.validate_choice(
        SideId::P1,
        &SideChoice {
            side: SideId::P1,
            actions: vec![action],
        },
    )
}

#[test]
fn a_rollout_chain_holds_its_user_to_the_move_without_pp_until_the_fifth_hit() {
    let (rules, dex, mut engine) = engine(BattleMode::Singles, "custom");
    run_turn(&mut engine, vec![rollout(0, None, None)], vec![harden(B)]);
    let pp_after_first = engine.state().pokemon(A).unwrap().moves[0].pp;
    assert!(
        engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .rolling()
            .is_some()
    );

    // The request holds Rollout alone: no switch, no other move, no mechanic.
    assert_eq!(
        validate(
            &engine,
            SlotChoice {
                actor: A,
                action: ActionChoice::Switch { pokemon: C }
            }
        ),
        Err(ChoiceError::Trapped {
            actor: A,
            by: VolatileConditionKind::Rolling
        })
    );
    assert_eq!(
        validate(&engine, rollout(1, None, None)),
        Err(ChoiceError::MoveDisabled {
            actor: A,
            move_id: MoveId::TACKLE,
            by: VolatileConditionKind::Rolling
        })
    );
    assert_eq!(
        validate(
            &engine,
            rollout(0, None, Some(MechanicChoice::Terastallize))
        ),
        Err(ChoiceError::MechanicUnavailable(
            MechanicChoice::Terastallize
        ))
    );

    // Its later turns spend no PP: a Rollout out of PP still goes on.
    let mut snapshot = engine.snapshot();
    snapshot.state.pokemon_mut(A).unwrap().moves[0].pp = 0;
    let mut engine = BattleEngine::restore(rules, dex, snapshot).unwrap();
    assert!(pp_after_first > 0);
    assert_eq!(validate(&engine, rollout(0, None, None)), Ok(()));
    for hit in 2..=5u8 {
        run_turn(&mut engine, vec![rollout(0, None, None)], vec![harden(B)]);
        assert_eq!(
            engine.state().pokemon(A).unwrap().moves[0].pp,
            0,
            "hit {hit}"
        );
        let rolling = engine.state().pokemon(A).unwrap().volatiles.rolling();
        if hit < 5 {
            assert_eq!(rolling.map(|(_, hits, _)| hits), Some(hit));
        } else {
            assert_eq!(rolling, None, "the fifth hit ends the chain");
        }
    }
    // Free again.
    assert_eq!(
        validate(
            &engine,
            SlotChoice {
                actor: A,
                action: ActionChoice::Switch { pokemon: C }
            }
        ),
        Ok(())
    );
}

#[test]
fn a_doubles_rollout_chain_keeps_to_the_position_it_started_on() {
    let (rules, _, mut engine) = engine(BattleMode::Doubles, "custom");
    let foe = |slot| {
        Some(DeclaredTarget(
            rules.topology().position(SideId::P2, slot).unwrap(),
        ))
    };
    run_turn(
        &mut engine,
        vec![rollout(0, foe(1), None), harden(C)],
        vec![harden(B), harden(D)],
    );
    // The later action names the other foe; the chain still goes where it started.
    let start = run_turn(
        &mut engine,
        vec![rollout(0, foe(0), None), harden(C)],
        vec![harden(B), harden(D)],
    );
    let struck: Vec<PokemonId> = engine.trace().events()[start..]
        .iter()
        .filter_map(|event| match event {
            TraceEvent::DamageApplied {
                target,
                source: EffectSource::Move { pokemon: A, .. },
                ..
            } => Some(*target),
            _ => None,
        })
        .collect();
    assert_eq!(struck, vec![D]);
}
