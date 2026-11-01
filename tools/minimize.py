#!/usr/bin/env python3
"""M8 minimizer (ROADMAP §12.3, TESTING.md §6.5).

    python3 tools/minimize.py <battle.json> [--out DIR] [--rounds 10]

Shrinks a diverging battle — a campaign pair `<id>.json` / `<id>.showdown.json`, or any
scenario — while its first divergence keeps the same signature (`tools/campaign.py`
`signature`: ids, numbers, positions and move names erased). Every candidate is played again
in Showdown (`showdown.mjs`, same seed) and replayed by the campaign runner (`tests/campaign.rs`,
built once); a candidate Showdown rejects, or whose divergence changes, is dropped. Passes, to a
fixpoint:

0. pin every gender Showdown draws while building the teams and advance the seed past those
   draws (`pin-genders.mjs`): the battle plays exactly as before, and deleting a Pokémon no
   longer shifts the random numbers;
1. cut the script after the boundary that diverges (the seed makes earlier frames identical);
2. name switches by label and moves by name (`switch Snorlax`, `move Body Slam`, both engines
   accept them) so that a choice keeps its meaning whatever the passes below delete — each
   rewrite kept only if Showdown's log is unchanged (a locked Pokémon's `move 1` stays);
3. delete script entries, in halving chunks (ddmin), and fold an opening entry of switches
   only into the team order (the Pokémon switched in lead instead);
4. delete team members;
5. delete moves never chosen, renumbering the `move N` choices of their owner;
6. neutral content: ability Run Away, no item, level 50.

Writes `<out>/<id>-min.json` and its `.showdown.json` (a campaign pair: `CAMPAIGN_DIR=<out>`
replays it) and prints each pass's effect. `tools/promote.py` turns it into a regression case
once the fix is in.
"""
import argparse
import copy
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / 'tools' / 'oracle-node'

_spec = importlib.util.spec_from_file_location('campaign', ROOT / 'tools' / 'campaign.py')
_campaign = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_campaign)
signature = _campaign.signature

NEUTRAL_ABILITY = 'Run Away'
NEUTRAL_LEVEL = 50


class Judge:
    """Plays a candidate in Showdown and replays it in Rust; says what its divergence is."""

    def __init__(self):
        built = subprocess.run(
            ['cargo', 'test', '--release', '--test', 'campaign', '--no-run', '--message-format=json'],
            cwd=ROOT, capture_output=True, text=True)
        executables = [json.loads(line).get('executable') for line in built.stdout.splitlines()
                       if line.startswith('{') and '"executable"' in line]
        executables = [e for e in executables if e]
        if built.returncode != 0 or not executables:
            sys.exit(built.stderr[-2000:])
        self.exe = executables[-1]
        self.dir = Path(tempfile.mkdtemp(prefix='m8-minimize-'))
        self.runs = 0

    def showdown(self, scenario):
        played = subprocess.run(['node', 'src/showdown.mjs'], input=json.dumps(scenario),
                                cwd=ORACLE, capture_output=True, text=True)
        if played.returncode != 0:
            return None
        return json.loads(played.stdout)

    def replay(self, scenario, oracle):
        for path in self.dir.iterdir():
            if path.is_dir():
                shutil.rmtree(path)
            else:
                path.unlink()
        (self.dir / 'battle.json').write_text(json.dumps(scenario))
        (self.dir / 'battle.showdown.json').write_text(json.dumps(oracle))
        subprocess.run([self.exe, '--ignored', '--nocapture'], cwd=ROOT, capture_output=True,
                       env={**os.environ, 'CAMPAIGN_DIR': str(self.dir)})
        self.runs += 1
        results = self.dir / 'results.json'
        if not results.exists():
            return ('error', '')
        entry = json.loads(results.read_text())['results'][0]
        return (entry['status'], entry.get('first_divergence') or entry.get('message') or '')

    def verdict(self, scenario):
        """(oracle, status, first divergence text, signature), or None when Showdown rejects."""
        oracle = self.showdown(scenario)
        if oracle is None:
            return None
        status, text = self.replay(scenario, oracle)
        return oracle, status, text, (status, signature(text))


