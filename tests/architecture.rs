use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonType};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleInitError, BattleRequest, ChoiceError, EngineStep, ForcedReplacement,
    ForcedSwitchChoice, ModifierSource, ModifierStage, SideChoice, SlotChoice, StatusBlockReason,
    TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    FormatRules, ResolvedTarget, RuleError, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattlePhase, BattleState, BattleStats, FieldConditionKind, MoveSlotState, NonVolatileStatus,
    PokemonState, PokemonTypes, SideConditionKind, SideState, StatusState, TerrainKind,
    VolatileConditionKind, WeatherKind,
};

#[test]
fn singles_and_doubles_are_topology_not_effect_branches() {
    let singles = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen9,
        BattleMode::Singles,
        "custom",
    ))
    .unwrap();
    let doubles = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen9,
        BattleMode::Doubles,
        "custom",
    ))
    .unwrap();

    assert_eq!(singles.topology().active_slots_per_side(), 1);
    assert_eq!(doubles.topology().active_slots_per_side(), 2);

    let p1_left = doubles.topology().position(SideId::P1, 0).unwrap();
    assert_eq!(doubles.topology().adjacent_foes(p1_left).len(), 2);
    assert_eq!(doubles.topology().adjacent_allies(p1_left).len(), 1);
    assert_eq!(doubles.topology().all_adjacent(p1_left).len(), 3);
}

#[test]
fn generation_rules_are_pre_resolved() {
    let gen6 = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen6,
        BattleMode::Singles,
        "custom",
    ))
    .unwrap();
    let gen9 = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen9,
        BattleMode::Singles,
        "custom",
    ))
    .unwrap();

    assert_eq!(gen6.status().paralysis_speed_modifier.numerator, 1);
    assert_eq!(gen6.status().paralysis_speed_modifier.denominator, 4);
    assert_eq!(gen9.status().paralysis_speed_modifier.denominator, 2);

    // Rules may support these mechanics, but the engine must not expose a choice until the
    // mechanic has a semantic implementation. Terastallization has one since R-O.2, Mega
    // Evolution and Ultra Burst since R-O.9; Z-Moves and Dynamax have theirs too.
    assert!(gen6.special().mega);
    assert!(!gen6.special().tera);
    assert!(!gen9.special().mega);
    assert!(gen9.special().tera);
    // The generation still gates it: Gen 8 has no Terastallization.
    let gen8 = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen8,
        BattleMode::Singles,
        "custom",
    ))
    .unwrap();
    assert!(!gen8.special().tera);
}

#[test]
fn format_is_distinct_from_mode() {
    // A doubles format's switches (as `format::BattleBuilder` derives them) under a singles
    // battle.
    let doubles_only = FormatRules {
        required_mode: Some(BattleMode::Doubles),
        ..FormatRules::custom()
    };
    let result = RulesResolver::resolve_format(
        &BattleSpec::new(Generation::Gen9, BattleMode::Singles, "gen9vgc2025regi"),
        &doubles_only,
    );

    assert!(matches!(
        result,
        Err(RuleError::ModeMismatch {
            requested: BattleMode::Singles,
            required: BattleMode::Doubles,
        })
    ));
}

/// Double battles begin in Gen 3 (the cartridges, Showdown's Doubles Custom Games): a Gen 1-2
/// double battle has nothing to be checked against and is refused, Gen 3's is resolved.
#[test]
fn double_battles_begin_in_gen_3() {
    for generation in [Generation::Gen1, Generation::Gen2] {
        assert_eq!(
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
                .err(),
            Some(RuleError::ModeUnavailable {
                generation,
                mode: BattleMode::Doubles,
            })
        );
        assert!(
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .is_ok()
        );
    }
    assert!(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen3,
            BattleMode::Doubles,
            "custom"
        ))
        .is_ok()
    );
}

/// `RulesResolver::resolve` knows the engine's own `custom` only: a real format's switches come
/// from the format layer (DESIGN.md §45.5).
#[test]
fn a_real_format_is_not_resolved_by_name() {
    assert_eq!(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "gen9ou"
        ))
        .err(),
        Some(RuleError::UnknownFormat("gen9ou".into()))
    );
}

#[test]
fn resolved_dex_applies_generation_overlays_before_battle() {
    let gen1 = DexResolver::demo(Generation::Gen1);
    assert!(gen1.move_by_id(MoveId::ELECTRIC_TERRAIN).is_none());
    assert!(gen1.ability_by_id(AbilityId::INTIMIDATE).is_none());
    assert_eq!(
        gen1.move_by_id(MoveId::FLAMETHROWER).unwrap().category,
        pokemon_battle_engine::dex::MoveCategory::Special
    );
    assert_eq!(
        gen1.move_by_id(MoveId::TACKLE).unwrap().category,
        pokemon_battle_engine::dex::MoveCategory::Physical
    );
}

#[test]
fn generation_validation_rejects_unavailable_types() {
    let rules = rules(Generation::Gen1, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen1));
    let a = PokemonId(1);
    let b = PokemonId(2);
    let mut invalid = pokemon(a, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]);
    invalid.types = PokemonTypes::mono(PokemonType::Fairy);
    let state = BattleState::new(
        vec![
            invalid,
            pokemon(b, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
        ],
        [side(vec![a], vec![Some(a)]), side(vec![b], vec![Some(b)])],
        1,
    );
    assert!(matches!(
        BattleEngine::try_new(rules, dex, state),
        Err(BattleInitError::UnavailablePokemonType { pokemon, pokemon_type: PokemonType::Fairy }) if pokemon == a
    ));
}

#[test]
fn initial_state_validation_rejects_topology_mismatch() {
    let rules = rules(Generation::Gen9, BattleMode::Doubles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let a = PokemonId(1);
    let b = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(a, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
            pokemon(b, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
        ],
        [side(vec![a], vec![Some(a)]), side(vec![b], vec![Some(b)])],
        1,
    );

    let result = BattleEngine::try_new(rules, dex, state);
    assert!(matches!(
        result,
        Err(BattleInitError::ActiveSlotCountMismatch {
            side: SideId::P1,
            expected: 2,
            actual: 1,
        })
    ));
}

#[test]
fn intimidate_uses_topology_and_hits_both_doubles_foes() {
    let rules = rules(Generation::Gen9, BattleMode::Doubles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));

    let p1a = PokemonId(1);
    let p1b = PokemonId(2);
    let p2a = PokemonId(3);
    let p2b = PokemonId(4);

    let state = BattleState::new(
        vec![
            pokemon(
                p1a,
                SideId::P1,
                AbilityId::INTIMIDATE,
                100,
                vec![MoveId::TACKLE],
            ),
            pokemon(p1b, SideId::P1, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
            pokemon(p2a, SideId::P2, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
            pokemon(p2b, SideId::P2, AbilityId::NONE, 70, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![p1a, p1b], vec![Some(p1a), Some(p1b)]),
            side(vec![p2a, p2b], vec![Some(p2a), Some(p2b)]),
        ],
        7,
    );

    let mut battle = BattleEngine::new(rules, dex, state);
    battle.run_switch_in_ability(p1a);

    assert_eq!(battle.state().pokemon(p1b).unwrap().stages.attack, 0);
    assert_eq!(battle.state().pokemon(p2a).unwrap().stages.attack, -1);
    assert_eq!(battle.state().pokemon(p2b).unwrap().stages.attack, -1);
}

#[test]
fn acrobatics_has_a_typed_base_power_contract() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));

    let user = PokemonId(1);
    let foe = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                user,
                SideId::P1,
                AbilityId::NONE,
                100,
                vec![MoveId::ACROBATICS],
            ),
            pokemon(foe, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![user], vec![Some(user)]),
            side(vec![foe], vec![Some(foe)]),
        ],
        9,
    );
    let foe_position = rules.topology().position(SideId::P2, 0).unwrap();
    let mut battle = BattleEngine::new(rules, dex, state);
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, user, MoveSlot::FIRST, Some(foe_position)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: foe,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    // The pipeline's ordered BasePower stage is the only entry point for move power behavior. The
    // handler receives the typed `BasePower` value and the trace records the exact transition.
    let acrobatics = battle
        .trace()
        .events()
        .iter()
        .find_map(|event| match event {
            TraceEvent::ModifierApplied {
                stage: ModifierStage::BasePower,
                source: ModifierSource::Move(MoveId::ACROBATICS),
                from,
                to,
                ..
            } => Some((*from, *to)),
            _ => None,
        });
    assert_eq!(acrobatics, Some((55, 110)));
}

