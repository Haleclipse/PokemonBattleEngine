//! M7's inventory stays empty (ROADMAP §11, `tools/inventory.py`): every move, ability and item
//! of every generation's snapshot is implemented. An ability with no callbacks of its own counts
//! only once reviewed (`CALLBACKLESS_ABILITIES`): Tera Shell, whose effect Showdown hard-codes in
//! `runEffectiveness`, had passed as implemented while the engine never modelled it (R-M9-45).

use pokemon_battle_engine::dex::{DexResolver, PrimaryEffect};
use pokemon_battle_engine::spec::Generation;

const GENERATIONS: [Generation; 9] = [
    Generation::Gen1,
    Generation::Gen2,
    Generation::Gen3,
    Generation::Gen4,
    Generation::Gen5,
    Generation::Gen6,
    Generation::Gen7,
    Generation::Gen8,
    Generation::Gen9,
];

#[test]
fn every_move_ability_and_item_is_implemented() {
    for generation in GENERATIONS {
        let dex = DexResolver::resolve(generation);
        let mut missing: Vec<String> = dex
            .moves()
            .iter()
            .filter(|m| m.primary_effect == PrimaryEffect::Unsupported)
            .map(|m| format!("move {}: {:?}", m.name, m.unsupported))
            .collect();
        missing.extend(
            dex.abilities()
                .iter()
                .filter(|a| !a.implemented)
                .map(|a| format!("ability {}: {:?}", a.name, a.missing_callbacks)),
        );
        missing.extend(
            dex.items()
                .iter()
                .filter(|i| !i.implemented)
                .map(|i| format!("item {}: {:?}", i.name, i.missing_callbacks)),
        );
        assert!(missing.is_empty(), "{generation:?}: {missing:#?}");
    }
}
