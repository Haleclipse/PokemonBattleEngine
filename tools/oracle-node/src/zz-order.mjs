import fs from 'node:fs';
import {Battle, Teams, Dex} from '@pkmn/sim';
import {packable} from './sets.mjs';
// Patch the gen1 Pokemon class's modifyStat through the mod data itself.
const gen1Pokemon = Dex.mod('gen1').data.Scripts.pokemon;
const origModify = gen1Pokemon.modifyStat;
gen1Pokemon.modifyStat = function (statName, modifier) {
  const before = this.modifiedStats[statName];
  const r = origModify.call(this, statName, modifier);
  process.stderr.write(`modifyStat ${this.name} ${statName} x${modifier}: ${before} -> ${this.modifiedStats[statName]} (turn ${this.battle.turn})\n`);
  return r;
};
const scenario = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const battle = new Battle({formatid: scenario.battle.format, seed: scenario.random.seed, strictChoices: true, debug: true});
battle.setPlayer('p1', {name: 'P1', team: Teams.pack(scenario.teams.p1.map(s => packable(battle, s)))});
battle.setPlayer('p2', {name: 'P2', team: Teams.pack(scenario.teams.p2.map(s => packable(battle, s)))});
if (battle.p1.activeRequest?.teamPreview) battle.makeChoices('default','default');
for (let i = 0; i < scenario.script.length && !battle.ended; i++) {
  const c = scenario.script[i];
  if (battle.turn === 33 && battle.requestState === 'move') {
    const p1a = battle.p1.active[0], p2a = battle.p2.active[0];
    process.stderr.write(`T33p1 ${p1a.name} spe=${p1a.getStat('spe',false,false)} mod=${JSON.stringify(p1a.modifiedStats)} status=${p1a.status}\n`);
    process.stderr.write(`T33p2 ${p2a.name} spe=${p2a.getStat('spe',false,false)} mod=${JSON.stringify(p2a.modifiedStats)} status=${p2a.status}\n`);
  }
  const ans = (side, choice) => choice ?? (side.activeRequest?.wait ? '' : 'pass');
  battle.makeChoices(ans(battle.p1, c.p1), ans(battle.p2, c.p2));
}
