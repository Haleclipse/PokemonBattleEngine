//! L1 rule tests for the reactions to a damaging hit that roll a chance after the ordered ones
//! (DESIGN.md §21, R92): Effect Spore, Cursed Body, Poison Touch, Toxic Chain, and Shield Dust
//! / Covert Cloak dropping target-directed secondaries. The Showdown replay plans these
//! decisions by asking the same engine predicates, so whether a roll is drawn at all — and its
//! chance — is invisible to it; a semantic tape pins each draw here (an unplanned draw hits the
//! next planned decision of another kind and desynchronizes).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EffectSource, EngineStep, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, EffectSporeRoll, Fraction, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, VolatileConditionKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

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

/// P1 = [A]; P2 = [B]. `prepare` edits the built Pokémon (A, B).
fn setup(
    generation: Generation,
    p1: PokemonSet,
    p2: PokemonSet,
    prepare: impl FnOnce(&mut [PokemonState]),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
    ];
    prepare(&mut pokemon);
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

    /// One turn, each side using the move in the given slot, on a semantic tape that must be
    /// used up exactly.
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

    fn events(&self, from: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[from..]
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn status(&self, id: PokemonId) -> Option<NonVolatileStatus> {
        self.pokemon(id).status.map(|s| s.kind)
    }
}

fn hit() -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ]
}

fn tape(parts: &[&[RandomOverride]]) -> Vec<RandomOverride> {
    parts.concat()
}

fn chance(value: bool) -> RandomOverride {
    RandomOverride::bool(RandomRequestKind::HitReaction, value)
}

#[test]
fn effect_spore_draws_by_generation() {
    let rules = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .hits()
            .effect_spore
    };
    assert_eq!(
        rules(Generation::Gen3),
        EffectSporeRoll::ChanceThenSample(Fraction::new(1, 10))
    );
    assert_eq!(
        rules(Generation::Gen4),
        EffectSporeRoll::ChanceThenSample(Fraction::new(3, 10))
    );
    assert_eq!(rules(Generation::Gen5), EffectSporeRoll::Percent);
    assert_eq!(rules(Generation::Gen9), EffectSporeRoll::Percent);
    // Gen 5+ `random(100)`: below 11 sleep, below 21 paralysis, below 30 poison.
    use pokemon_battle_engine::engine::effect_spore_status;
    for (roll, status) in [
        (0, Some(NonVolatileStatus::Sleep)),
        (10, Some(NonVolatileStatus::Sleep)),
        (11, Some(NonVolatileStatus::Paralysis)),
        (20, Some(NonVolatileStatus::Paralysis)),
        (21, Some(NonVolatileStatus::Poison)),
        (29, Some(NonVolatileStatus::Poison)),
        (30, None),
        (99, None),
    ] {
        assert_eq!(effect_spore_status(roll), status, "{roll}");
    }
    // Gen 3-4 `sample(['slp', 'par', 'psn'])`.
    assert_eq!(
        pokemon_battle_engine::engine::EFFECT_SPORE_SAMPLE,
        [
            NonVolatileStatus::Sleep,
            NonVolatileStatus::Paralysis,
            NonVolatileStatus::Poison
        ]
    );
}

#[test]
fn effect_spore_rolls_on_contact_against_an_attacker_not_immune_to_powder() {
    // Gen 9: Tackle draws the roll (15: paralysis), Swift does not; the foe's Tackle comes next.
    let mut arena = setup(
        Generation::Gen9,
        set("Mew", &["Tackle", "Swift"], Some("Run Away"), None),
        set("Snorlax", &["Tackle"], Some("Effect Spore"), None),
        |_| {},
    );
    let spore = RandomOverride::range(RandomRequestKind::EffectSpore, 15);
    let start = arena.turn(0, 0, tape(&[&hit(), &[spore], &hit()]));
    assert_eq!(arena.status(A), Some(NonVolatileStatus::Paralysis));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::StatusApplied {
            target: A,
            source: EffectSource::Ability {
                pokemon: B,
                ability: AbilityId::EFFECT_SPORE
            },
            ..
        }
    )));
    let full_paralysis = RandomOverride::bool(RandomRequestKind::FullParalysis, false);
    arena.turn(1, 0, tape(&[&[full_paralysis], &hit(), &hit()]));

    // A Grass type is immune to powder from Gen 6: nothing drawn. Gen 5 draws.
    let grass = |generation| {
        setup(
            generation,
            set("Leafeon", &["Tackle"], Some("Run Away"), None),
            set("Snorlax", &["Tackle"], Some("Effect Spore"), None),
            |_| {},
        )
    };
    grass(Generation::Gen9).turn(0, 0, tape(&[&hit(), &hit()]));
    let mut gen5 = grass(Generation::Gen5);
    let poison = RandomOverride::range(RandomRequestKind::EffectSpore, 25);
    gen5.turn(0, 0, tape(&[&hit(), &[poison], &hit()]));
    assert_eq!(gen5.status(A), Some(NonVolatileStatus::Poison));
}

#[test]
fn gen4_effect_spore_draws_a_chance_then_a_sample() {
    let mut arena = setup(
        Generation::Gen4,
        set("Mew", &["Pound"], Some("Run Away"), None),
        set("Snorlax", &["Pound"], Some("Effect Spore"), None),
        |_| {},
    );
    // A failed chance draws no sample.
    arena.turn(0, 0, tape(&[&hit(), &[chance(false)], &hit()]));
    assert_eq!(arena.status(A), None);
    let sample = RandomOverride::range(RandomRequestKind::Sample, 1);
    arena.turn(0, 0, tape(&[&hit(), &[chance(true), sample], &hit()]));
    assert_eq!(arena.status(A), Some(NonVolatileStatus::Paralysis));
}

