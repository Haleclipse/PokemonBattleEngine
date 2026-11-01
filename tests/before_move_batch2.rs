//! L1 rule tests for the second BeforeMove batch (ROADMAP §6.2, DESIGN.md §15.1): Hyper Beam's
//! recharge turn, Disable, Taunt, Encore and the Choice item lock. Each lives in the shared
//! BeforeMove order and in choice validation (Showdown's request-side `onDisableMove`);
//! the timed ones count down in the residual schedule.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BeforeMoveBlockReason, ChoiceError, EngineStep, MoveFailReason, SideChoice,
    SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, PokemonState, RandomOverride, RandomRequestKind, SideState,
    VolatileConditionKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

/// P1 = [A (Garchomp), C (Vaporeon)]; P2 = [B (Snorlax)]. Garchomp (102) outspeeds Snorlax (30).
fn setup(p1: PokemonSet, p2: PokemonSet) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &set("Vaporeon", &["Tackle"], None)).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

#[derive(Clone, Copy)]
enum Action {
    Move(usize),
    Switch(PokemonId),
}

impl Arena {
    fn choice(&self, side: SideId, action: Action) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
        let action = match action {
            Action::Move(slot) => {
                let move_id = self.engine.state().pokemon(actor).unwrap().moves[slot].move_id;
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
                ActionChoice::Move {
                    slot: MoveSlot::new(slot as u8).unwrap(),
                    target,
                    mechanic: None,
                }
            }
            Action::Switch(pokemon) => ActionChoice::Switch { pokemon },
        };
        SideChoice {
            side,
            actions: vec![SlotChoice { actor, action }],
        }
    }

    fn try_turn(
        &mut self,
        p1: Action,
        p2: Action,
        tape: Vec<RandomOverride>,
    ) -> Result<usize, pokemon_battle_engine::engine::BattleError> {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)];
        self.engine.submit_turn_choices(choices)?;
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
        Ok(start)
    }

    fn turn(&mut self, p1: Action, p2: Action, tape: Vec<RandomOverride>) -> usize {
        self.try_turn(p1, p2, tape).unwrap()
    }

    fn events(&self, from: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[from..]
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn blocked(&self, from: usize, actor: PokemonId, reason: BeforeMoveBlockReason) -> bool {
        self.events(from).iter().any(|e| matches!(
            e,
            TraceEvent::MoveFailed { actor: a, reason: MoveFailReason::BeforeMove(r), .. } if *a == actor && *r == reason
        ))
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

#[test]
fn batch_two_moves_classify_from_snapshot_data() {
    let dex = DexResolver::resolve(Generation::Gen9);
    for name in ["Taunt", "Encore", "Hyper Beam", "Giga Impact"] {
        assert_ne!(
            dex.move_by_name(name).unwrap().primary_effect,
            PrimaryEffect::Unsupported,
            "{name}"
        );
    }
    assert_eq!(
        dex.move_by_name("Disable").unwrap().primary_effect,
        PrimaryEffect::Custom
    );
    assert!(dex.item_by_name("Choice Band").unwrap().implemented);
}

#[test]
fn hyper_beam_spends_the_next_turn_recharging_only_when_it_landed() {
    let mut arena = setup(
        set("Garchomp", &["Hyper Beam"], None),
        set("Snorlax", &["Swords Dance"], None),
    );
    // 90 % accuracy: hit, then the recharge turn.
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(Action::Move(0), Action::Move(0), tape);
    assert!(
        arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.blocked(start, A, BeforeMoveBlockReason::Recharge));
    assert!(
        !arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
    );
    // A miss needs no recharge.
    let start = arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)],
    );
    assert!(
        !arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
    );
    assert!(!arena.blocked(start, A, BeforeMoveBlockReason::Recharge));
}

