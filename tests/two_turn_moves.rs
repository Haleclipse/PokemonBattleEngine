//! L1 rule tests for two-turn moves (ROADMAP §6.2 "Charging states", DESIGN.md §15.5): Fly,
//! Dig and Solar Beam charge for a turn inside the shared Move Shell (its TryMove stage), lock
//! the next request, hide the user from every move but the generation's listed exceptions, and
//! let those exceptions deal double damage. No move re-implements any part of the Shell.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    ChargeSkip, DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, build_pokemon,
};
use pokemon_battle_engine::engine::two_turn::ChargeSkipCause;
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, BeforeMoveBlockReason, ChoiceError, EngineStep, ModifierSource,
    ModifierStage, MoveFailReason, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DamageBonusStage, DeclaredTarget, ResolvedRules, RulesResolver, SemiInvulnerableKind,
    TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, Stat, StatusState, VolatileConditionKind, WeatherKind, WeatherState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set
}

/// P1 = [A, C (bench)]; P2 = [B]. `prepare` edits the built Pokémon (A, B, C in that order).
fn setup(
    generation: Generation,
    p1: PokemonSet,
    p2: PokemonSet,
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
        build_pokemon(&dex, C, SideId::P1, &set("Snorlax", &["Tackle"])).unwrap(),
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

    fn restore_with(&mut self, edit: impl FnOnce(&mut BattleState)) {
        let mut snapshot = self.engine.snapshot();
        edit(&mut snapshot.state);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
    }

    fn try_turn(
        &mut self,
        p1: Action,
        p2: Action,
        tape: Vec<RandomOverride>,
    ) -> Result<usize, BattleError> {
        self.restore_with(|state| state.rng = BattleRng::with_semantic_tape(1, tape));
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

    /// HP damage the move of `actor` dealt, per hit.
    fn dealt_by(&self, from: usize, actor: PokemonId) -> Vec<u16> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::DamageApplied {
                    amount,
                    source: pokemon_battle_engine::engine::EffectSource::Move { pokemon, .. },
                    ..
                } if *pokemon == actor => Some(*amount),
                _ => None,
            })
            .collect()
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

