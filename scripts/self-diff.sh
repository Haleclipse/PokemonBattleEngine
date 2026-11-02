#!/bin/bash
# The structural refactor's old-against-new check (DESIGN.md §48.3): the battles of
# `examples/self_diff.rs` and the replays of `tests/fixtures/cases` digested in a checkout of the
# commit before the batch and in this one; the batch changed no behavior they reach when the
# digests are equal, byte for byte (every request, trace event and random state at every
# boundary, every replay's full trace and frames, every rule profile in full).
#
# Usage: scripts/self-diff.sh <checkout of the base commit> [battles per format, default 100]
# Output: target/self-diff/{base,head}.txt and the first differences.
set -euo pipefail
cd "$(dirname "$0")/.."
base=$(cd "${1:?usage: scripts/self-diff.sh <checkout of the base commit> [battles]}" && pwd)
battles=${2:-100}
head=$(pwd)
out=$head/target/self-diff
mkdir -p "$out"

digests() {
  local tree=$1 name=$2
  if [ ! -e "$tree/examples/self_diff.rs" ]; then
    echo "$tree: no examples/self_diff.rs (a commit before R-M11-5)" >&2
    exit 2
  fi
  (
    cd "$tree"
    cargo run -q --release --example self_diff -- "$battles" > "$out/$name.battles.txt"
    REPLAY_DIGESTS="$out/$name.replays.txt" \
      cargo test -q --release --test showdown_oracle showdown_scenarios_replay_frame_by_frame \
      > /dev/null
  )
  cat "$out/$name.battles.txt" "$out/$name.replays.txt" > "$out/$name.txt"
}

echo "== base: $(git -C "$base" log --oneline -1)"
digests "$base" base
echo "== head: $(git log --oneline -1) + working tree"
digests "$head" head
if cmp -s "$out/base.txt" "$out/head.txt"; then
  echo "self-diff: identical ($(wc -l < "$out/head.txt" | tr -d ' ') digests)"
else
  echo "self-diff: DIFFERENT" >&2
  diff "$out/base.txt" "$out/head.txt" | head -20 >&2
  exit 1
fi
