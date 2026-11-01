//! L1 rule test for R176 (DESIGN.md §20.1, issues.md I5) where the replay cannot tell: it plans
//! an accuracy roll where the engine's `accuracy_roll` has one and reads hit or miss from the
//! log, so the Gen 4 cartridge's hit rate with Simple (each stage doubled, only their sum
//! clamped) only shows in the rate itself.

use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::ids::{AbilityId, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState, Stat};

const USER: PokemonId = PokemonId(1);
const TARGET: PokemonId = PokemonId(2);

/// The accuracy check of the user's `move_name` on the target, with the user's accuracy stage,
/// the target's evasion stage and who of them has Simple.
fn accuracy(
    generation: Generation,
    move_name: &str,
    (user_simple, accuracy): (bool, i8),
    (target_simple, evasion): (bool, i8),
) -> Option<(u16, u16)> {
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let mut user =
        build_pokemon(&dex, USER, SideId::P1, &PokemonSet::new("Mew", [move_name])).unwrap();
    let mut target = build_pokemon(
        &dex,
        TARGET,
        SideId::P2,
        &PokemonSet::new("Bibarel", ["Harden"]),
    )
    .unwrap();
    user.ability = if user_simple {
        AbilityId::SIMPLE
    } else {
        AbilityId::SYNCHRONIZE
    };
    target.ability = if target_simple {
        AbilityId::SIMPLE
    } else {
        AbilityId::OWN_TEMPO
    };
    user.stages.set(Stat::Accuracy, accuracy);
    target.stages.set(Stat::Evasion, evasion);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![user, target], [side(USER), side(TARGET)], 1);
    let move_id = state.pokemon(USER).unwrap().moves[0].move_id;
    let move_def = dex.move_by_id(move_id).unwrap().clone();
    pokemon_battle_engine::engine::accuracy_roll(&state, &rules, USER, TARGET, &move_def)
}

#[test]
fn gen_4_simple_doubles_each_stage_and_the_cartridge_clamps_only_the_sum() {
    // The target's -4 evasion reads -8: 6 - 2 + 8 = 12 → ×3, sure to hit (clamping each stage
    // to ±6 first would give 50 × 3/5 × 3 = 90).
    assert_eq!(
        accuracy(Generation::Gen4, "Zap Cannon", (false, -2), (true, -4)),
        None
    );
    // The user's -4 accuracy reads -8 against a -4 evasion: 6 - 8 + 4 = 2 → 95 × 43/100 = 40
    // (clamped first, 6 - 6 + 4 = 4 → 95 × 60/100 = 57).
    assert_eq!(
        accuracy(Generation::Gen4, "Tackle", (true, -4), (false, -4)),
        Some((40, 100))
    );
    // Without Simple the stages count once: 6 - 2 + 4 = 8 → 166/100 of 50.
    assert_eq!(
        accuracy(Generation::Gen4, "Zap Cannon", (false, -2), (false, -4)),
        Some((83, 100))
    );
    // Gen 5+ Simple doubles the change, not the reading: a stored -1 is -1.
    assert_eq!(
        accuracy(Generation::Gen5, "Hypnosis", (true, -1), (false, 0)),
        Some((45, 100))
    );
}
