//! L1 rule tests for M4 batch 3 (ROADMAP §8.2 "Trap" / "Protect-family", DESIGN.md §10.6):
//! Bind / Wrap / Fire Spin … as a timed trap with a residual chip, Ingrain, Suction Cups
//! refusing a drag-out, and Protect's consecutive-use counter.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, SecondaryEffect, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, ChoiceError, EffectSource, EngineStep, HitBlockReason,
    MoveFailReason, SideChoice, SlotChoice, TraceEvent, trapped_by, trapped_by_ability,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, ProtectStallRules, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, PokemonState, RandomOverride, RandomRequestKind, SideState,
    VolatileConditionKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);
const D: PokemonId = PokemonId(4);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], ability: Option<&str>, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.unwrap_or("Run Away").into());
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

/// P1 = [A, C (bench)]; P2 = [B, D (bench)].
fn setup(generation: Generation, p1: PokemonSet, p2: PokemonSet) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
        build_pokemon(
            &dex,
            C,
            SideId::P1,
            &set("Vaporeon", &["Tackle"], None, None),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            D,
            SideId::P2,
            &set("Vaporeon", &["Tackle"], None, None),
        )
        .unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B, D])], 1);
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
                    | TargetPattern::SelectedAdjacentFoe => Some(DeclaredTarget(
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
    ) -> Result<usize, BattleError> {
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

    fn has(&self, from: usize, pred: impl Fn(&TraceEvent) -> bool) -> bool {
        self.events(from).iter().any(pred)
    }

    fn turns_left(&self, id: PokemonId, kind: VolatileConditionKind) -> Option<u8> {
        self.pokemon(id).volatiles.turns_left(kind)
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

/// Bind (85 %): accuracy, the hit, then the bind's duration.
fn bind(duration: u16) -> Vec<RandomOverride> {
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    tape.push(RandomOverride::range(RandomRequestKind::Duration, duration));
    tape
}

fn residual_damage(e: &TraceEvent, target: PokemonId, kind: VolatileConditionKind) -> bool {
    matches!(
        e,
        TraceEvent::DamageApplied { target: t, source: EffectSource::Volatile(k), .. }
            if *t == target && *k == kind
    )
}

#[test]
fn binding_moves_protect_family_and_suction_cups_classify_from_snapshot_data() {
    let gen9 = DexResolver::resolve(Generation::Gen9);
    for name in [
        "Bind",
        "Wrap",
        "Fire Spin",
        "Whirlpool",
        "Sand Tomb",
        "Infestation",
    ] {
        let mv = gen9.move_by_name(name).unwrap();
        assert_eq!(mv.primary_effect, PrimaryEffect::Damage, "{name}");
        assert!(
            mv.secondaries.iter().any(|s| matches!(
                s.effect,
                SecondaryEffect::Volatile(VolatileConditionKind::PartiallyTrapped)
            )),
            "{name}"
        );
    }
    for name in ["Protect", "Detect", "Ingrain"] {
        assert_eq!(
            gen9.move_by_name(name).unwrap().primary_effect,
            PrimaryEffect::Custom,
            "{name}"
        );
    }
    assert!(gen9.ability_by_name("Suction Cups").unwrap().implemented);
}

#[test]
fn bind_traps_for_its_rolled_turns_chips_at_the_residual_and_ends_with_the_trapper() {
    let mut arena = setup(
        Generation::Gen9,
        set("Snorlax", &["Bind"], None, None),
        set("Garchomp", &["Swords Dance"], None, None),
    );
    // Turn 1: Bind lands, the duration is rolled (5), the residual ticks it to 4 and chips 1/8.
    let start = arena.turn(Action::Move(0), Action::Move(0), bind(5));
    let max_hp = arena.pokemon(B).max_hp;
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileStarted {
            target: B,
            kind: VolatileConditionKind::PartiallyTrapped
        }
    )));
    assert_eq!(
        arena.turns_left(B, VolatileConditionKind::PartiallyTrapped),
        Some(4)
    );
    assert!(arena.has(start, |e| residual_damage(
        e,
        B,
        VolatileConditionKind::PartiallyTrapped
    )));
    let after_hit = arena
        .events(start)
        .iter()
        .find_map(|e| match e {
            TraceEvent::DamageApplied {
                target: B,
                hp_after,
                source: EffectSource::Move { .. },
                ..
            } => Some(*hp_after),
            _ => None,
        })
        .unwrap();
    assert_eq!(arena.pokemon(B).hp, after_hit - max_hp / 8);

    // Bound: Garchomp cannot switch (Showdown `trapped`).
    assert!(matches!(
        arena.try_turn(Action::Move(0), Action::Switch(D), Vec::new()),
        Err(BattleError::Choice(ChoiceError::Trapped {
            actor: B,
            by: VolatileConditionKind::PartiallyTrapped,
        }))
    ));
    assert_eq!(
        trapped_by(arena.engine.state(), &arena.rules, B),
        Some(VolatileConditionKind::PartiallyTrapped)
    );

    // A second Bind on a bound target hits and rolls nothing new.
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(Action::Move(0), Action::Move(0), tape);
    assert_eq!(
        arena.turns_left(B, VolatileConditionKind::PartiallyTrapped),
        Some(3)
    );

    // The trapper switches out: the bind ends at that residual, with no chip, and the
    // target may switch again.
    let hp_before = arena.pokemon(B).hp;
    let start = arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileEnded {
            target: B,
            kind: VolatileConditionKind::PartiallyTrapped,
            ..
        }
    )));
    assert!(!arena.has(start, |e| residual_damage(
        e,
        B,
        VolatileConditionKind::PartiallyTrapped
    )));
    assert_eq!(arena.pokemon(B).hp, hp_before);
    assert!(
        arena
            .try_turn(Action::Move(0), Action::Switch(D), hit(100))
            .is_ok()
    );

    // A bind that runs its course: 5 turns rolled = 4 chips, the fifth residual ends it.
    let mut arena = setup(
        Generation::Gen9,
        set("Snorlax", &["Bind"], None, None),
        set("Blissey", &["Swords Dance"], None, None),
    );
    arena.turn(Action::Move(0), Action::Move(0), bind(5));
    let mut chips = 1;
    for _ in 0..3 {
        let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)];
        tape.truncate(1);
        let start = arena.turn(Action::Move(0), Action::Move(0), tape);
        if arena.has(start, |e| {
            residual_damage(e, B, VolatileConditionKind::PartiallyTrapped)
        }) {
            chips += 1;
        }
    }
    assert_eq!(chips, 4);
    assert_eq!(
        arena.turns_left(B, VolatileConditionKind::PartiallyTrapped),
        Some(1)
    );
    let start = arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)],
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileEnded {
            target: B,
            kind: VolatileConditionKind::PartiallyTrapped,
            ..
        }
    )));
    assert!(!arena.has(start, |e| residual_damage(
        e,
        B,
        VolatileConditionKind::PartiallyTrapped
    )));

    // A Ghost type walks out of a bind from Gen 6 (Fire Spin: Bind itself is Normal).
    let mut arena = setup(
        Generation::Gen9,
        set("Snorlax", &["Fire Spin"], None, None),
        set("Gengar", &["Swords Dance"], None, None),
    );
    arena.turn(Action::Move(0), Action::Move(0), bind(5));
    assert!(
        arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::PartiallyTrapped)
    );
    assert_eq!(trapped_by(arena.engine.state(), &arena.rules, B), None);
    assert!(
        arena
            .try_turn(Action::Move(0), Action::Switch(D), Vec::new())
            .is_ok()
    );

    // Gen 4: 3-6 turns, 1/16.
    let mut arena = setup(
        Generation::Gen4,
        set("Snorlax", &["Bind"], None, None),
        set("Garchomp", &["Swords Dance"], None, None),
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), bind(3));
    let max_hp = arena.pokemon(B).max_hp;
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DamageApplied {
            target: B,
            amount,
            source: EffectSource::Volatile(VolatileConditionKind::PartiallyTrapped),
            ..
        } if *amount == max_hp / 16
    )));
    let rules = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .conditions()
    };
    assert_eq!(
        (
            rules(Generation::Gen4).bind_turns_min,
            rules(Generation::Gen4).bind_turns_max
        ),
        (3, 6)
    );
    assert_eq!(
        (
            rules(Generation::Gen9).bind_turns_min,
            rules(Generation::Gen9).bind_turns_max
        ),
        (5, 6)
    );
    assert!(!rules(Generation::Gen5).ghost_escapes_trapping);
}

