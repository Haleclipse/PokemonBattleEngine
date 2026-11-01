//! Support classification shared by the resolvers, and the support counts the docs quote.

use crate::dex::{PrimaryEffect, ResolvedDex, SupportSummary};

pub(super) fn missing_callbacks(declared: &[String], covered: &[&str]) -> Vec<String> {
    declared
        .iter()
        .filter(|callback| !covered.contains(&callback.as_str()))
        .cloned()
        .collect()
}

impl ResolvedDex {
    /// How much of this generation the engine implements (DESIGN.md §27.2): the numbers the
    /// support matrix prints and the docs quote (checked by `tests/docs.rs`).
    pub fn support_summary(&self) -> SupportSummary {
        let moves = |matches: fn(&PrimaryEffect) -> bool| {
            self.moves
                .iter()
                .filter(|m| matches(&m.primary_effect))
                .count()
        };
        SupportSummary {
            moves: self.moves.len(),
            damage: moves(|e| *e == PrimaryEffect::Damage),
            status: moves(|e| matches!(e, PrimaryEffect::Status(_))),
            custom: moves(|e| *e == PrimaryEffect::Custom),
            unsupported: moves(|e| *e == PrimaryEffect::Unsupported),
            abilities: self.abilities.len(),
            abilities_implemented: self.abilities.iter().filter(|a| a.implemented).count(),
            items: self.items.len(),
            items_implemented: self.items.iter().filter(|i| i.implemented).count(),
            species: self.species.len(),
        }
    }
}
