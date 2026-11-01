#!/usr/bin/env python3
"""Instruction-count benchmarks (DESIGN.md §48.0 item 5): the production criteria's regression
threshold, on counts that do not move with the machine's load.

    python3 tools/bench_instructions.py [--check BASELINE] [--write BASELINE] [--repeat N]

Builds `benches/instructions.rs` and runs each format's workloads under Valgrind's cachegrind
(`--cache-sim=no`): a workload's count is its run's instructions less its setup's. Results go to
`target/bench-instructions/results.json`, with the rustc and Valgrind versions they were taken
with.

`--check BASELINE` compares them with a committed baseline: a workload more than `THRESHOLD`
above its baseline fails; one more than `THRESHOLD` below is reported, for the baseline to be
updated. Counts from another rustc or Valgrind than the baseline's are reported, never failed:
a new compiler moves them by itself. No baseline yet: the counts are reported. `--write
BASELINE` writes the results as the baseline. `--repeat N` measures N times and reports the
largest spread between the runs (the measurement's own noise: under 0.001% in CI).

Linux only (Valgrind); CI runs it (`.github/workflows/ci.yml`) and uploads the results as the
`instruction-counts` artifact. To update the baseline, commit that artifact's `results.json` as
`benches/instructions.json`: on a deliberate change of cost, or when the stable toolchain moves
(the run then warns that the baseline was taken with another rustc).
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FORMATS = ['gen9ou', 'gen9vgc2025regi', 'gen4ou', 'gen1ou']
WORKLOADS = ['battle', 'rollout', 'snapshot']
THRESHOLD = 0.03
OUT = ROOT / 'target' / 'bench-instructions' / 'results.json'


def build():
    output = subprocess.run(
        ['cargo', 'build', '--release', '--bench', 'instructions', '--message-format=json'],
        cwd=ROOT, check=True, capture_output=True, text=True).stdout
    for line in output.splitlines():
        message = json.loads(line)
        if (message.get('reason') == 'compiler-artifact' and message['target']['name'] == 'instructions'
                and message.get('executable')):
            return message['executable']
    sys.exit('no instructions bench executable')


def instructions(binary, format_id, workload):
    result = subprocess.run(
        ['valgrind', '--tool=cachegrind', '--cache-sim=no', '--cachegrind-out-file=/dev/null',
         binary, format_id, workload],
        cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f'{format_id} {workload}: exit {result.returncode}\n{result.stderr[-2000:]}')
    match = re.search(r'I\s+refs:\s+([\d,]+)', result.stderr)
    if not match:
        sys.exit(f'{format_id} {workload}: no instruction count\n{result.stderr[-2000:]}')
    return int(match.group(1).replace(',', ''))


def measure(binary):
    counts = {}
    for format_id in FORMATS:
        setup = instructions(binary, format_id, 'setup')
        for workload in WORKLOADS:
            counts[f'{format_id}/{workload}'] = instructions(binary, format_id, workload) - setup
            print(f'{format_id}/{workload}: {counts[f"{format_id}/{workload}"]:,}', flush=True)
    return counts


def version(command):
    return subprocess.run(command, capture_output=True, text=True).stdout.strip()


def warn(message):
    """A line that GitHub Actions also shows as a warning on the run's page."""
    print(f'::warning::{message}' if os.environ.get('GITHUB_ACTIONS') else message)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', type=Path)
    parser.add_argument('--write', type=Path)
    parser.add_argument('--repeat', type=int, default=1)
    args = parser.parse_args()

    binary = build()
    runs = [measure(binary) for _ in range(args.repeat)]
    counts = runs[0]
    if len(runs) > 1:
        spread = max(abs(run[key] - counts[key]) / counts[key] for run in runs[1:] for key in counts)
        print(f'largest spread between runs: {spread:.4%}')
    results = {
        'rustc': version(['rustc', '-V']),
        'valgrind': version(['valgrind', '--version']),
        'threshold': THRESHOLD,
        'counts': counts,
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(results, indent=1) + '\n')
    if args.write:
        args.write.write_text(json.dumps(results, indent=1) + '\n')
        print(f'baseline written: {args.write}')
    if not args.check:
        return 0
    if not args.check.exists():
        print(f'no baseline at {args.check}: nothing to compare (results in {OUT})')
        return 0
    baseline = json.loads(args.check.read_text())
    same_tools = (baseline['rustc'], baseline['valgrind']) == (results['rustc'], results['valgrind'])
    if not same_tools:
        warn(f"baseline taken with {baseline['rustc']} / {baseline['valgrind']}, now "
             f"{results['rustc']} / {results['valgrind']}: compared, not failed; update the baseline")
    failed = False
    for key, count in counts.items():
        base = baseline['counts'].get(key)
        if base is None:
            print(f'{key}: {count:,} (not in the baseline)')
            continue
        change = count / base - 1
        mark = ''
        if change > THRESHOLD:
            mark = '  REGRESSION' if same_tools else '  (above the threshold)'
            failed = failed or same_tools
        elif change < -THRESHOLD:
            mark = '  (below the threshold: update the baseline)'
        line = f'{key}: {count:,} vs {base:,} ({change:+.2%}){mark}'
        if mark:
            warn(line)
        else:
            print(line)
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