def size(scenario):
    teams = scenario['teams'].values()
    return {
        'entries': len(scenario['script']),
        'pokemon': sum(len(team) for team in teams),
        'moves': sum(len(member['moves']) for team in teams for member in team),
        'abilities': sum(1 for team in teams for m in team if m.get('ability') not in (None, NEUTRAL_ABILITY)),
        'items': sum(1 for team in teams for m in team if m.get('item')),
    }


def active_slots(scenario):
    return 2 if scenario['battle'].get('mode') == 'doubles' else 1


def side_key(who):
    return 'p1' if who['side'] == 0 else 'p2'


def walk_boundaries(scenario, oracle):
    """For every script entry, the Showdown party order and actives before it (by label),
    following every switch-in and drag in the oracle's events (Showdown swaps the positions of
    the Pokémon coming in and the one it replaces)."""
    party = {side: [m['label'] for m in scenario['teams'][side]] for side in ('p1', 'p2')}
    frames = {frame['checkpoint']: frame for frame in oracle['frames']}

    def apply(frame):
        for event in (frame or {}).get('events', []):
            if event['kind'] in ('switch', 'drag') and event.get('pokemon'):
                who = event['pokemon']
                order = party[side_key(who)]
                if who['name'] in order and who.get('slot') is not None:
                    at, slot = order.index(who['name']), who['slot']
                    order[at], order[slot] = order[slot], order[at]

    apply(frames.get('initial'))
    states = []
    for index in range(len(scenario['script'])):
        states.append({side: list(order) for side, order in party.items()})
        apply(frames.get(f'boundary_{index + 1}'))
    return states


def parts(text):
    return [part.strip() for part in text.split(',')] if text is not None else []


def with_labelled_switches(scenario, oracle):
    candidate = copy.deepcopy(scenario)
    for entry, party in zip(candidate['script'], walk_boundaries(scenario, oracle)):
        for side in ('p1', 'p2'):
            if entry.get(side) is None:
                continue
            rewritten = []
            for part in parts(entry[side]):
                match = re.fullmatch(r'switch (\d+)', part)
                if match and int(match.group(1)) <= len(party[side]):
                    part = f'switch {party[side][int(match.group(1)) - 1]}'
                rewritten.append(part)
            entry[side] = ', '.join(rewritten)
    return candidate


MOVE_CHOICE = re.compile(r'move (.+?)((?: [-+]?[1-3])?)$')


def to_id(name):
    return re.sub(r'[^a-z0-9]', '', name.lower())


def move_choices(scenario, oracle):
    """Every `move` choice as (entry, side, part, owner label, index or name, target suffix):
    the owner is the Pokémon active in that slot when the choice was made."""
    choices = []
    slots = active_slots(scenario)
    for index, (entry, party) in enumerate(zip(scenario['script'], walk_boundaries(scenario, oracle))):
        for side in ('p1', 'p2'):
            for at, part in enumerate(parts(entry.get(side))):
                match = MOVE_CHOICE.fullmatch(part)
                if match and at < slots and at < len(party[side]):
                    spec = match.group(1)
                    spec = int(spec) if spec.isdigit() else spec
                    choices.append((index, side, at, party[side][at], spec, match.group(2)))
    return choices


def with_named_moves(scenario, oracle, only=None):
    """`move N` → `move <name>` for the entries in `only` (all when None)."""
    candidate = copy.deepcopy(scenario)
    movesets = {side: {m['label']: m['moves'] for m in scenario['teams'][side]} for side in ('p1', 'p2')}
    for index, side, at, owner, spec, target in move_choices(scenario, oracle):
        if not isinstance(spec, int) or (only is not None and index not in only):
            continue
        moves = movesets[side].get(owner, [])
        if spec <= len(moves):
            pieces = parts(candidate['script'][index][side])
            pieces[at] = f'move {moves[spec - 1]}{target}'
            candidate['script'][index][side] = ', '.join(pieces)
    return candidate


