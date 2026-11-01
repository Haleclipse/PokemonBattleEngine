# Scenario cases (L3)

Each case is a pair:

| File | Content |
|---|---|
| `<id>.json` | neutral scenario (TESTING.md §4): battle, teams by name, script in Showdown choice syntax |
| `<id>.showdown.json` | pinned output of `tools/oracle-node/src/showdown.mjs`: frames per request boundary plus the normalised event stream |

Lifecycle:

1. Author `<id>.json`. Keep the first cases minimal: abilities without battle callbacks, one
   turn, moves the engine classifies as supported (`cargo run --example support_matrix`).
2. Generate the oracle output once:
   `cd tools/oracle-node && node src/showdown.mjs < ../../tests/fixtures/cases/<id>.json > ../../tests/fixtures/cases/<id>.showdown.json`
3. Review the generated frames against the cited mechanics sources before committing.
4. `cargo test --test showdown_oracle` replays every pair offline (oracle-guided random
   decisions, TESTING.md §6.3) and reports the first divergence.
5. A confirmed divergence is minimised and copied to `tests/fixtures/regressions`.

Expected outputs are never regenerated automatically; regenerate only with a provenance bump.
