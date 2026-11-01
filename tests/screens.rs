//! L1 rule tests for M7 E batches 1-2 (DESIGN.md §10.7, §10.8): the screens' damage modifier —
//! the value each generation uses when the target's side counts as more than one, and where
//! Gen 3-4 put it — and Lucky Chant turning back a rolled critical hit. The replay cannot see
//! these: its damage-roll search absorbs 2730 against 2732 (one point in 2048), 2703 against
//! 2732 (about one roll) and Gen 3's `ModifyDamagePhase1` moving (a rounding), and it plans a
//! critical-hit roll from Showdown's `-crit`, which a Lucky Chant side never shows — nor the
//! `-unboost` of a Gen 2 secondary (rolled even at 100 %) that Mist stopped.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, ModifierSource, ModifierStage, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, RandomOverride, RandomRequestKind, SideConditionKind, SideState,
    VolatileConditionKind,
};

const MEW: PokemonId = PokemonId(1);
const PARTNER: PokemonId = PokemonId(2);
const LAX: PokemonId = PokemonId(3);
const CHANSEY: PokemonId = PokemonId(4);

fn set(species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some("Run Away".into());
    set.level = level;
    set
}

/// The final-damage steps of Snorlax's Strength on Mew in a doubles battle where Mew's side has
/// Reflect up, with or without Mew's partner on the field: `(source, from, to)` in the order
/// they were applied. A level 100 Snorlax against a level 1 Mew: the damage the screen weakens
/// is in the thousands, so 2730 and 2732 give different results.
fn strength_on_a_screened_mew(generation: Generation, partner: bool) -> Vec<(String, u32, u32)> {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut partner_state =
        build_pokemon(&dex, PARTNER, SideId::P1, &set("Magikarp", &["Harden"], 50)).unwrap();
    // Without a partner: Magikarp has fainted and nobody replaces it.
    if !partner {
        partner_state.hp = 0;
    }
    let pokemon = vec![
        build_pokemon(&dex, MEW, SideId::P1, &set("Mew", &["Harden"], 1)).unwrap(),
        partner_state,
        build_pokemon(&dex, LAX, SideId::P2, &set("Snorlax", &["Strength"], 100)).unwrap(),
        build_pokemon(&dex, CHANSEY, SideId::P2, &set("Chansey", &["Harden"], 50)).unwrap(),
    ];
    let mut p1 = SideState::new(vec![MEW, PARTNER], 2);
    p1.active[0] = Some(MEW);
    p1.active[1] = partner.then_some(PARTNER);
    p1.conditions.set_timed(SideConditionKind::Reflect, 5);
    let mut p2 = SideState::new(vec![LAX, CHANSEY], 2);
    p2.active[0] = Some(LAX);
    p2.active[1] = Some(CHANSEY);
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();

    let mut snapshot = engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, rules_roll_max(generation)),
        ],
    );
    engine = BattleEngine::restore(rules.clone(), dex.clone(), snapshot).unwrap();
    let harden = |actor| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    };
    let mut p1_actions = vec![harden(MEW)];
    if partner {
        p1_actions.push(harden(PARTNER));
    }
    let strength = SlotChoice {
        actor: LAX,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: Some(DeclaredTarget(
                rules.topology().position(SideId::P1, 0).unwrap(),
            )),
            mechanic: None,
        },
    };
    let start = engine.trace().events().len();
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: p1_actions,
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![strength, harden(CHANSEY)],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert!(
        engine.state().rng.semantic_tape_exhausted(),
        "tape not fully consumed"
    );
    engine.trace().events()[start..]
        .iter()
        .filter_map(|event| match event {
            TraceEvent::ModifierApplied {
                stage: ModifierStage::FinalDamage,
                source,
                from,
                to,
                ..
            } => Some((
                match source {
                    ModifierSource::Rule(name) => (*name).to_string(),
                    other => format!("{other:?}"),
                },
                *from,
                *to,
            )),
            _ => None,
        })
        .collect()
}

