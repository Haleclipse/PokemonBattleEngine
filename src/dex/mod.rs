//! The dex (DESIGN.md §27): each generation's moves, abilities, items and species, resolved
//! from the committed `@pkmn/data` snapshot. `model` is what the engine reads, `resolver` builds
//! it (behavior per generation, support classification), `ResolvedDex` looks it up by id and
//! by name, `sets` turns a team file's Pokémon into battle state.

mod gimmicks;
mod model;
mod resolver;
mod sets;
mod types;

pub use gimmicks::{
    MaxMove, ZConversion, ZStatusEffect, max_move_for, max_move_power, z_conversion, z_move_power,
};
pub use model::{
    BaseStats, ChargeSkip, EffectChance, FixedDamage, FollowUp, FormeKind, MoveCategory, MoveFlags,
    MultiHit, Ohko, PrimaryEffect, Recoil, ResolvedAbility, ResolvedItem, ResolvedMove,
    ResolvedSpecies, Retaliation, SecondaryEffect, SecondaryEffectSpec, SecondaryRequirement,
    SecondaryTarget, SecondaryTrigger, SelfDestruct, SelfSwitch, StatSources, StatusMoveEffect,
    SupportSummary, TwoTurnSpec,
};
pub use resolver::DexResolver;
pub use sets::{
    Nature, PokemonSet, SetError, StatSpread, build_pokemon, calculate_stats, pp_with_ups,
};
pub use types::PokemonType;

use std::collections::BTreeMap;

use crate::data;
use crate::ids::{AbilityId, DenseRows, ItemId, MoveId, SpeciesId};
use crate::spec::Generation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDex {
    pub generation: Generation,
    fingerprint: String,
    moves: Vec<ResolvedMove>,
    abilities: Vec<ResolvedAbility>,
    items: Vec<ResolvedItem>,
    species: Vec<ResolvedSpecies>,
    move_names: BTreeMap<String, usize>,
    ability_names: BTreeMap<String, usize>,
    item_names: BTreeMap<String, usize>,
    species_names: BTreeMap<String, usize>,
    species_ids: BTreeMap<SpeciesId, usize>,
    /// Dense tables (DESIGN.md §47, R-M10-12): a move's, ability's or item's id → its row. The
    /// engine looks moves up by id at every turn of every battle; a search over the whole
    /// table was a fifth of a battle's time.
    move_rows: DenseRows,
    ability_rows: DenseRows,
    item_rows: DenseRows,
}

impl ResolvedDex {
    /// A dex from explicit tables. Used for empty / synthetic dexes; real battles use
    /// `DexResolver::resolve`, which builds from the committed snapshot.
    pub fn new(
        generation: Generation,
        moves: Vec<ResolvedMove>,
        abilities: Vec<ResolvedAbility>,
    ) -> Self {
        let canonical = format!("{generation:?}|{moves:?}|{abilities:?}");
        let fingerprint = format!("dex:synthetic:{:016x}", fnv1a64(canonical.as_bytes()));
        Self::assemble(
            generation,
            fingerprint,
            moves,
            abilities,
            Vec::new(),
            Vec::new(),
        )
    }

    fn assemble(
        generation: Generation,
        fingerprint: String,
        moves: Vec<ResolvedMove>,
        abilities: Vec<ResolvedAbility>,
        items: Vec<ResolvedItem>,
        species: Vec<ResolvedSpecies>,
    ) -> Self {
        let index = |names: Vec<String>| -> BTreeMap<String, usize> {
            names
                .into_iter()
                .enumerate()
                .map(|(index, name)| (data::to_id(&name), index))
                .collect()
        };
        let move_names = index(moves.iter().map(|m| m.name.clone()).collect());
        let ability_names = index(abilities.iter().map(|a| a.name.clone()).collect());
        let item_names = index(items.iter().map(|i| i.name.clone()).collect());
        let species_names = index(species.iter().map(|s| s.name.clone()).collect());
        let species_ids = species
            .iter()
            .enumerate()
            .map(|(index, s)| (s.id, index))
            .collect();
        let move_rows = DenseRows::new(moves.iter().map(|m| m.id.0));
        let ability_rows = DenseRows::new(abilities.iter().map(|a| a.id.0));
        let item_rows = DenseRows::new(items.iter().map(|i| i.id.0));
        Self {
            generation,
            fingerprint,
            moves,
            abilities,
            items,
            species,
            move_names,
            ability_names,
            item_names,
            species_names,
            species_ids,
            move_rows,
            ability_rows,
            item_rows,
        }
    }

