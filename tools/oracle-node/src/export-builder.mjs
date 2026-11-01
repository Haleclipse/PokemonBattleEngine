// The battle builder's oracle (M9.4, DESIGN.md §45.5): for every target format, what its rules
// ask of the battle (the team's size, picks and preview; the special mechanics its clauses take
// away; the registration clauses), and the Pokémon Showdown builds from teams it admits — each
// team validated (which normalises its sets) and handed to a battle, as the server does.
//
// Usage:  node src/export-builder.mjs ../../tests/fixtures/formats
import fs from 'node:fs';
import path from 'node:path';
import {Battle, Dex, Teams, TeamValidator, toID} from '@pkmn/sim';
import {LegalTeams} from './legal-teams.mjs';

const fixtureDir = process.argv[2] ?? path.join('..', '..', 'tests', 'fixtures', 'formats');
const VERSION = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url))).dependencies;
const TARGET = /^gen[1-9](ou|ubers|doublesou|vgc\d+.*)$/;
const targets = Dex.formats.all()
  .filter(f => f.effectType === 'Format' && TARGET.test(f.id))
  .sort((a, b) => a.id.localeCompare(b.id));
const TEAMS = 4;

/// What the format asks of a battle, read off a battle of two of its teams.
function rulesOf(format, teams) {
  const table = Dex.formats.getRuleTable(format);
  const battle = new Battle({formatid: format.id, seed: [1, 2, 3, 4]});
  battle.setPlayer('p1', {name: 'P1', team: Teams.pack(teams[0])});
  battle.setPlayer('p2', {name: 'P2', team: Teams.pack(teams[1])});
  const request = battle.p1.activeRequest;
  const itemClause = table.has('itemclause') ? Number(table.valueRules.get('itemclause')) || 1 : null;
  return {
    game_type: format.gameType,
    max_team_size: table.maxTeamSize,
    picked_team_size: table.pickedTeamSize ?? null,
    team_preview: !!request?.teamPreview,
    max_chosen_team_size: request?.teamPreview ? request.maxChosenTeamSize ?? null : null,
    // Dynamax Clause's `onBegin` spends each side's Dynamax; Terastal Clause takes every
    // Pokémon's Tera away. Read where the generation has the mechanic.
    dynamax: battle.gen === 8 ? !battle.p1.dynamaxUsed : null,
    tera: battle.gen === 9 ? battle.p1.pokemon.some(p => p.canTerastallize) : null,
    species_clause: table.has('speciesclause'),
    item_clause: itemClause,
    sleep_clause: table.has('sleepclausemod'),
  };
}

/// The Pokémon Showdown's `Side` builds from a validated team: the side alone, before a battle
/// starts (its leads' switch-in would change what is read).
function built(format, team) {
  const battle = new Battle({formatid: format.id, seed: [1, 2, 3, 4]});
  battle.setPlayer('p1', {name: 'P1', team: Teams.pack(team)});
  return battle.p1.pokemon.map((pokemon, index) => ({
    // A cosmetic forme battles as its base species: the engine keeps the look to the host.
    species: pokemon.species.isCosmeticForme ? toID(pokemon.species.baseSpecies) : pokemon.species.id,
    level: pokemon.level,
    // A gender the set left to the battle's draw is not compared.
    gender: team[index].gender || pokemon.species.gender ? pokemon.gender || 'N' : null,
    max_hp: pokemon.maxhp,
    stats: pokemon.storedStats,
    ability: pokemon.baseAbility,
    item: pokemon.item,
    moves: pokemon.baseMoveSlots.map(slot => [slot.id, slot.maxpp]),
    types: pokemon.getTypes(),
    tera_type: battle.gen >= 9 ? pokemon.teraType : null,
    hp_type: pokemon.hpType,
    happiness: pokemon.happiness,
  }));
}

// The team choices tried per format (`team <slots>`, 1-based; `default` for none): the leads,
// a whole order, a longer one cut to the picks, a shorter one completed, a slot twice, a slot
// the team does not have.
const CHOICES = ['', '2', '61', '56', '3142', '654321', '123456', '12345677', '11', '1123', '7'];

/// What each side sees at Team Preview (the `|poke|` lines, `|teampreview|N`), the open team
/// sheets where the format has them, and each team choice's result: the registered slots
/// brought, in order, or Showdown's refusal. P1 registers `pair[0]`, P2 `pair[1]`.
function preview(format, teams) {
  const make = () => {
    const battle = new Battle({formatid: format.id, seed: [5, 6, 7, 8], strictChoices: true});
    battle.setPlayer('p1', {name: 'P1', team: Teams.pack(teams[0])});
    battle.setPlayer('p2', {name: 'P2', team: Teams.pack(teams[1])});
    return battle;
  };
  const battle = make();
  if (!battle.p1.activeRequest?.teamPreview) return null;
  const dex = Dex.forFormat(format);
  const lines = battle.log.map(line => line.split('|'));
  // Each `|poke|` line's details and item flag.
  const poke = side => lines.filter(l => l[1] === 'poke' && l[2] === side).map(l => [l[3], l[4]]);
  const drawn = team => team.map(set => !set.gender && !dex.species.get(set.species).gender);
  const out = {
    teampreview: lines.filter(l => l[1] === 'teampreview').map(l => l[2] ?? null)[0] ?? null,
    poke: {p1: poke('p1'), p2: poke('p2')},
    drawn: {p1: drawn(teams[0]), p2: drawn(teams[1])},
    sheets: null,
    choices: [],
  };
  const table = Dex.formats.getRuleTable(format);
  if (table.has('openteamsheets') || table.has('forceopenteamsheets')) {
    const from = battle.log.length;
    battle.showOpenTeamSheets();
    const shown = battle.log.slice(from).map(line => line.split('|')).filter(l => l[1] === 'showteam');
    out.sheets = Object.fromEntries(shown.map(l => [l[2], Teams.unpack(l.slice(3).join('|'))]));
  }
  for (const choice of CHOICES) {
    const trial = make();
    try {
      trial.makeChoices(choice ? `team ${choice}` : 'default', 'default');
      out.choices.push({choice, brought: trial.p1.pokemon.map(p => trial.p1.team.indexOf(p.set))});
    } catch (error) {
      out.choices.push({choice, error: String(error.message).replace(/^\[Invalid choice\] /, '')});
    }
  }
  return out;
}

