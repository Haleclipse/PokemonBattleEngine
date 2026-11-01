//! L1 tests for the L.1 batch (DESIGN.md §42, R-L.1): what the replay cannot see. The
//! `sleepUsable` flag, where Quash sends an action, Mycelium Might's fractional priority and
//! its status-only `ignoreAbility`, and which moves a sleeping Pokémon's BeforeMove lets
//! through — the replay forces the `cant|p|slp` and the move's outcome from the log, so a
//! wrong flag there only shows as a planning mismatch.

use std::cell::RefCell;
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, QuashOrder, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleState, NonVolatileStatus, PokemonState, SideState, StatusState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);
const D: PokemonId = PokemonId(4);

fn set(species: &str, moves: &[&str], ability: &str) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    // An empty name leaves the set without an ability, as Gen 1-2 have none.
    if !ability.is_empty() {
        set.ability = Some(ability.into());
    }
    set.level = 50;
    set
}

fn rules_for(generation: Generation) -> ResolvedRules {
    RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom")).unwrap()
}

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

/// P1 = [A, C], P2 = [B, D]; A and B lead.
fn setup(
    generation: Generation,
    p1: PokemonSet,
    p2: PokemonSet,
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(rules_for(generation));
    let dex = Arc::new(DexResolver::resolve(generation));
    // Gen 1 has no abilities and no Blissey: a bench the tests never use.
    let (bench, bench_ability) = if generation == Generation::Gen1 {
        ("Chansey", "")
    } else if generation == Generation::Gen2 {
        ("Blissey", "")
    } else {
        ("Blissey", "Natural Cure")
    };
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &set(bench, &["Harden"], bench_ability)).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &set(bench, &["Harden"], bench_ability)).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side([A, C]), side([B, D])], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

/// Edits the battle state through the snapshot / restore seam (a record the engine has no
/// public setter for).
fn edit_state(arena: &mut Arena, edit: impl FnOnce(&mut BattleState)) {
    let mut snapshot = arena.engine.snapshot();
    edit(&mut snapshot.state);
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
}

impl Arena {
    fn choice(&self, side: SideId, slot: u8) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
        let move_id = self.engine.state().pokemon(actor).unwrap().moves[usize::from(slot)].move_id;
        let target = match self.dex.move_by_id(move_id).unwrap().target {
            pokemon_battle_engine::rules::TargetPattern::SelectedAny
            | pokemon_battle_engine::rules::TargetPattern::SelectedAdjacent
            | pokemon_battle_engine::rules::TargetPattern::SelectedFoe
            | pokemon_battle_engine::rules::TargetPattern::SelectedAdjacentFoe => Some(
                DeclaredTarget(self.rules.topology().position(side.other(), 0).unwrap()),
            ),
            _ => None,
        };
        SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        }
    }

    /// One turn; both sides' moves are named by slot.
    fn turn(&mut self, p1: u8, p2: u8) -> usize {
        let start = self.engine.trace().events().len();
        self.engine
            .submit_turn_choices([self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)])
            .unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        start
    }

    fn events(&self, start: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[start..]
    }
}

/// `sleepUsable` is a move flag and nothing else: Snore and Sleep Talk have it from the
/// generation that introduced them (Snore and Sleep Talk Gen 2; Gen 9's Toedscruel keeps
/// Snore's), a plain move never does.
#[test]
fn sleep_usable_is_a_move_flag_per_generation() {
    for (generation, snore, sleep_talk) in [
        (Generation::Gen1, false, false),
        (Generation::Gen2, true, true),
        (Generation::Gen3, true, true),
        (Generation::Gen9, true, true),
    ] {
        let dex = DexResolver::resolve(generation);
        let flag = |id: MoveId| dex.move_by_id(id).map(|m| m.flags.sleep_usable);
        assert_eq!(flag(MoveId::SNORE), snore.then_some(true), "{generation:?}");
        assert_eq!(
            flag(MoveId::SLEEP_TALK),
            sleep_talk.then_some(true),
            "{generation:?}"
        );
        assert_eq!(flag(MoveId::TACKLE), Some(false), "{generation:?}");
        assert_eq!(flag(MoveId::REST), Some(false), "{generation:?}");
    }
    // Snore fails its `onTry` unless the user is asleep (Sleep Talk's own joins it with the
    // nested-move primitive, L.2).
    let dex = DexResolver::resolve(Generation::Gen9);
    let on_try = |id: MoveId| dex.move_by_id(id).unwrap().behavior.try_use.is_some();
    assert!(on_try(MoveId::SNORE));
    assert!(!on_try(MoveId::TACKLE));
}

/// Quash's order: Gen 5-7 move the target's action to the end of the queue, Gen 8+ put it in
/// the bracket after every move (Showdown's `action.order = 201`, re-sorted per action).
#[test]
fn quash_order_follows_the_generation() {
    let order = |generation| rules_for(generation).action_order().quash;
    assert_eq!(order(Generation::Gen5), QuashOrder::End);
    assert_eq!(order(Generation::Gen6), QuashOrder::End);
    assert_eq!(order(Generation::Gen7), QuashOrder::End);
    assert_eq!(order(Generation::Gen8), QuashOrder::LastBracket(-1));
    assert_eq!(order(Generation::Gen9), QuashOrder::LastBracket(-1));
}

/// Mycelium Might: a status move goes last in its bracket and ignores the breakable abilities
/// of the Pokémon it meets; a damaging move is left alone.
#[test]
fn mycelium_might_delays_status_moves_and_ignores_abilities_there() {
    let arena = setup(
        Generation::Gen9,
        set("Toedscruel", &["Spore", "Mud Shot"], "Mycelium Might"),
        set("Bronzong", &["Harden"], "Levitate"),
        |_| {},
    );
    let pokemon = arena.engine.state().pokemon(A).unwrap();
    let handlers = pokemon_battle_engine::engine::ability_handlers(
        arena.engine.state(),
        &arena.rules,
        pokemon,
    );
    assert_eq!(
        handlers.fractional_priority,
        Some(pokemon_battle_engine::effects::FractionalPriority::DelayStatus)
    );
    let modified = |move_id: MoveId| {
        let def = arena.dex.move_by_id(move_id).unwrap();
        pokemon_battle_engine::engine::modified_move(
            arena.engine.state(),
            &arena.rules,
            A,
            def,
            false,
        )
        .0
    };
    assert!(modified(MoveId::SPORE).ignores_ability);
    assert!(!modified(MoveId::MUD_SHOT).ignores_ability);
}

/// P1 = [A, C], P2 = [B, D] with two active each (After You and Quash only do anything here).
fn setup_doubles(generation: Generation, sets: [PokemonSet; 4]) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let ids = [
        (A, SideId::P1),
        (C, SideId::P1),
        (B, SideId::P2),
        (D, SideId::P2),
    ];
    let pokemon = ids
        .iter()
        .zip(&sets)
        .map(|((id, side), set)| build_pokemon(&dex, *id, *side, set).unwrap())
        .collect();
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let state = BattleState::new(pokemon, [side([A, C]), side([B, D])], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

impl Arena {
    /// A move for `actor` at the slot named, aimed at `target`'s own position.
    fn move_choice(&self, actor: PokemonId, slot: u8, target: Option<PokemonId>) -> SlotChoice {
        let target = target
            .and_then(|target| {
                self.engine
                    .state()
                    .active_position(target, self.rules.topology())
            })
            .map(DeclaredTarget);
        SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(slot).unwrap(),
                target,
                mechanic: None,
            },
        }
    }
}

/// After You needs the target's action still to come: a target that moved already makes it
/// fail, and a target that has not has its action moved to the front.
#[test]
fn after_you_needs_a_move_still_to_come() {
    let sets = || {
        [
            set("Clefable", &["After You", "Moonblast"], "Magic Guard"),
            set("Snorlax", &["Tackle"], "Thick Fat"),
            set("Jolteon", &["Thunder Shock"], "Volt Absorb"),
            set("Blissey", &["Tackle"], "Natural Cure"),
        ]
    };
    // Jolteon (faster) has moved by the time Clefable's After You runs: it fails.
    let mut arena = setup_doubles(Generation::Gen9, sets());
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![
                arena.move_choice(A, 0, Some(B)),
                arena.move_choice(C, 0, Some(B)),
            ],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![
                arena.move_choice(B, 0, Some(C)),
                arena.move_choice(D, 0, Some(C)),
            ],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveFailed {
            actor,
            reason: pokemon_battle_engine::engine::MoveFailReason::Condition(_),
            ..
        } if *actor == A
    )));
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::QueueChangeRequested { .. }))
    );

    // Snorlax has not moved yet (slower): After You puts its action first.
    let mut arena = setup_doubles(Generation::Gen9, sets());
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![
                arena.move_choice(A, 0, Some(C)),
                arena.move_choice(C, 0, Some(B)),
            ],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![
                arena.move_choice(B, 0, Some(C)),
                arena.move_choice(D, 0, Some(C)),
            ],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::QueueChangeRequested {
            request: pokemon_battle_engine::state::QueueRequest::Prioritize(actor),
        } if *actor == C
    )));
}

