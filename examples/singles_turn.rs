use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::DexResolver;
use pokemon_battle_engine::engine::{ActionChoice, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleState, BattleStats, MoveSlotState, PokemonState, SideState,
};

fn main() {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "custom",
        ))
        .expect("valid rules"),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));

    let p1 = PokemonId(1);
    let p2 = PokemonId(2);
    let state = BattleState::new(
        vec![pokemon(p1, SideId::P1, 120), pokemon(p2, SideId::P2, 90)],
        [side(p1), side(p2)],
        42,
    );

    let mut battle = BattleEngine::try_new(rules.clone(), dex, state).expect("valid battle");
    let p1_target = DeclaredTarget(rules.topology().position(SideId::P2, 0).unwrap());
    let p2_target = DeclaredTarget(rules.topology().position(SideId::P1, 0).unwrap());

    let result = battle
        .resolve_turn([
            choice(SideId::P1, p1, p1_target),
            choice(SideId::P2, p2, p2_target),
        ])
        .expect("turn resolves");

    println!("phase={:?}, turn={}", result.phase, battle.state().turn);
    println!("P1 HP={}", battle.state().pokemon(p1).unwrap().hp);
    println!("P2 HP={}", battle.state().pokemon(p2).unwrap().hp);
    for event in battle.trace().events() {
        println!("{event:?}");
    }
}

fn pokemon(id: PokemonId, side: SideId, speed: u16) -> PokemonState {
    let mut pokemon = PokemonState::new(id, side);
    pokemon.level = 50;
    pokemon.hp = 300;
    pokemon.max_hp = 300;
    pokemon.stats = BattleStats {
        speed,
        ..BattleStats::default()
    };
    pokemon.moves = vec![MoveSlotState::new(MoveId::TACKLE, 35)];
    pokemon
}

fn side(active: PokemonId) -> SideState {
    let mut side = SideState::new(vec![active], 1);
    side.active[0] = Some(active);
    side
}

fn choice(side: SideId, actor: PokemonId, target: DeclaredTarget) -> SideChoice {
    SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: Some(target),
                mechanic: None,
            },
        }],
    }
}