    /// Identifies the exact data a battle was resolved against. Snapshot-backed dexes hash the
    /// committed snapshot text, so a regenerated snapshot invalidates old battle snapshots.
    pub fn fingerprint(&self) -> String {
        self.fingerprint.clone()
    }

    /// Every move known to this generation, including unsupported ones, in snapshot order.
    pub fn moves(&self) -> &[ResolvedMove] {
        &self.moves
    }

    pub fn abilities(&self) -> &[ResolvedAbility] {
        &self.abilities
    }

    pub fn items(&self) -> &[ResolvedItem] {
        &self.items
    }

    pub fn species(&self) -> &[ResolvedSpecies] {
        &self.species
    }

    /// The generic Max Move of a type (极巨飞冲 …): the Max Move whose own type is that type
    /// (DESIGN.md §44.5). Showdown's `MAX_MOVES[type]`.
    pub fn max_move_of_type(&self, move_type: crate::dex::PokemonType) -> Option<MoveId> {
        self.moves
            .iter()
            .find(|move_def| {
                move_def.is_max
                    && move_def.category != crate::dex::MoveCategory::Status
                    && move_def.move_type == move_type
            })
            .map(|move_def| move_def.id)
    }

    /// Max Guard (`MAX_MOVES.Status`): the move every status move becomes under Dynamax.
    pub fn max_guard(&self) -> Option<MoveId> {
        self.moves
            .iter()
            .find(|move_def| {
                move_def.is_max && move_def.category == crate::dex::MoveCategory::Status
            })
            .map(|move_def| move_def.id)
    }

    /// The generic Z-Move of a type (究极无敌大冲撞 …): the Z-Move whose crystal is that
    /// type's Z-Crystal (DESIGN.md §44.4). Showdown's `this.Z_MOVES[type]`.
    pub fn z_move_of_type(&self, move_type: crate::dex::PokemonType) -> Option<MoveId> {
        self.moves.iter().find_map(|move_def| {
            let crystal = self.item_by_id(move_def.z_crystal?)?;
            (crystal.z_move_from.is_none() && crystal.z_move_type == Some(move_type))
                .then_some(move_def.id)
        })
    }

    pub fn move_by_id(&self, id: MoveId) -> Option<&ResolvedMove> {
        self.move_rows.get(id.0).map(|row| &self.moves[row])
    }

    pub fn ability_by_id(&self, id: AbilityId) -> Option<&ResolvedAbility> {
        self.ability_rows.get(id.0).map(|row| &self.abilities[row])
    }

    pub fn item_by_id(&self, id: ItemId) -> Option<&ResolvedItem> {
        self.item_rows.get(id.0).map(|row| &self.items[row])
    }

    /// Name lookup through Showdown's `toID` normalisation ("Power-Up Punch" == "powerUpPunch").
    pub fn move_by_name(&self, name: &str) -> Option<&ResolvedMove> {
        self.move_names
            .get(&data::to_id(name))
            .map(|index| &self.moves[*index])
    }

    pub fn ability_by_name(&self, name: &str) -> Option<&ResolvedAbility> {
        self.ability_names
            .get(&data::to_id(name))
            .map(|index| &self.abilities[*index])
    }

    pub fn item_by_name(&self, name: &str) -> Option<&ResolvedItem> {
        self.item_names
            .get(&data::to_id(name))
            .map(|index| &self.items[*index])
    }

    pub fn species_by_id(&self, id: SpeciesId) -> Option<&ResolvedSpecies> {
        self.species_ids.get(&id).map(|index| &self.species[*index])
    }

    pub fn species_by_name(&self, name: &str) -> Option<&ResolvedSpecies> {
        self.species_names
            .get(&data::to_id(name))
            .map(|index| &self.species[*index])
    }

    /// The species `species` shows everyone (Showdown `getUpdatedDetails`): its own, but for the
    /// formes that show their base species (`HIDDEN_FORMES`).
    pub fn shown_species<'a>(&'a self, species: &'a ResolvedSpecies) -> &'a ResolvedSpecies {
        if HIDDEN_FORMES.contains(&species.name.as_str())
            && let Some(base) = species
                .base_species
                .as_deref()
                .and_then(|base| self.species_by_name(base))
        {
            return base;
        }
        species
    }
}

/// The formes Showdown's details show as their base species: what sets them apart (Battle
/// Bond, Own Tempo) stays hidden until it shows itself.
pub const HIDDEN_FORMES: [&str; 2] = ["Greninja-Bond", "Rockruff-Dusk"];

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
