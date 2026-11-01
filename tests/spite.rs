//! L1 tests for Spite (DESIGN.md §15.15): what the replay cannot see — whether a Spite that fails
//! draws its amount (Gen 2-3: the cartridge draws only once the checks passed; the replay plans a
//! draw only from a Spite that took PP), and the slot's `used` mark (Last Resort), which no frame
//! shows.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, RandomOverride, RandomRequestKind, SideState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// One turn: a faster Gengar uses Spite on a Snorlax whose last move is Tackle (`last`) with `pp`
/// left, which then uses Harden. Returns Tackle's PP and `used` mark afterwards, and whether the
/// planned draw was taken.
fn spite(generation: Generation, last: bool, pp: u8, draw: u16) -> (u8, bool, bool) {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut spiter = PokemonSet::new("Gengar", ["Spite"]);
    spiter.level = 100;
    let mut target = PokemonSet::new("Snorlax", ["Tackle", "Harden"]);
    target.level = 50;
    let a = build_pokemon(&dex, A, SideId::P1, &spiter).unwrap();
    let mut b = build_pokemon(&dex, B, SideId::P2, &target).unwrap();
    b.moves[0].pp = pp;
    b.last_move = last.then_some(MoveId::TACKLE);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(vec![a, b], [side(A), side(B)], 1);
    // An accuracy check where the generation draws one (Gen 2's 255 / 256), then the amount.
    state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Accuracy, true).optional(),
            RandomOverride::range(RandomRequestKind::PpDrop, draw),
        ],
    );
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let choice =
        |side: SideId, actor: PokemonId, slot: u8, target: Option<DeclaredTarget>| SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        };
    let foe = Some(DeclaredTarget(
        rules.topology().position(SideId::P2, 0).unwrap(),
    ));
    engine
        .submit_turn_choices([
            choice(SideId::P1, A, 0, foe),
            choice(SideId::P2, B, 1, None),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let tackle = engine.state().pokemon(B).unwrap().moves[0];
    (
        tackle.pp,
        tackle.used,
        engine.state().rng.semantic_tape_exhausted(),
    )
}

#[test]
fn spite_draws_its_amount_only_once_its_checks_passed() {
    // (generation, last move Tackle, its PP, draw planned) → (PP after, used, draw taken)
    let cases = [
        // Gen 2: 1 PP is enough; the draw takes what is left.
        ((Generation::Gen2, true, 1, 5), (0, true, true)),
        // No last move: no draw.
        ((Generation::Gen2, false, 10, 3), (10, false, false)),
        // Gen 3 on the cartridge (issues.md I71): it needs 2 PP; at 1 it fails without a draw.
        ((Generation::Gen3, true, 1, 2), (1, false, false)),
        ((Generation::Gen3, true, 3, 2), (1, true, true)),
        // Gen 4+: 4, nothing drawn.
        ((Generation::Gen4, true, 3, 2), (0, true, false)),
        // Showdown `deductPP` marks the slot used before it finds no PP to take.
        ((Generation::Gen9, true, 0, 2), (0, true, false)),
    ];
    for ((generation, last, pp, draw), expected) in cases {
        assert_eq!(
            spite(generation, last, pp, draw),
            expected,
            "{generation:?}, last move {last}, {pp} PP"
        );
    }
}
