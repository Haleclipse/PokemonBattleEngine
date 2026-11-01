#!/usr/bin/env python3
"""Find a seed for a hand-scripted scenario (TESTING.md §6.3).

    python3 tools/seek-seed.py <scenario.json> <max> <regex> [regex ...] [--where EXPR]

Tries seeds [n, n+1, n+2, n+3] for n = 1 .. max until Showdown's protocol log (the oracle's
`debug_log`) matches every regex (`.` spans lines) — a freeze on turn 1 followed by a thaw, a
one-hit KO that lands — and, with `--where`, until the Python expression holds over `frames`
(the oracle's frames by checkpoint name: two confusion durations that differ, a status cleared
by a faint). It writes the scenario with that seed and its .showdown.json next to it. A seed
whose script Showdown rejects (a forced switch that never came) is skipped.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / 'tools' / 'oracle-node'


def main() -> None:
    args = sys.argv[1:]
    where = None
    if '--where' in args:
        at = args.index('--where')
        where = args[at + 1]
        del args[at:at + 2]
    path = Path(args[0])
    limit = int(args[1])
    patterns = args[2:]
    scenario = json.loads(path.read_text())
    for n in range(1, limit):
        scenario['random']['seed'] = [n, n + 1, n + 2, n + 3]
        text = json.dumps(scenario, indent=2) + '\n'
        run = subprocess.run(['node', 'src/showdown.mjs'], input=text, capture_output=True,
                             text=True, cwd=ORACLE)
        if run.returncode != 0:
            continue
        oracle = json.loads(run.stdout)
        if not all(re.search(p, oracle['debug_log'], re.S) for p in patterns):
            continue
        frames = {frame['checkpoint']: frame for frame in oracle['frames']}
        if where is None or eval(where, {}, {'frames': frames}):
            path.write_text(text)
            path.with_name(path.name.replace('.json', '.showdown.json')).write_text(run.stdout)
            print(f'seed {n} -> {path.name}')
            return
    raise SystemExit('no seed found')


if __name__ == '__main__':
    main()