/// Mycelium Might keeps a held Quick Claw quiet for a status move: the ordering asks for no
/// Quick Claw chance at all (a damaging move still rolls it).
#[test]
fn mycelium_might_keeps_quick_claw_quiet_for_status_moves() {
    let mut holder = set("Toedscruel", &["Growth", "Mud Shot"], "Mycelium Might");
    holder.item = Some("Quick Claw".into());
    let arena = setup(
        Generation::Gen9,
        holder,
        set("Snorlax", &["Tackle"], "Thick Fat"),
        |_| {},
    );
    type Ask = (PokemonId, pokemon_battle_engine::engine::PriorityChance);
    let asked: RefCell<Vec<Ask>> = RefCell::new(Vec::new());
    let roll = |actor: PokemonId, chance: pokemon_battle_engine::engine::PriorityChance| {
        asked.borrow_mut().push((actor, chance));
        true
    };
    let choices = [arena.choice(SideId::P1, 0), arena.choice(SideId::P2, 0)];
    arena.engine.preview_action_order(&choices, &roll).unwrap();
    assert!(
        !asked
            .borrow()
            .iter()
            .any(|(_, chance)| *chance == pokemon_battle_engine::engine::PriorityChance::QuickClaw),
        "a Mycelium Might status move asked for a Quick Claw roll"
    );

    // A damaging move still rolls it.
    asked.borrow_mut().clear();
    let choices = [arena.choice(SideId::P1, 1), arena.choice(SideId::P2, 0)];
    arena.engine.preview_action_order(&choices, &roll).unwrap();
    assert!(
        asked
            .borrow()
            .iter()
            .any(|(_, chance)| *chance == pokemon_battle_engine::engine::PriorityChance::QuickClaw)
    );
}

/// After You and Quash do nothing in a singles battle with one Pokémon a side: both fail.
#[test]
fn queue_moves_fail_in_singles() {
    for (moves, slot) in [
        (["After You", "Air Slash"], 0u8),
        (["Quash", "Thunder Shock"], 0u8),
    ] {
        let mut arena = setup(
            Generation::Gen9,
            set("Jolteon", &moves, "Volt Absorb"),
            set("Snorlax", &["Tackle"], "Thick Fat"),
            |_| {},
        );
        let start = arena.turn(slot, 0);
        assert!(
            arena.events(start).iter().any(|event| matches!(
                event,
                TraceEvent::MoveFailed {
                    actor,
                    reason: pokemon_battle_engine::engine::MoveFailReason::Condition(_),
                    ..
                } if *actor == A
            )),
            "{moves:?} did not fail in singles"
        );
        assert!(
            !arena
                .events(start)
                .iter()
                .any(|event| matches!(event, TraceEvent::QueueChangeRequested { .. }))
        );
    }
}

/// A sleeping Pokémon's BeforeMove: `cant|p|slp`, then the move — but only a `sleepUsable` one
/// (Snore, Sleep Talk) goes on; anything else is blocked.
#[test]
fn a_sleeping_pokemon_snores_through_its_before_move() {
    let asleep = |pokemon: &mut Vec<PokemonState>| {
        let mut status = StatusState::new(NonVolatileStatus::Sleep);
        status.sleep_turns_remaining = Some(3);
        status.sleep_turns_rolled = Some(3);
        pokemon[0].status = Some(status);
    };
    let mut arena = setup(
        Generation::Gen9,
        set("Snorlax", &["Snore", "Tackle"], "Thick Fat"),
        set("Blissey", &["Harden"], "Natural Cure"),
        asleep,
    );
    let start = arena.turn(0, 0);
    assert!(
        arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::MovesWhileAsleep { actor } if *actor == A))
    );
    assert!(arena.events(start).iter().any(|event| {
        matches!(event, TraceEvent::MoveUsed { actor, move_id, .. } if *actor == A && *move_id == MoveId::SNORE)
    }));

    let mut arena = setup(
        Generation::Gen9,
        set("Snorlax", &["Snore", "Tackle"], "Thick Fat"),
        set("Blissey", &["Harden"], "Natural Cure"),
        asleep,
    );
    let start = arena.turn(1, 0);
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|event| { matches!(event, TraceEvent::MoveUsed { actor, .. } if *actor == A) })
    );
    assert!(arena.events(start).iter().any(|event| {
        matches!(
            event,
            TraceEvent::BeforeMoveResolved {
                actor,
                result: pokemon_battle_engine::engine::CanMoveResult::Blocked(
                    pokemon_battle_engine::engine::BeforeMoveBlockReason::Sleep
                ),
                ..
            } if *actor == A
        )
    }));
}

// --- L.2: the moves that call another (DESIGN.md §42.1) --------------------------------------

/// The candidate tables: Metronome draws among the generation's `metronome` moves (the user's
/// own left out in Gen 2 and Gen 4), Assist among the party's (never the user's own, never a
/// `noassist` move), Sleep Talk among the user's own without `nosleeptalk` or `charge`.
#[test]
fn the_caller_candidate_tables_follow_the_generation() {
    use pokemon_battle_engine::engine::{
        assist_candidates, metronome_candidates, sleep_talk_candidates,
    };
    // Assist left the games after Gen 7 (`isNonstandard: Past`).
    let arena = setup_doubles(
        Generation::Gen7,
        [
            set(
                "Clefable",
                &["Metronome", "Assist", "Psychic"],
                "Magic Guard",
            ),
            set(
                "Snorlax",
                &["Sleep Talk", "Tackle", "Charge", "Solar Beam"],
                "Thick Fat",
            ),
            set("Jolteon", &["Tackle"], "Volt Absorb"),
            set("Blissey", &["Harden"], "Natural Cure"),
        ],
    );
    let dex = arena.dex.clone();
    let metronome = |generation: Generation| {
        let rules = rules_for(generation);
        let dex = DexResolver::resolve(generation);
        let arena = setup_doubles(
            generation,
            [
                set("Clefable", &["Metronome", "Psychic"], "Magic Guard"),
                set("Snorlax", &["Sleep Talk", "Tackle"], "Thick Fat"),
                set("Jolteon", &["Tackle"], "Volt Absorb"),
                set("Blissey", &["Harden"], "Natural Cure"),
            ],
        );
        metronome_candidates(arena.engine.state(), &rules, &dex, A)
    };
    // Gen 9's count: every move with the `metronome` flag in the snapshot — Showdown's 581, the
    // four Unobtainable ones included (R-M8-74).
    assert_eq!(metronome(Generation::Gen9).len(), 581);
    // Gen 4 leaves out the user's own (Metronome and Psychic), Gen 9 does not.
    let gen9 = metronome(Generation::Gen9);
    let gen4 = metronome(Generation::Gen4);
    assert!(gen9.contains(&MoveId::PSYCHIC));
    assert!(!gen4.contains(&MoveId::PSYCHIC));
    assert!(!gen4.contains(&MoveId::METRONOME));
    assert!(gen4.len() < gen9.len());
    // Assist: the party's moves without `noassist` (Assist itself is one), never the user's own.
    let assist = assist_candidates(arena.engine.state(), &arena.rules, &dex, A);
    assert!(assist.contains(&MoveId::TACKLE));
    // Sleep Talk carries `noassist` itself, and the user's own moves never come from Assist.
    assert!(!assist.contains(&MoveId::SLEEP_TALK));
    assert!(!assist.contains(&MoveId::PSYCHIC));
    assert!(!assist.contains(&MoveId::ASSIST));
    // Sleep Talk: the user's own, without `nosleeptalk` or `charge`.
    let snorlax = arena.engine.state().pokemon(C).unwrap();
    let sleep_talk = sleep_talk_candidates(&dex, snorlax);
    // Charge has no `charge` flag of its own (it is not a two-turn move): it stays callable.
    // Solar Beam has one: Sleep Talk cannot draw it.
    assert_eq!(sleep_talk, vec![MoveId::TACKLE, MoveId::CHARGE]);
    // A `charge` move is out even where it carries no `nosleeptalk` (Gen 1-5 Fly): the filter
    // has both halves.
    let gen3 = setup(
        Generation::Gen3,
        set("Snorlax", &["Sleep Talk", "Tackle", "Fly"], "Thick Fat"),
        set("Blissey", &["Harden"], "Natural Cure"),
        |_| {},
    );
    let fly_off = sleep_talk_candidates(
        &DexResolver::resolve(Generation::Gen3),
        gen3.engine.state().pokemon(A).unwrap(),
    );
    assert_eq!(fly_off, vec![MoveId::TACKLE]);
    let gen3 = setup(
        Generation::Gen5,
        set("Snorlax", &["Sleep Talk", "Tackle", "Fly"], "Thick Fat"),
        set("Blissey", &["Harden"], "Natural Cure"),
        |_| {},
    );
    assert_eq!(
        sleep_talk_candidates(
            &DexResolver::resolve(Generation::Gen5),
            gen3.engine.state().pokemon(A).unwrap(),
        ),
        vec![MoveId::TACKLE]
    );
}

