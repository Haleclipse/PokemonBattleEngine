//! U-turn / Volt Switch: the first mechanic that suspends a turn mid-way (DESIGN.md §30, ROADMAP
//! Batch C / M1). A landed self-switch move orders the user out, the engine returns a
//! `ForcedSwitch` request with the remaining actions still pending, the replacement runs the full
//! switch lifecycle, and the turn resumes. Snapshot → restore at the suspension point must be
//! indistinguishable from running straight through.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, SelfSwitch, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, BattleRequest, EngineStep, ForcedReplacement, ForcedSwitchChoice,
    HitBlockReason, ReplacementSlot, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattlePhase, BattleRng, BattleState, NonVolatileStatus, PassKind, RandomOverride,
    RandomRequestKind, SideState,
};

const JOLTEON: PokemonId = PokemonId(1);
const VAPOREON: PokemonId = PokemonId(2);
const SNORLAX: PokemonId = PokemonId(3);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

/// P1: Jolteon (U-turn) with Vaporeon on the bench unless `bench` is false; P2: Snorlax.
fn arena(bench: bool, snorlax_move: &str) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let mut pokemon = vec![
        build_pokemon(
            &dex,
            JOLTEON,
            SideId::P1,
            &PokemonSet::new("Jolteon", ["U-turn"]),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            SNORLAX,
            SideId::P2,
            &PokemonSet::new("Snorlax", [snorlax_move]),
        )
        .unwrap(),
    ];
    let mut p1_team = vec![JOLTEON];
    if bench {
        pokemon.push(
            build_pokemon(
                &dex,
                VAPOREON,
                SideId::P1,
                &PokemonSet::new("Vaporeon", ["Tackle"]),
            )
            .unwrap(),
        );
        p1_team.push(VAPOREON);
    }
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side(p1_team), side(vec![SNORLAX])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn with_tape(&mut self, tape: Vec<RandomOverride>) {
        self.with_tape_for(2, tape);
    }

    /// Speeds differ in every arena here, so no SpeedTie decision is drawn.
    fn with_tape_for(&mut self, _actors: usize, tape: Vec<RandomOverride>) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
    }

    fn submit_moves(&mut self, p1_actor: PokemonId) {
        let target =
            |side: SideId| DeclaredTarget(self.rules.topology().position(side, 0).unwrap());
        let choice = |side: SideId, actor: PokemonId, at: Option<DeclaredTarget>| SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::FIRST,
                    target: at,
                    mechanic: None,
                },
            }],
        };
        let p2_move = self.engine.state().pokemon(SNORLAX).unwrap().moves[0].move_id;
        let p2_target = match self.dex.move_by_id(p2_move).unwrap().target {
            pokemon_battle_engine::rules::TargetPattern::SelfOnly => None,
            _ => Some(target(SideId::P1)),
        };
        self.engine
            .submit_turn_choices([
                choice(SideId::P1, p1_actor, Some(target(SideId::P2))),
                choice(SideId::P2, SNORLAX, p2_target),
            ])
            .unwrap();
    }

    fn events(&self) -> &[TraceEvent] {
        self.engine.trace().events()
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn replace_with(arena: &mut Arena, position_side: SideId, incoming: PokemonId) -> BattlePhase {
    let position = arena.rules.topology().position(position_side, 0).unwrap();
    arena
        .engine
        .resolve_forced_switches([
            ForcedSwitchChoice {
                side: SideId::P1,
                replacements: if position_side == SideId::P1 {
                    vec![ForcedReplacement {
                        position,
                        pokemon: incoming,
                    }]
                } else {
                    vec![]
                },
            },
            ForcedSwitchChoice {
                side: SideId::P2,
                replacements: vec![],
            },
        ])
        .unwrap()
        .phase
}

#[test]
fn u_turn_suspends_the_turn_and_resumes_after_the_replacement() {
    let mut arena = arena(true, "Tackle");
    // Jolteon's U-turn, then (after the replacement) Snorlax's Tackle into Vaporeon.
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.with_tape(tape);
    arena.submit_moves(JOLTEON);

    let step = arena.engine.step().unwrap();
    let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) = step else {
        panic!("expected a mid-turn ForcedSwitch request, got {step:?}");
    };
    let p1_position = arena.rules.topology().position(SideId::P1, 0).unwrap();
    assert_eq!(
        replacements[0].positions,
        vec![ReplacementSlot {
            position: p1_position,
            outgoing: Some(JOLTEON)
        }]
    );
    assert_eq!(replacements[0].required_count, 1);
    assert_eq!(replacements[1].required_count, 0);
    assert!(arena.events().iter().any(|e| matches!(
        e,
        TraceEvent::SwitchOutRequested { actor, .. } if *actor == JOLTEON
    )));
    assert!(
        arena.engine.state().pokemon(SNORLAX).unwrap().hp
            < arena.engine.state().pokemon(SNORLAX).unwrap().max_hp
    );
    // Jolteon is still on the field until the choice is made.
    assert_eq!(
        arena.engine.state().side(SideId::P1).active[0],
        Some(JOLTEON)
    );

    // The replacement runs the full switch lifecycle and the turn resumes.
    assert_eq!(
        replace_with(&mut arena, SideId::P1, VAPOREON),
        BattlePhase::TurnResolution
    );
    assert_eq!(
        arena.engine.state().side(SideId::P1).active[0],
        Some(VAPOREON)
    );
    assert!(arena.engine.state().side(SideId::P1).switch_out.is_none());
    assert!(arena.events().iter().any(|e| matches!(
        e,
        TraceEvent::Switched { outgoing: Some(out), incoming, .. } if *out == JOLTEON && *incoming == VAPOREON
    )));

    // Snorlax's pending Tackle now hits the Pokémon that came in.
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
    assert!(arena.events().iter().any(|e| matches!(
        e,
        TraceEvent::DamageApplied { target, .. } if *target == VAPOREON
    )));
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
    assert_eq!(arena.engine.state().turn, 2);
    assert!(arena.engine.state().rng.semantic_tape_exhausted());
}