#[test]
fn the_recharge_turn_takes_any_declared_move_at_priority_zero_and_traps() {
    // Showdown's request holds "Recharge" alone (`mustrecharge`'s `onLockMove`): whatever slot
    // is declared, the action is the priority-0 `recharge`, and a hard lock traps.
    let mut arena = setup(
        set(
            "Snorlax",
            &["Hyper Beam", "Quick Attack"],
            Some("Choice Scarf"),
        ),
        set("Skarmory", &["Swords Dance"], None),
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(Action::Move(0), Action::Move(0), tape);
    assert!(matches!(
        arena.try_turn(Action::Switch(C), Action::Move(0), Vec::new()),
        Err(pokemon_battle_engine::engine::BattleError::Choice(
            ChoiceError::Trapped {
                actor: A,
                by: VolatileConditionKind::MustRecharge,
            }
        ))
    ));
    // Quick Attack is off the Choice-locked request, and its +1 would put Snorlax first.
    let start = arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    let position =
        |pred: &dyn Fn(&TraceEvent) -> bool| arena.events(start).iter().position(pred).unwrap();
    let skarmory = position(&|e| matches!(e, TraceEvent::MoveUsed { actor: B, .. }));
    let recharge = position(&|e| {
        matches!(
            e,
            TraceEvent::MoveFailed {
                actor: A,
                reason: MoveFailReason::BeforeMove(BeforeMoveBlockReason::Recharge),
                ..
            }
        )
    });
    assert!(skarmory < recharge);
}

#[test]
fn encore_overrides_the_move_its_target_chose_this_turn() {
    // Showdown `encore`'s `onOverrideAction`: a target Encored before it moves uses the
    // Encored move instead of the one it chose.
    let mut arena = setup(
        set("Garchomp", &["Encore", "Tackle"], None),
        set("Snorlax", &["Swords Dance", "Tackle"], None),
    );
    arena.turn(Action::Move(1), Action::Move(0), hit(100));
    let start = arena.turn(Action::Move(0), Action::Move(1), Vec::new());
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::MoveOverridden {
            actor: B,
            chosen: MoveId::TACKLE,
            used: MoveId::SWORDS_DANCE,
        }
    )));
    assert_eq!(arena.pokemon(B).stages.attack, 4);
}

#[test]
fn taunt_refuses_status_moves_for_three_turns_and_choice_validation_mirrors_it() {
    let mut arena = setup(
        set("Garchomp", &["Taunt", "Tackle"], None),
        set("Snorlax", &["Swords Dance", "Tackle"], None),
    );
    // Garchomp Taunts first; Snorlax still had its move to make, so 3 turns: it is refused
    // this turn (BeforeMove) and cannot even be declared the next two.
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.blocked(start, B, BeforeMoveBlockReason::Taunt));
    assert_eq!(
        arena
            .pokemon(B)
            .volatiles
            .turns_left(VolatileConditionKind::Taunt),
        Some(2)
    );
    assert!(matches!(
        arena.try_turn(Action::Move(1), Action::Move(0), hit(100)),
        Err(pokemon_battle_engine::engine::BattleError::Choice(
            ChoiceError::MoveDisabled {
                actor: B,
                by: VolatileConditionKind::Taunt,
                ..
            }
        ))
    ));
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.turn(Action::Move(1), Action::Move(1), tape.clone());
    let start = arena.turn(Action::Move(1), Action::Move(1), tape);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::VolatileEnded {
            target: B,
            kind: VolatileConditionKind::Taunt,
            ..
        }
    )));
    assert!(
        !arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::Taunt)
    );
    // Swords Dance is allowed again.
    arena.turn(Action::Move(1), Action::Move(0), hit(100));
    assert_eq!(arena.pokemon(B).stages.attack, 2);

    // A target that has been out for a turn and already moved this turn is taunted for one
    // turn longer (Showdown: `activeTurns && !willMove` → duration 4).
    let mut arena = setup(
        set("Snorlax", &["Tackle", "Taunt"], None),
        set("Garchomp", &["Tackle"], None),
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.turn(Action::Move(0), Action::Move(0), tape);
    arena.turn(Action::Move(1), Action::Move(0), hit(100));
    assert_eq!(
        arena
            .pokemon(B)
            .volatiles
            .turns_left(VolatileConditionKind::Taunt),
        Some(3),
        "4 turns, one ticked"
    );
}

