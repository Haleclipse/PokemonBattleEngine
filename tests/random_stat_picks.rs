//! L1 rule tests for R166 (DESIGN.md §20.2) where the replay cannot tell: it plans Moody's
//! `sample` by the stat Showdown named, with the engine's own candidate list, so which stats the
//! generation draws from only shows when the list itself changes — here, with the five stats
//! already at +6.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{AbilityId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, RandomOverride, RandomRequestKind, SideState, Stat,
};

const MOODY: PokemonId = PokemonId(1);
const FOE: PokemonId = PokemonId(2);

/// A Moody holder with Attack … Speed at +6, both sides using Harden; the stages after the
/// turn's residual, which draws `Sample` 0 for every pick it makes.
fn after_one_residual(generation: Generation) -> [i8; 7] {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut holder = build_pokemon(
        &dex,
        MOODY,
        SideId::P1,
        &PokemonSet::new("Snorlax", ["Harden"]),
    )
    .unwrap();
    holder.ability = AbilityId::MOODY;
    for stat in [
        Stat::Attack,
        Stat::Defense,
        Stat::SpecialAttack,
        Stat::SpecialDefense,
        Stat::Speed,
    ] {
        holder.stages.set(stat, 6);
    }
    let foe = build_pokemon(
        &dex,
        FOE,
        SideId::P2,
        &PokemonSet::new("Blissey", ["Harden"]),
    )
    .unwrap();
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(vec![holder, foe], [side(MOODY), side(FOE)], 1);
    state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::range(RandomRequestKind::Sample, 0),
            RandomOverride::range(RandomRequestKind::Sample, 0),
        ],
    );
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    let harden = |side: SideId, actor: PokemonId| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: None::<DeclaredTarget>,
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([harden(SideId::P1, MOODY), harden(SideId::P2, FOE)])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let stages = engine.state().pokemon(MOODY).unwrap().stages;
    pokemon_battle_engine::engine::STAGED_STATS.map(|stat| stages.get(stat))
}

#[test]
fn moody_draws_accuracy_and_evasion_only_in_gen_5_to_7() {
    // Gen 9: nothing below +6 to raise; the lowered stat is the first of the five (Attack),
    // after Harden took Defense nowhere.
    assert_eq!(after_one_residual(Generation::Gen9), [5, 6, 6, 6, 6, 0, 0]);
    // Gen 5: accuracy rises by 2 (the first stat below +6), then Attack falls.
    assert_eq!(after_one_residual(Generation::Gen5), [5, 6, 6, 6, 6, 2, 0]);
}

/// The two candidate lists' boundaries (Showdown `pokemon.boosts[stat] < 6` / `> -6`, the rise
/// excluded from the drop): a stat at +6 may not rise, one at -6 may not fall, and the stat that
/// rose is not a drop candidate.
#[test]
fn moody_candidate_lists_use_the_live_stages_at_their_bounds() {
    use pokemon_battle_engine::effects::abilities::{lowerable, raisable};
    use pokemon_battle_engine::state::StatStages;
    let mut stages = StatStages::default();
    stages.set(Stat::Attack, 6);
    stages.set(Stat::Defense, -6);
    stages.set(Stat::SpecialAttack, -5);
    // Gen 9: the five stats, no accuracy or evasion.
    let may_rise = raisable(|stat| stages.get(stat), false);
    assert!(!may_rise.contains(&Stat::Attack), "a +6 stat may rise");
    assert!(may_rise.contains(&Stat::Defense));
    assert!(may_rise.contains(&Stat::SpecialAttack));
    let may_fall = lowerable(|stat| stages.get(stat), false, Some(Stat::SpecialAttack));
    assert!(!may_fall.contains(&Stat::Defense), "a -6 stat may fall");
    assert!(
        !may_fall.contains(&Stat::SpecialAttack),
        "the stat that rose may fall too"
    );
    assert!(may_fall.contains(&Stat::Attack));
    assert!(may_fall.contains(&Stat::Speed));
    // Gen 5-7 draw from the seven, accuracy and evasion included.
    let seven = raisable(|stat| stages.get(stat), true);
    assert!(seven.contains(&Stat::Accuracy));
    assert!(seven.contains(&Stat::Evasion));
}
