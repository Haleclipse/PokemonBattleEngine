//! L1 rule tests for declarative status moves (DESIGN.md §15): SetStatus, StatStages,
//! Volatile(Confusion) and Heal, plus the hit-gate rules that only status moves exercise
//! (type immunity by flag, Substitute, Protect).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, StatusMoveEffect, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BeforeMoveBlockReason, EngineStep, HitBlockReason, MoveFailReason, SideChoice,
    SlotChoice, StatusBlockReason, TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState, VolatileConditionKind,
};

const P1: PokemonId = PokemonId(1);
const P2: PokemonId = PokemonId(2);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn setup(
    generation: Generation,
    p1: (&str, &str),
    p2: (&str, &str),
    prepare: impl FnOnce(&mut PokemonState, &mut PokemonState),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut a = build_pokemon(&dex, P1, SideId::P1, &PokemonSet::new(p1.0, [p1.1])).unwrap();
    let mut b = build_pokemon(&dex, P2, SideId::P2, &PokemonSet::new(p2.0, [p2.1])).unwrap();
    prepare(&mut a, &mut b);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![a, b], [side(P1), side(P2)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn choice(&self, side: SideId, actor: PokemonId) -> SideChoice {
        let move_id = self.engine.state().pokemon(actor).unwrap().moves[0].move_id;
        let target = match self.dex.move_by_id(move_id).unwrap().target {
            TargetPattern::SelectedAny
            | TargetPattern::SelectedAdjacent
            | TargetPattern::SelectedFoe
            | TargetPattern::SelectedAdjacentFoe
            | TargetPattern::Ally
            | TargetPattern::AllyOrSelf => Some(DeclaredTarget(
                self.rules.topology().position(side.other(), 0).unwrap(),
            )),
            _ => None,
        };
        SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::FIRST,
                    target,
                    mechanic: None,
                },
            }],
        }
    }

    /// One turn, both sides using their first move. Speeds differ, so no SpeedTie is drawn.
    fn turn(&mut self, tape: Vec<RandomOverride>) -> usize {
        self.turn_seeded(1, tape)
    }

    /// `turn`, with the seed a draw the tape does not plan falls back to.
    fn turn_seeded(&mut self, seed: u64, tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(seed, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1, P1), self.choice(SideId::P2, P2)];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
        start
    }

    fn events(&self, from: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[from..]
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }
}

fn tackle_hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

#[test]
fn declarative_status_moves_classify_from_snapshot_data() {
    let dex = DexResolver::resolve(Generation::Gen9);
    let effect = |name: &str| dex.move_by_name(name).unwrap().primary_effect.clone();
    assert_eq!(
        effect("Thunder Wave"),
        PrimaryEffect::Status(StatusMoveEffect::SetStatus(NonVolatileStatus::Paralysis))
    );
    assert!(
        dex.move_by_name("Thunder Wave")
            .unwrap()
            .flags
            .type_immunity,
        "ignoreImmunity: false"
    );
    assert_eq!(
        effect("Will-O-Wisp"),
        PrimaryEffect::Status(StatusMoveEffect::SetStatus(NonVolatileStatus::Burn))
    );
    assert!(matches!(
        effect("Swords Dance"),
        PrimaryEffect::Status(StatusMoveEffect::StatStages(_))
    ));
    assert!(matches!(
        effect("Growl"),
        PrimaryEffect::Status(StatusMoveEffect::StatStages(_))
    ));
    assert!(matches!(
        effect("Confuse Ray"),
        PrimaryEffect::Status(StatusMoveEffect::Volatile(VolatileConditionKind::Confusion))
    ));
    assert!(matches!(
        effect("Recover"),
        PrimaryEffect::Status(StatusMoveEffect::Heal(_))
    ));
    assert!(matches!(
        effect("Hypnosis"),
        PrimaryEffect::Status(StatusMoveEffect::SetStatus(NonVolatileStatus::Sleep))
    ));
    // Powder moves are declarative once powder immunity is modelled (Grass / Overcoat / Safety
    // Goggles at the hit gate, DESIGN.md §15.6).
    assert_eq!(
        effect("Spore"),
        PrimaryEffect::Status(StatusMoveEffect::SetStatus(NonVolatileStatus::Sleep))
    );
    assert!(dex.move_by_name("Spore").unwrap().flags.powder);
}