#[test]
fn protect_in_a_row_rolls_the_stall_counter_and_fails_for_the_last_to_act() {
    let mut arena = setup(
        Generation::Gen9,
        set("Garchomp", &["Protect", "Swords Dance"], None, None),
        set("Snorlax", &["Tackle"], None, None),
    );
    let protected = |arena: &Arena, from: usize| {
        arena.has(from, |e| {
            matches!(
                e,
                TraceEvent::HitBlocked {
                    target: A,
                    reason: HitBlockReason::Protect,
                    ..
                }
            )
        })
    };
    // First use: no roll, the counter starts (stall, duration 2 → 1 after the residual).
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(protected(&arena, start));
    assert!(!arena.has(start, |e| matches!(e, TraceEvent::ProtectChecked { .. })));
    assert_eq!(arena.pokemon(A).volatiles.stall_counter(), Some(3));
    assert_eq!(arena.turns_left(A, VolatileConditionKind::Stall), Some(1));
    // Second use: 1/3, succeeds; the counter is 9.
    let start = arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(
            RandomRequestKind::ProtectSuccess,
            true,
        )],
    );
    assert!(protected(&arena, start));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ProtectChecked {
            actor: A,
            counter: 3,
            success: true
        }
    )));
    assert_eq!(arena.pokemon(A).volatiles.stall_counter(), Some(9));
    // Third use: 1/9, fails; the move fails, the counter is gone, Tackle lands.
    let mut tape = vec![RandomOverride::bool(
        RandomRequestKind::ProtectSuccess,
        false,
    )];
    tape.extend(hit(100));
    let start = arena.turn(Action::Move(0), Action::Move(0), tape);
    assert!(!protected(&arena, start));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: A,
            reason: MoveFailReason::Condition(_),
            ..
        }
    )));
    assert_eq!(arena.pokemon(A).volatiles.stall_counter(), None);
    // A turn without Protect, then Protect works again without a roll.
    arena.turn(Action::Move(1), Action::Move(0), hit(100));
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(protected(&arena, start));
    assert_eq!(arena.pokemon(A).volatiles.stall_counter(), Some(3));

    // The last Pokémon to act cannot protect (Showdown `willAct`): both protect, the faster
    // Garchomp's works, the slower Snorlax's fails with no roll.
    let mut arena = setup(
        Generation::Gen9,
        set("Garchomp", &["Protect"], None, None),
        set("Snorlax", &["Protect"], None, None),
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileStarted {
            target: A,
            kind: VolatileConditionKind::Protect
        }
    )));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: B,
            reason: MoveFailReason::Condition(_),
            ..
        }
    )));
    assert!(
        !arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::Protect)
    );

    // Gen 3-4 double the counter up to 8; Gen 6+ triple it up to 729.
    let stall = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .conditions()
            .protect_stall
    };
    assert_eq!(
        stall(Generation::Gen4),
        ProtectStallRules::Growing {
            initial: 2,
            multiplier: 2,
            max: 8,
            failure_ends: false
        }
    );
    assert_eq!(
        stall(Generation::Gen9),
        ProtectStallRules::Growing {
            initial: 3,
            multiplier: 3,
            max: 729,
            failure_ends: true
        }
    );
    // Gen 2: 127 / 255, halving on every success; 0 counts as 127 again; a failure keeps it.
    let gen2 = stall(Generation::Gen2);
    assert_eq!(gen2.start(), 127);
    assert_eq!(gen2.chance(gen2.start()), (127, 255));
    assert_eq!(gen2.next(127), 63);
    assert_eq!(gen2.chance(0), (127, 255));
    assert!(!gen2.failure_ends_stall());
}

