//! Building a generation's dex from its `@pkmn/data` snapshot (DESIGN.md §27): every move,
//! ability, item and species, with the generation's behavior and support classification.

mod abilities;
mod items;
mod moves;
mod species;
mod support;

use super::{ResolvedDex, fnv1a64};
use crate::data::{self};
use crate::spec::Generation;

pub struct DexResolver;

impl DexResolver {
    pub fn empty(generation: Generation) -> ResolvedDex {
        ResolvedDex::new(generation, Vec::new(), Vec::new())
    }

    /// The generation's full dex, built from the committed `@pkmn/data` snapshot. Every move,
    /// ability and item is classified as implemented / unsupported against the behavior
    /// tables, so the dex never pretends to support behaviour the engine lacks.
    pub fn resolve(generation: Generation) -> ResolvedDex {
        let snapshot = data::snapshot(generation);
        let fingerprint = format!(
            "dex:{}:{}:gen{}:{:016x}",
            snapshot.provenance.engine,
            snapshot.provenance.version,
            snapshot.generation,
            fnv1a64(data::raw(generation).as_bytes())
        );

        let abilities = abilities::resolve_abilities(snapshot);
        let items = items::resolve_items(snapshot);
        let species = species::resolve_species(snapshot, &abilities);

        // The gimmicks the generation has decide which of their move data the dex resolves
        // (DESIGN.md §44.1).
        let capabilities = crate::rules::GenerationRules::for_generation(generation).capabilities;
        // A Z-Move names its crystal by the item's Showdown id, so the moves are resolved with
        // the generation's item ids at hand (DESIGN.md §44.4).
        let item_ids: std::collections::BTreeMap<String, crate::ids::ItemId> = snapshot
            .items
            .iter()
            .map(|item| (item.id.clone(), crate::ids::ItemId(item.eid)))
            .collect();
        let moves = snapshot
            .moves
            .iter()
            .filter_map(|snapshot| moves::resolve_move(snapshot, &capabilities, &item_ids))
            .collect::<Vec<_>>();

        ResolvedDex::assemble(generation, fingerprint, moves, abilities, items, species)
    }

    /// Alias kept for existing tests and examples. Not a hand-written sample any more: it is
    /// the full snapshot-backed dex.
    pub fn demo(generation: Generation) -> ResolvedDex {
        Self::resolve(generation)
    }
}
