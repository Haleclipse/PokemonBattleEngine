//! Prints the engine's support matrix: how much of each generation's canonical dex the engine
//! actually implements. "Unsupported" is a first-class answer, never a silent no-op.
//!
//! Run: cargo run --example support_matrix

use std::collections::BTreeMap;

use pokemon_battle_engine::dex::DexResolver;
use pokemon_battle_engine::effects;
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

fn main() {
    println!(
        "{:<5} {:>6} {:>6} {:>11} {:>7} | {:>9} {:>9} | {:>9} {:>9} | {:>7}",
        "gen",
        "damage",
        "custom",
        "unsupported",
        "moves",
        "abil impl",
        "abil all",
        "item impl",
        "item all",
        "species"
    );
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for generation in GENERATIONS {
        let dex = DexResolver::resolve(generation);
        let summary = dex.support_summary();
        println!(
            "{:<5} {:>6} {:>6} {:>11} {:>7} | {:>9} {:>9} | {:>9} {:>9} | {:>7}",
            format!("gen{}", generation.number()),
            summary.damage,
            summary.custom,
            summary.unsupported,
            summary.moves,
            summary.abilities_implemented,
            summary.abilities,
            summary.items_implemented,
            summary.items,
            summary.species,
        );
        if generation == Generation::Gen9 {
            let partial: Vec<String> = dex
                .abilities()
                .iter()
                .filter(|a| {
                    !a.implemented && effects::abilities::definition(a.id).switch_in.is_some()
                        || !a.implemented && ability_partially_covered(a.id)
                })
                .map(|a| format!("{} (missing {})", a.name, a.missing_callbacks.join(", ")))
                .chain(
                    dex.items()
                        .iter()
                        .filter(|i| !i.implemented && item_partially_covered(i.id))
                        .map(|i| {
                            format!("{} (missing {})", i.name, i.missing_callbacks.join(", "))
                        }),
                )
                .collect();
            if !partial.is_empty() {
                println!(
                    "\nGen 9 partially covered (counted as unsupported): {}",
                    partial.join("; ")
                );
            }
            for move_def in dex.moves() {
                for reason in &move_def.unsupported {
                    let class = reason.splitn(3, ':').take(2).collect::<Vec<_>>().join(":");
                    *reasons.entry(class).or_default() += 1;
                }
            }
        }
    }

    let mut ranked = reasons.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("\nGen 9 unsupported reasons by class (a move may carry several):");
    for (class, count) in ranked.iter().take(30) {
        println!("  {count:>5}  {class}");
    }
}

fn ability_partially_covered(id: pokemon_battle_engine::ids::AbilityId) -> bool {
    let h = effects::abilities::definition(id);
    h.switch_in.is_some()
        || h.try_hit.is_some()
        || h.try_set_status.is_some()
        || h.after_set_status.is_some()
        || h.modify_offense.is_some()
        || h.base_power.is_some()
        || h.ally_base_power.is_some()
        || h.damaging_hit_reaction.is_some()
        || h.damage_resolution.is_some()
        || h.modify_stat_change.is_some()
        || h.try_stat_change.is_some()
}

fn item_partially_covered(id: pokemon_battle_engine::ids::ItemId) -> bool {
    let h = effects::items::definition(id);
    h.try_hit.is_some()
        || h.try_set_status.is_some()
        || h.after_set_status.is_some()
        || h.modify_offense.is_some()
        || h.damaging_hit_reaction.is_some()
        || h.damage_resolution.is_some()
        || h.try_stat_change.is_some()
        || h.residual.is_some()
}
