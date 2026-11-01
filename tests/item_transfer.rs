//! L1 rule tests for R211 where the replay cannot tell: Gen 2 Thief's steal is a secondary
//! whose 100 % chance is a byte roll, so a Thief that took nothing reads as a roll that failed —
//! Mail's refusal (issues.md I53) only shows with the roll fixed to succeed.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, RandomOverride, RandomRequestKind, SideState,
};

const THIEF: PokemonId = PokemonId(1);
const HOLDER: PokemonId = PokemonId(2);

/// `thief`'s Thief on a `holder` holding `item`, `tape` planning the draws; the two Pokémon's
/// items afterwards.
fn thief_on(
    generation: Generation,
    thief: &str,
    holder: &str,
    item: &str,
    tape: Vec<RandomOverride>,
) -> (Option<ItemId>, Option<ItemId>) {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut holder_set = PokemonSet::new(holder, ["Harden"]);
    holder_set.item = Some(item.into());
    let pokemon = vec![
        build_pokemon(&dex, THIEF, SideId::P1, &PokemonSet::new(thief, ["Thief"])).unwrap(),
        build_pokemon(&dex, HOLDER, SideId::P2, &holder_set).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(pokemon, [side(THIEF), side(HOLDER)], 1);
    state.rng = BattleRng::with_semantic_tape(1, tape);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    engine.run_lead_switch_ins();
    let choice = |side, actor, target| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(0).unwrap(),
                target,
                mechanic: None,
            },
        }],
    };
    let foe = Some(DeclaredTarget(
        rules.topology().position(SideId::P2, 0).unwrap(),
    ));
    engine
        .submit_turn_choices([
            choice(SideId::P1, THIEF, foe),
            choice(SideId::P2, HOLDER, None),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    let held = |id| engine.state().pokemon(id).unwrap().held_item;
    (held(THIEF), held(HOLDER))
}

/// Gen 2 Sneasel's Thief on a Lapras holding `item`, every draw a success.
fn gen_2_thief_on(item: &str) -> (Option<ItemId>, Option<ItemId>) {
    thief_on(
        Generation::Gen2,
        "Sneasel",
        "Lapras",
        item,
        vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, 255),
            RandomOverride::bool(RandomRequestKind::Secondary, true),
        ],
    )
}

#[test]
fn gen_2_thief_takes_anything_but_mail() {
    assert_eq!(gen_2_thief_on("Leftovers"), (Some(ItemId::LEFTOVERS), None));
    assert_eq!(gen_2_thief_on("Mail"), (None, Some(ItemId::MAIL)));
}

/// Thief asks the item's `TakeItem` again once it is taken, its user as the holder
/// (`singleEvent('TakeItem', item, …, source, target)`): a Mega Stone stays with a holder whose
/// thief would Mega Evolve with it, unseen (R-M10-43).
#[test]
fn thief_leaves_a_mega_stone_its_thief_would_mega_evolve_with() {
    let steal = |thief: &str| {
        thief_on(
            Generation::Gen7,
            thief,
            "Forretress",
            "Lucarionite",
            Vec::new(),
        )
    };
    assert_eq!(steal("Lucario"), (None, Some(ItemId::LUCARIONITE)));
    assert_eq!(steal("Machamp"), (Some(ItemId::LUCARIONITE), None));
}

/// Trick's `onTryImmunity` (`TryImmunity::NoStickyHold`): a Sticky Hold target is immune. The
/// replay cannot tell it from the `TakeItem` refusal that would follow (both fail the move).
#[test]
fn trick_is_immune_to_a_sticky_hold_target() {
    let generation = Generation::Gen9;
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let immune = |ability: &str| {
        let mut target = PokemonSet::new("Muk", ["Harden"]);
        target.ability = Some(ability.into());
        target.item = Some("Black Sludge".into());
        let pokemon = vec![
            build_pokemon(
                &dex,
                THIEF,
                SideId::P1,
                &PokemonSet::new("Espeon", ["Trick"]),
            )
            .unwrap(),
            build_pokemon(&dex, HOLDER, SideId::P2, &target).unwrap(),
        ];
        let side = |id: PokemonId| {
            let mut side = SideState::new(vec![id], 1);
            side.active[0] = Some(id);
            side
        };
        let state = BattleState::new(pokemon, [side(THIEF), side(HOLDER)], 1);
        let trick = dex.move_by_name("Trick").unwrap();
        pokemon_battle_engine::engine::move_immune(&state, &rules, THIEF, HOLDER, trick)
    };
    assert!(immune("Sticky Hold"));
    assert!(!immune("Stench"));
}
