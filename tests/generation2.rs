//! L1 rule tests for M6 batch 2 (DESIGN.md §31.2): Gen 2's historical algorithms — the byte
//! chance scale, the critical hit's stage rule, status damage right after the user's own
//! action, the `residualdmg` bad-poison counter, the end-of-turn thaw, Protect's 127/255
//! halving, one layer of Spikes, faints cutting a singles turn short (Gen 1-3), the heal's
//! rounding — and Leech Seed needing someone at the seeder's position (every generation).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, EngineStep, SecondaryRoll, SideChoice, SlotChoice, TraceEvent,
    accuracy_roll, secondary_roll,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind, SideState,
    StatusState, VolatileConditionKind,
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

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set
}

fn filler() -> PokemonSet {
    set("Snorlax", &["Swords Dance"])
}

/// Singles: P1 = [A, C], P2 = [B, D]; the leads' switch-in runs first.
fn singles(
    generation: Generation,
    [a, b, c, d]: [PokemonSet; 4],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &a).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &b).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &c).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &d).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = pokemon_battle_engine::state::BattleState::new(
        pokemon,
        [side(vec![A, C]), side(vec![B, D])],
        1,
    );
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

enum Act {
    Move(usize),
    Switch(PokemonId),
}

impl Arena {
    fn slot_choice(&self, side: SideId, act: &Act) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
        let action = match act {
            Act::Move(slot) => {
                let move_id = self.engine.state().pokemon(actor).unwrap().moves[*slot].move_id;
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
                    slot: MoveSlot::new(*slot as u8).unwrap(),
                    target,
                    mechanic: None,
                }
            }
            Act::Switch(pokemon) => ActionChoice::Switch { pokemon: *pokemon },
        };
        SideChoice {
            side,
            actions: vec![SlotChoice { actor, action }],
        }
    }

    fn turn(&mut self, p1: Act, p2: Act, tape: Vec<RandomOverride>) -> (usize, EngineStep) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [
            self.slot_choice(SideId::P1, &p1),
            self.slot_choice(SideId::P2, &p2),
        ];
        self.engine.submit_turn_choices(choices).unwrap();
        let step = loop {
            match self.engine.step().unwrap() {
                EngineStep::Progressed { .. } => {}
                other => break other,
            }
        };
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
        (start, step)
    }

    fn events(&self, from: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[from..]
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn position_of(&self, pred: impl Fn(&TraceEvent) -> bool, from: usize) -> Option<usize> {
        self.events(from).iter().position(pred)
    }
}

fn gen2_hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

