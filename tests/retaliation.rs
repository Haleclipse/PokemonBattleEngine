//! L1 tests for the retaliating moves (DESIGN.md §15.8): what the replay cannot tell apart.
//! In doubles, Counter's random target (Showdown `getRandomTarget` for `scripted`) is taken
//! from the Showdown log's final target, so a Counter that forgot to strike back at its
//! attacker would still replay; here the draw is left to the RNG.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

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

fn slot_move(actor: PokemonId, slot: u8, target: Option<DeclaredTarget>) -> SlotChoice {
    SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(slot).unwrap(),
            target,
            mechanic: None,
        },
    }
}

/// Doubles: whoever the random draw picks, Counter goes to the foe whose physical hit it
/// answers (its volatile's `onRedirectTarget`) — not to the partner that hit the user after.
#[test]
fn doubles_counter_strikes_back_at_its_attacker_whatever_the_draw() {
    for seed in 1..=12u64 {
        let rules = Arc::new(
            RulesResolver::resolve(&BattleSpec::new(
                Generation::Gen9,
                BattleMode::Doubles,
                "custom",
            ))
            .unwrap(),
        );
        let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
        let pokemon = vec![
            build_pokemon(&dex, A, SideId::P1, &set("Mew", &["Counter", "Harden"], 50)).unwrap(),
            build_pokemon(&dex, C, SideId::P1, &set("Snorlax", &["Tackle"], 50)).unwrap(),
            build_pokemon(&dex, B, SideId::P2, &set("Snorlax", &["Harden"], 55)).unwrap(),
            build_pokemon(&dex, D, SideId::P2, &set("Snorlax", &["Tackle"], 60)).unwrap(),
        ];
        let side = |team: Vec<PokemonId>| {
            let mut side = SideState::new(team.clone(), 2);
            side.active = vec![Some(team[0]), Some(team[1])];
            side
        };
        let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B, D])], seed);
        let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
        let a_position = rules.topology().position(SideId::P1, 0).unwrap();
        engine
            .submit_turn_choices([
                SideChoice {
                    side: SideId::P1,
                    actions: vec![
                        slot_move(A, 0, None),
                        slot_move(C, 0, Some(DeclaredTarget(a_position))),
                    ],
                },
                SideChoice {
                    side: SideId::P2,
                    actions: vec![
                        slot_move(B, 0, None),
                        slot_move(D, 0, Some(DeclaredTarget(a_position))),
                    ],
                },
            ])
            .unwrap();
        while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
        let countered: Vec<PokemonId> = engine
            .trace()
            .events()
            .iter()
            .filter_map(|event| match event {
                TraceEvent::DamageApplied {
                    target,
                    source: pokemon_battle_engine::engine::EffectSource::Move { pokemon: A, .. },
                    ..
                } => Some(*target),
                _ => None,
            })
            .collect();
        let damaged_by_c = engine.trace().events().iter().any(|event| {
            matches!(
                event,
                TraceEvent::DamageApplied {
                    target: A,
                    source: pokemon_battle_engine::engine::EffectSource::Move { pokemon: C, .. },
                    ..
                }
            )
        });
        assert!(
            damaged_by_c,
            "seed {seed}: the partner's Tackle hit the user"
        );
        assert_eq!(countered, vec![D], "seed {seed}");
    }
}

/// Gen 2 (pokecrystal `BattleCommand_Counter`, issues.md I40): Counter answers neither another
/// Counter — even one that dealt damage, which a single battle never sees within a turn — nor an
/// attack that dealt nothing; a damaging Tackle it doubles.
#[test]
fn gen2_counter_answers_neither_a_counter_nor_an_attack_for_nothing() {
    use pokemon_battle_engine::dex::MoveCategory;
    use pokemon_battle_engine::dex::Retaliation;
    use pokemon_battle_engine::ids::MoveId;
    use pokemon_battle_engine::state::AttackRecord;

    let answer = |generation: Generation, move_id: MoveId, damage: u16| {
        let rules =
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .unwrap();
        let dex = DexResolver::resolve(generation);
        let gen2 = |species: &str, moves: &[&str]| {
            let mut set = set(species, moves, 50);
            set.ability = None;
            set
        };
        let mut user = build_pokemon(&dex, A, SideId::P1, &gen2("Snorlax", &["Counter"])).unwrap();
        let mut foe = build_pokemon(
            &dex,
            B,
            SideId::P2,
            &gen2("Snorlax", &["Counter", "Tackle"]),
        )
        .unwrap();
        foe.last_move = Some(move_id);
        user.history.attacked_this_turn.push(AttackRecord {
            source: B,
            damage: Some(damage),
            move_id,
            position: rules.topology().position(SideId::P2, 0),
            category: MoveCategory::Physical,
            damaging_hit_only: false,
        });
        let side = |team: Vec<PokemonId>| {
            let mut side = SideState::new(team.clone(), 1);
            side.active = vec![Some(team[0])];
            side
        };
        let state = BattleState::new(vec![user, foe], [side(vec![A]), side(vec![B])], 1);
        pokemon_battle_engine::engine::retaliation_record(&state, &rules, A, Retaliation::Counter)
            .map(|(_, damage)| damage)
    };
    assert_eq!(answer(Generation::Gen2, MoveId::TACKLE, 40), Some(80));
    assert_eq!(
        answer(Generation::Gen2, MoveId::COUNTER, 40),
        None,
        "Counter against Counter"
    );
    assert_eq!(
        answer(Generation::Gen2, MoveId::TACKLE, 0),
        None,
        "an attack for nothing"
    );
}
