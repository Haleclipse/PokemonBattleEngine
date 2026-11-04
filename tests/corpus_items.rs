//! L1 rule tests for the M4 closing batch (ROADMAP §8, DESIGN.md §22.4): the items and
//! abilities the curated corpus needed — type-boosting items / Muscle Band as a base-power
//! modifier, Life Orb / Expert Belt chained into ModifyDamage, Choice Scarf / Specs, Speed
//! Boost, Serene Grace, the weather rocks — plus two request-boundary facts the corpus exposed
//! (a mid-turn request asks only for ordered-out slots; a fainted Pokémon's status gives way at
//! the end-of-turn faint check).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, CureReason, EffectSource, EngineStep, ModifierSource,
    ModifierStage, SideChoice, SlotChoice, TraceEvent, effective_secondary_chance,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState, WeatherKind,
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

fn set(species: &str, moves: &[&str], ability: &str, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

fn bench() -> PokemonSet {
    set("Vaporeon", &["Tackle"], "Water Absorb", None)
}

/// P1 = [A, C]; P2 = [B, D]. Leads' switch-in runs first (weather abilities).
fn setup(p1: PokemonSet, p2: PokemonSet, prepare: impl FnOnce(&mut Vec<PokemonState>)) -> Arena {
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
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &bench()).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &bench()).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B, D])], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
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

    /// Submits the turn and drives it to the next request; returns the trace start and the
    /// request the engine stopped at.
    fn turn(&mut self, p1: Action, p2: Action, tape: Vec<RandomOverride>) -> (usize, EngineStep) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)];
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

    fn has(&self, from: usize, pred: impl Fn(&TraceEvent) -> bool) -> bool {
        self.events(from).iter().any(pred)
    }

    fn movers(&self, from: usize) -> Vec<PokemonId> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::MoveUsed { actor, .. } => Some(*actor),
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

fn damage_to(arena: &Arena, from: usize, target: PokemonId) -> u16 {
    arena
        .events(from)
        .iter()
        .find_map(|e| match e {
            TraceEvent::DamageCalculated {
                target: t,
                final_damage,
                ..
            } if *t == target => Some(*final_damage),
            _ => None,
        })
        .unwrap()
}

#[test]
fn corpus_items_and_abilities_classify_from_snapshot_data() {
    let dex = DexResolver::resolve(Generation::Gen9);
    for name in [
        "Life Orb",
        "Expert Belt",
        "Choice Scarf",
        "Choice Specs",
        "Muscle Band",
        "Wise Glasses",
        "Charcoal",
        "Silk Scarf",
        "Fairy Feather",
        "Heat Rock",
        "Damp Rock",
        "Light Clay",
        "Terrain Extender",
    ] {
        assert!(dex.item_by_name(name).unwrap().implemented, "{name}");
    }
    for name in ["Speed Boost", "Serene Grace"] {
        assert!(dex.ability_by_name(name).unwrap().implemented, "{name}");
    }
    // Protective Pads carry no callback of their own: Showdown handles them inside the
    // simulator's `checkMoveMakesContact`, which the engine models once (`move_makes_contact`,
    // DESIGN.md §43.4), so they count as implemented.
    assert!(dex.item_by_name("Protective Pads").unwrap().implemented);
    // Rusted Sword / Shield: no callback of their own, and the crowned forme they give lives
    // in the species' `onBattleStart` — which the engine runs at the start of the battle
    // (`effects::items::battle_start_forme`, DESIGN.md §43.4), so they are implemented.
    for name in ["Rusted Sword", "Rusted Shield"] {
        assert!(dex.item_by_name(name).unwrap().implemented, "{name}");
    }
    // Gen 4's Life Orb is a different mechanism (×1.3 at ModifyDamagePhase2, recoil only
    // after a hit on a body): covered by `damage_phase2`.
    assert!(
        DexResolver::resolve(Generation::Gen4)
            .item_by_name("Life Orb")
            .unwrap()
            .implemented
    );
}

