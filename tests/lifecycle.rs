//! L1 rule tests for the switch / faint / residual lifecycles (ROADMAP M3, DESIGN.md §30.1):
//! the residual scheduler and its generation order, Leftovers and Leech Seed, the entry hazards
//! (Spikes, Toxic Spikes, Sticky Web, Heavy-Duty Boots) in the switch-in stage, and the faint
//! step's ordering rule for double knock-outs.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, EffectSource, EngineStep, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleOutcome, BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride,
    RandomRequestKind, SideConditionKind, SideState, StatusState, VolatileConditionKind,
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
    set.ability = ability.map(Into::into);
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

/// P1 = [A, C], P2 = [B, D]; the first of each side leads.
fn setup(
    generation: Generation,
    p1: [PokemonSet; 2],
    p2: [PokemonSet; 2],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1[0]).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2[0]).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &p1[1]).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &p2[1]).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side([A, C]), side([B, D])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
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
                    | TargetPattern::SelectedAdjacentFoe
                    | TargetPattern::Ally
                    | TargetPattern::AllyOrSelf => Some(DeclaredTarget(
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

    /// One turn (speeds differ, so no SpeedTie is drawn). Returns the trace index it started at.
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

    fn residuals(&self, from: usize) -> Vec<(PokemonId, ResidualKind)> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::ResidualEffect { holder, kind, .. } => Some((*holder, *kind)),
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
enum Action {
    Move(usize),
    Switch(PokemonId),
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn accurate() -> RandomOverride {
    RandomOverride::bool(RandomRequestKind::Accuracy, true)
}

// ---------------------------------------------------------------------------------------------
// Residual scheduler
// ---------------------------------------------------------------------------------------------

#[test]
fn residuals_run_in_generation_order_then_by_speed() {
    // Forretress (40 Spe, Leftovers, burned) vs Garchomp (102 Spe, Leftovers, seeded, poisoned).
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Forretress", &["Leech Seed"], None, Some("Leftovers")),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Garchomp", &["Tackle"], None, Some("Leftovers")),
            set("Pikachu", &["Tackle"], None, None),
        ],
        |pokemon| {
            pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Burn));
            pokemon[0].hp -= 40;
            pokemon[1].status = Some(StatusState::new(NonVolatileStatus::Poison));
            pokemon[1].hp -= 40;
        },
    );
    // Garchomp Tackles, Forretress seeds it (90 % accuracy).
    let mut tape = hit(100);
    tape.push(accurate());
    let start = arena.turn(Action::Move(0), Action::Move(0), tape);

    assert_eq!(
        arena.residuals(start),
        vec![
            (B, ResidualKind::Leftovers),
            (A, ResidualKind::Leftovers),
            (B, ResidualKind::LeechSeed),
            (B, ResidualKind::Poison),
            (A, ResidualKind::Burn),
        ],
        "Leftovers 5.4 (faster first) → Leech Seed 8 → poison 9 → burn 10"
    );
    let orders: Vec<(u8, u8)> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::ResidualEffect {
                order, sub_order, ..
            } => Some((*order, *sub_order)),
            _ => None,
        })
        .collect();
    assert_eq!(orders, vec![(5, 4), (5, 4), (8, 0), (9, 0), (10, 0)]);
}

#[test]
fn leftovers_heals_a_sixteenth_and_nothing_at_full_hp() {
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Tackle"], None, Some("Leftovers")),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Garchomp", &["Tackle"], None, Some("Leftovers")),
            set("Pikachu", &["Tackle"], None, None),
        ],
        |pokemon| pokemon[0].hp -= 100,
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(Action::Move(0), Action::Move(0), tape);
    let heals: Vec<(PokemonId, u16)> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::HealApplied {
                target,
                amount,
                source:
                    EffectSource::Item {
                        item: ItemId::LEFTOVERS,
                        ..
                    },
                ..
            } => Some((*target, *amount)),
            _ => None,
        })
        .collect();
    // Garchomp was hit by Tackle so it heals too; both 1/16, floor.
    assert_eq!(
        heals,
        vec![
            (B, arena.pokemon(B).max_hp / 16),
            (A, arena.pokemon(A).max_hp / 16)
        ]
    );
    // Full HP: no heal event (refused by the HealRequest).
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Swords Dance"], None, Some("Leftovers")),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Garchomp", &["Swords Dance"], None, None),
            set("Pikachu", &["Tackle"], None, None),
        ],
        |_| {},
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::HealApplied { .. }))
    );
    assert_eq!(arena.residuals(start), vec![(A, ResidualKind::Leftovers)]);
}