#[test]
fn a_complete_singles_turn_runs_through_queue_move_damage_and_residual_boundary() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));

    let fast = PokemonId(1);
    let slow = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                fast,
                SideId::P1,
                AbilityId::NONE,
                140,
                vec![MoveId::ACROBATICS],
            ),
            pokemon(slow, SideId::P2, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![fast], vec![Some(fast)]),
            side(vec![slow], vec![Some(slow)]),
        ],
        42,
    );

    let mut battle = BattleEngine::new(rules.clone(), dex, state);
    let p1_target = rules.topology().position(SideId::P2, 0).unwrap();
    let p2_target = rules.topology().position(SideId::P1, 0).unwrap();

    let result = battle
        .resolve_turn([
            side_choice_move(SideId::P1, fast, MoveSlot::FIRST, Some(p1_target)),
            side_choice_move(SideId::P2, slow, MoveSlot::FIRST, Some(p2_target)),
        ])
        .unwrap();

    assert_eq!(result.phase, BattlePhase::TurnChoice);
    assert_eq!(battle.state().turn, 2);
    assert!(battle.state().pokemon(fast).unwrap().hp < 300);
    assert!(battle.state().pokemon(slow).unwrap().hp < 300);
    assert_eq!(battle.state().pokemon(fast).unwrap().moves[0].pp, 14);

    let move_order = battle
        .trace()
        .events()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::MoveUsed { actor, .. } => Some(*actor),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(move_order, vec![fast, slow]);
}

#[test]
fn doubles_spread_targeting_is_derived_from_topology() {
    let rules = rules(Generation::Gen9, BattleMode::Doubles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));

    let p1a = PokemonId(1);
    let p1b = PokemonId(2);
    let p2a = PokemonId(3);
    let p2b = PokemonId(4);
    let state = BattleState::new(
        vec![
            pokemon(
                p1a,
                SideId::P1,
                AbilityId::NONE,
                200,
                vec![MoveId::EARTHQUAKE],
            ),
            pokemon(p1b, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
            pokemon(p2a, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
            pokemon(p2b, SideId::P2, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![p1a, p1b], vec![Some(p1a), Some(p1b)]),
            side(vec![p2a, p2b], vec![Some(p2a), Some(p2b)]),
        ],
        77,
    );

    let mut battle = BattleEngine::new(rules, dex, state);
    battle
        .resolve_turn([
            SideChoice {
                side: SideId::P1,
                actions: vec![
                    SlotChoice {
                        actor: p1a,
                        action: ActionChoice::Move {
                            slot: MoveSlot::FIRST,
                            target: None,
                            mechanic: None,
                        },
                    },
                    SlotChoice {
                        actor: p1b,
                        action: ActionChoice::Pass,
                    },
                ],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![
                    SlotChoice {
                        actor: p2a,
                        action: ActionChoice::Pass,
                    },
                    SlotChoice {
                        actor: p2b,
                        action: ActionChoice::Pass,
                    },
                ],
            },
        ])
        .unwrap();

    assert_eq!(battle.state().pokemon(p1a).unwrap().hp, 300);
    assert!(battle.state().pokemon(p1b).unwrap().hp < 300);
    assert!(battle.state().pokemon(p2a).unwrap().hp < 300);
    assert!(battle.state().pokemon(p2b).unwrap().hp < 300);

    let spread_hits = battle
        .trace()
        .events()
        .iter()
        .filter(|event| {
            matches!(
                event,
                TraceEvent::DamageCalculated {
                    spread_applied: true,
                    ..
                }
            )
        })
        .count();
    assert_eq!(spread_hits, 3);
}

#[test]
fn end_turn_status_damage_uses_resolved_generation_rules() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let burned = PokemonId(1);
    let foe = PokemonId(2);
    let mut burned_state = pokemon(
        burned,
        SideId::P1,
        AbilityId::NONE,
        100,
        vec![MoveId::TACKLE],
    );
    burned_state.status = Some(StatusState::new(NonVolatileStatus::Burn));
    let state = BattleState::new(
        vec![
            burned_state,
            pokemon(foe, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![burned], vec![Some(burned)]),
            side(vec![foe], vec![Some(foe)]),
        ],
        12,
    );
    let mut battle = BattleEngine::new(rules, dex, state);

    battle
        .resolve_turn([
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: burned,
                    action: ActionChoice::Pass,
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: foe,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert_eq!(battle.state().pokemon(burned).unwrap().hp, 282);
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::StatusResidual {
            target,
            status: NonVolatileStatus::Burn,
            damage: 18,
        } if *target == burned
    )));
}

#[test]
fn fainted_slots_enter_forced_switch_phase_and_can_be_filled_explicitly() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));

    let attacker = PokemonId(1);
    let victim = PokemonId(2);
    let bench = PokemonId(3);
    let mut victim_state = pokemon(
        victim,
        SideId::P2,
        AbilityId::NONE,
        60,
        vec![MoveId::TACKLE],
    );
    victim_state.hp = 1;

    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::NONE,
                120,
                vec![MoveId::TACKLE],
            ),
            victim_state,
            pokemon(
                bench,
                SideId::P2,
                AbilityId::INTIMIDATE,
                90,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![victim, bench], vec![Some(victim)]),
        ],
        99,
    );
    let mut battle = BattleEngine::new(rules.clone(), dex, state);
    let target = rules.topology().position(SideId::P2, 0).unwrap();

    let turn = battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: victim,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert_eq!(turn.phase, BattlePhase::ForcedSwitch);
    assert_eq!(turn.replacements[1].required_count, 1);

    let position = turn.replacements[1].positions[0].position;
    let after = battle
        .resolve_forced_switches([
            ForcedSwitchChoice {
                side: SideId::P1,
                replacements: vec![],
            },
            ForcedSwitchChoice {
                side: SideId::P2,
                replacements: vec![ForcedReplacement {
                    position,
                    pokemon: bench,
                }],
            },
        ])
        .unwrap();

    assert_eq!(after.phase, BattlePhase::TurnChoice);
    assert_eq!(battle.state().turn, 2);
    assert_eq!(battle.state().side(SideId::P2).active[0], Some(bench));
}

