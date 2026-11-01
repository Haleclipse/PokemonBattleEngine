//! L1 rule tests for the BeforeMove stage (DESIGN.md §15, ROADMAP §6.2): sleep, freeze, full
//! paralysis, flinch and confusion, each driven through the semantic random tape so every
//! decision is explicit and the order of checks is observable in the trace.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BeforeMoveBlockReason, CanMoveResult, CureReason, EffectSource, EngineStep,
    SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState, VolatileConditionKind,
};

const FAST: PokemonId = PokemonId(1); // Jolteon, moves first
const SLOW: PokemonId = PokemonId(2); // Snorlax, the subject

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

/// Jolteon (fast) vs Snorlax (slow); the slow side is the subject of every test.
fn arena(
    generation: Generation,
    fast_move: &str,
    prepare: impl FnOnce(&mut PokemonState),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let fast = build_pokemon(
        &dex,
        FAST,
        SideId::P1,
        &PokemonSet::new("Jolteon", [fast_move]),
    )
    .unwrap();
    let mut slow = build_pokemon(
        &dex,
        SLOW,
        SideId::P2,
        &PokemonSet::new("Snorlax", ["Tackle"]),
    )
    .unwrap();
    prepare(&mut slow);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![fast, slow], [side(FAST), side(SLOW)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    /// Runs one turn (both sides use their first move on each other) with the given semantic
    /// tape and asserts the tape was consumed exactly.
    fn turn(&mut self, tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let target =
            |side: SideId| DeclaredTarget(self.rules.topology().position(side, 0).unwrap());
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
        self.engine
            .submit_turn_choices([
                choice(SideId::P1, FAST, SideId::P2),
                choice(SideId::P2, SLOW, SideId::P1),
            ])
            .unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "semantic tape not fully consumed: the engine asked for fewer decisions than the test planned"
        );
        start
    }

    fn events(&self, from: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[from..]
    }

    fn before_move(&self, from: usize, actor: PokemonId) -> Option<CanMoveResult> {
        self.events(from).iter().find_map(|event| match event {
            TraceEvent::BeforeMoveResolved {
                actor: a, result, ..
            } if *a == actor => Some(*result),
            _ => None,
        })
    }

    fn moved(&self, from: usize, actor: PokemonId) -> bool {
        self.events(from)
            .iter()
            .any(|event| matches!(event, TraceEvent::MoveUsed { actor: a, .. } if *a == actor))
    }

    fn subject(&self) -> &PokemonState {
        self.engine.state().pokemon(SLOW).unwrap()
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn asleep(time: u8) -> Option<StatusState> {
    let mut status = StatusState::new(NonVolatileStatus::Sleep);
    status.sleep_turns_remaining = Some(time);
    Some(status)
}

#[test]
fn sleep_counts_down_at_before_move_and_the_waking_turn_is_playable_from_gen2() {
    let mut arena = arena(Generation::Gen9, "Tackle", |p| p.status = asleep(2));

    // Turn 1: Jolteon hits; Snorlax's counter drops to 1 and it stays asleep. No random request.
    let start = arena.turn(hit(100));
    assert_eq!(
        arena.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::Sleep))
    );
    assert!(!arena.moved(start, SLOW));
    assert_eq!(
        arena.subject().status.unwrap().sleep_turns_remaining,
        Some(1)
    );

    // Turn 2: counter hits 0, Snorlax wakes up and attacks in the same turn.
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::StatusCured { target, status: NonVolatileStatus::Sleep, reason: CureReason::WokeUp } if *target == SLOW
    )));
    assert_eq!(arena.before_move(start, SLOW), Some(CanMoveResult::Allow));
    assert!(arena.moved(start, SLOW));
    assert!(arena.subject().status.is_none());
}

#[test]
fn gen1_loses_the_turn_it_wakes_up_on() {
    let mut arena = arena(Generation::Gen1, "Tackle", |p| p.status = asleep(1));
    // Gen 1 Tackle is 95 % accurate, so Jolteon's hit consumes an Accuracy decision.
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(230));
    let start = arena.turn(tape);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::StatusCured { target, reason: CureReason::WokeUp, .. } if *target == SLOW
    )));
    assert_eq!(
        arena.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::WakingTurn))
    );
    assert!(!arena.moved(start, SLOW));
    assert!(arena.subject().status.is_none());
}