#[test]
fn type_boosters_raise_base_power_with_showdowns_modify_rounding() {
    // Silk Scarf on Body Slam: 85 × 4915 / 4096 = 101.99… → 102 (round half down).
    let mut arena = setup(
        set(
            "Snorlax",
            &["Body Slam", "Earthquake"],
            "Thick Fat",
            Some("Silk Scarf"),
        ),
        set("Blissey", &["Softboiled"], "Natural Cure", None),
        |_| {},
    );
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    let (start, _) = arena.turn(Action::Move(0), Action::Move(0), tape);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ModifierApplied {
            stage: ModifierStage::BasePower,
            source: ModifierSource::ItemInstance {
                item: ItemId::SILK_SCARF,
                ..
            },
            from: 85,
            to: 102,
            ..
        }
    )));
    // A Ground move is not boosted by a Normal booster.
    let (start, _) = arena.turn(Action::Move(1), Action::Move(0), hit(100));
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::ModifierApplied {
            source: ModifierSource::ItemInstance { .. },
            stage: ModifierStage::BasePower,
            ..
        }
    )));
}

#[test]
fn life_orb_and_expert_belt_chain_into_modify_damage_and_life_orb_costs_a_tenth() {
    // Reference damage without an item.
    let mut plain = setup(
        set(
            "Garchomp",
            &["Earthquake", "Dragon Claw"],
            "Rough Skin",
            None,
        ),
        set("Metagross", &["Iron Defense"], "Clear Body", None),
        |_| {},
    );
    let (start, _) = plain.turn(Action::Move(0), Action::Move(0), hit(100));
    let base = damage_to(&plain, start, B);

    let mut orb = setup(
        set(
            "Garchomp",
            &["Earthquake", "Dragon Claw"],
            "Rough Skin",
            Some("Life Orb"),
        ),
        set("Metagross", &["Iron Defense"], "Clear Body", None),
        |_| {},
    );
    let max_hp = orb.pokemon(A).max_hp;
    let (start, _) = orb.turn(Action::Move(0), Action::Move(0), hit(100));
    assert_eq!(
        damage_to(&orb, start, B),
        ((u32::from(base) * 5324 + 2047) / 4096) as u16
    );
    assert!(orb.has(start, |e| matches!(
        e,
        TraceEvent::DamageApplied {
            target: A,
            amount,
            source: EffectSource::Item {
                item: ItemId::LIFE_ORB,
                ..
            },
            ..
        } if *amount == max_hp / 10
    )));

    // Expert Belt: only on a super-effective hit.
    let mut belt = setup(
        set(
            "Garchomp",
            &["Earthquake", "Dragon Claw"],
            "Rough Skin",
            Some("Expert Belt"),
        ),
        set("Metagross", &["Iron Defense"], "Clear Body", None),
        |_| {},
    );
    let (start, _) = belt.turn(Action::Move(0), Action::Move(0), hit(100));
    assert_eq!(
        damage_to(&belt, start, B),
        ((u32::from(base) * 4915 + 2047) / 4096) as u16
    );
    // A neutral Dragon Claw: no boost (fresh battles, the Earthquakes above may have knocked
    // Metagross out).
    let neutral = |item: Option<&str>| {
        let mut arena = setup(
            set(
                "Garchomp",
                &["Earthquake", "Dragon Claw"],
                "Rough Skin",
                item,
            ),
            set("Metagross", &["Iron Defense"], "Clear Body", None),
            |_| {},
        );
        let (start, _) = arena.turn(Action::Move(1), Action::Move(0), hit(100));
        damage_to(&arena, start, B)
    };
    assert_eq!(neutral(Some("Expert Belt")), neutral(None));
}

#[test]
fn choice_scarf_speeds_up_and_locks_and_speed_boost_skips_the_entry_turn() {
    // Metagross (70) with a Scarf outspeeds Garchomp (102) at level 50.
    let mut arena = setup(
        set(
            "Metagross",
            &["Iron Head", "Earthquake"],
            "Clear Body",
            Some("Choice Scarf"),
        ),
        set("Garchomp", &["Swords Dance"], "Rough Skin", None),
        |_| {},
    );
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    let (start, _) = arena.turn(Action::Move(0), Action::Move(0), tape);
    assert_eq!(arena.movers(start), vec![A, B]);
    assert!(arena.pokemon(A).volatiles.choice_locked_move().is_some());

    // Speed Boost: +1 at the end of every turn the holder started on the field.
    let mut arena = setup(
        set("Breloom", &["Swords Dance"], "Speed Boost", None),
        set("Snorlax", &["Swords Dance"], "Thick Fat", None),
        |_| {},
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(arena.pokemon(A).stages.speed, 1);
    // Switched-in Breloom (bench C → back) does not boost on its entry turn.
    let mut arena = setup(
        set("Vaporeon", &["Tackle"], "Water Absorb", None),
        set("Snorlax", &["Swords Dance"], "Thick Fat", None),
        |pokemon| {
            pokemon[2] = build_pokemon(
                &DexResolver::resolve(Generation::Gen9),
                C,
                SideId::P1,
                &set("Breloom", &["Swords Dance"], "Speed Boost", None),
            )
            .unwrap();
        },
    );
    let (start, _) = arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::StatChanged { target: C, .. }
    )));
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(arena.pokemon(C).stages.speed, 1);
}

