//! L1 rule tests for M7 batch 2 (DESIGN.md §22.7): move formulas as data — base power
//! formulas, conditional BasePower participants, weather accuracy and weather healing, each
//! gated by the generation's declared callbacks — and the end-of-turn durations a battle
//! decided in the residual never reaches.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, ResolvedDex, ResolvedMove, build_pokemon,
};
use pokemon_battle_engine::effects::moves::WeatherHeal;
use pokemon_battle_engine::engine::{
    ActionChoice, CriticalRoll, EngineStep, MoveUse, SideChoice, SlotChoice, accuracy_roll,
    critical_roll, formula_power, weather_heal_amount,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, RandomOverride, RandomRequestKind, SideState,
    StatusState, TerrainKind, WeatherKind,
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

fn singles(
    generation: Generation,
    [a, b, c, d]: [PokemonSet; 4],
    prepare: impl FnOnce(&mut BattleState),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &a).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &b).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &c).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &d).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let mut state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B, D])], 1);
    prepare(&mut state);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn state(&self) -> &BattleState {
        self.engine.state()
    }

    fn move_def(&self, id: PokemonId, slot: usize) -> ResolvedMove {
        let move_id = self.state().pokemon(id).unwrap().moves[slot].move_id;
        self.dex.move_by_id(move_id).unwrap().clone()
    }

    fn power(&self, slot: usize) -> u16 {
        let move_def = self.move_def(A, slot);
        let formula = move_def.behavior.power_formula.expect("a power formula");
        formula_power(
            self.state(),
            &self.rules,
            &self.dex,
            A,
            B,
            &move_def,
            formula,
            MoveUse::FIRST,
        )
    }

    fn turn(&mut self, tape: Vec<RandomOverride>) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let choice = |arena: &Arena, side: SideId| {
            let actor = arena.state().side(side).active[0].unwrap();
            let move_def = arena.move_def(actor, 0);
            let target = matches!(move_def.target, TargetPattern::SelectedAdjacent)
                .then(|| DeclaredTarget(arena.rules.topology().position(side.other(), 0).unwrap()));
            SideChoice {
                side,
                actions: vec![SlotChoice {
                    actor,
                    action: ActionChoice::Move {
                        slot: MoveSlot::FIRST,
                        target,
                        mechanic: None,
                    },
                }],
            }
        };
        let choices = [choice(self, SideId::P1), choice(self, SideId::P2)];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(
            self.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
    }
}

fn hp(state: &mut BattleState, id: PokemonId, hp: u16) {
    state.pokemon_mut(id).unwrap().hp = hp;
}

#[test]
fn hp_formulas_follow_the_generation() {
    // Eruption: 150 × HP / max HP, floored. Flail at the same HP: Gen 9 48ths, Gen 4 64ths.
    let arena = |generation, fraction: (u16, u16)| {
        singles(
            generation,
            [
                set("Typhlosion", &["Eruption", "Flail"]),
                set("Snorlax", &["Crush Grip"]),
                filler(),
                filler(),
            ],
            |state| {
                let max = state.pokemon(A).unwrap().max_hp;
                hp(state, A, max * fraction.0 / fraction.1);
            },
        )
    };
    let gen9 = arena(Generation::Gen9, (1, 2));
    let max = gen9.state().pokemon(A).unwrap().max_hp;
    let current = gen9.state().pokemon(A).unwrap().hp;
    assert_eq!(gen9.power(0), 150 * current / max);
    // Flail bands (Typhlosion, 153 max HP). One eighth (19 HP): Gen 9 ⌊5.96⌋ → 100, Gen 4
    // ⌊7.9⌋ → 100. Two fifths (61): Gen 9 19 → 40, Gen 4 25 → 40. One tenth (15): Gen 9
    // ⌊4.7⌋ → 150 (below 5), Gen 4 ⌊6.3⌋ → 100 (6 or more) — the scales part ways.
    for (fraction, gen9_power, gen4_power) in
        [((1, 8), 100, 100), ((2, 5), 40, 40), ((1, 10), 150, 100)]
    {
        let gen9 = arena(Generation::Gen9, fraction);
        let gen4 = arena(Generation::Gen4, fraction);
        assert_eq!(
            (gen9.power(1), gen4.power(1)),
            (gen9_power, gen4_power),
            "{fraction:?}"
        );
    }
}