#[test]
fn doubles_with_only_one_active_pokemon_requires_only_one_action() {
    let rules = rules(Generation::Gen9, BattleMode::Doubles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let only = PokemonId(1);
    let foe_a = PokemonId(2);
    let foe_b = PokemonId(3);
    let state = BattleState::new(
        vec![
            pokemon(only, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
            pokemon(foe_a, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
            pokemon(foe_b, SideId::P2, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![only], vec![Some(only), None]),
            side(vec![foe_a, foe_b], vec![Some(foe_a), Some(foe_b)]),
        ],
        5,
    );
    let battle = BattleEngine::new(rules, dex, state);

    let choice = SideChoice {
        side: SideId::P1,
        actions: vec![SlotChoice {
            actor: only,
            action: ActionChoice::Pass,
        }],
    };
    assert!(battle.validate_choice(SideId::P1, &choice).is_ok());
}

#[test]
fn double_switch_cannot_choose_the_same_bench_pokemon_twice() {
    let rules = rules(Generation::Gen9, BattleMode::Doubles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let a = PokemonId(1);
    let b = PokemonId(2);
    let bench = PokemonId(3);
    let x = PokemonId(4);
    let y = PokemonId(5);
    let state = BattleState::new(
        vec![
            pokemon(a, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
            pokemon(b, SideId::P1, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
            pokemon(bench, SideId::P1, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
            pokemon(x, SideId::P2, AbilityId::NONE, 70, vec![MoveId::TACKLE]),
            pokemon(y, SideId::P2, AbilityId::NONE, 60, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![a, b, bench], vec![Some(a), Some(b)]),
            side(vec![x, y], vec![Some(x), Some(y)]),
        ],
        11,
    );
    let battle = BattleEngine::new(rules, dex, state);
    let choice = SideChoice {
        side: SideId::P1,
        actions: vec![
            SlotChoice {
                actor: a,
                action: ActionChoice::Switch { pokemon: bench },
            },
            SlotChoice {
                actor: b,
                action: ActionChoice::Switch { pokemon: bench },
            },
        ],
    };

    assert!(matches!(
        battle.validate_choice(SideId::P1, &choice),
        Err(ChoiceError::DuplicateSwitchTarget(id)) if id == bench
    ));
}

#[test]
fn equal_seed_and_choices_produce_equal_state_and_trace() {
    let (mut left, choices_left) = deterministic_fixture(1234);
    let (mut right, choices_right) = deterministic_fixture(1234);

    left.resolve_turn(choices_left).unwrap();
    right.resolve_turn(choices_right).unwrap();

    assert_eq!(left.state(), right.state());
    assert_eq!(left.trace(), right.trace());
}

#[test]
fn ground_immunity_is_a_blocked_hit_not_zero_damage() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let defender = PokemonId(2);

    let mut defender_state = pokemon(
        defender,
        SideId::P2,
        AbilityId::NONE,
        80,
        vec![MoveId::TACKLE],
    );
    defender_state.types = PokemonTypes::mono(PokemonType::Flying);

    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::NONE,
                100,
                vec![MoveId::EARTHQUAKE],
            ),
            defender_state,
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![defender], vec![Some(defender)]),
        ],
        22,
    );
    let mut battle = BattleEngine::new(rules, dex, state);

    battle
        .resolve_turn([
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: attacker,
                    action: ActionChoice::Move {
                        slot: MoveSlot::FIRST,
                        target: None,
                        mechanic: None,
                    },
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: defender,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert_eq!(battle.state().pokemon(defender).unwrap().hp, 300);
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::HitBlocked {
            target,
            reason: pokemon_battle_engine::engine::HitBlockReason::TypeImmunity,
            ..
        } if *target == defender
    )));
}

/// Levitate is Showdown's `isGrounded`, not a callback of the ability: the hit gate's
/// type-immunity step stops a Ground damaging move (a Ground status move skips that step).
#[test]
fn levitate_stops_ground_moves_at_the_type_immunity_step() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let defender = PokemonId(2);

    let mut defender_state = pokemon(
        defender,
        SideId::P2,
        AbilityId::LEVITATE,
        80,
        vec![MoveId::TACKLE],
    );
    defender_state.types = PokemonTypes::mono(PokemonType::Electric);

    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::NONE,
                100,
                vec![MoveId::EARTHQUAKE],
            ),
            defender_state,
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![defender], vec![Some(defender)]),
        ],
        23,
    );
    let mut battle = BattleEngine::new(rules, dex, state);

    battle
        .resolve_turn([
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: attacker,
                    action: ActionChoice::Move {
                        slot: MoveSlot::FIRST,
                        target: None,
                        mechanic: None,
                    },
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: defender,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert_eq!(battle.state().pokemon(defender).unwrap().hp, 300);
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::HitBlocked {
            target,
            reason: pokemon_battle_engine::engine::HitBlockReason::Ability(AbilityId::LEVITATE),
            ..
        } if *target == defender
    )));
}

#[test]
fn ability_item_and_move_modifiers_compose_without_generic_event_results() {
    use pokemon_battle_engine::ids::ItemId;

    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let defender = PokemonId(2);

    let mut attacker_state = pokemon(
        attacker,
        SideId::P1,
        AbilityId::TECHNICIAN,
        100,
        vec![MoveId::ACROBATICS],
    );
    attacker_state.held_item = Some(ItemId::CHOICE_BAND);
    attacker_state.types = PokemonTypes::mono(PokemonType::Flying);

    let state = BattleState::new(
        vec![
            attacker_state,
            pokemon(
                defender,
                SideId::P2,
                AbilityId::NONE,
                80,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![defender], vec![Some(defender)]),
        ],
        24,
    );
    let mut battle = BattleEngine::new(rules.clone(), dex, state);
    let target = rules.topology().position(SideId::P2, 0).unwrap();

    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: defender,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    // Technician is a participant in the BasePower chain (×1.5 = 6144, applied with Showdown's
    // `modify`): 55 → 82.
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::ModifierApplied {
            stage: ModifierStage::BasePower,
            source: ModifierSource::AbilityInstance {
                ability: AbilityId::TECHNICIAN,
                ..
            },
            from: 55,
            to: 82,
            ..
        }
    )));
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::OffensiveStatResolved {
            base: 100,
            after_ability: 100,
            after_item: 150,
            ..
        }
    )));
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::StabResolved { modifier, .. }
            if modifier.numerator == 3 && modifier.denominator == 2
    )));
}

#[test]
fn huge_power_modifies_only_the_offensive_stat_stage() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let defender = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::HUGE_POWER,
                100,
                vec![MoveId::TACKLE],
            ),
            pokemon(
                defender,
                SideId::P2,
                AbilityId::NONE,
                80,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![defender], vec![Some(defender)]),
        ],
        25,
    );
    let mut battle = BattleEngine::new(rules.clone(), dex, state);
    let target = rules.topology().position(SideId::P2, 0).unwrap();

    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: defender,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::OffensiveStatResolved {
            base: 100,
            after_ability: 200,
            after_item: 200,
            ..
        }
    )));
}

#[test]
fn type_chart_and_stab_are_resolved_rules_not_effect_branches() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    assert_eq!(
        rules
            .type_chart()
            .effectiveness(PokemonType::Ground, PokemonType::Electric),
        pokemon_battle_engine::rules::Fraction::new(2, 1)
    );
    assert_eq!(
        rules
            .type_chart()
            .effectiveness(PokemonType::Ground, PokemonType::Flying),
        pokemon_battle_engine::rules::Fraction::ZERO
    );
}

#[test]
fn accuracy_and_evasion_resolve_to_a_typed_probability_before_rng() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let defender = PokemonId(2);
    let mut attacker_state = pokemon(
        attacker,
        SideId::P1,
        AbilityId::NONE,
        100,
        vec![MoveId::TACKLE],
    );
    let mut defender_state = pokemon(
        defender,
        SideId::P2,
        AbilityId::NONE,
        80,
        vec![MoveId::TACKLE],
    );
    attacker_state.stages.accuracy = -6;
    defender_state.stages.evasion = 6;
    let state = BattleState::new(
        vec![attacker_state, defender_state],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![defender], vec![Some(defender)]),
        ],
        26,
    );
    let mut battle = BattleEngine::new(rules.clone(), dex, state);
    let target = rules.topology().position(SideId::P2, 0).unwrap();

    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: defender,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    // Gen 5+ truncates the staged accuracy to a whole percent (Showdown `hitStepAccuracy`):
    // 100 × 3 / 9 = 33.
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::AccuracyResolved {
            actor,
            target: event_target,
            final_basis_points: 3300,
            ..
        } if *actor == attacker && *event_target == defender
    )));
}

#[test]
fn type_chart_generation_overlay_is_resolved_before_battle() {
    let gen5 = rules(Generation::Gen5, BattleMode::Singles, "custom");
    let gen9 = rules(Generation::Gen9, BattleMode::Singles, "custom");

    assert_eq!(
        gen5.type_chart()
            .effectiveness(PokemonType::Ghost, PokemonType::Steel),
        pokemon_battle_engine::rules::Fraction::new(1, 2),
    );
    assert_eq!(
        gen9.type_chart()
            .effectiveness(PokemonType::Ghost, PokemonType::Steel),
        pokemon_battle_engine::rules::Fraction::ONE,
    );
}

#[test]
fn side_and_field_moves_use_the_same_target_resolution_model() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let a = PokemonId(1);
    let b = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(a, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TAILWIND]),
            pokemon(b, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
        ],
        [side(vec![a], vec![Some(a)]), side(vec![b], vec![Some(b)])],
        31,
    );

    let user_side = rules
        .targeting()
        .resolve(rules.topology(), &state, a, TargetPattern::UserSide, None)
        .unwrap();
    let opponent_side = rules
        .targeting()
        .resolve(
            rules.topology(),
            &state,
            a,
            TargetPattern::OpponentSide,
            None,
        )
        .unwrap();
    let field = rules
        .targeting()
        .resolve(rules.topology(), &state, a, TargetPattern::Field, None)
        .unwrap();

    assert_eq!(user_side.targets, vec![ResolvedTarget::Side(SideId::P1)]);
    assert_eq!(
        opponent_side.targets,
        vec![ResolvedTarget::Side(SideId::P2)]
    );
    assert_eq!(field.targets, vec![ResolvedTarget::Field]);
}

