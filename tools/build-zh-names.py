#!/usr/bin/env python3
"""Build data/dex/names-zh.json: the official Chinese names of moves, abilities, items, species,
types, stats, natures and damage classes, from PokéAPI's CSV tables (tools/pokeapi, the
pinned commit data/dex/official-index.json records).

Simplified Chinese (language 12, zh-Hans); Traditional (4, zh-Hant) where PokéAPI has no
Simplified name. Keys are the English names normalised with Showdown's `toID` (lowercase
alphanumerics), so a Showdown id looks a name up directly. The documents (DESIGN.md, issues.md,
ROADMAP.md …) write Pokémon terms with these names, never an ad-hoc translation;
tools/zh-terms.py checks them.

Run tools/fetch-pokeapi.sh first. Regenerate only together with a provenance bump.
"""
from __future__ import annotations

import csv
import json
import re
import subprocess
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
POKEAPI = ROOT / 'tools' / 'pokeapi'
CSV = POKEAPI / 'data' / 'v2' / 'csv'
OUT = ROOT / 'data' / 'dex' / 'names-zh.json'
ENGLISH, SIMPLIFIED, TRADITIONAL = '9', '12', '4'

# kind -> (names table, its key column, name column)
TABLES = {
    'move': ('move_names.csv', 'move_id', 'name'),
    'ability': ('ability_names.csv', 'ability_id', 'name'),
    'item': ('item_names.csv', 'item_id', 'name'),
    'species': ('pokemon_species_names.csv', 'pokemon_species_id', 'name'),
    'type': ('type_names.csv', 'type_id', 'name'),
    'stat': ('stat_names.csv', 'stat_id', 'name'),
    'nature': ('nature_names.csv', 'nature_id', 'name'),
    'damage_class': ('move_damage_class_prose.csv', 'move_damage_class_id', 'name'),
}


def to_id(text: str) -> str:
    return re.sub(r'[^a-z0-9]', '', text.lower())


def main() -> None:
    commit = subprocess.run(['git', '-C', str(POKEAPI), 'rev-parse', 'HEAD'], capture_output=True,
                            text=True, check=True).stdout.strip()
    out = {
        'schema': 1,
        'provenance': {
            'source': 'https://github.com/PokeAPI/pokeapi',
            'commit': commit,
            'files': [table for table, _, _ in TABLES.values()],
            'languages': 'zh-Hans (local_language_id 12); zh-Hant (4) where zh-Hans is missing',
        },
    }
    for kind, (table, key, column) in TABLES.items():
        names = defaultdict(dict)
        with open(CSV / table, encoding='utf-8') as f:
            for row in csv.DictReader(f):
                if row[column]:
                    names[row[key]][row['local_language_id']] = row[column]
        entries = {}
        for by_language in sorted(names.values(), key=lambda n: n.get(ENGLISH, '')):
            english = by_language.get(ENGLISH)
            chinese = by_language.get(SIMPLIFIED) or by_language.get(TRADITIONAL)
            if english and chinese:
                # The first entry keeps an English name (PokéAPI lists some twice, e.g. two
                # Roseli Berry rows); a later duplicate with the same name adds nothing.
                entries.setdefault(to_id(english), {'en': english, 'zh': chinese})
        out[kind] = dict(sorted(entries.items()))
    OUT.write_text(json.dumps(out, ensure_ascii=False, indent=1) + '\n')
    print(OUT.relative_to(ROOT), {kind: len(out[kind]) for kind in TABLES})


if __name__ == '__main__':
    main()