/// A move that called another is nobody's last move: Gen 1 keeps the called move (Encore and
/// Disable read it), Gen 2 clears the last move for the callers, Gen 3+ keep the chosen one.
/// The draw is pinned to Psychic, which no generation's table refuses.
#[test]
fn a_called_move_is_not_the_last_move() {
    let watched = |generation: Generation| {
        // Gen 1-2 have no abilities; Blissey is Gen 2+.
        let (other, ability) = if generation == Generation::Gen1 {
            ("Chansey", "")
        } else if generation == Generation::Gen2 {
            ("Blissey", "")
        } else {
            ("Blissey", "Natural Cure")
        };
        let mut arena = setup(
            generation,
            set("Snorlax", &["Metronome", "Psychic"], ability),
            set(other, &["Harden", "Tackle"], ability),
            |_| {},
        );
        // Pin the draw: the place of Psychic in this generation's own candidate table — Swift
        // where the user knows Psychic and the generation leaves its own moves out (Gen 2).
        let candidates = pokemon_battle_engine::engine::metronome_candidates(
            arena.engine.state(),
            &arena.rules,
            &arena.dex,
            A,
        );
        let (index, called) = [MoveId::PSYCHIC, MoveId::SWIFT]
            .into_iter()
            .find_map(|called| {
                candidates
                    .iter()
                    .position(|id| *id == called)
                    .map(|index| (index as u16, called))
            })
            .expect("a plain metronome move");
        let mut snapshot = arena.engine.snapshot();
        snapshot.state.rng = pokemon_battle_engine::state::BattleRng::with_semantic_tape(
            1,
            vec![pokemon_battle_engine::state::RandomOverride::range(
                pokemon_battle_engine::state::RandomRequestKind::Sample,
                index,
            )],
        );
        arena.engine = pokemon_battle_engine::BattleEngine::restore(
            arena.rules.clone(),
            arena.dex.clone(),
            snapshot,
        )
        .unwrap();
        let start = arena.turn(0, 0);
        let used: Vec<MoveId> = arena
            .events(start)
            .iter()
            .filter_map(|event| match event {
                TraceEvent::MoveUsed { actor, move_id, .. } if *actor == A => Some(*move_id),
                _ => None,
            })
            .collect();
        assert_eq!(used, vec![MoveId::METRONOME, called], "{generation:?}");
        (used[1], arena.engine.state().pokemon(A).unwrap().last_move)
    };
    // Gen 9: the chosen move stays the last move.
    let (_, last) = watched(Generation::Gen9);
    assert_eq!(last, Some(MoveId::METRONOME));
    // Gen 1: the called move becomes the last move.
    let (called, last) = watched(Generation::Gen1);
    assert_eq!(last, Some(called));
    // Gen 2: `moveUsed` clears it.
    let (_, last) = watched(Generation::Gen2);
    assert_eq!(last, None);
}

/// An Encore of Metronome keeps on the turn its user is in the air from the Bounce Metronome
/// called (Showdown's `OverrideAction` over the locked move), and the Metronome misses its user
/// there: `Invulnerability` runs on the user too, from Gen 2 (R-M10-50) — no second draw.
#[test]
fn an_encored_metronome_misses_its_user_in_the_air() {
    let mut arena = setup(
        Generation::Gen9,
        set("Snorlax", &["Metronome"], "Thick Fat"),
        set("Blissey", &["Harden"], "Natural Cure"),
        |pokemon| {
            pokemon[0].last_move = Some(MoveId::METRONOME);
            pokemon[0].volatiles.set_encore(MoveId::METRONOME, 3);
        },
    );
    let candidates = pokemon_battle_engine::engine::metronome_candidates(
        arena.engine.state(),
        &arena.rules,
        &arena.dex,
        A,
    );
    let bounce = candidates
        .iter()
        .position(|id| *id == MoveId::BOUNCE)
        .expect("Bounce is a Metronome move") as u16;
    edit_state(&mut arena, |state| {
        state.rng = pokemon_battle_engine::state::BattleRng::with_semantic_tape(
            1,
            vec![pokemon_battle_engine::state::RandomOverride::range(
                pokemon_battle_engine::state::RandomRequestKind::Sample,
                bounce,
            )],
        );
    });
    arena.turn(0, 0);
    assert!(
        arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(pokemon_battle_engine::state::VolatileConditionKind::TwoTurnMove)
    );
    let start = arena.turn(0, 0);
    let used: Vec<MoveId> = arena
        .events(start)
        .iter()
        .filter_map(|event| match event {
            TraceEvent::MoveUsed { actor, move_id, .. } if *actor == A => Some(*move_id),
            _ => None,
        })
        .collect();
    assert_eq!(used, vec![MoveId::METRONOME]);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::SemiInvulnerableDodged {
            actor: A,
            target: A,
            ..
        }
    )));
}

/// Gen 3 Sleep Talk's `cant|p|nopp|<move>`: a draw that lands on a move with no PP left is not
/// used at all. The replay reads the log's `cant nopp`; the trace is the engine's own record.
#[test]
fn gen3_sleep_talk_names_a_depleted_move() {
    let mut arena = setup(
        Generation::Gen3,
        set("Snorlax", &["Sleep Talk", "Tackle"], "Thick Fat"),
        set("Blissey", &["Harden"], "Natural Cure"),
        |pokemon| {
            let mut status = StatusState::new(NonVolatileStatus::Sleep);
            status.sleep_turns_remaining = Some(3);
            status.sleep_turns_rolled = Some(3);
            pokemon[0].status = Some(status);
            // Tackle is the only candidate and has no PP left: the draw lands on it.
            pokemon[0].moves[1].pp = 0;
        },
    );
    let start = arena.turn(0, 0);
    assert!(arena
        .events(start)
        .iter()
        .any(|event| matches!(event, TraceEvent::MovedWithoutPp { actor, move_id } if *actor == A && *move_id == MoveId::TACKLE)));
    assert!(!arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveUsed { move_id, .. } if *move_id == MoveId::TACKLE
    )));
    // Gen 9 uses the move all the same.
    let mut arena = setup(
        Generation::Gen9,
        set("Snorlax", &["Sleep Talk", "Tackle"], "Thick Fat"),
        set("Blissey", &["Harden"], "Natural Cure"),
        |pokemon| {
            let mut status = StatusState::new(NonVolatileStatus::Sleep);
            status.sleep_turns_remaining = Some(3);
            status.sleep_turns_rolled = Some(3);
            pokemon[0].status = Some(status);
            pokemon[0].moves[1].pp = 0;
        },
    );
    let start = arena.turn(0, 0);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveUsed { move_id, .. } if *move_id == MoveId::TACKLE
    )));
}

/// Mirror Move's refusal tables per generation (Showdown's `noMirror` lists) and its sources:
/// Gen 2 refuses the calling moves on the foe's last move, Gen 3-4 refuse their tables on the
/// last move that attacked the user, Gen 5+ take the target's last move when it carries
/// `mirror`.
#[test]
fn mirror_move_tables_follow_the_generation() {
    use pokemon_battle_engine::engine::mirror_move_from;
    use pokemon_battle_engine::state::AttackRecord;
    let record = |arena: &Arena, move_id: MoveId, category| AttackRecord {
        source: B,
        damage: Some(10),
        move_id,
        position: arena.rules.topology().position(SideId::P2, 0),
        category,
        damaging_hit_only: false,
    };
    let taken = |arena: &Arena| {
        mirror_move_from(arena.engine.state(), &arena.rules, &arena.dex, A, Some(B))
    };
    // The tables grow: Gen 2 has six moves, Gen 4 the longest list.
    let table_len = |generation: Generation| {
        let rules = rules_for(generation);
        rules
            .conditions()
            .calls
            .mirror_tables
            .for_generation(generation)
            .len()
    };
    assert_eq!(table_len(Generation::Gen2), 6);
    assert_eq!(table_len(Generation::Gen3), 18);
    assert!(table_len(Generation::Gen4) > 18);

    // Gen 2: the foe's last move, refused when the table names it (Transform), and never one
    // the user already knows.
    let mut arena = setup(
        Generation::Gen2,
        set("Clefable", &["Mirror Move", "Psychic"], ""),
        set("Snorlax", &["Transform", "Tackle"], ""),
        |_| {},
    );
    edit_state(&mut arena, |state| {
        state.pokemon_mut(B).unwrap().last_move = Some(MoveId::TRANSFORM);
    });
    assert_eq!(taken(&arena), None);
    edit_state(&mut arena, |state| {
        state.pokemon_mut(B).unwrap().last_move = Some(MoveId::TACKLE);
    });
    assert_eq!(taken(&arena), Some((MoveId::TACKLE, Some(B))));
    // Gen 2 refuses the move when the user knows it too.
    edit_state(&mut arena, |state| {
        state.pokemon_mut(A).unwrap().moves[1].move_id = MoveId::TACKLE;
    });
    assert_eq!(taken(&arena), None);

    // Gen 4: the last move that attacked the user, refused when the table names it (Chatter).
    let mut arena = setup(
        Generation::Gen4,
        set("Clefable", &["Mirror Move", "Psychic"], "Magic Guard"),
        set("Snorlax", &["Chatter", "Tackle"], "Thick Fat"),
        |_| {},
    );
    let chatter = record(
        &arena,
        MoveId::CHATTER,
        pokemon_battle_engine::dex::MoveCategory::Special,
    );
    edit_state(&mut arena, |state| {
        state.pokemon_mut(A).unwrap().history.attacked_by = vec![chatter];
        state.pokemon_mut(B).unwrap().last_move = Some(MoveId::CHATTER);
    });
    assert_eq!(taken(&arena), None);
    let tackled = record(
        &arena,
        MoveId::TACKLE,
        pokemon_battle_engine::dex::MoveCategory::Physical,
    );
    edit_state(&mut arena, |state| {
        state.pokemon_mut(A).unwrap().history.attacked_by = vec![tackled];
        state.pokemon_mut(B).unwrap().last_move = Some(MoveId::TACKLE);
    });
    assert_eq!(taken(&arena), Some((MoveId::TACKLE, Some(B))));
    // An attacker gone since (switched out, knocked out: no last move) leaves nothing to copy
    // (R-M8-80).
    edit_state(&mut arena, |state| {
        state.pokemon_mut(B).unwrap().last_move = None;
    });
    assert_eq!(taken(&arena), None);
    edit_state(&mut arena, |state| {
        state.pokemon_mut(B).unwrap().last_move = Some(MoveId::TACKLE);
    });
    // Its user must still know the move.
    edit_state(&mut arena, |state| {
        state.pokemon_mut(B).unwrap().moves[1].move_id = MoveId::PSYCHIC;
    });
    assert_eq!(taken(&arena), None);

    // Gen 7: the target's last move, which must carry `mirror` (Tackle does, Transform not).
    let mut arena = setup(
        Generation::Gen7,
        set("Clefable", &["Mirror Move", "Psychic"], "Magic Guard"),
        set("Snorlax", &["Transform", "Tackle"], "Thick Fat"),
        |_| {},
    );
    edit_state(&mut arena, |state| {
        state.pokemon_mut(B).unwrap().last_move = Some(MoveId::TRANSFORM);
    });
    assert_eq!(taken(&arena), None);
    edit_state(&mut arena, |state| {
        state.pokemon_mut(B).unwrap().last_move = Some(MoveId::TACKLE);
    });
    assert_eq!(taken(&arena), Some((MoveId::TACKLE, Some(B))));
}

