#!/usr/bin/env python3
"""Rebuild the campaigns' coverage file (`target/m8/coverage.json`) from the recorded corpus.

    python3 tools/rebuild-coverage.py [--root target/m8] [--out target/m8/coverage.json]

`tools/campaign.py` keeps running counts in that file and samples the least-covered content
from them. This script counts again from every campaign `campaign.py` generated:
- `gen*-*`, `nightly-*/gen*-*` and `nightly-runs/*/gen*-*` are counted;
- `ab/` holds A/B copies and is skipped.

Each campaign is counted the way `campaign.py` counts it:
- each team member's moves, its ability (Gen 3+) and its item (`team`);
- every recording, through `observe`, with the generation's move tags (`content_pool`).

Battles whose generation failed left no recording, and a recording an interrupted campaign left
half-written is skipped, so they are not counted: the rebuilt counts are close to the original,
not identical. The file is written to a temporary file and then moved
into place.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('campaign', ROOT / 'tools' / 'campaign.py')
campaign = importlib.util.module_from_spec(spec)
spec.loader.exec_module(campaign)


def campaign_dirs(root):
    patterns = ['gen*-*', 'nightly-*/gen*-*', 'nightly-runs/*/gen*-*']
    seen = set()
    for pattern in patterns:
        for directory in sorted(root.glob(pattern)):
            if directory.is_dir() and not directory.is_symlink() and directory.resolve() not in seen:
                seen.add(directory.resolve())
                yield directory


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, default=ROOT / 'target' / 'm8')
    parser.add_argument('--out', type=Path, default=ROOT / 'target' / 'm8' / 'coverage.json')
    args = parser.parse_args()
    data, tags_by_gen = {}, {}
    battles = unreadable = 0
    for directory in campaign_dirs(args.root):
        if 'nightly-runs/latest' in str(directory):
            continue
        for scenario_path in sorted(directory.glob('m8-*.json')):
            if scenario_path.name.endswith('.showdown.json'):
                continue
            recording_path = scenario_path.with_name(scenario_path.name.replace('.json', '.showdown.json'))
            if not recording_path.exists():
                continue
            try:
                scenario = json.loads(scenario_path.read_text())
                recording = json.loads(recording_path.read_text())
            except ValueError:
                # A battle an interrupted campaign left half-written (an empty recording).
                unreadable += 1
                continue
            generation = scenario['battle']['generation']
            if generation not in tags_by_gen:
                tags_by_gen[generation] = campaign.content_pool(generation)['move_tags']
            key = f'gen{generation}'
            counts = data.setdefault(key, {'moves': {}, 'abilities': {}, 'items': {},
                                           'tags': {}, 'pairs': {}, 'triples': {}})
            for team in scenario['teams'].values():
                for member in team:
                    for kind, values in (('abilities', [member.get('ability')] if generation >= 3 else []),
                                         ('items', [member.get('item')]),
                                         ('moves', member.get('moves', []))):
                        for name in values:
                            if name:
                                counts[kind][name] = counts[kind].get(name, 0) + 1
            holder = type('Coverage', (), {'counts': counts})
            campaign.observe(holder, recording, tags_by_gen[generation])
            battles += 1
    tmp = args.out.with_name(args.out.name + '.tmp')
    tmp.write_text(json.dumps(data, indent=1, sort_keys=True) + '\n')
    os.replace(tmp, args.out)
    print(battles, 'battles,', unreadable, 'unreadable;',
          {key: sum(c['moves'].values()) for key, c in sorted(data.items())})


if __name__ == '__main__':
    main()
