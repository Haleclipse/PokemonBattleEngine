//! L1 rule tests for R122 (DESIGN.md §20.2) where the replay cannot tell: Merciless makes the
//! critical-hit check certain (the replay asks the engine whether a Critical decision is drawn),
//! and Stench's flinch chance (the replay plans a secondary's outcome, not its odds).

use pokemon_battle_engine::dex::{
    DexResolver, EffectChance, PokemonSet, SecondaryEffect, SecondaryTarget, build_pokemon,
};
use pokemon_battle_engine::engine::{CriticalRoll, critical_roll, modified_move};
use pokemon_battle_engine::ids::{PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleState, NonVolatileStatus, SideState, StatusState, VolatileConditionKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

fn set(species: &str, moves: &[&str], ability: &str) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set
}

/// A singles state of `attacker` against a Snorlax, the Snorlax given `status`.
fn state(
    generation: Generation,
    attacker: PokemonSet,
    status: Option<NonVolatileStatus>,
) -> (
    pokemon_battle_engine::rules::ResolvedRules,
    pokemon_battle_engine::dex::ResolvedDex,
    BattleState,
) {
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let a = build_pokemon(&dex, A, SideId::P1, &attacker).unwrap();
    let mut b = build_pokemon(
        &dex,
        B,
        SideId::P2,
        &set("Snorlax", &["Harden"], "Run Away"),
    )
    .unwrap();
    b.status = status.map(StatusState::new);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![a, b], [side(A), side(B)], 1);
    (rules, dex, state)
}

#[test]
fn merciless_hits_a_poisoned_target_critically() {
    for generation in [Generation::Gen7, Generation::Gen9] {
        for (status, certain) in [
            (Some(NonVolatileStatus::Poison), true),
            (Some(NonVolatileStatus::Toxic), true),
            (Some(NonVolatileStatus::Burn), false),
            (None, false),
        ] {
            let (rules, dex, state) =
                state(generation, set("Toxapex", &["Scald"], "Merciless"), status);
            let scald = dex.move_by_name("Scald").unwrap();
            let roll = critical_roll(&state, &rules, &dex, A, B, scald);
            assert_eq!(
                roll == CriticalRoll::Always,
                certain,
                "{generation:?} {status:?}: {roll:?}"
            );
        }
    }
}

#[test]
fn stench_adds_a_ten_percent_flinch_to_damaging_moves_without_one() {
    let (rules, dex, state) = state(
        Generation::Gen9,
        set("Skuntank", &["Tackle", "Bite", "Harden"], "Stench"),
        None,
    );
    let flinches = |name: &str| {
        let (modified, _) =
            modified_move(&state, &rules, A, dex.move_by_name(name).unwrap(), false);
        modified
            .secondaries
            .iter()
            .filter(|s| {
                s.target == SecondaryTarget::Target
                    && s.effect == SecondaryEffect::Volatile(VolatileConditionKind::Flinch)
            })
            .map(|s| s.chance)
            .collect::<Vec<_>>()
    };
    assert_eq!(flinches("Tackle"), vec![EffectChance::percent(10)]);
    assert_eq!(flinches("Bite"), vec![EffectChance::percent(30)]);
    assert_eq!(flinches("Harden"), Vec::<EffectChance>::new());
}
