#!/usr/bin/env python3
"""Rewrite the support numbers in README.md and DESIGN.md from tests/docs.rs's failure message.
Usage: tools/m7/sync_support.py <check.log>   (WT=<worktree> to target another checkout)"""
import re
import sys

from _paths import WT

log = open(sys.argv[1]).read()
m = re.search(r"Custom \| (\d+) / (\d+) / (\d+) \| 685 \|.*?Unsupported \| (\d+) \|.*?特性 implemented \| (\d+) \| 310 \|"
              r".*?道具 implemented \| (\d+) \| (\d+) \|", log, re.S)
assert m, 'no support-summary message in the log'
dmg, st, cu, un, ab, it, _ = m.groups()
p = f'{WT}/README.md'
s = open(p).read()
s = re.sub(r"\| 招式 Damage / Status / Custom \| \d+ / \d+ / \d+ \| 685 \|", f"| 招式 Damage / Status / Custom | {dmg} / {st} / {cu} | 685 |", s)
s = re.sub(r"\| 招式 Unsupported \| \d+ \| \|", f"| 招式 Unsupported | {un} | |", s)
s = re.sub(r"\| 特性 implemented \| \d+ \| 310 \|", f"| 特性 implemented | {ab} | 310 |", s)
s = re.sub(r"\| 道具 implemented \| \d+ \| (\d+) \|", f"| 道具 implemented | {it} | \\1 |", s)
open(p, 'w').write(s)
p = f'{WT}/DESIGN.md'
s = open(p).read()
s = re.sub(r"685 个招式中 \d+ Damage、\d+ Status、\d+ Custom、\d+ Unsupported；特性 \d+ / 310，道具 \d+ /",
           f"685 个招式中 {dmg} Damage、{st} Status、{cu} Custom、{un} Unsupported；特性 {ab} / 310，道具 {it} /", s)
open(p, 'w').write(s)
print('synced', dmg, st, cu, un, ab, it)