#[test]
fn suction_cups_and_ingrain_refuse_a_drag_out_and_ingrain_roots_and_heals() {
    // Suction Cups: Roar is held (Showdown's `DragOut` returns `null`: the move still hit, no
    // `-fail`), Dragon Tail hits without dragging, Red Card is spent for nothing.
    let mut arena = setup(
        Generation::Gen9,
        set(
            "Malamar",
            &["Swords Dance", "Tackle"],
            Some("Suction Cups"),
            None,
        ),
        set(
            "Snorlax",
            &["Roar", "Dragon Tail", "Tackle"],
            None,
            Some("Red Card"),
        ),
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DragOutBlocked {
            target: A,
            by: EffectSource::Ability {
                ability: AbilityId::SUCTION_CUPS,
                ..
            },
        }
    )));
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed { actor: B, .. }
    )));
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    let start = arena.turn(Action::Move(0), Action::Move(1), tape);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DamageApplied { target: A, .. }
    )));
    assert!(!arena.has(start, |e| matches!(e, TraceEvent::DraggedOut { .. })));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DragOutBlocked { target: A, .. }
    )));
    // Cradily's Tackle triggers Snorlax's Red Card: consumed, but Cradily stays.
    let start = arena.turn(Action::Move(1), Action::Move(2), {
        let mut tape = hit(100);
        tape.extend(hit(100));
        tape
    });
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ItemConsumed {
            holder: B,
            item: ItemId::RED_CARD,
            seen: true,
        }
    )));
    assert!(!arena.has(start, |e| matches!(e, TraceEvent::DraggedOut { .. })));
    assert_eq!(arena.engine.state().side(SideId::P1).active[0], Some(A));

    // Ingrain: rooted (no switch, no drag-out), healed 1/16 at the residual.
    let mut arena = setup(
        Generation::Gen9,
        set("Garchomp", &["Ingrain", "Swords Dance"], None, None),
        set("Snorlax", &["Tackle", "Roar"], None, None),
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::HealApplied {
            target: A,
            source: EffectSource::Volatile(VolatileConditionKind::Ingrain),
            ..
        }
    )));
    assert!(matches!(
        arena.try_turn(Action::Switch(C), Action::Move(0), hit(100)),
        Err(BattleError::Choice(ChoiceError::Trapped {
            actor: A,
            by: VolatileConditionKind::Ingrain,
        }))
    ));
    let start = arena.turn(Action::Move(1), Action::Move(1), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DragOutBlocked {
            target: A,
            by: EffectSource::Volatile(VolatileConditionKind::Ingrain),
        }
    )));
    // Ingrain again fails.
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: A,
            reason: MoveFailReason::Condition(_),
            ..
        }
    )));
    let _ = MoveId::INGRAIN;
}

