//! L1 rule test for Gen 1's zero damage (`HitRules::zero_damage_misses`, issues.md I221): pokered's
//! `AdjustDamageForMoveType` makes a move miss when the type chart takes its damage to 0 — a 2 or
//! 3 at 0.25× — so a crash move crashes. The replay cannot tell this from an accuracy miss (it
//! plans the accuracy from the `-miss` line), so the decision is driven here through the tape:
//! the accuracy check passes and the critical-hit check is made before the damage comes to 0.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, MoveFailReason, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, RandomOverride, RandomRequestKind, SideState,
};

const KICKER: PokemonId = PokemonId(1);
const BAT: PokemonId = PokemonId(2);

#[test]
fn a_jump_kick_the_type_chart_takes_to_zero_misses_and_crashes() {
    let generation = Generation::Gen1;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut kicker = PokemonSet::new("Hitmonlee", ["Jump Kick"]);
    kicker.level = 5;
    let kicker = build_pokemon(&dex, KICKER, SideId::P1, &kicker).unwrap();
    let bat = build_pokemon(
        &dex,
        BAT,
        SideId::P2,
        &PokemonSet::new("Golbat", ["Splash"]),
    )
    .unwrap();
    let kicker_hp = kicker.max_hp;
    let bat_hp = bat.max_hp;
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![kicker, bat], [side(KICKER), side(BAT)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let mut snapshot = engine.snapshot();
    // The Jump Kick's accuracy passes and its critical-hit check fails; a damage of 3 at 0.25×
    // asks for no damage roll.
    snapshot.state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Accuracy, true),
            RandomOverride::bool(RandomRequestKind::Critical, false),
        ],
    );
    let mut engine = BattleEngine::restore(rules.clone(), dex, snapshot).unwrap();
    let start = engine.trace().events().len();
    let target = |side: SideId| DeclaredTarget(rules.topology().position(side, 0).unwrap());
    let choice = |side: SideId, actor: PokemonId, at: SideId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: Some(target(at)),
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, KICKER, SideId::P2),
            choice(SideId::P2, BAT, SideId::P1),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert!(engine.state().rng.semantic_tape_exhausted());

    let events = &engine.trace().events()[start..];
    assert!(events.iter().any(|event| matches!(
        event,
        TraceEvent::MoveFailed {
            actor: KICKER,
            reason: MoveFailReason::ZeroDamage,
            ..
        }
    )));
    assert_eq!(engine.state().pokemon(BAT).unwrap().hp, bat_hp);
    // Gen 1's crash: 1 HP.
    assert_eq!(engine.state().pokemon(KICKER).unwrap().hp, kicker_hp - 1);
}