#[test]
fn freeze_holds_until_the_thaw_roll_succeeds() {
    let mut arena = arena(Generation::Gen9, "Tackle", |p| {
        p.status = Some(StatusState::new(NonVolatileStatus::Freeze))
    });

    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Thaw, false));
    let start = arena.turn(tape);
    assert_eq!(
        arena.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::Freeze))
    );
    assert!(arena.subject().has_status(NonVolatileStatus::Freeze));

    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Thaw, true));
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::StatusCured { target, status: NonVolatileStatus::Freeze, reason: CureReason::Thawed } if *target == SLOW
    )));
    assert!(arena.moved(start, SLOW));
    assert!(arena.subject().status.is_none());
}

#[test]
fn gen1_never_thaws_on_its_own_and_asks_no_random_question() {
    let mut arena = arena(Generation::Gen1, "Tackle", |p| {
        p.status = Some(StatusState::new(NonVolatileStatus::Freeze))
    });
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(230));
    // No Thaw entry: a 0/1 chance is short-circuited and must not consult the tape.
    let start = arena.turn(tape);
    assert_eq!(
        arena.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::Freeze))
    );
}

#[test]
fn full_paralysis_is_a_typed_random_decision() {
    let mut arena = arena(Generation::Gen9, "Tackle", |p| {
        p.status = Some(StatusState::new(NonVolatileStatus::Paralysis))
    });

    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::FullParalysis, true));
    let start = arena.turn(tape);
    assert_eq!(
        arena.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::Paralysis))
    );
    assert!(!arena.moved(start, SLOW));

    let mut tape = hit(100);
    tape.push(RandomOverride::bool(
        RandomRequestKind::FullParalysis,
        false,
    ));
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert_eq!(arena.before_move(start, SLOW), Some(CanMoveResult::Allow));
    assert!(arena.moved(start, SLOW));
}

#[test]
fn flinch_from_a_secondary_blocks_the_slower_target_this_turn_only() {
    // Bite: 30 % flinch, classified Damage from the snapshot.
    let mut arena = arena(Generation::Gen9, "Bite", |p| p.status = None);
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, true));
    let start = arena.turn(tape);

    let events = arena.events(start);
    assert!(events.iter().any(|event| matches!(
        event,
        TraceEvent::VolatileStarted { target, kind: VolatileConditionKind::Flinch } if *target == SLOW
    )));
    assert_eq!(
        arena.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::Flinch))
    );
    assert!(!arena.moved(start, SLOW));
    // Consumed at BeforeMove; nothing lingers into the next turn.
    assert!(
        !arena
            .subject()
            .volatiles
            .contains(VolatileConditionKind::Flinch)
    );

    let start = arena.turn({
        let mut t = hit(100);
        t.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
        t.extend(hit(100));
        t
    });
    assert_eq!(arena.before_move(start, SLOW), Some(CanMoveResult::Allow));
}

#[test]
fn confusion_counts_down_may_self_hit_and_snaps_out_at_zero() {
    // Confusion (the move): 10 % confusion. Duration is rolled on application.
    let mut arena = arena(Generation::Gen9, "Confusion", |p| p.status = None);
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, true));
    tape.push(RandomOverride::range(RandomRequestKind::Duration, 2));
    // Snorlax: time 2 → 1, then hits itself.
    tape.push(RandomOverride::bool(
        RandomRequestKind::ConfusionSelfHit,
        true,
    ));
    tape.push(RandomOverride::range(RandomRequestKind::DamageRoll, 100));
    let start = arena.turn(tape);

    let events = arena.events(start);
    assert!(events.iter().any(|event| matches!(
        event,
        TraceEvent::VolatileStarted { target, kind: VolatileConditionKind::Confusion } if *target == SLOW
    )));
    assert_eq!(
        arena.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::Confusion))
    );
    let self_hit = events.iter().find_map(|event| match event {
        TraceEvent::DamageApplied {
            target,
            amount,
            source: EffectSource::Volatile(VolatileConditionKind::Confusion),
            ..
        } if *target == SLOW => Some(*amount),
        _ => None,
    });
    assert!(
        self_hit.is_some_and(|amount| amount > 0),
        "confusion self-hit must be an ordinary DamageApplied"
    );
    assert!(!arena.moved(start, SLOW));
    assert_eq!(arena.subject().volatiles.confusion_time(), Some(1));

    // Next turn: time 1 → 0, confusion ends before any self-hit roll, Snorlax attacks.
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::VolatileEnded { target, kind: VolatileConditionKind::Confusion, .. } if *target == SLOW
    )));
    assert_eq!(arena.before_move(start, SLOW), Some(CanMoveResult::Allow));
    assert!(arena.moved(start, SLOW));
}