/// Mean Look (DESIGN.md §30.8): the target may not switch while the trapper stays in; the
/// trapper leaving releases it. A Ghost type is immune from Gen 6 (the move fails), not before.
#[test]
fn mean_look_refuses_the_switch_until_its_user_leaves() {
    let mut arena = setup(
        Generation::Gen9,
        set("Mew", &["Mean Look", "Harden"], None, None),
        set("Snorlax", &["Harden"], None, None),
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(
        arena.pokemon(B).volatiles.trapped_by(),
        Some(A),
        "Mean Look traps its target"
    );
    assert!(
        arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::Trapper)
    );
    assert!(matches!(
        arena.try_turn(Action::Move(1), Action::Switch(D), Vec::new()),
        Err(BattleError::Choice(ChoiceError::Trapped {
            actor: B,
            by: VolatileConditionKind::Trapped,
        }))
    ));
    // The trapper switches out: the target is free again at once.
    let start = arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileEnded {
            target: B,
            kind: VolatileConditionKind::Trapped,
            ..
        }
    )));
    assert!(
        !arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::Trapper)
    );
    assert_eq!(trapped_by(arena.engine.state(), &arena.rules, B), None);
    assert!(
        arena
            .try_turn(Action::Move(0), Action::Switch(D), Vec::new())
            .is_ok()
    );

    // A Ghost type: Gen 6+ immune, Gen 5 trapped.
    for (generation, trapped) in [(Generation::Gen9, false), (Generation::Gen5, true)] {
        let mut arena = setup(
            generation,
            set("Mew", &["Mean Look"], None, None),
            set("Gengar", &["Harden"], None, None),
        );
        arena.turn(Action::Move(0), Action::Move(0), Vec::new());
        assert_eq!(
            trapped_by(arena.engine.state(), &arena.rules, B).is_some(),
            trapped,
            "{generation:?}"
        );
    }
}