#[test]
fn protect_is_a_turn_scoped_volatile_and_blocks_try_hit() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let protector = PokemonId(1);
    let attacker = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                protector,
                SideId::P1,
                AbilityId::NONE,
                60,
                vec![MoveId::PROTECT],
            ),
            pokemon(
                attacker,
                SideId::P2,
                AbilityId::NONE,
                120,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![protector], vec![Some(protector)]),
            side(vec![attacker], vec![Some(attacker)]),
        ],
        32,
    );
    let mut battle = BattleEngine::new(rules.clone(), dex, state);
    let target = rules.topology().position(SideId::P1, 0).unwrap();

    battle
        .resolve_turn([
            side_choice_move(SideId::P1, protector, MoveSlot::FIRST, None),
            side_choice_move(SideId::P2, attacker, MoveSlot::FIRST, Some(target)),
        ])
        .unwrap();

    assert_eq!(battle.state().pokemon(protector).unwrap().hp, 300);
    assert!(
        !battle
            .state()
            .pokemon(protector)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Protect)
    );
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::HitBlocked {
            target,
            reason: pokemon_battle_engine::engine::HitBlockReason::Protect,
            ..
        } if *target == protector
    )));
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::VolatileEnded {
            target,
            kind: VolatileConditionKind::Protect, ..
        } if *target == protector
    )));
}

#[test]
fn substitute_routes_move_damage_to_volatile_hp_without_spill() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let user = PokemonId(1);
    let attacker = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                user,
                SideId::P1,
                AbilityId::NONE,
                120,
                vec![MoveId::SUBSTITUTE],
            ),
            pokemon(
                attacker,
                SideId::P2,
                AbilityId::NONE,
                80,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![user], vec![Some(user)]),
            side(vec![attacker], vec![Some(attacker)]),
        ],
        33,
    );
    let mut battle = BattleEngine::new(rules.clone(), dex, state);
    let target = rules.topology().position(SideId::P1, 0).unwrap();

    battle
        .resolve_turn([
            side_choice_move(SideId::P1, user, MoveSlot::FIRST, None),
            side_choice_move(SideId::P2, attacker, MoveSlot::FIRST, Some(target)),
        ])
        .unwrap();

    let user_state = battle.state().pokemon(user).unwrap();
    assert_eq!(user_state.hp, 225);
    assert!(
        user_state
            .volatiles
            .substitute_hp()
            .is_some_and(|hp| hp < 75)
    );
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::SubstituteDamaged { target, .. } if *target == user
    )));
}

#[test]
fn stealth_rock_is_a_persistent_side_condition_and_runs_in_switch_in_lifecycle() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let setter = PokemonId(1);
    let active = PokemonId(2);
    let bench = PokemonId(3);
    let state = BattleState::new(
        vec![
            pokemon(
                setter,
                SideId::P1,
                AbilityId::NONE,
                100,
                vec![MoveId::STEALTH_ROCK],
            ),
            pokemon(
                active,
                SideId::P2,
                AbilityId::NONE,
                90,
                vec![MoveId::TACKLE],
            ),
            pokemon(bench, SideId::P2, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![setter], vec![Some(setter)]),
            side(vec![active, bench], vec![Some(active)]),
        ],
        34,
    );
    let mut battle = BattleEngine::new(rules, dex, state);

    battle
        .resolve_turn([
            side_choice_move(SideId::P1, setter, MoveSlot::FIRST, None),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: active,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert!(
        battle
            .state()
            .side(SideId::P2)
            .conditions
            .contains(SideConditionKind::StealthRock)
    );

    battle
        .resolve_turn([
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: setter,
                    action: ActionChoice::Pass,
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: active,
                    action: ActionChoice::Switch { pokemon: bench },
                }],
            },
        ])
        .unwrap();

    assert!(battle.state().pokemon(bench).unwrap().hp < 300);
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::EntryHazardTriggered {
            target,
            kind: SideConditionKind::StealthRock,
            ..
        } if *target == bench
    )));
}

#[test]
fn tailwind_modifies_speed_while_trick_room_changes_only_speed_comparison_direction() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let tailwind_user = PokemonId(1);
    let foe = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                tailwind_user,
                SideId::P1,
                AbilityId::NONE,
                50,
                vec![MoveId::TAILWIND, MoveId::TACKLE],
            ),
            pokemon(foe, SideId::P2, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![tailwind_user], vec![Some(tailwind_user)]),
            side(vec![foe], vec![Some(foe)]),
        ],
        35,
    );
    let mut battle = BattleEngine::new(rules.clone(), dex.clone(), state);

    battle
        .resolve_turn([
            side_choice_move(SideId::P1, tailwind_user, MoveSlot::FIRST, None),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: foe,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();
    assert_eq!(
        battle.state().side(SideId::P1).conditions.tailwind_turns(),
        Some(3)
    );

    let p1_target = rules.topology().position(SideId::P2, 0).unwrap();
    let p2_target = rules.topology().position(SideId::P1, 0).unwrap();
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, tailwind_user, MoveSlot::SECOND, Some(p1_target)),
            side_choice_move(SideId::P2, foe, MoveSlot::FIRST, Some(p2_target)),
        ])
        .unwrap();
    let ordered = battle
        .trace()
        .events()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::MoveUsed { actor, move_id, .. } if *move_id == MoveId::TACKLE => {
                Some(*actor)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ordered, vec![tailwind_user, foe]);

    let fast = PokemonId(11);
    let slow = PokemonId(12);
    let trick_state = BattleState::new(
        vec![
            pokemon(
                fast,
                SideId::P1,
                AbilityId::NONE,
                120,
                vec![MoveId::TRICK_ROOM, MoveId::TACKLE],
            ),
            pokemon(slow, SideId::P2, AbilityId::NONE, 40, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![fast], vec![Some(fast)]),
            side(vec![slow], vec![Some(slow)]),
        ],
        36,
    );
    let mut trick_battle = BattleEngine::new(rules.clone(), dex, trick_state);
    trick_battle
        .resolve_turn([
            side_choice_move(SideId::P1, fast, MoveSlot::FIRST, None),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: slow,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();
    assert!(
        trick_battle
            .state()
            .field
            .contains(FieldConditionKind::TrickRoom)
    );

    let fast_target = rules.topology().position(SideId::P2, 0).unwrap();
    let slow_target = rules.topology().position(SideId::P1, 0).unwrap();
    trick_battle
        .resolve_turn([
            side_choice_move(SideId::P1, fast, MoveSlot::SECOND, Some(fast_target)),
            side_choice_move(SideId::P2, slow, MoveSlot::FIRST, Some(slow_target)),
        ])
        .unwrap();
    let ordered = trick_battle
        .trace()
        .events()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::MoveUsed { actor, move_id, .. } if *move_id == MoveId::TACKLE => {
                Some(*actor)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ordered, vec![slow, fast]);
}

#[test]
fn weather_and_terrain_feed_typed_damage_and_base_power_stages() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let user = PokemonId(1);
    let foe = PokemonId(2);
    let mut fire_user = pokemon(
        user,
        SideId::P1,
        AbilityId::NONE,
        100,
        vec![MoveId::SUNNY_DAY, MoveId::FLAMETHROWER],
    );
    fire_user.types = PokemonTypes::mono(PokemonType::Fire);
    let state = BattleState::new(
        vec![
            fire_user,
            pokemon(foe, SideId::P2, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![user], vec![Some(user)]),
            side(vec![foe], vec![Some(foe)]),
        ],
        37,
    );
    let mut battle = BattleEngine::new(rules.clone(), dex.clone(), state);
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, user, MoveSlot::FIRST, None),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: foe,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();
    assert_eq!(
        battle.state().field.weather.map(|weather| weather.kind),
        Some(WeatherKind::Sun)
    );

    let target = rules.topology().position(SideId::P2, 0).unwrap();
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, user, MoveSlot::SECOND, Some(target)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: foe,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::WeatherModifierResolved {
            weather: Some(WeatherKind::Sun),
            modifier,
            ..
        } if *modifier == pokemon_battle_engine::rules::Fraction::new(3, 2)
    )));

    let electric = PokemonId(11);
    let defender = PokemonId(12);
    let terrain_state = BattleState::new(
        vec![
            pokemon(
                electric,
                SideId::P1,
                AbilityId::NONE,
                100,
                vec![MoveId::ELECTRIC_TERRAIN, MoveId::THUNDERBOLT],
            ),
            pokemon(
                defender,
                SideId::P2,
                AbilityId::NONE,
                80,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![electric], vec![Some(electric)]),
            side(vec![defender], vec![Some(defender)]),
        ],
        38,
    );
    let mut terrain_battle = BattleEngine::new(rules.clone(), dex, terrain_state);
    terrain_battle
        .resolve_turn([
            side_choice_move(SideId::P1, electric, MoveSlot::FIRST, None),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: defender,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();
    assert_eq!(
        terrain_battle
            .state()
            .field
            .terrain
            .map(|terrain| terrain.kind),
        Some(TerrainKind::Electric)
    );

    let target = rules.topology().position(SideId::P2, 0).unwrap();
    terrain_battle
        .resolve_turn([
            side_choice_move(SideId::P1, electric, MoveSlot::SECOND, Some(target)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: defender,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();
    assert!(terrain_battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::ModifierApplied {
            stage: ModifierStage::BasePower,
            source: ModifierSource::Terrain(_),
            from: 90,
            to: 117,
            ..
        }
    )));
}

#[test]
fn timed_conditions_expire_at_their_lifecycle_boundary() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let user = PokemonId(1);
    let foe = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                user,
                SideId::P1,
                AbilityId::NONE,
                100,
                vec![MoveId::TAILWIND],
            ),
            pokemon(foe, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![user], vec![Some(user)]),
            side(vec![foe], vec![Some(foe)]),
        ],
        39,
    );
    let mut battle = BattleEngine::new(rules, dex, state);
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, user, MoveSlot::FIRST, None),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: foe,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    for _ in 0..3 {
        battle
            .resolve_turn([
                SideChoice {
                    side: SideId::P1,
                    actions: vec![SlotChoice {
                        actor: user,
                        action: ActionChoice::Pass,
                    }],
                },
                SideChoice {
                    side: SideId::P2,
                    actions: vec![SlotChoice {
                        actor: foe,
                        action: ActionChoice::Pass,
                    }],
                },
            ])
            .unwrap();
    }

    assert_eq!(
        battle.state().side(SideId::P1).conditions.tailwind_turns(),
        None
    );
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::SideConditionEnded {
            side: SideId::P1,
            kind: SideConditionKind::Tailwind,
        }
    )));
}

