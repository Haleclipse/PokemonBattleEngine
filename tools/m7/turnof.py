#!/usr/bin/env python3
"""Print the turn of a Showdown log in which <who> used <move> (moves, damage, heals, faints …).
Usage: tools/m7/turnof.py <showdown.log.txt | x.showdown.json> <move> <who> [--all]   (--all: every line of the turn)"""
import json
import sys

path, move, who = sys.argv[1:4]
everything = '--all' in sys.argv[4:]
KEEP = ('|turn', '|move', '|-damage', '|-heal', '|-activate', '|-hitcount', '|faint', '|-crit',
        '|-supereffective', '|-resisted', '|-immune', '|-miss', '|cant', '|switch', '|-status', '|-fail')
turns, current = [], []
text = json.load(open(path))['debug_log'] if path.endswith('.json') else open(path).read()
for line in text.split('\n'):
    if line.startswith('|turn|'):
        turns.append(current)
        current = [line]
    else:
        current.append(line)
turns.append(current)
for turn in turns:
    if any(l.startswith('|move|') and f'|{move}|' in l and who in l for l in turn):
        print('\n'.join(l for l in turn if everything or l.startswith(KEEP)))
        break
else:
    sys.exit(f'no turn where {who} used {move}')
