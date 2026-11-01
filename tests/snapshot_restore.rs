//! DESIGN.md §11.2: a battle's start and a battle's moment are checked apart. `create_new`
//! takes a coherent state (invariants); `restore` takes a snapshot of a battle under way — taken
//! under these rules and dex at a moment a battle hands control back (integrity), in a coherent
//! state (invariants). Neither reads the format's registration rules (team size, Item Clause):
//! they hold of the teams as registered, before the battle (the pre-battle session, §45.5), and
//! the battle itself may break them.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleInitError, EngineStep, SideChoice, SlotChoice,
};
use pokemon_battle_engine::ids::{ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattlePhase, BattleState, SideState};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);
const D: PokemonId = PokemonId(4);

/// Gen 9 doubles.
fn vgc() -> (Arc<ResolvedRules>, Arc<ResolvedDex>) {
    let rules = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen9,
        BattleMode::Doubles,
        "custom",
    ))
    .unwrap();
    (
        Arc::new(rules),
        Arc::new(DexResolver::resolve(Generation::Gen9)),
    )
}

fn holding(species: &str, moves: &[&str], item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.item = item.map(str::to_string);
    set
}

/// P1: A (Leftovers), B (Choice Scarf); P2: C (Choice Scarf), D. Every team has one of each
/// item, as Item Clause wants.
fn teams(dex: &ResolvedDex) -> BattleState {
    let sets = [
        (
            A,
            SideId::P1,
            holding("Espeon", &["Trick"], Some("Leftovers")),
        ),
        (
            B,
            SideId::P1,
            holding("Chansey", &["Harden"], Some("Choice Scarf")),
        ),
        (
            C,
            SideId::P2,
            holding("Muk", &["Harden"], Some("Choice Scarf")),
        ),
        (D, SideId::P2, holding("Snorlax", &["Harden"], None)),
    ];
    let pokemon = sets
        .iter()
        .map(|(id, side, set)| build_pokemon(dex, *id, *side, set).unwrap())
        .collect();
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 2);
        side.active = vec![Some(team[0]), Some(team[1])];
        side
    };
    BattleState::new(pokemon, [side([A, B]), side([C, D])], 1)
}

fn first_move(actor: PokemonId, target: Option<DeclaredTarget>) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target,
            mechanic: None,
        },
    }
}

/// A battle in which A Tricks C: P1 ends the turn with two Choice Scarves.
fn after_the_trick() -> (Arc<ResolvedRules>, Arc<ResolvedDex>, BattleEngine) {
    let (rules, dex) = vgc();
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), teams(&dex)).unwrap();
    engine.run_lead_switch_ins();
    let c = Some(DeclaredTarget(
        rules.topology().position(SideId::P2, 0).unwrap(),
    ));
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![first_move(A, c), first_move(B, None)],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![first_move(C, None), first_move(D, None)],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let held = |id| engine.state().pokemon(id).unwrap().held_item;
    assert_eq!(held(A), Some(ItemId::CHOICE_SCARF));
    assert_eq!(held(B), Some(ItemId::CHOICE_SCARF));
    assert_eq!(held(C), Some(ItemId::LEFTOVERS));
    (rules, dex, engine)
}

/// The battle broke Item Clause by playing, not by cheating: its snapshot restores, and the
/// restored battle is the same battle.
#[test]
fn a_trick_that_leaves_a_side_two_of_an_item_restores() {
    let (rules, dex, engine) = after_the_trick();
    let snapshot = engine.snapshot();
    let restored = BattleEngine::restore(rules, dex, snapshot.clone()).unwrap();
    assert_eq!(restored.snapshot(), snapshot);
}

/// A snapshot edited into a phase its pending execution is not in is refused.
#[test]
fn a_snapshot_in_a_phase_a_battle_does_not_stop_in_is_refused() {
    let (rules, dex) = vgc();
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), teams(&dex)).unwrap();
    for phase in [
        BattlePhase::TurnResolution,
        BattlePhase::Revive,
        BattlePhase::EndTurn,
    ] {
        let mut snapshot = engine.snapshot();
        snapshot.state.phase = phase;
        assert_eq!(
            BattleEngine::restore(rules.clone(), dex.clone(), snapshot).err(),
            Some(BattleInitError::SnapshotPhaseMismatch { phase }),
            "{phase:?}"
        );
    }
    // The phase the battle is in restores.
    assert!(BattleEngine::restore(rules, dex, engine.snapshot()).is_ok());
}

