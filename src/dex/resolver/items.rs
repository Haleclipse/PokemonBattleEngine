//! Items: behavior coverage and the items whose effect lives in another effect.

use crate::data::{self, DexSnapshot};
use crate::effects;
use crate::ids::ItemId;

use super::support::missing_callbacks;
use crate::dex::ResolvedItem;

/// Every item of the generation, classified against its behavior components (DESIGN.md
/// §27.2).
pub(super) fn resolve_items(snapshot: &DexSnapshot) -> Vec<ResolvedItem> {
    // A Z-Crystal names its move and its species by name: the generation's own ids resolve them
    // (DESIGN.md §44.4).
    let moves: std::collections::BTreeMap<String, crate::ids::MoveId> = snapshot
        .moves
        .iter()
        .map(|move_| (move_.id.clone(), crate::ids::MoveId(move_.eid)))
        .collect();
    let species: std::collections::BTreeMap<String, crate::ids::SpeciesId> = snapshot
        .species
        .iter()
        .map(|species| {
            (
                species.id.clone(),
                crate::ids::SpeciesId {
                    dex: species.num as u16,
                    form: species.form,
                },
            )
        })
        .collect();
    snapshot
        .items
        .iter()
        .map(|item| {
            let id = ItemId(item.eid);
            let mut covered = item_covered_callbacks(id);
            // A Z-Crystal's `onTakeItem: false` is data (`z_move`), not a behavior entry
            // (DESIGN.md §44.4): the registry reads the same flag.
            if item.other.contains_key("z_move") {
                covered.push("onTakeItem");
            }
            // `plate` is data (DESIGN.md §44.4): a type Z-Crystal gives its holder's Multitype
            // the type without a behavior component of its own (a Plate's own `plate(...)`
            // covers `onPlate` already, §43.4). The Memories' and Drives' `memory` / `drive`
            // are covered by their constructors' components alone.
            if item.other.contains_key("plate") {
                covered.push("onPlate");
            }
            // A Mega Stone's `onTakeItem` is data too (DESIGN.md §44.3): the species it belongs
            // to is what refuses the take.
            if item.other.contains_key("mega_stone") {
                covered.push("onTakeItem");
            }
            let mut missing_callbacks = missing_callbacks(&item.callbacks, &covered);
            // An item whose effect lives in another effect's `hasItem` check is only as
            // implemented as that effect.
            if let Some((_, reference, _)) =
                UNMODELLED_ITEM_REFERENCES
                    .iter()
                    .find(|(name, _, generations)| {
                        *name == data::to_id(&item.name)
                            && generations.contains(&snapshot.generation)
                    })
            {
                missing_callbacks.push(format!("referenced-by:{reference}"));
            }
            // `zMove` is `true` for a type crystal and the signature move's name otherwise.
            let z_crystal = match item.other.get("z_move") {
                Some(serde_json::Value::Bool(true)) => Some(None),
                Some(serde_json::Value::String(name)) => {
                    moves.get(&crate::data::to_id(name)).copied().map(Some)
                }
                _ => None,
            };
            let z_move_from = item
                .other
                .get("z_move_from")
                .and_then(serde_json::Value::as_str)
                .and_then(|name| moves.get(&crate::data::to_id(name)).copied());
            let z_move_type = item
                .other
                .get("z_move_type")
                .and_then(serde_json::Value::as_str)
                .and_then(crate::dex::PokemonType::from_name);
            let plate = item
                .other
                .get("plate")
                .and_then(serde_json::Value::as_str)
                .and_then(crate::dex::PokemonType::from_name);
            let memory = item
                .other
                .get("memory")
                .and_then(serde_json::Value::as_str)
                .and_then(crate::dex::PokemonType::from_name);
            let drive = item
                .other
                .get("drive")
                .and_then(serde_json::Value::as_str)
                .and_then(crate::dex::PokemonType::from_name);
            // `megaStone: {BaseSpeciesName: 'Forme-Name'}` (DESIGN.md §44.3): one entry in
            // every stone the snapshots carry.
            let mega_stone = item
                .other
                .get("mega_stone")
                .and_then(serde_json::Value::as_object)
                .and_then(|stones| stones.iter().next())
                .and_then(|(holder, forme)| {
                    Some((
                        *species.get(&crate::data::to_id(holder))?,
                        *species.get(&crate::data::to_id(forme.as_str()?))?,
                    ))
                });
            let item_user = item
                .other
                .get("item_user")
                .and_then(serde_json::Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .filter_map(|name| species.get(&crate::data::to_id(name)).copied())
                        .collect()
                })
                .unwrap_or_default();
            ResolvedItem {
                id,
                name: item.name.clone(),
                z_crystal,
                z_move_from,
                z_move_type,
                plate,
                memory,
                drive,
                item_user,
                mega_stone,
                is_berry: item
                    .other
                    .get("is_berry")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                implemented: missing_callbacks.is_empty(),
                missing_callbacks,
            }
        })
        .collect()
}

/// Items with no callbacks of their own whose effect Showdown implements inside another effect
/// (none at the moment: Protective Pads was the last, modelled by `engine::move_makes_contact`,
/// DESIGN.md §43.4).
/// (`hasItem('…')` in a condition, move or the simulator; found by grepping `@pkmn/sim` for
/// `hasItem` / `item ===`). Without this list they would count as implemented just for having
/// nothing to cover. Heavy-Duty Boots (entry hazards), the weather rocks (weather duration) and
/// Light Clay (the screens' duration), Terrain Extender (the terrains') and Utility Umbrella
/// (`engine::weather_for`) are modelled where they are referenced and are therefore
/// not listed, as are Gen 2's Thick Club, Light Ball and Metal Powder (inside `getStat`:
/// `StatRules::species_item_stats`) and Gen 2-3 Quick Claw (inside `getActionSpeed`:
/// `ActionOrderRules::quick_claw_turn_roll`). Rusted Sword and Rusted Shield carry no callback at
/// all: the crowned forme they give lives in the `zacian` / `zamazenta` species condition's
/// `onBattleStart`, which the engine runs at the start of the battle
/// (`effects::items::battle_start_forme`, DESIGN.md §43.4).
const UNMODELLED_ITEM_REFERENCES: &[(&str, &str, std::ops::RangeInclusive<u8>)] = &[];

fn item_covered_callbacks(id: ItemId) -> Vec<&'static str> {
    effects::items::definition(id).covered_callbacks()
}
