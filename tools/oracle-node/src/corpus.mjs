// Drives a Showdown battle with a deterministic policy and writes the choices back into the
// scenario as its `script`, so a long battle can be pinned without hand-writing every turn.
// With `corpus: {mechanics: true}` the policy also spends each side's once-per-battle gimmick
// (Mega Evolution, Z-Move, Dynamax, Terastallization) on the first turn its request offers one
// (DESIGN.md §44.8); off by default.
// Policy, per active slot: rotate through the usable moves (a move that needs a target aims at
// the first standing foe, alternating between foes in doubles), switch the first slot to the
// first healthy bench Pokémon every `switchEvery` turns unless trapped, answer a forced switch
// with distinct healthy bench Pokémon. Input: a scenario without `script` (plus optional
// `corpus: {switchEvery, maxTurns}`).
import fs from 'node:fs';
import {Battle} from '@pkmn/sim';
import './cartridge.mjs';
import {battleTeam} from './sets.mjs';

const scenario = JSON.parse(fs.readFileSync(0, 'utf8'));
const generation = scenario.battle?.generation;
const doubles = scenario.battle?.mode === 'doubles';
const formatid = scenario.battle?.format ?? `gen${generation}${doubles ? 'doubles' : ''}customgame`;
const seed = scenario.random?.seed ?? [1, 2, 3, 4];
const switchEvery = scenario.corpus?.switchEvery ?? 7;
const maxTurns = scenario.corpus?.maxTurns ?? 80;

const battle = new Battle({formatid, seed, strictChoices: true});
battle.setPlayer('p1', {name: 'P1', team: battleTeam(battle, formatid, scenario.teams.p1)});
battle.setPlayer('p2', {name: 'P2', team: battleTeam(battle, formatid, scenario.teams.p2)});
if (battle.p1.activeRequest?.teamPreview || battle.p2.activeRequest?.teamPreview) {
  battle.makeChoices(scenario.preview?.p1 ?? 'default', scenario.preview?.p2 ?? 'default');
}

// Gimmicks (DESIGN.md §44.8): opt in with `corpus: {mechanics: true}`. The policy spends a
// side's once-per-battle gimmick on the first turn whose request offers one, as Showdown's
// choice syntax asks for it (`move N terastallize` / `mega` / `zmove` / `dynamax`), one per
// side. Off by default: a campaign recorded without it plays exactly as before, so enabling it
// only for new campaigns leaves the old recordings untouched.
const gimmicks = scenario.corpus?.mechanics === true;
const gimmickUsed = {p1: false, p2: false};

/// The suffix the side's gimmick adds to its chosen move, and whether it has one (`active` is
/// the request's slot entry: `canTerastallize` is the Tera type, the rest flags).
function gimmickSuffix(active, moves, pick) {
  if (active.canTerastallize) return ' terastallize';
  if (active.canMegaEvo) return ' mega';
  if (active.canZMove?.[pick.index - 1]) return ' zmove';
  if (active.canDynamax && moves[pick.index - 1]?.id && active.maxMoves?.maxMoves?.[pick.index - 1] &&
      !active.maxMoves.maxMoves[pick.index - 1].disabled) {
    return ' dynamax';
  }
  return '';
}

const CHOOSABLE = new Set(['normal', 'any', 'adjacentFoe']);
const ALLY = new Set(['adjacentAlly', 'adjacentAllyOrSelf']);
const rotation = {p1: [0, 0], p2: [0, 0]};
const aim = {p1: 0, p2: 0};

function healthyBench(request, taken) {
  const out = [];
  request.side.pokemon.forEach((p, i) => {
    const fainted = p.condition.endsWith(' fnt') || p.condition === '0 fnt';
    if (!p.active && !fainted && !taken.has(i + 1)) out.push(i + 1);
  });
  return out;
}

function foeSlots(sideId) {
  const foe = sideId === 'p1' ? battle.p2 : battle.p1;
  return foe.active.map((p, i) => (p && !p.fainted ? i + 1 : null)).filter(x => x !== null);
}

