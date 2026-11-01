#!/usr/bin/env python3
"""M8 nightly (ROADMAP §12 DoD, TESTING.md §6.5).

    python3 tools/nightly.py [--minutes 60] [--battles 20] [--out DIR] [--minimize-share 0.3]
                             [--seed-base N] [--fail-on-new] [--stop-on-new] [--resume]

Runs differential campaigns (`tools/campaign.py`) for a fixed time, with fresh seeds. The
rotation alternates two kinds of slot:
- custom games, every generation in singles and Gen 3-9 in doubles;
- real formats, the 34 targets in `data/formats/formats.json` (M9.6, DESIGN.md §45.8).

The next seed per rotation slot is kept in `target/m8/nightly-state.json`, so consecutive nights
never replay a battle. Then
groups the divergences by signature, reports the ones `tests/fixtures/quarantine.json` knows,
and minimizes (`tools/minimize.py`) the first battle of every new signature into its failure
bundle, with what is left of the budget. The night ends with:

- `nightly.json` / `nightly.md`: pass metrics per campaign and overall;
- the known (quarantined) disagreements;
- one self-contained failure bundle per new signature, holding the minimized battle;
- the mechanic coverage so far (`target/m8/coverage.json`: move tags, event tags, pairs,
  event-tag triples).

Locally it runs through `scripts/nightly.sh` (one directory per run under
`target/m8/nightly-runs/`); `.github/workflows/nightly.yml` runs it in CI.

`--seed-base N` starts every rotation slot at seed N at least (CI passes one derived from the
run number, so a night never replays an earlier one even without the state file);
`--fail-on-new` exits 1 when a new signature was found, after writing everything.
`--stop-on-new` runs no further campaign once one has a new signature: that night cannot be a
clean one, and the next night's fresh seeds reach the campaigns it leaves (a closing run of
clean nights, DESIGN.md R-M10-70). Its battles are still triaged, minimized and reported.
"""
import argparse
import importlib.util
import itertools
import json
import os
import re
import subprocess
import sys
import time
from collections import defaultdict
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location('campaign', ROOT / 'tools' / 'campaign.py')
_campaign = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_campaign)
signature = _campaign.signature

CUSTOM_SLOTS = [('custom', gen, 'singles') for gen in range(1, 10)] + \
    [('custom', gen, 'doubles') for gen in range(3, 10)]
FORMAT_SLOTS = [('format', f['id']) for f in
                json.loads((ROOT / 'data' / 'formats' / 'formats.json').read_text())['formats']]
# Custom games and real formats take turns, so that every night plays both.
ROTATION = [slot for pair in itertools.zip_longest(CUSTOM_SLOTS, FORMAT_SLOTS) for slot in pair if slot]