// --- L.3a: Magic Coat and Magic Bounce (DESIGN.md §42.8) ---------------------------------------

/// The coat reflects what the rules allow: a reflectable move from someone else, used again by
/// the coat's holder at the original user. Gen 4 ends the coat as it reflects (the scenarios
/// `gen4-magic-coat` / `gen5-magic-coat` show the second move that turn going through only in
/// Gen 4); Gen 3 and Gen 5+ keep it for the turn.
#[test]
fn magic_coat_reflects_once_and_ends_by_generation() {
    use pokemon_battle_engine::state::VolatileConditionKind;
    for (generation, ends) in [
        (Generation::Gen3, true),
        (Generation::Gen4, true),
        (Generation::Gen5, false),
    ] {
        let mut arena = setup(
            generation,
            set(
                "Clefable",
                &["Magic Coat", "Psychic"],
                if generation == Generation::Gen3 {
                    "Cute Charm"
                } else {
                    "Magic Guard"
                },
            ),
            // Thunder Wave is `reflectable` and was 100 % accurate in Gen 3 (90 % later, and
            // the engine's seed says which).
            set("Snorlax", &["Thunder Wave", "Harden"], "Thick Fat"),
            |_| {},
        );
        assert_eq!(
            arena.rules.conditions().calls.magic_coat_ends_on_reflection,
            ends,
            "{generation:?}"
        );
        // Clefable is faster: it casts the coat, then Snorlax's Thunder Wave is reflected.
        let start = arena.turn(0, 0);
        assert!(
            arena.events(start).iter().any(|event| matches!(
                event,
                TraceEvent::MoveReflected { reflector, .. } if *reflector == A
            )),
            "{generation:?}: the Thunder Wave was not reflected"
        );
        let used: Vec<(MoveId, Option<pokemon_battle_engine::state::CallSource>)> = arena
            .events(start)
            .iter()
            .filter_map(|event| match event {
                TraceEvent::MoveUsed {
                    actor,
                    move_id,
                    called_by,
                    ..
                } if *actor == A => Some((*move_id, *called_by)),
                _ => None,
            })
            .collect();
        assert_eq!(
            used,
            vec![
                (MoveId::MAGIC_COAT, None),
                (
                    MoveId::THUNDER_WAVE,
                    Some(pokemon_battle_engine::state::CallSource::Move(
                        MoveId::MAGIC_COAT
                    ))
                )
            ],
            "{generation:?}"
        );
        // The coat ends with the turn either way; the difference is what the same turn's second
        // move meets, which the scenarios show.
        assert!(
            !arena
                .engine
                .state()
                .pokemon(A)
                .unwrap()
                .volatiles
                .contains(VolatileConditionKind::MagicCoat)
        );
        // A reflected move is not reflected back (`move.hasBounced`): Clefable's own Tackle
        // against a coated Snorlax would come back only if Snorlax had a coat of its own.
    }
    // The coat does not take its user's own move, nor a move without `reflectable`: Harden goes
    // through even as the coat is up.
    let mut arena = setup(
        Generation::Gen5,
        set("Clefable", &["Magic Coat", "Harden"], "Magic Guard"),
        set("Snorlax", &["Harden", "Tackle"], "Thick Fat"),
        |_| {},
    );
    let start = arena.turn(0, 0);
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::MoveReflected { .. }))
    );
}

/// Magic Bounce reflects as an ability, and a move that is not `reflectable` (or one the
/// reflector aimed at itself) is left alone.
#[test]
fn magic_bounce_reflects_and_plain_moves_pass() {
    let mut arena = setup(
        Generation::Gen9,
        set("Espeon", &["Harden", "Psychic"], "Magic Bounce"),
        set("Snorlax", &["Toxic", "Harden"], "Thick Fat"),
        |_| {},
    );
    // A status move with `reflectable` is sent back.
    let start = arena.turn(0, 0);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveReflected { reflector, .. } if *reflector == A
    )));
    assert!(!arena.engine.state().pokemon(B).unwrap().status.is_none());
    // Poisoned instead, which the log's frame shows too.
    assert!(arena.engine.state().pokemon(B).unwrap().status.is_some());
    // A move without `reflectable` (Harden) is not.
    let start = arena.turn(0, 1);
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::MoveReflected { .. })),
        "a non-reflectable move was reflected"
    );
    // Magic Bounce is breakable: a move that ignores abilities (Sunsteel Strike's user is a
    // different Pokémon here, so Mold Breaker stands in) leaves it out — with Gastro Acid on
    // the holder, nothing reflects.
    let mut arena = setup(
        Generation::Gen9,
        set("Espeon", &["Harden"], "Magic Bounce"),
        set("Snorlax", &["Gastro Acid", "Toxic"], "Mold Breaker"),
        |_| {},
    );
    let _ = arena.turn(0, 0);
    assert!(
        arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(pokemon_battle_engine::state::VolatileConditionKind::GastroAcid)
    );
    let start = arena.turn(0, 1);
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::MoveReflected { .. })),
        "a suppressed Magic Bounce reflected a move"
    );
}

/// Two coats in play (`move.hasBounced`): the move the first coat sent back is not sent back
/// again by the second — Showdown marks a reflected move and the reflection reads the mark.
#[test]
fn a_reflected_move_is_not_reflected_again() {
    let mut arena = setup(
        Generation::Gen5,
        set("Clefable", &["Magic Coat", "Harden"], "Magic Guard"),
        set("Snorlax", &["Magic Coat", "Thunder Wave"], "Thick Fat"),
        |_| {},
    );
    // Both wear a coat (raised directly: in singles only one of them could cast it per turn).
    edit_state(&mut arena, |state| {
        state
            .pokemon_mut(A)
            .unwrap()
            .volatiles
            .set_magic_coat(false);
        state
            .pokemon_mut(B)
            .unwrap()
            .volatiles
            .set_magic_coat(false);
    });
    // Clefable hardens; Snorlax's Thunder Wave meets Clefable's coat and comes back at Snorlax,
    // whose own coat does not send it on again.
    let start = arena.turn(1, 1);
    assert_eq!(
        arena
            .events(start)
            .iter()
            .filter(|event| matches!(event, TraceEvent::MoveReflected { .. }))
            .count(),
        1,
        "the reflected Thunder Wave bounced a second time"
    );
    assert!(arena.engine.state().pokemon(B).unwrap().status.is_some());
}

// --- L.3b: Snatch (DESIGN.md §42.8) ------------------------------------------------------------

/// Snatch takes a `snatch`-flagged move over at the PrepareHit step, once per cast: the volatile
/// is gone as it steals, so a second flagged move in the same turn goes through.
#[test]
fn snatch_steals_a_flagged_move_once() {
    use pokemon_battle_engine::state::VolatileConditionKind;
    let mut arena = setup_doubles(
        Generation::Gen5,
        [
            set("Clefable", &["Snatch", "Psychic"], "Magic Guard"),
            set("Blissey", &["Harden"], "Natural Cure"),
            set("Snorlax", &["Swords Dance", "Tackle"], "Thick Fat"),
            set("Ninetales", &["Harden", "Tackle"], "Flash Fire"),
        ],
    );
    // Clefable raises its Snatch; the two foes use flagged moves (Swords Dance, Harden).
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 0, None), arena.move_choice(C, 0, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, None), arena.move_choice(D, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    let snatches = arena
        .events(start)
        .iter()
        .filter(|event| matches!(event, TraceEvent::MoveSnatched { .. }))
        .count();
    assert_eq!(snatches, 1, "Snatch stole more than once in a turn");
    // The stolen move was used by the snatcher, from Snatch.
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveUsed {
            actor,
            called_by: Some(pokemon_battle_engine::state::CallSource::Move(MoveId::SNATCH)),
            ..
        } if *actor == A
    )));
    assert!(
        !arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Snatch)
    );
}

