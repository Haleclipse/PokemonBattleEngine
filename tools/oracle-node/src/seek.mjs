// Seed search for hand-written scenarios: plays the scenario on stdin with seeds [a, 2, 3, 4] for
// a = 1, 2, … and prints the first seed whose battle runs its whole script and whose log matches
// every pattern given as an argument (JavaScript regular expressions, `s` flag). Exit 1: none
// within --max (default 500) seeds.
import fs from 'node:fs';
import {Battle, Teams} from '@pkmn/sim';
import './cartridge.mjs';

const args = process.argv.slice(2);
const maxAt = args.indexOf('--max');
const max = maxAt >= 0 ? Number(args.splice(maxAt, 2)[1]) : 500;
const patterns = args.map(p => new RegExp(p, 's'));
const scenario = JSON.parse(fs.readFileSync(0, 'utf8'));
const formatid = scenario.battle?.format ?? `gen${scenario.battle?.generation}customgame`;
const packable = set => ({name: set.label ?? set.name ?? set.species, ...set});
const answer = (side, choice) => choice ?? (side.activeRequest?.wait ? '' : 'pass');
for (let a = 1; a <= max; a++) {
  const battle = new Battle({formatid, seed: [a, 2, 3, 4], strictChoices: true});
  battle.setPlayer('p1', {name: 'P1', team: Teams.pack(scenario.teams.p1.map(packable))});
  battle.setPlayer('p2', {name: 'P2', team: Teams.pack(scenario.teams.p2.map(packable))});
  if (battle.p1.activeRequest?.teamPreview || battle.p2.activeRequest?.teamPreview) battle.makeChoices('default', 'default');
  let complete = true;
  for (const choice of scenario.script ?? []) {
    if (battle.ended) break;
    try {
      battle.makeChoices(answer(battle.p1, choice.p1), answer(battle.p2, choice.p2));
    } catch {
      complete = false;
      break;
    }
  }
  const log = battle.log.join('\n');
  if (complete && patterns.every(p => p.test(log))) {
    console.log([a, 2, 3, 4].join(','));
    process.exit(0);
  }
}
process.exit(1);
