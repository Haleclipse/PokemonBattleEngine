//! L1 rule tests for genders and infatuation (DESIGN.md §20.2, R102): a set's gender, Cute
//! Charm's roll before the genders are checked, and Attract's immunity. The Showdown replay asks
//! the engine whether Cute Charm rolls (`engine::cute_charm_rolls`), so whether it draws — and
//! what a draw on a Pokémon of the same gender does — is invisible to it; a semantic tape pins
//! it here.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, HitBlockReason, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, Gender, PokemonState, RandomOverride, RandomRequestKind, SideState,
    VolatileConditionKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], ability: &str, gender: Option<Gender>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.level = 50;
    set.gender = gender;
    set
}

/// Clefable moves before a Snorlax holder, whose own move then follows any reaction roll in
/// the tape (an unplanned roll would meet its critical-hit decision).
fn attacker(moves: &[&str], gender: Option<Gender>) -> PokemonSet {
    set("Clefable", moves, "Run Away", gender)
}

fn setup(generation: Generation, mut p1: PokemonSet, mut p2: PokemonSet) -> Arena {
    // No abilities before Gen 3.
    if generation <= Generation::Gen2 {
        p1.ability = None;
        p2.ability = None;
    }
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(A), side(B)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn choice(&self, side: SideId, slot: usize) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
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
        SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot as u8).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        }
    }

    fn turn(&mut self, p1: usize, p2: usize, tape: Vec<RandomOverride>) -> usize {
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

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn attracted(&self, id: PokemonId) -> bool {
        self.pokemon(id)
            .volatiles
            .contains(VolatileConditionKind::Attract)
    }
}

fn hit() -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ]
}

fn charm(value: bool) -> RandomOverride {
    RandomOverride::bool(RandomRequestKind::HitReaction, value)
}

#[test]
fn a_set_names_the_gender_and_none_is_genderless() {
    let dex = DexResolver::resolve(Generation::Gen9);
    let build = |gender| {
        build_pokemon(
            &dex,
            A,
            SideId::P1,
            &set("Snorlax", &["Tackle"], "Run Away", gender),
        )
        .unwrap()
        .gender
    };
    assert_eq!(build(Some(Gender::Female)), Gender::Female);
    assert_eq!(build(Some(Gender::Male)), Gender::Male);
    assert_eq!(build(None), Gender::Genderless);
    assert!(Gender::Male.opposite(Gender::Female));
    assert!(!Gender::Male.opposite(Gender::Male));
    assert!(!Gender::Genderless.opposite(Gender::Female));
}

/// A set that names no gender takes its species' fixed one (the exported `gender`, Showdown
/// `species.gender`); a species with a ratio stays genderless, and a named gender comes first.
#[test]
fn a_set_without_a_gender_takes_the_species_fixed_one() {
    let build = |generation, species: &str, gender| {
        let dex = DexResolver::resolve(generation);
        let mut set = PokemonSet::new(species, ["Tackle"]);
        set.gender = gender;
        build_pokemon(&dex, A, SideId::P1, &set).unwrap().gender
    };
    for generation in [Generation::Gen2, Generation::Gen4, Generation::Gen9] {
        assert_eq!(
            build(generation, "Tauros", None),
            Gender::Male,
            "{generation:?}"
        );
        assert_eq!(
            build(generation, "Chansey", None),
            Gender::Female,
            "{generation:?}"
        );
        assert_eq!(build(generation, "Magnemite", None), Gender::Genderless);
        assert_eq!(build(generation, "Snorlax", None), Gender::Genderless);
        assert_eq!(
            build(generation, "Tauros", Some(Gender::Female)),
            Gender::Female
        );
    }
    // The gen1 mod makes every species genderless.
    assert_eq!(build(Generation::Gen1, "Tauros", None), Gender::Genderless);
    let dex = DexResolver::resolve(Generation::Gen1);
    assert!(
        dex.species_by_name("Tauros")
            .is_some_and(|s| s.gender == Some(Gender::Genderless))
    );
}