#[test]
fn thunder_wave_paralyses_and_the_type_chart_blocks_it_against_ground() {
    // Jolteon (130 Spe) moves first. Thunder Wave is 90 % accurate.
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Thunder Wave"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(tackle_hit(100));
    // Snorlax's Tackle comes after the paralysis, so the 25 % check is asked.
    tape.insert(
        1,
        RandomOverride::bool(RandomRequestKind::FullParalysis, false),
    );
    let start = arena.turn(tape);
    assert!(arena.pokemon(P2).has_status(NonVolatileStatus::Paralysis));
    assert!(arena.events(start).iter().any(|e| matches!(e, TraceEvent::StatusApplied { target, status: NonVolatileStatus::Paralysis, .. } if *target == P2)));

    let mut grounded = setup(
        Generation::Gen9,
        ("Jolteon", "Thunder Wave"),
        ("Garchomp", "Tackle"),
        |_, _| {},
    );
    let start = grounded.turn(tackle_hit(100));
    assert!(grounded.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::HitBlocked { target, reason: HitBlockReason::TypeImmunity, .. } if *target == P2
    )));
    assert!(grounded.pokemon(P2).status.is_none());
}

#[test]
fn thunder_wave_fails_against_an_already_paralysed_target() {
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Thunder Wave"),
        ("Snorlax", "Tackle"),
        |_, b| {
            b.status = Some(pokemon_battle_engine::state::StatusState::new(
                NonVolatileStatus::Paralysis,
            ));
        },
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.push(RandomOverride::bool(
        RandomRequestKind::FullParalysis,
        false,
    ));
    tape.extend(tackle_hit(100));
    let start = arena.turn(tape);
    let events = arena.events(start);
    assert!(events.iter().any(|e| matches!(
        e,
        TraceEvent::StatusBlocked {
            reason: StatusBlockReason::AlreadyStatus,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(e, TraceEvent::MoveFailed { actor, reason: MoveFailReason::NoEffect, .. } if *actor == P1)));
}

#[test]
fn will_o_wisp_is_refused_by_fire_types_inside_the_status_pipeline() {
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Will-O-Wisp"),
        ("Arcanine", "Tackle"),
        |_, _| {},
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(tackle_hit(100));
    let start = arena.turn(tape);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::StatusBlocked { target, status: NonVolatileStatus::Burn, reason: StatusBlockReason::TypeImmunity, .. } if *target == P2
    )));
    assert!(arena.pokemon(P2).status.is_none());
}

#[test]
fn swords_dance_raises_the_users_attack_and_fails_at_the_cap() {
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Swords Dance"),
        ("Snorlax", "Tackle"),
        |a, _| {
            a.stages.attack = 5;
        },
    );
    let start = arena.turn(tackle_hit(100));
    assert_eq!(arena.pokemon(P1).stages.attack, 6, "+2 clamps at +6");
    assert!(arena.events(start).iter().any(
        |e| matches!(e, TraceEvent::StatChanged { target, from: 5, to: 6, .. } if *target == P1)
    ));

    let start = arena.turn(tackle_hit(100));
    assert!(arena.events(start).iter().any(|e| matches!(e, TraceEvent::MoveFailed { actor, reason: MoveFailReason::NoEffect, .. } if *actor == P1)));
}

#[test]
fn growl_lowers_the_targets_attack() {
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Growl"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let start = arena.turn(tackle_hit(100));
    assert_eq!(arena.pokemon(P2).stages.attack, -1);
    assert!(arena.events(start).iter().any(
        |e| matches!(e, TraceEvent::StatChanged { target, from: 0, to: -1, .. } if *target == P2)
    ));
}

#[test]
fn confuse_ray_confuses_for_a_rolled_duration() {
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Confuse Ray"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let mut tape = vec![RandomOverride::range(RandomRequestKind::Duration, 4)];
    // Snorlax: time 4 → 3, no self-hit, attacks.
    tape.push(RandomOverride::bool(
        RandomRequestKind::ConfusionSelfHit,
        false,
    ));
    tape.extend(tackle_hit(100));
    let start = arena.turn(tape);
    assert!(arena.events(start).iter().any(|e| matches!(e, TraceEvent::VolatileStarted { target, kind: VolatileConditionKind::Confusion } if *target == P2)));
    assert_eq!(arena.pokemon(P2).volatiles.confusion_time(), Some(3));
}