#[test]
fn mid_turn_snapshot_restore_resumes_identically() {
    let mut arena = arena(true, "Tackle");
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.with_tape(tape);
    arena.submit_moves(JOLTEON);
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::ForcedSwitch { .. })
    ));
    let suspended = arena.engine.snapshot();
    let suspended_at = arena.engine.trace().events().len();

    // Path A: keep going.
    replace_with(&mut arena, SideId::P1, VAPOREON);
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}

    // Path B: destroy, restore at the suspension point, keep going.
    let mut restored = Arena {
        rules: arena.rules.clone(),
        dex: arena.dex.clone(),
        engine: BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), suspended).unwrap(),
    };
    assert!(matches!(
        restored.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::ForcedSwitch { .. })
    ));
    replace_with(&mut restored, SideId::P1, VAPOREON);
    while let EngineStep::Progressed { .. } = restored.engine.step().unwrap() {}

    // The same state; the restored battle's trace, empty at the restore (DESIGN.md §47.0 item
    // 6), is what the uninterrupted one traced from the suspension on.
    assert_eq!(arena.engine.state(), restored.engine.state());
    assert_eq!(
        &arena.engine.trace().events()[suspended_at..],
        restored.engine.trace().events()
    );
}

#[test]
fn u_turn_without_a_bench_lets_the_turn_continue() {
    let mut arena = arena(false, "Tackle");
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.with_tape(tape);
    arena.submit_moves(JOLTEON);
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
    assert!(arena.engine.state().side(SideId::P1).switch_out.is_none());
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
}

#[test]
fn a_blocked_u_turn_does_not_order_the_user_out() {
    // Snorlax's Protect (+4 priority) resolves first; U-turn is blocked and lands nothing.
    let mut arena = arena(true, "Protect");
    arena.with_tape(vec![]);
    arena.submit_moves(JOLTEON);
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
    assert!(arena.engine.state().side(SideId::P1).switch_out.is_none());
    assert!(
        !arena
            .events()
            .iter()
            .any(|e| matches!(e, TraceEvent::SwitchOutRequested { .. }))
    );
}

