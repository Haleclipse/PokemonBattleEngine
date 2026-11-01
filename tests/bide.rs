//! L1 test for Bide (DESIGN.md §15.10): what the replay cannot set up. In Gen 1 a faint anywhere
//! wipes what a Bide has stored (Showdown `faintMessages`, the Desync Clause Mod).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const HOLDER: PokemonId = PokemonId(1);
const BOMB: PokemonId = PokemonId(2);
const BENCH: PokemonId = PokemonId(3);

fn set(species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = level;
    set
}

fn first_move(side: SideId, actor: PokemonId) -> SideChoice {
    SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target: None,
                mechanic: None,
            },
        }],
    }
}

#[test]
fn gen1_faint_wipes_the_bide_store() {
    let generation = Generation::Gen1;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut holder =
        build_pokemon(&dex, HOLDER, SideId::P1, &set("Snorlax", &["Bide"], 100)).unwrap();
    assert!(holder.volatiles.start_bide(None, 3, false));
    holder.volatiles.bide_mut().unwrap().damage = 50;
    let pokemon = vec![
        holder,
        build_pokemon(&dex, BOMB, SideId::P2, &set("Electrode", &["Explosion"], 5)).unwrap(),
        build_pokemon(&dex, BENCH, SideId::P2, &set("Snorlax", &["Harden"], 50)).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active = vec![Some(team[0])];
        side
    };
    let state = BattleState::new(pokemon, [side(vec![HOLDER]), side(vec![BOMB, BENCH])], 3);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine
        .submit_turn_choices([first_move(SideId::P1, HOLDER), first_move(SideId::P2, BOMB)])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let state = engine.state();
    assert_eq!(
        state.pokemon(BOMB).unwrap().hp,
        0,
        "Explosion knocked its user out"
    );
    let bide = state.pokemon(HOLDER).unwrap().volatiles.bide().copied();
    assert_eq!(
        bide.map(|b| b.damage),
        Some(0),
        "the faint wiped the store: {bide:?}"
    );
}
