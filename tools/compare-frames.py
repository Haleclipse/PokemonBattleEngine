#!/usr/bin/env python3
"""Compare normalized semantic frame files and optionally write a failure bundle."""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path


def first_diff(a, b, path='$'):
    if type(a) is not type(b):
        return path, a, b
    if isinstance(a, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a:
                return f'{path}.{key}', '<missing>', b[key]
            if key not in b:
                return f'{path}.{key}', a[key], '<missing>'
            diff = first_diff(a[key], b[key], f'{path}.{key}')
            if diff:
                return diff
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f'{path}.length', len(a), len(b)
        for i, (left, right) in enumerate(zip(a, b)):
            diff = first_diff(left, right, f'{path}[{i}]')
            if diff:
                return diff
        return None
    if a != b:
        return path, a, b
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('rust')
    parser.add_argument('oracle')
    parser.add_argument('--case-id', default='manual-diff')
    parser.add_argument('--bundle-dir')
    parser.add_argument('--scenario')
    parser.add_argument('--rust-trace')
    parser.add_argument('--oracle-log')
    args = parser.parse_args()

    rust_path = Path(args.rust)
    oracle_path = Path(args.oracle)
    rust = json.loads(rust_path.read_text())
    oracle = json.loads(oracle_path.read_text())

    rust_frames = rust.get('frames', rust)
    oracle_frames = oracle.get('frames', oracle)
    count = max(len(rust_frames), len(oracle_frames))
    failure = None
    for i in range(count):
        if i >= len(rust_frames) or i >= len(oracle_frames):
            failure = {'frame': i, 'path': '$.frames.length', 'rust': len(rust_frames), 'oracle': len(oracle_frames)}
            break
        diff = first_diff(rust_frames[i], oracle_frames[i])
        if diff:
            path, left, right = diff
            failure = {'frame': i, 'path': path, 'rust': left, 'oracle': right}
            break

    if not failure:
        print('semantic frames match')
        return 0

    print(f"first divergence: frame={failure['frame']} path={failure['path']}")
    print(f"  rust:   {failure['rust']!r}")
    print(f"  oracle: {failure['oracle']!r}")

    if args.bundle_dir:
        bundle = Path(args.bundle_dir) / args.case_id
        bundle.mkdir(parents=True, exist_ok=True)
        shutil.copy2(rust_path, bundle / 'rust.frames.json')
        shutil.copy2(oracle_path, bundle / 'oracle.frames.json')
        (bundle / 'diff.json').write_text(json.dumps(failure, indent=2) + '\n')
        for src, dst in [
            (args.scenario, 'scenario.json'),
            (args.rust_trace, 'rust.trace.txt'),
            (args.oracle_log, 'showdown.log.txt'),
        ]:
            if src:
                shutil.copy2(src, bundle / dst)
        (bundle / 'reproduce.txt').write_text(
            f'python3 tools/compare-frames.py {rust_path} {oracle_path} --case-id {args.case_id}\n'
        )
        print(f'failure bundle: {bundle}')
    return 1


if __name__ == '__main__':
    raise SystemExit(main())