#[test]
fn crush_grip_is_linear_in_gen_4_and_percentage_based_from_gen_5() {
    let power = |generation| {
        let arena = singles(
            generation,
            [
                set("Snorlax", &["Crush Grip"]),
                set("Blissey", &["Softboiled"]),
                filler(),
                filler(),
            ],
            |state| hp(state, B, 1),
        );
        arena.power(0)
    };
    // Gen 4: ⌊1 × 120 / max⌋ + 1 = 1; Gen 9: the percentage rounds to 0 → at least 1.
    assert_eq!(power(Generation::Gen4), 1);
    assert_eq!(power(Generation::Gen9), 1);
    let full = |generation| {
        singles(
            generation,
            [
                set("Snorlax", &["Crush Grip"]),
                set("Blissey", &["Softboiled"]),
                filler(),
                filler(),
            ],
            |_| {},
        )
        .power(0)
    };
    assert_eq!(full(Generation::Gen4), 121);
    assert_eq!(full(Generation::Gen9), 120);
}

#[test]
fn stage_weight_status_and_terrain_formulas() {
    let arena = singles(
        Generation::Gen9,
        [
            set("Espeon", &["Stored Power", "Low Kick", "Heavy Slam", "Hex"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |state| {
            let espeon = state.pokemon_mut(A).unwrap();
            espeon.stages.special_attack = 2;
            espeon.stages.speed = 1;
            espeon.stages.accuracy = -1;
            state.pokemon_mut(B).unwrap().status =
                Some(StatusState::new(NonVolatileStatus::Paralysis));
            state.field.set_terrain(TerrainKind::Electric, A, 5);
        },
    );
    // 20 + 20 × 3 positive stages.
    assert_eq!(arena.power(0), 80);
    // Snorlax weighs 460 kg (4600 hg): Low Kick 120; Espeon (26.5 kg) against it: Heavy
    // Slam 40.
    assert_eq!(arena.power(1), 120);
    assert_eq!(arena.power(2), 40);
    // Hex doubles against a statused target.
    assert_eq!(arena.power(3), 130);
    // Rising Voltage doubles in Electric Terrain against a grounded target, not a Levitating
    // one.
    for (foe, doubled) in [("Snorlax", true), ("Gengar", false)] {
        let mut foe_set = set(foe, &["Swords Dance"]);
        if foe == "Gengar" {
            foe_set.ability = Some("Levitate".into());
        }
        let arena = singles(
            Generation::Gen9,
            [
                set("Jolteon", &["Rising Voltage"]),
                foe_set,
                filler(),
                filler(),
            ],
            |state| {
                state.field.set_terrain(TerrainKind::Electric, A, 5);
            },
        );
        assert_eq!(arena.power(0), if doubled { 140 } else { 70 }, "{foe}");
    }
    // Return in Gen 4: happiness 255 → 102.
    let gen4 = singles(
        Generation::Gen4,
        [set("Snorlax", &["Return"]), filler(), filler(), filler()],
        |_| {},
    );
    assert_eq!(gen4.power(0), 102);
}

#[test]
fn weather_accuracy_and_the_generations_that_declare_it() {
    let accuracy = |generation, weather: Option<WeatherKind>, slot: usize| {
        let arena = singles(
            generation,
            [
                set("Jolteon", &["Thunder", "Blizzard"]),
                set("Snorlax", &["Swords Dance"]),
                filler(),
                filler(),
            ],
            |state| {
                if let Some(weather) = weather {
                    state.field.set_weather(weather, A, 5);
                }
            },
        );
        let move_def = arena.move_def(A, slot);
        accuracy_roll(arena.state(), &arena.rules, A, B, &move_def)
    };
    assert_eq!(accuracy(Generation::Gen9, Some(WeatherKind::Rain), 0), None);
    assert_eq!(
        accuracy(Generation::Gen9, Some(WeatherKind::Sun), 0),
        Some((50, 100))
    );
    // Blizzard never misses in hail from Gen 4; Gen 3's Blizzard declares no such callback
    // (the cartridge's whole percent, DESIGN.md §15.7).
    assert_eq!(accuracy(Generation::Gen4, Some(WeatherKind::Snow), 1), None);
    assert_eq!(
        accuracy(Generation::Gen3, Some(WeatherKind::Snow), 1),
        Some((70, 100))
    );
}

#[test]
fn growth_doubles_in_sun_only_where_the_generation_says_so() {
    let doubles = |generation| {
        DexResolver::resolve(generation)
            .move_by_name("Growth")
            .unwrap()
            .behavior
            .sun_doubles_boosts
    };
    assert!(!doubles(Generation::Gen2));
    assert!(!doubles(Generation::Gen4));
    assert!(doubles(Generation::Gen5));
    assert!(doubles(Generation::Gen9));
}

#[test]
fn weather_healing_follows_the_generation() {
    let heal = |generation, weather: Option<WeatherKind>, kind| {
        let arena = singles(
            generation,
            [set("Umbreon", &["Moonlight"]), filler(), filler(), filler()],
            |state| {
                if let Some(weather) = weather {
                    state.field.set_weather(weather, A, 5);
                }
            },
        );
        let max = arena.state().pokemon(A).unwrap().max_hp;
        (
            weather_heal_amount(arena.state(), &arena.rules, A, kind),
            max,
        )
    };
    let (full, max) = heal(
        Generation::Gen2,
        Some(WeatherKind::Sun),
        WeatherHeal::Sunlight,
    );
    assert_eq!(full, max);
    let (two_thirds, max) = heal(
        Generation::Gen4,
        Some(WeatherKind::Sun),
        WeatherHeal::Sunlight,
    );
    assert_eq!(two_thirds, max * 2 / 3);
    let (modified, max) = heal(
        Generation::Gen9,
        Some(WeatherKind::Sun),
        WeatherHeal::Sunlight,
    );
    assert_eq!(u32::from(modified), (u32::from(max) * 2732 + 2047) / 4096);
    let (quarter, max) = heal(
        Generation::Gen9,
        Some(WeatherKind::Rain),
        WeatherHeal::Sunlight,
    );
    assert_eq!(u32::from(quarter), (u32::from(max) * 1024 + 2047) / 4096);
    // Shore Up ignores rain; sand boosts it.
    let (half, max) = heal(Generation::Gen9, Some(WeatherKind::Rain), WeatherHeal::Sand);
    assert_eq!(u32::from(half), (u32::from(max) * 2048 + 2047) / 4096);
}

#[test]
fn gen_2_flail_neither_crits_nor_rolls() {
    let arena = singles(
        Generation::Gen2,
        [set("Heracross", &["Flail"]), filler(), filler(), filler()],
        |_| {},
    );
    let flail = arena.move_def(A, 0);
    assert!(flail.no_damage_variance);
    assert_eq!(
        critical_roll(arena.state(), &arena.rules, &arena.dex, A, B, &flail),
        CriticalRoll::Never
    );
}

#[test]
fn facade_ignores_the_burn_from_gen_6() {
    for (generation, halved) in [(Generation::Gen5, true), (Generation::Gen6, false)] {
        let mut arena = singles(
            generation,
            [
                set("Snorlax", &["Facade"]),
                set("Chansey", &["Softboiled"]),
                filler(),
                filler(),
            ],
            |state| {
                state.pokemon_mut(A).unwrap().status =
                    Some(StatusState::new(NonVolatileStatus::Burn));
            },
        );
        let start = arena.engine.trace().events().len();
        arena.turn(vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, 100),
        ]);
        let burned = arena.engine.trace().events()[start..].iter().any(|e| {
            matches!(
                e,
                pokemon_battle_engine::engine::TraceEvent::DamageCalculated {
                    burn_applied: true,
                    ..
                }
            )
        });
        assert_eq!(burned, halved, "{generation:?}");
    }
}

#[test]
fn a_battle_decided_in_the_residual_leaves_the_durations_alone() {
    // P2's last Pokémon faints to poison at the end of the turn: Electric Terrain keeps the
    // turns it had (Showdown stops the residual before the terrain's tick).
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Jolteon", &["Electric Terrain"]),
            // Not Snorlax: its first ability, Immunity, cures the poison at the next Update.
            set("Blissey", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |state| {
            state.pokemon_mut(B).unwrap().status =
                Some(StatusState::new(NonVolatileStatus::Poison));
            hp(state, B, 1);
            hp(state, D, 0);
        },
    );
    arena.turn(Vec::new());
    assert!(arena.state().outcome.is_some());
    assert_eq!(
        arena.state().field.terrain.map(|t| t.turns_remaining),
        Some(5)
    );
}