#[test]
fn secondary_status_is_data_driven_and_runs_through_status_pipeline() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let target = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::NONE,
                120,
                vec![MoveId::NUZZLE],
            ),
            pokemon(
                target,
                SideId::P2,
                AbilityId::NONE,
                80,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![target], vec![Some(target)]),
        ],
        101,
    );
    let target_position = rules.topology().position(SideId::P2, 0).unwrap();
    let mut battle = BattleEngine::new(rules, dex, state);

    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target_position)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: target,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert!(
        battle
            .state()
            .pokemon(target)
            .unwrap()
            .has_status(NonVolatileStatus::Paralysis)
    );
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::SecondaryEffectRolled { move_id, success: true, .. }
            if *move_id == MoveId::NUZZLE
    )));
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::StatusApplied { target: id, status: NonVolatileStatus::Paralysis, .. }
            if *id == target
    )));
}

#[test]
fn status_pipeline_distinguishes_type_immunity_and_ability_prevention() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let target = PokemonId(2);

    let mut electric_target = pokemon(
        target,
        SideId::P2,
        AbilityId::NONE,
        80,
        vec![MoveId::TACKLE],
    );
    electric_target.types = PokemonTypes::mono(PokemonType::Electric);
    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::NONE,
                120,
                vec![MoveId::NUZZLE],
            ),
            electric_target,
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![target], vec![Some(target)]),
        ],
        102,
    );
    let target_position = rules.topology().position(SideId::P2, 0).unwrap();
    let mut battle = BattleEngine::new(rules.clone(), dex.clone(), state);
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target_position)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: target,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();
    assert!(battle.state().pokemon(target).unwrap().status.is_none());
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::StatusBlocked { target: id, reason: StatusBlockReason::TypeImmunity, .. }
            if *id == target
    )));

    let mut limber_target = pokemon(
        target,
        SideId::P2,
        AbilityId::LIMBER,
        80,
        vec![MoveId::TACKLE],
    );
    limber_target.types = PokemonTypes::mono(PokemonType::Normal);
    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::NONE,
                120,
                vec![MoveId::NUZZLE],
            ),
            limber_target,
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![target], vec![Some(target)]),
        ],
        103,
    );
    let mut battle = BattleEngine::new(rules, dex, state);
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target_position)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: target,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();
    assert!(battle.state().pokemon(target).unwrap().status.is_none());
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::StatusBlocked {
            target: id,
            reason: StatusBlockReason::Ability(AbilityId::LIMBER),
            ..
        } if *id == target
    )));
}

#[test]
fn substitute_blocks_target_directed_secondary_status() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let target = PokemonId(2);
    let mut target_state = pokemon(
        target,
        SideId::P2,
        AbilityId::NONE,
        80,
        vec![MoveId::TACKLE],
    );
    target_state.volatiles.set_substitute(target, 100);
    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::NONE,
                120,
                vec![MoveId::NUZZLE],
            ),
            target_state,
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![target], vec![Some(target)]),
        ],
        104,
    );
    let target_position = rules.topology().position(SideId::P2, 0).unwrap();
    let mut battle = BattleEngine::new(rules, dex, state);
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target_position)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: target,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert!(battle.state().pokemon(target).unwrap().status.is_none());
    assert!(battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::SubstituteDamaged { target: id, .. } if *id == target
    )));
    assert!(!battle.trace().events().iter().any(|event| matches!(
        event,
        TraceEvent::StatusApplied { target: id, .. } if *id == target
    )));
}

#[test]
fn once_after_move_secondary_can_modify_the_user() {
    // Power-Up Punch was removed from the cartridge in Gen 9 (and @pkmn/data agrees), so the
    // demo dex filters it there; Gen 8 is the last generation that has it.
    let rules = rules(Generation::Gen8, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen8));
    let attacker = PokemonId(1);
    let target = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::NONE,
                120,
                vec![MoveId::POWER_UP_PUNCH],
            ),
            pokemon(
                target,
                SideId::P2,
                AbilityId::NONE,
                80,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![target], vec![Some(target)]),
        ],
        105,
    );
    let target_position = rules.topology().position(SideId::P2, 0).unwrap();
    let mut battle = BattleEngine::new(rules, dex, state);
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target_position)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: target,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    assert_eq!(battle.state().pokemon(attacker).unwrap().stages.attack, 1);
}

#[test]
fn toxic_uses_status_state_for_escalating_residual_damage() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let toxic = PokemonId(1);
    let foe = PokemonId(2);
    let mut toxic_state = pokemon(
        toxic,
        SideId::P1,
        AbilityId::NONE,
        100,
        vec![MoveId::TACKLE],
    );
    toxic_state.status = Some(StatusState::new(NonVolatileStatus::Toxic));
    let state = BattleState::new(
        vec![
            toxic_state,
            pokemon(foe, SideId::P2, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![toxic], vec![Some(toxic)]),
            side(vec![foe], vec![Some(foe)]),
        ],
        106,
    );
    let mut battle = BattleEngine::new(rules, dex, state);
    let pass = || {
        [
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: toxic,
                    action: ActionChoice::Pass,
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: foe,
                    action: ActionChoice::Pass,
                }],
            },
        ]
    };
    battle.resolve_turn(pass()).unwrap();
    battle.resolve_turn(pass()).unwrap();

    // 300 max HP: the sixteenth is truncated first (18) and then multiplied by the counter
    // (Showdown `clampIntRange(maxhp / 16, 1) * stage`): 18 + 36 = 54. The counter is
    // Showdown's `stage`: advanced by each residual, 2 after two of them.
    assert_eq!(battle.state().pokemon(toxic).unwrap().hp, 246);
    assert_eq!(
        battle
            .state()
            .pokemon(toxic)
            .unwrap()
            .status
            .unwrap()
            .toxic_counter,
        2
    );
}