#[test]
fn two_turn_moves_classify_from_snapshot_data() {
    let dex = DexResolver::resolve(Generation::Gen9);
    for (name, hides, skip, boost) in [
        (
            "Fly",
            Some(SemiInvulnerableKind::Flying),
            ChargeSkip::Never,
            None,
        ),
        (
            "Dig",
            Some(SemiInvulnerableKind::Underground),
            ChargeSkip::Never,
            None,
        ),
        ("Solar Beam", None, ChargeSkip::InSun, None),
        ("Solar Blade", None, ChargeSkip::InSun, None),
        ("Sky Attack", None, ChargeSkip::Never, None),
        ("Freeze Shock", None, ChargeSkip::Never, None),
        ("Ice Burn", None, ChargeSkip::Never, None),
        (
            "Meteor Beam",
            None,
            ChargeSkip::Never,
            Some((Stat::SpecialAttack, 1)),
        ),
        (
            "Electro Shot",
            None,
            ChargeSkip::InRain,
            Some((Stat::SpecialAttack, 1)),
        ),
        (
            "Bounce",
            Some(SemiInvulnerableKind::Flying),
            ChargeSkip::Never,
            None,
        ),
        (
            "Dive",
            Some(SemiInvulnerableKind::Underwater),
            ChargeSkip::Never,
            None,
        ),
        (
            "Phantom Force",
            Some(SemiInvulnerableKind::Vanished),
            ChargeSkip::Never,
            None,
        ),
        (
            "Shadow Force",
            Some(SemiInvulnerableKind::Vanished),
            ChargeSkip::Never,
            None,
        ),
    ] {
        let resolved = dex.move_by_name(name).unwrap();
        assert_eq!(
            resolved.primary_effect,
            PrimaryEffect::Damage,
            "{name}: {:?}",
            resolved.unsupported
        );
        let spec = resolved.behavior.two_turn.expect(name);
        assert_eq!(spec.invulnerable, hides, "{name}");
        assert_eq!(spec.skip, skip, "{name}");
        assert_eq!(spec.charge_boost, boost, "{name}");
    }
    // Skull Bash (no longer in Gen 9) raises Defense as it charges; the snapshot's
    // `hasSheerForceBoost` reaches Electro Shot's flags.
    let gen8 = DexResolver::resolve(Generation::Gen8);
    let skull_bash = gen8.move_by_name("Skull Bash").unwrap();
    assert_eq!(skull_bash.primary_effect, PrimaryEffect::Damage);
    assert_eq!(
        skull_bash.behavior.two_turn.unwrap().charge_boost,
        Some((Stat::Defense, 1))
    );
    assert!(
        dex.move_by_name("Electro Shot")
            .unwrap()
            .flags
            .sheer_force_boost
    );
    assert!(
        !dex.move_by_name("Meteor Beam")
            .unwrap()
            .flags
            .sheer_force_boost
    );
    // The hiding place's doubling stage is the condition's: `onSourceBasePower` (Bounce) is the
    // power, `onSourceModifyDamage` (Fly, Dig, Dive) the final damage.
    for (name, stage) in [
        ("Bounce", DamageBonusStage::BasePower),
        ("Fly", DamageBonusStage::FinalDamage),
        ("Dig", DamageBonusStage::FinalDamage),
        ("Dive", DamageBonusStage::FinalDamage),
    ] {
        let resolved = dex.move_by_name(name).unwrap();
        assert_eq!(
            resolved.behavior.two_turn.unwrap().bonus_stage,
            stage,
            "{name}"
        );
    }
    // Sky Drop charges in its own `onTryHit` (`MoveBehavior::sky_drop`, DESIGN.md §15.23), not
    // through a `TwoTurnSpec`.
    let sky_drop = DexResolver::resolve(Generation::Gen7)
        .move_by_name("Sky Drop")
        .cloned()
        .unwrap();
    assert_eq!(sky_drop.primary_effect, PrimaryEffect::Damage);
    assert!(
        sky_drop.unsupported.is_empty(),
        "{:?}",
        sky_drop.unsupported
    );
    assert!(sky_drop.behavior.sky_drop && sky_drop.behavior.two_turn.is_none());
    assert_eq!(
        DexResolver::resolve(Generation::Gen4)
            .move_by_name("Fly")
            .unwrap()
            .primary_effect,
        PrimaryEffect::Damage
    );
}

#[test]
fn fly_charges_for_a_turn_locks_the_request_and_spends_pp_once() {
    let mut arena = setup(
        Generation::Gen9,
        set("Talonflame", &["Fly", "Tackle"]),
        set("Snorlax", &["Swords Dance"]),
        |_| {},
    );
    // 15 PP with Showdown's three PP Ups.
    let fly = arena.pokemon(A).moves[0];
    assert_eq!(fly.pp, 24);

    // Turn 1: the charge. PP is paid now, nothing is rolled, the foe is untouched.
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ChargeStarted {
            actor: A,
            move_id: MoveId::FLY
        }
    )));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileStarted {
            target: A,
            kind: VolatileConditionKind::TwoTurnMove
        }
    )));
    assert_eq!(arena.pokemon(A).moves[0].pp, 23);
    assert!(arena.dealt_by(start, A).is_empty());
    let volatiles = &arena.pokemon(A).volatiles;
    assert_eq!(volatiles.charging_move(), Some(MoveId::FLY));
    assert_eq!(arena.pokemon(A).locked_move(), Some(MoveId::FLY));
    // Showdown's `twoturnmove` duration 2, ticked once at the end of the turn.
    assert_eq!(
        volatiles.turns_left(VolatileConditionKind::TwoTurnMove),
        Some(1)
    );

    // The request is locked: no other move, no switch (Showdown `trapped`).
    assert!(matches!(
        arena.try_turn(Action::Move(1), Action::Move(0), Vec::new()),
        Err(BattleError::Choice(ChoiceError::MoveDisabled {
            actor: A,
            by: VolatileConditionKind::TwoTurnMove,
            ..
        }))
    ));
    assert!(matches!(
        arena.try_turn(Action::Switch(C), Action::Move(0), Vec::new()),
        Err(BattleError::Choice(ChoiceError::Trapped {
            actor: A,
            by: VolatileConditionKind::TwoTurnMove,
        }))
    ));

    // Turn 2: the attack. 95 % accuracy is rolled now, no PP is spent, the lock ends with the
    // turn.
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    let start = arena.turn(Action::Move(0), Action::Move(0), tape);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ChargeReleased {
            actor: A,
            move_id: MoveId::FLY
        }
    )));
    assert!(!arena.has(start, |e| matches!(e, TraceEvent::PpSpent { actor: A, .. })));
    assert_eq!(arena.pokemon(A).moves[0].pp, 23);
    assert_eq!(arena.dealt_by(start, A).len(), 1);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileEnded {
            target: A,
            kind: VolatileConditionKind::TwoTurnMove,
            ..
        }
    )));
    assert!(arena.pokemon(A).locked_move().is_none());
    assert!(
        arena
            .try_turn(Action::Move(1), Action::Move(0), hit(100))
            .is_ok()
    );
}

