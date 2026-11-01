//! The behavior registry (DESIGN.md §25.1): every ability's and item's behavior as one
//! generation has it — each definition restricted to the components whose Showdown callbacks
//! that generation's snapshot declares — resolved once per generation, before a battle starts.
//! Showdown callback names are read here, once; nothing on a battle's path sees them.
//! (Moves are resolved the same way by the dex, into `ResolvedMove.behavior`.)

use std::sync::OnceLock;

use crate::ids::{AbilityId, DenseRows, ItemId};
use crate::spec::Generation;

use super::abilities::{self, AbilityBehavior};
use super::items::{self, ItemBehavior};

pub struct BehaviorRegistry {
    /// Abilities with some active component, sorted by id.
    abilities: Vec<(AbilityId, AbilityBehavior)>,
    /// Items with some active component, sorted by id.
    items: Vec<(ItemId, ItemBehavior)>,
    /// Dense tables (DESIGN.md §47, R-M10-7): an ability's or item's id → its row in `abilities`
    /// / `items`. Every battle asks them all the time — whether an ability or item is in effect,
    /// its handlers — so the lookup is one index, not a search.
    ability_rows: DenseRows,
    item_rows: DenseRows,
    no_ability: AbilityBehavior,
    no_item: ItemBehavior,
}

impl BehaviorRegistry {
    /// The registry of `generation`, resolved on first use (`BattleEngine` resolves it when a
    /// battle is created) and shared by every battle of that generation.
    pub fn for_generation(generation: Generation) -> &'static BehaviorRegistry {
        static REGISTRIES: [OnceLock<BehaviorRegistry>; 9] = [const { OnceLock::new() }; 9];
        REGISTRIES[usize::from(generation.number() - 1)].get_or_init(|| Self::resolve(generation))
    }

    fn resolve(generation: Generation) -> Self {
        let snapshot = crate::data::snapshot(generation);
        let mut abilities: Vec<(AbilityId, AbilityBehavior)> = snapshot
            .abilities
            .iter()
            .filter_map(|row| {
                let id = AbilityId(row.eid);
                let behavior = abilities::definition(id).restricted_to(&row.callbacks);
                // Dancer has no callback of its own (Showdown hard-codes it in `runMove`): its
                // component carries the flag alone, so it is kept by that flag (DESIGN.md §42.9).
                (!behavior.covered_callbacks().is_empty() || behavior.dancer)
                    .then_some((id, behavior))
            })
            .collect();
        abilities.sort_by_key(|(id, _)| *id);
        let mut items: Vec<(ItemId, ItemBehavior)> = snapshot
            .items
            .iter()
            .filter_map(|row| {
                let id = ItemId(row.eid);
                let mut behavior = items::definition(id).restricted_to(&row.callbacks);
                // Data rather than callbacks: Showdown `ignoreKlutz` (Macho Brace, the Power
                // items, Ability Shield), `isChoice`, `isBerry`, `isGem`.
                let flag = |key: &str| row.other.get(key) == Some(&serde_json::Value::Bool(true));
                // A Z-Crystal refuses to be taken (`onTakeItem: false` in its data, DESIGN.md
                // §44.4); its real effect is the Z-Move itself.
                if row.other.contains_key("z_move") {
                    behavior.take_refusal = Some(items::TakeRefusal::ZCrystal);
                }
                // A Mega Stone cannot be taken off the species it belongs to (Showdown's
                // `onTakeItem`, DESIGN.md §44.3): the snapshot's `megaStone` map names it.
                if let Some(holder) = row
                    .other
                    .get("mega_stone")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|stones| stones.keys().next())
                    && let Some(dex) = snapshot
                        .species
                        .iter()
                        .find(|species| species.id == crate::data::to_id(holder))
                        .and_then(|species| u16::try_from(species.num).ok())
                {
                    behavior.take_refusal = Some(items::TakeRefusal::MegaStone(dex));
                }
                // Showdown `onPlate` (the snapshot's `plate`): the type the item gives its
                // holder's Multitype — a Plate or a type Z-Crystal (DESIGN.md §44.4).
                behavior.plate = row
                    .other
                    .get("plate")
                    .and_then(serde_json::Value::as_str)
                    .and_then(crate::dex::PokemonType::from_name);
                // Showdown `onMemory` / `onDrive` (the snapshot's `memory` / `drive`): the type
                // a Memory / Drive gives RKS System and Multi-Attack / Techno Blast
                // (DESIGN.md §43.4).
                behavior.memory = row
                    .other
                    .get("memory")
                    .and_then(serde_json::Value::as_str)
                    .and_then(crate::dex::PokemonType::from_name);
                behavior.drive = row
                    .other
                    .get("drive")
                    .and_then(serde_json::Value::as_str)
                    .and_then(crate::dex::PokemonType::from_name);
                behavior.ignores_klutz = flag("ignore_klutz");
                behavior.is_choice = flag("is_choice");
                behavior.is_berry = flag("is_berry");
                behavior.is_gem = flag("is_gem");
                // Fling's power and Natural Gift's type and power (Showdown `fling`,
                // `naturalGift`): an item kept for these alone (most items have no callback)
                // still answers what Fling and Natural Gift do with it.
                // A Flower Sweet flies at 10 like every Sweet: the Sword / Shield item table
                // (Smogon's battle mechanics research, Marty's datamine) where Showdown's data
                // has 0 (issues.md I196). A 0 is an item Fling cannot throw.
                behavior.fling_power = row
                    .other
                    .get("fling_bp")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|power| u16::try_from(power).ok())
                    .filter(|power| *power > 0)
                    .or((id == ItemId::FLOWER_SWEET).then_some(10));
                behavior.natural_gift = row.other.get("natural_gift").and_then(|gift| {
                    Some(items::NaturalGift {
                        move_type: crate::dex::PokemonType::from_name(gift["type"].as_str()?)?,
                        power: u16::try_from(gift["basePower"].as_u64()?).ok()?,
                    })
                });
                (!behavior.covered_callbacks().is_empty()
                    || behavior.fling_power.is_some()
                    || behavior.natural_gift.is_some())
                .then_some((id, behavior))
            })
            .collect();
        items.sort_by_key(|(id, _)| *id);
        let ability_rows = DenseRows::new(abilities.iter().map(|(id, _)| id.0));
        let item_rows = DenseRows::new(items.iter().map(|(id, _)| id.0));
        Self {
            abilities,
            items,
            ability_rows,
            item_rows,
            no_ability: AbilityBehavior::default(),
            no_item: ItemBehavior::default(),
        }
    }

    /// The behavior of no ability (a suppressed one's handlers: none).
    pub fn no_ability(&self) -> &AbilityBehavior {
        &self.no_ability
    }

    /// The ability's behavior in this generation (none for an ability the generation does not
    /// have, or whose components it declares no callback for).
    pub fn ability(&self, id: AbilityId) -> &AbilityBehavior {
        match self.ability_rows.get(id.0) {
            Some(row) => &self.abilities[row].1,
            None => &self.no_ability,
        }
    }

    /// The item's behavior in this generation (Gen 2's Dragon Fang: none).
    pub fn item(&self, id: ItemId) -> &ItemBehavior {
        match self.item_rows.get(id.0) {
            Some(row) => &self.items[row].1,
            None => &self.no_item,
        }
    }
}