#[test]
fn leech_seed_drains_towards_the_seeders_position_and_grass_is_immune() {
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Forretress", &["Leech Seed", "Tackle"], None, None),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Garchomp", &["Swords Dance"], None, None),
            set("Pikachu", &["Tackle"], None, None),
        ],
        |pokemon| pokemon[0].hp -= 50,
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), vec![accurate()]);
    assert!(
        arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::LeechSeed)
    );
    let eighth = arena.pokemon(B).max_hp / 8;
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::DamageApplied { target: B, amount, source: EffectSource::Volatile(VolatileConditionKind::LeechSeed), .. } if *amount == eighth
    )));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::HealApplied { target: A, amount, source: EffectSource::Volatile(VolatileConditionKind::LeechSeed), .. } if *amount == eighth
    )));

    // Seeding again fails; after Forretress switches out, Snorlax (now at the seeder's
    // position) receives the drain.
    let start = arena.turn(Action::Move(0), Action::Move(0), vec![accurate()]);
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveFailed { actor: A, .. }))
    );
    let start = arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    assert!(
        arena.events(start).iter().any(|e| matches!(
            e,
            TraceEvent::HealApplied {
                target: C,
                source: EffectSource::Volatile(VolatileConditionKind::LeechSeed),
                ..
            }
        )) || arena.pokemon(C).hp == arena.pokemon(C).max_hp,
        "Snorlax is at full HP so the heal is refused, but the drain still ran"
    );
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::DamageApplied {
            target: B,
            source: EffectSource::Volatile(VolatileConditionKind::LeechSeed),
            ..
        }
    )));

    // Grass types are immune.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Forretress", &["Leech Seed"], None, None),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Venusaur", &["Swords Dance"], None, None),
            set("Pikachu", &["Tackle"], None, None),
        ],
        |_| {},
    );
    // A hit step (`onTryImmunity`, before the accuracy check): no accuracy is drawn (R-M8-107).
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(
        !arena
            .pokemon(B)
            .volatiles
            .contains(VolatileConditionKind::LeechSeed)
    );
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::HitBlocked {
            actor: A,
            reason: pokemon_battle_engine::engine::HitBlockReason::MoveRule,
            ..
        }
    )));
}

// ---------------------------------------------------------------------------------------------
// Entry hazards in the switch-in stage
// ---------------------------------------------------------------------------------------------

#[test]
fn spikes_stack_to_three_layers_and_only_hit_grounded_pokemon() {
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Forretress", &["Spikes"], None, None),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Garchomp", &["Swords Dance"], None, None),
            set("Pikachu", &["Tackle"], None, Some("Heavy-Duty Boots")),
        ],
        |_| {},
    );
    let expected: [u16; 3] = {
        let max_hp = arena.pokemon(B).max_hp;
        [max_hp * 3 / 24, max_hp * 4 / 24, max_hp * 6 / 24]
    };
    for layer in 1..=3u16 {
        let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
        assert!(arena.events(start).iter().any(|e| matches!(
            e,
            TraceEvent::SideConditionLayered { side: SideId::P2, kind: SideConditionKind::Spikes, layers } if u16::from(*layers) == layer
        )));
    }
    // A fourth layer fails.
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveFailed { actor: A, .. }))
    );
    assert_eq!(
        arena
            .engine
            .state()
            .side(SideId::P2)
            .conditions
            .spikes_layers(),
        3
    );

    // Pikachu (Heavy-Duty Boots) comes in unharmed; Garchomp comes back onto three layers.
    let start = arena.turn(Action::Move(0), Action::Switch(D), Vec::new());
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::EntryHazardTriggered { .. }))
    );
    assert_eq!(arena.pokemon(D).hp, arena.pokemon(D).max_hp);
    let start = arena.turn(Action::Move(0), Action::Switch(B), Vec::new());
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::EntryHazardTriggered { target: B, kind: SideConditionKind::Spikes, damage, .. } if *damage == expected[2]
    )));
    assert_eq!(arena.pokemon(B).hp, arena.pokemon(B).max_hp - expected[2]);
    let _ = expected;
}

