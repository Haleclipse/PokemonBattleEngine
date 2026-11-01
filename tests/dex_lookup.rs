//! The dex's lookups by id (DESIGN.md R-M10-12): its dense tables find what a search of its
//! tables finds — every move, ability and item of every generation's dex, the first row of an
//! id, and nothing for an id no row holds.

use pokemon_battle_engine::dex::DexResolver;
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveId};
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
fn a_lookup_by_id_finds_what_a_search_finds() {
    for generation in GENERATIONS {
        let dex = DexResolver::resolve(generation);
        assert!(!dex.moves().is_empty(), "{generation:?}: no moves");
        // Every id up to past the largest a row holds: present or not.
        let largest = dex
            .moves()
            .iter()
            .map(|m| m.id.0)
            .chain(dex.abilities().iter().map(|a| a.id.0))
            .chain(dex.items().iter().map(|i| i.id.0))
            .max()
            .unwrap_or(0);
        for id in 0..=largest + 1 {
            let id_move = MoveId(id);
            assert!(
                dex.move_by_id(id_move).map(|m| m as *const _)
                    == dex
                        .moves()
                        .iter()
                        .find(|m| m.id == id_move)
                        .map(|m| m as *const _),
                "{generation:?}: move {id}"
            );
            let id_ability = AbilityId(id);
            assert!(
                dex.ability_by_id(id_ability).map(|a| a as *const _)
                    == dex
                        .abilities()
                        .iter()
                        .find(|a| a.id == id_ability)
                        .map(|a| a as *const _),
                "{generation:?}: ability {id}"
            );
            let id_item = ItemId(id);
            assert!(
                dex.item_by_id(id_item).map(|i| i as *const _)
                    == dex
                        .items()
                        .iter()
                        .find(|i| i.id == id_item)
                        .map(|i| i as *const _),
                "{generation:?}: item {id}"
            );
        }
        assert!(dex.move_by_id(MoveId(u16::MAX)).is_none());
    }
}