function choose(sideId, request) {
  if (!request || request.wait) return undefined;
  const taken = new Set();
  if (request.forceSwitch) {
    return request.forceSwitch.map((needed, slot) => {
      if (!needed) return 'pass';
      // Revival Blessing asks for a fainted Pokémon. Showdown also takes a pass when no healthy
      // Pokémon is left to switch in (its forced-pass count); a player names one.
      if (request.side.pokemon[slot]?.reviving) {
        const n = request.side.pokemon.findIndex((p, i) =>
          p.condition.endsWith(' fnt') && !taken.has(i + 1)) + 1;
        if (!n) return 'pass';
        taken.add(n);
        return `switch ${n}`;
      }
      const [n] = healthyBench(request, taken);
      if (!n) return 'pass';
      taken.add(n);
      return `switch ${n}`;
    }).join(', ');
  }
  if (!request.active) return 'pass';
  const parts = request.active.map((active, slot) => {
    const me = request.side.pokemon[slot];
    if (!me || me.condition.endsWith(' fnt')) return 'pass';
    if (slot === 0 && battle.turn % switchEvery === 0 && !active.trapped && !active.maybeTrapped) {
      const [n] = healthyBench(request, taken);
      if (n) { taken.add(n); return `switch ${n}`; }
    }
    const usable = active.moves.map((m, i) => ({...m, index: i + 1}))
      .filter(m => !m.disabled && (m.pp === undefined || m.pp > 0));
    if (usable.length === 0) return 'move 1';
    const pick = usable[rotation[sideId][slot] % usable.length];
    rotation[sideId][slot] += 1;
    const suffix =
      gimmicks && !gimmickUsed[sideId] ? gimmickSuffix(active, active.moves, pick) : '';
    if (suffix) gimmickUsed[sideId] = true;
    if (!doubles) return `move ${pick.index}${suffix}`;
    // A Dynamax Pokémon (or one Dynamaxing now) uses the Max Move: its target, not the base
    // move's (Bulldoze hits all, Max Quake one foe).
    const dynamaxed = suffix === ' dynamax' || battle[sideId].active[slot]?.volatiles['dynamax'];
    // A Z-Move aims as the Z-Move does too: Explosion hits all, Breakneck Blitz one foe.
    const zTarget = suffix === ' zmove' && active.canZMove?.[pick.index - 1]?.target;
    const target = zTarget || dynamaxed && active.maxMoves?.maxMoves?.[pick.index - 1]?.target ||
      pick.target;
    if (CHOOSABLE.has(target)) {
      const foes = foeSlots(sideId);
      const target = foes[aim[sideId] % Math.max(foes.length, 1)] ?? 1;
      aim[sideId] += 1;
      return `move ${pick.index} ${target}${suffix}`;
    }
    if (ALLY.has(target)) return `move ${pick.index} ${slot === 0 ? -2 : -1}`;
    return `move ${pick.index}${suffix}`;
  });
  return parts.join(', ');
}

// A move a foe's Imprison disables is hidden from the request (`disableMove(id, 'hidden')`):
// choosing it fails with "[Unavailable choice]", and Showdown marks it disabled in that side's
// request first, as it does for a player — so the policy chooses again from the updated request.
const script = [];
while (!battle.ended && battle.turn <= maxTurns) {
  for (let attempt = 1; ; attempt++) {
    const c1 = choose('p1', battle.p1.activeRequest);
    const c2 = choose('p2', battle.p2.activeRequest);
    try {
      battle.makeChoices(c1 ?? '', c2 ?? '');
    } catch (error) {
      if (attempt < 8 && String(error.message).startsWith('[Unavailable choice]')) continue;
      throw error;
    }
    const entry = {};
    if (c1 !== undefined) entry.p1 = c1;
    if (c2 !== undefined) entry.p2 = c2;
    script.push(entry);
    break;
  }
}
const out = {...scenario, script};
delete out.corpus;
out.notes = [...(scenario.notes ?? []), `corpus: ${script.length} script entries, ${battle.turn} turns, ${battle.ended ? 'winner ' + battle.winner : 'unfinished'}`];
process.stdout.write(JSON.stringify(out, null, 2) + '\n');
process.stderr.write(`${script.length} entries, turn ${battle.turn}, ended=${battle.ended} winner=${battle.winner}\n`);