#[test]
fn cursed_body_rolls_until_the_move_is_disabled_and_never_for_struggle() {
    assert_eq!(
        pokemon_battle_engine::engine::HIT_REACTION_CHANCE,
        Fraction::new(3, 10)
    );
    let mut arena = setup(
        Generation::Gen9,
        set("Mew", &["Tackle", "Swift"], Some("Run Away"), None),
        set("Snorlax", &["Tackle"], Some("Cursed Body"), None),
        |_| {},
    );
    arena.turn(0, 0, tape(&[&hit(), &[chance(true)], &hit()]));
    let mew = arena.pokemon(A);
    assert_eq!(mew.volatiles.disabled_move(), Some(MoveId::TACKLE));
    // Five turns, one short for the Pokémon now moving; the end of this turn took one.
    assert_eq!(
        mew.volatiles.turns_left(VolatileConditionKind::Disable),
        Some(3)
    );
    // While it lasts, Swift draws no roll.
    arena.turn(1, 0, tape(&[&hit(), &hit()]));

    // Struggle never rolls it.
    let mut arena = setup(
        Generation::Gen9,
        set("Mew", &["Tackle"], Some("Run Away"), None),
        set("Snorlax", &["Tackle"], Some("Cursed Body"), None),
        |pokemon| pokemon[0].moves[0].pp = 0,
    );
    arena.turn(0, 0, tape(&[&hit(), &hit()]));
    assert_eq!(arena.pokemon(A).volatiles.disabled_move(), None);
}

#[test]
fn cursed_body_draws_nothing_for_a_max_move_or_a_foreseen_strike() {
    // Showdown `cursedbody.onDamagingHit`: `!move.isMax && !move.flags['futuremove']`. A
    // foreseen strike runs Doom Desire's own data (`future_strike`); the replay pins that one
    // (`gen7-cursed-body-ignores-a-doom-desire-strike`), a Max Move's draw would show nowhere
    // in a frame (R-M8-144).
    let arena = setup(
        Generation::Gen8,
        set("Mew", &["Tackle"], Some("Run Away"), None),
        set("Snorlax", &["Tackle"], Some("Cursed Body"), None),
        |_| {},
    );
    let state = arena.engine.state();
    let reactors = pokemon_battle_engine::engine::chance_reactors(state, &arena.rules, A, B);
    let rolls = |name: &str| {
        let def = arena.dex.move_by_name(name).expect(name);
        pokemon_battle_engine::engine::cursed_body_rolls(state, reactors, A, def)
    };
    assert!(rolls("Tackle"));
    assert!(!rolls("Max Strike"));
    assert!(!rolls("Doom Desire"));
}

#[test]
fn poison_touch_and_toxic_chain_spare_a_target_that_shields_secondaries() {
    // Poison Touch: a contact hit (Tackle) rolls, Swift does not.
    let mut arena = setup(
        Generation::Gen9,
        set("Mew", &["Tackle", "Swift"], Some("Poison Touch"), None),
        set("Snorlax", &["Tackle"], Some("Thick Fat"), None),
        |_| {},
    );
    arena.turn(1, 0, tape(&[&hit(), &hit()]));
    let start = arena.turn(0, 0, tape(&[&hit(), &[chance(true)], &hit()]));
    assert_eq!(arena.status(B), Some(NonVolatileStatus::Poison));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::StatusApplied {
            target: B,
            source: EffectSource::Ability {
                pokemon: A,
                ability: AbilityId::POISON_TOUCH
            },
            ..
        }
    )));
    // Toxic Chain: any hit rolls.
    let mut arena = setup(
        Generation::Gen9,
        set("Mew", &["Swift"], Some("Toxic Chain"), None),
        set("Snorlax", &["Tackle"], Some("Thick Fat"), None),
        |_| {},
    );
    arena.turn(0, 0, tape(&[&hit(), &[chance(true)], &hit()]));
    assert_eq!(arena.status(B), Some(NonVolatileStatus::Toxic));
    // Shield Dust and Covert Cloak: nothing rolls.
    for (ability, item) in [(Some("Shield Dust"), None), (None, Some("Covert Cloak"))] {
        for (user_ability, moves) in [("Poison Touch", ["Tackle"]), ("Toxic Chain", ["Swift"])] {
            let mut arena = setup(
                Generation::Gen9,
                set("Mew", &moves, Some(user_ability), None),
                set("Snorlax", &["Tackle"], ability, item),
                |_| {},
            );
            arena.turn(0, 0, tape(&[&hit(), &hit()]));
        }
    }
}

#[test]
fn shield_dust_drops_target_secondaries_before_their_roll() {
    // Body Slam's 30 % paralysis is not rolled; Metal Claw's 10 % Attack boost on its user is.
    for (ability, item) in [(Some("Shield Dust"), None), (None, Some("Covert Cloak"))] {
        let mut arena = setup(
            Generation::Gen9,
            set("Mew", &["Body Slam", "Metal Claw"], Some("Run Away"), None),
            set("Snorlax", &["Tackle"], ability, item),
            |_| {},
        );
        arena.turn(0, 0, tape(&[&hit(), &hit()]));
        let accuracy = RandomOverride::bool(RandomRequestKind::Accuracy, true);
        let boost = RandomOverride::bool(RandomRequestKind::Secondary, true);
        arena.turn(1, 0, tape(&[&[accuracy], &hit(), &[boost], &hit()]));
        assert_eq!(arena.pokemon(A).stages.attack, 1);
    }
}
