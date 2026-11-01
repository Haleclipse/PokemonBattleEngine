//! Species: the snapshot's rows as `ResolvedSpecies`.

use std::collections::BTreeMap;

use crate::data::{self, DexSnapshot};
use crate::dex::{BaseStats, FormeKind, PokemonType, ResolvedAbility, ResolvedSpecies};
use crate::ids::SpeciesId;
use crate::state::{Gender, PokemonTypes};

/// Every species of the generation (DESIGN.md §27.1): types, base stats, abilities by id,
/// weight and forme.
pub(super) fn resolve_species(
    snapshot: &DexSnapshot,
    abilities: &[ResolvedAbility],
) -> Vec<ResolvedSpecies> {
    let ability_by_name = abilities
        .iter()
        .map(|ability| (data::to_id(&ability.name), ability.id))
        .collect::<BTreeMap<_, _>>();
    // A forme's `requiredMove` (Mega Rayquaza) resolves by name (DESIGN.md §44.3).
    let move_by_name = snapshot
        .moves
        .iter()
        .map(|move_| (move_.id.clone(), crate::ids::MoveId(move_.eid)))
        .collect::<BTreeMap<_, _>>();

    snapshot
        .species
        .iter()
        .filter_map(|species| {
            let mut types = species
                .types
                .iter()
                .map(|name| PokemonType::from_name(name));
            let primary = types.next()??;
            let secondary = types.next().flatten();
            let mut abilities = Vec::new();
            for slot in ["0", "1", "H", "S"] {
                if let Some(name) = species.abilities.get(slot)
                    && let Some(id) = ability_by_name.get(&data::to_id(name))
                {
                    abilities.push(*id);
                }
            }
            Some(ResolvedSpecies {
                id: SpeciesId {
                    dex: u16::try_from(species.num).ok()?,
                    form: species.form,
                },
                key: species.id.clone(),
                name: species.name.clone(),
                forme_kind: match species.forme_kind.as_deref() {
                    None => FormeKind::Base,
                    Some("mega") => FormeKind::Mega,
                    Some("primal") => FormeKind::Primal,
                    Some("regional") => FormeKind::Regional,
                    Some("battle_only") => FormeKind::BattleOnly,
                    Some(_) => FormeKind::Other,
                },
                types: match secondary {
                    Some(secondary) => PokemonTypes::dual(primary, secondary),
                    None => PokemonTypes::mono(primary),
                },
                base_stats: BaseStats {
                    hp: species.base_stats.hp,
                    attack: species.base_stats.atk,
                    defense: species.base_stats.def,
                    special_attack: species.base_stats.spa,
                    special_defense: species.base_stats.spd,
                    speed: species.base_stats.spe,
                },
                abilities,
                weight_hg: (species.weight_kg * 10.0).round() as u32,
                base_species: species.base_species.clone(),
                forme: species.forme.clone(),
                changes_from: species.changes_from.clone(),
                nfe: species.nfe,
                gender: species.gender.map(|gender| match gender {
                    data::GenderSnapshot::M => Gender::Male,
                    data::GenderSnapshot::F => Gender::Female,
                    data::GenderSnapshot::N => Gender::Genderless,
                }),
                cannot_dynamax: species.cannot_dynamax,
                required_move: species
                    .required_move
                    .as_deref()
                    .and_then(|name| move_by_name.get(&data::to_id(name)).copied()),
            })
        })
        .collect()
}
