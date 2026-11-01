#!/usr/bin/env python3
"""Find a seed under which a scenario's script runs to the end and its Showdown log matches every pattern.
Usage: tools/m7/seek.py <scenario.json> <regex>... [--max N]   (prints a,b,c,d for tools/m7/rec.sh; exit 1: none found)"""
import subprocess
import sys

from _paths import WT

template, rest = sys.argv[1], sys.argv[2:]
out = subprocess.run(['node', 'src/seek.mjs', *rest], stdin=open(template), cwd=f'{WT}/tools/oracle-node',
                     capture_output=True, text=True)
if out.returncode:
    sys.exit(out.stderr.strip() or 'no seed found')
print(out.stdout.strip())
