//! L2 damage micro-oracle: the Rust damage pipeline against pinned `@smogon/calc` rolls.
//!
//! Each case in `tests/fixtures/oracle/damage-expected.json` is replayed sixteen times with the
//! semantic random tape forcing hit / no-crit / roll = 85..=100, and the committed HP loss on the
//! defender must match calc's roll table value for value. A mismatch is a disagreement to
//! classify (TESTING.md §1); it is never auto-corrected.

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonType, ResolvedDex};
use pokemon_battle_engine::engine::{
    ActionChoice, CriticalRoll, EffectSource, SideChoice, SlotChoice, TraceEvent, accuracy_roll,
    critical_roll, fixed_damage,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, BattleStats, MoveSlotState, NonVolatileStatus, PokemonState,
    PokemonTypes, RandomOverride, RandomRequestKind, SideState, StatusState, TerrainKind,
    WeatherKind,
};
use support::{DamageCase, DamageExpectations, DamageSide, load_json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn damage_pipeline_agrees_with_smogon_calc_roll_tables() {
    let expectations: DamageExpectations =
        load_json(&root().join("tests/fixtures/oracle/damage-expected.json"));
    assert_eq!(expectations.provenance.engine, "@smogon/calc");
    assert_eq!(expectations.provenance.version, "0.11.0");
    assert!(!expectations.cases.is_empty());

    let mut report = Vec::new();
    for case in &expectations.cases {
        let generation = generation(case.generation);
        let mode = match case.mode.as_str() {
            "doubles" => BattleMode::Doubles,
            _ => BattleMode::Singles,
        };
        let rules = Arc::new(
            RulesResolver::resolve(&BattleSpec::new(generation, mode, "custom"))
                .unwrap_or_else(|error| panic!("{}: {error:?}", case.id)),
        );
        let dex = Arc::new(DexResolver::resolve(generation));
        // 85..=100 from Gen 3; 217..=255 in Gen 1-2.
        let damage = rules.damage();
        let rolls = damage.random_min..=damage.random_max;
        assert_eq!(
            case.expected.damage.len(),
            rolls.clone().count(),
            "{}",
            case.id
        );
        let rust: Vec<u16> = rolls
            .clone()
            .map(|roll| single_roll(case, &rules, &dex, roll))
            .collect();
        if rust != case.expected.damage {
            let first = rust
                .iter()
                .zip(&case.expected.damage)
                .position(|(r, o)| r != o)
                .unwrap_or(0);
            report.push(format!(
                "{}: first divergence at roll {}: rust={} calc={}\n    rust={:?}\n    calc={:?}\n    ({})",
                case.id,
                damage.random_min + first as u16,
                rust[first],
                case.expected.damage[first],
                rust,
                case.expected.damage,
                case.description.as_deref().unwrap_or("")
            ));
        }
    }

    assert!(
        report.is_empty(),
        "damage pipeline disagrees with @smogon/calc in {} of {} case(s):\n{}",
        report.len(),
        expectations.cases.len(),
        report.join("\n")
    );
}

/// Runs one turn of the case with the damage roll pinned and returns the HP the defender lost
/// to the attacker's move.
fn single_roll(
    case: &DamageCase,
    rules: &Arc<ResolvedRules>,
    dex: &Arc<ResolvedDex>,
    roll: u16,
) -> u16 {
    let mode = rules.mode();
    let move_def = dex
        .move_by_name(&case.move_name)
        .unwrap_or_else(|| {
            panic!(
                "{}: {} not in gen{} dex",
                case.id, case.move_name, case.generation
            )
        })
        .clone();
    let move_id = move_def.id;

    let attacker = PokemonId(1);
    let defender = PokemonId(2);
    let bystander = PokemonId(3);
    let mut pokemon = vec![
        build(dex, attacker, SideId::P1, &case.attacker, move_id),
        build(dex, defender, SideId::P2, &case.defender, MoveId::TACKLE),
    ];
    let (p1, p2) = match mode {
        BattleMode::Singles => (
            side(vec![attacker], vec![Some(attacker)]),
            side(vec![defender], vec![Some(defender)]),
        ),
        BattleMode::Doubles => {
            pokemon.push(build(
                dex,
                bystander,
                SideId::P2,
                &case.defender,
                MoveId::TACKLE,
            ));
            (
                side(vec![attacker], vec![Some(attacker), None]),
                side(
                    vec![defender, bystander],
                    vec![Some(defender), Some(bystander)],
                ),
            )
        }
    };

    // A SpeedTie decision is drawn only for actions that genuinely tie (the doubles bystander
    // shares the defender's set, so those two always do).
    let speeds: Vec<u16> = pokemon.iter().map(|p| p.stats.speed).collect();
    let tied = speeds
        .iter()
        .filter(|speed| speeds.iter().filter(|other| other == speed).count() > 1)
        .count();
    let mut state = BattleState::new(pokemon, [p1, p2], 1);
    let mut tape = vec![RandomOverride::raw(RandomRequestKind::SpeedTie, 1); tied];
    // The Accuracy decision is on the tape only when the engine will ask for it (a 100 %
    // move rolls in Gen 1 only).
    if accuracy_roll(&state, rules, attacker, defender, &move_def).is_some() {
        tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, true));
    }
    // Fixed damage asks nothing more; otherwise the engine says whether a Critical decision is
    // drawn (Gen 1's chance comes from the species' base Speed, and these Pokémon are built
    // by hand without one).
    if fixed_damage(&state, rules, attacker, defender, &move_def).is_none() {
        if let CriticalRoll::Chance(..) =
            critical_roll(&state, rules, dex, attacker, defender, &move_def)
        {
            tape.push(RandomOverride::bool(RandomRequestKind::Critical, false));
        }
        tape.push(RandomOverride::range(RandomRequestKind::DamageRoll, roll));
    }
    state.rng = BattleRng::with_semantic_tape(1, tape);
    if let Some(weather) = case.field.weather.as_deref() {
        state.field.set_weather(weather_kind(weather), attacker, 5);
    }
    if let Some(terrain) = case.field.terrain.as_deref() {
        state.field.set_terrain(terrain_kind(terrain), attacker, 5);
    }

    let mut battle = BattleEngine::try_new(rules.clone(), dex.clone(), state)
        .unwrap_or_else(|error| panic!("{}: {error:?}", case.id));

    let declared = match move_def.target {
        TargetPattern::SelectedAny
        | TargetPattern::SelectedAdjacent
        | TargetPattern::SelectedFoe
        | TargetPattern::SelectedAdjacentFoe
        | TargetPattern::Ally
        | TargetPattern::AllyOrSelf => Some(DeclaredTarget(
            rules.topology().position(SideId::P2, 0).unwrap(),
        )),
        _ => None,
    };
    let p2_actions = battle
        .state()
        .side(SideId::P2)
        .active
        .iter()
        .flatten()
        .map(|actor| SlotChoice {
            actor: *actor,
            action: ActionChoice::Pass,
        })
        .collect();
    battle
        .resolve_turn([
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: attacker,
                    action: ActionChoice::Move {
                        slot: MoveSlot::FIRST,
                        target: declared,
                        mechanic: None,
                    },
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: p2_actions,
            },
        ])
        .unwrap_or_else(|error| panic!("{}: {error:?}", case.id));

    battle
        .trace()
        .events()
        .iter()
        .find_map(|event| match event {
            TraceEvent::DamageApplied {
                target,
                amount,
                source: EffectSource::Move { .. },
                ..
            } if *target == defender => Some(*amount),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{}: attacker's move never damaged the defender", case.id))
}

fn build(
    dex: &ResolvedDex,
    id: PokemonId,
    side_id: SideId,
    spec: &DamageSide,
    move_id: MoveId,
) -> PokemonState {
    let mut pokemon = PokemonState::new(id, side_id);
    pokemon.level = spec.level;
    pokemon.hp = spec.stats.hp;
    pokemon.max_hp = spec.stats.hp;
    pokemon.stats = BattleStats {
        attack: spec.stats.atk,
        defense: spec.stats.def,
        special_attack: spec.stats.spa,
        special_defense: spec.stats.spd,
        speed: spec.stats.spe,
    };
    pokemon.types = match spec.types.as_slice() {
        [primary] => PokemonTypes::mono(pokemon_type(primary)),
        [primary, secondary] => PokemonTypes::dual(pokemon_type(primary), pokemon_type(secondary)),
        other => panic!("unsupported type list {other:?}"),
    };
    pokemon.ability = ability(dex, spec.ability.as_deref().unwrap_or("None"));
    pokemon.held_item = spec.item.as_deref().map(|name| item(dex, name));
    pokemon.status = spec
        .status
        .as_deref()
        .map(|name| StatusState::new(status(name)));
    pokemon.moves = vec![MoveSlotState::new(move_id, 5)];
    pokemon
}

fn side(team: Vec<PokemonId>, active: Vec<Option<PokemonId>>) -> SideState {
    let mut side = SideState::new(team, active.len());
    side.active = active;
    side
}

fn generation(number: u8) -> Generation {
    match number {
        1 => Generation::Gen1,
        2 => Generation::Gen2,
        3 => Generation::Gen3,
        4 => Generation::Gen4,
        5 => Generation::Gen5,
        6 => Generation::Gen6,
        7 => Generation::Gen7,
        8 => Generation::Gen8,
        9 => Generation::Gen9,
        other => panic!("unsupported generation {other}"),
    }
}

fn pokemon_type(name: &str) -> PokemonType {
    PokemonType::from_name(name).unwrap_or_else(|| panic!("unsupported type {name}"))
}

fn ability(dex: &ResolvedDex, name: &str) -> AbilityId {
    if name == "None" {
        return AbilityId::NONE;
    }
    dex.ability_by_name(name)
        .unwrap_or_else(|| panic!("unknown ability {name}"))
        .id
}

fn item(dex: &ResolvedDex, name: &str) -> ItemId {
    dex.item_by_name(name)
        .unwrap_or_else(|| panic!("unknown item {name}"))
        .id
}

fn status(name: &str) -> NonVolatileStatus {
    match name {
        "brn" => NonVolatileStatus::Burn,
        "par" => NonVolatileStatus::Paralysis,
        "psn" => NonVolatileStatus::Poison,
        "tox" => NonVolatileStatus::Toxic,
        "slp" => NonVolatileStatus::Sleep,
        "frz" => NonVolatileStatus::Freeze,
        other => panic!("unsupported status {other}"),
    }
}

fn weather_kind(name: &str) -> WeatherKind {
    match name {
        "Sun" => WeatherKind::Sun,
        "Rain" => WeatherKind::Rain,
        "Sand" => WeatherKind::Sandstorm,
        "Snow" => WeatherKind::Snow,
        other => panic!("unsupported weather {other}"),
    }
}

fn terrain_kind(name: &str) -> TerrainKind {
    match name {
        "Electric" => TerrainKind::Electric,
        other => panic!("unsupported terrain {other}"),
    }
}