#[test]
fn recover_restores_half_of_max_hp_rounded_and_fails_at_full_hp() {
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Recover"),
        ("Snorlax", "Tackle"),
        |a, _| {
            a.hp = 10;
        },
    );
    let max = arena.pokemon(P1).max_hp;
    let start = arena.turn(tackle_hit(100));
    let healed = arena.events(start).iter().find_map(|e| match e {
        TraceEvent::HealApplied { target, amount, .. } if *target == P1 => Some(*amount),
        _ => None,
    });
    assert_eq!(healed, Some(max.div_ceil(2)), "Gen 5+ rounds half up");

    let mut full = setup(
        Generation::Gen9,
        ("Jolteon", "Recover"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let start = full.turn(tackle_hit(100));
    assert!(full.events(start).iter().any(|e| matches!(e, TraceEvent::MoveFailed { actor, reason: MoveFailReason::NoEffect, .. } if *actor == P1)));
}

#[test]
fn hypnosis_can_miss() {
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Hypnosis"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)];
    tape.extend(tackle_hit(100));
    let start = arena.turn(tape);
    assert!(arena.events(start).iter().any(
        |e| matches!(e, TraceEvent::AccuracyChecked { actor, hit: false, .. } if *actor == P1)
    ));
    assert!(arena.pokemon(P2).status.is_none());
}

#[test]
fn status_moves_fail_against_a_substitute_but_damage_still_hits_it() {
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Thunder Wave"),
        ("Snorlax", "Tackle"),
        |_, b| {
            b.volatiles.set_substitute(P2, 50);
        },
    );
    // Thunder Wave checks its accuracy first: Showdown's Substitute stops a status move in the
    // hit loop, after the accuracy step (it can miss a Pokémon behind a Substitute).
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(tackle_hit(100));
    let start = arena.turn(tape);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::HitBlocked { target, reason: HitBlockReason::Substitute, .. } if *target == P2
    )));
    assert!(arena.pokemon(P2).status.is_none());
}

#[test]
fn protect_blocks_thunder_wave_like_any_protectable_move() {
    // Jolteon protects first; Snorlax's Thunder Wave then hits the gate.
    let mut arena = setup(
        Generation::Gen9,
        ("Jolteon", "Protect"),
        ("Snorlax", "Thunder Wave"),
        |_, _| {},
    );
    let start = arena.turn(vec![]);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::HitBlocked { target, reason: HitBlockReason::Protect, .. } if *target == P1
    )));
    assert!(arena.pokemon(P1).status.is_none());
}

#[test]
fn a_custom_status_move_meets_a_substitute_like_any_status_move() {
    // Showdown `substitute.onTryPrimaryHit`: every status move without `bypasssub` fails
    // against the doll, Leech Seed (a custom primary) included.
    let mut arena = setup(
        Generation::Gen9,
        ("Venusaur", "Leech Seed"),
        ("Snorlax", "Swords Dance"),
        |_, b| b.volatiles.set_substitute(P2, 50),
    );
    let start = arena.turn(vec![RandomOverride::bool(
        RandomRequestKind::Accuracy,
        true,
    )]);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::HitBlocked {
            target: P2,
            reason: HitBlockReason::Substitute,
            ..
        }
    )));
    assert!(
        !arena
            .pokemon(P2)
            .volatiles
            .contains(VolatileConditionKind::LeechSeed)
    );
}

#[test]
fn confuse_ray_on_an_already_confused_target_draws_no_duration() {
    // Showdown `addVolatile` fails on a volatile already there before `onStart` rolls its time:
    // no Duration decision, and the target's own counter is untouched (then ticked at its move).
    let mut arena = setup(
        Generation::Gen9,
        ("Gengar", "Confuse Ray"),
        ("Snorlax", "Growl"),
        |_, b| {
            b.volatiles.set_confusion(P1, 3);
        },
    );
    arena.turn(vec![RandomOverride::bool(
        RandomRequestKind::ConfusionSelfHit,
        false,
    )]);
    assert_eq!(arena.pokemon(P2).volatiles.confusion_time(), Some(2));
}

