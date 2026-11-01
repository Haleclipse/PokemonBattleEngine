import fs from 'node:fs';
import {Battle} from '@pkmn/sim';
import './cartridge.mjs';
import {frame, parseEvents} from './normalize.mjs';
import {battleTeam} from './sets.mjs';
import {observers} from './views.mjs';

// Accepts the neutral scenario format (TESTING.md §4: battle / teams / script) and, for older
// fixtures, the raw form (formatid / p1 / p2 / choices).
const scenario = JSON.parse(fs.readFileSync(0, 'utf8'));
const generation = scenario.battle?.generation;
const formatid = scenario.formatid ?? scenario.battle?.format ?? `gen${generation}customgame`;
const teams = {p1: scenario.p1 ?? scenario.teams?.p1 ?? [], p2: scenario.p2 ?? scenario.teams?.p2 ?? []};
const script = scenario.choices ?? scenario.script ?? [];
const seed = scenario.seed ?? scenario.random?.seed ?? [1, 2, 3, 4];

const battle = new Battle({formatid, seed, strictChoices: true, debug: true});

battle.setPlayer('p1', {name: 'P1', team: battleTeam(battle, formatid, teams.p1)});
battle.setPlayer('p2', {name: 'P2', team: battleTeam(battle, formatid, teams.p2)});

// Team Preview (M9.4, DESIGN.md §45.5): the scenario's `preview` answers it (`team 3142`), or
// `default` (the declared order) — the custom games' only answer, whose replay starts at the
// turn-1 boundary. A real format's replay compares the `team_preview` frame too. From the
// choice on, a frame lists the Pokémon brought, in the order chosen (`oracleParty`).
// Showdown erases a failed move's target from its `move` line (`attrLastMove('[still]')`: "the
// target should never be known" to a player). The oracle needs it — a random retarget after an
// Encore that picked an already confused Pokémon fails silently — so keep it by log line.
const stillTargets = new Map();
const attrLastMove = battle.attrLastMove.bind(battle);
battle.attrLastMove = (...args) => {
  if (args.includes('[still]') && battle.lastMoveLine >= 0) {
    const target = battle.log[battle.lastMoveLine].split('|')[4];
    if (target && !stillTargets.has(battle.lastMoveLine)) stillTargets.set(battle.lastMoveLine, target);
  }
  return attrLastMove(...args);
};

// Illusion (DESIGN.md §41.7): Showdown names a disguised Pokémon by its disguise (`p1a: Lax`),
// in every line, while the frames and the replay go by who it is. Each log line keeps the names
// it was written under, mapped to the true ones; the events read the line with them put back,
// a `switch` into a disguise marked `illusion` (its details are the disguise's). The debug log
// stays as Showdown wrote it.
const aliases = new Map();
const trueDetails = new Map();
const logPush = battle.log.push;
battle.log.push = function (...lines) {
  const map = new Map();
  for (const side of battle.sides) {
    for (const pokemon of side.active) {
      if (pokemon?.illusion) map.set(`${pokemon.getSlot()}: ${pokemon.illusion.name}`, `${pokemon.getSlot()}: ${pokemon.name}`);
    }
  }
  if (map.size) {
    for (let k = 0; k < lines.length; k++) {
      aliases.set(this.length + k, map);
      // The disguised entrant's own details (its gender, which the line hides).
      const entry = /^\|(?:switch|drag)\|(p\d[a-d]): /.exec(lines[k]);
      const entrant = entry && battle.sides.flatMap(side => side.active)
        .find(pokemon => pokemon?.illusion && pokemon.getSlot() === entry[1]);
      if (entrant) trueDetails.set(this.length + k, entrant.details);
    }
  }
  return logPush.apply(this, lines);
};
const unalias = (index, text) => {
  const map = aliases.get(index);
  if (!map || text === undefined) return text;
  let out = text;
  for (const [shown, actual] of map) {
    const escaped = shown.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    out = out.replace(new RegExp(`${escaped}(?=\\||$)`, 'g'), actual);
  }
  return out;
};

const frames = [];
let cursor = 0;
// Each observer's knowledge at every checkpoint (DESIGN.md §47.0 items 9 and 12): what the
// stable API's `BattleView` is checked against.
const seen = observers(battle);
const take = checkpoint => {
  const from = cursor;
  const lines = battle.log.slice(cursor).map((line, i) => unalias(from + i, line));
  const disguised = i => trueDetails.get(from + i) ?? false;
  const events = parseEvents(lines, i => unalias(from + i, stillTargets.get(from + i)), disguised);
  cursor = battle.log.length;
  frames.push({...frame(battle, checkpoint), events, views: seen.take()});
};
if (battle.p1.activeRequest?.teamPreview || battle.p2.activeRequest?.teamPreview) {
  take('team_preview');
  // Both sides accepted the open team sheets (`Open Team Sheets`, DESIGN.md §47.0 item 14): the
  // server's `/acceptopenteamsheets` writes `>show-openteamsheets` once both have.
  if (scenario.preview?.open_team_sheets) battle.showOpenTeamSheets();
  battle.makeChoices(scenario.preview?.p1 ?? 'default', scenario.preview?.p2 ?? 'default');
  for (const side of battle.sides) side.oracleParty = side.pokemon.map(pokemon => pokemon.set);
}
take('initial');
// A script entry answers whatever Showdown is waiting for: turn choices, or the replacements
// of a turn suspended mid-way (U-turn / Volt Switch, fainting). A side that is not asked
// (`wait` request) must send nothing — an explicit `pass` there is a choice error.
const answer = (side, choice) => choice ?? (side.activeRequest?.wait ? '' : 'pass');
for (let i = 0; i < script.length && !battle.ended; i++) {
  const choice = script[i];
  battle.makeChoices(answer(battle.p1, choice.p1), answer(battle.p2, choice.p2));
  take(`boundary_${i + 1}`);
}

process.stdout.write(JSON.stringify({
  schema: 1,
  provenance: {engine: '@pkmn/sim', version: '0.10.11', formatid, seed},
  frames,
  // Wall-clock `|t:|` lines are stripped so a regenerated fixture only changes when the battle does.
  debug_log: battle.getDebugLog().split('\n').filter(line => !line.startsWith('|t:|')).join('\n'),
}, null, 2));
