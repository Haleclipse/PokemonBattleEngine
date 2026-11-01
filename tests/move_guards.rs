//! L1 tests for the K.9a abilities (DESIGN.md R275): what the replay cannot see. Anticipation
//! and Forewarn only announce, Forewarn's draw names a move no frame shows, and Soundproof's
//! `onAllyTryHitSide` announces a partner's sound move without stopping it.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, PokemonType, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{AbilityId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, PokemonState, SideState};

const P1A: PokemonId = PokemonId(1);
const P1B: PokemonId = PokemonId(2);
const P2A: PokemonId = PokemonId(3);
const P2B: PokemonId = PokemonId(4);

fn set(species: &str, moves: &[&str], ability: &str) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.level = 50;
    set
}

/// A doubles battle of the four sets, P1 = [A, B], P2 = [A, B], all leading.
fn doubles(
    generation: Generation,
    sets: [PokemonSet; 4],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> (Arc<ResolvedRules>, BattleEngine) {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let ids = [
        (P1A, SideId::P1),
        (P1B, SideId::P1),
        (P2A, SideId::P2),
        (P2B, SideId::P2),
    ];
    let mut pokemon: Vec<PokemonState> = ids
        .iter()
        .zip(&sets)
        .map(|((id, side), set)| build_pokemon(&dex, *id, *side, set).unwrap())
        .collect();
    prepare(&mut pokemon);
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let state = BattleState::new(pokemon, [side([P1A, P1B]), side([P2A, P2B])], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    engine.run_lead_switch_ins();
    (rules, engine)
}

/// Whether `holder` announced `ability` so far.
fn announced(engine: &BattleEngine, holder: PokemonId, ability: AbilityId) -> bool {
    engine.trace().events().iter().any(|event| {
        matches!(event, TraceEvent::AbilityActivated { source, ability: a }
            if *source == holder && *a == ability)
    })
}

/// Every Pokémon uses its first move, P1's at P2's first slot and P2's at P1's.
fn turn(rules: &ResolvedRules, engine: &mut BattleEngine) {
    let choice = |side: SideId, actors: [PokemonId; 2], target: SideId| SideChoice {
        side,
        actions: actors
            .iter()
            .map(|actor| SlotChoice {
                actor: *actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(0).unwrap(),
                    target: Some(DeclaredTarget(
                        rules.topology().position(target, 0).unwrap(),
                    )),
                    mechanic: None,
                },
            })
            .collect(),
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, [P1A, P1B], SideId::P2),
            choice(SideId::P2, [P2A, P2B], SideId::P1),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
}

#[test]
fn anticipation_fears_a_super_effective_move_or_a_one_hit_ko_and_hidden_power_by_generation() {
    let leads = |foe_moves: &[&str]| {
        [
            set("Snorlax", &["Harden"], "Anticipation"),
            set("Blissey", &["Harden"], "Natural Cure"),
            set("Hariyama", foe_moves, "Thick Fat"),
            set("Blissey", &["Harden"], "Natural Cure"),
        ]
    };
    let (_, engine) = doubles(Generation::Gen9, leads(&["Close Combat"]), |_| {});
    assert!(announced(&engine, P1A, AbilityId::ANTICIPATION));
    let (_, engine) = doubles(Generation::Gen9, leads(&["Tackle", "Bulk Up"]), |_| {});
    assert!(!announced(&engine, P1A, AbilityId::ANTICIPATION));
    let (_, engine) = doubles(Generation::Gen9, leads(&["Fissure"]), |_| {});
    assert!(announced(&engine, P1A, AbilityId::ANTICIPATION));
    // Hidden Power by its user's type from Gen 6, by the move's own (Normal) before.
    let fighting_power = |pokemon: &mut Vec<PokemonState>| {
        pokemon[2].hidden_power.0 = PokemonType::Fighting;
    };
    let (_, engine) = doubles(Generation::Gen7, leads(&["Hidden Power"]), fighting_power);
    assert!(announced(&engine, P1A, AbilityId::ANTICIPATION));
    let (_, engine) = doubles(Generation::Gen5, leads(&["Hidden Power"]), fighting_power);
    assert!(!announced(&engine, P1A, AbilityId::ANTICIPATION));
}

#[test]
fn forewarn_warns_of_the_strongest_moves_with_the_generations_one_hit_ko_power() {
    let candidates = |generation: Generation| {
        let (rules, engine) = doubles(
            generation,
            [
                set("Hypno", &["Harden"], "Forewarn"),
                set("Blissey", &["Harden"], "Natural Cure"),
                set(
                    "Hariyama",
                    &["Fissure", "Close Combat", "Counter"],
                    "Thick Fat",
                ),
                set("Camerupt", &["Eruption", "Tackle"], "Solid Rock"),
            ],
            |_| {},
        );
        assert!(announced(&engine, P1A, AbilityId::FOREWARN));
        pokemon_battle_engine::engine::forewarn_candidates(engine.state(), &rules, P1A)
    };
    // Fissure 150 ties Eruption from Gen 5; the gen4 mod's 160 stands alone.
    assert_eq!(
        candidates(Generation::Gen9),
        vec![(MoveId::FISSURE, P2A), (MoveId::ERUPTION, P2B)]
    );
    assert_eq!(candidates(Generation::Gen4), vec![(MoveId::FISSURE, P2A)]);
}

#[test]
fn soundproof_announces_a_partners_sound_move_at_its_side_from_gen_6() {
    let leads = || {
        [
            set("Blissey", &["Heal Bell"], "Natural Cure"),
            set("Snorlax", &["Harden"], "Soundproof"),
            set("Hariyama", &["Bulk Up"], "Thick Fat"),
            set("Blissey", &["Harden"], "Natural Cure"),
        ]
    };
    for (generation, announces) in [(Generation::Gen9, true), (Generation::Gen5, false)] {
        let (rules, mut engine) = doubles(generation, leads(), |_| {});
        turn(&rules, &mut engine);
        assert_eq!(
            announced(&engine, P1B, AbilityId::SOUNDPROOF),
            announces,
            "{generation:?}"
        );
    }
}
