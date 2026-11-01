#!/usr/bin/env python3
"""Cross-check the @pkmn/data snapshots against PokéAPI's CSV tables (tools/pokeapi, pinned).

Writes data/dex/crosscheck-pokeapi.json: PokéAPI's view of species base stats / types and of
move numbers, keyed by Showdown id so the Rust test can compare it with data/dex/gen9.json.
This is the advisory drift check from TESTING.md: the engine's data source is Showdown, and a
difference here is a disagreement to classify, not automatically an error on either side.
"""
from __future__ import annotations

import csv
import datetime
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
POKEAPI = ROOT / 'tools' / 'pokeapi'
CSV = POKEAPI / 'data' / 'v2' / 'csv'
OUT = ROOT / 'data' / 'dex' / 'crosscheck-pokeapi.json'
STATS = {'1': 'hp', '2': 'atk', '3': 'def', '4': 'spa', '5': 'spd', '6': 'spe'}
CATEGORY = {'1': 'Status', '2': 'Physical', '3': 'Special'}


def toid(name: str) -> str:
    return ''.join(c for c in name.lower() if c.isalnum())


def rows(name: str):
    with open(CSV / name, newline='', encoding='utf-8') as handle:
        yield from csv.DictReader(handle)


def main() -> None:
    types = {r['id']: r['identifier'].capitalize() for r in rows('types.csv')}
    species_ident = {r['id']: r['identifier'] for r in rows('pokemon_species.csv')}
    stats: dict[str, dict[str, int]] = {}
    for r in rows('pokemon_stats.csv'):
        stats.setdefault(r['pokemon_id'], {})[STATS[r['stat_id']]] = int(r['base_stat'])
    ptypes: dict[str, list[str]] = {}
    for r in rows('pokemon_types.csv'):
        ptypes.setdefault(r['pokemon_id'], []).append((int(r['slot']), types[r['type_id']]))

    species = {}
    for r in rows('pokemon.csv'):
        if int(r['id']) > 10000 and r['is_default'] == '1':
            continue
        # PokéAPI names the default forme "deoxys-normal"; Showdown calls it "deoxys".
        key = toid(species_ident[r['species_id']]) if r['is_default'] == '1' else toid(r['identifier'])
        species[key] = {
            'dex': int(r['species_id']),
            'types': [t for _, t in sorted(ptypes.get(r['id'], []))],
            'base_stats': stats.get(r['id'], {}),
        }

    aliases = json.loads((ROOT / 'data' / 'dex' / 'official-index.json').read_text())['provenance']['aliases']
    reverse = {v: k for k, v in aliases['moves'].items()}
    moves = {}
    for r in rows('moves.csv'):
        if int(r['id']) >= 10000:
            continue
        key = toid(r['identifier'])
        key = reverse.get(key, key)
        # Encoding differences, normalised to Showdown's conventions: PokéAPI writes accuracy 0
        # and power 1 for "always hits" / "variable power"; Showdown writes true (null) and 0.
        accuracy = int(r['accuracy']) if r['accuracy'] else 0
        power = int(r['power'] or 0)
        moves[key] = {
            'type': types[r['type_id']],
            'category': CATEGORY.get(r['damage_class_id'], 'Status'),
            'bp': 0 if power == 1 else power,
            'acc': None if accuracy == 0 else accuracy,
            'pp': int(r['pp'] or 0),
            'pri': int(r['priority'] or 0),
        }

    commit = subprocess.run(['git', '-C', str(POKEAPI), 'rev-parse', 'HEAD'], capture_output=True, text=True, check=True).stdout.strip()
    out = {
        'schema': 1,
        'provenance': {'source': 'https://github.com/PokeAPI/pokeapi', 'commit': commit, 'built': datetime.date.today().isoformat(),
                       'note': 'latest-generation values; historical values live in move_changelog.csv'},
        'species': dict(sorted(species.items())),
        'moves': dict(sorted(moves.items())),
    }
    OUT.write_text(json.dumps(out, indent=1) + '\n')
    print(f'wrote {OUT.relative_to(ROOT)} species={len(species)} moves={len(moves)}')

    # immediate report against the Gen 9 snapshot
    snap = json.loads((ROOT / 'data' / 'dex' / 'gen9.json').read_text())
    unmatched, stat_diff, type_diff = [], [], []
    for s in snap['species']:
        p = species.get(s['id'])
        if p is None:
            unmatched.append(s['id']); continue
        if p['base_stats'] != s['base_stats']:
            stat_diff.append((s['id'], s['base_stats'], p['base_stats']))
        if p['types'] != s['types']:
            type_diff.append((s['id'], s['types'], p['types']))
    print(f'species: {len(snap["species"])} snapshot, unmatched={len(unmatched)} {unmatched[:15]}')
    print(f'  base_stats differ={len(stat_diff)} {stat_diff[:4]}')
    print(f'  types differ={len(type_diff)} {type_diff[:6]}')
    munmatched, mdiff = [], []
    for m in snap['moves']:
        p = moves.get(m['id'])
        if p is None:
            munmatched.append(m['id']); continue
        mine = {'type': m['type'], 'category': m['category'], 'bp': m['bp'], 'acc': m['acc'], 'pp': m['pp'], 'pri': m['pri']}
        if mine != p:
            mdiff.append((m['id'], {k: (mine[k], p[k]) for k in mine if mine[k] != p[k]}))
    print(f'moves: {len(snap["moves"])} snapshot, unmatched={len(munmatched)} {munmatched[:12]}')
    print(f'  differ={len(mdiff)}')
    for d in mdiff[:25]:
        print('   ', d)


if __name__ == '__main__':
    main()