#[test]
fn a_hiding_pokemon_is_missed_except_by_the_listed_moves_which_deal_double() {
    // Reference: Gust and Earthquake against a user that is not hiding.
    let mut reference = setup(
        Generation::Gen9,
        set("Talonflame", &["Tackle"]),
        set("Snorlax", &["Gust"]),
        |_| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = reference.turn(Action::Move(0), Action::Move(0), tape);
    let gust_on_ground = reference.dealt_by(start, B)[0];

    // In the air: Tackle is missed with no accuracy roll at all; Gust reaches and doubles.
    let mut arena = setup(
        Generation::Gen9,
        set("Talonflame", &["Fly"]),
        set("Snorlax", &["Tackle", "Gust"]),
        |_| {},
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::SemiInvulnerableDodged {
            actor: B,
            target: A,
            move_id: MoveId::TACKLE,
            kind: SemiInvulnerableKind::Flying,
        }
    )));
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::AccuracyChecked { actor: B, .. }
    )));
    assert_eq!(arena.pokemon(A).hp, arena.pokemon(A).max_hp);
    // Come down (hit), then go up again while Gust strikes.
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    tape.extend(hit(100));
    arena.turn(Action::Move(0), Action::Move(0), tape);
    let start = arena.turn(Action::Move(0), Action::Move(1), hit(100));
    assert_eq!(arena.dealt_by(start, B), vec![gust_on_ground * 2]);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ModifierApplied {
            stage: ModifierStage::FinalDamage,
            source: ModifierSource::Rule("ModifyDamage"),
            ..
        } if matches!(e, TraceEvent::ModifierApplied { from, to, .. } if *to == *from * 2)
    )));

    // Underground: Earthquake reaches and doubles.
    let mut reference = setup(
        Generation::Gen9,
        set("Garchomp", &["Tackle"]),
        set("Snorlax", &["Earthquake"]),
        |_| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = reference.turn(Action::Move(0), Action::Move(0), tape);
    let quake_on_ground = reference.dealt_by(start, B)[0];
    let mut arena = setup(
        Generation::Gen9,
        set("Garchomp", &["Dig"]),
        set("Snorlax", &["Tackle", "Earthquake"]),
        |_| {},
    );
    let start = arena.turn(Action::Move(0), Action::Move(1), hit(100));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ChargeStarted {
            actor: A,
            move_id: MoveId::DIG
        }
    )));
    assert_eq!(arena.dealt_by(start, B), vec![quake_on_ground * 2]);
}

