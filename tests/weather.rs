//! L1 rule tests for the weather lifecycle (ROADMAP §8.3 "Weather", DESIGN.md §23.1): weather
//! moves and abilities set it, its upkeep is a field-scoped residual (tick first, then sandstorm
//! / hail chip on everyone the generation does not shield), it boosts a Rock type's Special
//! Defense under sandstorm (Gen 4+) and an Ice type's Defense under snow (Gen 9), and the
//! generation's residual table decides where it runs relative to Leftovers.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, StatusMoveEffect, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, EffectSource, EngineStep, MoveFailReason, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, Fraction, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, PokemonState, RandomOverride, RandomRequestKind, SideState, WeatherKind,
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

/// P1 = [A, C (bench)]; P2 = [B]. The leads' switch-in (weather abilities) runs before the
/// first request, as in a compiled scenario.
fn setup(generation: Generation, p1: PokemonSet, p2: PokemonSet, bench: PokemonSet) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &bench).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B])], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

fn bench() -> PokemonSet {
    set("Vaporeon", &["Tackle"], None, None)
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

    fn weather(&self) -> Option<(WeatherKind, u8)> {
        self.engine
            .state()
            .field
            .weather
            .map(|w| (w.kind, w.turns_remaining))
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

fn weather_damage(e: &TraceEvent, target: PokemonId) -> bool {
    matches!(e, TraceEvent::WeatherDamage { target: t, .. } if *t == target)
}

fn weather_immune(e: &TraceEvent, target: PokemonId) -> bool {
    matches!(e, TraceEvent::WeatherImmune { target: t, .. } if *t == target)
}

#[test]
fn weather_moves_and_abilities_classify_from_snapshot_data() {
    let gen9 = DexResolver::resolve(Generation::Gen9);
    for (name, kind) in [
        ("Sunny Day", WeatherKind::Sun),
        ("Rain Dance", WeatherKind::Rain),
        ("Sandstorm", WeatherKind::Sandstorm),
        ("Snowscape", WeatherKind::Snow),
    ] {
        assert_eq!(
            gen9.move_by_name(name).unwrap().primary_effect,
            PrimaryEffect::Status(StatusMoveEffect::Weather(kind)),
            "{name}"
        );
    }
    assert_eq!(
        DexResolver::resolve(Generation::Gen4)
            .move_by_name("Hail")
            .unwrap()
            .primary_effect,
        PrimaryEffect::Status(StatusMoveEffect::Weather(WeatherKind::Snow))
    );
    for name in ["Drought", "Drizzle", "Sand Stream", "Snow Warning"] {
        assert!(gen9.ability_by_name(name).unwrap().implemented, "{name}");
    }
}

#[test]
fn sandstorm_chips_a_sixteenth_at_the_upkeep_and_shields_rock_ground_and_steel() {
    let mut arena = setup(
        Generation::Gen9,
        set("Tyranitar", &["Swords Dance"], Some("Sand Stream"), None),
        set("Snorlax", &["Swords Dance"], None, None),
        bench(),
    );
    // Sand Stream on entry: five turns from Gen 6.
    assert_eq!(arena.weather(), Some((WeatherKind::Sandstorm, 5)));
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    // The upkeep ticks first (5 → 4), then chips floor(235 / 16) = 14 off Snorlax; the Rock
    // type is shielded.
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::WeatherUpkeep {
            weather: WeatherKind::Sandstorm,
            turns_remaining: 4
        }
    )));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::WeatherDamage {
            target: B,
            weather: WeatherKind::Sandstorm,
            damage: 14
        }
    )));
    assert!(arena.has(start, |e| weather_immune(e, A)));
    assert_eq!(arena.pokemon(B).hp, 235 - 14);
    assert_eq!(arena.pokemon(A).hp, arena.pokemon(A).max_hp);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DamageApplied {
            target: B,
            amount: 14,
            source: EffectSource::Weather(WeatherKind::Sandstorm),
            ..
        }
    )));
}

#[test]
fn the_weather_ticks_before_leftovers_and_ends_on_the_fifth_upkeep() {
    let mut arena = setup(
        Generation::Gen9,
        set("Garchomp", &["Sandstorm", "Swords Dance"], None, None),
        set("Snorlax", &["Swords Dance"], None, Some("Leftovers")),
        bench(),
    );
    // Turn 1: Sandstorm the move. At the upkeep the chip (weather 1) comes before Leftovers
    // (5.4): a full-HP Snorlax loses 14 and heals 14, ending the turn at full HP.
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::WeatherChanged {
            from: None,
            to: Some(WeatherKind::Sandstorm)
        }
    )));
    assert_eq!(arena.weather(), Some((WeatherKind::Sandstorm, 4)));
    let chip = arena.position(start, |e| weather_damage(e, B)).unwrap();
    let leftovers = arena
        .position(start, |e| {
            matches!(
                e,
                TraceEvent::ResidualEffect {
                    holder: B,
                    kind: ResidualKind::Leftovers,
                    ..
                }
            )
        })
        .unwrap();
    assert!(chip < leftovers);
    assert_eq!(arena.pokemon(B).hp, arena.pokemon(B).max_hp);
    // The Ground type is shielded.
    assert!(arena.has(start, |e| weather_immune(e, A)));

    // Turn 2: Sandstorm under sandstorm fails (Gen 3+).
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: A,
            reason: MoveFailReason::NoEffect,
            ..
        }
    )));
    assert_eq!(arena.weather(), Some((WeatherKind::Sandstorm, 3)));

    // Turns 3-5: the fifth upkeep ends the weather instead of chipping.
    arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    assert_eq!(arena.weather(), Some((WeatherKind::Sandstorm, 1)));
    let start = arena.turn(Action::Move(1), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::WeatherChanged {
            from: Some(WeatherKind::Sandstorm),
            to: None
        }
    )));
    assert!(!arena.has(start, |e| weather_damage(e, B)));
    assert_eq!(arena.weather(), None);
}

