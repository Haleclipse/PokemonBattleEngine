#!/usr/bin/env bash
# Local equivalent of the CI `check` job. Run before every commit; CI runs exactly these steps.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== rustfmt"
cargo fmt --all --check
cargo fmt --manifest-path stable-api/Cargo.toml --check
echo "== clippy (zero warnings)"
cargo clippy --all-targets --all-features -- -D warnings
# The stable API alone (DESIGN.md §47.5): the default build, whose lints refuse a type the API
# reaches but does not name, and a package of its own that uses nothing else. (Every other step
# builds with `internals`: the crate's own tests depend on it with the feature on.)
echo "== the stable API"
cargo clippy --lib -- -D warnings
cargo clippy --manifest-path stable-api/Cargo.toml --target-dir target/stable-api --all-targets -- -D warnings
cargo test --manifest-path stable-api/Cargo.toml --target-dir target/stable-api
echo "== the stable API's docs and examples"
# The default build's docs show the stable API alone (DESIGN.md §47.5): no internal module has a
# page of its own (rustdoc keeps redirects at a re-exported item's own path), and every link holds.
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --target-dir target/stable-doc
for module in data dex digest effects engine format ids rules session spec state; do
  if [ -e "target/stable-doc/doc/pokemon_battle_engine/$module/index.html" ]; then
    echo "the default build's docs show the internal module $module" >&2
    exit 1
  fi
done
stable_example() {
  cargo run --manifest-path stable-api/Cargo.toml --target-dir target/stable-api --example "$@" >/dev/null
}
stable_example rollout -- 2
stable_example server_loop
stable_example research
echo "== tests"
cargo test --all-targets
cargo test --doc
echo "== example"
cargo run --example singles_turn >/dev/null
echo "all checks passed"
