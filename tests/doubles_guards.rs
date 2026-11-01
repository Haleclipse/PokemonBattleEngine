//! L1 rule tests for M5 batch 2 (ROADMAP §9 "Wide Guard / Quick Guard", "Follow Me / Rage
//! Powder", DESIGN.md §15.6): side guards at TryHit, powder immunity (Grass, Overcoat, Safety
//! Goggles) and Rage Powder, Fake Out's first-action rule, absorbing abilities, and choice
//! validation by position (a slot emptied by a faint is a legal choice).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, StatusMoveEffect, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, EffectSource, EngineStep, HitBlockReason, MoveFailReason, SideChoice, SlotChoice,
    TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, PokemonState, RandomOverride, RandomRequestKind, SideConditionKind,
    SideState, VolatileConditionKind,
};

const P1A: PokemonId = PokemonId(1);
const P1B: PokemonId = PokemonId(2);
const P2A: PokemonId = PokemonId(3);
const P2B: PokemonId = PokemonId(4);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], ability: &str, item: Option<&str>, level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set.level = level;
    set
}

fn doubles(
    generation: Generation,
    p1: [PokemonSet; 2],
    p2: [PokemonSet; 2],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon = vec![
        build_pokemon(&dex, P1A, SideId::P1, &p1[0]).unwrap(),
        build_pokemon(&dex, P1B, SideId::P1, &p1[1]).unwrap(),
        build_pokemon(&dex, P2A, SideId::P2, &p2[0]).unwrap(),
        build_pokemon(&dex, P2B, SideId::P2, &p2[1]).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let state = BattleState::new(pokemon, [side([P1A, P1B]), side([P2A, P2B])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

type Decl = (PokemonId, usize, Option<(SideId, u8)>);

impl Arena {
    fn choice(&self, side: SideId, decls: &[Decl]) -> SideChoice {
        SideChoice {
            side,
            actions: decls
                .iter()
                .map(|(actor, slot, target)| SlotChoice {
                    actor: *actor,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(*slot as u8).unwrap(),
                        target: target.map(|(s, i)| {
                            DeclaredTarget(self.rules.topology().position(s, i).unwrap())
                        }),
                        mechanic: None,
                    },
                })
                .collect(),
        }
    }

    fn turn(&mut self, p1: &[Decl], p2: &[Decl], tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)];
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

    fn has(&self, from: usize, pred: impl Fn(&TraceEvent) -> bool) -> bool {
        self.events(from).iter().any(pred)
    }

    fn blocked(&self, from: usize, target: PokemonId, reason: HitBlockReason) -> bool {
        self.has(from, |e| {
            matches!(
                e,
                TraceEvent::HitBlocked { target: t, reason: r, .. } if *t == target && *r == reason
            )
        })
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn bulky(species: &str, moves: &[&str]) -> PokemonSet {
    set(species, moves, "Run Away", None, 100)
}

#[test]
fn guards_rage_powder_powder_and_absorbers_classify_from_snapshot_data() {
    let dex = DexResolver::resolve(Generation::Gen9);
    for name in ["Wide Guard", "Quick Guard", "Rage Powder"] {
        assert_eq!(
            dex.move_by_name(name).unwrap().primary_effect,
            PrimaryEffect::Custom,
            "{name}"
        );
    }
    assert_eq!(
        dex.move_by_name("Fake Out").unwrap().primary_effect,
        PrimaryEffect::Damage
    );
    for name in ["Sleep Powder", "Stun Spore", "Poison Powder"] {
        let mv = dex.move_by_name(name).unwrap();
        assert!(mv.flags.powder, "{name}");
        assert!(
            matches!(
                mv.primary_effect,
                PrimaryEffect::Status(StatusMoveEffect::SetStatus(_))
            ),
            "{name}"
        );
    }
    for name in ["Water Absorb", "Volt Absorb", "Overcoat"] {
        assert!(dex.ability_by_name(name).unwrap().implemented, "{name}");
    }
    assert!(dex.item_by_name("Safety Goggles").unwrap().implemented);
}

#[test]
fn wide_guard_stops_spread_moves_on_its_side_and_advances_the_protect_counter() {
    let mut arena = doubles(
        Generation::Gen9,
        [
            set(
                "Hitmontop",
                &["Wide Guard", "Protect"],
                "Run Away",
                None,
                50,
            ),
            bulky("Snorlax", &["Swords Dance"]),
        ],
        [
            set("Garchomp", &["Earthquake"], "Run Away", None, 50),
            bulky("Tyranitar", &["Rock Slide"]),
        ],
        |_| {},
    );
    // Rock Slide (90 %) and Earthquake are refused on P1's side before any decision; the
    // Earthquake still hits Garchomp's own partner.
    let start = arena.turn(
        &[(P1A, 0, None), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        hit(100),
    );
    for target in [P1A, P1B] {
        assert!(arena.blocked(start, target, HitBlockReason::WideGuard));
    }
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DamageApplied { target: P2B, .. }
    )));
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::DamageApplied {
            target: P1A | P1B,
            ..
        }
    )));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::SideConditionEnded {
            side: SideId::P1,
            kind: SideConditionKind::WideGuard
        }
    )));
    // No roll for the guard itself, but the counter it started makes the next Protect roll.
    assert_eq!(arena.pokemon(P1A).volatiles.stall_counter(), Some(3));
    let start = arena.turn(
        &[(P1A, 1, None), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        vec![RandomOverride::bool(
            RandomRequestKind::ProtectSuccess,
            false,
        )],
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ProtectChecked {
            actor: P1A,
            counter: 3,
            success: false
        }
    )));
}

