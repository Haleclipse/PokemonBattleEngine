//! L1 rule tests for R162 / R163 (DESIGN.md §20.1) where the replay cannot tell: the replay
//! asks `engine::critical_roll` whether a Critical decision is drawn and plans it from the log, so
//! the chance itself — Focus Energy's, Dragon Cheer's, Laser Focus's — is only visible here; and
//! it never tries a switch No Retreat forbids.

use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{CriticalRoll, critical_roll};
use pokemon_battle_engine::ids::{PokemonId, SideId};
use pokemon_battle_engine::rules::{CriticalTable, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, PokemonState, SideState, VolatileConditionKind};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

/// `species` using `move_name` on a Snorlax, after `prepare` set its volatiles.
fn roll(
    generation: Generation,
    species: &str,
    move_name: &str,
    prepare: impl FnOnce(&mut PokemonState),
) -> CriticalRoll {
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let mut a = build_pokemon(&dex, A, SideId::P1, &PokemonSet::new(species, [move_name])).unwrap();
    prepare(&mut a);
    let b = build_pokemon(&dex, B, SideId::P2, &PokemonSet::new("Snorlax", ["Harden"])).unwrap();
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![a, b], [side(A), side(B)], 1);
    critical_roll(
        &state,
        &rules,
        &dex,
        A,
        B,
        dex.move_by_name(move_name).unwrap(),
    )
}

fn chance(table: CriticalTable, ratio: u8) -> CriticalRoll {
    let (numerator, denominator) = table.chance(ratio).unwrap();
    CriticalRoll::Chance(numerator, denominator)
}

#[test]
fn focus_energy_adds_one_stage_in_gen_2_and_two_from_gen_3() {
    for (generation, stages) in [
        (Generation::Gen2, 1),
        (Generation::Gen3, 2),
        (Generation::Gen9, 2),
    ] {
        let table =
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .unwrap()
                .damage()
                .critical;
        let focused = roll(generation, "Mew", "Tackle", |p| {
            p.volatiles.set_marker(VolatileConditionKind::FocusEnergy);
        });
        assert_eq!(focused, chance(table, 1 + stages), "{generation:?}");
    }
}

#[test]
fn gen_1_focus_energy_halves_the_critical_chance() {
    // Persian (base Speed 115): Tackle 57 / 256, under Focus Energy 14 / 256.
    let focused = roll(Generation::Gen1, "Persian", "Tackle", |p| {
        p.volatiles.set_marker(VolatileConditionKind::FocusEnergy);
    });
    assert_eq!(focused, CriticalRoll::Chance(14, 256));
    let plain = roll(Generation::Gen1, "Persian", "Tackle", |_| {});
    assert_eq!(plain, CriticalRoll::Chance(57, 256));
}

#[test]
fn dragon_cheer_adds_its_stages_and_laser_focus_is_certain() {
    let table = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen9,
        BattleMode::Singles,
        "custom",
    ))
    .unwrap()
    .damage()
    .critical;
    for stages in [1, 2] {
        let cheered = roll(Generation::Gen9, "Mew", "Tackle", |p| {
            p.volatiles.set_dragon_cheer(stages);
        });
        assert_eq!(cheered, chance(table, 1 + stages));
    }
    let focused = roll(Generation::Gen8, "Mew", "Tackle", |p| {
        p.volatiles.set_laser_focus(2);
    });
    assert_eq!(focused, CriticalRoll::Always);
}

/// Dragon Cheer through the engine in doubles: +2 on a Dragon-type partner, +1 on another.
#[test]
fn dragon_cheer_gives_a_dragon_type_two_stages() {
    use std::sync::Arc;

    use pokemon_battle_engine::BattleEngine;
    use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
    use pokemon_battle_engine::ids::MoveSlot;
    use pokemon_battle_engine::rules::DeclaredTarget;

    let stages_given = |partner: &str| {
        let rules = Arc::new(
            RulesResolver::resolve(&BattleSpec::new(
                Generation::Gen9,
                BattleMode::Doubles,
                "custom",
            ))
            .unwrap(),
        );
        let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
        let ids = [PokemonId(1), PokemonId(2), PokemonId(3), PokemonId(4)];
        let sets = [
            ("Mew", SideId::P1, "Dragon Cheer"),
            (partner, SideId::P1, "Harden"),
            ("Snorlax", SideId::P2, "Harden"),
            ("Chansey", SideId::P2, "Harden"),
        ];
        let pokemon = ids
            .iter()
            .zip(sets)
            .map(|(id, (species, side, move_name))| {
                build_pokemon(&dex, *id, side, &PokemonSet::new(species, [move_name])).unwrap()
            })
            .collect();
        let side = |team: [PokemonId; 2]| {
            let mut side = SideState::new(team.to_vec(), 2);
            side.active = vec![Some(team[0]), Some(team[1])];
            side
        };
        let state = BattleState::new(pokemon, [side([ids[0], ids[1]]), side([ids[2], ids[3]])], 1);
        let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
        let slot = |actor: PokemonId, target: Option<(SideId, u8)>| SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: target
                    .map(|(s, i)| DeclaredTarget(rules.topology().position(s, i).unwrap())),
                mechanic: None,
            },
        };
        engine
            .submit_turn_choices([
                SideChoice {
                    side: SideId::P1,
                    actions: vec![slot(ids[0], Some((SideId::P1, 1))), slot(ids[1], None)],
                },
                SideChoice {
                    side: SideId::P2,
                    actions: vec![slot(ids[2], None), slot(ids[3], None)],
                },
            ])
            .unwrap();
        while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
        engine
            .state()
            .pokemon(ids[1])
            .unwrap()
            .volatiles
            .dragon_cheer_stages()
    };
    assert_eq!(stages_given("Dragonite"), 2);
    assert_eq!(stages_given("Snorlax"), 1);
}

/// No Retreat's volatile keeps its holder from switching (`onTrapPokemon`) — the replay never
/// tries a switch it could not make, so only here.
#[test]
fn no_retreat_traps_its_holder() {
    let rules = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen9,
        BattleMode::Singles,
        "custom",
    ))
    .unwrap();
    let dex = DexResolver::resolve(Generation::Gen9);
    let mut a = build_pokemon(
        &dex,
        A,
        SideId::P1,
        &PokemonSet::new("Falinks", ["No Retreat"]),
    )
    .unwrap();
    let b = build_pokemon(&dex, B, SideId::P2, &PokemonSet::new("Snorlax", ["Harden"])).unwrap();
    let state_with = |a: PokemonState| {
        let side = |id: PokemonId| {
            let mut side = SideState::new(vec![id], 1);
            side.active[0] = Some(id);
            side
        };
        BattleState::new(vec![a, b.clone()], [side(A), side(B)], 1)
    };
    let free = state_with(a.clone());
    assert_eq!(
        pokemon_battle_engine::engine::trapped_by(&free, &rules, A),
        None
    );
    a.volatiles.set_marker(VolatileConditionKind::NoRetreat);
    let held = state_with(a);
    assert_eq!(
        pokemon_battle_engine::engine::trapped_by(&held, &rules, A),
        Some(VolatileConditionKind::NoRetreat)
    );
}
