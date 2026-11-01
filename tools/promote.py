#!/usr/bin/env python3
"""M8 regression promotion (ROADMAP §12.4, TESTING.md §6.5).

    python3 tools/promote.py <minimized.json> <case-name> "<what the case pins>" [--force]

Turns a minimized battle (`tools/minimize.py`) into a permanent regression case once the fix
is in: `tests/fixtures/cases/<case-name>.json` with the note and its provenance, and the
`.showdown.json` recorded again from Showdown. The case must replay without a divergence
(`showdown_oracle`); otherwise nothing is kept and the divergence is printed — the fix is not
in yet, or the minimized battle pins something else. Check it catches the bug by undoing the
fix and watching the case diverge (a mutation check).
"""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / 'tools' / 'oracle-node'
CASES = ROOT / 'tests' / 'fixtures' / 'cases'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('battle', type=Path)
    parser.add_argument('name')
    parser.add_argument('note')
    parser.add_argument('--force', action='store_true', help='replace an existing case')
    args = parser.parse_args()

    case = CASES / f'{args.name}.json'
    oracle_path = CASES / f'{args.name}.showdown.json'
    if case.exists() and not args.force:
        sys.exit(f'{case.name} exists (--force replaces it)')

    scenario = json.loads(args.battle.read_text())
    origin = scenario.get('id', args.battle.stem)
    scenario.pop('corpus', None)
    scenario['id'] = args.name
    scenario['notes'] = [args.note, f'promoted from {origin} (tools/minimize.py, tools/promote.py)']
    recorded = subprocess.run(['node', 'src/showdown.mjs'], input=json.dumps(scenario), cwd=ORACLE,
                              capture_output=True, text=True)
    if recorded.returncode != 0:
        sys.exit(f'Showdown rejects the battle: {recorded.stderr.strip().splitlines()[-1:]}')

    backup = {path: path.read_text() for path in (case, oracle_path) if path.exists()}
    case.write_text(json.dumps(scenario, indent=2, ensure_ascii=False) + '\n')
    oracle_path.write_text(recorded.stdout)
    replay = subprocess.run(['cargo', 'test', '-q', '--release', '--test', 'showdown_oracle', '--', '--nocapture'],
                            cwd=ROOT, capture_output=True, text=True, env={**os.environ, 'SCENARIO': args.name})
    report = [line.lstrip('.') for line in replay.stdout.splitlines() if f'{args.name}:' in line]
    if replay.returncode != 0:
        for path in (case, oracle_path):
            if path in backup:
                path.write_text(backup[path])
            else:
                path.unlink()
        detail = [line for line in (replay.stdout + replay.stderr).splitlines()
                  if 'boundary' in line or 'panicked' in line][:5]
        sys.exit('the case still diverges, nothing kept:\n' + '\n'.join(report + detail))
    print('\n'.join(report))
    print(f'promoted {origin} → {case.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