#[test]
fn showdown_chains_run_their_highest_priority_first() {
    // Showdown `comparePriority`: a `ModifyAtk` / `ModifyDef` chain runs its highest priority
    // first (Thick Fat 6, Huge Power 5, Choice Band 1, a Ruin 0), equal priorities an ability
    // before an item. `chainModify` rounds at every step, so from three participants the order
    // shows: [2048, 5325, 8192] chains to 5326, the reverse to 5325.
    use pokemon_battle_engine::engine::{ModifierDescriptor, sort_by_showdown_priority};
    use pokemon_battle_engine::ids::ItemId;
    let ability = |priority, ability| {
        ModifierDescriptor::new(
            ModifierStage::OffensiveStat,
            ModifierSource::AbilityInstance {
                holder: PokemonId(1),
                ability,
            },
            priority,
        )
    };
    let item = |priority| {
        ModifierDescriptor::new(
            ModifierStage::OffensiveStat,
            ModifierSource::ItemInstance {
                holder: PokemonId(1),
                item: ItemId::CHOICE_BAND,
            },
            priority,
        )
    };
    let mut chain = vec![
        (item(1), "Choice Band"),
        (ability(0, AbilityId::TABLETS_OF_RUIN), "Tablets of Ruin"),
        (ability(6, AbilityId::THICK_FAT), "Thick Fat"),
        (item(5), "an item at 5"),
        (ability(5, AbilityId::HUGE_POWER), "Huge Power"),
    ];
    sort_by_showdown_priority(&mut chain);
    assert_eq!(
        chain.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        [
            "Thick Fat",
            "Huge Power",
            "an item at 5",
            "Choice Band",
            "Tablets of Ruin"
        ]
    );
}

#[test]
fn modifier_order_is_explicit_and_stage_local() {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let attacker = PokemonId(1);
    let target = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(
                attacker,
                SideId::P1,
                AbilityId::TECHNICIAN,
                120,
                vec![MoveId::ACROBATICS],
            ),
            pokemon(
                target,
                SideId::P2,
                AbilityId::NONE,
                80,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![target], vec![Some(target)]),
        ],
        107,
    );
    let target_position = rules.topology().position(SideId::P2, 0).unwrap();
    let mut battle = BattleEngine::new(rules, dex, state);
    battle
        .resolve_turn([
            side_choice_move(SideId::P1, attacker, MoveSlot::FIRST, Some(target_position)),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: target,
                    action: ActionChoice::Pass,
                }],
            },
        ])
        .unwrap();

    let sources = battle
        .trace()
        .events()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::ModifierApplied {
                stage: ModifierStage::BasePower,
                source,
                ..
            } => Some(*source),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Acrobatics is a `basePowerCallback`: it runs before the `onBasePower` chain and doubles
    // 55 to 110, so Technician (which looks at the power so far) does not contribute — as in
    // Showdown. Only participants that contribute are traced.
    assert_eq!(sources, vec![ModifierSource::Move(MoveId::ACROBATICS)]);
}

#[test]
fn speed_is_one_modify_spe_chain_with_paralysis_placed_by_generation() {
    use pokemon_battle_engine::ids::ItemId;

    // Showdown chains Choice Scarf (×1.5) and Tailwind (×2) into one 4096-based modifier and
    // applies it once; paralysis joins the chain in Gen 3-4 (×0.25) and follows it, floored,
    // from Gen 5 (×25/100; ×50/100 from Gen 7). Gen 3 has no Scarf or Tailwind.
    for (generation, base, scarf_and_tailwind, expected) in [
        (Generation::Gen3, 51, false, 13),
        (Generation::Gen4, 101, true, 76),
        (Generation::Gen5, 101, true, 75),
        (Generation::Gen7, 101, true, 151),
        (Generation::Gen9, 101, true, 151),
    ] {
        let rules = rules(generation, BattleMode::Singles, "custom");
        let dex = Arc::new(DexResolver::resolve(generation));
        let user = PokemonId(1);
        let foe = PokemonId(2);
        let mut user_state = pokemon(
            user,
            SideId::P1,
            AbilityId::NONE,
            base,
            vec![MoveId::TACKLE],
        );
        user_state.status = Some(StatusState::new(NonVolatileStatus::Paralysis));
        let mut user_side = side(vec![user], vec![Some(user)]);
        if scarf_and_tailwind {
            user_state.held_item = Some(ItemId::CHOICE_SCARF);
            user_side.conditions.set_tailwind(user, 4);
        }
        let state = BattleState::new(
            vec![
                user_state,
                pokemon(foe, SideId::P2, AbilityId::NONE, 10, vec![MoveId::TACKLE]),
            ],
            [user_side, side(vec![foe], vec![Some(foe)])],
            7,
        );
        let mut battle = BattleEngine::new(rules, dex, state);
        let pass = |side, actor| SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Pass,
            }],
        };
        battle
            .resolve_turn([pass(SideId::P1, user), pass(SideId::P2, foe)])
            .unwrap();
        let speed = battle
            .trace()
            .events()
            .iter()
            .find_map(|event| match event {
                TraceEvent::ActionOrderResolved {
                    actor,
                    effective_speed,
                    ..
                } if *actor == user => Some(*effective_speed),
                _ => None,
            });
        assert_eq!(speed, Some(expected), "{generation:?}");
    }
}

#[test]
fn gen4_tailwind_duration_and_action_order_strategy_are_resolved() {
    let gen4 = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen4,
        BattleMode::Doubles,
        "custom",
    ))
    .unwrap();
    let gen7 = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen7,
        BattleMode::Doubles,
        "custom",
    ))
    .unwrap();
    let gen9 = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen9,
        BattleMode::Doubles,
        "custom",
    ))
    .unwrap();
    assert_eq!(gen4.conditions().tailwind_turns, 3);
    assert_eq!(
        gen7.action_order().strategy,
        pokemon_battle_engine::rules::ActionOrderStrategy::StaticAtTurnStart
    );
    assert_eq!(
        gen9.action_order().strategy,
        pokemon_battle_engine::rules::ActionOrderStrategy::RecalculateAfterAction
    );
}

#[test]
fn engine_step_exposes_awaiting_boundaries() {
    let (mut battle, choices) = deterministic_fixture(200);
    assert!(matches!(
        battle.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
    battle.submit_turn_choices(choices).unwrap();
    assert!(matches!(
        battle.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
}

#[test]
fn snapshot_restore_preserves_dynamic_state_instead_of_reinitializing_it() {
    let (battle, _) = deterministic_fixture(201);
    let mut snapshot = battle.snapshot();
    snapshot.state.sides[SideId::P1.index()].mechanics.tera =
        pokemon_battle_engine::state::MechanicUsage::Consumed;
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    snapshot.rules_fingerprint = rules.fingerprint().0.clone();
    snapshot.dex_fingerprint = dex.fingerprint();
    let restored = BattleEngine::restore(rules, dex, snapshot).unwrap();
    assert_eq!(
        restored.state().side(SideId::P1).mechanics.tera,
        pokemon_battle_engine::state::MechanicUsage::Consumed
    );
}

#[test]
fn random_tape_can_drive_deterministic_reference_tests() {
    let mut rng = pokemon_battle_engine::state::BattleRng::with_tape(1, vec![7, 11]);
    assert_eq!(rng.next_u64(), 7);
    assert_eq!(rng.next_u64(), 11);
    assert_eq!(rng.tape_cursor(), 2);
}

#[test]
fn semantic_random_tape_is_stage_named_and_must_be_exhausted() {
    use pokemon_battle_engine::state::{BattleRng, RandomOverride, RandomRequestKind};

    let mut rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Accuracy, true),
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, 93),
            RandomOverride::bool(RandomRequestKind::Secondary, true),
        ],
    );

    assert!(rng.roll_basis_points_for(RandomRequestKind::Accuracy, 9500));
    assert!(!rng.roll_one_in_for(RandomRequestKind::Critical, 24));
    assert_eq!(
        rng.range_inclusive_for(RandomRequestKind::DamageRoll, 85, 100),
        93
    );
    assert!(rng.roll_basis_points_for(RandomRequestKind::Secondary, 1000));
    assert!(rng.semantic_tape_exhausted());
}

#[test]
#[should_panic(expected = "semantic random plan desynchronized")]
fn semantic_random_tape_detects_stage_desynchronization() {
    use pokemon_battle_engine::state::{BattleRng, RandomOverride, RandomRequestKind};

    let mut rng = BattleRng::with_semantic_tape(
        1,
        vec![RandomOverride::bool(RandomRequestKind::Critical, false)],
    );
    let _ = rng.roll_basis_points_for(RandomRequestKind::Accuracy, 9500);
}