/// At every moment control comes back the faints are resolved: a fainted Pokémon left on the
/// field is no state a battle stops in.
#[test]
fn a_snapshot_with_a_fainted_pokemon_on_the_field_is_refused() {
    let (rules, dex, engine) = after_the_trick();
    let mut snapshot = engine.snapshot();
    assert_eq!(snapshot.state.phase, BattlePhase::TurnChoice);
    snapshot.state.pokemon_mut(C).unwrap().hp = 0;
    assert_eq!(
        BattleEngine::restore(rules, dex, snapshot).err(),
        Some(BattleInitError::ActivePokemonFainted(C))
    );
}

/// A battle decided inside a call that is not a step — Gen 2's replacement right after the move
/// that knocked out the last but one, the replacement knocked out by Spikes on its way in — is
/// finished with nothing left pending, like every finished battle, and its snapshot restores
/// (`work/m9/restore-check.sh` met it in three Gen 2 campaigns).
#[test]
fn a_battle_decided_by_a_replacements_spikes_restores() {
    use pokemon_battle_engine::engine::{BattleRequest, ForcedReplacement, ForcedSwitchChoice};
    use pokemon_battle_engine::state::BattleOutcome;

    let generation = Generation::Gen2;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let build = |id, side, species: &str, moves: &[&str]| {
        build_pokemon(
            &dex,
            id,
            side,
            &PokemonSet::new(species, moves.iter().copied()),
        )
        .unwrap()
    };
    let mut pokemon = vec![
        build(A, SideId::P1, "Cloyster", &["Spikes", "Swift"]),
        build(B, SideId::P2, "Caterpie", &["Harden"]),
        build(C, SideId::P2, "Weedle", &["Harden"]),
    ];
    pokemon[1].hp = 1;
    pokemon[2].hp = 1;
    let mut p1 = SideState::new(vec![A], 1);
    p1.active[0] = Some(A);
    let mut p2 = SideState::new(vec![B, C], 1);
    p2.active[0] = Some(B);
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    let foe = Some(DeclaredTarget(
        rules.topology().position(SideId::P2, 0).unwrap(),
    ));
    let turn = |engine: &mut BattleEngine, slot: u8, target| {
        engine
            .submit_turn_choices([
                SideChoice {
                    side: SideId::P1,
                    actions: vec![SlotChoice {
                        actor: A,
                        action: ActionChoice::Move {
                            slot: MoveSlot::new(slot).unwrap(),
                            target,
                            mechanic: None,
                        },
                    }],
                },
                SideChoice {
                    side: SideId::P2,
                    actions: vec![first_move(B, None)],
                },
            ])
            .unwrap();
    };
    // Spikes, then Swift (which never misses in Gen 2) on the last but one.
    turn(&mut engine, 0, None);
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    turn(&mut engine, 1, foe);
    let step = loop {
        match engine.step().unwrap() {
            EngineStep::Progressed { .. } => continue,
            step => break step,
        }
    };
    let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) = step else {
        panic!("{step:?}");
    };
    let position = replacements[1].positions[0].position;
    let p1_none = ForcedSwitchChoice {
        side: SideId::P1,
        replacements: vec![],
    };
    let result = engine
        .resolve_forced_switches([
            p1_none,
            ForcedSwitchChoice {
                side: SideId::P2,
                replacements: vec![ForcedReplacement {
                    position,
                    pokemon: C,
                }],
            },
        ])
        .unwrap();
    assert_eq!(result.phase, BattlePhase::Finished);
    assert_eq!(result.outcome, Some(BattleOutcome::Winner(SideId::P1)));
    let snapshot = engine.snapshot();
    let restored = BattleEngine::restore(rules, dex, snapshot.clone()).unwrap();
    assert_eq!(restored.snapshot(), snapshot);
}