/// Octolock (DESIGN.md §30.8): no switching while the octolocker stays in; its leaving frees
/// the target at once (the volatile itself lapses at the next residual).
#[test]
fn octolock_refuses_the_switch_while_its_user_stays_in() {
    let mut arena = setup(
        Generation::Gen8,
        set("Mew", &["Octolock", "Harden"], None, None),
        set("Snorlax", &["Harden"], None, None),
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(matches!(
        arena.try_turn(Action::Move(1), Action::Switch(D), Vec::new()),
        Err(BattleError::Choice(ChoiceError::Trapped {
            actor: B,
            by: VolatileConditionKind::Octolock,
        }))
    ));
    // Its source off the field, the volatile no longer traps (`onTrapPokemon` asks
    // `source.isActive`), even before the residual lets it lapse.
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.side_mut(SideId::P1).active[0] = Some(C);
    assert!(
        snapshot
            .state
            .pokemon(B)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Octolock)
    );
    assert_eq!(trapped_by(&snapshot.state, &arena.rules, B), None);
    arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    assert!(
        !arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::Octolock)
    );
    assert!(
        arena
            .try_turn(Action::Move(0), Action::Switch(D), Vec::new())
            .is_ok()
    );
}

/// Shadow Tag, Arena Trap, Magnet Pull (`onFoeTrapPokemon`; DESIGN.md §30.8): whom each keeps
/// from switching, generation by generation.
#[test]
fn trapping_abilities_refuse_the_switches_they_should() {
    // (generation, the holder's ability, the foe, its ability, trapped)
    let cases = [
        (Generation::Gen9, "Shadow Tag", "Snorlax", None, true),
        (Generation::Gen9, "Shadow Tag", "Gengar", None, false),
        (Generation::Gen5, "Shadow Tag", "Gengar", None, true),
        (
            Generation::Gen9,
            "Shadow Tag",
            "Mew",
            Some("Shadow Tag"),
            false,
        ),
        (
            Generation::Gen4,
            "Shadow Tag",
            "Mew",
            Some("Shadow Tag"),
            false,
        ),
        (
            Generation::Gen3,
            "Shadow Tag",
            "Mew",
            Some("Shadow Tag"),
            true,
        ),
        (Generation::Gen9, "Arena Trap", "Snorlax", None, true),
        (Generation::Gen9, "Arena Trap", "Corviknight", None, false),
        (
            Generation::Gen9,
            "Arena Trap",
            "Bronzong",
            Some("Levitate"),
            false,
        ),
        (Generation::Gen9, "Magnet Pull", "Corviknight", None, true),
        (Generation::Gen9, "Magnet Pull", "Snorlax", None, false),
        (Generation::Gen3, "Magnet Pull", "Metagross", None, true),
    ];
    for (generation, ability, foe, foe_ability, trapped) in cases {
        let mut arena = setup(
            generation,
            set("Mew", &["Harden"], Some(ability), None),
            set(foe, &["Harden"], foe_ability, None),
        );
        let label = format!("{generation:?} {ability} vs {foe}");
        let holder = arena.pokemon(A).ability;
        assert_eq!(
            trapped_by_ability(arena.engine.state(), &arena.rules, B),
            trapped.then_some((A, holder)),
            "{label}"
        );
        let switch = arena.try_turn(Action::Move(0), Action::Switch(D), Vec::new());
        if trapped {
            assert!(
                matches!(
                    switch,
                    Err(BattleError::Choice(ChoiceError::TrappedByAbility {
                        actor: B
                    }))
                ),
                "{label}"
            );
        } else {
            assert!(switch.is_ok(), "{label}");
        }
    }
}

