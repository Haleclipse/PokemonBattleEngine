//! L1 tests for Uproar (DESIGN.md §15.12): what the replay cannot see — the request an
//! uproar's holder gets, the PP its later turns do not spend (and Gen 3's that they do on a
//! miss, issues.md I65), Gen 3's cancel the moment its holder is frozen.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, ChoiceError, EngineStep, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, VolatileConditionKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some("Run Away".into());
    set.level = level;
    set
}

/// P1 = [A (the user, `moves`), C (bench)], P2 = [B, a sturdy wall with `foe` moves].
fn arena(generation: Generation, moves: &[&str], foe: &[&str]) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &set("Snorlax", moves, 30)).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &set("Chansey", foe, 100)).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &set("Snorlax", foe, 50)).unwrap(),
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

    fn uproar_turns(&self) -> Option<u8> {
        self.user().volatiles.uproar().map(|uproar| uproar.turns)
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

fn miss() -> Vec<RandomOverride> {
    vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)]
}

#[test]
fn an_uproar_holds_its_user_without_pp_for_three_turns() {
    // Gen 9: 3 turns, drawn from nothing; the first use spends PP, the later ones do not.
    let mut arena = arena(Generation::Gen9, &["Uproar", "Harden"], &["Harden"]);
    arena.turn(0, 0, hit(90));
    let pp = arena.user().moves[0].pp;
    assert_eq!(arena.uproar_turns(), Some(2));

    // The request holds Uproar alone: no switch, no other move.
    assert_eq!(
        arena.validate(ActionChoice::Switch { pokemon: C }),
        Err(ChoiceError::Trapped {
            actor: A,
            by: VolatileConditionKind::Uproar
        })
    );
    assert!(matches!(
        arena.validate(ActionChoice::Move {
            slot: MoveSlot::new(1).unwrap(),
            target: None,
            mechanic: None,
        }),
        Err(ChoiceError::MoveDisabled {
            by: VolatileConditionKind::Uproar,
            ..
        })
    ));

    // A miss leaves it going.
    arena.edit(|state| state.pokemon_mut(B).unwrap().stages.evasion = 1);
    arena.turn(0, 0, miss());
    assert_eq!(arena.uproar_turns(), Some(1));
    arena.edit(|state| state.pokemon_mut(B).unwrap().stages.evasion = 0);
    arena.turn(0, 0, hit(90));
    assert_eq!(arena.user().moves[0].pp, pp, "a locked turn spends no PP");
    assert_eq!(arena.uproar_turns(), None);
    assert!(arena.validate(ActionChoice::Switch { pokemon: C }).is_ok());
}

#[test]
fn gen3_locked_turns_that_miss_spend_pp_and_gen4_ones_do_not() {
    // issues.md I65: pokeemerald's `BattleScript_PrintMoveMissed` runs `ppreduce`.
    for (generation, spent) in [(Generation::Gen3, 1), (Generation::Gen4, 0)] {
        let mut uproarer = arena(generation, &["Uproar", "Thrash"], &["Harden"]);
        let mut tape = hit(90);
        tape.push(duration(5));
        uproarer.turn(0, 0, tape);
        let pp = uproarer.user().moves[0].pp;
        uproarer.edit(|state| state.pokemon_mut(B).unwrap().stages.evasion = 1);
        uproarer.turn(0, 0, miss());
        assert_eq!(
            uproarer.user().moves[0].pp,
            pp - spent,
            "{generation:?} Uproar"
        );
        // A hit on a locked turn spends nothing.
        uproarer.edit(|state| state.pokemon_mut(B).unwrap().stages.evasion = 0);
        uproarer.turn(0, 0, hit(90));
        assert_eq!(
            uproarer.user().moves[0].pp,
            pp - spent,
            "{generation:?} Uproar"
        );

        let mut thrasher = arena(generation, &["Uproar", "Thrash"], &["Harden"]);
        let mut tape = hit(90);
        tape.push(duration(3));
        thrasher.turn(1, 0, tape);
        let pp = thrasher.user().moves[1].pp;
        thrasher.edit(|state| state.pokemon_mut(B).unwrap().stages.evasion = 1);
        thrasher.turn(1, 0, miss());
        assert_eq!(
            thrasher.user().moves[1].pp,
            pp - spent,
            "{generation:?} Thrash"
        );
    }
}

#[test]
fn gen3_freeze_ends_an_uproar_the_moment_it_lands() {
    // pokeemerald `SetMoveEffect`'s `CancelMultiTurnMoves`: frozen, the holder loses the
    // uproar at once (before the turn's end); Gen 4 keeps it.
    for (generation, ends) in [(Generation::Gen3, true), (Generation::Gen4, false)] {
        let mut arena = arena(generation, &["Uproar"], &["Ice Beam", "Harden"]);
        let mut tape = hit(90);
        tape.push(duration(5));
        arena.turn(0, 1, tape);
        // The faster foe's Ice Beam freezes the holder, which then draws no thaw. The turn runs
        // on a battle restored for its tape: its trace is that turn's (DESIGN.md §47.0 item 6).
        let mut tape = hit(90);
        tape.push(RandomOverride::bool(RandomRequestKind::Secondary, true));
        tape.push(RandomOverride::bool(RandomRequestKind::Thaw, false));
        arena.turn(0, 0, tape);
        let events = arena.engine.trace().events();
        let frozen = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    TraceEvent::StatusApplied {
                        target: A,
                        status: NonVolatileStatus::Freeze,
                        ..
                    }
                )
            })
            .expect("frozen");
        let ended = events.iter().position(|event| {
            matches!(
                event,
                TraceEvent::VolatileEnded {
                    target: A,
                    kind: VolatileConditionKind::Uproar,
                    ..
                }
            )
        });
        if ends {
            assert_eq!(ended, Some(frozen + 1), "{generation:?}");
        } else {
            assert_eq!(ended, None, "{generation:?}");
            assert!(arena.uproar_turns().is_some(), "{generation:?}");
        }
    }
}