#[test]
fn taunt_refuses_a_status_move_before_confusion_and_paralysis() {
    // Taunt stops the confused, paralysed Snorlax before either check — Gen 3 too, on the
    // cartridge (issues.md I66; Showdown's gen3 mod checks it last) — so its confusion does not
    // tick. Gen 4 rolls Taunt's duration as it lands, Gen 3's is 2.
    for (generation, tape, confusion_left) in [
        (Generation::Gen3, Vec::new(), 3),
        (
            Generation::Gen4,
            vec![RandomOverride::range(RandomRequestKind::Duration, 3)],
            3,
        ),
    ] {
        let mut arena = setup(
            generation,
            ("Gengar", "Taunt"),
            ("Snorlax", "Growl"),
            |_, b| {
                b.status = Some(StatusState::new(NonVolatileStatus::Paralysis));
                b.volatiles.set_confusion(P1, 3);
            },
        );
        let start = arena.turn(tape);
        assert!(
            arena.events(start).iter().any(|e| matches!(
                e,
                TraceEvent::MoveFailed { actor, reason: MoveFailReason::BeforeMove(BeforeMoveBlockReason::Taunt), .. }
                    if *actor == P2
            )),
            "{generation:?}"
        );
        assert_eq!(
            arena.pokemon(P2).volatiles.confusion_time(),
            Some(confusion_left),
            "{generation:?}"
        );
    }
}

#[test]
fn gen_3_volt_absorb_lets_thunder_wave_through() {
    // The gen3 mod's Volt Absorb `onTryHit` excludes Thunder Wave; from Gen 4 it takes every
    // Electric move.
    for (generation, paralysed) in [(Generation::Gen3, true), (Generation::Gen4, false)] {
        let mut arena = setup(
            generation,
            ("Jolteon", "Thunder Wave"),
            ("Snorlax", "Growl"),
            |_, b| {
                b.ability = AbilityId::VOLT_ABSORB;
            },
        );
        arena.turn(Vec::new());
        assert_eq!(
            arena.pokemon(P2).has_status(NonVolatileStatus::Paralysis),
            paralysed,
            "{generation:?}"
        );
    }
}

#[test]
fn stat_stages_come_before_the_status_or_confusion_that_follows() {
    let failed = |arena: &Arena, start: usize| {
        arena.events(start).iter().any(|e| {
            matches!(
                e,
                TraceEvent::MoveFailed {
                    actor: P1,
                    reason: MoveFailReason::NoEffect,
                    ..
                }
            )
        })
    };
    // Toxic Thread on a poisoned target: Speed still drops, and the move fails.
    let mut arena = setup(
        Generation::Gen9,
        ("Mew", "Toxic Thread"),
        ("Chansey", "Harden"),
        |_, b| b.status = Some(StatusState::new(NonVolatileStatus::Poison)),
    );
    let start = arena.turn(Vec::new());
    assert_eq!(arena.pokemon(P2).stages.speed, -1);
    assert!(failed(&arena, start));
    // Swagger on a confused target: the boost alone makes it work; at +6 nothing does.
    for (attack, fails) in [(0, false), (6, true)] {
        let mut arena = setup(
            Generation::Gen9,
            ("Mew", "Swagger"),
            ("Chansey", "Harden"),
            |_, b| {
                b.stages.attack = attack;
                b.volatiles.set_confusion(P1, 4);
            },
        );
        let start = arena.turn(vec![
            RandomOverride::bool(RandomRequestKind::Accuracy, true),
            RandomOverride::bool(RandomRequestKind::ConfusionSelfHit, false),
        ]);
        assert_eq!(failed(&arena, start), fails, "{attack}");
    }
}

#[test]
fn gen2_swagger_misses_a_maxed_attack_after_its_accuracy_roll() {
    // Gen 2 Swagger's `onTryHit`: stage +6, or an Attack of 999 with its stages.
    for (stage, misses) in [(6, true), (5, true), (4, false)] {
        let mut arena = setup(
            Generation::Gen2,
            ("Mew", "Swagger"),
            ("Snorlax", "Harden"),
            |_, b| {
                b.stats.attack = 300;
                b.stages.attack = stage;
            },
        );
        let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
        if !misses {
            tape.extend([
                RandomOverride::range(RandomRequestKind::Duration, 3),
                RandomOverride::bool(RandomRequestKind::ConfusionSelfHit, false),
            ]);
        }
        let start = arena.turn(tape);
        let confused = arena
            .pokemon(P2)
            .volatiles
            .contains(VolatileConditionKind::Confusion);
        assert_eq!(confused, !misses, "+{stage}");
        assert_eq!(
            arena.events(start).iter().any(|e| matches!(
                e,
                TraceEvent::HitBlocked {
                    reason: HitBlockReason::MoveRule,
                    ..
                }
            )),
            misses,
            "+{stage}"
        );
    }
}

