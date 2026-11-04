//! L1 rule tests for M4 batch 2 (ROADMAP §8.1 "Cure / Transfer", DESIGN.md §20.2): contact
//! reactions that status the attacker (Static, Flame Body, Poison Point), Synchronize passing a
//! status back, Shed Skin and Natural Cure curing, Heal Bell / Refresh, and the residual items
//! that status or chip their holder (Toxic Orb, Flame Orb, Black Sludge).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, ConditionApplyError, CureReason, EffectSource, EngineStep, MoveFailReason,
    SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, Fraction, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], ability: Option<&str>, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = ability.map(Into::into);
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

fn bench() -> PokemonSet {
    set("Vaporeon", &["Tackle"], Some("Run Away"), None)
}

/// P1 = [A, C (bench)]; P2 = [B]. `prepare` edits the built Pokémon (A, B, C in that order).
fn setup(
    generation: Generation,
    p1: PokemonSet,
    p2: PokemonSet,
    bench: PokemonSet,
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &bench).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

fn statused(status: NonVolatileStatus) -> impl FnOnce(&mut Vec<PokemonState>) {
    move |pokemon| pokemon[0].status = Some(StatusState::new(status))
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

    fn turn(&mut self, p1: Action, p2: Action, tape: Vec<RandomOverride>) -> usize {
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

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn status(&self, id: PokemonId) -> Option<NonVolatileStatus> {
        self.pokemon(id).status.map(|s| s.kind)
    }

    fn position(&self, from: usize, pred: impl Fn(&TraceEvent) -> bool) -> Option<usize> {
        self.events(from).iter().position(pred)
    }

    fn has(&self, from: usize, pred: impl Fn(&TraceEvent) -> bool) -> bool {
        self.position(from, pred).is_some()
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn contact(roll: u16, triggered: bool) -> Vec<RandomOverride> {
    let mut tape = hit(roll);
    tape.push(RandomOverride::bool(
        RandomRequestKind::ContactReaction,
        triggered,
    ));
    tape
}

#[test]
fn contact_reactions_cures_and_residual_items_classify_from_snapshot_data() {
    let gen9 = DexResolver::resolve(Generation::Gen9);
    for name in [
        "Static",
        "Flame Body",
        "Poison Point",
        "Synchronize",
        "Shed Skin",
        "Natural Cure",
        "Healer",
    ] {
        assert!(gen9.ability_by_name(name).unwrap().implemented, "{name}");
    }
    for name in ["Toxic Orb", "Flame Orb", "Black Sludge", "Sticky Barb"] {
        assert!(gen9.item_by_name(name).unwrap().implemented, "{name}");
    }
    // Protective Pads used to stand here as the example of an item whose effect lives in
    // another effect's check; it is modelled now (`engine::move_makes_contact`, DESIGN.md
    // §43.4), and no such item is left in the inventory.
    assert!(gen9.item_by_name("Protective Pads").unwrap().implemented);
    assert_eq!(
        gen9.move_by_name("Heal Bell").unwrap().primary_effect,
        PrimaryEffect::Custom
    );
    let gen4 = DexResolver::resolve(Generation::Gen4);
    for name in ["Aromatherapy", "Refresh"] {
        assert_eq!(
            gen4.move_by_name(name).unwrap().primary_effect,
            PrimaryEffect::Custom,
            "{name}"
        );
    }
}

#[test]
fn static_paralyses_the_attacker_on_a_contact_hit_with_the_generations_chance() {
    let mut arena = setup(
        Generation::Gen9,
        set("Blissey", &["Swords Dance"], Some("Static"), None),
        set("Snorlax", &["Tackle", "Gust"], Some("Run Away"), None),
        bench(),
        |_| {},
    );
    // Tackle (contact): the roll is made, fails, nothing happens.
    let start = arena.turn(Action::Move(0), Action::Move(0), contact(100, false));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ContactReactionChecked {
            holder: A,
            attacker: B,
            ability: AbilityId::STATIC,
            triggered: false,
        }
    )));
    assert_eq!(arena.status(B), None);
    // Gust (no contact): no roll at all.
    arena.turn(Action::Move(0), Action::Move(1), hit(100));
    // Tackle again, the roll succeeds: Snorlax is paralysed by Static.
    let start = arena.turn(Action::Move(0), Action::Move(0), contact(100, true));
    assert_eq!(arena.status(B), Some(NonVolatileStatus::Paralysis));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::StatusApplied {
            target: B,
            status: NonVolatileStatus::Paralysis,
            source: EffectSource::Ability {
                pokemon: A,
                ability: AbilityId::STATIC,
            },
        }
    )));
    // Already statused: the roll is still made (Showdown rolls before `trySetStatus`), the
    // status is refused. Blissey moves first; the paralysed Snorlax rolls for full paralysis
    // at its own action.
    let mut tape = vec![RandomOverride::bool(
        RandomRequestKind::FullParalysis,
        false,
    )];
    tape.extend(contact(100, true));
    let start = arena.turn(Action::Move(0), Action::Move(0), tape);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::StatusBlocked { target: B, .. }
    )));

    // The chance is a generation fact: 1/3 in Gen 3, 3/10 from Gen 4.
    let chance = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .status()
            .lifecycle
            .contact_status_chance
    };
    assert_eq!(chance(Generation::Gen3), Fraction::new(1, 3));
    assert_eq!(chance(Generation::Gen4), Fraction::new(3, 10));
    assert_eq!(chance(Generation::Gen9), Fraction::new(3, 10));

    // Flame Body burns, Poison Point poisons.
    for (ability, status) in [
        ("Flame Body", NonVolatileStatus::Burn),
        ("Poison Point", NonVolatileStatus::Poison),
    ] {
        let mut arena = setup(
            Generation::Gen9,
            set("Talonflame", &["Swords Dance"], Some(ability), None),
            set("Snorlax", &["Tackle"], Some("Run Away"), None),
            bench(),
            |_| {},
        );
        arena.turn(Action::Move(0), Action::Move(0), contact(100, true));
        assert_eq!(arena.status(B), Some(status), "{ability}");
    }
}

