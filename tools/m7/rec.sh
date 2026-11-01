#!/bin/bash
# Record a scenario against Showdown into the worktree's fixtures and print each frame's moves, switches and faints.
# Usage: tools/m7/rec.sh <scenario.json> [seed a,b,c,d]   (WT=<worktree>, default: the checkout you run it from)
set -e
WT=${WT:-$(git rev-parse --show-toplevel)}
MAIN=${MAIN:-$(dirname "$(git -C "$WT" rev-parse --path-format=absolute --git-common-dir)")}
template=$1
id=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['id'])" "$template")
if [ -n "$2" ]; then
  python3 - "$template" "$2" <<'PY'
import json, sys
path = sys.argv[1]; scenario = json.load(open(path))
scenario['random']['seed'] = [int(x) for x in sys.argv[2].split(',')]
open(path, 'w').write(json.dumps(scenario, indent=1) + '\n')
PY
fi
# A worktree borrows the main repository's oracle packages (the symlink is never committed).
if [ ! -e "$WT/tools/oracle-node/node_modules" ]; then
  ln -s "$MAIN/tools/oracle-node/node_modules" "$WT/tools/oracle-node/node_modules"
fi
fixture=$WT/tests/fixtures/cases/$id.json
copied=
if [ "$(cd "$(dirname "$template")" && pwd)/$(basename "$template")" != "$fixture" ]; then
  cp "$template" "$fixture"; copied=1
fi
recording=$WT/tests/fixtures/cases/$id.showdown.json
if ! (cd "$WT/tools/oracle-node" && node src/showdown.mjs < "$fixture" > "$recording" 2> "$recording.err"); then
  # The script no longer fits the battle (a faint, a forced switch …): keep no half recording.
  grep -m1 "^Error" "$recording.err" >&2
  rm -f "$recording" "$recording.err"
  [ -n "$copied" ] && rm -f "$fixture"
  exit 1
fi
rm -f "$recording.err"
python3 - "$recording" <<'PY'
import json, sys
recording = json.load(open(sys.argv[1]))
for frame in recording['frames']:
    out = []
    for e in frame['events']:
        kind = e['kind']
        if kind == 'move': out.append(f"{e['user']['name']}:{e['move']}")
        elif kind == 'switch': out.append('sw:' + e['pokemon']['name'])
        elif kind == 'faint': out.append('faint:' + e['pokemon']['name'])
        elif kind == 'cant': out.append('cant:' + e['pokemon']['name'])
    print(frame['checkpoint'], ' '.join(out))
PY
