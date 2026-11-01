#!/usr/bin/env python3
"""Rewrite campaign recordings (`*.showdown.json`) as compact JSON, in place.

    python3 tools/compact-recordings.py <dir> [<dir> ...] [--jobs N]

The oracle prints its recordings indented; the same JSON without the indentation takes about a
third of the space, and every reader parses it the same way. The script works one file at a
time:
- it parses the file and writes the compact form to a temporary file beside it;
- it reads that back and checks it equals the original object;
- only then does it move the temporary file over the original.

A file that fails at any step keeps its original. Files already compact are skipped, so the
script can be run again. Symlinks (`nightly-runs/latest`) are not followed.
"""
import json
import os
import sys
from multiprocessing import Pool


def compact(path):
    tmp = path + '.compact-tmp'
    try:
        with open(path, 'rb') as f:
            raw = f.read()
        if not raw.startswith(b'{\n'):
            return 'skip', len(raw), len(raw), path
        original = json.loads(raw)
        out = (json.dumps(original, separators=(',', ':'), ensure_ascii=False) + '\n').encode()
        if json.loads(out) != original:
            return 'mismatch', len(raw), len(raw), path
        with open(tmp, 'wb') as f:
            f.write(out)
        os.replace(tmp, path)
        return 'ok', len(raw), len(out), path
    except Exception as error:  # a full disk, an unreadable file: keep the original
        if os.path.exists(tmp):
            os.remove(tmp)
        return f'error: {error}', 0, 0, path


def recordings(roots):
    for root in roots:
        for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
            dirnames[:] = [d for d in dirnames if not os.path.islink(os.path.join(dirpath, d))]
            for name in filenames:
                if name.endswith('.showdown.json'):
                    yield os.path.join(dirpath, name)


def main():
    args = sys.argv[1:]
    jobs = 8
    if '--jobs' in args:
        at = args.index('--jobs')
        jobs = int(args[at + 1])
        del args[at:at + 2]
    counts, before, after = {}, 0, 0
    with Pool(jobs) as pool:
        for status, old, new, path in pool.imap_unordered(compact, recordings(args), chunksize=64):
            key = status if not status.startswith('error') else 'error'
            counts[key] = counts.get(key, 0) + 1
            before += old
            after += new
            if key in ('mismatch', 'error'):
                print(status, path, flush=True)
    print(counts, f'{before / 2**30:.1f} GiB -> {after / 2**30:.1f} GiB')


if __name__ == '__main__':
    main()