#[test]
fn synchronize_passes_the_status_back_and_downgrades_toxic_before_gen_5() {
    // Paralysis from Thunder Wave comes back to its user.
    let mut arena = setup(
        Generation::Gen9,
        set("Umbreon", &["Swords Dance"], Some("Synchronize"), None),
        set(
            "Snorlax",
            &["Thunder Wave", "Hypnosis"],
            Some("Run Away"),
            None,
        ),
        bench(),
        |_| {},
    );
    let start = arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)],
    );
    assert_eq!(arena.status(A), Some(NonVolatileStatus::Paralysis));
    assert_eq!(arena.status(B), Some(NonVolatileStatus::Paralysis));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::AbilityActivated {
            source: A,
            ability: AbilityId::SYNCHRONIZE
        }
    )));

    // Sleep is not passed.
    let mut arena = setup(
        Generation::Gen9,
        set("Umbreon", &["Swords Dance"], Some("Synchronize"), None),
        set("Snorlax", &["Hypnosis"], Some("Run Away"), None),
        bench(),
        |_| {},
    );
    arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![
            RandomOverride::bool(RandomRequestKind::Accuracy, true),
            RandomOverride::range(RandomRequestKind::Duration, 2),
        ],
    );
    assert_eq!(arena.status(A), Some(NonVolatileStatus::Sleep));
    assert_eq!(arena.status(B), None);

    // Bad poison is passed as is from Gen 5.
    let mut arena = setup(
        Generation::Gen9,
        set("Umbreon", &["Swords Dance"], Some("Synchronize"), None),
        set("Snorlax", &["Toxic"], Some("Run Away"), None),
        bench(),
        |_| {},
    );
    arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)],
    );
    assert_eq!(arena.status(A), Some(NonVolatileStatus::Toxic));
    assert_eq!(arena.status(B), Some(NonVolatileStatus::Toxic));
    // Gen 3-4 pass ordinary poison instead: a rule value only, since Gen 4's Toxic (the one
    // bad-poison source another Pokémon can be) carries a condition key inherited from the
    // gen7 mod that the engine does not model (DESIGN.md §36).
    let passes = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .status()
            .lifecycle
            .synchronize_passes_toxic
    };
    assert!(!passes(Generation::Gen4));
    assert!(passes(Generation::Gen5));
}