#[test]
fn encore_locks_the_last_move_and_ends_early_without_pp() {
    let mut arena = setup(
        set("Garchomp", &["Encore", "Tackle"], None),
        set("Snorlax", &["Swords Dance", "Tackle"], None),
    );
    // Encore before the target has moved this turn fails (no last move yet).
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveFailed { actor: A, .. }))
    );
    assert_eq!(arena.pokemon(B).stages.attack, 2);
    // Now Snorlax's last move is Swords Dance: Encore locks it for 3 turns (it still moves
    // this turn, so no extra turn).
    let start = arena.turn(Action::Move(0), Action::Move(1), Vec::new());
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::VolatileStarted {
            target: B,
            kind: VolatileConditionKind::Encore
        }
    )));
    assert!(matches!(
        arena.try_turn(Action::Move(1), Action::Move(1), hit(100)),
        Err(pokemon_battle_engine::engine::BattleError::Choice(
            ChoiceError::MoveDisabled {
                actor: B,
                by: VolatileConditionKind::Encore,
                ..
            }
        ))
    ));
    assert_eq!(
        arena
            .pokemon(B)
            .volatiles
            .turns_left(VolatileConditionKind::Encore),
        Some(2)
    );

    // Out of PP ends it early: drain Swords Dance's PP.
    {
        let mut snapshot = arena.engine.snapshot();
        snapshot.state.pokemon_mut(B).unwrap().moves[0].pp = 1;
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    }
    let start = arena.turn(Action::Move(1), Action::Move(0), hit(100));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::VolatileEnded {
            target: B,
            kind: VolatileConditionKind::Encore,
            ..
        }
    )));
    assert!(
        !arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::Encore)
    );
}

#[test]
fn disable_seals_the_last_move_for_four_turns_when_the_target_still_moves() {
    let mut arena = setup(
        set("Garchomp", &["Disable", "Tackle"], None),
        set("Snorlax", &["Tackle", "Swords Dance"], None),
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.turn(Action::Move(1), Action::Move(0), tape);
    // Snorlax's last move is Tackle; it still moves this turn, so 5 - 1 = 4 turns, and its
    // Tackle this turn is refused.
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::VolatileStarted {
            target: B,
            kind: VolatileConditionKind::Disable
        }
    )));
    assert!(arena.blocked(start, B, BeforeMoveBlockReason::Disabled));
    assert_eq!(
        arena
            .pokemon(B)
            .volatiles
            .turns_left(VolatileConditionKind::Disable),
        Some(3)
    );
    assert!(matches!(
        arena.try_turn(Action::Move(1), Action::Move(0), hit(100)),
        Err(pokemon_battle_engine::engine::BattleError::Choice(
            ChoiceError::MoveDisabled {
                actor: B,
                by: VolatileConditionKind::Disable,
                ..
            }
        ))
    ));
    for _ in 0..3 {
        arena.turn(Action::Move(1), Action::Move(1), hit(100));
    }
    assert!(
        !arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::Disable)
    );
    // Disable without a last move fails.
    let mut arena = setup(
        set("Garchomp", &["Disable"], None),
        set("Snorlax", &["Tackle"], None),
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveFailed { actor: A, .. }))
    );
}