#[test]
fn solar_beam_skips_the_charge_under_sun_and_is_halved_in_rain() {
    let mut arena = setup(
        Generation::Gen9,
        set("Venusaur", &["Solar Beam", "Sunny Day"]),
        set("Snorlax", &["Swords Dance"]),
        |_| {},
    );
    // No sun: charge, then attack.
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(
        arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::TwoTurnMove)
    );
    assert!(arena.dealt_by(start, A).is_empty());
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    let charged = arena.dealt_by(start, A);
    assert_eq!(charged.len(), 1);

    // Sun: `-prepare` is still announced, but the attack goes out at once, with no lock.
    arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ChargeSkipped {
            actor: A,
            move_id: MoveId::SOLAR_BEAM,
            cause: ChargeSkipCause::Weather(WeatherKind::Sun),
        }
    )));
    assert!(
        !arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::TwoTurnMove)
    );
    assert_eq!(arena.dealt_by(start, A), charged);

    // Rain: the charge is back and the power is halved (Showdown `onBasePower`, floor).
    arena.restore_with(|state| {
        state.field.weather = Some(WeatherState {
            kind: WeatherKind::Rain,
            source: B,
            turns_remaining: 5,
            strong: false,
        });
    });
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(
        arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::TwoTurnMove)
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ModifierApplied {
            stage: ModifierStage::BasePower,
            source: ModifierSource::Move(MoveId::SOLAR_BEAM),
            from: 120,
            to: 60,
            ..
        }
    )));
}

#[test]
fn a_before_move_block_aborts_the_charge() {
    let mut arena = setup(
        Generation::Gen9,
        set("Talonflame", &["Fly"]),
        set("Snorlax", &["Tackle"]),
        |pokemon| pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Paralysis)),
    );
    // Charging turn: not fully paralysed; the foe's Tackle is dodged.
    let start = arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(
            RandomRequestKind::FullParalysis,
            false,
        )],
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ChargeStarted { actor: A, .. }
    )));
    assert_eq!(arena.pokemon(A).moves[0].pp, 23);

    // Attacking turn: fully paralysed. The two-turn move is dropped (Showdown `MoveAborted`),
    // the user is back within reach and the foe's Tackle lands.
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::FullParalysis, true)];
    tape.extend(hit(100));
    let start = arena.turn(Action::Move(0), Action::Move(0), tape);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: A,
            reason: MoveFailReason::BeforeMove(BeforeMoveBlockReason::Paralysis),
            ..
        }
    )));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileEnded {
            target: A,
            kind: VolatileConditionKind::TwoTurnMove,
            ..
        }
    )));
    assert!(arena.pokemon(A).locked_move().is_none());
    assert_eq!(arena.dealt_by(start, B).len(), 1);

    // The next Fly is a fresh charge, paid again.
    let start = arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(
            RandomRequestKind::FullParalysis,
            false,
        )],
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ChargeStarted { actor: A, .. }
    )));
    assert_eq!(arena.pokemon(A).moves[0].pp, 22);
}