def load(path, default):
    return json.loads(path.read_text()) if path.exists() else default


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--minutes', type=float, default=60)
    parser.add_argument('--battles', type=int, default=20)
    parser.add_argument('--out', type=Path)
    parser.add_argument('--state', type=Path, default=ROOT / 'target' / 'm8' / 'nightly-state.json')
    parser.add_argument('--minimize-share', type=float, default=0.3,
                        help='part of the budget kept for minimizing new signatures')
    parser.add_argument('--seed-base', type=int, default=100)
    parser.add_argument('--fail-on-new', action='store_true')
    parser.add_argument('--stop-on-new', action='store_true',
                        help='run no further campaign once one has a new signature')
    parser.add_argument('--coverage', type=Path, default=ROOT / 'target' / 'm8' / 'coverage.json')
    parser.add_argument('--resume', action='store_true',
                        help='run no campaign: triage, minimize and report the ones already in --out')
    args = parser.parse_args()
    out = (args.out or ROOT / 'target' / 'm8' / f'nightly-{datetime.now():%Y%m%d-%H%M}').resolve()
    out.mkdir(parents=True, exist_ok=True)
    start = time.monotonic()
    budget = args.minutes * 60
    campaign_budget = budget * (1 - args.minimize_share)

    state = load(args.state, {'next': 0, 'seeds': {}})
    quarantine = load(ROOT / 'tests' / 'fixtures' / 'quarantine.json', {'signatures': []})['signatures']

    def labels(campaign):
        """The signature of every battle of `campaign` that did not pass, with its id."""
        for entry in campaign.get('results', []):
            if entry['status'] == 'pass':
                continue
            text = entry.get('first_divergence') or entry.get('message') or ''
            yield (signature(text) if entry['status'] == 'diverged' else 'panic: ' + signature(text),
                   entry['id'])

    def quarantined(sig):
        return any(re.search(q['pattern'], sig) for q in quarantine)

    campaigns = []
    stopped = None
    if args.resume:
        campaign_budget = 0
        for directory in sorted(p for p in out.iterdir() if (p / 'results.json').exists()):
            results = load(directory / 'results.json', None)
            campaigns.append({'campaign': directory.name, 'battles': results['battles'],
                              'passed': results['passed'], 'diverged': results['diverged'],
                              'panicked': results['panicked'], 'results': results['results']})
    while time.monotonic() - start < campaign_budget:
        slot = ROTATION[state['next'] % len(ROTATION)]
        if slot[0] == 'format':
            key = slot[1]
            kind = ['--format', key]
        else:
            _, gen, mode = slot
            key = f'gen{gen}-{mode}'
            kind = ['--gen', str(gen), '--mode', mode]
        # Seeds 1-99 belong to the hand-run campaigns; nightly ones start at 100.
        seed = max(state['seeds'].get(key, 99) + 1, args.seed_base)
        state['seeds'][key] = seed
        state['next'] += 1
        args.state.parent.mkdir(parents=True, exist_ok=True)
        tmp = args.state.with_name(args.state.name + '.tmp')
        tmp.write_text(json.dumps(state, indent=1) + '\n')
        os.replace(tmp, args.state)
        directory = out / f'{key}-s{seed}'
        ran = subprocess.run([sys.executable, str(ROOT / 'tools' / 'campaign.py'), *kind,
                              '--seed', str(seed), '--battles', str(args.battles),
                              '--out', str(directory), '--coverage', str(args.coverage)],
                             cwd=ROOT, capture_output=True, text=True)
        results = load(directory / 'results.json', None)
        if ran.returncode != 0 or results is None:
            campaigns.append({'campaign': directory.name, 'error': (ran.stderr or ran.stdout)[-500:]})
            # Campaigns that cannot even start (a broken coverage file, a full disk) fail at
            # once, over and over: three in a row stop the night instead of spinning out its
            # budget on empty directories.
            if len(campaigns) >= 3 and all('error' in c for c in campaigns[-3:]):
                print('three campaigns in a row failed to run: stopping', flush=True)
                print(campaigns[-1]['error'], flush=True)
                break
            continue
        campaigns.append({'campaign': directory.name, 'battles': results['battles'],
                          'passed': results['passed'], 'diverged': results['diverged'],
                          'panicked': results['panicked'], 'results': results['results']})
        print(f'{directory.name}: {results["passed"]} / {results["battles"]}', flush=True)
        if args.stop_on_new and any(not quarantined(sig) for sig, _ in labels(campaigns[-1])):
            stopped = round((time.monotonic() - start) / 60, 1)
            print('a new signature: stopping (--stop-on-new)', flush=True)
            break

    groups = defaultdict(list)
    for campaign in campaigns:
        for label, battle in labels(campaign):
            groups[label].append((campaign['campaign'], battle))
    known = {sig: next(q for q in quarantine if re.search(q['pattern'], sig))
             for sig in groups if quarantined(sig)}

    # New signatures, most frequent first: minimize one battle each into its failure bundle.
    minimized = {}
    for sig, battles in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        if sig in known or time.monotonic() - start >= budget:
            continue
        campaign, battle = battles[0]
        bundle = out / campaign / 'failures' / battle
        pair = bundle / f'{battle}-min.json'
        if args.resume and pair.exists():
            made = subprocess.CompletedProcess([], 0, '', '')
        else:
            made = subprocess.run([sys.executable, str(ROOT / 'tools' / 'minimize.py'),
                                   str(out / campaign / f'{battle}.json'), '--out', str(bundle)],
                                  cwd=ROOT, capture_output=True, text=True)
        if made.returncode == 0 and pair.exists():
            scenario = json.loads(pair.read_text())
            # Relative to the checkout where the run directory is in it (an `--out` elsewhere: as given).
            minimized[sig] = {'battle': battle,
                              'path': str(pair.relative_to(ROOT) if pair.is_relative_to(ROOT) else pair),
                              'entries': len(scenario['script']),
                              'pokemon': sum(len(t) for t in scenario['teams'].values())}
        else:
            minimized[sig] = {'battle': battle, 'error': (made.stdout + made.stderr)[-300:]}

    battles = sum(c.get('battles', 0) for c in campaigns)
    passed = sum(c.get('passed', 0) for c in campaigns)
    metrics = {
        'minutes': round((time.monotonic() - start) / 60, 1),
        'stopped_on_new_after_minutes': stopped,
        'campaigns': len(campaigns),
        'battles': battles,
        'passed': passed,
        'pass_rate': round(passed / battles, 4) if battles else None,
        'per_campaign': [{k: v for k, v in c.items() if k != 'results'} for c in campaigns],
        'known': [{'signature': sig, 'battles': len(groups[sig]), 'why': q.get('why', '')}
                  for sig, q in known.items()],
        'new': [{'signature': sig, 'battles': [b for _, b in groups[sig]], 'minimized': minimized.get(sig)}
                for sig in groups if sig not in known],
    }
    coverage = load(args.coverage, {})
    metrics['coverage'] = {
        generation: {'move_tags': sum(1 for t in counts.get('tags', {}) if not t.startswith('event:')),
                     'event_tags': sum(1 for t in counts.get('tags', {}) if t.startswith('event:')),
                     'pairs': len(counts.get('pairs', {})),
                     'triples': len(counts.get('triples', {}))}
        for generation, counts in sorted(coverage.items())
    }
    (out / 'nightly.json').write_text(json.dumps(metrics, indent=1, ensure_ascii=False) + '\n')
    lines = [f'# M8 nightly {out.name}', '',
             f'- {metrics["campaigns"]} campaigns, {battles} battles, {passed} passed'
             f' ({metrics["pass_rate"]:.1%})' if battles else '- no battle ran']
    if stopped is not None:
        lines.append(f'- stopped after {stopped} minutes of campaigns on a new signature (--stop-on-new)')
    lines.append('')
    lines += ['## Campaigns', ''] + [
        f'- {c["campaign"]}: ' + (f'{c["passed"]} / {c["battles"]}' if 'battles' in c else f'error {c["error"][-120:]!r}')
        for c in campaigns]
    lines += ['', '## Known (quarantined)', ''] + [
        f'- {k["battles"]}× `{k["signature"]}` — {k["why"]}' for k in metrics['known']] or ['- none']
    lines += ['', '## New', '']
    for entry in metrics['new']:
        made = entry['minimized'] or {}
        where = (f'minimized to {made["entries"]} entries, {made["pokemon"]} Pokémon: `{made["path"]}`'
                 if 'path' in made else f'not minimized {made.get("error", "(no time left)")[-120:]!r}')
        lines.append(f'- {len(entry["battles"])}× `{entry["signature"]}` — {where}')
    if not metrics['new']:
        lines.append('- none')
    lines += ['', '## Mechanic coverage so far', ''] + [
        f'- {generation}: {c["move_tags"]} move tags, {c["event_tags"]} event tags, {c["pairs"]} pairs, '
        f'{c["triples"]} triples'
        for generation, c in metrics['coverage'].items()]
    (out / 'nightly.md').write_text('\n'.join(lines) + '\n')
    print('\n'.join(lines))
    if args.fail_on_new and metrics['new']:
        sys.exit(1)


if __name__ == '__main__':
    main()