#[test]
fn a_choice_item_locks_the_first_move_until_the_holder_switches_out() {
    let mut arena = setup(
        set("Garchomp", &["Tackle", "Swords Dance"], Some("Choice Band")),
        set("Snorlax", &["Swords Dance"], None),
    );
    arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert_eq!(
        arena.pokemon(A).volatiles.choice_locked_move(),
        Some(arena.pokemon(A).moves[0].move_id)
    );
    assert!(matches!(
        arena.try_turn(Action::Move(1), Action::Move(0), Vec::new()),
        Err(pokemon_battle_engine::engine::BattleError::Choice(
            ChoiceError::MoveDisabled {
                actor: A,
                by: VolatileConditionKind::ChoiceLock,
                ..
            }
        ))
    ));
    arena.turn(Action::Move(0), Action::Move(0), hit(100));
    // Switching out clears the lock; coming back starts fresh.
    arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    arena.turn(Action::Switch(A), Action::Move(0), Vec::new());
    assert_eq!(arena.pokemon(A).volatiles.choice_locked_move(), None);
    arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    assert_eq!(
        arena.pokemon(A).volatiles.choice_locked_move(),
        Some(arena.pokemon(A).moves[1].move_id)
    );
}

#[test]
fn gorilla_tactics_locks_the_first_move_but_never_struggle() {
    // Gorilla Tactics' lock is the ability's (`AbilityState::choice_lock`, Showdown
    // `abilityState.choiceLock`): the request offers nothing else — invisible to the replay,
    // which only ever sends the choices Showdown's request allowed — and coming back in starts it
    // over.
    let gorilla = |moves: &[&str]| {
        let mut set = set("Garchomp", moves, None);
        set.ability = Some("Gorilla Tactics".into());
        set
    };
    let mut arena = setup(
        gorilla(&["Tackle", "Swords Dance"]),
        set("Snorlax", &["Swords Dance"], None),
    );
    arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert_eq!(
        arena.pokemon(A).ability_state.choice_lock,
        Some(MoveId::TACKLE)
    );
    assert!(matches!(
        arena.try_turn(Action::Move(1), Action::Move(0), Vec::new()),
        Err(pokemon_battle_engine::engine::BattleError::Choice(
            ChoiceError::MoveDisabled {
                actor: A,
                by: VolatileConditionKind::ChoiceLock,
                ..
            }
        ))
    ));
    arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    arena.turn(Action::Switch(A), Action::Move(0), Vec::new());
    assert_eq!(arena.pokemon(A).ability_state.choice_lock, None);
    arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    assert_eq!(
        arena.pokemon(A).ability_state.choice_lock,
        Some(MoveId::SWORDS_DANCE)
    );
    // BeforeMove refuses a move the lock does not allow (`-fail`, no PP): a lock taken after the
    // choice — here set between the two actions of the turn.
    let mut arena = setup(
        {
            let mut set = set("Snorlax", &["Tackle", "Swords Dance"], None);
            set.ability = Some("Gorilla Tactics".into());
            set
        },
        set("Garchomp", &["Swords Dance"], None),
    );
    let start = arena.engine.trace().events().len();
    let choices = [
        arena.choice(SideId::P1, Action::Move(1)),
        arena.choice(SideId::P2, Action::Move(0)),
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    arena.engine.step().unwrap();
    let mut snapshot = arena.engine.snapshot();
    snapshot
        .state
        .pokemon_mut(A)
        .unwrap()
        .ability_state
        .choice_lock = Some(MoveId::TACKLE);
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let pp = arena.pokemon(A).moves[1].pp;
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(arena.blocked(start, A, BeforeMoveBlockReason::ChoiceLock));
    assert_eq!(arena.pokemon(A).moves[1].pp, pp);
    // Struggle does not lock it (`onModifyMove` skips it): once a move has PP again, it is free.
    let mut arena = setup(
        gorilla(&["Tackle"]),
        set("Snorlax", &["Swords Dance"], None),
    );
    {
        let mut snapshot = arena.engine.snapshot();
        snapshot.state.pokemon_mut(A).unwrap().moves[0].pp = 0;
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    }
    arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert_eq!(arena.pokemon(A).ability_state.choice_lock, None);
}

#[test]
fn with_no_move_left_the_action_is_a_typeless_struggle_with_a_quarter_recoil() {
    // Showdown `getMoves`: the Choice-locked move is out of PP and the others are locked away,
    // so the request holds Struggle alone: any declared slot becomes Struggle (priority 0, no PP,
    // no Choice lock onto it), typeless (it hits a Ghost), with a quarter of max HP recoil.
    let mut arena = setup(
        set(
            "Garchomp",
            &["Tackle", "Quick Attack"],
            Some("Choice Scarf"),
        ),
        set("Gengar", &["Swords Dance"], None),
    );
    {
        let mut snapshot = arena.engine.snapshot();
        let garchomp = snapshot.state.pokemon_mut(A).unwrap();
        garchomp.moves[0].pp = 0;
        garchomp.volatiles.set_choice_lock(MoveId::TACKLE);
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    }
    assert!(pokemon_battle_engine::engine::must_struggle(
        arena.engine.state(),
        &arena.rules,
        &arena.dex,
        A
    ));
    let max_hp = arena.pokemon(A).max_hp;
    let quick_attack_pp = arena.pokemon(A).moves[1].pp;
    let start = arena.turn(Action::Move(1), Action::Move(0), hit(100));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::MoveUsed {
            actor: A,
            move_id: MoveId::STRUGGLE,
            ..
        }
    )));
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::DamageApplied { target: B, .. }))
    );
    assert_eq!(arena.pokemon(A).moves[1].pp, quick_attack_pp);
    assert_eq!(arena.pokemon(A).hp, max_hp - (max_hp + 2) / 4);
}

