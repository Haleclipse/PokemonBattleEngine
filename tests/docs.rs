//! Numbers the docs quote are derived, not maintained by hand: README's support table (every
//! generation) and the Gen 9 sentence in DESIGN.md §27.2 sit between `support-summary` markers and
//! must equal what `ResolvedDex::support_summary` computes. A stale block fails with the text to
//! paste.

use std::path::PathBuf;

use pokemon_battle_engine::dex::{DexResolver, SupportSummary};
use pokemon_battle_engine::spec::Generation;

const BEGIN: &str = "<!-- support-summary:begin (generated: tests/docs.rs) -->";
const END: &str = "<!-- support-summary:end -->";

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

/// One row per generation; a generation with no abilities (Gen 1-2) shows a dash.
fn readme_block(summaries: &[(Generation, SupportSummary)]) -> String {
    let mut block = String::from(
        "\n| 世代 | 招式 | Damage / Status / Custom | 不支持的招式 | 特性（已实现 / 全部） | 道具（已实现 / 全部） |\n\
         |---|---|---|---|---|---|\n",
    );
    for (generation, s) in summaries {
        let abilities = if s.abilities == 0 {
            "—".to_owned()
        } else {
            format!("{} / {}", s.abilities_implemented, s.abilities)
        };
        block.push_str(&format!(
            "| Gen {} | {} | {} / {} / {} | {} | {} | {} / {} |\n",
            generation.number(),
            s.moves,
            s.damage,
            s.status,
            s.custom,
            s.unsupported,
            abilities,
            s.items_implemented,
            s.items,
        ));
    }
    block
}

fn design_block(s: &SupportSummary) -> String {
    format!(
        "当前 Gen9：{} 个招式中 {} Damage、{} Status、{} Custom、{} Unsupported；特性 {} / {}，道具 {} / {}。",
        s.moves,
        s.damage,
        s.status,
        s.custom,
        s.unsupported,
        s.abilities_implemented,
        s.abilities,
        s.items_implemented,
        s.items,
    )
}

fn block(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(file);
    let text = std::fs::read_to_string(&path).expect("read");
    let start = text
        .find(BEGIN)
        .unwrap_or_else(|| panic!("{file}: no {BEGIN}"))
        + BEGIN.len();
    let end = text[start..]
        .find(END)
        .unwrap_or_else(|| panic!("{file}: no {END}"))
        + start;
    text[start..end].to_owned()
}

#[test]
fn support_numbers_in_the_docs_match_the_dex() {
    let summaries: Vec<(Generation, SupportSummary)> = GENERATIONS
        .iter()
        .map(|generation| {
            (
                *generation,
                DexResolver::resolve(*generation).support_summary(),
            )
        })
        .collect();
    let summary = summaries.last().expect("Gen 9").1;
    // DESIGN.md is a working document kept locally, not in the published repository: its block is
    // checked where the file is present.
    let local = |file: &str| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(file)
            .exists()
    };
    for (file, expected) in [
        ("README.md", readme_block(&summaries)),
        ("DESIGN.md", design_block(&summary)),
    ] {
        if file != "README.md" && !local(file) {
            continue;
        }
        let actual = block(file);
        assert!(
            actual == expected,
            "{file}: the support-summary block is stale; replace it with:\n{BEGIN}{expected}{END}"
        );
    }
}