#[test]
fn shed_skin_cures_at_the_end_of_the_turn_on_its_roll_before_leftovers() {
    let mut arena = setup(
        Generation::Gen9,
        set(
            "Seviper",
            &["Swords Dance"],
            Some("Shed Skin"),
            Some("Leftovers"),
        ),
        set("Snorlax", &["Tackle"], Some("Run Away"), None),
        bench(),
        statused(NonVolatileStatus::Paralysis),
    );
    // Failed roll: still paralysed; the roll comes before Leftovers (5.3 < 5.4). The paralysed
    // Seviper (speed halved) acts after Snorlax's Tackle.
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(
        RandomRequestKind::FullParalysis,
        false,
    ));
    tape.push(RandomOverride::bool(RandomRequestKind::ShedSkin, false));
    let start = arena.turn(Action::Move(0), Action::Move(0), tape);
    assert_eq!(arena.status(A), Some(NonVolatileStatus::Paralysis));
    let shed = arena
        .position(start, |e| {
            matches!(
                e,
                TraceEvent::ShedSkinChecked {
                    holder: A,
                    triggered: false
                }
            )
        })
        .unwrap();
    let leftovers = arena
        .position(start, |e| {
            matches!(
                e,
                TraceEvent::ResidualEffect {
                    holder: A,
                    kind: ResidualKind::Leftovers,
                    ..
                }
            )
        })
        .unwrap();
    assert!(shed < leftovers);
    // Successful roll: cured.
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(
        RandomRequestKind::FullParalysis,
        false,
    ));
    tape.push(RandomOverride::bool(RandomRequestKind::ShedSkin, true));
    let start = arena.turn(Action::Move(0), Action::Move(0), tape);
    assert_eq!(arena.status(A), None);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::StatusCured {
            target: A,
            status: NonVolatileStatus::Paralysis,
            reason: CureReason::Ability(AbilityId::SHED_SKIN),
            ..
        }
    )));
    // No status: no roll.
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(!arena.has(start, |e| matches!(e, TraceEvent::ShedSkinChecked { .. })));
}

#[test]
fn natural_cure_cures_on_a_switch_but_not_on_a_faint() {
    let mut arena = setup(
        Generation::Gen9,
        set("Blissey", &["Tackle"], Some("Natural Cure"), None),
        set("Snorlax", &["Swords Dance"], Some("Run Away"), None),
        bench(),
        statused(NonVolatileStatus::Paralysis),
    );
    let start = arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    assert_eq!(arena.status(A), None);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::StatusCured {
            target: A,
            reason: CureReason::Ability(AbilityId::NATURAL_CURE),
            ..
        }
    )));

    // Knocked out while statused: no cure.
    let mut arena = setup(
        Generation::Gen9,
        set("Blissey", &["Tackle"], Some("Natural Cure"), None),
        set("Snorlax", &["Tackle"], Some("Run Away"), None),
        bench(),
        |pokemon| {
            pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Paralysis));
            pokemon[0].hp = 1;
        },
    );
    // Snorlax outspeeds the paralysed Blissey and knocks it out before it acts.
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(arena.pokemon(A).is_fainted());
    // Natural Cure does not fire; the status gives way to the faint at the end-of-turn check.
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::StatusCured {
            target: A,
            reason: CureReason::Ability(AbilityId::NATURAL_CURE),
            ..
        }
    )));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::StatusCured {
            target: A,
            reason: CureReason::Fainted,
            ..
        }
    )));
}