#[test]
fn a_flinch_stops_the_move_and_lasts_until_the_end_of_the_turn() {
    // Showdown `flinch` (`duration: 1`): `onBeforeMove` refuses the move without removing it;
    // the residual's duration countdown does, last.
    let mut arena = setup(
        set("Garchomp", &["Fake Out"], None),
        set("Snorlax", &["Tackle"], None),
    );
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(1, hit(100));
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let start = arena.engine.trace().events().len();
    let choices = [
        arena.choice(SideId::P1, Action::Move(0)),
        arena.choice(SideId::P2, Action::Move(0)),
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    let flinched = |arena: &Arena| {
        arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::Flinch)
    };
    arena.engine.step().unwrap(); // Fake Out
    assert!(flinched(&arena));
    arena.engine.step().unwrap(); // Snorlax's Tackle, refused
    assert!(arena.blocked(start, B, BeforeMoveBlockReason::Flinch));
    assert!(flinched(&arena));
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(!flinched(&arena));
    assert!(arena.engine.state().rng.semantic_tape_exhausted());
}

#[test]
fn encore_cannot_lock_a_target_into_encore() {
    // Showdown `failencore` (Encore, Transform, Mimic …): Garchomp's last move is Encore, so
    // Snorlax's Encore on it fails.
    let mut arena = setup(
        set("Garchomp", &["Encore"], None),
        set("Snorlax", &["Encore"], None),
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(
        !arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::Encore)
    );
}

#[test]
fn gigaton_hammer_is_off_the_request_right_after_it_was_used() {
    // Showdown's `DisableMove` pass (`cantusetwice`): the move its user used last cannot be
    // chosen again; any other move in between frees it. Showdown never offers the choice, so
    // the replay cannot see the refusal.
    let mut arena = setup(
        set("Garchomp", &["Gigaton Hammer", "Swords Dance"], None),
        set("Snorlax", &["Swords Dance"], None),
    );
    let hammer = arena.pokemon(A).moves[0].move_id;
    arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(matches!(
        arena.try_turn(Action::Move(0), Action::Move(0), Vec::new()),
        Err(pokemon_battle_engine::engine::BattleError::Choice(
            ChoiceError::MoveUsedTwice { actor: A, move_id }
        )) if move_id == hammer
    ));
    assert!(!pokemon_battle_engine::engine::must_struggle(
        arena.engine.state(),
        &arena.rules,
        &arena.dex,
        A
    ));
    arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    arena.turn(Action::Move(0), Action::Move(0), hit(100));
}
