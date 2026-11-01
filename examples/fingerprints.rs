//! Prints the engine's fingerprints as JSON — what a release publishes beside its tag
//! (`.github/workflows/release.yml`): the crate's version and commit, every generation's dex, the
//! rules of every custom game the engine plays (Gen 1-9 singles, Gen 3-9 doubles) and the
//! session and battle rules of every format in `data/formats`. A snapshot or a session taken under
//! one fingerprint is refused under another, so two releases with equal fingerprints restore each
//! other's snapshots, and a changed one says which rules or data moved.
//!
//! Run: cargo run --example fingerprints [-- <commit>]

use pokemon_battle_engine::FormatLibrary;
use pokemon_battle_engine::SessionFormat;
use pokemon_battle_engine::dex::DexResolver;
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use serde_json::json;

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
    let commit = std::env::args().nth(1);
    let dex: Vec<_> = GENERATIONS
        .iter()
        .map(|generation| {
            json!({
                "generation": generation.number(),
                "fingerprint": DexResolver::resolve(*generation).fingerprint(),
            })
        })
        .collect();
    let mut custom = Vec::new();
    for generation in GENERATIONS {
        for (mode, name) in [
            (BattleMode::Singles, "singles"),
            (BattleMode::Doubles, "doubles"),
        ] {
            // A mode the generation never had (Gen 1-2 doubles) is refused, and listed as such.
            let rules = RulesResolver::resolve(&BattleSpec::new(generation, mode, "custom"))
                .ok()
                .map(|rules| rules.fingerprint().0.clone());
            if let Some(rules) = rules {
                custom.push(json!({
                    "generation": generation.number(),
                    "mode": name,
                    "rules": rules,
                }));
            }
        }
    }
    let library = FormatLibrary::open(concat!(env!("CARGO_MANIFEST_DIR"), "/data"))
        .unwrap_or_else(|error| panic!("the format data: {error}"));
    let formats: Vec<_> = library
        .formats()
        .formats
        .iter()
        .map(|entry| {
            let format = SessionFormat::new(&library, &entry.id)
                .unwrap_or_else(|error| panic!("{}: {error:?}", entry.id));
            let rules = format
                .rules()
                .unwrap_or_else(|error| panic!("{}: {error:?}", entry.id));
            json!({
                "id": entry.id,
                "session": format.fingerprint(),
                "rules": rules.fingerprint().0,
            })
        })
        .collect();
    let manifest = json!({
        "crate": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        "commit": commit,
        "dex": dex,
        "custom": custom,
        "formats": formats,
    });
    println!("{}", serde_json::to_string_pretty(&manifest).expect("JSON"));
}