#[test]
fn hail_chips_before_gen_9_and_snow_boosts_an_ice_types_defense_from_gen_9() {
    // Gen 4: Snow Warning's hail is permanent and chips the non-Ice Pokémon.
    let mut arena = setup(
        Generation::Gen4,
        set("Abomasnow", &["Swords Dance"], Some("Snow Warning"), None),
        set("Snorlax", &["Swords Dance"], None, None),
        bench(),
    );
    assert_eq!(arena.weather(), Some((WeatherKind::Snow, 0)));
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| weather_damage(e, B)));
    assert!(arena.has(start, |e| weather_immune(e, A)));
    assert_eq!(arena.weather(), Some((WeatherKind::Snow, 0)));

    // Gen 9: snow chips nobody and lasts five turns; the Ice type's Defense is x1.5 (floor).
    let mut arena = setup(
        Generation::Gen9,
        set("Abomasnow", &["Swords Dance"], Some("Snow Warning"), None),
        set("Snorlax", &["Tackle"], None, None),
        bench(),
    );
    assert_eq!(arena.weather(), Some((WeatherKind::Snow, 5)));
    let defense = arena.pokemon(A).stats.defense;
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(!arena.has(start, |e| weather_damage(e, B)));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DefensiveStatResolved { target: A, value, .. }
            if *value == u32::from(defense) * 3 / 2
    )));
    assert_eq!(arena.weather(), Some((WeatherKind::Snow, 4)));
}

#[test]
fn sandstorm_boosts_a_rock_types_special_defense_from_gen_4() {
    for (generation, boosted) in [(Generation::Gen3, false), (Generation::Gen9, true)] {
        let mut arena = setup(
            generation,
            set("Tyranitar", &["Swords Dance"], Some("Sand Stream"), None),
            set("Snorlax", &["Surf"], None, None),
            bench(),
        );
        let special_defense = arena.pokemon(A).stats.special_defense;
        let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
        let expected = if boosted {
            u32::from(special_defense) * 3 / 2
        } else {
            u32::from(special_defense)
        };
        assert!(
            arena.has(start, |e| matches!(
                e,
                TraceEvent::DefensiveStatResolved { target: A, value, .. } if *value == expected
            )),
            "{generation:?}"
        );
    }
}

#[test]
fn magic_guard_and_being_underground_refuse_the_chip() {
    // Magic Guard: the chip is requested and refused (no HP change).
    let mut arena = setup(
        Generation::Gen9,
        set("Clefable", &["Swords Dance"], Some("Magic Guard"), None),
        set("Tyranitar", &["Swords Dance"], Some("Sand Stream"), None),
        bench(),
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| weather_damage(e, A)));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::DamagePrevented {
            target: A,
            source: EffectSource::Weather(WeatherKind::Sandstorm),
            by: EffectSource::Ability {
                ability: AbilityId::MAGIC_GUARD,
                ..
            },
            ..
        }
    )));
    assert_eq!(arena.pokemon(A).hp, arena.pokemon(A).max_hp);

    // Underground (Dig's charging turn): shielded at that upkeep, chipped once back up.
    let mut arena = setup(
        Generation::Gen9,
        set("Snorlax", &["Dig"], None, None),
        set("Tyranitar", &["Swords Dance"], Some("Sand Stream"), None),
        bench(),
    );
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| weather_immune(e, A)));
    assert_eq!(arena.pokemon(A).hp, arena.pokemon(A).max_hp);
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert!(arena.has(start, |e| weather_damage(e, A)));
}

#[test]
fn a_weather_ability_never_resets_the_same_weather() {
    let mut arena = setup(
        Generation::Gen9,
        set("Tyranitar", &["Swords Dance"], Some("Sand Stream"), None),
        set("Snorlax", &["Swords Dance"], None, None),
        set("Hippowdon", &["Tackle"], Some("Sand Stream"), None),
    );
    assert_eq!(arena.weather(), Some((WeatherKind::Sandstorm, 5)));
    // Turn 1: Hippowdon comes in under sandstorm (4 left after the upkeep, not 5 again).
    let start = arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    assert!(!arena.has(start, |e| matches!(e, TraceEvent::WeatherChanged { .. })));
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::AbilityActivated {
            ability: AbilityId::SAND_STREAM,
            ..
        }
    )));
    assert_eq!(arena.weather(), Some((WeatherKind::Sandstorm, 4)));
}

