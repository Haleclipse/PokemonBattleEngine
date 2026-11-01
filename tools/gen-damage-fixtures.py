#!/usr/bin/env python3
"""Generate tests/fixtures/oracle/damage-expected.json from damage-cases.json via @smogon/calc.

Each hand-written case describes both Pokémon with final (pre-boost) stats. The calc adapter
overrides rawStats so the arithmetic on both sides starts from identical numbers; species is a
placeholder and types are overridden explicitly. Run only together with a provenance bump.
"""
from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / 'tools' / 'oracle-node'
CASES = ROOT / 'tests' / 'fixtures' / 'oracle' / 'damage-cases.json'
OUT = ROOT / 'tests' / 'fixtures' / 'oracle' / 'damage-expected.json'

# The Rust engine models "no ability" explicitly; calc needs some name. Illuminate has no
# in-battle damage effect in any generation that has abilities.
NO_ABILITY = 'Illuminate'


def calc_side(side: dict) -> dict:
    options = {
        'level': side['level'],
        'overrides': {'types': side['types']},
        'ability': side.get('ability') or NO_ABILITY,
    }
    if options['ability'] == 'None':
        options['ability'] = NO_ABILITY
    if side.get('item'):
        options['item'] = side['item']
    if side.get('status'):
        options['status'] = side['status']
    return {'species': 'Mew', 'options': options, 'raw_stats': side['stats']}


def calc_input(case: dict) -> dict:
    field = {'gameType': 'Doubles' if case['mode'] == 'doubles' else 'Singles'}
    if case['field'].get('weather'):
        field['weather'] = case['field']['weather']
    if case['field'].get('terrain'):
        field['terrain'] = case['field']['terrain']
    return {
        'generation': case['generation'],
        'attacker': calc_side(case['attacker']),
        'defender': calc_side(case['defender']),
        'move': {'name': case['move'], 'options': {'isCrit': False}},
        'field': field,
    }


def main() -> None:
    cases = json.loads(CASES.read_text())
    out = {'schema': 1, 'provenance': None, 'cases': []}
    for case in cases['cases']:
        result = subprocess.run(
            ['node', 'src/damage.mjs'],
            cwd=ORACLE,
            input=json.dumps(calc_input(case)),
            capture_output=True,
            text=True,
            check=True,
        )
        oracle = json.loads(result.stdout)
        out['provenance'] = oracle['provenance']
        damage = oracle['damage']
        # 85..=100 from Gen 3; 217..=255 in Gen 1-2.
        rolls = 39 if case['generation'] <= 2 else 16
        if not (isinstance(damage, list) and len(damage) == rolls and all(isinstance(d, int) for d in damage)):
            raise SystemExit(f"{case['id']}: expected {rolls} single-hit rolls, got {damage!r}")
        out['cases'].append({
            **case,
            'expected': {'damage': damage, 'range': oracle['range']},
            'description': oracle['description'],
            'calc_inputs': oracle['inputs'],
        })
        print(f"{case['id']:40} {oracle['description']}")
    OUT.write_text(json.dumps(out, indent=2) + '\n')
    print(f"wrote {OUT.relative_to(ROOT)} ({len(out['cases'])} cases)")


if __name__ == '__main__':
    main()