#[test]
fn serene_grace_doubles_secondary_chances_and_weather_rocks_last_eight_turns() {
    let arena = setup(
        set("Snorlax", &["Body Slam"], "Serene Grace", None),
        set("Blissey", &["Softboiled"], "Natural Cure", None),
        |_| {},
    );
    assert_eq!(
        effective_secondary_chance(arena.engine.state(), &arena.rules, A, 3_000),
        6_000
    );
    assert_eq!(
        effective_secondary_chance(arena.engine.state(), &arena.rules, A, 6_000),
        10_000
    );
    assert_eq!(
        effective_secondary_chance(arena.engine.state(), &arena.rules, B, 3_000),
        3_000
    );

    // Drizzle with Damp Rock: eight turns (Gen 6+); without: five.
    let arena = setup(
        set("Pelipper", &["Surf"], "Drizzle", Some("Damp Rock")),
        set("Blissey", &["Softboiled"], "Natural Cure", None),
        |_| {},
    );
    let weather = arena.engine.state().field.weather.unwrap();
    assert_eq!(
        (weather.kind, weather.turns_remaining),
        (WeatherKind::Rain, 8)
    );
    let arena = setup(
        set("Pelipper", &["Surf"], "Drizzle", None),
        set("Blissey", &["Softboiled"], "Natural Cure", None),
        |_| {},
    );
    assert_eq!(
        arena.engine.state().field.weather.unwrap().turns_remaining,
        5
    );
    let _ = AbilityId::DRIZZLE;
}

#[test]
fn a_mid_turn_request_asks_only_for_ordered_out_slots_and_fainted_status_clears_at_turn_end() {
    // Talonflame's U-turn knocks out the last-slot foe: the mid-turn request asks P1 only;
    // P2's empty slot is filled after the turn.
    let mut arena = setup(
        set("Talonflame", &["U-turn"], "Flame Body", None),
        set("Snorlax", &["Swords Dance"], "Thick Fat", None),
        |pokemon| {
            pokemon[1].hp = 1;
            pokemon[1].status = Some(StatusState::new(NonVolatileStatus::Burn));
        },
    );
    let (start, step) = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) = step else {
        panic!("expected a mid-turn forced switch, got {step:?}");
    };
    assert_eq!(replacements[0].required_count, 1);
    assert_eq!(replacements[1].required_count, 0);
    // Still mid-turn: the fainted Snorlax keeps its burn (Showdown shows it until
    // `checkFainted`).
    assert_eq!(
        arena.pokemon(B).status.map(|s| s.kind),
        Some(NonVolatileStatus::Burn)
    );
    let _ = start;
    let choices = [
        pokemon_battle_engine::engine::ForcedSwitchChoice {
            side: SideId::P1,
            replacements: vec![pokemon_battle_engine::engine::ForcedReplacement {
                position: arena.rules.topology().position(SideId::P1, 0).unwrap(),
                pokemon: C,
            }],
        },
        pokemon_battle_engine::engine::ForcedSwitchChoice {
            side: SideId::P2,
            replacements: Vec::new(),
        },
    ];
    arena.engine.resolve_forced_switches(choices).unwrap();
    let from = arena.engine.trace().events().len();
    let step = loop {
        match arena.engine.step().unwrap() {
            EngineStep::Progressed { .. } => {}
            other => break other,
        }
    };
    // End of turn: the faint check clears the status, then P2 is asked for its replacement.
    assert!(arena.has(from, |e| matches!(
        e,
        TraceEvent::StatusCured {
            target: B,
            status: NonVolatileStatus::Burn,
            reason: CureReason::Fainted,
            ..
        }
    )));
    assert_eq!(arena.pokemon(B).status, None);
    let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) = step else {
        panic!("expected the end-of-turn forced switch, got {step:?}");
    };
    assert_eq!(replacements[0].required_count, 0);
    assert_eq!(replacements[1].required_count, 1);
}