/// Gen 3 Magnet Pull (`onAnyTrapPokemon`) also keeps an adjacent ally Steel type in; from Gen 4
/// (`onFoeTrapPokemon`) only foes.
#[test]
fn gen3_magnet_pull_traps_an_ally_steel_type() {
    for (generation, trapped) in [(Generation::Gen3, true), (Generation::Gen4, false)] {
        let rules = Arc::new(
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
                .unwrap(),
        );
        let dex = Arc::new(DexResolver::resolve(generation));
        let e = PokemonId(5);
        let f = PokemonId(6);
        let build = |id, side, species: &str, ability: Option<&str>| {
            build_pokemon(&dex, id, side, &set(species, &["Harden"], ability, None)).unwrap()
        };
        let pokemon = vec![
            build(A, SideId::P1, "Mew", Some("Magnet Pull")),
            build(B, SideId::P2, "Snorlax", None),
            build(C, SideId::P1, "Metagross", Some("Clear Body")),
            build(D, SideId::P2, "Snorlax", None),
            build(e, SideId::P1, "Snorlax", None),
            build(f, SideId::P2, "Snorlax", None),
        ];
        let side = |team: Vec<PokemonId>| {
            let mut side = SideState::new(team.clone(), 2);
            side.active[0] = Some(team[0]);
            side.active[1] = Some(team[1]);
            side
        };
        let state = BattleState::new(pokemon, [side(vec![A, C, e]), side(vec![B, D, f])], 1);
        assert_eq!(
            trapped_by_ability(&state, &rules, C).is_some(),
            trapped,
            "{generation:?}"
        );
        assert_eq!(
            trapped_by_ability(&state, &rules, B),
            None,
            "{generation:?}"
        );
    }
}

/// Shed Shell frees its holder from every trap — but a recharge (and a move it is locked
/// into, Showdown's `lockedMove`) still holds it: the request offers no switch then, so a
/// replay never asks.
#[test]
fn shed_shell_frees_from_traps_but_not_from_a_recharge() {
    for recharge in [false, true] {
        let mut arena = setup(
            Generation::Gen9,
            set("Snorlax", &["Harden"], None, Some("Shed Shell")),
            set("Umbreon", &["Harden"], None, None),
        );
        let mut snapshot = arena.engine.snapshot();
        let snorlax = snapshot.state.pokemon_mut(A).unwrap();
        snorlax.volatiles.set_trapped(B);
        if recharge {
            snorlax.volatiles.set_must_recharge(None);
        }
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
        assert_eq!(
            trapped_by(arena.engine.state(), &arena.rules, A),
            recharge.then_some(VolatileConditionKind::MustRecharge),
            "{recharge}"
        );
    }
}
