//! L1 rule tests for issues.md I50 and I51 (DESIGN.md §20.1) where the replay cannot tell: it
//! searches the damage roll that reproduces Showdown's HP, so the Gen 2 cartridge's order only
//! shows with the roll held fixed — Minimize doubling Stomp as the last step, after the + 2, STAB
//! and the random roll (I50), and a type-boosting item taken before the critical hit (I51).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, RandomOverride, RandomRequestKind, SideState, VolatileConditionKind,
};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

/// The HP a Gen 2 Stomp takes off a Chansey, Minimized or not, at damage roll `roll`, no
/// critical hit, no flinch.
fn stomp_damage(minimized: bool, roll: u16) -> u16 {
    first_hit(
        ("Snorlax", "Stomp", None, 100),
        "Chansey",
        minimized,
        vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, roll),
            RandomOverride::bool(RandomRequestKind::Secondary, false),
        ],
    )
}

/// The HP the user's only move takes off a Lv. 100 `target` (whose Harden, faster, changes no
/// stat the hit reads), the semantic tape given.
fn first_hit(
    (species, move_name, item, level): (&str, &str, Option<&str>, u8),
    target_species: &str,
    minimized: bool,
    tape: Vec<RandomOverride>,
) -> u16 {
    let generation = Generation::Gen2;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut set = PokemonSet::new(species, [move_name]);
    set.level = level;
    set.item = item.map(str::to_owned);
    let user = build_pokemon(&dex, USER, SideId::P1, &set).unwrap();
    let mut target = build_pokemon(
        &dex,
        TARGET,
        SideId::P2,
        &PokemonSet::new(target_species, ["Harden"]),
    )
    .unwrap();
    if minimized {
        target.volatiles.set_marker(VolatileConditionKind::Minimize);
    }
    let before = target.hp;
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(vec![user, target], [side(USER), side(TARGET)], 1);
    state.rng = BattleRng::with_semantic_tape(1, tape);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    let first_move = |side: SideId, actor: PokemonId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: None::<DeclaredTarget>,
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([first_move(SideId::P1, USER), first_move(SideId::P2, TARGET)])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    before - engine.state().pokemon(TARGET).unwrap().hp
}

#[test]
fn gen_2_minimize_doubles_the_final_damage() {
    for roll in [217, 236, 255] {
        let plain = stomp_damage(false, roll);
        assert!(plain > 1);
        assert_eq!(stomp_damage(true, roll), 2 * plain, "roll {roll}");
    }
}

#[test]
fn gen_2_type_boosting_items_come_before_the_critical_hit() {
    let karate_chop = |critical: bool| {
        first_hit(
            ("Machamp", "Karate Chop", Some("Black Belt"), 53),
            "Blissey",
            false,
            vec![
                RandomOverride::bool(RandomRequestKind::Critical, critical),
                RandomOverride::range(RandomRequestKind::DamageRoll, 255),
            ],
        )
    };
    // 66 before the item and the critical hit: 72 × 2 = 144 (Showdown's 132 × 1.1 = 145), + 2,
    // STAB, super effective: 438 at the top roll (Showdown 441).
    // (A critical hit sets aside the Defense Blissey's Harden raised first.)
    assert_eq!(karate_chop(true), 438);
}
