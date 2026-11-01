//! The M7 inventory (ROADMAP §11): every move, ability and item of every generation's snapshot
//! that the engine does not implement, with what blocks it, as JSON lines
//! (`{"generation", "kind", "id", "name", "reasons"}`) — `cargo run --example inventory`.
//! `tools/inventory.py` folds the generations together and groups the entries by the mechanism
//! they miss. The snapshot lists each generation exhaustively, so an empty output is the end of
//! the content work.

use pokemon_battle_engine::dex::{DexResolver, PrimaryEffect};
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

fn line(generation: Generation, kind: &str, id: String, name: &str, reasons: &[String]) {
    let mut reasons = reasons.to_vec();
    reasons.sort();
    reasons.dedup();
    println!(
        "{}",
        serde_json::json!({
            "generation": generation.number(),
            "kind": kind,
            "id": id,
            "name": name,
            "reasons": reasons,
        })
    );
}

fn main() {
    for generation in GENERATIONS {
        let dex = DexResolver::resolve(generation);
        for move_def in dex.moves() {
            if move_def.primary_effect == PrimaryEffect::Unsupported {
                let id = format!("{:?}", move_def.id);
                line(
                    generation,
                    "move",
                    id,
                    &move_def.name,
                    &move_def.unsupported,
                );
            }
        }
        for ability in dex.abilities().iter().filter(|a| !a.implemented) {
            let id = format!("{:?}", ability.id);
            line(
                generation,
                "ability",
                id,
                &ability.name,
                &ability.missing_callbacks,
            );
        }
        for item in dex.items().iter().filter(|i| !i.implemented) {
            let id = format!("{:?}", item.id);
            line(generation, "item", id, &item.name, &item.missing_callbacks);
        }
    }
}
