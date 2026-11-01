#!/usr/bin/env python3
"""Mutation check: apply each mutation of a batch's list, run its target, report CAUGHT / MISSED /
BUILD-ERROR, then restore the sources.
Usage: tools/m7/mutate.py <mutations.py> [name ...]   (WT=<worktree> to mutate another checkout)
The list file defines MUTATIONS = [(name, [(path, old, new[, count]), ...], target), ...]: every `old`
must occur exactly `count` times (default 1) in its file and is replaced everywhere; `target` is a
scenario filter (SCENARIO=… showdown_oracle) or a (test binary, test filter) pair for an L1 test.
A mutation whose `old` does not occur the expected number of times is reported as ANCHOR-MISSING
(nothing is changed) and the run goes on; fix the anchor (they are matched against the formatted
source: run `cargo fmt` first) and rerun it by name."""
import os
import pathlib
import runpy
import subprocess
import sys

from _paths import WT

mutations = runpy.run_path(sys.argv[1])['MUTATIONS']
only = sys.argv[2:]
root = pathlib.Path(WT)
for name, edits, target in mutations:
    if only and name not in only:
        continue
    missing = [
        (rel, old[:60], count)
        for rel, old, new, *expected in edits
        if (count := (root / rel).read_text().count(old)) != (expected[0] if expected else 1)
    ]
    if missing:
        print('ANCHOR-MISSING', name, missing, flush=True)
        continue
    originals = {}
    try:
        for rel, old, new, *expected in edits:
            path = root / rel
            text = path.read_text()
            originals.setdefault(rel, text)
            path.write_text(text.replace(old, new))
        if isinstance(target, tuple):
            command = ['cargo', 'test', '-q', '--release', '--test', target[0], target[1]]
            env = os.environ
        else:
            command = ['cargo', 'test', '-q', '--release', '--test', 'showdown_oracle']
            env = {**os.environ, 'SCENARIO': target}
        out = subprocess.run(command, cwd=root, capture_output=True, text=True, env=env)
        verdict = 'BUILD-ERROR' if 'error[' in out.stderr else 'CAUGHT' if out.returncode else 'MISSED'
        print(verdict, name, flush=True)
    finally:
        # write_text, not a move back: the restored file gets a new mtime, so cargo rebuilds.
        for rel, text in originals.items():
            (root / rel).write_text(text)