/// A spread move one of whose targets reflects it (Magic Coat) goes on to its other targets —
/// after the reflection, which Showdown runs inside that target's `TryHit` (R-M8-68). The frames
/// show only the stat changes, whichever order they came in.
#[test]
fn a_spread_move_goes_on_after_a_reflection() {
    use pokemon_battle_engine::state::CallSource;
    let mut arena = setup_doubles(
        Generation::Gen5,
        [
            set("Mawile", &["Magic Coat"], "Hyper Cutter"),
            set("Zigzagoon", &["Harden"], "Pickup"),
            set("Absol", &["Leer"], "Pressure"),
            set("Pidgey", &["Harden"], "Keen Eye"),
        ],
    );
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 0, None), arena.move_choice(C, 0, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, None), arena.move_choice(D, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    let events = arena.events(start);
    let reflected = events
        .iter()
        .position(|event| {
            matches!(
                event,
                TraceEvent::MoveUsed {
                    actor,
                    called_by: Some(CallSource::Move(MoveId::MAGIC_COAT)),
                    ..
                } if *actor == A
            )
        })
        .expect("Mawile reflected the Leer");
    let lowered = |pokemon: PokemonId| {
        events.iter().position(|event| {
            matches!(event, TraceEvent::StatChanged { target, to: -1, .. } if *target == pokemon)
        })
    };
    let zigzagoon = lowered(C).expect("the Leer went on to Zigzagoon");
    assert!(
        reflected < zigzagoon,
        "the reflection ran after the Leer's other target"
    );
    assert!(
        lowered(B).is_some() && lowered(D).is_some(),
        "the reflection lowered both foes"
    );
}

/// The `snatch` flag is the move's, and Snatch itself is Gen 3-7 (`isNonstandard: Past` from
/// Gen 8, so the snapshot has no such move in Gen 9).
#[test]
fn the_snatch_flag_and_the_moves_years() {
    for generation in [Generation::Gen3, Generation::Gen5, Generation::Gen7] {
        let dex = DexResolver::resolve(generation);
        assert!(dex.move_by_id(MoveId::SNATCH).is_some(), "{generation:?}");
        let flag = |id: MoveId| dex.move_by_id(id).map(|m| m.flags.snatch);
        assert_eq!(flag(MoveId::HARDEN), Some(true), "{generation:?}");
        assert_eq!(flag(MoveId::SWORDS_DANCE), Some(true), "{generation:?}");
        assert_eq!(flag(MoveId::TACKLE), Some(false), "{generation:?}");
    }
    assert!(
        DexResolver::resolve(Generation::Gen9)
            .move_by_id(MoveId::SNATCH)
            .is_none()
    );
}

// --- L.3c: Me First (DESIGN.md §42.8) ----------------------------------------------------------

/// Me First needs a damaging move still queued for its (faster) target, and the boost's stage
/// follows the generation (Gen 4 at the damage phase, Gen 5+ at the BasePower chain).
#[test]
fn me_first_takes_the_queued_move_and_boosts_by_generation() {
    use pokemon_battle_engine::rules::MeFirstBoost;
    use pokemon_battle_engine::state::{CallSource, VolatileConditionKind};
    for (generation, stage) in [
        (Generation::Gen4, Some(MeFirstBoost::DamagePhase2)),
        (Generation::Gen5, Some(MeFirstBoost::BasePower)),
        (Generation::Gen3, None),
    ] {
        let rules = rules_for(generation);
        assert_eq!(
            rules.conditions().calls.me_first_boost,
            stage,
            "{generation:?}"
        );
    }
    // Gen 9 has no Me First (it left after Gen 7).
    assert!(
        DexResolver::resolve(Generation::Gen9)
            .move_by_id(MoveId::ME_FIRST)
            .is_none()
    );

    // A faster user takes the target's queued Tackle.
    let mut arena = setup(
        Generation::Gen5,
        set("Jolteon", &["Me First", "Harden"], "Volt Absorb"),
        set("Snorlax", &["Tackle", "Harden"], "Thick Fat"),
        |_| {},
    );
    let start = arena.turn(0, 0);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveUsed {
            actor,
            move_id: MoveId::TACKLE,
            called_by: Some(CallSource::Move(MoveId::ME_FIRST)),
            ..
        } if *actor == A
    )));
    // The boost marker is on the user for the turn only (the residual clears it).
    assert!(
        !arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::MeFirst)
    );

    // Gen 4 checks the same table at the damage phase.
    let rules = rules_for(Generation::Gen4);
    assert_eq!(
        rules.conditions().calls.me_first_boost,
        Some(MeFirstBoost::DamagePhase2)
    );
    // A move carrying `failmefirst` (Counter) queued for the target fails it too.
    let mut arena = setup(
        Generation::Gen5,
        set("Jolteon", &["Me First", "Harden"], "Volt Absorb"),
        set("Snorlax", &["Counter", "Tackle"], "Thick Fat"),
        |_| {},
    );
    let start = arena.turn(0, 0);
    assert!(!arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveUsed {
            called_by: Some(CallSource::Move(MoveId::ME_FIRST)),
            ..
        }
    )));
    // A status move queued for the target fails it.
    let mut arena = setup(
        Generation::Gen5,
        set("Jolteon", &["Me First", "Harden"], "Volt Absorb"),
        set("Snorlax", &["Harden", "Tackle"], "Thick Fat"),
        |_| {},
    );
    let start = arena.turn(0, 0);
    assert!(!arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveUsed {
            called_by: Some(CallSource::Move(MoveId::ME_FIRST)),
            ..
        }
    )));
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveFailed {
            actor,
            reason: pokemon_battle_engine::engine::MoveFailReason::Condition(_),
            ..
        } if *actor == A
    )));
}

// --- L.5: Mimic and Sketch (DESIGN.md §42.5) ---------------------------------------------------

/// Mimic writes an overlay over the user's own slots — thrown away on switch-out — while Sketch
/// writes the own slots for good.
#[test]
fn mimic_writes_an_overlay_and_sketch_the_slots() {
    let mut arena = setup(
        Generation::Gen9,
        set("Clefable", &["Mimic", "Psychic"], "Magic Guard"),
        set("Snorlax", &["Tackle", "Harden"], "Thick Fat"),
        |_| {},
    );
    let _ = arena.turn(1, 0);
    let start = arena.turn(0, 1);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveSlotsRewritten {
            pokemon,
            slot: 0,
            move_id: MoveId::TACKLE,
            by: MoveId::MIMIC,
            ..
        } if *pokemon == A
    )));
    let clefable = arena.engine.state().pokemon(A).unwrap();
    assert_eq!(clefable.move_slots()[0].move_id, MoveId::TACKLE);
    assert_eq!(
        clefable.moves[0].move_id,
        MoveId::MIMIC,
        "the own slot changed"
    );
    assert!(clefable.move_overlay().is_some());
    // Leaving the field forgets the copied move: out, and back in.
    let switch = |arena: &mut Arena, to: PokemonId| {
        let choices = [
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: arena.engine.state().side(SideId::P1).active[0].unwrap(),
                    action: ActionChoice::Switch { pokemon: to },
                }],
            },
            arena.choice(SideId::P2, 1),
        ];
        arena.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    };
    switch(&mut arena, C);
    switch(&mut arena, A);
    let clefable = arena.engine.state().pokemon(A).unwrap();
    assert!(clefable.move_overlay().is_none());
    assert_eq!(clefable.move_slots()[0].move_id, MoveId::MIMIC);
}

/// Gen 1 writes a move the user already knows — `this.sample(target.moves)` takes the draw
/// without a look — while Gen 2+ refuse it (`source.moves.includes(move.id)`).
#[test]
fn gen1_mimic_may_copy_a_move_it_already_knows() {
    let wrote = |generation| {
        let mut arena = setup(
            generation,
            set("Clefable", &["Mimic", "Tackle"], ""),
            set("Snorlax", &["Tackle"], ""),
            |_| {},
        );
        let _ = arena.turn(1, 0);
        let start = arena.turn(0, 0);
        arena.events(start).iter().any(|event| {
            matches!(
                event,
                TraceEvent::MoveSlotsRewritten {
                    pokemon,
                    slot: 0,
                    move_id: MoveId::TACKLE,
                    by: MoveId::MIMIC,
                    ..
                } if *pokemon == A
            )
        })
    };
    assert!(
        wrote(Generation::Gen1),
        "the draw landed on the known Tackle"
    );
    assert!(!wrote(Generation::Gen2), "Gen 2 refuses a known move");
}

