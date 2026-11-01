#!/usr/bin/env python3
"""Build a whole-battle corpus scenario from a roster (TESTING.md §6.4).

    python3 tools/build-corpus.py <gen> <s0,s1,s2,s3> <roster.json> [suffix] [singles|doubles]

The roster is {"p1": [[label, species, [moves], ability|null, item|null], ...], "p2": [...]}.
Every name is checked with `examples/check_support` first (an unsupported move is refused at
choice validation, an unsupported ability or item would silently do nothing). The deterministic
policy (`tools/oracle-node/src/corpus.mjs`) plays both sides in Showdown and writes the choices
back as the script; `showdown.mjs` then records the oracle frames. Writes
tests/fixtures/cases/gen<N>[-doubles]-corpus<suffix>.json and its .showdown.json.
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / 'tools' / 'oracle-node'


def main() -> None:
    gen = int(sys.argv[1])
    seed = [int(x) for x in sys.argv[2].split(',')]
    roster = json.loads(Path(sys.argv[3]).read_text())
    suffix = sys.argv[4] if len(sys.argv) > 4 else ''
    mode = sys.argv[5] if len(sys.argv) > 5 else 'singles'
    fmt = f'gen{gen}doublescustomgame' if mode == 'doubles' else f'gen{gen}customgame'

    def mon(m):
        out = {'label': m[0], 'species': m[1], 'level': 50, 'nature': 'Hardy', 'moves': m[2]}
        if m[3]:
            out['ability'] = m[3]
        if m[4]:
            out['item'] = m[4]
        return out

    scenario = {
        'schema': 1,
        'id': f'gen{gen}-{mode}-corpus{suffix}',
        'notes': [f'Whole-battle corpus: a six-on-six Gen {gen} {mode} battle from the '
                  'deterministic policy (tools/oracle-node/src/corpus.mjs).'],
        'battle': {'generation': gen, 'mode': mode, 'format': fmt},
        'teams': {'p1': [mon(m) for m in roster['p1']], 'p2': [mon(m) for m in roster['p2']]},
        'random': {'mode': 'oracle-guided', 'seed': seed},
        'corpus': {'switchEvery': 6, 'maxTurns': 80},
    }
    names = set()
    for m in roster['p1'] + roster['p2']:
        names.update([m[1]] + m[2] + [x for x in m[3:5] if x])
    check = subprocess.run(['cargo', 'run', '-q', '--example', 'check_support', '--', '--gen',
                            str(gen), *sorted(names)], capture_output=True, text=True, cwd=ROOT)
    unsupported = [line for line in check.stdout.splitlines() if not line.startswith('ok')]
    if unsupported:
        raise SystemExit(f'unsupported: {unsupported}')
    corpus = subprocess.run(['node', 'src/corpus.mjs'], input=json.dumps(scenario),
                            capture_output=True, text=True, cwd=ORACLE, check=True)
    stem = f'gen{gen}-doubles-corpus{suffix}' if mode == 'doubles' else f'gen{gen}-corpus{suffix}'
    (ROOT / 'tests/fixtures/cases' / f'{stem}.json').write_text(corpus.stdout)
    frames = subprocess.run(['node', 'src/showdown.mjs'], input=corpus.stdout,
                            capture_output=True, text=True, cwd=ORACLE, check=True)
    (ROOT / 'tests/fixtures/cases' / f'{stem}.showdown.json').write_text(frames.stdout)
    print(json.loads(corpus.stdout)['notes'][-1])


if __name__ == '__main__':
    main()