#[test]
fn two_turn_rules_follow_the_generation() {
    let rules = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .two_turn()
    };
    // Gen 1: nothing reaches a hiding Pokémon (but Swift), nothing doubles.
    let gen1 = rules(Generation::Gen1);
    assert!(!gen1.bypasses(SemiInvulnerableKind::Flying, MoveId::GUST));
    assert!(gen1.bypasses(SemiInvulnerableKind::Flying, MoveId::SWIFT));
    assert!(!gen1.doubles(SemiInvulnerableKind::Underground, MoveId::EARTHQUAKE));
    // Gen 2: Whirlwind and Fissure reach; the doubling is a base-power step.
    let gen2 = rules(Generation::Gen2);
    assert!(gen2.bypasses(SemiInvulnerableKind::Flying, MoveId::WHIRLWIND));
    assert!(gen2.bypasses(SemiInvulnerableKind::Underground, MoveId::FISSURE));
    assert_eq!(
        gen2.bonus_stage(DamageBonusStage::FinalDamage),
        DamageBonusStage::BasePower
    );
    // Gen 3+: the modern lists, doubling where the hiding move's condition does it (Fly's
    // `onSourceModifyDamage`, Bounce's `onSourceBasePower`) — but Gen 4, whose cartridge doubles
    // the power (the move's `BTLVAR_POWER_MULTI`, issues.md I17).
    for (generation, fly_stage, bounce_stage) in [
        (
            Generation::Gen4,
            DamageBonusStage::BasePower,
            DamageBonusStage::BasePower,
        ),
        (
            Generation::Gen9,
            DamageBonusStage::FinalDamage,
            DamageBonusStage::BasePower,
        ),
    ] {
        let modern = rules(generation);
        for reaching in [MoveId::GUST, MoveId::THUNDER, MoveId::HURRICANE] {
            assert!(modern.bypasses(SemiInvulnerableKind::Flying, reaching));
        }
        assert!(!modern.bypasses(SemiInvulnerableKind::Flying, MoveId::WHIRLWIND));
        assert!(modern.doubles(SemiInvulnerableKind::Underground, MoveId::EARTHQUAKE));
        assert_eq!(modern.bonus_stage(DamageBonusStage::FinalDamage), fly_stage);
        assert_eq!(
            modern.bonus_stage(DamageBonusStage::BasePower),
            bounce_stage
        );
        // Under water: Surf and Whirlpool reach and double; nothing reaches a vanished Pokémon
        // but (Gen 5+) Helping Hand.
        for reaching in [MoveId::SURF, MoveId::WHIRLPOOL] {
            assert!(modern.bypasses(SemiInvulnerableKind::Underwater, reaching));
            assert!(modern.doubles(SemiInvulnerableKind::Underwater, reaching));
        }
        for missing in [MoveId::EARTHQUAKE, MoveId::GUST, MoveId::SURF] {
            assert!(!modern.bypasses(SemiInvulnerableKind::Vanished, missing));
            assert!(!modern.doubles(SemiInvulnerableKind::Vanished, missing));
        }
        assert_eq!(
            modern.bypasses(SemiInvulnerableKind::Vanished, MoveId::HELPING_HAND),
            generation >= Generation::Gen5
        );
    }
    assert!(SemiInvulnerableKind::Underwater.sheltered_from_weather());
    assert!(SemiInvulnerableKind::Underground.sheltered_from_weather());
    assert!(!SemiInvulnerableKind::Flying.sheltered_from_weather());
    assert!(!SemiInvulnerableKind::Vanished.sheltered_from_weather());

    // Gen 1-2 declare `onPrepareHit` on their two-turn moves (the Sleep Talk rule): covered
    // (`MoveBehavior.charge_fails_asleep`), the moves are supported.
    for generation in [Generation::Gen1, Generation::Gen2] {
        let fly = DexResolver::resolve(generation)
            .move_by_name("Fly")
            .unwrap()
            .clone();
        assert_eq!(fly.primary_effect, PrimaryEffect::Damage);
        assert!(fly.unsupported.is_empty(), "{generation:?}");
    }
    // Gen 1 keeps its charge its own way (`ChargeModel::Gen1`).
    assert!(gen1.gen1());
    assert!(!gen2.gen1());
}

#[test]
fn gen1_full_paralysis_ends_the_charge_but_not_the_hiding() {
    let mut slow = set("Snorlax", &["Tackle"]);
    slow.level = 5;
    let mut arena = setup(
        Generation::Gen1,
        set("Dodrio", &["Fly", "Harden"]),
        slow,
        |pokemon| pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Paralysis)),
    );
    let not_paralysed = || {
        vec![RandomOverride::bool(
            RandomRequestKind::FullParalysis,
            false,
        )]
    };
    // Charging turn: free of PP (`ChargeModel::Gen1`), hidden by a flag of its own.
    let start = arena.turn(Action::Move(0), Action::Move(0), not_paralysed());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ChargeStarted { actor: A, .. }
    )));
    assert!(
        arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::Invulnerability)
    );
    assert_eq!(arena.pokemon(A).moves[0].pp, 24);

    // Attacking turn: fully paralysed. The charge goes, the hiding stays: the foe's Tackle is
    // dodged without a draw.
    let start = arena.turn(
        Action::Move(0),
        Action::Move(0),
        vec![RandomOverride::bool(RandomRequestKind::FullParalysis, true)],
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileEnded {
            target: A,
            kind: VolatileConditionKind::TwoTurnMove,
            ..
        }
    )));
    assert!(arena.pokemon(A).locked_move().is_none());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::SemiInvulnerableDodged { target: A, .. }
    )));

    // Free again, it hardens; still out of reach of the Tackle. No PP went on Fly.
    let start = arena.turn(Action::Move(1), Action::Move(0), not_paralysed());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::SemiInvulnerableDodged { target: A, .. }
    )));
    assert_eq!(arena.pokemon(A).moves[0].pp, 24);
}
