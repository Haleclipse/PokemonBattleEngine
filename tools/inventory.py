#!/usr/bin/env python3
"""M7 inventory (ROADMAP §11): everything the snapshots list that the engine does not implement.

    python3 tools/inventory.py [--out target/m7/inventory.md]

Runs `cargo run --example inventory` (one JSON line per generation and unimplemented move,
ability or item, with what blocks it), folds the generations together and groups the entries by
the mechanism family they miss (FAMILIES below: the M7 work queue, in the order of ROADMAP §11).
The snapshots list each generation exhaustively, so the inventory is finite: M7 is done when it
is empty. Exits 1 when an entry belongs to no family, or a family still names something the engine
now implements — the table must stay exactly the remaining work.
"""
import argparse
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KINDS = {'move': 'moves', 'ability': 'abilities', 'item': 'items'}

FAMILIES = {
    'A1': {
        'title': '伤害修正：特性（威力 / 攻防 / 伤害倍率）',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'A2': {
        'title': '伤害修正：道具（属性强化 / 专属道具 / 命中 / 宝石 / 抗性树果）',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'A3': {
        'title': '伤害公式与招式数据（无视防御变化、固定 / 条件威力、多段命中、类型变化）',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'B': {
        'title': '状态与易变状态的免疫、治愈与反应',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'C': {
        'title': 'HP 阈值与消耗型道具（树果、气势披带、换出触发道具）',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'D': {
        'title': '道具的转移、移除与压制',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'E': {
        'title': '场地：屏障、护场与场地状态',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'F': {
        'title': '场地：地形与天气的剩余部分',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'G': {
        'title': '守住变体、反击与同归',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'H': {
        'title': '自身倒下与换出（大爆炸、接棒、治愈之愿……）',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'I': {
        'title': '束缚与无法逃走',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'J': {
        'title': '能力等级与数值交换',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'K': {
        'title': '属性与特性的改写（类型变化、特性交换、破格、识破）',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'L': {
        'title': '招式调用、复制与行动顺序',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'M': {
        'title': '多回合、锁招与请求层限制',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'N': {
        'title': '形态变化与专属形态道具',
        'moves': [],
        'abilities': [],
        'items': [],
    },
    'O': {
        'title': '世代机制：超级进化、Z 招式、极巨化、太晶化',
        'moves': [],
        'abilities': [],
        'items': [],
    },
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, default=ROOT / 'target' / 'm7' / 'inventory.md')
    args = parser.parse_args()
    run = subprocess.run(['cargo', 'run', '-q', '--example', 'inventory'], cwd=ROOT,
                         capture_output=True, text=True, check=True)
    entries = defaultdict(lambda: {'generations': [], 'reasons': set()})
    per_generation = defaultdict(lambda: defaultdict(int))
    for line in run.stdout.splitlines():
        entry = json.loads(line)
        kind = KINDS[entry['kind']]
        found = entries[(kind, entry['name'])]
        found['generations'].append(entry['generation'])
        found['reasons'] |= set(entry['reasons'])
        per_generation[entry['generation']][kind] += 1

    family_of = {}
    for key, family in FAMILIES.items():
        for kind in KINDS.values():
            for name in family[kind]:
                family_of[(kind, name)] = key
    unassigned = sorted(set(entries) - set(family_of))
    stale = sorted(set(family_of) - set(entries))

    out = ['# M7 inventory', '']
    totals = {kind: sum(1 for (k, _) in entries if k == kind) for kind in KINDS.values()}
    out.append(f"{len(entries)} entries: {totals['moves']} moves, {totals['abilities']} abilities, "
               f"{totals['items']} items (each counted once across generations).")
    out += ['', '| generation | moves | abilities | items |', '|---|---|---|---|']
    for generation in sorted(per_generation):
        counts = per_generation[generation]
        out.append(f"| {generation} | {counts['moves']} | {counts['abilities']} | {counts['items']} |")
    for key, family in FAMILIES.items():
        names = [(kind, name) for kind in KINDS.values() for name in family[kind]
                 if (kind, name) in entries]
        out += ['', f"## {key} {family['title']} ({len(names)})", '']
        for kind, name in names:
            found = entries[(kind, name)]
            generations = sorted(found['generations'])
            span = (f'{generations[0]}' if len(generations) == 1
                    else f'{generations[0]}-{generations[-1]}'
                    if generations == list(range(generations[0], generations[-1] + 1))
                    else ','.join(map(str, generations)))
            reasons = ', '.join(sorted(found['reasons']))
            out.append(f"- {kind[:-1] if kind != 'abilities' else 'ability'} **{name}** "
                       f"(Gen {span}): {reasons}")
    if unassigned:
        out += ['', '## Unassigned', ''] + [f'- {kind} {name}' for kind, name in unassigned]
    if stale:
        out += ['', '## Implemented but still listed', ''] + [f'- {kind} {name}' for kind, name in stale]
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text('\n'.join(out) + '\n')
    summary = [f"{key} {len([1 for kind in KINDS.values() for name in family[kind] if (kind, name) in entries])}"
               for key, family in FAMILIES.items()]
    print(f"{len(entries)} entries ({totals['moves']} moves, {totals['abilities']} abilities, "
          f"{totals['items']} items) → {args.out}")
    print('by family: ' + ', '.join(summary))
    if unassigned or stale:
        print(f'{len(unassigned)} unassigned, {len(stale)} implemented but still listed', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