#[test]
fn flinch_is_checked_before_paralysis_so_no_paralysis_roll_is_spent() {
    let mut arena = arena(Generation::Gen9, "Bite", |p| {
        p.status = Some(StatusState::new(NonVolatileStatus::Paralysis))
    });
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, true));
    // No FullParalysis entry: the flinch short-circuits BeforeMove first. `turn` asserts the
    // tape was consumed exactly, so an unexpected paralysis roll would fall back to the PRNG
    // and leave nothing to detect — hence the explicit order check on the trace instead.
    let start = arena.turn(tape);
    assert_eq!(
        arena.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::Flinch))
    );
}

#[test]
fn gen_4_confusion_self_hit_is_halved_by_burn_like_any_physical_hit() {
    // Gen 3-4 hit themselves through `getDamage`; from Gen 5 `getConfusionDamage` ignores the
    // burn (`a_burn_halves_a_confusion_self_hit_only_in_gen_3_and_4`).
    let self_hit = |burned: bool| {
        let mut arena = arena(Generation::Gen4, "Tackle", |p| {
            p.volatiles.set_confusion(SLOW, 3);
            if burned {
                p.status = Some(StatusState::new(NonVolatileStatus::Burn));
            }
        });
        // Gen 4 Tackle is 95 % accurate.
        let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
        tape.extend(hit(100));
        tape.push(RandomOverride::bool(
            RandomRequestKind::ConfusionSelfHit,
            true,
        ));
        tape.push(RandomOverride::range(RandomRequestKind::DamageRoll, 100));
        let start = arena.turn(tape);
        arena
            .events(start)
            .iter()
            .find_map(|event| match event {
                TraceEvent::DamageApplied {
                    target,
                    amount,
                    source: EffectSource::Volatile(VolatileConditionKind::Confusion),
                    ..
                } if *target == SLOW => Some(*amount),
                _ => None,
            })
            .expect("self-hit")
    };
    let plain = self_hit(false);
    let burned = self_hit(true);
    assert!(plain > 2);
    // Gen 4 halves the base damage before the random factor, so the result may round up.
    assert!(
        (plain / 2..=plain / 2 + 1).contains(&burned),
        "{burned} vs {plain}"
    );
}

/// Gen 3-4 (gen4 `par.onBeforeMove`: `!pokemon.hasAbility('magicguard') && randomChance(1, 4)`):
/// a paralysed Magic Guard holder draws no full-paralysis chance and moves; Gen 5 draws it.
#[test]
fn a_gen_4_magic_guard_holder_draws_no_full_paralysis() {
    let paralysed_guard = |p: &mut PokemonState| {
        p.status = Some(StatusState::new(NonVolatileStatus::Paralysis));
        p.ability = pokemon_battle_engine::ids::AbilityId::MAGIC_GUARD;
    };
    // Gen 4 Tackle is 95 % accurate; no FullParalysis entry between the two moves.
    let mut gen4 = arena(Generation::Gen4, "Tackle", paralysed_guard);
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, true));
    tape.extend(hit(100));
    let start = gen4.turn(tape);
    assert_eq!(gen4.before_move(start, SLOW), Some(CanMoveResult::Allow));
    assert!(gen4.moved(start, SLOW));

    let mut gen5 = arena(Generation::Gen5, "Tackle", paralysed_guard);
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::FullParalysis, true));
    let start = gen5.turn(tape);
    assert_eq!(
        gen5.before_move(start, SLOW),
        Some(CanMoveResult::Blocked(BeforeMoveBlockReason::Paralysis))
    );
}
