//! Pokémon GO as an origin (M9.3, DESIGN.md §45.4): Showdown's `validatePokemonGo` — a species
//! met in GO (in the wild, from an egg, a raid, research, Giovanni …) reaches Gen 8-9 through
//! HOME at a minimum level with GO's IVs (odd, Atk = SpA and Def = SpD after the transfer).
//! `None` is a set no Pokémon GO origin can explain; an empty list one GO explains.

use super::legality::SpeciesFacts;
use super::set::{Stats, TeamSet};
use super::sources::Sources;
use super::validate::TeamValidator;

impl TeamValidator<'_> {
    /// Showdown `validatePokemonGo(species, set, setSources, name)`.
    pub fn validate_pokemon_go(
        &self,
        species: &SpeciesFacts,
        set: &TeamSet,
        sources: &mut Sources,
        name: Option<&str>,
    ) -> Option<Vec<String>> {
        let table = self.pokemon_go?;
        let mut name = name.unwrap_or(&species.name).to_string();
        if self.generation < 8 || self.mod_name == "gen8dlc1" {
            return None;
        }
        let Some(go) = table.get(&species.id) else {
            // A forme or an evolution GO does not have: its own origin, if it does not have to
            // be levelled up to evolve.
            let other_name = species
                .changes_from
                .clone()
                .or_else(|| species.prevo.clone())
                .unwrap_or_default();
            let other = self.species(&other_name);
            if !other.name.is_empty() && species.evo_level.is_none() {
                return match self.validate_pokemon_go(&other, set, sources, Some(&name)) {
                    Some(problems) => Some(problems),
                    None => {
                        sources.is_from_pokemon_go = Some(false);
                        None
                    }
                };
            }
            sources.is_from_pokemon_go = Some(false);
            return None;
        };
        // (Showdown throws for a species with GO data and no encounters.)
        let encounters = go.encounters.clone().unwrap_or_default();
        let has = |what: &str| encounters.iter().any(|e| e == what);
        let mut problems = Vec::new();
        if set.shiny {
            name = format!("Shiny {name}");
        }
        if set.shiny && has("noshiny") {
            problems.push(format!("{name} is not obtainable from Pokemon GO."));
            return Some(problems);
        }
        let mut min_level: u32 = 50;
        let mut min_ivs: i32 = 15;
        if has("wild") {
            min_level = 1;
            min_ivs = 0;
        }
        if has("egg") {
            min_level = min_level.min(2);
            min_ivs = min_ivs.min(10);
        }
        if has("12kmegg") {
            min_level = min_level.min(8);
            min_ivs = min_ivs.min(10);
        }
        if has("raid") {
            min_level = min_level.min(20);
            min_ivs = min_ivs.min(10);
        }
        if species.id == "mewtwo" && set.level >= 20 {
            // GO Battle League once gave Mewtwo with an IV floor of 0.
            min_ivs = min_ivs.min(0);
        }
        if has("research") {
            min_level = min_level.min(if species.id == "cresselia" { 10 } else { 15 });
            min_ivs = min_ivs.min(10);
        }
        if has("giovanni") && !set.shiny {
            // Purified: levelled down to 8 by a trade.
            min_level = min_level.min(8);
            min_ivs = min_ivs.min(1);
            if set.level < 12 {
                sources.pokemon_go_source = Some("purified".into());
            }
        }
        if !has("notrade") && (!set.shiny || species.id != "deoxys") {
            let special_trade = has("specialtrade") || set.shiny;
            min_level = min_level.min(12);
            min_ivs = min_ivs.min(if special_trade { 1 } else { 0 });
        }
        if set.level < min_level {
            problems.push(format!(
                "{name} must be at least level {min_level} to be from Pokemon GO."
            ));
        }
        let ivs = set.ivs.unwrap_or(Stats::filled(31));
        let post_transfer_min = min_ivs * 2 + 1;
        let non_speed = [ivs.hp, ivs.atk, ivs.def, ivs.spa, ivs.spd];
        if non_speed.iter().any(|iv| *iv < post_transfer_min) {
            problems.push(format!(
                "{name} must have at least {post_transfer_min} {} in non-Speed stats to be from \
                 Pokemon GO.",
                if post_transfer_min == 1 { "IV" } else { "IVs" }
            ));
        }
        if non_speed.iter().any(|iv| iv % 2 == 0) {
            problems.push(format!(
                "{name} must have odd non-Speed IVs to be from Pokemon GO."
            ));
        }
        let can_bottle_cap =
            self.generation >= 7 && set.level >= if self.generation < 9 { 100 } else { 50 };
        if ivs.atk != ivs.spa && !(can_bottle_cap && (ivs.atk == 31 || ivs.spa == 31)) {
            problems.push(format!(
                "{name}'s Atk and Sp. Atk IVs must match to be from Pokemon GO."
            ));
        }
        if ivs.def != ivs.spd && !(can_bottle_cap && (ivs.def == 31 || ivs.spd == 31)) {
            problems.push(format!(
                "{name}'s Def and Sp. Def IVs must match to be from Pokemon GO."
            ));
        }
        Some(problems)
    }
}
