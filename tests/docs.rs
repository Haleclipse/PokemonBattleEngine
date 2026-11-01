//! Numbers the docs quote are derived, not maintained by hand: README's support table and the
//! Gen 9 sentence in DESIGN.md §27.2 sit between `support-summary` markers and must equal what
//! `ResolvedDex::support_summary` computes. A stale block fails with the text to paste.

use std::path::PathBuf;

use pokemon_battle_engine::dex::{DexResolver, SupportSummary};
use pokemon_battle_engine::spec::Generation;

const BEGIN: &str = "<!-- support-summary:begin (generated: tests/docs.rs) -->";
const END: &str = "<!-- support-summary:end -->";

fn readme_block(s: &SupportSummary) -> String {
    format!(
        "\n| 对象 | 支持 | 总数 |\n|---|---|---|\n\
         | 招式 Damage / Status / Custom | {} / {} / {} | {} |\n\
         | 招式 Unsupported | {} | |\n\
         | 特性 implemented | {} | {} |\n\
         | 道具 implemented | {} | {} |\n",
        s.damage,
        s.status,
        s.custom,
        s.moves,
        s.unsupported,
        s.abilities_implemented,
        s.abilities,
        s.items_implemented,
        s.items,
    )
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
    let summary = DexResolver::resolve(Generation::Gen9).support_summary();
    // DESIGN.md is a working document kept locally, not in the published repository: its block is
    // checked where the file is present.
    let local = |file: &str| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(file)
            .exists()
    };
    for (file, expected) in [
        ("README.md", readme_block(&summary)),
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