#[test]
fn quick_guard_stops_moves_with_positive_priority() {
    let mut arena = doubles(
        Generation::Gen9,
        [
            set("Hitmontop", &["Quick Guard"], "Run Away", None, 50),
            bulky("Snorlax", &["Swords Dance"]),
        ],
        [
            set(
                "Garchomp",
                &["Aqua Jet", "Dragon Claw"],
                "Run Away",
                None,
                50,
            ),
            bulky("Tyranitar", &["Swords Dance"]),
        ],
        |_| {},
    );
    let start = arena.turn(
        &[(P1A, 0, None), (P1B, 0, None)],
        &[(P2A, 0, Some((SideId::P1, 1))), (P2B, 0, None)],
        Vec::new(),
    );
    assert!(arena.blocked(start, P1B, HitBlockReason::QuickGuard));
    // A priority-0 move goes through the guard of the same kind next turn? The guard is gone:
    // Dragon Claw simply hits.
    let start = arena.turn(
        &[(P1A, 0, None), (P1B, 0, None)],
        &[(P2A, 1, Some((SideId::P1, 1))), (P2B, 0, None)],
        hit(100),
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DamageApplied { target: P1B, .. }
    )));
}

#[test]
fn powder_moves_skip_grass_overcoat_and_goggles_and_rage_powder_ignores_them() {
    let mut arena = doubles(
        Generation::Gen9,
        [
            bulky("Amoonguss", &["Rage Powder", "Spore"]),
            set("Snorlax", &["Swords Dance"], "Run Away", None, 90),
        ],
        [
            set(
                "Garchomp",
                &["Dragon Claw"],
                "Run Away",
                Some("Safety Goggles"),
                50,
            ),
            set("Venusaur", &["Body Slam"], "Run Away", None, 50),
        ],
        |_| {},
    );
    // Rage Powder: neither the Goggles holder nor the Grass type is pulled; both hit Snorlax.
    let mut tape = hit(100);
    tape.extend(hit(100));
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    let start = arena.turn(
        &[(P1A, 0, None), (P1B, 0, None)],
        &[
            (P2A, 0, Some((SideId::P1, 1))),
            (P2B, 0, Some((SideId::P1, 1))),
        ],
        tape,
    );
    assert!(!arena.has(start, |e| matches!(e, TraceEvent::TargetRedirected { .. })));
    // Spore: refused by the Grass type (no accuracy check follows) and by the Goggles.
    let start = arena.turn(
        &[(P1A, 1, Some((SideId::P2, 1))), (P1B, 0, None)],
        &[
            (P2A, 0, Some((SideId::P1, 1))),
            (P2B, 0, Some((SideId::P1, 1))),
        ],
        {
            let mut tape = hit(100);
            tape.extend(hit(100));
            tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
            tape
        },
    );
    assert!(arena.blocked(start, P2B, HitBlockReason::Powder));
    let start = arena.turn(
        &[(P1A, 1, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[
            (P2A, 0, Some((SideId::P1, 1))),
            (P2B, 0, Some((SideId::P1, 1))),
        ],
        {
            let mut tape = hit(100);
            tape.extend(hit(100));
            tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
            tape
        },
    );
    assert!(arena.blocked(start, P2A, HitBlockReason::Item(ItemId::SAFETY_GOGGLES)));

    // A plain attacker (Blissey) is pulled by Rage Powder; Garchomp with Overcoat is not, and
    // Overcoat refuses Spore.
    let mut arena = doubles(
        Generation::Gen9,
        [
            bulky("Amoonguss", &["Rage Powder", "Spore"]),
            set("Snorlax", &["Swords Dance"], "Run Away", None, 90),
        ],
        [
            set("Garchomp", &["Dragon Claw"], "Overcoat", None, 50),
            bulky("Blissey", &["Tackle"]),
        ],
        |_| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(
        &[(P1A, 0, None), (P1B, 0, None)],
        &[
            (P2A, 0, Some((SideId::P1, 1))),
            (P2B, 0, Some((SideId::P1, 1))),
        ],
        tape,
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::TargetRedirected {
            actor: P2B,
            to: P1A,
            by: EffectSource::Volatile(VolatileConditionKind::RagePowder),
            ..
        }
    )));
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::TargetRedirected { actor: P2A, .. }
    )));
    let start = arena.turn(
        &[(P1A, 1, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[
            (P2A, 0, Some((SideId::P1, 1))),
            (P2B, 0, Some((SideId::P1, 1))),
        ],
        {
            let mut tape = hit(100);
            tape.extend(hit(100));
            tape
        },
    );
    assert!(arena.blocked(start, P2A, HitBlockReason::Ability(AbilityId::OVERCOAT)));

    // Gen 5: no powder immunity for Grass types.
    let gen5 = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen5,
        BattleMode::Doubles,
        "custom",
    ))
    .unwrap();
    assert!(!gen5.conditions().powder_immunity);
}

