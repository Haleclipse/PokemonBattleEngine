#!/usr/bin/env bash
# Local M8 nightly (ROADMAP §12 DoD, TESTING.md §6.5): differential campaigns against the pinned
# Showdown for a fixed time, then triage, minimization of every new signature and a report. It
# runs entirely on this machine; nothing is uploaded.
#
#   scripts/nightly.sh [--minutes N] [--stop-on-new] [other tools/nightly.py options …]
#
# `--stop-on-new` runs no further campaign once one has a new signature, which is still minimized
# and reported: a night that cannot be clean stops early (DESIGN.md R-M10-70).
#
# Each run writes its own directory, target/m8/nightly-runs/<YYYYmmdd-HHMMSS>/ — `nightly.md` /
# `nightly.json` (pass metrics, known disagreements, one minimized failure bundle per new
# signature, mechanic coverage), the campaigns' summaries, results and failure bundles, and the
# run's log — and target/m8/nightly-runs/latest points at the newest. The mechanic coverage
# (target/m8/coverage.json) and the next seed per rotation slot (target/m8/nightly-state.json)
# carry over from run to run, so a night never replays an earlier one.
#
# Exit status: 0 no new signature, 1 new signatures, 2 the run could not start.
#
# The campaigns judge with this checkout's release build: do not edit Rust sources or rebuild in
# this checkout while a run is going (work in a separate worktree instead).
set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
  echo "nightly: $*" >&2
  exit 2
}

minutes=60
args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --minutes) minutes="${2:?--minutes needs a value}"; shift 2 ;;
    --minutes=*) minutes="${1#--minutes=}"; shift ;;
    --out | --out=*) fail "the run directory is chosen by this script" ;;
    *) args+=("$1"); shift ;;
  esac
done

for tool in cargo node npm python3; do
  command -v "$tool" >/dev/null || fail "$tool not found"
done
node_major="$(node -p 'process.versions.node.split(".")[0]')"
[ "$node_major" -ge 22 ] || fail "node 22 or newer is needed (found $(node --version))"

# One run at a time: two would share the seed state and the coverage file.
mkdir -p target/m8
lock=target/m8/nightly.lock
mkdir "$lock" 2>/dev/null || fail "another run holds $lock (remove it if that run is gone)"
trap 'rmdir "$lock"' EXIT

# The pinned oracle (@pkmn/sim): installed from the lockfile, again whenever the lockfile changed.
oracle=tools/oracle-node
if [ ! -d "$oracle/node_modules" ] || [ "$oracle/package-lock.json" -nt "$oracle/node_modules/.package-lock.json" ]; then
  echo "== npm ci ($oracle)"
  (cd "$oracle" && npm ci --no-audit --no-fund)
fi

# Build up front, so a compile error stops the night before any campaign.
echo "== build"
cargo build -q --example content_pool
cargo test -q --release --test campaign --no-run

run="target/m8/nightly-runs/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$run"
echo "== nightly: $minutes minutes → $run"
status=0
python3 tools/nightly.py --minutes "$minutes" --out "$PWD/$run" --fail-on-new ${args[@]+"${args[@]}"} \
  2>&1 | tee "$run/nightly.log" || status=${PIPESTATUS[0]}
ln -sfn "$(basename "$run")" target/m8/nightly-runs/latest

echo
echo "report: $run/nightly.md"
if [ "$status" -eq 1 ]; then
  echo "new signatures: minimized battles in $run/*/failures/"
fi
exit "$status"
