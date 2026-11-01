#!/bin/bash
# Replay campaign directories on the worktree's build, on copies (the originals keep their results); only the copies of diverging campaigns are kept.
# Usage: tools/m7/replay_all.sh [campaign dir ...]   (default: every campaign under <main>/target/m8; WT=<worktree>, OUT=<dir>, default <main>/work/<branch>/replay)
WT=${WT:-$(git rev-parse --show-toplevel)}
MAIN=${MAIN:-$(dirname "$(git -C "$WT" rev-parse --path-format=absolute --git-common-dir)")}
M8=$MAIN/target/m8
OUT=${OUT:-$MAIN/work/$(git -C "$WT" branch --show-current)/replay}
rm -rf "$OUT"; mkdir -p "$OUT"
(cd "$WT" && cargo test -q --release --test campaign --no-run 2>/dev/null)
if [ $# -gt 0 ]; then dirs=("$@"); else
  dirs=("$M8"/gen*-* "$M8"/nightly-1/gen*-* "$M8"/nightly-2/gen*-* "$M8"/nightly-3/gen*-* "$M8"/nightly-runs/2*/gen*-* "$M8"/ab/*-gen*)
fi
for d in "${dirs[@]}"; do
  d=${d%/}
  [ -f "$d/results.json" ] || continue
  case "$d" in "$M8"/*) name=$(echo "${d#"$M8"/}" | tr '/' '_');; *) name=$(basename "$d");; esac
  mkdir -p "$OUT/$name"; cp "$d"/*.json "$OUT/$name/" 2>/dev/null; rm -f "$OUT/$name/results.json"
  line=$(cd "$WT" && CAMPAIGN_DIR="$OUT/$name" cargo test -q --release --test campaign -- --ignored --nocapture 2>&1 | grep "campaign:")
  echo "$name: $line"
  case "$line" in *" 0 diverged, 0 panicked"*) rm -rf "${OUT:?}/$name";; esac
done