#[test]
fn gen_2_chances_are_bytes_and_skip_some_secondaries() {
    let arena = singles(
        Generation::Gen2,
        [
            set(
                "Snorlax",
                &["Body Slam", "Hypnosis", "DynamicPunch", "Ice Beam"],
            ),
            set("Cloyster", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[0].stages.accuracy = 0,
    );
    let state = arena.engine.state();
    let rules = &arena.rules;
    let move_def = |slot: usize| {
        arena
            .dex
            .move_by_id(state.pokemon(A).unwrap().moves[slot].move_id)
            .unwrap()
            .clone()
    };
    // 100 % → 255 → no roll; 60 % → ⌊60 × 255 / 100⌋ = 153 out of 256.
    assert_eq!(accuracy_roll(state, rules, A, B, &move_def(0)), None);
    assert_eq!(
        accuracy_roll(state, rules, A, B, &move_def(1)),
        Some((153, 256))
    );
    // Body Slam's 30 % paralysis: 76 / 256; Dynamic Punch's declared 100 % confusion still
    // rolls (255 / 256); Ice Beam's freeze never rolls against an Ice type (Cloyster).
    let body_slam = move_def(0);
    let secondary = |m: &pokemon_battle_engine::dex::ResolvedMove| m.secondaries[0];
    assert_eq!(
        secondary_roll(state, rules, A, B, &body_slam, &secondary(&body_slam)),
        SecondaryRoll::Chance(76, 256)
    );
    let dynamic_punch = move_def(2);
    assert_eq!(
        secondary_roll(
            state,
            rules,
            A,
            B,
            &dynamic_punch,
            &secondary(&dynamic_punch)
        ),
        SecondaryRoll::Chance(255, 256)
    );
    let ice_beam = move_def(3);
    assert_eq!(
        secondary_roll(state, rules, A, B, &ice_beam, &secondary(&ice_beam)),
        SecondaryRoll::Skipped
    );

    // An accuracy drop makes even a 100 % move roll: 255 × 0.75 → 191.
    let dropped = singles(
        Generation::Gen2,
        [
            set("Snorlax", &["Body Slam"]),
            set("Cloyster", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[0].stages.accuracy = -1,
    );
    let state = dropped.engine.state();
    let body_slam = dropped
        .dex
        .move_by_id(state.pokemon(A).unwrap().moves[0].move_id)
        .unwrap()
        .clone();
    assert_eq!(
        accuracy_roll(state, &dropped.rules, A, B, &body_slam),
        Some((191, 256))
    );
    // Gen 3: percent; a declared 100 % secondary is certain.
    let gen3 = singles(
        Generation::Gen3,
        [
            set("Snorlax", &["DynamicPunch"]),
            set("Cloyster", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let state = gen3.engine.state();
    let dynamic_punch = gen3
        .dex
        .move_by_id(state.pokemon(A).unwrap().moves[0].move_id)
        .unwrap()
        .clone();
    assert_eq!(
        secondary_roll(
            state,
            &gen3.rules,
            A,
            B,
            &dynamic_punch,
            &dynamic_punch.secondaries[0]
        ),
        SecondaryRoll::Certain
    );
}

#[test]
fn a_gen_2_critical_hit_ignores_the_stages_unless_the_attacker_is_ahead() {
    // Attacker +1 Attack, defender +0 Defense: ahead, the stages count (×1.5 Attack).
    // Attacker +0, defender +1 Defense: both stages are ignored (raw Defense).
    for (attack_stage, defense_stage, boosted_attack, raw_defense) in
        [(1, 0, true, true), (0, 1, false, true)]
    {
        let mut arena = singles(
            Generation::Gen2,
            [
                set("Snorlax", &["Body Slam"]),
                set("Blissey", &["Swords Dance"]),
                filler(),
                filler(),
            ],
            |pokemon| {
                pokemon[0].stages.attack = attack_stage;
                pokemon[1].stages.defense = defense_stage;
            },
        );
        let attack = u32::from(arena.pokemon(A).stats.attack);
        let defense = u32::from(arena.pokemon(B).stats.defense);
        let tape = vec![
            RandomOverride::bool(RandomRequestKind::Critical, true),
            RandomOverride::range(RandomRequestKind::DamageRoll, 217),
            RandomOverride::bool(RandomRequestKind::Secondary, false),
        ];
        let (start, _) = arena.turn(Act::Move(0), Act::Move(0), tape);
        let offense = arena.events(start).iter().find_map(|e| match e {
            TraceEvent::OffensiveStatResolved { base, .. } => Some(*base),
            _ => None,
        });
        let used_defense = arena.events(start).iter().find_map(|e| match e {
            TraceEvent::DefensiveStatResolved { value, .. } => Some(*value),
            _ => None,
        });
        assert!(
            attack * 3 / 2 < 256 && defense < 256,
            "stay under the rollover"
        );
        assert_eq!(
            offense,
            Some(if boosted_attack {
                attack * 3 / 2
            } else {
                attack
            })
        );
        assert_eq!(used_defense.is_some_and(|d| d == defense), raw_defense);
    }
}

#[test]
fn gen_2_poison_hurts_right_after_the_users_own_action() {
    // Snorlax (poisoned, slower) acts after Blissey's Swords Dance; its poison damage follows
    // its own move, and there is no end-of-turn poison residual.
    let mut arena = singles(
        Generation::Gen2,
        [
            set("Snorlax", &["Swords Dance"]),
            set("Jolteon", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Poison)),
    );
    // A poisoned lead already took 1/8 on entry (`onAfterSwitchInSelf`).
    let max_hp = arena.pokemon(A).max_hp;
    assert_eq!(arena.pokemon(A).hp, max_hp - max_hp / 8);
    let (start, _) = arena.turn(Act::Move(0), Act::Move(0), Vec::new());
    assert_eq!(arena.pokemon(A).hp, max_hp - 2 * (max_hp / 8));
    let poison = arena
        .position_of(
            |e| matches!(e, TraceEvent::StatusResidual { target, .. } if *target == A),
            start,
        )
        .unwrap();
    let snorlax_moved = arena
        .position_of(
            |e| matches!(e, TraceEvent::MoveUsed { actor, .. } if *actor == A),
            start,
        )
        .unwrap();
    assert!(snorlax_moved < poison);
    assert!(!arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::ResidualEffect {
            kind: ResidualKind::Poison,
            ..
        }
    )));

    // After a move that knocks the foe out, the user's poison waits.
    let mut arena = singles(
        Generation::Gen2,
        [
            set("Jolteon", &["Thunderbolt"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| {
            pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Poison));
            pokemon[1].hp = 1;
        },
    );
    let before = arena.pokemon(A).hp;
    let mut tape = gen2_hit(217);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    let (_, step) = arena.turn(Act::Move(0), Act::Move(0), tape);
    assert!(matches!(
        step,
        EngineStep::Awaiting(BattleRequest::ForcedSwitch { .. })
    ));
    assert_eq!(arena.pokemon(A).hp, before);
}

#[test]
fn gen_2_bad_poison_counts_up_and_turns_into_poison_on_the_way_back_in() {
    // Jolteon badly poisons Snorlax (slower): 1/16 right after Snorlax's move, then 2/16.
    let mut arena = singles(
        Generation::Gen2,
        [
            set("Jolteon", &["Toxic", "Swords Dance"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            set("Blissey", &["Swords Dance"]),
        ],
        |_| {},
    );
    let max_hp = arena.pokemon(B).max_hp;
    let sixteenth = max_hp / 16;
    arena.turn(
        Act::Move(0),
        Act::Move(0),
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)],
    );
    assert_eq!(arena.pokemon(B).hp, max_hp - sixteenth);
    assert_eq!(
        arena.pokemon(B).volatiles.residual_damage_counter(),
        Some(1)
    );
    arena.turn(Act::Move(1), Act::Move(0), Vec::new());
    assert_eq!(arena.pokemon(B).hp, max_hp - 3 * sixteenth);

    // Out and back in: the counter is gone, bad poison becomes poison, 1/8 on entry.
    arena.turn(Act::Move(1), Act::Switch(D), Vec::new());
    assert_eq!(
        arena.pokemon(B).status.map(|s| s.kind),
        Some(NonVolatileStatus::Toxic)
    );
    let before = arena.pokemon(B).hp;
    arena.turn(Act::Move(1), Act::Switch(B), Vec::new());
    assert_eq!(
        arena.pokemon(B).status.map(|s| s.kind),
        Some(NonVolatileStatus::Poison)
    );
    assert_eq!(arena.pokemon(B).volatiles.residual_damage_counter(), None);
    assert_eq!(arena.pokemon(B).hp, before - max_hp / 8);
}

#[test]
fn a_gen_2_freeze_thaws_only_at_the_end_of_the_turn() {
    let mut arena = singles(
        Generation::Gen2,
        [
            set("Snorlax", &["Swords Dance"]),
            set("Jolteon", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Freeze)),
    );
    // One Thaw decision per turn, in the residual; the frozen Snorlax never moves.
    arena.turn(
        Act::Move(0),
        Act::Move(0),
        vec![RandomOverride::bool(RandomRequestKind::Thaw, false)],
    );
    assert!(arena.pokemon(A).has_status(NonVolatileStatus::Freeze));
    let (start, _) = arena.turn(
        Act::Move(0),
        Act::Move(0),
        vec![RandomOverride::bool(RandomRequestKind::Thaw, true)],
    );
    assert!(arena.pokemon(A).status.is_none());
    assert_eq!(arena.pokemon(A).stages.attack, 0);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::ResidualEffect {
            holder,
            kind: ResidualKind::FreezeThaw,
            ..
        } if *holder == A
    )));
}

#[test]
fn gen_2_protect_halves_its_chance_from_127_over_255() {
    let mut arena = singles(
        Generation::Gen2,
        [
            set("Jolteon", &["Protect"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let counters = |arena: &Arena, start: usize| -> Vec<u16> {
        arena
            .events(start)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::ProtectChecked { counter, .. } => Some(*counter),
                _ => None,
            })
            .collect()
    };
    let (start, _) = arena.turn(Act::Move(0), Act::Move(0), Vec::new());
    assert!(counters(&arena, start).is_empty());
    let (start, _) = arena.turn(
        Act::Move(0),
        Act::Move(0),
        vec![RandomOverride::bool(
            RandomRequestKind::ProtectSuccess,
            true,
        )],
    );
    assert_eq!(counters(&arena, start), vec![127]);
    let (start, _) = arena.turn(
        Act::Move(0),
        Act::Move(0),
        vec![RandomOverride::bool(
            RandomRequestKind::ProtectSuccess,
            false,
        )],
    );
    assert_eq!(counters(&arena, start), vec![63]);
}

#[test]
fn gen_2_spikes_hold_a_single_layer() {
    let mut arena = singles(
        Generation::Gen2,
        [
            set("Skarmory", &["Spikes"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    arena.turn(Act::Move(0), Act::Move(0), Vec::new());
    let (start, _) = arena.turn(Act::Move(0), Act::Move(0), Vec::new());
    assert_eq!(
        arena
            .engine
            .state()
            .side(SideId::P2)
            .conditions
            .spikes_layers(),
        1
    );
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveFailed { actor, .. } if *actor == A))
    );
}

#[test]
fn a_faint_cuts_a_gen_2_or_3_singles_turn_short() {
    // Tauros (faster, 1 HP) knocks itself out with Double-Edge's recoil. Gen 2-3: Snorlax's
    // Swords Dance is cancelled; Gen 4: it still happens.
    for (generation, roll, cancelled) in [
        (Generation::Gen2, 217, true),
        (Generation::Gen3, 100, true),
        (Generation::Gen4, 100, false),
    ] {
        let mut arena = singles(
            generation,
            [
                set("Tauros", &["Double-Edge"]),
                set("Snorlax", &["Swords Dance"]),
                filler(),
                filler(),
            ],
            |pokemon| pokemon[0].hp = 1,
        );
        let tape = vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, roll),
        ];
        let (start, _) = arena.turn(Act::Move(0), Act::Move(0), tape);
        assert!(arena.pokemon(A).is_fainted(), "{generation:?}");
        let snorlax_moved = arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveUsed { actor, .. } if *actor == B));
        assert_eq!(snorlax_moved, !cancelled, "{generation:?}");
        assert_eq!(
            arena
                .events(start)
                .iter()
                .any(|e| matches!(e, TraceEvent::ActionsCancelledByFaint { .. })),
            cancelled,
            "{generation:?}"
        );
    }
}

#[test]
fn leech_seed_does_nothing_without_someone_at_the_seeders_position() {
    // Gen 9: Jolteon seeds Snorlax, then Snorlax knocks Jolteon out; at the end of the turn
    // the seeder's position is empty, so Snorlax loses nothing (Showdown `leechseed`).
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Jolteon", &["Leech Seed"]),
            set("Snorlax", &["Tackle"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[0].hp = 1,
    );
    let max_hp = arena.pokemon(B).max_hp;
    let tape = vec![
        RandomOverride::bool(RandomRequestKind::Accuracy, true),
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ];
    arena.turn(Act::Move(0), Act::Move(0), tape);
    assert!(arena.pokemon(A).is_fainted());
    assert!(
        arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::LeechSeed)
    );
    assert_eq!(arena.pokemon(B).hp, max_hp);
}

#[test]
fn gen_2_recovery_rounds_half_up() {
    for (generation, half_up) in [(Generation::Gen2, true), (Generation::Gen3, false)] {
        let mut arena = singles(
            generation,
            [
                set("Starmie", &["Recover"]),
                set("Snorlax", &["Swords Dance"]),
                filler(),
                filler(),
            ],
            |pokemon| pokemon[0].hp = 1,
        );
        let max_hp = arena.pokemon(A).max_hp;
        assert_eq!(max_hp % 2, 1, "the test needs an odd max HP");
        arena.turn(Act::Move(0), Act::Move(0), Vec::new());
        let healed = arena.pokemon(A).hp - 1;
        assert_eq!(
            healed,
            if half_up { max_hp / 2 + 1 } else { max_hp / 2 },
            "{generation:?}"
        );
    }
}

#[test]
fn an_item_does_only_what_its_generation_declares() {
    use pokemon_battle_engine::effects::BehaviorRegistry;
    use pokemon_battle_engine::ids::ItemId;
    let boosts = |generation, item: ItemId| {
        BehaviorRegistry::for_generation(generation)
            .item(item)
            .base_power_boost
            .is_some()
    };
    // Gen 2's Dragon Fang declares nothing and does nothing; Dragon Scale is the Dragon
    // booster there, and an evolution item afterwards.
    assert!(!boosts(Generation::Gen2, ItemId::DRAGON_FANG));
    assert!(boosts(Generation::Gen2, ItemId::DRAGON_SCALE));
    assert!(boosts(Generation::Gen3, ItemId::DRAGON_FANG));
    assert!(!boosts(Generation::Gen9, ItemId::DRAGON_SCALE));
    // Pink Bow exists as a Normal booster only in Gen 2.
    assert!(boosts(Generation::Gen2, ItemId::PINK_BOW));
    assert!(!boosts(Generation::Gen9, ItemId::PINK_BOW));
    // Gen 4's Life Orb recoil lives in the volatile its `onBasePower` sets.
    assert!(
        BehaviorRegistry::for_generation(Generation::Gen4)
            .item(ItemId::LIFE_ORB)
            .after_move_self
            .is_some()
    );
}

#[test]
fn gen_2_sandstorm_fails_under_sandstorm_but_rain_dance_starts_over() {
    let mut arena = singles(
        Generation::Gen2,
        [
            set("Tyranitar", &["Sandstorm", "Rain Dance"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let weather = |arena: &Arena| {
        arena
            .engine
            .state()
            .field
            .weather
            .map(|w| (w.kind, w.turns_remaining))
    };
    use pokemon_battle_engine::state::WeatherKind;
    arena.turn(Act::Move(0), Act::Move(0), Vec::new());
    assert_eq!(weather(&arena), Some((WeatherKind::Sandstorm, 4)));
    let (start, _) = arena.turn(Act::Move(0), Act::Move(0), Vec::new());
    assert_eq!(weather(&arena), Some((WeatherKind::Sandstorm, 3)));
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveFailed { actor, .. } if *actor == A))
    );
    arena.turn(Act::Move(1), Act::Move(0), Vec::new());
    assert_eq!(weather(&arena), Some((WeatherKind::Rain, 4)));
    arena.turn(Act::Move(1), Act::Move(0), Vec::new());
    assert_eq!(weather(&arena), Some((WeatherKind::Rain, 4)));
}

#[test]
fn gen_2_encore_takes_no_extra_turn_for_a_target_that_already_moved() {
    // Showdown gen2 `encore` overrides `onStart` without the +1 later generations give a
    // target that does not act again this turn.
    use pokemon_battle_engine::rules::{DurationAdjust, RulesResolver};
    use pokemon_battle_engine::spec::{BattleMode, BattleSpec};
    for (generation, adjust) in [
        (Generation::Gen2, DurationAdjust::None),
        (Generation::Gen3, DurationAdjust::PlusIfWontMove),
    ] {
        let rules =
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .unwrap();
        assert_eq!(rules.conditions().encore.adjust, adjust, "{generation:?}");
    }
}

#[test]
fn gen_2_recoil_off_a_substitute_is_one_hp() {
    // Showdown gen2 `substitute`: `this.damage(1, source, target, 'recoil')`, whatever the hit.
    let mut arena = singles(
        Generation::Gen2,
        [
            set("Snorlax", &["Double-Edge"]),
            set("Jolteon", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[1].volatiles.set_substitute(B, 200),
    );
    let hp = arena.pokemon(A).hp;
    arena.turn(Act::Move(0), Act::Move(0), gen2_hit(217));
    assert_eq!(arena.pokemon(A).hp, hp - 1);
    assert!(
        arena
            .pokemon(B)
            .volatiles
            .substitute_hp()
            .is_some_and(|doll| doll < 200)
    );
}