#[test]
fn fake_out_works_on_the_first_action_only_and_absorbers_heal_instead_of_being_hit() {
    let mut arena = doubles(
        Generation::Gen9,
        [
            set("Incineroar", &["Fake Out"], "Run Away", None, 50),
            set(
                "Rotom",
                &["Hydro Pump", "Thunderbolt"],
                "Levitate",
                None,
                50,
            ),
        ],
        [
            set("Vaporeon", &["Swords Dance"], "Water Absorb", None, 100),
            set("Lanturn", &["Swords Dance"], "Volt Absorb", None, 100),
        ],
        |pokemon| pokemon[2].hp -= 100,
    );
    // Fake Out flinches Vaporeon (certain secondary); Hydro Pump is absorbed: no accuracy roll,
    // a quarter healed.
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P2, 0))),
            (P1B, 0, Some((SideId::P2, 0))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        hit(100),
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileStarted {
            target: P2A,
            kind: VolatileConditionKind::Flinch
        }
    )));
    assert!(arena.blocked(start, P2A, HitBlockReason::Ability(AbilityId::WATER_ABSORB)));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::HealApplied {
            target: P2A,
            source: EffectSource::Ability {
                ability: AbilityId::WATER_ABSORB,
                ..
            },
            ..
        }
    )));
    // Second action: Fake Out fails (PP spent); Volt Absorb at full HP just refuses.
    let pp_before = arena.pokemon(P1A).moves[0].pp;
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P2, 1))),
            (P1B, 1, Some((SideId::P2, 1))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        Vec::new(),
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: P1A,
            reason: MoveFailReason::NoEffect,
            ..
        }
    )));
    assert_eq!(arena.pokemon(P1A).moves[0].pp, pp_before - 1);
    assert!(arena.blocked(start, P2B, HitBlockReason::Ability(AbilityId::VOLT_ABSORB)));
    assert_eq!(arena.pokemon(P2B).hp, arena.pokemon(P2B).max_hp);
}