def cut_after_divergence(scenario, text):
    match = re.match(r'boundary_(\d+)', text)
    if not match or int(match.group(1)) >= len(scenario['script']):
        return None
    candidate = copy.deepcopy(scenario)
    candidate['script'] = candidate['script'][:int(match.group(1))]
    return candidate


class Minimizer:
    def __init__(self, judge, scenario, log):
        self.judge = judge
        self.log = log
        found = judge.verdict(scenario)
        if found is None:
            sys.exit('Showdown rejects the battle as given')
        self.oracle, status, self.text, self.target = found
        if status == 'pass':
            sys.exit('the battle does not diverge: nothing to minimize')
        self.scenario = scenario
        log(f'signature: {self.target[0]}: {self.target[1]}')

    def accept(self, candidate):
        found = self.judge.verdict(candidate)
        if found is None or found[3] != self.target:
            return False
        self.scenario, (self.oracle, _, self.text, _) = candidate, found
        return True

    def cut(self):
        candidate = cut_after_divergence(self.scenario, self.text)
        return candidate is not None and self.accept(candidate)

    def pin_genders(self):
        pinned = subprocess.run(['node', 'src/pin-genders.mjs'], input=json.dumps(self.scenario),
                                cwd=ORACLE, capture_output=True, text=True)
        if pinned.returncode != 0:
            return False
        candidate = json.loads(pinned.stdout)
        return candidate != self.scenario and self.same_battle(candidate) and self.accept(candidate)

    def same_battle(self, candidate):
        replayed = self.judge.showdown(candidate)
        return replayed is not None and replayed['debug_log'] == self.oracle['debug_log']

    def name_choices(self):
        candidate = with_labelled_switches(self.scenario, self.oracle)
        if not self.same_battle(candidate):
            self.log('  switches kept by number: the labelled script plays differently')
            candidate = self.scenario
        named = with_named_moves(candidate, self.oracle)
        if self.same_battle(named):
            candidate = named
        else:
            # A locked Pokémon's request lists only the locked move: its `move 1` stays.
            for index in sorted({c[0] for c in move_choices(candidate, self.oracle)}):
                one = with_named_moves(candidate, self.oracle, {index})
                if one != candidate and self.same_battle(one):
                    candidate = one
        return candidate != self.scenario and self.accept(candidate)

    def drop_entries(self):
        changed = False
        chunk = max(1, (len(self.scenario['script']) - 1) // 2)
        while chunk >= 1:
            start = 0
            # The last entry is where the battle diverges: keep it.
            while start < len(self.scenario['script']) - 1:
                candidate = copy.deepcopy(self.scenario)
                del candidate['script'][start:min(start + chunk, len(candidate['script']) - 1)]
                if self.accept(candidate):
                    changed = True
                    self.cut()
                else:
                    start += chunk
            chunk //= 2
        return changed

    def fold_opening_switches(self):
        script = self.scenario['script']
        if len(script) < 2:
            return False
        first = script[0]
        pieces = {side: parts(first.get(side)) for side in ('p1', 'p2')}
        if not all(p == 'pass' or p.startswith('switch ') for side in pieces for p in pieces[side]):
            return False
        party = walk_boundaries(self.scenario, self.oracle)[0]
        candidate = copy.deepcopy(self.scenario)
        for side, side_parts in pieces.items():
            team = candidate['teams'][side]
            for slot, part in enumerate(side_parts):
                if not part.startswith('switch '):
                    continue
                target = part[len('switch '):]
                label = party[side][int(target) - 1] if target.isdigit() else target
                at = next(i for i, m in enumerate(team) if m['label'] == label)
                team[slot], team[at] = team[at], team[slot]
        del candidate['script'][0]
        return self.accept(candidate)

    def drop_members(self):
        changed = False
        for side in ('p1', 'p2'):
            index = len(self.scenario['teams'][side]) - 1
            while index >= 0 and len(self.scenario['teams'][side]) > active_slots(self.scenario):
                label = self.scenario['teams'][side][index]['label']
                named = any(re.search(rf'switch {re.escape(label)}\b', entry.get(side) or '')
                            for entry in self.scenario['script'])
                if not named:
                    candidate = copy.deepcopy(self.scenario)
                    del candidate['teams'][side][index]
                    if self.accept(candidate):
                        changed = True
                index -= 1
        return changed

    def drop_moves(self):
        changed = False
        for side in ('p1', 'p2'):
            for member in range(len(self.scenario['teams'][side])):
                slot = len(self.scenario['teams'][side][member]['moves']) - 1
                while slot >= 0 and len(self.scenario['teams'][side][member]['moves']) > 1:
                    owner = self.scenario['teams'][side][member]
                    name = to_id(owner['moves'][slot])
                    choices = [c for c in move_choices(self.scenario, self.oracle)
                               if c[1] == side and c[3] == owner['label']]
                    used = any(spec == slot + 1 if isinstance(spec, int) else to_id(spec) == name
                               for *_, spec, _ in choices)
                    if not used:
                        candidate = copy.deepcopy(self.scenario)
                        del candidate['teams'][side][member]['moves'][slot]
                        for index, _, at, _, spec, target in choices:
                            if isinstance(spec, int) and spec - 1 > slot:
                                pieces = parts(candidate['script'][index][side])
                                pieces[at] = f'move {spec - 1}{target}'
                                candidate['script'][index][side] = ', '.join(pieces)
                        if self.accept(candidate):
                            changed = True
                    slot -= 1
        return changed

    def neutralize(self):
        changed = False
        generation = self.scenario['battle']['generation']
        edits = [('ability', NEUTRAL_ABILITY)] if generation >= 3 else []
        edits += [('item', None), ('level', NEUTRAL_LEVEL)]
        for field, value in edits:
            # Everyone at once first, then one by one.
            members = [(side, i) for side in ('p1', 'p2') for i in range(len(self.scenario['teams'][side]))]
            todo = [m for m in members if self.scenario['teams'][m[0]][m[1]].get(field) != value]
            for group in ([todo] if len(todo) > 1 else []) + [[m] for m in todo]:
                candidate = copy.deepcopy(self.scenario)
                for side, i in group:
                    member = candidate['teams'][side][i]
                    if member.get(field) == value:
                        continue
                    if value is None:
                        member.pop(field, None)
                    else:
                        member[field] = value
                if candidate != self.scenario and self.accept(candidate):
                    changed = True
                    if len(group) > 1:
                        break
        return changed

    def run(self, rounds):
        if self.pin_genders():
            self.log('  genders pinned, seed advanced past their draws')
        steps = [('cut after the divergence', self.cut), ('choices by name', self.name_choices),
                 ('script entries', self.drop_entries), ('opening switches', self.fold_opening_switches),
                 ('team members', self.drop_members),
                 ('unused moves', self.drop_moves), ('neutral content', self.neutralize)]
        for round_number in range(1, rounds + 1):
            any_change = False
            for name, step in steps:
                before = size(self.scenario)
                if step():
                    any_change = True
                    self.log(f'  round {round_number}, {name}: {before} → {size(self.scenario)}')
            if not any_change:
                break
        return self.scenario, self.oracle


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('battle', type=Path)
    parser.add_argument('--out', type=Path, default=ROOT / 'target' / 'm8' / 'minimized')
    parser.add_argument('--rounds', type=int, default=10)
    args = parser.parse_args()

    scenario = json.loads(args.battle.read_text())
    scenario.pop('corpus', None)
    original_id = scenario['id']
    judge = Judge()
    minimizer = Minimizer(judge, scenario, print)
    start = size(minimizer.scenario)
    result, oracle = minimizer.run(args.rounds)

    result = copy.deepcopy(result)
    result['id'] = f'{original_id}-min'
    result['notes'] = [f'minimized from {original_id} (tools/minimize.py): {minimizer.text}']
    args.out.mkdir(parents=True, exist_ok=True)
    path = args.out / f'{result["id"]}.json'
    path.write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')
    path.with_name(f'{result["id"]}.showdown.json').write_text(json.dumps(oracle))
    print(f'{start} → {size(result)} in {judge.runs} replays')
    print(f'first divergence: {minimizer.text}')
    print(f'wrote {path}')


if __name__ == '__main__':
    main()