/// The top of the generation's damage roll (the tape fixes it; its value does not matter here).
fn rules_roll_max(generation: Generation) -> u16 {
    RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
        .unwrap()
        .damage()
        .random_max
}

/// Showdown `modify`.
fn modify(value: u32, modifier: u32) -> u32 {
    (value * modifier + 2047) / 4096
}

#[test]
fn the_crowded_screen_modifier_follows_the_generation() {
    // (generation, the step the screen is in, the damage with Mew's partner standing, without):
    // Showdown's `modify`, or the Gen 4 cartridge's `damage * 2 / 3` and `damage / 2`.
    type Weaken = fn(u32) -> u32;
    let cases: [(Generation, &str, Weaken, Weaken); 5] = [
        (
            Generation::Gen3,
            "ModifyDamagePhase1",
            |d| modify(d, 2730),
            |d| modify(d, 2048),
        ),
        (
            Generation::Gen4,
            "ModifyDamagePhase1",
            |d| d * 2 / 3,
            |d| d / 2,
        ),
        (
            Generation::Gen5,
            "ModifyDamage",
            |d| modify(d, 2703),
            |d| modify(d, 2703),
        ),
        (
            Generation::Gen6,
            "ModifyDamage",
            |d| modify(d, 2732),
            |d| modify(d, 2732),
        ),
        (
            Generation::Gen9,
            "ModifyDamage",
            |d| modify(d, 2732),
            |d| modify(d, 2732),
        ),
    ];
    for (generation, step, crowded, alone) in cases {
        for (partner, weaken) in [(true, crowded), (false, alone)] {
            let steps = strength_on_a_screened_mew(generation, partner);
            let (_, from, to) = steps
                .iter()
                .find(|(source, from, to)| source == step && from != to)
                .unwrap_or_else(|| panic!("{generation:?} {partner}: no screen in {step}"));
            assert!(
                *from > 2048,
                "{generation:?}: {from} too small to tell 2730 from 2732"
            );
            assert_eq!(
                *to,
                weaken(*from),
                "{generation:?}, partner {partner}: {from} → {to}"
            );
        }
    }
}

#[test]
fn gen_3_and_4_weaken_the_hit_right_after_the_burn() {
    // `modifyDamage`: burn → ModifyDamagePhase1 (the screens) → spread → weather → + 2 …
    for generation in [Generation::Gen3, Generation::Gen4] {
        let steps = strength_on_a_screened_mew(generation, true);
        let names: Vec<&str> = steps.iter().map(|(source, _, _)| source.as_str()).collect();
        let burn = names.iter().position(|name| *name == "Burn").unwrap();
        assert_eq!(
            &names[burn..burn + 3],
            ["Burn", "ModifyDamagePhase1", "Spread"],
            "{generation:?}: {names:?}"
        );
        let base = steps
            .iter()
            .find(|(source, _, _)| source == "BaseOffset")
            .unwrap();
        let phase1 = &steps[burn + 1];
        assert!(phase1.2 < phase1.1, "{generation:?}: {phase1:?}");
        assert!(phase1.2 <= base.1, "{generation:?}: before the + 2");
    }
}