#[test]
fn toxic_spikes_poison_badly_at_two_layers_and_a_poison_type_absorbs_them() {
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Forretress", &["Toxic Spikes"], None, None),
            set("Snorlax", &["Tackle"], Some("Thick Fat"), None),
        ],
        [
            set("Garchomp", &["Swords Dance"], None, None),
            set("Snorlax", &["Tackle"], Some("Thick Fat"), None),
        ],
        |_| {},
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    let start = arena.turn(Action::Move(0), Action::Switch(D), Vec::new());
    assert!(
        arena.pokemon(D).has_status(NonVolatileStatus::Poison),
        "one layer: poison"
    );
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::StatusApplied {
            target: D,
            status: NonVolatileStatus::Poison,
            source: EffectSource::SideCondition(SideConditionKind::ToxicSpikes)
        }
    )));
    // Second layer, then Garchomp (already switched out, healthy) comes back: badly poisoned.
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(
        arena
            .engine
            .state()
            .side(SideId::P2)
            .conditions
            .toxic_spikes_layers(),
        2
    );
    arena.turn(Action::Move(0), Action::Switch(B), Vec::new());
    assert!(
        arena.pokemon(B).has_status(NonVolatileStatus::Toxic),
        "two layers: toxic"
    );

    // A grounded Poison type absorbs the layers.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Forretress", &["Toxic Spikes"], None, None),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Garchomp", &["Swords Dance"], None, None),
            set("Muk", &["Tackle"], None, None),
        ],
        |_| {},
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    // Muk switches in first (switches go before moves), absorbs the layer, and Forretress
    // lays a fresh one afterwards: one layer, not two.
    let start = arena.turn(Action::Move(0), Action::Switch(D), Vec::new());
    assert!(arena.pokemon(D).status.is_none());
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::SideConditionEnded {
            side: SideId::P2,
            kind: SideConditionKind::ToxicSpikes
        }
    )));
    assert_eq!(
        arena
            .engine
            .state()
            .side(SideId::P2)
            .conditions
            .toxic_spikes_layers(),
        1
    );

    // Flying types are not grounded: untouched, layers stay.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Forretress", &["Toxic Spikes"], None, None),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Garchomp", &["Swords Dance"], None, None),
            set("Talonflame", &["Tackle"], None, None),
        ],
        |_| {},
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    // Talonflame is untouched and absorbs nothing; Forretress's second layer still lands.
    arena.turn(Action::Move(0), Action::Switch(D), Vec::new());
    assert!(arena.pokemon(D).status.is_none());
    assert_eq!(
        arena
            .engine
            .state()
            .side(SideId::P2)
            .conditions
            .toxic_spikes_layers(),
        2
    );
}

#[test]
fn sticky_web_lowers_speed_through_the_stat_change_request() {
    // Clear Body refuses the drop (it comes from the web's setter), Contrary inverts it.
    for (species, ability, expected) in [
        ("Snorlax", None, -1),
        ("Metagross", Some("Clear Body"), 0),
        ("Serperior", Some("Contrary"), 1),
    ] {
        let mut arena = setup(
            Generation::Gen9,
            [
                set("Forretress", &["Sticky Web"], None, None),
                set("Snorlax", &["Tackle"], None, None),
            ],
            [
                set("Garchomp", &["Swords Dance"], None, None),
                set(species, &["Tackle"], ability, None),
            ],
            |_| {},
        );
        arena.turn(Action::Move(0), Action::Move(0), Vec::new());
        arena.turn(Action::Move(0), Action::Switch(D), Vec::new());
        assert_eq!(arena.pokemon(D).stages.speed, expected, "{species}");
    }
}

#[test]
fn hazards_trigger_in_the_order_they_were_laid_and_a_knock_out_stops_them() {
    // Spikes first, then Stealth Rock; the incoming Pokémon has 1 HP and Spikes finish it, so
    // Stealth Rock never triggers and neither does its switch-in ability.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Forretress", &["Spikes", "Stealth Rock"], None, None),
            set("Snorlax", &["Tackle"], None, None),
        ],
        [
            set("Garchomp", &["Swords Dance"], None, None),
            set(
                "Gyarados",
                &["Tackle"],
                Some("Intimidate"),
                Some("Iron Ball"),
            ),
        ],
        |pokemon| pokemon[3].hp = 1,
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    // Gyarados is Flying, but its Iron Ball grounds it: Spikes knock it out and Stealth Rock
    // never triggers.
    let start = arena.turn(Action::Move(0), Action::Switch(D), Vec::new());
    let hazards: Vec<SideConditionKind> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::EntryHazardTriggered {
                target: D, kind, ..
            } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(hazards, vec![SideConditionKind::Spikes]);
    assert!(arena.pokemon(D).is_fainted());
    assert!(!arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::AbilityActivated {
            source: D,
            ability: AbilityId::INTIMIDATE
        }
    )));
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(pokemon_battle_engine::engine::BattleRequest::ForcedSwitch { .. })
    ));
}

// ---------------------------------------------------------------------------------------------
// Faint step
// ---------------------------------------------------------------------------------------------