// Team Preview's hidden formes, unvalidated: the species whose forme the preview hides, the
// heroes short of their Crowned forme, Greninja-Bond and Rockruff-Dusk as their base species, a
// shiny one, one at level 100.
const TARGETED = {
  gen9ubers: [
    {species: 'Zacian', item: 'Rusted Sword', ability: 'Intrepid Sword', moves: ['Iron Head'], level: 100, shiny: true},
    {species: 'Zamazenta-Crowned', item: 'Rusted Shield', ability: 'Dauntless Shield', moves: ['Iron Head'], level: 70},
    {species: 'Urshifu-Rapid-Strike', ability: 'Unseen Fist', moves: ['Surging Strikes'], level: 88, gender: 'M'},
    {species: 'Dudunsparce-Three-Segment', ability: 'Serene Grace', moves: ['Glare'], level: 90, gender: 'F'},
    {species: 'Xerneas', ability: 'Fairy Aura', moves: ['Geomancy'], level: 100},
    {species: 'Urshifu', ability: 'Unseen Fist', moves: ['Wicked Blow'], level: 100, gender: 'F', shiny: true},
  ],
  gen5ou: [
    {species: 'Arceus-Fire', item: 'Flame Plate', ability: 'Multitype', moves: ['Judgment'], level: 100},
    {species: 'Genesect', item: 'Air Mail', ability: 'Download', moves: ['U-turn'], level: 100},
    {species: 'Greninja', ability: 'Torrent', moves: ['Surf'], level: 100, gender: 'F'},
  ],
  gen8ubers: [
    {species: 'Arceus-Water', item: 'Splash Plate', ability: 'Multitype', moves: ['Judgment'], level: 100},
    {species: 'Greninja', item: 'Choice Specs', ability: 'Protean', moves: ['Surf'], level: 100, gender: 'M'},
    {species: 'Zacian-Crowned', item: 'Rusted Sword', ability: 'Intrepid Sword', moves: ['Behemoth Blade'], level: 100},
  ],
  gen7ubers: [
    {species: 'Greninja-Ash', ability: 'Battle Bond', moves: ['Water Shuriken'], level: 100, gender: 'M'},
    {species: 'Greninja', ability: 'Battle Bond', moves: ['Water Shuriken'], level: 90, gender: 'M'},
    {species: 'Silvally-Fire', item: 'Fire Memory', ability: 'RKS System', moves: ['Multi-Attack'], level: 80},
    {species: 'Pumpkaboo-Super', ability: 'Frisk', moves: ['Trick'], level: 50, gender: 'F'},
    {species: 'Rockruff-Dusk', ability: 'Own Tempo', moves: ['Bite'], level: 30, gender: 'M'},
    {species: 'Xerneas-Neutral', ability: 'Fairy Aura', moves: ['Geomancy'], level: 100},
  ],
};
function targetedPreview(formatid, team) {
  const battle = new Battle({formatid, seed: [5, 6, 7, 8]});
  battle.setPlayer('p1', {name: 'P1', team: Teams.pack(team)});
  battle.setPlayer('p2', {name: 'P2', team: Teams.pack(team)});
  return battle.log.map(line => line.split('|')).filter(l => l[1] === 'poke' && l[2] === 'p1')
    .map(l => [l[3], l[4]]);
}

const out = {};
targets.forEach((format, index) => {
  const draws = new LegalTeams(format.id, [index + 1, 17, 29, 41]);
  const size = Math.min(6, Dex.formats.getRuleTable(format).maxTeamSize);
  const teams = [];
  for (let i = 0; i < TEAMS; i++) {
    const team = draws.team(size);
    if (!team) throw new Error(`${format.id}: no legal team drawn`);
    teams.push(team);
  }
  const validator = new TeamValidator(format);
  const normalised = teams.map(team => {
    const copy = JSON.parse(JSON.stringify(team));
    const problems = validator.validateTeam(copy);
    if (problems) throw new Error(`${format.id}: ${problems.join('; ')}`);
    return copy;
  });
  out[format.id] = {
    rules: rulesOf(format, normalised),
    teams: teams.map((team, i) => ({sets: team, built: built(format, normalised[i])})),
    // P1 registers the first team, P2 the second.
    preview: preview(format, normalised.slice(0, 2)),
  };
  process.stderr.write(`${format.id} `);
});
process.stderr.write('\n');

const targeted = Object.fromEntries(Object.entries(TARGETED)
  .map(([formatid, team]) => [formatid, {sets: team, poke: targetedPreview(formatid, team)}]));

fs.writeFileSync(path.join(fixtureDir, 'built.json'), JSON.stringify({
  schema: 1,
  provenance: {engine: '@pkmn/sim', version: VERSION['@pkmn/sim'], seed: 'per format: [index + 1, 17, 29, 41]'},
  formats: out,
  targeted_previews: targeted,
}, null, 1) + '\n');