/// The copying rules per generation: Gen 1 draws from the target's slots keeping the slot's PP,
/// Gen 2-4 cap the copied PP at 5, Gen 5+ copy it whole; Sketch always fails in Gen 2; both fail
/// when the user knows the move already.
#[test]
fn mimic_and_sketch_rules_follow_the_generation() {
    let rules = |generation| rules_for(generation).conditions().calls;
    assert!(rules(Generation::Gen1).mimic_draws_from_the_slots);
    assert!(rules(Generation::Gen1).mimic_keeps_the_slot_pp);
    assert!(!rules(Generation::Gen1).mimic_refuses_a_known_move);
    assert!(rules(Generation::Gen2).mimic_refuses_a_known_move);
    assert!(!rules(Generation::Gen2).mimic_draws_from_the_slots);
    assert!(rules(Generation::Gen2).mimic_pp_capped);
    assert!(rules(Generation::Gen4).mimic_pp_capped);
    assert!(!rules(Generation::Gen5).mimic_pp_capped);
    assert!(rules(Generation::Gen2).mimic_fails_behind_substitute);
    assert!(!rules(Generation::Gen5).mimic_fails_behind_substitute);
    assert!(rules(Generation::Gen2).sketch_always_fails);
    assert!(!rules(Generation::Gen9).sketch_always_fails);

    // Gen 2: Sketch does nothing at all.
    let mut arena = setup(
        Generation::Gen2,
        set("Smeargle", &["Sketch", "Harden"], ""),
        set("Snorlax", &["Tackle"], ""),
        |_| {},
    );
    let _ = arena.turn(1, 0);
    let start = arena.turn(0, 0);
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::MoveSlotsRewritten { .. }))
    );
    // Mimic fails when the user already knows the move (Psychic here).
    let mut arena = setup(
        Generation::Gen9,
        set("Clefable", &["Mimic", "Psychic"], "Magic Guard"),
        set("Snorlax", &["Psychic", "Tackle"], "Thick Fat"),
        |_| {},
    );
    let _ = arena.turn(1, 0);
    let start = arena.turn(0, 0);
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::MoveSlotsRewritten { .. }))
    );
}

/// Gen 1 Mimic draws among the target's slots (`this.sample(target.moves)`), not its last move:
/// with the draw pinned to the first slot, the copy is the move there even though the target
/// used another one last.
#[test]
fn gen1_mimic_draws_from_the_slots() {
    let mut arena = setup(
        Generation::Gen1,
        set("Clefable", &["Mimic", "Harden"], ""),
        set("Snorlax", &["Tackle", "Harden"], ""),
        |_| {},
    );
    // Both harden (no accuracy roll); Clefable's Mimic then draws slot 0 (Tackle).
    let _ = arena.turn(1, 1);
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.rng = pokemon_battle_engine::state::BattleRng::with_semantic_tape(
        1,
        vec![
            // Gen 1's moves roll accuracy first (Harden hits), then Mimic draws slot 0.
            pokemon_battle_engine::state::RandomOverride::bool(
                pokemon_battle_engine::state::RandomRequestKind::Accuracy,
                true,
            ),
            pokemon_battle_engine::state::RandomOverride::range(
                pokemon_battle_engine::state::RandomRequestKind::Sample,
                0,
            ),
        ],
    );
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let start = arena.turn(0, 0);
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveSlotsRewritten {
            pokemon,
            slot: 0,
            move_id: MoveId::TACKLE,
            by: MoveId::MIMIC,
            ..
        } if *pokemon == A
    )));
}

/// Gen 2+ (`if (source.transformed …) return false`; pokeemerald `Cmd_mimicattackcopy` checks
/// `STATUS2_TRANSFORMED`): a transformed Pokémon's Mimic fails even at a move it does not know.
/// No replay shows it — a transformed Pokémon knows its target's moves, and from Gen 2 Mimic
/// refuses a known one — so the target's last move is set here (R-M8-75). Gen 1's goes ahead
/// (`gen1-transformed-mimic-copies-a-move`).
#[test]
fn a_transformed_mimic_fails_from_gen_2() {
    let mut arena = setup(
        Generation::Gen3,
        set("Mew", &["Transform", "Splash"], "Synchronize"),
        set("Snorlax", &["Mimic", "Counter"], "Thick Fat"),
        |_| {},
    );
    // Mew transforms; Snorlax's Counter (priority −5, with nothing to counter) comes after it
    // on both turns.
    let _ = arena.turn(0, 1);
    edit_state(&mut arena, |state| {
        state.pokemon_mut(B).unwrap().last_move = Some(MoveId::SURF);
    });
    // Mew's slot 0 is now Snorlax's Mimic, aimed at a last move it does not know.
    let start = arena.turn(0, 1);
    assert!(
        !arena.events(start).iter().any(|event| matches!(
            event,
            TraceEvent::MoveSlotsRewritten { pokemon, .. } if *pokemon == A
        )),
        "a transformed Pokémon's Gen 3 Mimic wrote a slot"
    );
}

// --- L.4a: Ally Switch (DESIGN.md §42.4) -------------------------------------------------------

/// Ally Switch trades the two partners' slots and fails in singles; Gen 9's consecutive-use
/// chance is the only generation with one.
#[test]
fn ally_switch_swaps_and_fails_in_singles() {
    let mut arena = setup_doubles(
        Generation::Gen7,
        [
            set("Clefable", &["Ally Switch", "Harden"], "Magic Guard"),
            set("Blissey", &["Ally Switch", "Harden"], "Natural Cure"),
            set("Snorlax", &["Tackle"], "Thick Fat"),
            set("Ninetales", &["Ember"], "Flash Fire"),
        ],
    );
    assert_eq!(arena.engine.state().side(SideId::P1).active[0], Some(A));
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 0, None), arena.move_choice(C, 1, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![
                arena.move_choice(B, 0, Some(A)),
                arena.move_choice(D, 0, Some(C)),
            ],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(
        arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::PositionsSwapped { side: SideId::P1 }))
    );
    assert_eq!(arena.engine.state().side(SideId::P1).active[0], Some(C));
    assert_eq!(arena.engine.state().side(SideId::P1).active[1], Some(A));

    // Singles: nothing to switch with.
    let mut arena = setup(
        Generation::Gen7,
        set("Clefable", &["Ally Switch", "Harden"], "Magic Guard"),
        set("Snorlax", &["Harden"], "Thick Fat"),
        |_| {},
    );
    let start = arena.turn(0, 0);
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::PositionsSwapped { .. }))
    );
    // The chance exists in Gen 9 only.
    assert!(
        !rules_for(Generation::Gen7)
            .conditions()
            .calls
            .ally_switch_consecutive_chance
    );
    assert!(
        rules_for(Generation::Gen9)
            .conditions()
            .calls
            .ally_switch_consecutive_chance
    );
}

/// The `allyswitch` counter (Showdown `onStart` 3, `onRestart` ×3 at most 729): the roll's
/// denominator, which the replay reads only as "a counter stands" — the test pins the values.
#[test]
fn the_ally_switch_counter_grows_by_three() {
    use pokemon_battle_engine::state::VolatileState;
    let mut volatiles = VolatileState::default();
    assert!(volatiles.ally_switch_counter().is_none());
    assert_eq!(volatiles.bump_ally_switch(2), 3);
    assert_eq!(volatiles.bump_ally_switch(2), 9);
    assert_eq!(volatiles.bump_ally_switch(2), 27);
    for _ in 0..8 {
        volatiles.bump_ally_switch(2);
    }
    assert_eq!(volatiles.ally_switch_counter(), Some(729));
    assert!(volatiles.remove_ally_switch());
    assert!(volatiles.ally_switch_counter().is_none());
}

// --- L.4b: the Pledges (DESIGN.md §42.9) -------------------------------------------------------

/// The combo table: the pair decides the type and the side condition (Water on the user's side;
/// Fire and Grass on the target's), and equal or unrelated moves do not combine.
#[test]
fn the_pledge_combo_table() {
    use pokemon_battle_engine::dex::PokemonType;
    use pokemon_battle_engine::engine::pledges::{PledgeSide, combo};
    use pokemon_battle_engine::state::SideConditionKind as S;
    let fire = MoveId::FIRE_PLEDGE;
    let water = MoveId::WATER_PLEDGE;
    let grass = MoveId::GRASS_PLEDGE;
    for (by, combined) in [(fire, water), (water, fire)] {
        let combo = combo(by, combined).expect("fire + water");
        assert_eq!(combo.move_type, PokemonType::Water);
        assert_eq!(combo.condition, S::WaterPledge);
        assert_eq!(combo.side, PledgeSide::User);
    }
    for (by, combined) in [(fire, grass), (grass, fire)] {
        let combo = combo(by, combined).expect("fire + grass");
        assert_eq!(combo.move_type, PokemonType::Fire);
        assert_eq!(combo.condition, S::FirePledge);
        assert_eq!(combo.side, PledgeSide::Target);
    }
    for (by, combined) in [(water, grass), (grass, water)] {
        let combo = combo(by, combined).expect("water + grass");
        assert_eq!(combo.move_type, PokemonType::Grass);
        assert_eq!(combo.condition, S::GrassPledge);
        assert_eq!(combo.side, PledgeSide::Target);
    }
    assert!(combo(fire, fire).is_none());
    assert!(combo(fire, MoveId::TACKLE).is_none());
}