#[test]
fn the_replacement_must_target_the_ordered_out_position() {
    let mut arena = arena(true, "Tackle");
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.with_tape(tape);
    arena.submit_moves(JOLTEON);
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::ForcedSwitch { .. })
    ));
    let wrong = arena.rules.topology().position(SideId::P2, 0).unwrap();
    let result = arena.engine.resolve_forced_switches([
        ForcedSwitchChoice {
            side: SideId::P1,
            replacements: vec![ForcedReplacement {
                position: wrong,
                pokemon: VAPOREON,
            }],
        },
        ForcedSwitchChoice {
            side: SideId::P2,
            replacements: vec![],
        },
    ]);
    assert!(matches!(
        result,
        Err(BattleError::Choice(_)) | Err(BattleError::InvalidReplacementPosition(_))
    ));
}

#[test]
fn u_turn_as_the_last_action_still_runs_the_end_of_turn_residual() {
    // Snorlax's Quick Attack (+1) goes first, so U-turn is the turn's last action. The
    // replacement must resume the same turn: the residual (Snorlax's burn) runs before the
    // next TurnChoices, not never.
    let mut arena = arena(true, "Quick Attack");
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.with_tape(tape);
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.pokemon_mut(SNORLAX).unwrap().status = Some(
        pokemon_battle_engine::state::StatusState::new(NonVolatileStatus::Burn),
    );
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    arena.submit_moves(JOLTEON);

    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::ForcedSwitch { .. })
    ));
    let hp_after_u_turn = arena.engine.state().pokemon(SNORLAX).unwrap().hp;

    assert_eq!(
        replace_with(&mut arena, SideId::P1, VAPOREON),
        BattlePhase::TurnResolution
    );
    assert_eq!(arena.engine.state().turn, 1);
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
    assert_eq!(arena.engine.state().turn, 2);
    let residual = hp_after_u_turn - arena.engine.state().pokemon(SNORLAX).unwrap().hp;
    assert_eq!(
        u32::from(residual),
        u32::from(arena.engine.state().pokemon(SNORLAX).unwrap().max_hp) / 16,
        "burn residual after the resumed turn"
    );
}

#[test]
fn volt_switch_is_self_switch_and_an_immune_target_cancels_the_switch() {
    let dex = DexResolver::resolve(Generation::Gen9);
    for name in ["U-turn", "Volt Switch", "Flip Turn"] {
        let mv = dex.move_by_name(name).unwrap();
        assert!(
            mv.self_switch.is_some() && mv.unsupported.is_empty(),
            "{name}"
        );
    }
    // Status self-switches: Teleport and Chilly Reception use the plain `selfSwitch`; Parting
    // Shot sends its user out itself (DESIGN.md §30.6); Baton Pass and Shed Tail hand things on
    // ("copyvolatile", "shedtail", §30.7).
    for name in ["Teleport", "Chilly Reception"] {
        let mv = dex.move_by_name(name).unwrap();
        assert!(
            mv.self_switch == Some(SelfSwitch::Plain) && mv.unsupported.is_empty(),
            "{name}"
        );
    }
    let parting_shot = dex.move_by_name("Parting Shot").unwrap();
    assert!(parting_shot.self_switch.is_none() && parting_shot.unsupported.is_empty());
    for (name, kind) in [
        ("Baton Pass", PassKind::BatonPass),
        ("Shed Tail", PassKind::ShedTail),
    ] {
        let mv = dex.move_by_name(name).unwrap();
        assert!(
            mv.self_switch == Some(SelfSwitch::Pass(kind)) && mv.unsupported.is_empty(),
            "{name}"
        );
    }

    // Volt Switch into a Ground type lands nothing, so nobody is ordered out.
    let mut arena = arena(true, "Tackle");
    let mut snapshot = arena.engine.snapshot();
    let garchomp = build_pokemon(
        &arena.dex,
        SNORLAX,
        SideId::P2,
        &PokemonSet::new("Garchomp", ["Tackle"]),
    )
    .unwrap();
    *snapshot.state.pokemon_mut(SNORLAX).unwrap() = garchomp;
    let jolteon = build_pokemon(
        &arena.dex,
        JOLTEON,
        SideId::P1,
        &PokemonSet::new("Jolteon", ["Volt Switch"]),
    )
    .unwrap();
    *snapshot.state.pokemon_mut(JOLTEON).unwrap() = jolteon;
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    arena.with_tape(hit(100)); // Garchomp's Tackle only; Volt Switch never reaches a roll
    arena.submit_moves(JOLTEON);
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
    assert!(arena.events().iter().any(|e| matches!(
        e,
        TraceEvent::HitBlocked {
            reason: HitBlockReason::TypeImmunity,
            ..
        }
    )));
    assert!(arena.engine.state().side(SideId::P1).switch_out.is_none());
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Progressed { .. }
    ));
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
}