#[test]
fn residual_order_tables_follow_the_generation() {
    let residual = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .residual()
    };
    let modern = residual(Generation::Gen9);
    assert_eq!(modern.key(ResidualKind::Weather), Some((1, 0)));
    assert_eq!(modern.key(ResidualKind::Leftovers), Some((5, 4)));
    assert!(modern.key(ResidualKind::Taunt) < modern.key(ResidualKind::Disable));
    for generation in [Generation::Gen3, Generation::Gen4] {
        let old = residual(generation);
        assert_eq!(
            old.key(ResidualKind::Weather),
            Some((8, 0)),
            "{generation:?}"
        );
        assert_eq!(old.key(ResidualKind::Leftovers), Some((10, 4)));
        assert_eq!(old.key(ResidualKind::LeechSeed), Some((10, 5)));
        assert_eq!(old.key(ResidualKind::Burn), Some((10, 6)));
        assert!(old.key(ResidualKind::Encore) < old.key(ResidualKind::Taunt));
    }
    // Gen 4 counts Disable, Encore, Taunt down in that order; Gen 5+ the other way round; Gen 3
    // drops Disable's order, which puts it after everything else.
    let gen4 = residual(Generation::Gen4);
    assert!(gen4.key(ResidualKind::Disable) < gen4.key(ResidualKind::Encore));
    let gen3 = residual(Generation::Gen3);
    assert_eq!(gen3.key(ResidualKind::Disable), Some((u8::MAX, 0)));
    // Gen 2: poison, burn and Leech Seed are not end-of-turn residuals; the freeze thaws at 7.
    let gen2 = residual(Generation::Gen2);
    assert_eq!(gen2.key(ResidualKind::Weather), Some((2, 0)));
    assert_eq!(gen2.key(ResidualKind::Leftovers), Some((5, 1)));
    assert_eq!(gen2.key(ResidualKind::FreezeThaw), Some((7, 0)));
    for kind in [
        ResidualKind::Poison,
        ResidualKind::Burn,
        ResidualKind::LeechSeed,
    ] {
        assert_eq!(gen2.key(kind), None, "{kind:?}");
    }
}

#[test]
fn sun_boosts_hydro_steam_where_it_weakens_every_other_water_move() {
    let mut arena = setup(
        Generation::Gen9,
        set("Torkoal", &["Hydro Steam", "Surf"], Some("Drought"), None),
        set("Snorlax", &["Swords Dance"], None, None),
        bench(),
    );
    let modifier = |arena: &Arena, from: usize| {
        arena.events(from).iter().find_map(|e| match e {
            TraceEvent::WeatherModifierResolved {
                actor: A,
                weather: Some(WeatherKind::Sun),
                modifier,
                ..
            } => Some(*modifier),
            _ => None,
        })
    };
    let start = arena.turn(Action::Move(0), Action::Move(0), hit(100));
    assert_eq!(modifier(&arena, start), Some(Fraction::new(3, 2)));
    let start = arena.turn(Action::Move(1), Action::Move(0), hit(100));
    assert_eq!(modifier(&arena, start), Some(Fraction::new(1, 2)));
}

#[test]
fn before_gen_6_a_weather_ability_makes_the_same_timed_weather_permanent() {
    // Showdown `setWeather`: an ability is only stopped by the same weather when that one is
    // already permanent (`duration 0`) before Gen 6.
    let mut arena = setup(
        Generation::Gen4,
        set("Snorlax", &["Sandstorm"], None, None),
        set("Blissey", &["Swords Dance"], None, None),
        set("Hippowdon", &["Tackle"], Some("Sand Stream"), None),
    );
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(arena.weather(), Some((WeatherKind::Sandstorm, 4)));
    let start = arena.turn(Action::Switch(C), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::AbilityActivated {
            ability: AbilityId::SAND_STREAM,
            ..
        }
    )));
    assert_eq!(arena.weather(), Some((WeatherKind::Sandstorm, 0)));
}

#[test]
fn a_terrain_move_fails_under_the_same_terrain() {
    // Showdown `setTerrain`: the same terrain already up makes the move fail, turns untouched.
    let mut arena = setup(
        Generation::Gen9,
        set("Pikachu", &["Electric Terrain"], None, None),
        set("Snorlax", &["Swords Dance"], None, None),
        bench(),
    );
    let terrain = |arena: &Arena| {
        arena
            .engine
            .state()
            .field
            .terrain
            .map(|t| t.turns_remaining)
    };
    arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert_eq!(terrain(&arena), Some(4));
    let start = arena.turn(Action::Move(0), Action::Move(0), Vec::new());
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed { actor: A, .. }
    )));
    assert_eq!(terrain(&arena), Some(3));
}