/// The combined Pledges' side conditions: Fire chips the side's non-Fire types, Grass quarters
/// their Speed, Water doubles the side's secondary chances (a Serene Grace flinch is left
/// alone).
#[test]
fn the_pledge_side_conditions() {
    use pokemon_battle_engine::state::SideConditionKind as S;
    let mut arena = setup(
        Generation::Gen9,
        set("Clefable", &["Harden", "Ice Beam"], "Magic Guard"),
        set("Snorlax", &["Harden", "Tackle"], "Thick Fat"),
        |_| {},
    );
    let speed_before =
        pokemon_battle_engine::engine::effective_speed(arena.engine.state(), &arena.rules, A);
    edit_state(&mut arena, |state| {
        state
            .side_mut(SideId::P1)
            .conditions
            .set_timed(S::GrassPledge, 4);
        state
            .side_mut(SideId::P2)
            .conditions
            .set_timed(S::FirePledge, 4);
    });
    let speed_after =
        pokemon_battle_engine::engine::effective_speed(arena.engine.state(), &arena.rules, A);
    assert!(
        speed_after * 4 <= speed_before,
        "Grass Pledge did not quarter the Speed ({speed_before} → {speed_after})"
    );
    // Water Pledge doubles the side's secondary chances.
    let ice_beam = arena.dex.move_by_id(MoveId::ICE_BEAM).cloned().unwrap();
    let chance = |arena: &Arena| {
        pokemon_battle_engine::engine::modified_move(
            arena.engine.state(),
            &arena.rules,
            A,
            &ice_beam,
            false,
        )
        .0
        .secondaries[0]
            .chance
            .basis_points()
    };
    assert_eq!(chance(&arena), 1_000);
    edit_state(&mut arena, |state| {
        state
            .side_mut(SideId::P1)
            .conditions
            .set_timed(S::WaterPledge, 4);
    });
    assert_eq!(
        chance(&arena),
        2_000,
        "Water Pledge did not double the secondary"
    );
    // A turn under Fire Pledge (P2's side) chips Snorlax (Normal) by an eighth.
    let hp_before = arena.engine.state().pokemon(B).unwrap().hp;
    let max_hp = arena.engine.state().pokemon(B).unwrap().max_hp;
    let _ = arena.turn(0, 0);
    let hp_after = arena.engine.state().pokemon(B).unwrap().hp;
    let chip = hp_before.saturating_sub(hp_after);
    assert!(chip >= max_hp / 8, "Fire Pledge chipped {chip} of {max_hp}");
    // A Fire type on that side is spared.
    let mut arena = setup(
        Generation::Gen9,
        set("Clefable", &["Harden"], "Magic Guard"),
        set("Ninetales", &["Harden"], "Flash Fire"),
        |_| {},
    );
    edit_state(&mut arena, |state| {
        state
            .side_mut(SideId::P2)
            .conditions
            .set_timed(S::FirePledge, 4);
    });
    let full = arena.engine.state().pokemon(B).unwrap().max_hp;
    let _ = arena.turn(0, 0);
    assert_eq!(
        arena.engine.state().pokemon(B).unwrap().hp,
        full,
        "Fire Pledge chipped a Fire type"
    );
}

// --- L.4c: Dancer (DESIGN.md §42.9) ------------------------------------------------------------

/// Dancer dances along with another Pokémon's dance, from the ability, and a dancer whose
/// BeforeMove blocks the action (asleep) does not — while a Pokémon without the ability never
/// joins.
#[test]
fn dancer_dances_along_after_a_dance() {
    use pokemon_battle_engine::state::CallSource;
    let mut arena = setup_doubles(
        Generation::Gen9,
        [
            set("Oricorio", &["Quiver Dance", "Harden"], "Dancer"),
            set("Snorlax", &["Harden"], "Thick Fat"),
            set("Volcarona", &["Quiver Dance", "Harden"], "Flame Body"),
            set("Blissey", &["Harden"], "Natural Cure"),
        ],
    );
    let start = arena.engine.trace().events().len();
    // The Volcarona (P2's lead) dances; Oricorio (P1's lead) dances along.
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 1, None), arena.move_choice(C, 0, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, None), arena.move_choice(D, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::DancerActivated { dancer, move_id } if *dancer == A && *move_id == MoveId::QUIVER_DANCE
    )));
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveUsed {
            actor,
            move_id: MoveId::QUIVER_DANCE,
            called_by: Some(CallSource::Ability(pokemon_battle_engine::ids::AbilityId::DANCER)),
            ..
        } if *actor == A
    )));
    // It got the boosts the dance gives.
    assert_eq!(
        arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .stages
            .special_attack,
        1
    );
    // Snorlax (no Dancer) did not.
    assert_eq!(
        arena
            .engine
            .state()
            .pokemon(C)
            .unwrap()
            .stages
            .special_attack,
        0
    );

    // Two dancers dance in order of stored Speed, slowest first: a level-30 Oricorio (slower)
    // goes before the level-50 one.
    let mut arena = setup_doubles(
        Generation::Gen9,
        [
            set("Oricorio", &["Harden"], "Dancer"),
            set("Oricorio", &["Harden"], "Dancer"),
            set("Volcarona", &["Quiver Dance", "Harden"], "Flame Body"),
            set("Blissey", &["Harden"], "Natural Cure"),
        ],
    );
    edit_state(&mut arena, |state| {
        let slowest = state.pokemon(C).unwrap().id();
        let pokemon = state.pokemon_mut(slowest).unwrap();
        pokemon.stats.speed /= 4;
    });
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 0, None), arena.move_choice(C, 0, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, None), arena.move_choice(D, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    let order: Vec<PokemonId> = arena
        .events(start)
        .iter()
        .filter_map(|event| match event {
            TraceEvent::DancerActivated { dancer, .. } => Some(*dancer),
            _ => None,
        })
        .collect();
    assert_eq!(order, vec![C, A], "the dancers went in the wrong order");

    // A sleeping dancer: the dance-along stops at BeforeMove.
    let mut arena = setup(
        Generation::Gen9,
        set("Oricorio", &["Quiver Dance", "Harden"], "Dancer"),
        set("Volcarona", &["Quiver Dance", "Harden"], "Flame Body"),
        |pokemon| {
            let mut status = StatusState::new(NonVolatileStatus::Sleep);
            status.sleep_turns_remaining = Some(3);
            status.sleep_turns_rolled = Some(3);
            pokemon[0].status = Some(status);
        },
    );
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 1, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::DancerActivated { dancer, .. } if *dancer == A
    )));
    assert!(!arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::MoveUsed { actor, move_id: MoveId::QUIVER_DANCE, .. } if *actor == A
    )));
    assert_eq!(
        arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .stages
            .special_attack,
        0
    );
}

/// Showdown's `noLock` in `runMove` (the "Dancer Petal Dance hack", O.5b): the dance a Dancer
/// copies deletes the `lockedmove` volatile it just set — the dancer is not locked into the
/// rampage — while a dancer that already carried its own lock keeps it.
/// A dance Snatch takes over dances no one along: the user's own move failed (`useMove`
/// returns `false`, `moveDidSomething`), and the snatcher's is a `useMove` of its own, outside
/// `runMove` (a Gen 7 Oricorio beside a snatched Swords Dance, R-M10-69).
#[test]
fn a_snatched_dance_dances_no_one_along() {
    let mut arena = setup_doubles(
        Generation::Gen7,
        [
            set("Oricorio", &["Splash"], "Dancer"),
            set("Snorlax", &["Swords Dance"], "Thick Fat"),
            set("Volcarona", &["Snatch"], "Flame Body"),
            set("Blissey", &["Splash"], "Natural Cure"),
        ],
    );
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 0, None), arena.move_choice(C, 0, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, None), arena.move_choice(D, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    let events = arena.events(start);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::MoveSnatched { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, TraceEvent::DancerActivated { .. }))
    );
    let attack = |id| arena.engine.state().pokemon(id).unwrap().stages.attack;
    assert_eq!((attack(A), attack(B), attack(C)), (0, 2, 0));
}

#[test]
fn a_dancer_does_not_keep_a_copied_rampages_lock() {
    use pokemon_battle_engine::state::VolatileConditionKind;
    let mut arena = setup(
        Generation::Gen9,
        set("Oricorio", &["Petal Dance", "Harden"], "Dancer"),
        set("Lilligant", &["Petal Dance", "Harden"], "Chlorophyll"),
        |_| {},
    );
    // Oricorio (the faster one) Harden s; Lilligant's Petal Dance is copied by the Dancer.
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 1, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, Some(A))],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    let rampage = |arena: &Arena, id| {
        arena
            .engine
            .state()
            .pokemon(id)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Rampage)
    };
    assert!(!rampage(&arena, A), "the copied rampage left no lock");
    assert!(rampage(&arena, B), "the dance's own user keeps its lock");

    // A dancer already locked in its own rampage keeps it when it dances along.
    let mut arena = setup(
        Generation::Gen9,
        set("Oricorio", &["Petal Dance", "Harden"], "Dancer"),
        set("Lilligant", &["Petal Dance", "Harden"], "Chlorophyll"),
        |_| {},
    );
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 0, Some(B))],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, Some(A))],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(rampage(&arena, A), "its own rampage goes on");
}

