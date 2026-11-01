#!/usr/bin/env python3
"""M8 differential campaign (ROADMAP §12, TESTING.md §6.5).

    python3 tools/campaign.py --gen 9 [--mode singles|doubles] [--battles 20] [--seed 1]
                              [--out target/m8/<name>] [--coverage target/m8/coverage.json]
    python3 tools/campaign.py --format gen9ou [--battles 20] [--seed 1] …

`--format` plays a real format instead of a custom game (M9.6, DESIGN.md §45.8): each battle's
two teams are legal ones drawn by `format-teams.mjs` (`legal-teams.mjs`, every gender named),
with a Team Preview choice per side, and the replay goes through the pre-battle session as the
real-format fixtures do. Step 1 below is then skipped; the rest is the same.

1. Samples teams from the content the engine claims (`examples/content_pool`), least-covered
   first: every move, ability and item has a running count across campaigns (`--coverage`),
   and each new Pokémon takes the ones used least so far. Moves go by their mechanic tags
   first (ROADMAP §12.1) in every other slot: a move carrying a tag exercised least often
   wins, then one that would pair its tags with the battle's for the first time; the other
   slots keep taking the least used move.
2. Plays each battle in Showdown with the deterministic corpus policy (`corpus.mjs`) and
   records the oracle frames (`showdown.mjs`). What Showdown actually did feeds the mechanic
   coverage: per turn, the tags of the moves used and the events seen (a status landing, a
   volatile starting, a side condition, the weather, a critical hit, a faint …), each counted
   once, every pair of them seen together in a turn, and every triple of event tags (events
   only: a turn shows about three, so their triples stay few).
3. Replays every battle in Rust (`tests/campaign.rs`), which writes `results.json` and a
   failure bundle per diverging battle.
4. Groups the divergences by signature (the first divergence with ids, numbers and positions
   erased) and writes `summary.md`: pass / diverge / panic counts, the signatures with their
   battles, and what the campaign covered. Signatures listed in `tests/fixtures/quarantine.json`
   are reported as known, not new.
"""
import argparse
import itertools
import json
import os
import random
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / 'tools' / 'oracle-node'


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def content_pool(generation):
    out = run(['cargo', 'run', '-q', '--example', 'content_pool', '--', str(generation)], cwd=ROOT)
    if out.returncode != 0:
        sys.exit(out.stderr)
    return json.loads(out.stdout)


class Coverage:
    """Running use counts per generation and kind, persisted across campaigns."""

    def __init__(self, path, generation):
        self.path = path
        self.key = f'gen{generation}'
        self.data = json.loads(path.read_text()) if path.exists() else {}
        self.counts = self.data.setdefault(self.key, {'moves': {}, 'abilities': {}, 'items': {}})
        self.counts.setdefault('tags', {})
        self.counts.setdefault('pairs', {})
        self.counts.setdefault('triples', {})

    def least_covered(self, rng, kind, pool, k, exclude=()):
        counts = self.counts[kind]
        candidates = [name for name in pool if name not in exclude]
        rng.shuffle(candidates)
        candidates.sort(key=lambda name: counts.get(name, 0))
        return candidates[:k]

    def mark(self, kind, name):
        self.counts[kind][name] = self.counts[kind].get(name, 0) + 1

    def save(self):
        # Written beside it, then moved into place: a full disk leaves the old file whole, not
        # an empty one every later campaign fails to read.
        self.path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.path.with_name(self.path.name + '.tmp')
        tmp.write_text(json.dumps(self.data, indent=1, sort_keys=True) + '\n')
        os.replace(tmp, self.path)


def pair(a, b):
    return '|'.join(sorted((a, b)))


