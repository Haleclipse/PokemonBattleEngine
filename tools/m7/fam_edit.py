#!/usr/bin/env python3
"""Edit the FAMILIES table in tools/inventory.py.
Usage: tools/m7/fam_edit.py remove <name>... | move <family> <name>...   (WT=<worktree> to target another checkout)"""
import importlib.util
import sys

from _paths import WT

p = f'{WT}/tools/inventory.py'
spec = importlib.util.spec_from_file_location('inventory', p)
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)
F = inventory.FAMILIES
op, args = sys.argv[1], sys.argv[2:]
assert op in ('remove', 'move'), __doc__
dest = None
if op == 'move':
    dest, args = args[0], args[1:]
for name in args:
    found = False
    for family in F.values():
        for kind in ('moves', 'abilities', 'items'):
            if name in family[kind]:
                family[kind].remove(name)
                found = kind
    assert found, f'{name} is in no family'
    if dest:
        F[dest][found].append(name)


def render(families):
    lines = []
    for key, family in families.items():
        lines.append(f"    {key!r}: {{\n        'title': {family['title']!r},")
        for kind in ('moves', 'abilities', 'items'):
            names = sorted(family[kind])
            if not names:
                lines.append(f"        {kind!r}: [],")
                continue
            lines.append(f"        {kind!r}: [")
            current = '           '
            for name in names:
                piece = ' ' + repr(name) + ','
                if len(current) + len(piece) > 99:
                    lines.append(current)
                    current = '           '
                current += piece
            lines.append(current)
            lines.append("        ],")
        lines.append("    },")
    return "FAMILIES = {\n" + "\n".join(lines) + "\n}\n"


s = open(p).read()
start = s.index("FAMILIES = {")
end = s.index("\n}\n", start) + 3
open(p, 'w').write(s[:start] + render(F) + s[end:])