#[test]
fn status_orbs_run_late_and_black_sludge_at_leftovers_key() {
    let mut arena = setup(
        Generation::Gen9,
        set(
            "Breloom",
            &["Swords Dance"],
            Some("Run Away"),
            Some("Toxic Orb"),
        ),
        set(
            "Snorlax",
            &["Swords Dance"],
            Some("Run Away"),
            Some("Black Sludge"),
        ),
        set("Muk", &["Tackle"], Some("Run Away"), Some("Black Sludge")),
        |_| {},
    );
    let snorlax_max = arena.pokemon(B).max_hp;
    // Turn 1: Black Sludge (5.4) chips Snorlax 1/8; Toxic Orb (28.3) badly poisons Breloom
    // after the status residuals ran, so no poison damage yet.
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(arena.status(A), Some(NonVolatileStatus::Toxic));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::StatusApplied {
            target: A,
            status: NonVolatileStatus::Toxic,
            source: EffectSource::Item {
                item: ItemId::TOXIC_ORB,
                ..
            },
        }
    )));
    assert_eq!(arena.pokemon(A).hp, arena.pokemon(A).max_hp);
    assert_eq!(arena.pokemon(B).hp, snorlax_max - snorlax_max / 8);
    let sludge = arena
        .position(start, |e| {
            matches!(
                e,
                TraceEvent::ResidualEffect {
                    holder: B,
                    kind: ResidualKind::BlackSludge,
                    ..
                }
            )
        })
        .unwrap();
    let orb = arena
        .position(start, |e| {
            matches!(
                e,
                TraceEvent::ResidualEffect {
                    holder: A,
                    kind: ResidualKind::StatusOrb,
                    ..
                }
            )
        })
        .unwrap();
    assert!(sludge < orb);
    // Turn 2: the first toxic chip is 1/16 (the counter starts at the first residual).
    let breloom_max = arena.pokemon(A).max_hp;
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(arena.pokemon(A).hp, breloom_max - breloom_max / 16);
    // Muk (Poison type) is healed 1/16 by Black Sludge.
    let mut arena = setup(
        Generation::Gen9,
        set(
            "Muk",
            &["Swords Dance"],
            Some("Run Away"),
            Some("Black Sludge"),
        ),
        set("Snorlax", &["Tackle"], Some("Run Away"), None),
        bench(),
        |_| {},
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::HealApplied {
            target: A,
            source: EffectSource::Item {
                item: ItemId::BLACK_SLUDGE,
                ..
            },
            ..
        }
    )));
    // Flame Orb burns.
    let mut arena = setup(
        Generation::Gen9,
        set(
            "Breloom",
            &["Swords Dance"],
            Some("Run Away"),
            Some("Flame Orb"),
        ),
        set("Snorlax", &["Swords Dance"], Some("Run Away"), None),
        bench(),
        |_| {},
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(arena.status(A), Some(NonVolatileStatus::Burn));
}

#[test]
fn heal_bell_cures_the_whole_team_and_refresh_the_user() {
    let mut arena = setup(
        Generation::Gen9,
        set("Blissey", &["Heal Bell"], Some("Run Away"), None),
        set("Snorlax", &["Swords Dance"], Some("Run Away"), None),
        bench(),
        |pokemon| {
            pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Paralysis));
            pokemon[2].status = Some(StatusState::new(NonVolatileStatus::Poison));
        },
    );
    let start = arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(
            RandomRequestKind::FullParalysis,
            false,
        )],
    );
    assert_eq!(arena.status(A), None);
    assert_eq!(arena.status(C), None);
    assert_eq!(
        arena
            .events(start)
            .iter()
            .filter(|e| matches!(e, TraceEvent::StatusCured { .. }))
            .count(),
        2
    );
    // Nobody statused: the move fails.
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: A,
            reason: MoveFailReason::Condition(ConditionApplyError::AlreadyActive),
            ..
        }
    )));

    // Refresh (Gen 4): cures the user's own burn; nothing to cure fails.
    let mut arena = setup(
        Generation::Gen4,
        set("Umbreon", &["Refresh"], Some("Run Away"), None),
        set("Snorlax", &["Swords Dance"], Some("Run Away"), None),
        bench(),
        statused(NonVolatileStatus::Burn),
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(arena.status(A), None);
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: A,
            reason: MoveFailReason::Condition(ConditionApplyError::AlreadyActive),
            ..
        }
    )));
}
