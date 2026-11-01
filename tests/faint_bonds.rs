//! L1 tests for Destiny Bond and Grudge (DESIGN.md §15.9): what the replay cannot set up. Their
//! `onFaint` acts only on a move's knock-out (`effect.effectType === 'Move'`); a Pokémon that
//! caused the faint otherwise (Bad Dreams in the residual) is spared.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const HOLDER: PokemonId = PokemonId(1);
const BENCH: PokemonId = PokemonId(2);
const DREAMER: PokemonId = PokemonId(3);

fn set(species: &str, moves: &[&str], level: u8, ability: &str) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
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

/// The bond (or grudge) holder, at 1 HP, is put to sleep after setting it up and falls to Bad
/// Dreams in the residual: its foe caused the faint, but not with a move.
fn falls_to_bad_dreams(generation: Generation, bond: &str) -> BattleEngine {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut holder = build_pokemon(
        &dex,
        HOLDER,
        SideId::P1,
        &set("Mew", &[bond], 100, "Synchronize"),
    )
    .unwrap();
    holder.hp = 1;
    let pokemon = vec![
        holder,
        build_pokemon(
            &dex,
            BENCH,
            SideId::P1,
            &set("Mew", &["Harden"], 100, "Synchronize"),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            DREAMER,
            SideId::P2,
            &set("Snorlax", &["Spore"], 50, "Bad Dreams"),
        )
        .unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active = vec![Some(team[0])];
        side
    };
    let state = BattleState::new(pokemon, [side(vec![HOLDER, BENCH]), side(vec![DREAMER])], 7);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine
        .submit_turn_choices([
            first_move(SideId::P1, HOLDER),
            first_move(SideId::P2, DREAMER),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    engine
}

#[test]
fn destiny_bond_spares_a_foe_that_knocked_its_holder_out_without_a_move() {
    let engine = falls_to_bad_dreams(Generation::Gen9, "Destiny Bond");
    let state = engine.state();
    assert_eq!(
        state.pokemon(HOLDER).unwrap().hp,
        0,
        "Bad Dreams took the holder down"
    );
    assert!(state.pokemon(DREAMER).unwrap().hp > 0, "no move: no bond");
}

#[test]
fn grudge_spares_the_pp_of_a_foe_that_knocked_its_holder_out_without_a_move() {
    let engine = falls_to_bad_dreams(Generation::Gen8, "Grudge");
    let state = engine.state();
    assert_eq!(
        state.pokemon(HOLDER).unwrap().hp,
        0,
        "Bad Dreams took the holder down"
    );
    let spore = &state.pokemon(DREAMER).unwrap().moves[0];
    assert_eq!(spore.pp, spore.max_pp - 1, "no move: no grudge");
}
