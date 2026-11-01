//! L1 tests for Thrash and its kind (DESIGN.md §15.12): what the replay cannot see — the
//! request a locked Pokémon gets, the PP its later turns do not spend, the cartridge's count
//! behind the Gen 3-4 frames, Gen 1's carried accuracy byte.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, ChoiceError, EngineStep, SideChoice, SlotChoice, accuracy_roll,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState, VolatileConditionKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(generation: Generation, species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    if generation >= Generation::Gen3 {
        set.ability = Some("Run Away".into());
    }
    set.level = level;
    set
}

/// P1 = [A (the rampager, `moves`), C (bench)], P2 = [B, a sturdy wall with `foe` moves].
fn arena(generation: Generation, moves: &[&str], foe: &[&str]) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &set(generation, "Snorlax", moves, 30)).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &set(generation, "Chansey", foe, 100)).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &set(generation, "Snorlax", foe, 50)).unwrap(),
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

impl Arena {
    fn edit(&mut self, edit: impl FnOnce(&mut BattleState)) {
        let mut snapshot = self.engine.snapshot();
        edit(&mut snapshot.state);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
    }

    fn turn(&mut self, a: usize, b: usize, tape: Vec<RandomOverride>) {
        self.edit(|state| state.rng = BattleRng::with_semantic_tape(1, tape));
        let action = |actor: PokemonId, slot: usize| SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(slot as u8).unwrap(),
                target: None,
                mechanic: None,
            },
        };
        self.engine
            .submit_turn_choices([
                SideChoice {
                    side: SideId::P1,
                    actions: vec![action(A, a)],
                },
                SideChoice {
                    side: SideId::P2,
                    actions: vec![action(B, b)],
                },
            ])
            .unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
    }

    fn user(&self) -> &PokemonState {
        self.engine.state().pokemon(A).unwrap()
    }

    fn validate(&self, action: ActionChoice) -> Result<(), ChoiceError> {
        self.engine.validate_choice(
            SideId::P1,
            &SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice { actor: A, action }],
            },
        )
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn duration(value: u16) -> RandomOverride {
    RandomOverride::range(RandomRequestKind::Duration, value)
}

#[test]
fn a_rampage_holds_its_user_without_pp_and_ends_in_fatigue() {
    // Gen 9: the hidden length 2; the first use spends PP, the second does not and ends the lock
    // as the move is over, confusing its user.
    let mut arena = arena(Generation::Gen9, &["Outrage", "Harden"], &["Harden"]);
    let mut tape = hit(90);
    tape.push(duration(2));
    arena.turn(0, 0, tape);
    let pp = arena.user().moves[0].pp;
    let rampage = *arena.user().volatiles.rampage().unwrap();
    assert_eq!((rampage.turns, rampage.left), (1, 1));

    // The request holds Outrage alone: no switch, no other move.
    assert_eq!(
        arena.validate(ActionChoice::Switch { pokemon: C }),
        Err(ChoiceError::Trapped {
            actor: A,
            by: VolatileConditionKind::Rampage
        })
    );
    assert!(matches!(
        arena.validate(ActionChoice::Move {
            slot: MoveSlot::new(1).unwrap(),
            target: None,
            mechanic: None,
        }),
        Err(ChoiceError::MoveDisabled {
            by: VolatileConditionKind::Rampage,
            ..
        })
    ));

    let mut tape = hit(90);
    tape.push(duration(3));
    arena.turn(0, 0, tape);
    assert_eq!(arena.user().moves[0].pp, pp, "a locked turn spends no PP");
    assert!(arena.user().volatiles.rampage().is_none());
    assert!(
        arena
            .user()
            .volatiles
            .contains(VolatileConditionKind::Confusion)
    );
}

#[test]
fn a_gen5_rampage_ends_silently_when_it_misses_before_its_last_turn() {
    // Length 3: a miss on the second turn ends the lock with no confusion (no roll for one).
    let mut arena = arena(Generation::Gen9, &["Outrage"], &["Harden"]);
    let mut tape = hit(90);
    tape.push(duration(3));
    arena.turn(0, 0, tape);
    arena.edit(|state| state.pokemon_mut(B).unwrap().stages.evasion = 1);
    arena.turn(
        0,
        0,
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)],
    );
    assert!(arena.user().volatiles.rampage().is_none());
    assert!(
        !arena
            .user()
            .volatiles
            .contains(VolatileConditionKind::Confusion)
    );
}

