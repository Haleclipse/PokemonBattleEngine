// A real format's battle before it is played (M9.4c, M9.6; DESIGN.md §45.5, §45.8). Per side it
// draws a legal team (legal-teams.mjs) with every gender named, so that no draw stands between
// the two engines, and a Team Preview choice. The real-format fixtures (format-cases.mjs) and the
// real-format campaigns (tools/campaign.py --format) both start here.
//
// Usage:  node src/format-teams.mjs <format> <campaign seed> <count>
//         prints a JSON array of `count` scenario skeletons (battle, teams, preview, random).
import {Dex, PRNG, TeamValidator} from '@pkmn/sim';
import {LegalTeams} from './legal-teams.mjs';

/// A legal team with every gender named, as scenario sets (`label`: the name Showdown gives).
export function legalTeam(formatid, draws, prng) {
  const dex = Dex.forFormat(Dex.formats.get(formatid));
  const validator = new TeamValidator(formatid);
  for (let attempt = 0; attempt < 20; attempt++) {
    const sets = draws.team(Math.min(6, Dex.formats.getRuleTable(Dex.formats.get(formatid)).maxTeamSize));
    if (!sets) break;
    for (const set of sets) {
      const species = dex.species.get(set.species);
      if (!set.gender && !species.gender) set.gender = prng.sample(['M', 'F']);
    }
    if (validator.validateTeam(JSON.parse(JSON.stringify(sets)))) continue;
    return sets.map(({name, ...set}) => ({label: name || dex.species.get(set.species).baseSpecies, ...set}));
  }
  throw new Error(`${formatid}: no legal team`);
}

/// A Team Preview choice: the whole team shuffled (`team 3142…`).
export function previewChoice(prng, size) {
  const slots = Array.from({length: size}, (_, i) => i + 1);
  prng.shuffle(slots);
  return `team ${slots.join('')}`;
}

/// A format's own number, so that two formats of one generation draw other teams.
export const formatHash = text => [...text].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 65521, 7);

/// The battle `n` of a format under these seeds: the side's team draws, then the genders, the
/// preview choices and the battle's seed from `seed`.
export function formatBattle(formatid, n, seeds) {
  const format = Dex.formats.get(formatid);
  const prng = new PRNG(seeds.battle);
  const p1 = legalTeam(formatid, new LegalTeams(formatid, seeds.p1), prng);
  const p2 = legalTeam(formatid, new LegalTeams(formatid, seeds.p2), prng);
  return {
    battle: {generation: Dex.forFormat(format).gen, mode: format.gameType, format: formatid},
    teams: {p1, p2},
    preview: {p1: previewChoice(prng, p1.length), p2: previewChoice(prng, p2.length)},
    random: {seed: seeds.battle},
  };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [formatid, seedArg, countArg] = process.argv.slice(2);
  const own = formatHash(formatid);
  const seed = Number(seedArg);
  // Campaign seeds put the campaign's seed and the battle's index where the fixtures' put the
  // battle's number and a constant (format-cases.mjs), so the two never draw the same battle.
  const battles = Array.from({length: Number(countArg)}, (_, i) => formatBattle(formatid, i, {
    p1: [own, seed, 1000 + 2 * i, 7],
    p2: [own, seed, 1001 + 2 * i, 17],
    battle: [own, seed, 1000 + i, 31],
  }));
  // Where the format offers open team sheets (`Open Team Sheets`, DESIGN.md §47.0 item 14), both
  // sides accept them in every other battle (no draw: the battles are those drawn before).
  if (Dex.formats.getRuleTable(Dex.formats.get(formatid)).has('openteamsheets')) {
    battles.forEach((battle, i) => {
      if (i % 2 === 0) battle.preview.open_team_sheets = true;
    });
  }
  process.stdout.write(JSON.stringify(battles) + '\n');
}