/// The dance-along aims where the dance did when the original target is a foe of the dancer's
/// and the dancer's ally used it: the dancer's Revelation Dance reaches the same foe (a random
/// one would be a draw the plan does not have).
#[test]
fn a_dancer_dances_at_the_same_foe() {
    let mut arena = setup_doubles(
        Generation::Gen9,
        [
            set("Snorlax", &["Harden"], "Dancer"),
            set("Volcarona", &["Revelation Dance", "Harden"], "Flame Body"),
            set("Snorlax", &["Harden"], "Thick Fat"),
            set("Blissey", &["Harden"], "Natural Cure"),
        ],
    );
    // Every planned decision is marked optional and of a kind the action does draw: a random
    // target would be a `RandomTarget` draw, a kind the tape has none of — the engine's own
    // request fails the plan assert (`rng.rs`), which is this test's signal.
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.rng = pokemon_battle_engine::state::BattleRng::with_semantic_tape(
        1,
        vec![
            // The order draws its speed ties first (marked optional: the engine may ask for
            // fewer); the plan's damage roll follows.
            pokemon_battle_engine::state::RandomOverride::raw(
                pokemon_battle_engine::state::RandomRequestKind::SpeedTie,
                1,
            )
            .optional(),
            pokemon_battle_engine::state::RandomOverride::raw(
                pokemon_battle_engine::state::RandomRequestKind::SpeedTie,
                1,
            )
            .optional(),
            pokemon_battle_engine::state::RandomOverride::bool(
                pokemon_battle_engine::state::RandomRequestKind::Critical,
                false,
            )
            .optional(),
            pokemon_battle_engine::state::RandomOverride::range(
                pokemon_battle_engine::state::RandomRequestKind::DamageRoll,
                100,
            )
            .optional(),
            // A wrong implementation would draw a random target here (the first foe, Snorlax);
            // the engine that keeps the dance's target never asks.
            pokemon_battle_engine::state::RandomOverride::range(
                pokemon_battle_engine::state::RandomRequestKind::RandomTarget,
                0,
            )
            .optional(),
        ],
    );
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let start = arena.engine.trace().events().len();
    // A (Dancer Snorlax) hardens; C (Volcarona) dances at the foe's second slot (Blissey).
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![
                arena.move_choice(A, 0, None),
                arena.move_choice(C, 0, Some(D)),
            ],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.move_choice(B, 0, None), arena.move_choice(D, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    // The dancer's own Revelation Dance (from the ability) reached Blissey.
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::DamageApplied {
            target,
            source: pokemon_battle_engine::engine::EffectSource::Move { pokemon, move_id },
            ..
        } if *target == D && *pokemon == A && *move_id == MoveId::REVELATION_DANCE
    )));
    assert!(!arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::DamageApplied {
            target,
            source: pokemon_battle_engine::engine::EffectSource::Move { pokemon, move_id },
            ..
        } if *target == B && *pokemon == A && *move_id == MoveId::REVELATION_DANCE
    )));
}

/// When the dance's target is not a foe of the dancer's (the dance was aimed at the dancer
/// itself, or the original user is not its ally), the dance-along goes at the original user —
/// not at the foe the move would default to.
#[test]
fn a_dancer_falls_back_to_the_original_user() {
    let mut arena = setup_doubles(
        Generation::Gen9,
        [
            set("Snorlax", &["Harden"], "Dancer"),
            set("Blissey", &["Harden"], "Natural Cure"),
            set("Snorlax", &["Harden"], "Thick Fat"),
            set("Volcarona", &["Revelation Dance", "Harden"], "Flame Body"),
        ],
    );
    let start = arena.engine.trace().events().len();
    // Volcarona (P2's second) dances at its own ally (P2's first, Snorlax): the dance's target
    // is a Pokémon of the dancer's foe's side but not ... the dancer itself dances at the
    // original user, Volcarona.
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 0, None), arena.move_choice(C, 0, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![
                arena.move_choice(B, 0, None),
                arena.move_choice(D, 0, Some(B)),
            ],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    // The dancer (A) hit Volcarona (D), the original user.
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::DamageApplied {
            target,
            source: pokemon_battle_engine::engine::EffectSource::Move { pokemon, move_id },
            ..
        } if *target == D && *pokemon == A && *move_id == MoveId::REVELATION_DANCE
    )));
    assert!(!arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::DamageApplied {
            target,
            source: pokemon_battle_engine::engine::EffectSource::Move { pokemon, move_id },
            ..
        } if *target == B && *pokemon == A && *move_id == MoveId::REVELATION_DANCE
    )));
}

// --- L.4d: Instruct (DESIGN.md §42.9) ----------------------------------------------------------

/// Instruct injects an action of its own for the target — the target's own action for the turn
/// stays — and refuses what the move cannot repeat.
#[test]
fn instruct_injects_an_action_and_refuses_by_the_move() {
    let mut arena = setup_doubles(
        Generation::Gen9,
        [
            set("Clefable", &["Instruct", "Harden"], "Magic Guard"),
            set("Snorlax", &["Harden", "Tackle"], "Thick Fat"),
            set("Volcarona", &["Quiver Dance", "Harden"], "Flame Body"),
            set("Blissey", &["Instruct", "Harden"], "Natural Cure"),
        ],
    );
    // Turn 1: everyone hardens (Snorlax's last move becomes Harden).
    let first = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.move_choice(A, 1, None), arena.move_choice(C, 0, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![
                arena.move_choice(B, 0, None),
                arena.move_choice(D, 1, Some(B)),
            ],
        },
    ];
    arena.engine.submit_turn_choices(first).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    // Both act: Clefable instructs Snorlax, and Snorlax still takes the action it chose.
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![
                arena.move_choice(A, 0, Some(C)),
                arena.move_choice(C, 1, Some(B)),
            ],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![
                arena.move_choice(B, 1, Some(C)),
                arena.move_choice(D, 0, Some(A)),
            ],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    // The injected action: Snorlax's Harden, marked Instructed.
    assert!(arena.events(start).iter().any(|event| matches!(
        event,
        TraceEvent::Instructed { target, instructor, move_id }
            if *target == C && *instructor == A && *move_id == MoveId::HARDEN
    )));
    let snorlax_moves: Vec<MoveId> = arena
        .events(start)
        .iter()
        .filter_map(|event| match event {
            TraceEvent::MoveUsed { actor, move_id, .. } if *actor == C => Some(*move_id),
            _ => None,
        })
        .collect();
    // The injected Harden first, then its own action (Tackle, at the foe).
    assert_eq!(
        snorlax_moves,
        vec![MoveId::HARDEN, MoveId::TACKLE],
        "the target's own action was consumed"
    );

    // A Charging move (Solar Beam) cannot be instructed; neither can Instruct itself.
    let refused = |last: MoveId, moves: &[&str]| {
        let mut arena = setup(
            Generation::Gen9,
            set("Clefable", &["Instruct", "Harden"], "Magic Guard"),
            set("Snorlax", moves, "Thick Fat"),
            |_| {},
        );
        edit_state(&mut arena, |state| {
            let snorlax = state.pokemon(B).unwrap().id();
            state.pokemon_mut(snorlax).unwrap().last_move = Some(last);
        });
        let start = arena.turn(0, 0);
        !arena
            .events(start)
            .iter()
            .any(|event| matches!(event, TraceEvent::Instructed { .. }))
    };
    // A charging move that carries no `failinstruct` (Electro Shot): the `charge` check alone
    // must refuse it.
    assert!(
        refused(MoveId::ELECTRO_SHOT, &["Electro Shot", "Tackle"]),
        "a charging move was repeated"
    );
    assert!(
        refused(MoveId::INSTRUCT, &["Instruct", "Tackle"]),
        "failinstruct was ignored"
    );
    assert!(
        !refused(MoveId::TACKLE, &["Tackle", "Harden"]),
        "a plain move was refused"
    );
    // The injected move is the last move, taken from its own slot — not merely the first slot
    // with PP (Harden here comes before Tackle).
    let mut arena = setup(
        Generation::Gen9,
        set("Clefable", &["Instruct", "Harden"], "Magic Guard"),
        set("Snorlax", &["Harden", "Tackle"], "Thick Fat"),
        |_| {},
    );
    edit_state(&mut arena, |state| {
        let snorlax = state.pokemon(B).unwrap().id();
        state.pokemon_mut(snorlax).unwrap().last_move = Some(MoveId::TACKLE);
    });
    let start = arena.turn(0, 0);
    // The injected action actually uses Tackle (its own slot), then Snorlax's own action
    // (Harden, the slot it chose).
    let used: Vec<MoveId> = arena
        .events(start)
        .iter()
        .filter_map(|event| match event {
            TraceEvent::MoveUsed { actor, move_id, .. } if *actor == B => Some(*move_id),
            _ => None,
        })
        .collect();
    assert_eq!(used, vec![MoveId::TACKLE, MoveId::HARDEN]);
}