#[test]
fn semantically_equal_doubles_choice_order_has_the_same_rng_assignment() {
    fn fixture(reverse: bool) -> BattleEngine {
        let rules = rules(Generation::Gen9, BattleMode::Doubles, "custom");
        let dex = Arc::new(DexResolver::demo(Generation::Gen9));
        let a = PokemonId(1);
        let b = PokemonId(2);
        let c = PokemonId(3);
        let d = PokemonId(4);
        let state = BattleState::new(
            vec![
                pokemon(a, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
                pokemon(b, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
                pokemon(c, SideId::P2, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
                pokemon(d, SideId::P2, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
            ],
            [
                side(vec![a, b], vec![Some(a), Some(b)]),
                side(vec![c, d], vec![Some(c), Some(d)]),
            ],
            333,
        );
        let mut battle = BattleEngine::new(rules, dex, state);
        let mut p1_actions = vec![
            SlotChoice {
                actor: a,
                action: ActionChoice::Pass,
            },
            SlotChoice {
                actor: b,
                action: ActionChoice::Pass,
            },
        ];
        let mut p2_actions = vec![
            SlotChoice {
                actor: c,
                action: ActionChoice::Pass,
            },
            SlotChoice {
                actor: d,
                action: ActionChoice::Pass,
            },
        ];
        if reverse {
            p1_actions.reverse();
            p2_actions.reverse();
        }
        battle
            .resolve_turn([
                SideChoice {
                    side: SideId::P1,
                    actions: p1_actions,
                },
                SideChoice {
                    side: SideId::P2,
                    actions: p2_actions,
                },
            ])
            .unwrap();
        battle
    }
    let a = fixture(false);
    let b = fixture(true);
    assert_eq!(a.state(), b.state());
    assert_eq!(a.trace(), b.trace());
}

#[test]
fn gen8_plus_reorders_remaining_actions_after_tailwind() {
    let rules = rules(Generation::Gen9, BattleMode::Doubles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let setter = PokemonId(1);
    let ally = PokemonId(2);
    let foe_fast = PokemonId(3);
    let foe_pass = PokemonId(4);
    let state = BattleState::new(
        vec![
            pokemon(
                setter,
                SideId::P1,
                AbilityId::NONE,
                200,
                vec![MoveId::TAILWIND],
            ),
            pokemon(ally, SideId::P1, AbilityId::NONE, 60, vec![MoveId::TACKLE]),
            pokemon(
                foe_fast,
                SideId::P2,
                AbilityId::NONE,
                100,
                vec![MoveId::TACKLE],
            ),
            pokemon(
                foe_pass,
                SideId::P2,
                AbilityId::NONE,
                50,
                vec![MoveId::TACKLE],
            ),
        ],
        [
            side(vec![setter, ally], vec![Some(setter), Some(ally)]),
            side(
                vec![foe_fast, foe_pass],
                vec![Some(foe_fast), Some(foe_pass)],
            ),
        ],
        202,
    );
    let p2 = rules.topology().position(SideId::P2, 0).unwrap();
    let p1 = rules.topology().position(SideId::P1, 0).unwrap();
    let mut battle = BattleEngine::new(rules, dex, state);
    battle
        .resolve_turn([
            SideChoice {
                side: SideId::P1,
                actions: vec![
                    SlotChoice {
                        actor: setter,
                        action: ActionChoice::Move {
                            slot: MoveSlot::FIRST,
                            target: None,
                            mechanic: None,
                        },
                    },
                    SlotChoice {
                        actor: ally,
                        action: ActionChoice::Move {
                            slot: MoveSlot::FIRST,
                            target: Some(pokemon_battle_engine::rules::DeclaredTarget(p2)),
                            mechanic: None,
                        },
                    },
                ],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![
                    SlotChoice {
                        actor: foe_fast,
                        action: ActionChoice::Move {
                            slot: MoveSlot::FIRST,
                            target: Some(pokemon_battle_engine::rules::DeclaredTarget(p1)),
                            mechanic: None,
                        },
                    },
                    SlotChoice {
                        actor: foe_pass,
                        action: ActionChoice::Pass,
                    },
                ],
            },
        ])
        .unwrap();
    let move_order = battle
        .trace()
        .events()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::MoveUsed { actor, .. } => Some(*actor),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(move_order[0], setter);
    assert_eq!(move_order[1], ally);
    assert_eq!(move_order[2], foe_fast);
}

fn deterministic_fixture(seed: u64) -> (BattleEngine, [SideChoice; 2]) {
    let rules = rules(Generation::Gen9, BattleMode::Singles, "custom");
    let dex = Arc::new(DexResolver::demo(Generation::Gen9));
    let a = PokemonId(1);
    let b = PokemonId(2);
    let state = BattleState::new(
        vec![
            pokemon(a, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
            pokemon(b, SideId::P2, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
        ],
        [side(vec![a], vec![Some(a)]), side(vec![b], vec![Some(b)])],
        seed,
    );
    let p1_target = rules.topology().position(SideId::P2, 0).unwrap();
    let p2_target = rules.topology().position(SideId::P1, 0).unwrap();
    let choices = [
        side_choice_move(SideId::P1, a, MoveSlot::FIRST, Some(p1_target)),
        side_choice_move(SideId::P2, b, MoveSlot::FIRST, Some(p2_target)),
    ];
    (BattleEngine::new(rules, dex, state), choices)
}

fn rules(
    generation: Generation,
    mode: BattleMode,
    format: &str,
) -> Arc<pokemon_battle_engine::ResolvedRules> {
    Arc::new(RulesResolver::resolve(&BattleSpec::new(generation, mode, format)).unwrap())
}

fn pokemon(
    id: PokemonId,
    side: SideId,
    ability: AbilityId,
    speed: u16,
    moves: Vec<MoveId>,
) -> PokemonState {
    let mut pokemon = PokemonState::new(id, side);
    pokemon.level = 50;
    pokemon.hp = 300;
    pokemon.max_hp = 300;
    pokemon.stats = BattleStats {
        speed,
        ..BattleStats::default()
    };
    pokemon.ability = ability;
    pokemon.moves = moves
        .into_iter()
        .map(|move_id| MoveSlotState::new(move_id, default_pp(move_id)))
        .collect();
    pokemon
}

fn default_pp(move_id: MoveId) -> u8 {
    match move_id {
        MoveId::TACKLE => 35,
        MoveId::FLAMETHROWER | MoveId::THUNDERBOLT => 15,
        MoveId::NUZZLE | MoveId::POWER_UP_PUNCH => 20,
        MoveId::EARTHQUAKE => 10,
        MoveId::ACROBATICS => 15,
        MoveId::PROTECT | MoveId::SUBSTITUTE => 10,
        MoveId::STEALTH_ROCK => 20,
        MoveId::TAILWIND => 15,
        MoveId::TRICK_ROOM | MoveId::SUNNY_DAY => 5,
        MoveId::ELECTRIC_TERRAIN => 10,
        _ => 1,
    }
}

fn side(team: Vec<PokemonId>, active: Vec<Option<PokemonId>>) -> SideState {
    let mut side = SideState::new(team, active.len());
    side.active = active;
    side
}

fn side_choice_move(
    side: SideId,
    actor: PokemonId,
    slot: MoveSlot,
    target: Option<pokemon_battle_engine::ids::PositionId>,
) -> SideChoice {
    SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot,
                target: target.map(pokemon_battle_engine::rules::DeclaredTarget),
                mechanic: None,
            },
        }],
    }
}

/// DESIGN.md §31: generation differences are table values in `ResolvedRules`; an effect never
/// branches on the generation. The effect-facing rules view (`EffectRules`) has no
/// `generation()`, and this scan keeps the source honest as well.
#[test]
fn effects_never_branch_on_the_generation() {
    fn scan(dir: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("effects dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                scan(&path, offenders);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).expect("read");
                if source.contains("generation()") || source.contains("Generation::") {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    let mut offenders = Vec::new();
    scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/effects"),
        &mut offenders,
    );
    assert!(
        offenders.is_empty(),
        "effects branching on the generation: {offenders:?}"
    );
}

/// DESIGN.md §45: `Generation ≠ BattleMode ≠ Format`. The format layer (`crate::format`) is read
/// by the team validator and the pre-battle session (`crate::session`); the engine, its state and
/// the effects see only the switches the session resolves a format to (`ResolvedRules`), never
/// a format nor the session.
#[test]
fn the_engine_never_reads_a_format() {
    fn scan(dir: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("src dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                scan(&path, offenders);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).expect("read");
                if [
                    "crate::format",
                    "pokemon_battle_engine::format",
                    "crate::session",
                ]
                .iter()
                .any(|layer| source.contains(layer))
                {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    let mut offenders = Vec::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for dir in ["engine", "effects", "state"] {
        scan(&root.join(dir), &mut offenders);
    }
    assert!(
        offenders.is_empty(),
        "the engine reading a format: {offenders:?}"
    );
}

/// DESIGN.md §47 (R-M10-2): the trace is the engine's output, never its input. No engine, effect
/// or state code reads the events back (`.events()`) — a decision that did (Gen 3's Choice lock
/// after a Trick, I184) now reads the state — so a sink that keeps fewer events, or none, cannot
/// change a battle.
#[test]
fn the_engine_never_reads_its_trace() {
    fn scan(dir: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("src dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                scan(&path, offenders);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path.ends_with("engine/trace.rs")
            {
                let source = std::fs::read_to_string(&path).expect("read");
                if source.contains(".events()") {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    let mut offenders = Vec::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for dir in ["engine", "effects", "state"] {
        scan(&root.join(dir), &mut offenders);
    }
    assert!(
        offenders.is_empty(),
        "the engine reading its trace: {offenders:?}"
    );
}

/// DESIGN.md §47.0 item 12 (M10.4c): what the battle has shown its observers is the views'
/// alone, as the trace is the caller's: no rule reads `knowledge`. The engine writes it as control
/// goes back (`handed_back`) and reads it back only for the views and a snapshot
/// (`engine/battle/mod.rs`); the state holds it (`state/battle.rs`, `state/knowledge.rs`).
#[test]
fn the_engine_never_reads_its_knowledge() {
    fn scan(dir: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("src dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                scan(&path, offenders);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path.ends_with("engine/battle/mod.rs")
                && !path.ends_with("state/battle.rs")
                && !path.ends_with("state/knowledge.rs")
            {
                let source = std::fs::read_to_string(&path).expect("read");
                if source.contains(".knowledge") {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    let mut offenders = Vec::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for dir in ["engine", "effects", "state", "rules", "session"] {
        scan(&root.join(dir), &mut offenders);
    }
    assert!(
        offenders.is_empty(),
        "the engine reading its knowledge: {offenders:?}"
    );
}

/// R-M10-17: rustfmt formats the items of a parenthesized macro call, not a braced one's, and
/// follows no module declared inside a macro, so `cargo fmt` (and `scripts/check.sh`) would pass
/// over what such an `internal!` call holds: every call is `internal!(…);`, and no module is
/// declared through one.
#[test]
fn every_internal_item_is_within_rustfmts_reach() {
    fn scan(dir: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("src dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                scan(&path, offenders);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).expect("read");
                for (index, line) in source.lines().enumerate() {
                    let Some(call) = line.trim_start().strip_prefix("internal!") else {
                        continue;
                    };
                    let call = call.trim_start();
                    if !call.starts_with('(') || call.contains(" mod ") {
                        offenders.push(format!("{}:{}", path.display(), index + 1));
                    }
                }
            }
        }
    }
    let mut offenders = Vec::new();
    scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut offenders,
    );
    assert!(
        offenders.is_empty(),
        "internal! calls rustfmt cannot reach: {offenders:?}"
    );
}

/// DESIGN.md §45: the other way round, the format layer validates teams and resolves rules; it
/// reads no battle — no battle state, no engine — so a set's ability it reads is a set's. The
/// pre-battle session, above both, builds the battle (§45.5).
#[test]
fn the_format_layer_never_reads_a_battle() {
    let mut offenders = Vec::new();
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/format");
    for entry in std::fs::read_dir(dir).expect("src/format") {
        let path = entry.expect("entry").path();
        let source = std::fs::read_to_string(&path).expect("read");
        if source.contains("crate::state") || source.contains("crate::engine") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "the format layer reading a battle: {offenders:?}"
    );
}

/// DESIGN.md §41.1 (K.0): every reading of a Pokémon's ability or types goes through the
/// engine's entries (`engine::{raw_ability, ability_active, ability_handlers}`,
/// `engine::types_of`), so the suppression, Mold Breaker, Roost and the added types join in one
/// place. Outside the entries, the state, the dex and the rules, no source reads the `ability`
/// or `types` field.
#[test]
fn abilities_and_types_are_read_through_the_engine_entries() {
    fn reads_the_field(line: &str) -> bool {
        [".ability", ".types"]
            .iter()
            .any(|field| reads(line, field))
    }
    fn reads(line: &str, field: &str) -> bool {
        let mut rest = line;
        while let Some(at) = rest.find(field) {
            let after = &rest[at + field.len()..];
            let next = after.chars().next();
            // A type reading its own field (`self.ability` of a scenario set) is not one.
            let own = rest[..at].ends_with("self");
            if !own && !next.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '(') {
                return true;
            }
            rest = after;
        }
        false
    }
    fn scan(dir: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("src dir") {
            let path = entry.expect("entry").path();
            let shown = path.display().to_string();
            // The format layer reads a team's sets, never a battle's Pokémon
            // (`the_format_layer_never_reads_a_battle`); the pre-battle session reads the sets
            // it builds a battle from.
            if shown.ends_with("src/state")
                || shown.ends_with("src/dex")
                || shown.ends_with("src/rules")
                || shown.ends_with("src/format")
                || shown.ends_with("src/session")
            {
                continue;
            }
            if path.is_dir() {
                scan(&path, offenders);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !shown.ends_with("src/engine/ability.rs")
                && !shown.ends_with("src/engine/typing.rs")
            {
                let source = std::fs::read_to_string(&path).expect("read");
                for (number, line) in source.lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    if reads_the_field(code) {
                        offenders.push(format!("{shown}:{}", number + 1));
                    }
                }
            }
        }
    }
    let mut offenders = Vec::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    scan(&root.join("src"), &mut offenders);
    // The replay plans what the engine will do, so it asks the same entries (§41.8).
    scan(&root.join("tests/support"), &mut offenders);
    assert!(
        offenders.is_empty(),
        "abilities or types read around their entries: {offenders:?}"
    );
}

/// Showdown's `normal` target (Tackle) may be an adjacent ally in doubles; `adjacentFoe` may
/// not, and the user itself never is.
#[test]
fn a_normal_target_move_can_hit_an_adjacent_ally_in_doubles() {
    use pokemon_battle_engine::rules::{
        DeclaredTarget, TargetError, TargetPattern, TargetingRules,
    };

    let rules = rules(Generation::Gen9, BattleMode::Doubles, "custom");
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    assert_eq!(
        dex.move_by_name("Tackle").unwrap().target,
        TargetPattern::SelectedAdjacent
    );
    assert_eq!(
        dex.move_by_name("Aura Sphere").unwrap().target,
        TargetPattern::SelectedAny
    );

    let (p1a, p1b, p2a, p2b) = (PokemonId(1), PokemonId(2), PokemonId(3), PokemonId(4));
    let state = BattleState::new(
        vec![
            pokemon(p1a, SideId::P1, AbilityId::NONE, 100, vec![MoveId::TACKLE]),
            pokemon(p1b, SideId::P1, AbilityId::NONE, 90, vec![MoveId::TACKLE]),
            pokemon(p2a, SideId::P2, AbilityId::NONE, 80, vec![MoveId::TACKLE]),
            pokemon(p2b, SideId::P2, AbilityId::NONE, 70, vec![MoveId::TACKLE]),
        ],
        [
            side(vec![p1a, p1b], vec![Some(p1a), Some(p1b)]),
            side(vec![p2a, p2b], vec![Some(p2a), Some(p2b)]),
        ],
        7,
    );
    let topology = rules.topology();
    let ally = DeclaredTarget(topology.position(SideId::P1, 1).unwrap());
    let own = DeclaredTarget(topology.position(SideId::P1, 0).unwrap());
    let resolve =
        |pattern, declared| TargetingRules.resolve(topology, &state, p1a, pattern, Some(declared));
    assert!(resolve(TargetPattern::SelectedAdjacent, ally).is_ok());
    assert!(matches!(
        resolve(TargetPattern::SelectedAdjacentFoe, ally),
        Err(TargetError::WrongSide(_))
    ));
    assert!(matches!(
        resolve(TargetPattern::SelectedAdjacent, own),
        Err(TargetError::SelfNotAllowed(_))
    ));
}