#[test]
fn cute_charm_rolls_on_contact_before_the_genders_are_checked() {
    // The other gender: the roll that takes infatuates the attacker.
    let mut arena = setup(
        Generation::Gen9,
        attacker(&["Tackle", "Swift"], Some(Gender::Male)),
        set("Snorlax", &["Tackle"], "Cute Charm", Some(Gender::Female)),
    );
    arena.turn(0, 0, [hit(), vec![charm(true)], hit()].concat());
    assert!(arena.attracted(A));
    // Same gender, and genderless: the roll is still drawn and takes, the infatuation does not.
    for gender in [Some(Gender::Female), None] {
        let mut arena = setup(
            Generation::Gen9,
            attacker(&["Tackle", "Swift"], gender),
            set("Snorlax", &["Tackle"], "Cute Charm", Some(Gender::Female)),
        );
        arena.turn(0, 0, [hit(), vec![charm(true)], hit()].concat());
        assert!(!arena.attracted(A), "{gender:?}");
        // Swift does not make contact: no roll.
        arena.turn(1, 0, [hit(), hit()].concat());
    }
}

#[test]
fn attract_needs_the_other_gender() {
    for (gender, lands) in [
        (Some(Gender::Female), true),
        (Some(Gender::Male), false),
        (None, false),
    ] {
        let mut arena = setup(
            Generation::Gen9,
            set("Snorlax", &["Attract"], "Run Away", Some(Gender::Male)),
            set("Clefable", &["Harden"], "Run Away", gender),
        );
        let start = arena.turn(0, 0, Vec::new());
        assert_eq!(arena.attracted(B), lands, "{gender:?}");
        let immune = arena.engine.trace().events()[start..].iter().any(|e| {
            matches!(
                e,
                TraceEvent::HitBlocked {
                    reason: HitBlockReason::MoveRule,
                    ..
                }
            )
        });
        assert_eq!(immune, !lands, "{gender:?}");
    }
}

#[test]
fn attract_rolls_before_paralysis_except_in_gens_2_to_4() {
    use RandomRequestKind::{FullParalysis, Infatuation};
    // Gen 2's paralysis shares Attract's BeforeMove priority 2 and, as a status, goes first;
    // Gen 3-4 check paralysis before love on the cartridge (issues.md I66).
    for (generation, order) in [
        (Generation::Gen2, [FullParalysis, Infatuation]),
        (Generation::Gen3, [FullParalysis, Infatuation]),
        (Generation::Gen4, [FullParalysis, Infatuation]),
        (Generation::Gen5, [Infatuation, FullParalysis]),
        (Generation::Gen9, [Infatuation, FullParalysis]),
    ] {
        let mut arena = setup(
            generation,
            set("Clefable", &["Harden"], "Run Away", Some(Gender::Male)),
            set("Snorlax", &["Pound"], "Run Away", Some(Gender::Female)),
        );
        let mut snapshot = arena.engine.snapshot();
        let snorlax = snapshot.state.pokemon_mut(B).unwrap();
        snorlax.status = Some(pokemon_battle_engine::state::StatusState::new(
            pokemon_battle_engine::state::NonVolatileStatus::Paralysis,
        ));
        snorlax.volatiles.set_attract(A);
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
        // Gen 1-2 damage rolls run 217-255.
        let roll = if generation <= Generation::Gen2 {
            255
        } else {
            100
        };
        let tape = [
            order.map(|kind| RandomOverride::bool(kind, false)).to_vec(),
            vec![
                RandomOverride::bool(RandomRequestKind::Critical, false),
                RandomOverride::range(RandomRequestKind::DamageRoll, roll),
            ],
        ]
        .concat();
        arena.turn(0, 0, tape);
        assert!(arena.attracted(B), "{generation:?}");
    }
}