def pick_moves(rng, pool, coverage, battle_tags, k=4):
    """k moves, one at a time, alternating two aims: every other slot takes the least used move
    (so every move keeps being played), the others the move with the least exercised tag, then
    the most tag pairs the battle has never shown. Ranking every slot by tags alone keeps a few
    rare-tag moves in every team and leaves most moves unplayed."""
    tags, pairs, uses = coverage.counts['tags'], coverage.counts['pairs'], coverage.counts['moves']
    candidates = list(pool['moves'])
    rng.shuffle(candidates)
    chosen = []
    for slot in range(k):
        def key(move):
            own = pool['move_tags'].get(move, [])
            rarest = min((tags.get(t, 0) for t in own), default=0)
            fresh = sum(1 for t in own for u in battle_tags if t != u and pair(t, u) not in pairs)
            if slot % 2 == 0:
                return (uses.get(move, 0), rarest, -fresh)
            return (rarest, -fresh, uses.get(move, 0))
        move = min((m for m in candidates if m not in chosen), key=key)
        chosen.append(move)
        battle_tags.update(pool['move_tags'].get(move, []))
    return chosen


def event_tags(event):
    """The mechanic an oracle event shows, as a tag (None for bookkeeping events)."""
    kind = event.get('kind')
    def clean(text):
        text = str(text or '')
        for prefix in ('[from] ', 'move: ', 'ability: ', 'item: '):
            text = text.replace(prefix, '')
        return text.strip().lower()
    args = event.get('args') or []
    if kind == 'status':
        return f"event:status:{clean(event.get('status'))}"
    # `-start|who|what`, `-sidestart|side|what`; `-fieldstart|what`, `-weather|what`.
    if kind in ('start', 'end', 'activate', 'sidestart', 'sideend') and len(args) > 1:
        return f'event:{kind}:{clean(args[1])}'
    if kind in ('fieldstart', 'fieldend', 'weather') and args:
        return f'event:{kind}:{clean(args[0])}'
    if kind == 'cant':
        return f"event:cant:{clean(event.get('detail'))}"
    if kind in ('damage', 'heal') and event.get('from'):
        return f"event:{kind}-from:{clean(event.get('from'))}"
    if kind in ('crit', 'miss', 'immune', 'faint', 'drag', 'hitcount', 'prepare', 'fail', 'boost',
                'unboost', 'singleturn', 'curestatus', 'enditem', 'ability', 'item', 'block',
                'notarget', 'supereffective', 'resisted'):
        return f'event:{kind}'
    return None


def observe(coverage, oracle, move_tags):
    """Counts, per boundary of the recorded battle, the tags of the moves used and the events
    seen (once each), every pair of them, and every triple of the event tags."""
    tags, pairs, triples = coverage.counts['tags'], coverage.counts['pairs'], coverage.counts['triples']
    for frame in oracle['frames']:
        seen = set()
        for event in frame.get('events', []):
            if event.get('kind') == 'move':
                seen.update(move_tags.get(event.get('move'), []))
            else:
                tag = event_tags(event)
                if tag:
                    seen.add(tag)
        for tag in seen:
            tags[tag] = tags.get(tag, 0) + 1
        ordered = sorted(seen)
        for i, a in enumerate(ordered):
            for b in ordered[i + 1:]:
                key = pair(a, b)
                pairs[key] = pairs.get(key, 0) + 1
        events = [tag for tag in ordered if tag.startswith('event:')]
        for triple in itertools.combinations(events, 3):
            key = '|'.join(triple)
            triples[key] = triples.get(key, 0) + 1


def team(rng, pool, coverage, generation, size, taken_species, battle_tags):
    members = []
    for index in range(size):
        species = rng.choice([s for s in pool['species'] if s not in taken_species])
        taken_species.add(species)
        moves = pick_moves(rng, pool, coverage, battle_tags)
        member = {
            'label': f'{species} {index + 1}' if species in [m['species'] for m in members] else species,
            'species': species,
            'level': rng.randint(45, 55),
            'nature': 'Hardy',
            'moves': moves,
        }
        if generation >= 3 and pool['abilities']:
            member['ability'] = coverage.least_covered(rng, 'abilities', pool['abilities'], 1)[0]
            coverage.mark('abilities', member['ability'])
        if generation >= 2 and pool['items'] and rng.random() < 0.7:
            member['item'] = coverage.least_covered(rng, 'items', pool['items'], 1)[0]
            coverage.mark('items', member['item'])
        for move in moves:
            coverage.mark('moves', move)
        members.append(member)
    return members