#[test]
fn the_cartridges_gen4_count_goes_on_after_a_miss_and_a_failed_last_turn_is_silent() {
    // Gen 4 (issues.md I61): 3 turns; a miss leaves the count going down; the last turn lost to
    // full paralysis ends the lock without confusion.
    let mut arena = arena(Generation::Gen4, &["Thrash"], &["Harden"]);
    let mut tape = hit(90);
    tape.push(duration(3));
    arena.turn(0, 0, tape);
    assert_eq!(arena.user().volatiles.rampage().unwrap().turns, 2);
    arena.edit(|state| state.pokemon_mut(B).unwrap().stages.evasion = 1);
    arena.turn(
        0,
        0,
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)],
    );
    assert_eq!(arena.user().volatiles.rampage().unwrap().turns, 1);
    arena.edit(|state| {
        state.pokemon_mut(A).unwrap().status = Some(StatusState::new(NonVolatileStatus::Paralysis))
    });
    arena.turn(
        0,
        0,
        vec![RandomOverride::bool(RandomRequestKind::FullParalysis, true)],
    );
    assert!(arena.user().volatiles.rampage().is_none());
    assert!(
        !arena
            .user()
            .volatiles
            .contains(VolatileConditionKind::Confusion)
    );
}

#[test]
fn gen3_protect_ends_the_lock_at_once_and_gen4_protect_does_not() {
    for (generation, ends) in [(Generation::Gen3, true), (Generation::Gen4, false)] {
        let mut arena = arena(generation, &["Thrash"], &["Protect", "Harden"]);
        let mut tape = hit(90);
        tape.push(duration(3));
        arena.turn(0, 1, tape);
        arena.turn(0, 0, Vec::new());
        assert_eq!(
            arena.user().volatiles.rampage().is_none(),
            ends,
            "{generation:?}"
        );
        assert!(
            !arena
                .user()
                .volatiles
                .contains(VolatileConditionKind::Confusion),
            "{generation:?}"
        );
    }
}

#[test]
fn gen1_thrash_spends_pp_once_and_carries_its_accuracy_byte() {
    // Gen 1: the uses drawn before the hit test (2: three uses), the byte 255 × 66 % = 168
    // against +1 evasion kept for the next use: 168 × 66 % = 110.
    let mut arena = arena(Generation::Gen1, &["Thrash"], &["Harden"]);
    arena.edit(|state| state.pokemon_mut(B).unwrap().stages.evasion = 1);
    let mut tape = vec![
        duration(2),
        RandomOverride::bool(RandomRequestKind::Accuracy, true),
    ];
    tape.extend(hit(230));
    arena.turn(0, 0, tape);
    let pp = arena.user().moves[0].pp;
    assert_eq!(arena.user().volatiles.rampage().unwrap().accuracy, 168);
    let thrash = arena
        .dex
        .move_by_id(arena.user().moves[0].move_id)
        .unwrap()
        .clone();
    assert_eq!(
        accuracy_roll(arena.engine.state(), &arena.rules, A, B, &thrash),
        Some((110, 256))
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(230));
    arena.turn(0, 0, tape);
    assert_eq!(arena.user().moves[0].pp, pp, "a locked use spends no PP");
    assert_eq!(arena.user().volatiles.rampage().unwrap().turns, 1);
}

#[test]
fn gen1_full_paralysis_ends_the_lock_without_confusion() {
    let mut arena = arena(Generation::Gen1, &["Petal Dance"], &["Harden"]);
    let mut tape = vec![
        duration(3),
        RandomOverride::bool(RandomRequestKind::Accuracy, true),
    ];
    tape.extend(hit(230));
    arena.turn(0, 0, tape);
    arena.edit(|state| {
        state.pokemon_mut(A).unwrap().status = Some(StatusState::new(NonVolatileStatus::Paralysis))
    });
    arena.turn(
        0,
        0,
        vec![RandomOverride::bool(RandomRequestKind::FullParalysis, true)],
    );
    assert!(arena.user().volatiles.rampage().is_none());
    assert!(
        !arena
            .user()
            .volatiles
            .contains(VolatileConditionKind::Confusion)
    );
}

#[test]
fn gen2_fatigue_confuses_for_two_or_three_turns() {
    // Gen 2 (pokecrystal `BattleCommand_CheckRampage`, issues.md I108): the drawn 2 leaves one
    // more use, whose start confuses its user for `BattleRandom & 1` + 2 turns — a draw of 3 here,
    // where the ordinary 2-5 would read the same raw value as 5.
    let mut arena = arena(Generation::Gen2, &["Thrash"], &["Harden"]);
    let mut tape = vec![duration(2)];
    tape.extend(hit(230));
    arena.turn(0, 0, tape);
    assert_eq!(arena.user().volatiles.rampage().unwrap().turns, 1);
    arena.edit(|state| state.rng = BattleRng::with_tape(1, vec![3; 32]));
    arena
        .engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: A,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: None,
                    },
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: B,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: None,
                        mechanic: None,
                    },
                }],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(arena.user().volatiles.rampage().is_none());
    assert_eq!(arena.user().volatiles.confusion_time(), Some(3));
}