/// Snorlax's Strength on Mew in a Gen 4 singles battle, the critical-hit roll landing, with or
/// without Lucky Chant on Mew's side: whether the hit was critical.
fn gen4_critical_strength(lucky_chant: bool) -> bool {
    let generation = Generation::Gen4;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, MEW, SideId::P1, &set("Mew", &["Harden"], 50)).unwrap(),
        build_pokemon(&dex, LAX, SideId::P2, &set("Snorlax", &["Strength"], 50)).unwrap(),
    ];
    let mut p1 = SideState::new(vec![MEW], 1);
    p1.active[0] = Some(MEW);
    if lucky_chant {
        p1.conditions.set_timed(SideConditionKind::LuckyChant, 5);
    }
    let mut p2 = SideState::new(vec![LAX], 1);
    p2.active[0] = Some(LAX);
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let mut snapshot = engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Critical, true),
            RandomOverride::range(RandomRequestKind::DamageRoll, rules_roll_max(generation)),
        ],
    );
    engine = BattleEngine::restore(rules.clone(), dex, snapshot).unwrap();
    let choice = |side, actor, target: Option<(SideId, u8)>| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: target
                    .map(|(s, i)| DeclaredTarget(rules.topology().position(s, i).unwrap())),
                mechanic: None,
            },
        }],
    };
    let start = engine.trace().events().len();
    engine
        .submit_turn_choices([
            choice(SideId::P1, MEW, None),
            choice(SideId::P2, LAX, Some((SideId::P1, 0))),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert!(
        engine.state().rng.semantic_tape_exhausted(),
        "tape not fully consumed"
    );
    engine.trace().events()[start..]
        .iter()
        .find_map(|event| match event {
            TraceEvent::CriticalChecked { critical, .. } => Some(*critical),
            _ => None,
        })
        .expect("a critical-hit check")
}

#[test]
fn lucky_chant_turns_a_rolled_critical_hit_back() {
    // Gen 4 has no move that always crits: the roll is drawn and lands, and Lucky Chant
    // (`onCriticalHit: false`) turns it back, like Battle Armor.
    assert!(gen4_critical_strength(false));
    assert!(!gen4_critical_strength(true));
}

/// A damaging move's Speed drop (its secondary rolled and landing) on Snorlax, which has Mist
/// (the Gen 1-2 volatile): Snorlax's Speed stage afterwards.
fn speed_drop_through_mist(generation: Generation, attack: &str) -> i8 {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    // No abilities before Gen 3.
    let early = |species: &str, moves: &[&str], level| {
        let mut set = set(species, moves, level);
        set.ability = None;
        set
    };
    let mut snorlax =
        build_pokemon(&dex, MEW, SideId::P1, &early("Snorlax", &["Harden"], 100)).unwrap();
    assert!(
        snorlax
            .volatiles
            .set_user_guard(VolatileConditionKind::Mist)
    );
    let pokemon = vec![
        snorlax,
        build_pokemon(&dex, LAX, SideId::P2, &early("Starmie", &[attack], 50)).unwrap(),
    ];
    let mut p1 = SideState::new(vec![MEW], 1);
    p1.active[0] = Some(MEW);
    let mut p2 = SideState::new(vec![LAX], 1);
    p2.active[0] = Some(LAX);
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let mut snapshot = engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Accuracy, true),
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, rules_roll_max(generation)),
            RandomOverride::bool(RandomRequestKind::Secondary, true),
        ],
    );
    engine = BattleEngine::restore(rules.clone(), dex, snapshot).unwrap();
    let choice = |side, actor, target: Option<(SideId, u8)>| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: target
                    .map(|(s, i)| DeclaredTarget(rules.topology().position(s, i).unwrap())),
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, MEW, None),
            choice(SideId::P2, LAX, Some((SideId::P1, 0))),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    assert!(
        engine.state().rng.semantic_tape_exhausted(),
        "tape not fully consumed"
    );
    engine.state().pokemon(MEW).unwrap().stages.speed
}

#[test]
fn gen_2_mist_stops_a_damaging_moves_drop_and_gen_1_mist_does_not() {
    // Gen 1's `onTryBoost` lets a damaging move's drop through; Gen 2's stops every drop another
    // Pokémon causes (Icy Wind's 100 % is still a 255 / 256 roll there, so the replay plans it
    // from an `-unboost` that never comes).
    assert_eq!(speed_drop_through_mist(Generation::Gen1, "Bubble Beam"), -1);
    assert_eq!(speed_drop_through_mist(Generation::Gen2, "Icy Wind"), 0);
}