#[test]
fn psycho_shift_rolls_accuracy_before_its_own_rule() {
    // Gen 4 Psycho Shift (accuracy 90) with no status to pass on: its `onTryHit` runs in the
    // hit, after the accuracy roll, so a miss comes first. The replay asks the engine whether
    // accuracy is rolled, so the order is invisible to it.
    for hit in [false, true] {
        let mut arena = setup(
            Generation::Gen4,
            ("Mew", "Psycho Shift"),
            ("Snorlax", "Harden"),
            |_, _| {},
        );
        let start = arena.turn(vec![RandomOverride::bool(RandomRequestKind::Accuracy, hit)]);
        let failed = arena.events(start).iter().any(|e| {
            matches!(
                e,
                TraceEvent::MoveFailed {
                    actor: P1,
                    reason: MoveFailReason::NoEffect,
                    ..
                }
            )
        });
        assert_eq!(failed, hit);
    }
}

#[test]
fn yawn_puts_the_target_to_sleep_at_the_end_of_the_next_turn() {
    // Yawn (`duration: 2`): the first residual ticks it, the second ends it and the sleep's
    // duration is drawn there.
    let mut arena = setup(
        Generation::Gen9,
        ("Mew", "Yawn"),
        ("Snorlax", "Harden"),
        |_, _| {},
    );
    arena.turn(vec![]);
    assert!(arena.pokemon(P2).status.is_none());
    assert!(
        arena
            .pokemon(P2)
            .volatiles
            .contains(VolatileConditionKind::Yawn)
    );
    arena.turn(vec![RandomOverride::range(RandomRequestKind::Duration, 3)]);
    let status = arena.pokemon(P2).status.unwrap();
    assert_eq!(status.kind, NonVolatileStatus::Sleep);
    assert_eq!(status.sleep_turns_rolled, Some(3));
    assert!(
        !arena
            .pokemon(P2)
            .volatiles
            .contains(VolatileConditionKind::Yawn)
    );
}

#[test]
fn yawn_sleep_comes_after_shed_skin_in_the_residual() {
    // Gen 5+: Yawn ends at residual order 23, after Shed Skin (5.3) found nothing to cure, so
    // the new sleep is not rolled for until the next turn. The replay plans Shed Skin only for a
    // Pokémon statused before the residual, and an unplanned roll falls back to the seed:
    // over a range of seeds one of them would cure.
    for seed in 1..=12 {
        let mut arena = setup(
            Generation::Gen5,
            ("Mew", "Yawn"),
            ("Dragonair", "Harden"),
            |_, _| {},
        );
        assert_eq!(arena.pokemon(P2).ability, AbilityId::SHED_SKIN);
        arena.turn_seeded(seed, vec![]);
        arena.turn_seeded(
            seed,
            vec![RandomOverride::range(RandomRequestKind::Duration, 3)],
        );
        assert!(
            arena.pokemon(P2).has_status(NonVolatileStatus::Sleep),
            "seed {seed}"
        );
    }
}

#[test]
fn tar_shot_fails_only_when_neither_part_lands() {
    // Gen 9 Tar Shot: Speed -1, then the tar (`boosts` before `volatileStatus`); the move worked
    // if either part did. A tarred target at -6 takes neither: the move fails.
    for (stage, tarred, fails) in [(-6, true, true), (-5, true, false), (-6, false, false)] {
        let mut arena = setup(
            Generation::Gen9,
            ("Mew", "Tar Shot"),
            ("Snorlax", "Harden"),
            |_, b| {
                b.stages.speed = stage;
                if tarred {
                    assert!(b.volatiles.set_marker(VolatileConditionKind::TarShot));
                }
            },
        );
        let start = arena.turn(vec![]);
        let failed = arena.events(start).iter().any(|e| {
            matches!(
                e,
                TraceEvent::MoveFailed {
                    actor: P1,
                    reason: MoveFailReason::NoEffect,
                    ..
                }
            )
        });
        assert_eq!(failed, fails, "{stage} {tarred}");
        assert!(
            arena
                .pokemon(P2)
                .volatiles
                .contains(VolatileConditionKind::TarShot)
        );
    }
}
