#!/usr/bin/env python3
"""Build data/dex/official-index.json from PokéAPI's CSV tables (tools/pokeapi, pinned commit).

Engine ids for moves, abilities and items are PokéAPI's primary keys. For moves and abilities
those are also the in-game index numbers; for items the in-game index changes per generation
(item_game_indices.csv: Choice Band is 186 in Gen 3 and 220 from Gen 4), so only the primary key
is stable enough to be an engine id. Identifiers are normalised with Showdown's `toID`
(lowercase alphanumerics) so both data sources key the same way.

Run tools/fetch-pokeapi.sh first. Regenerate only together with a provenance bump.
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
OUT = ROOT / 'data' / 'dex' / 'official-index.json'
FILES = ['abilities.csv', 'moves.csv', 'items.csv']

# Showdown spellings that differ from PokéAPI's identifier for the same entry. The alias key is
# written into the index with the same id so every consumer (exporter, tests) can look up by
# Showdown id without its own alias table.
ALIASES = {'moves': {'visegrip': 'vicegrip'}, 'abilities': {}, 'items': {'prettyfeather': 'prettywing'}}


def toid(name: str) -> str:
    return ''.join(c for c in name.lower() if c.isalnum())


def rows(name: str) -> list[dict[str, str]]:
    with open(CSV / name, newline='', encoding='utf-8') as handle:
        return list(csv.DictReader(handle))


def table(name: str, keep, collisions: dict[str, list[int]]) -> dict[str, int]:
    """identifier → id. PokéAPI has a few duplicated identifiers (items.csv lists roseli-berry
    twice); the lowest id wins and the duplicate is recorded so the choice is visible."""
    out: dict[str, int] = {}
    for row in rows(name):
        if not keep(row):
            continue
        key = toid(row['identifier'])
        ident = int(row['id'])
        if key in out:
            collisions.setdefault(f'{name}:{key}', [out[key]]).append(ident)
            out[key] = min(out[key], ident)
            continue
        out[key] = ident
    return dict(sorted(out.items()))


def main() -> None:
    if not CSV.is_dir():
        raise SystemExit('tools/pokeapi is missing; run tools/fetch-pokeapi.sh first')
    commit = subprocess.run(
        ['git', '-C', str(POKEAPI), 'rev-parse', 'HEAD'], capture_output=True, text=True, check=True
    ).stdout.strip()
    collisions: dict[str, list[int]] = {}
    index = {
        'schema': 1,
        'provenance': {
            'source': 'https://github.com/PokeAPI/pokeapi',
            'commit': commit,
            'files': [f'data/v2/csv/{f}' for f in FILES],
            'built': datetime.date.today().isoformat(),
            'note': 'PokéAPI primary keys. Main-series abilities and moves only (ids < 10000); all items.',
        },
        # is_main_series excludes Pokémon GO / non-main-series abilities; ids >= 10000 are
        # non-main-series moves (Shadow / Z / Max variants).
        'abilities': table('abilities.csv', lambda r: r['is_main_series'] == '1' and int(r['id']) < 10000, collisions),
        'moves': table('moves.csv', lambda r: int(r['id']) < 10000, collisions),
        'items': table('items.csv', lambda r: True, collisions),
    }
    index['provenance']['identifier_collisions'] = collisions
    index['provenance']['aliases'] = ALIASES
    for kind, aliases in ALIASES.items():
        for showdown_id, pokeapi_id in aliases.items():
            if pokeapi_id not in index[kind]:
                raise SystemExit(f'alias target missing: {kind}:{pokeapi_id}')
            index[kind][showdown_id] = index[kind][pokeapi_id]
        index[kind] = dict(sorted(index[kind].items()))
    OUT.write_text(json.dumps(index, indent=1) + '\n')
    print(
        f'wrote {OUT.relative_to(ROOT)} @ {commit[:12]}: '
        f'abilities={len(index["abilities"])} moves={len(index["moves"])} items={len(index["items"])}'
    )


if __name__ == '__main__':
    main()