def signature(first):
    """A divergence with the battle-specific parts erased, so equal causes group together."""
    text = re.sub(r'^boundary_\d+: ', '', first or '')
    text = re.sub(r'Who \{[^}]*\}', '<who>', text)
    text = re.sub(r'PokemonId\(\d+\)', '<pokemon>', text)
    text = re.sub(r'\[\d+\]', '[*]', text)
    text = re.sub(r'rust gave .*', '', text)
    text = re.sub(r'Some\("[^"]*"\)', '<move>', text)
    text = re.sub(r'outcome \[.*?\];?', 'outcome', text)
    text = re.sub(r'rust=\S+ showdown=\S+', 'differs', text)
    text = re.sub(r'\d+', 'N', text)
    return text.strip(' ;')


def keep_generation_error(out, battle_id, scenario, failed):
    """A battle Showdown could not play or record: its scenario and the whole stderr go to
    `generation-errors/`, so it can be run again; the error line (not Node's version banner,
    the last line of a crash) goes to the summary."""
    kept = out / 'generation-errors'
    kept.mkdir(exist_ok=True)
    (kept / f'{battle_id}.json').write_text(json.dumps(scenario, indent=2, ensure_ascii=False) + '\n')
    (kept / f'{battle_id}.stderr').write_text(failed.stderr)
    lines = failed.stderr.strip().splitlines()
    return [next((line for line in lines if 'Error' in line), lines[-1] if lines else '')]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--gen', type=int, default=9)
    parser.add_argument('--mode', choices=['singles', 'doubles'], default='singles')
    parser.add_argument('--battles', type=int, default=20)
    parser.add_argument('--seed', type=int, default=1)
    parser.add_argument('--out', type=Path)
    parser.add_argument('--coverage', type=Path, default=ROOT / 'target' / 'm8' / 'coverage.json')
    parser.add_argument('--format', help='a real format (DESIGN.md §45.8); --gen and --mode follow it')
    args = parser.parse_args()
    drawn = None
    if args.format:
        teams = run(['node', 'src/format-teams.mjs', args.format, str(args.seed), str(args.battles)],
                    cwd=ORACLE)
        if teams.returncode != 0:
            sys.exit(teams.stderr[-2000:])
        drawn = json.loads(teams.stdout)
        args.gen = drawn[0]['battle']['generation'] if drawn else args.gen
        args.mode = drawn[0]['battle']['mode'] if drawn else args.mode
        name = f'{args.format}-s{args.seed}'
    else:
        name = f'gen{args.gen}-{args.mode}-s{args.seed}'
    out = args.out or ROOT / 'target' / 'm8' / name
    out.mkdir(parents=True, exist_ok=True)

    rng = random.Random(f'{args.gen}-{args.mode}-{args.seed}')
    pool = content_pool(args.gen)
    coverage = Coverage(args.coverage, args.gen)
    size = 6 if args.mode == 'singles' else 4
    fmt = f'gen{args.gen}{"doubles" if args.mode == "doubles" else ""}customgame'
    generated, errors = 0, []
    for index in range(args.battles):
        if drawn is not None:
            battle_id = f'm9-{name}-{index:03d}'
            scenario = {
                'schema': 1,
                'id': battle_id,
                'notes': [f'M9.6 campaign {name}, battle {index}: legal teams from Team Preview on'],
                **drawn[index],
                'corpus': {'mechanics': True, 'switchEvery': rng.randint(4, 9), 'maxTurns': 60},
            }
            scenario['random'] = {'mode': 'oracle-guided', **scenario['random']}
        else:
            battle_id = f'm8-{name}-{index:03d}'
        taken = set()
        battle_tags = set()
        scenario = scenario if drawn is not None else {
            'schema': 1,
            'id': battle_id,
            'notes': [f'M8 campaign {name}, battle {index}: least-covered sampling'],
            'battle': {'generation': args.gen, 'mode': args.mode, 'format': fmt},
            'teams': {
                'p1': team(rng, pool, coverage, args.gen, size, taken, battle_tags),
                'p2': team(rng, pool, coverage, args.gen, size, taken, battle_tags),
            },
            'random': {'mode': 'oracle-guided', 'seed': [rng.randrange(1, 65536) for _ in range(4)]},
            'corpus': {'switchEvery': rng.randint(4, 9), 'maxTurns': 60},
        }
        played = run(['node', 'src/corpus.mjs'], input=json.dumps(scenario), cwd=ORACLE)
        if played.returncode != 0:
            errors.append((battle_id, 'corpus', keep_generation_error(out, battle_id, scenario, played)))
            continue
        recorded = run(['node', 'src/showdown.mjs'], input=played.stdout, cwd=ORACLE)
        if recorded.returncode != 0:
            errors.append((battle_id, 'showdown',
                           keep_generation_error(out, battle_id, json.loads(played.stdout), recorded)))
            continue
        (out / f'{battle_id}.json').write_text(played.stdout)
        # The recording kept compact (no indentation): the same JSON in about a third of the
        # space — a Nightly's recordings are the regression corpus the full replay keeps.
        recording = json.loads(recorded.stdout)
        (out / f'{battle_id}.showdown.json').write_text(
            json.dumps(recording, separators=(',', ':'), ensure_ascii=False) + '\n')
        observe(coverage, recording, pool['move_tags'])
        generated += 1
    coverage.save()

    replay = run(['cargo', 'test', '--release', '--test', 'campaign', '--', '--ignored', '--nocapture'],
                 cwd=ROOT, env={**os.environ, 'CAMPAIGN_DIR': str(out)})
    results_path = out / 'results.json'
    if not results_path.exists():
        sys.exit(replay.stdout[-2000:] + replay.stderr[-2000:])
    results = json.loads(results_path.read_text())

    quarantine_path = ROOT / 'tests' / 'fixtures' / 'quarantine.json'
    quarantine = json.loads(quarantine_path.read_text())['signatures'] if quarantine_path.exists() else []
    groups = defaultdict(list)
    for entry in results['results']:
        if entry['status'] == 'diverged':
            groups[signature(entry['first_divergence'])].append(entry['id'])
        elif entry['status'] == 'panicked':
            groups['panic: ' + signature(entry['message'])].append(entry['id'])
    known = {sig for sig in groups if any(re.search(q['pattern'], sig) for q in quarantine)}

    lines = [f'# {"M9" if drawn is not None else "M8"} campaign {name}', '',
             f'- battles generated: {generated} (generation errors: {len(errors)})',
             f'- passed: {results["passed"]}, diverged: {results["diverged"]}, panicked: {results["panicked"]}',
             f'- signatures: {len(groups)} ({len(groups) - len(known)} new, {len(known)} quarantined)', '']
    for sig, ids in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        tag = 'known' if sig in known else 'NEW'
        lines.append(f'- [{tag}] {len(ids)}× `{sig}` — {", ".join(ids[:5])}{" …" if len(ids) > 5 else ""}')
    if errors:
        lines += ['', '## Generation errors', ''] + [f'- {i} ({stage}): {msg}' for i, stage, msg in errors]
    counts = coverage.counts
    lines += ['', '## Coverage after this campaign', '']
    for kind in ('moves', 'abilities', 'items'):
        values = [counts[kind].get(n, 0) for n in pool[kind]]
        if values:
            lines.append(f'- {kind}: {sum(1 for v in values if v)} / {len(values)} used at least once, '
                         f'minimum {min(values)}')
    move_tags = sorted({t for tags in pool['move_tags'].values() for t in tags})
    exercised = [counts['tags'].get(t, 0) for t in move_tags]
    lines.append(f'- move tags: {sum(1 for v in exercised if v)} / {len(move_tags)} exercised, minimum '
                 f'{min(exercised) if exercised else 0}')
    lines.append(f'- event tags seen: {sum(1 for t in counts["tags"] if t.startswith("event:"))}')
    lines.append(f'- tag pairs seen together in a turn: {len(counts["pairs"])}')
    lines.append(f'- event-tag triples seen together in a turn: {len(counts["triples"])}')
    (out / 'summary.md').write_text('\n'.join(lines) + '\n')
    print('\n'.join(lines))


if __name__ == '__main__':
    main()