#[test]
fn a_double_knock_out_is_won_by_the_side_that_fainted_last_from_gen5() {
    for (generation, expected) in [
        (Generation::Gen9, BattleOutcome::Winner(SideId::P1)),
        (Generation::Gen4, BattleOutcome::Draw),
    ] {
        // Both leads are the last Pokémon standing on each side: Double-Edge knocks Snorlax out
        // and its recoil knocks Tauros out afterwards, so Tauros's side wins from Gen 5.
        let mut arena = setup(
            generation,
            [
                set("Tauros", &["Double-Edge"], None, None),
                set("Snorlax", &["Tackle"], None, None),
            ],
            [
                set("Snorlax", &["Tackle"], None, None),
                set("Pikachu", &["Tackle"], None, None),
            ],
            |pokemon| {
                // Double-Edge deals ≥ 40 and recoils a third of it: enough for Tauros's 5 HP.
                pokemon[0].hp = 5;
                pokemon[1].hp = 40;
                pokemon[2].hp = 0;
                pokemon[3].hp = 0;
            },
        );
        let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
        let fainted: Vec<PokemonId> = arena
            .events(start)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::Fainted { pokemon, .. } => Some(*pokemon),
                _ => None,
            })
            .collect();
        assert_eq!(
            fainted,
            vec![B, A],
            "{generation:?}: target first, then the recoil"
        );
        assert_eq!(
            arena.engine.state().outcome,
            Some(expected),
            "{generation:?}"
        );
    }
}

#[test]
fn gen_5_puts_the_sleep_counter_back_as_the_sleeper_switches_in() {
    // Showdown gen5 `slp.onSwitchIn`: `time = startTime`. A sleeper rolled for two turns that
    // slept one before switching out sleeps one more after coming back.
    for (generation, asleep_after_return) in [(Generation::Gen5, true), (Generation::Gen6, false)] {
        let mut arena = setup(
            generation,
            [
                set("Snorlax", &["Tackle"], None, None),
                set("Blissey", &["Growl"], None, None),
            ],
            [
                set("Chansey", &["Growl"], None, None),
                set("Blissey", &["Growl"], None, None),
            ],
            |pokemon| {
                let mut sleep = StatusState::new(NonVolatileStatus::Sleep);
                sleep.sleep_turns_remaining = Some(2);
                sleep.sleep_turns_rolled = Some(2);
                pokemon[0].status = Some(sleep);
            },
        );
        arena.turn(Action::Move(0), Action::Move(0), Vec::new());
        arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
        arena.turn(Action::Switch(A), Action::Move(0), Vec::new());
        // Gen 5: back to 2, so this turn is spent asleep; Gen 6: 1 left, it wakes and acts.
        let tape = if asleep_after_return {
            Vec::new()
        } else {
            hit(100)
        };
        arena.turn(Action::Move(0), Action::Move(0), tape);
        assert_eq!(
            arena.pokemon(A).has_status(NonVolatileStatus::Sleep),
            asleep_after_return,
            "{generation:?}"
        );
    }
}

#[test]
fn substitute_ends_a_bind_on_its_user() {
    // Showdown `substitute`'s `onStart`: the bind ends, so no bind damage at the end of turn.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Substitute"], None, None),
            set("Blissey", &["Growl"], None, None),
        ],
        [
            set("Arbok", &["Growl"], None, None),
            set("Blissey", &["Growl"], None, None),
        ],
        |pokemon| {
            pokemon[0]
                .volatiles
                .set_partially_trapped(B, MoveId::WRAP, 4);
        },
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(
        !arena
            .pokemon(A)
            .volatiles
            .contains(VolatileConditionKind::PartiallyTrapped)
    );
    assert!(
        !arena
            .residuals(start)
            .iter()
            .any(|(holder, kind)| { *holder == A && *kind == ResidualKind::PartiallyTrapped })
    );
}

#[test]
fn gen_1_to_3_replace_a_fainted_pokemon_only_after_a_queued_switch() {
    // Showdown `runAction` → `checkFainted` only when the next queued action is a move or the
    // residual: Gen 2 poison fells the newcomer right after its switch, the foe's switch still
    // goes first.
    let mut arena = setup(
        Generation::Gen2,
        [
            set("Jolteon", &["Growl"], None, None),
            set("Snorlax", &["Growl"], None, None),
        ],
        [
            set("Slowbro", &["Growl"], None, None),
            set("Chansey", &["Growl"], None, None),
        ],
        |pokemon| {
            pokemon[2].status = Some(StatusState::new(NonVolatileStatus::Poison));
            pokemon[2].hp = 1;
        },
    );
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(1, Vec::new());
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let choices = [
        arena.choice(SideId::P1, Action::Switch(C)),
        arena.choice(SideId::P2, Action::Switch(D)),
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    let step = loop {
        match arena.engine.step().unwrap() {
            EngineStep::Progressed { .. } => {}
            other => break other,
        }
    };
    assert!(matches!(
        step,
        EngineStep::Awaiting(BattleRequest::ForcedSwitch { .. })
    ));
    assert!(arena.pokemon(C).is_fainted());
    assert_eq!(arena.engine.state().side(SideId::P2).active[0], Some(D));
}
