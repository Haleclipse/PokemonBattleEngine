//! L1 tests for the status-immunity abilities (DESIGN.md R270): what the replay cannot see.
//! Own Tempo's `onHit` only announces that a confusion move did nothing (`-immune|p|confusion`);
//! the confusion it keeps off is its `onTryAddVolatile`, which a frame shows. Magma Armor's
//! `onImmunity('frz')` keeps a freeze off that its `onUpdate` would cure before any frame.
//! And `engine::ability_refuses_status`, the replay driver's question (DESIGN.md R277): its
//! answer only shows when the engine's own draw would have missed.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{AbilityId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleState, NonVolatileStatus, SideState, VolatileConditionKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// Jolteon (A) uses `move_name` on Snorlax (B, `ability`, Harden): the turn's trace and whether
/// B ends it confused.
fn turn(move_name: &str, ability: &str) -> (Vec<TraceEvent>, bool) {
    turn_on("Snorlax", move_name, ability, 1)
}

/// Jolteon (A) uses `move_name` on `species` (B, `ability`, Harden), the PRNG seeded with `seed`.
fn turn_on(species: &str, move_name: &str, ability: &str, seed: u64) -> (Vec<TraceEvent>, bool) {
    let generation = Generation::Gen9;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let set = |species: &str, moves: &[&str], ability: &str| {
        let mut set = PokemonSet::new(species, moves.iter().copied());
        set.level = 50;
        set.ability = Some(ability.to_string());
        set
    };
    let pokemon = vec![
        build_pokemon(
            &dex,
            A,
            SideId::P1,
            &set("Jolteon", &[move_name], "Volt Absorb"),
        )
        .unwrap(),
        build_pokemon(&dex, B, SideId::P2, &set(species, &["Harden"], ability)).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(A), side(B)], seed);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
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
            choice(SideId::P1, A, SideId::P2),
            choice(SideId::P2, B, SideId::P1),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let confused = engine
        .state()
        .pokemon(B)
        .unwrap()
        .volatiles
        .contains(VolatileConditionKind::Confusion);
    (engine.trace().events().to_vec(), confused)
}

fn own_tempo_announced(trace: &[TraceEvent]) -> bool {
    trace.iter().any(|event| {
        matches!(
            event,
            TraceEvent::AbilityActivated {
                source: B,
                ability: AbilityId::OWN_TEMPO,
            }
        )
    })
}

#[test]
fn own_tempo_announces_a_confusion_move_it_shrugs_off() {
    let (trace, confused) = turn("Confuse Ray", "Own Tempo");
    assert!(!confused);
    assert!(own_tempo_announced(&trace));
    // Without Own Tempo, Confuse Ray confuses and nothing is announced.
    let (trace, confused) = turn("Confuse Ray", "Thick Fat");
    assert!(confused);
    assert!(!own_tempo_announced(&trace));
}

#[test]
fn magma_armor_refuses_a_freeze_its_secondary_rolled() {
    // Ice Beam's 10 % freeze on Magcargo: over the seeds where the chance came up, Magma Armor
    // refuses the status itself (`StatusBlocked`), never letting it land for its Update to cure.
    let mut rolled = 0;
    for seed in 1..400 {
        let (trace, _) = turn_on("Magcargo", "Ice Beam", "Magma Armor", seed);
        if !trace.iter().any(|event| {
            matches!(
                event,
                TraceEvent::SecondaryEffectRolled { success: true, .. }
            )
        }) {
            continue;
        }
        rolled += 1;
        assert!(trace.iter().any(|event| matches!(
            event,
            TraceEvent::StatusBlocked {
                target: B,
                reason: pokemon_battle_engine::engine::StatusBlockReason::Ability(
                    AbilityId::MAGMA_ARMOR
                ),
                ..
            }
        )));
        assert!(
            !trace
                .iter()
                .any(|event| matches!(event, TraceEvent::StatusApplied { target: B, .. }))
        );
    }
    assert!(rolled > 0);
}

#[test]
fn ability_refuses_status_asks_the_holder_and_its_partners() {
    // The replay driver's question (DESIGN.md R277): will an ability refuse the status as it
    // is set, after the move's accuracy check? The holder's own `onSetStatus` (Purifying Salt)
    // and a partner's `onAllySetStatus` (Sweet Veil) both answer.
    let generation = Generation::Gen9;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let set = |species: &str, ability: &str| {
        let mut set = PokemonSet::new(species, ["Harden"]);
        set.level = 50;
        set.ability = Some(ability.to_string());
        set
    };
    let ids = [PokemonId(1), PokemonId(2), PokemonId(3), PokemonId(4)];
    let sets = [
        set("Jigglypuff", "Competitive"),
        set("Chansey", "Natural Cure"),
        set("Garganacl", "Purifying Salt"),
        set("Snorlax", "Sweet Veil"),
    ];
    let pokemon = ids
        .iter()
        .zip(&sets)
        .enumerate()
        .map(|(index, (id, set))| {
            let side = if index < 2 { SideId::P1 } else { SideId::P2 };
            build_pokemon(&dex, *id, side, set).unwrap()
        })
        .collect();
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 2);
        side.active = vec![Some(team[0]), Some(team[1])];
        side
    };
    let state = BattleState::new(pokemon, [side([ids[0], ids[1]]), side([ids[2], ids[3]])], 1);
    let refuses = |target: PokemonId, status: NonVolatileStatus| {
        pokemon_battle_engine::engine::ability_refuses_status(
            &state,
            &rules,
            target,
            status,
            pokemon_battle_engine::engine::EffectSource::Move {
                pokemon: ids[0],
                move_id: MoveId::SING,
            },
        )
    };
    // Purifying Salt on its holder; Sweet Veil on its holder's partner, for sleep alone.
    assert!(refuses(ids[2], NonVolatileStatus::Burn));
    assert!(refuses(ids[2], NonVolatileStatus::Sleep));
    assert!(!refuses(ids[3], NonVolatileStatus::Burn));
    assert!(refuses(ids[3], NonVolatileStatus::Sleep));
    // Nothing on the other side.
    assert!(!refuses(ids[1], NonVolatileStatus::Sleep));
}
