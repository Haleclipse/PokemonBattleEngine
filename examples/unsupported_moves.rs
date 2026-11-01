//! Lists the moves a generation leaves unsupported with their reasons, grouped by the reason
//! set: `cargo run --example unsupported_moves -- 9 [max-reasons]`. The M7 work queue.

use std::collections::BTreeMap;

use pokemon_battle_engine::dex::{DexResolver, PrimaryEffect};
use pokemon_battle_engine::spec::Generation;

fn main() {
    let mut args = std::env::args().skip(1);
    let generation = match args.next().as_deref().unwrap_or("9") {
        "1" => Generation::Gen1,
        "2" => Generation::Gen2,
        "3" => Generation::Gen3,
        "4" => Generation::Gen4,
        "5" => Generation::Gen5,
        "6" => Generation::Gen6,
        "7" => Generation::Gen7,
        "8" => Generation::Gen8,
        _ => Generation::Gen9,
    };
    let max_reasons: usize = args
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(usize::MAX);
    let dex = DexResolver::resolve(generation);
    let mut groups: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
    for move_def in dex.moves() {
        if move_def.primary_effect != PrimaryEffect::Unsupported {
            continue;
        }
        let mut reasons = move_def.unsupported.clone();
        reasons.sort();
        reasons.dedup();
        if reasons.len() > max_reasons {
            continue;
        }
        groups
            .entry(reasons)
            .or_default()
            .push(move_def.name.clone());
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|(_, moves)| std::cmp::Reverse(moves.len()));
    for (reasons, moves) in groups {
        println!("{:3}  {}", moves.len(), reasons.join(" + "));
        println!("     {}", moves.join(", "));
    }
}