#[test]
fn a_slot_emptied_by_a_faint_is_a_legal_choice_settled_at_execution() {
    // P1's partner is down with no replacement: Helping Hand aimed at its slot is accepted at
    // choice time (Showdown `validTargetLoc` is positional) and fails when it runs.
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Doubles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let mut fainted =
        build_pokemon(&dex, P1B, SideId::P1, &bulky("Snorlax", &["Swords Dance"])).unwrap();
    fainted.hp = 0;
    let pokemon = vec![
        build_pokemon(
            &dex,
            P1A,
            SideId::P1,
            &set("Clefable", &["Helping Hand"], "Run Away", None, 50),
        )
        .unwrap(),
        fainted,
        build_pokemon(&dex, P2A, SideId::P2, &bulky("Blissey", &["Swords Dance"])).unwrap(),
        build_pokemon(&dex, P2B, SideId::P2, &bulky("Chansey", &["Swords Dance"])).unwrap(),
    ];
    let mut p1 = SideState::new(vec![P1A, P1B], 2);
    p1.active[0] = Some(P1A);
    let mut p2 = SideState::new(vec![P2A, P2B], 2);
    p2.active = vec![Some(P2A), Some(P2B)];
    let state = BattleState::new(pokemon, [p1, p2], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let mut arena = Arena { rules, dex, engine };
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P1, 1)))],
        &[(P2A, 0, None), (P2B, 0, None)],
        Vec::new(),
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: P1A,
            reason: MoveFailReason::NoLegalTarget,
            ..
        }
    )));
}

#[test]
fn a_second_wide_guard_on_a_guarded_side_still_advances_its_users_counter() {
    // Showdown `onHitSide` (`addVolatile('stall')`) runs whether or not `addSideCondition`
    // added anything: the partner's Wide Guard fails but its user's counter moves on.
    let mut arena = doubles(
        Generation::Gen9,
        [
            bulky("Hitmontop", &["Wide Guard"]),
            bulky("Blissey", &["Wide Guard"]),
        ],
        [
            bulky("Snorlax", &["Swords Dance"]),
            bulky("Chansey", &["Swords Dance"]),
        ],
        |_| {},
    );
    arena.turn(
        &[(P1A, 0, None), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        Vec::new(),
    );
    for guard in [P1A, P1B] {
        assert!(arena.pokemon(guard).volatiles.stall_counter().is_some());
    }
}

#[test]
fn gen_5_wide_guard_rolls_the_protect_counter() {
    // Gen 5 Wide Guard is a stalling move (`onTry`: `StallMove`): right after a guard it rolls
    // 1 / counter like Protect; from Gen 6 it does not.
    for (generation, tape, guarded) in [
        (
            Generation::Gen5,
            vec![RandomOverride::bool(
                RandomRequestKind::ProtectSuccess,
                false,
            )],
            false,
        ),
        (Generation::Gen6, Vec::new(), true),
    ] {
        let mut arena = doubles(
            generation,
            [
                bulky("Hitmontop", &["Wide Guard"]),
                bulky("Blissey", &["Swords Dance"]),
            ],
            [
                bulky("Snorlax", &["Swords Dance"]),
                bulky("Chansey", &["Swords Dance"]),
            ],
            |_| {},
        );
        let p1: [Decl; 2] = [(P1A, 0, None), (P1B, 0, None)];
        let p2: [Decl; 2] = [(P2A, 0, None), (P2B, 0, None)];
        arena.turn(&p1, &p2, Vec::new());
        let start = arena.turn(&p1, &p2, tape);
        let started = arena.has(start, |e| {
            matches!(
                e,
                TraceEvent::SideConditionStarted {
                    kind: SideConditionKind::WideGuard,
                    ..
                }
            )
        });
        assert_eq!(started, guarded, "{generation:?}");
    }
}