const PIKACHU: PokemonId = PokemonId(4);
const SNORLAX_B: PokemonId = PokemonId(5);

/// Doubles: the replacement request names the ordered-out slot only; the ally and both foes
/// still act after the replacement.
#[test]
fn doubles_u_turn_requests_only_the_ordered_out_slot() {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Doubles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let build = |id, side, species, mv| {
        build_pokemon(&dex, id, side, &PokemonSet::new(species, [mv])).unwrap()
    };
    let pokemon = vec![
        build(JOLTEON, SideId::P1, "Jolteon", "U-turn"),
        build(PIKACHU, SideId::P1, "Pikachu", "Tackle"),
        build(VAPOREON, SideId::P1, "Vaporeon", "Tackle"),
        build(SNORLAX, SideId::P2, "Snorlax", "Tackle"),
        build(SNORLAX_B, SideId::P2, "Snorlax", "Tackle"),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let state = BattleState::new(
        pokemon,
        [
            side(vec![JOLTEON, PIKACHU, VAPOREON]),
            side(vec![SNORLAX, SNORLAX_B]),
        ],
        1,
    );
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let mut arena = Arena { rules, dex, engine };
    // The two Snorlax tie on speed: two SpeedTie decisions at the start of the turn, and two
    // more at each Gen 8+ re-sort that still finds both of them waiting (before Pikachu's and
    // before the first Snorlax's action; Showdown `speedSort` reshuffles ties every time).
    let tie = || vec![RandomOverride::raw(RandomRequestKind::SpeedTie, 1); 2];
    let mut tape = tie();
    tape.extend(hit(100)); // Jolteon's U-turn
    tape.extend(tie());
    tape.extend(hit(100)); // Pikachu
    tape.extend(tie());
    tape.extend(hit(100)); // Snorlax
    tape.extend(hit(100)); // the other Snorlax
    arena.with_tape_for(4, tape);

    let at = |side: SideId, slot: u8| {
        DeclaredTarget(arena.rules.topology().position(side, slot).unwrap())
    };
    let mv = |actor, target| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::FIRST,
            target: Some(target),
            mechanic: None,
        },
    };
    arena
        .engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![
                    mv(JOLTEON, at(SideId::P2, 0)),
                    mv(PIKACHU, at(SideId::P2, 1)),
                ],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![
                    mv(SNORLAX, at(SideId::P1, 0)),
                    mv(SNORLAX_B, at(SideId::P1, 1)),
                ],
            },
        ])
        .unwrap();

    // Jolteon (130) is fastest: U-turn suspends the turn.
    let step = arena.engine.step().unwrap();
    let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) = step else {
        panic!("expected a mid-turn ForcedSwitch request, got {step:?}");
    };
    assert_eq!(
        replacements[0].positions,
        vec![ReplacementSlot {
            position: arena.rules.topology().position(SideId::P1, 0).unwrap(),
            outgoing: Some(JOLTEON)
        }]
    );
    assert_eq!(replacements[0].available_pokemon, vec![VAPOREON]);
    assert_eq!(replacements[1].required_count, 0);

    assert_eq!(
        replace_with(&mut arena, SideId::P1, VAPOREON),
        BattlePhase::TurnResolution
    );
    assert_eq!(
        arena.engine.state().side(SideId::P1).active,
        vec![Some(VAPOREON), Some(PIKACHU)]
    );
    // Pikachu, then both Snorlax, still act; Snorlax's Tackle into slot 0 hits Vaporeon.
    let mut acted = Vec::new();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {
        acted.push(());
    }
    assert_eq!(acted.len(), 3, "three actions left after the replacement");
    assert!(arena.events().iter().any(|e| matches!(
        e,
        TraceEvent::DamageApplied { target, .. } if *target == VAPOREON
    )));
    assert_eq!(arena.engine.state().turn, 2);
    assert!(arena.engine.state().rng.semantic_tape_exhausted());
}
