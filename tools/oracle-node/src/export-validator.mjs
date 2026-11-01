// The team validator's oracles (M9.2, DESIGN.md §45.3): Showdown's `TeamValidator` asked about
// one thing at a time, under every target format.
//
// M9.2b: `checkSpecies` / `checkItem` / `checkMove` / `checkAbility` / `checkNature` for every
// species, item, move, ability and nature of the format's dex — the problem, when there is one,
// and (for a sample) what the check records in `setHas`. The formats of one mod mostly agree:
// each mod keeps the answer most of its formats give (`mods`), each format only where it differs
// (`formats`, `null` for no problem).
//
// Usage:  node src/export-validator.mjs ../../tests/fixtures/formats
import fs from 'node:fs';
import path from 'node:path';
import {Dex, TeamValidator} from '@pkmn/sim';

const fixtureDir = process.argv[2] ?? path.join('..', '..', 'tests', 'fixtures', 'formats');
const VERSION = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url))).dependencies;
const TARGET = /^gen[1-9](ou|ubers|doublesou|vgc\d+.*)$/;
const targets = Dex.formats.all()
  .filter(f => f.effectType === 'Format' && TARGET.test(f.id))
  .sort((a, b) => a.id.localeCompare(b.id));

// The name a set of Pikachu gets: what the item, move, ability and nature messages show.
const SET_NAME = 'Pikachu';
const sampled = new Set();
const checks = {};
function rulings(format, sample) {
  const validator = new TeamValidator(format);
  const dex = validator.dex;
  const data = dex.data;
  const out = {mod: format.mod, species: {}, pairs: {}, items: {}, moves: {}, abilities: {}, natures: {}, set_has: {}};
  const record = (table, key, problem, setHas, index) => {
    out[table][key] = problem || null;
    if (sample && index % 7 === 0) out.set_has[`${table}:${key}`] = Object.keys(setHas).sort();
  };

  Object.keys(data.Pokedex).sort().forEach((id, index) => {
    const species = dex.species.get(id);
    const set = {species: species.name, name: species.baseSpecies, ability: '', moves: []};
    const setHas = {};
    record('species', id, validator.checkSpecies(set, species, species, setHas), setHas, index);
  });
  // A species validated as another (`getValidationSpecies`): its Mega Stone's Mega, the
  // Primals, the Crowned heroes, Mega Rayquaza; Battle Bond Greninja, Own Tempo Rockruff;
  // Gigantamax flagged on a species that can and one that cannot.
  const pairs = [];
  for (const id of Object.keys(data.Items).sort()) {
    const item = dex.items.get(id);
    for (const [base, mega] of Object.entries(item.megaStone || {})) pairs.push([base, mega, '', false]);
  }
  pairs.push(['Groudon', 'Groudon-Primal', '', false], ['Kyogre', 'Kyogre-Primal', '', false],
    ['Zacian', 'Zacian-Crowned', '', false], ['Zamazenta', 'Zamazenta-Crowned', '', false],
    ['Rayquaza', 'Rayquaza-Mega', '', false], ['Greninja', 'Greninja', 'Battle Bond', false],
    ['Rockruff', 'Rockruff', 'Own Tempo', false], ['Bulbasaur', 'Bulbasaur', '', true]);
  for (const id of Object.keys(data.Pokedex).sort()) {
    const species = dex.species.get(id);
    if (species.canGigantamax) pairs.push([species.name, species.name, '', true]);
  }
  pairs.forEach(([base, tier, ability, gigantamax], index) => {
    const species = dex.species.get(base);
    const tierSpecies = dex.species.get(tier);
    const set = {species: species.name, name: species.baseSpecies, ability, gigantamax, moves: []};
    const setHas = {};
    const problem = validator.checkSpecies(set, species, tierSpecies, setHas);
    // `base|tier|ability|gigantamax`, by id.
    const key = [species.id, tierSpecies.id, Dex.toID(ability), gigantamax ? 'gmax' : ''].join('|');
    record('pairs', key, problem, setHas, index);
  });

  const things = [
    ['items', data.Items, id => dex.items.get(id), (set, thing, setHas) => validator.checkItem(set, thing, setHas)],
    ['moves', data.Moves, id => dex.moves.get(id), (set, thing, setHas) => validator.checkMove(set, thing, setHas)],
    ['abilities', data.Abilities, id => dex.abilities.get(id), (set, thing, setHas) => validator.checkAbility(set, thing, setHas)],
    ['natures', data.Natures, id => dex.natures.get(id), (set, thing, setHas) => validator.checkNature(set, thing, setHas)],
  ];
  for (const [table, raw, get, check] of things) {
    // An empty id too: no item (`item:noitem`), no ability, no nature (a move is never empty:
    // `validateSet` drops empty move names before it checks any).
    [...(table === 'moves' ? [] : ['']), ...Object.keys(raw).sort()].forEach((id, index) => {
      const set = {species: 'Pikachu', name: SET_NAME, ability: '', moves: []};
      const setHas = {};
      record(table, id, check(set, get(id), setHas), setHas, index);
    });
  }
  return out;
}
for (const format of targets) {
  // One format per mod records `setHas` for every 7th thing.
  checks[format.id] = rulings(format, !sampled.has(format.mod));
  sampled.add(format.mod);
}

// M9.2c-1: `validateForme` per mod (it reads the dex alone) on sets made to reach each branch;
// `getValidationSpecies` per format (it reads the rule table). A set is
// [species, item, ability, moves, ivs?]; an answer [problems, species, ability, moves], or 0
// when nothing changed and nothing is wrong.
const forme = {};
const validationSpecies = {};
const nameOf = (dex, id) => dex.species.get(id).name;
for (const format of targets) {
  const validator = new TeamValidator(format);
  const dex = validator.dex;
  const data = dex.data;
  if (!forme[format.mod]) {
    const sets = [];
    const keys = Object.keys(data.Pokedex).sort();
    for (const key of keys) {
      const s = dex.species.get(key);
      sets.push([s.name, '', '', []]);
      for (const name of s.cosmeticFormes || []) sets.push([name, '', '', []]);
      if (s.requiredItems) {
        sets.push([s.name, s.requiredItems[0], '', []], [s.name, 'Leftovers', '', []]);
      }
      if (s.requiredMove) sets.push([s.name, '', '', [s.requiredMove]], [s.name, '', '', [Dex.toID(s.requiredMove)]]);
      if (s.requiredAbility) sets.push([s.name, '', s.requiredAbility, []], [s.name, '', 'Pressure', []]);
    }
    for (const key of Object.keys(data.Items).sort()) {
      const item = dex.items.get(key);
      if (item.forcedForme) sets.push([dex.species.get(item.forcedForme).baseSpecies, item.name, '', []]);
    }
    sets.push(
      ['Necrozma-Ultra', 'Ultranecrozium Z', '', ['Sunsteel Strike']],
      ['Necrozma-Ultra', 'Ultranecrozium Z', '', ['Moongeist Beam']],
      ['Necrozma-Ultra', 'Ultranecrozium Z', '', ['Sunsteel Strike', 'Moongeist Beam']],
      ['Necrozma-Ultra', 'Leftovers', '', []],
      ['Zygarde-Complete', '', '', []], ['Zygarde', 'Zygardite', 'Aura Break', []],
      ['Zygarde', 'Zygardite', 'Power Construct', []], ['Terapagos-Stellar', '', '', []],
      ['Pikachu-Cosplay', '', '', ['meteormash']], ['Pikachu-Cosplay', '', '', ['Meteor Mash']],
      ['Pikachu-Cosplay', '', '', ['flyingpress', 'iciclecrash']], ['Keldeo', '', '', ['Secret Sword']],
      ['Zacian-Crowned', 'Rusted Sword', '', ['Behemoth Blade', 'Crunch']],
      ['Zamazenta-Crowned', 'Rusted Shield', '', ['Behemoth Bash']],
      ['Hoopa', '', '', ['Hyperspace Fury']], ['Hoopa-Unbound', '', '', ['Hyperspace Hole']],
      ['Greninja', '', 'Battle Bond', []], ['Rockruff', '', 'Own Tempo', []],
      ['Unown', '', '', [], {hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31}],
      ['Unown-B', '', '', [], {hp: 31, atk: 4, def: 6, spa: 2, spd: 2, spe: 8}],
      ['Unown-Z', '', '', [], {hp: 31, atk: 30, def: 30, spa: 30, spd: 30, spe: 30}],
      ['Unown-C', '', '', [], {hp: 31, atk: 0, def: 0, spa: 0, spd: 0, spe: 0}],
      ['Arceus', 'Flame Plate', '', []], ['Arceus-Fire', 'Splash Plate', '', []], ['Arceus-Fire', '', '', []],
      ['Silvally-Fire', 'Water Memory', '', []], ['Giratina-Origin', '', '', []],
      ['Meloetta-Pirouette', '', '', []], ['Darmanitan-Zen', '', 'Sheer Force', []],
      ['Charizard-Mega-X', 'Charizardite Y', '', []], ['Kyogre-Primal', 'Blue Orb', '', []],
      ['Nosuchmon', '', '', []],
    );
    const answers = {};
    for (const [species, item, ability, moves, ivs] of sets) {
      const set = {species, item, ability, moves: [...moves], name: '', ivs: ivs ?? {hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31}};
      const problems = validator.validateForme(set);
      const changed = set.species !== species || set.ability !== ability || JSON.stringify(set.moves) !== JSON.stringify(moves);
      const key = JSON.stringify([species, item, ability, moves, ivs ?? null]);
      answers[key] = problems.length || changed ? [problems, set.species, set.ability, set.moves] : 0;
    }
    forme[format.mod] = answers;
  }
  // getValidationSpecies: [outOfBattle id, tier id].
  const vsets = [];
  for (const key of Object.keys(data.Items).sort()) {
    const item = dex.items.get(key);
    for (const base of Object.keys(item.megaStone || {})) vsets.push([base, item.name, '', []]);
  }
  vsets.push(['Groudon', 'Red Orb', '', []], ['Kyogre', 'Blue Orb', '', []], ['Groudon', 'Blue Orb', '', []],
    ['Rayquaza', '', '', ['Dragon Ascent']], ['Rayquaza', '', '', []], ['Zacian', 'Rusted Sword', '', []],
    ['Zamazenta', 'Rusted Shield', '', []], ['Zacian', 'Rusted Shield', '', []],
    ['Greninja', '', 'Battle Bond', []], ['Rockruff', '', 'Own Tempo', []], ['Charizard', 'Leftovers', '', []]);
  validationSpecies[format.id] = Object.fromEntries(vsets.map(([species, item, ability, moves]) => {
    const {outOfBattleSpecies, tierSpecies} = validator.getValidationSpecies({species, item, ability, moves});
    return [JSON.stringify([species, item, ability, moves]), [outOfBattleSpecies.id, tierSpecies.id]];
  }));
}

// M9.2c-2: `validateStats` per format on sets made to reach each branch, with the sources as
// no learnset narrows them (`allSources`) and no Pokémon GO origin (`null`): the problems, and
// the set's EVs, IVs, Hidden Power type, gender and nature as it comes back.
const STATS_SPECIES = ['Pikachu', 'Mewtwo', 'Diancie', 'Marowak', 'Snorlax', 'Chansey', 'Magnemite',
  'Manaphy', 'Koraidon', 'Unown', 'Zapdos', 'Eevee'];
const full = v => ({hp: v, atk: v, def: v, spa: v, spd: v, spe: v});
const STATS_CASES = [
  {}, {evs: full(0)}, {evs: {...full(0), atk: 252, spe: 252, hp: 4}}, {evs: {...full(0), atk: 252, spe: 252, hp: 6}},
  {evs: {...full(0), atk: 256}}, {evs: {...full(0), hp: -4}}, {evs: full(100)}, {level: 50, evs: {...full(0), atk: 252, spe: 252, hp: 4}},
  {moves: ['Hidden Power Fire']}, {moves: ['Hidden Power Fire'], ivs: full(0)}, {moves: ['Hidden Power'], ivs: full(30)},
  {hpType: 'Ice'}, {hpType: 'Fighting'}, {hpType: 'Dark', ivs: {...full(31), atk: 30}}, {hpType: 'Fire', ivs: {...full(31), spe: 29}},
  {ivs: full(0)}, {ivs: {...full(0), hp: 31, atk: 31, def: 31}}, {shiny: true, ivs: full(0)}, {shiny: true},
  {ivs: {...full(30), hp: 28}}, {ivs: {...full(30), spd: 28}}, {gender: 'F', ivs: full(30)}, {gender: 'M', ivs: {...full(30), atk: 2}},
  {shiny: true, ivs: {hp: 0, atk: 20, def: 20, spa: 20, spd: 20, spe: 20}}, {ivs: {hp: 0, atk: 20, def: 20, spa: 20, spd: 20, spe: 20}},
  {evs: {...full(0), spa: 252, spd: 4}}, {level: 99, moves: ['Hidden Power Grass'], ivs: full(31)},
  {item: 'Thick Club', moves: ['Swords Dance'], evs: full(252)},
  // Gen 2's gender at the Atk DV threshold: 8 for an even ratio, 2 for Eevee's 7:1.
  {ivs: {...full(30), atk: 16}}, {gender: 'F', ivs: {...full(30), atk: 16}}, {ivs: {...full(30), atk: 4}},
  {gender: 'F', ivs: {...full(30), atk: 4}},
];
const statsCases = {};
for (const format of targets) {
  const validator = new TeamValidator(format);
  const dex = validator.dex;
  const answers = {};
  for (const name of STATS_SPECIES) {
    const species = dex.species.get(name);
    if (!species.exists || species.isNonstandard === 'Future') continue;
    for (const c of STATS_CASES) {
      const set = {species: species.name, name: '', item: c.item ?? '', ability: '', moves: c.moves ?? [],
        level: c.level ?? 100, nature: '', gender: c.gender ?? '', shiny: !!c.shiny, hpType: c.hpType ?? ''};
      if (c.evs) set.evs = {...c.evs};
      if (c.ivs) set.ivs = {...c.ivs};
      const problems = validator.validateStats(set, species, validator.allSources(species), null);
      answers[JSON.stringify([species.name, c])] = [problems, set.evs, set.ivs, set.hpType || '', set.gender || '', set.nature || ''];
    }
  }
  statsCases[format.id] = answers;
}

// M9.2c-3: `validateSet` on the learnset seam's empty side (DESIGN.md §45.3): every move
// learnable, no species event-only, no Pokémon GO origin, no Gen 1 encounter. Per mod, sets
// drawn from a seeded generator over legal, banned, nonexistent and malformed values, run under
// each of the mod's formats: the problems and the set as it comes back.
function mulberry32(seed) {
  return () => {
    seed |= 0; seed = (seed + 0x6D2B79F5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const SETS_PER_MOD = 300;
function drawSets(dex, seed) {
  const random = mulberry32(seed);
  const pick = list => list[Math.floor(random() * list.length)];
  const chance = p => random() < p;
  const data = dex.data;
  const speciesNames = Object.keys(data.Pokedex).map(id => dex.species.get(id).name);
  const itemNames = Object.keys(data.Items).map(id => dex.items.get(id).name);
  const moveNames = Object.keys(data.Moves).map(id => dex.moves.get(id).name);
  const abilityNames = Object.keys(data.Abilities).map(id => dex.abilities.get(id).name);
  const natureNames = Object.keys(data.Natures).map(id => dex.natures.get(id).name);
  const typeNames = Object.keys(data.TypeChart).map(id => dex.types.get(id).name);
  const special = ['Fissure', 'Spore', 'Mean Look', 'Mud-Slap', 'Hidden Power Fire', 'Secret Sword', 'Swords Dance', 'Dragon Ascent'];
  const stat = f => ({hp: f(), atk: f(), def: f(), spa: f(), spd: f(), spe: f()});
  const sets = [];
  for (let i = 0; i < SETS_PER_MOD; i++) {
    const speciesName = chance(0.9) ? pick(speciesNames)
      : pick(['Nosuchmon', 'mence', 'Charizard-Gmax', 'Gastrodon-East', 'Pikachu-Cosplay', 'Greninja-Ash']);
    const species = dex.species.get(speciesName);
    const abilities = Object.values(species.abilities || {});
    const set = {species: speciesName};
    set.name = chance(0.6) ? '' : pick([speciesName, pick(speciesNames), 'A Very Long Nickname Indeed', 'Bob|the [Mon]', '  Spaced  ', 'Pikachu']);
    set.item = chance(0.3) ? '' : chance(0.9) ? pick(itemNames) : pick(['Nosuchitem', 'cb', 'Oran']);
    set.ability = chance(0.2) ? '' : chance(0.7) && abilities.length ? pick(abilities) : chance(0.8) ? pick(abilityNames) : 'Nosuchability';
    const count = Math.floor(random() * 6);
    set.moves = [];
    for (let m = 0; m < count; m++) {
      set.moves.push(chance(0.75) ? pick(moveNames) : chance(0.85) ? pick(special) : pick(['', 'Nosuchmove', set.moves[0] || 'Tackle']));
    }
    set.nature = chance(0.3) ? '' : chance(0.95) ? pick(natureNames) : 'Nosuchnature';
    set.level = chance(0.5) ? 0 : pick([1, 5, 50, 99, 100, 101]);
    if (chance(0.6)) set.evs = chance(0.2) ? stat(() => 0) : stat(() => pick([0, 4, 84, 252, 255, 256, -4]));
    if (chance(0.5)) set.ivs = chance(0.3) ? stat(() => 31) : stat(() => Math.floor(random() * 32));
    set.gender = pick(['', '', 'M', 'F', 'X']);
    set.shiny = chance(0.1);
    if (chance(0.1)) set.happiness = chance(0.5) ? Math.floor(random() * 256) : 'abc';
    set.hpType = chance(0.85) ? '' : pick([...typeNames, 'Bird']);
    set.teraType = chance(0.7) ? '' : pick([...typeNames, 'Bird']);
    set.gigantamax = chance(0.1);
    sets.push(set);
  }
  // Branches a draw rarely reaches: the Gen 4 Griseous Orb, Accuracy Trap Clause's
  // combinations, a level 100 set where the format's maximum is lower.
  const base = {name: '', nature: '', level: 0, gender: '', shiny: false, hpType: '', teraType: '', gigantamax: false};
  sets.push(
    {...base, species: 'Pikachu', item: 'Griseous Orb', ability: 'Static', moves: ['Thunderbolt']},
    {...base, species: 'Giratina', item: 'Griseous Orb', ability: 'Pressure', moves: ['Shadow Ball']},
    {...base, species: 'Dugtrio', item: '', ability: 'Arena Trap', moves: ['Mud-Slap', 'Earthquake']},
    {...base, species: 'Gengar', item: '', ability: 'Levitate', moves: ['Mean Look', 'Sand Attack']},
    {...base, species: 'Gengar', item: '', ability: 'Levitate', moves: ['Mean Look', 'Hypnosis']},
    {...base, species: 'Snorlax', item: '', ability: 'Thick Fat', moves: ['Body Slam'], level: 100},
  );
  return sets;
}
const setCases = {};
const setOutputs = {};
for (const format of targets) {
  const validator = new TeamValidator(format);
  const dex = validator.dex;
  validator.validateMoves = () => [];
  validator.getEventOnlyData = () => null;
  validator.validatePokemonGo = () => null;
  dex.species.getLearnsetData = () => ({});
  if (!setCases[format.mod]) setCases[format.mod] = drawSets(dex, 9000 + parseInt(format.mod.slice(3)));
  setOutputs[format.id] = setCases[format.mod].map(input => {
    const set = JSON.parse(JSON.stringify(input));
    const problems = validator.validateSet(set, {}) || [];
    return [problems, {species: set.species, name: set.name, item: set.item, ability: set.ability, moves: set.moves,
      nature: set.nature, gender: set.gender, level: set.level, evs: set.evs, ivs: set.ivs, hpType: set.hpType ?? '',
      teraType: set.teraType ?? '', shiny: !!set.shiny}];
  });
  delete dex.species.getLearnsetData;
}
// The mods' sets under custom formats: a maximum level under 100, a minimum level, Accuracy
// Trap Clause where no target has it.
const SET_CUSTOM = [['gen9ou', ['Max Level = 50']], ['gen4ou', ['Min Level = 10']], ['gen7ou', ['Accuracy Trap Clause']]];
const setCustom = [];
for (const [id, rules] of SET_CUSTOM) {
  const format = Dex.formats.get(Dex.formats.validate(`${id}@@@${rules.join(',')}`), true);
  const validator = new TeamValidator(format);
  const dex = validator.dex;
  validator.validateMoves = () => [];
  validator.getEventOnlyData = () => null;
  validator.validatePokemonGo = () => null;
  dex.species.getLearnsetData = () => ({});
  const outputs = setCases[format.mod].map(input => {
    const set = JSON.parse(JSON.stringify(input));
    const problems = validator.validateSet(set, {}) || [];
    return [problems, {species: set.species, name: set.name, item: set.item, ability: set.ability, moves: set.moves,
      nature: set.nature, gender: set.gender, level: set.level, evs: set.evs, ivs: set.ivs, hpType: set.hpType ?? '',
      teraType: set.teraType ?? '', shiny: !!set.shiny}];
  });
  delete dex.species.getLearnsetData;
  setCustom.push({format: id, rules, mod: format.mod, outputs});
}
const setMods = {};
const setFormats = {};
for (const mod of Object.keys(setCases)) {
  const ofMod = targets.filter(f => f.mod === mod);
  const common = setCases[mod].map((_, index) => {
    const votes = new Map();
    for (const f of ofMod) {
      const answer = JSON.stringify(setOutputs[f.id][index]);
      votes.set(answer, (votes.get(answer) || 0) + 1);
    }
    return JSON.parse([...votes].sort((a, b) => b[1] - a[1])[0][0]);
  });
  setMods[mod] = {sets: setCases[mod], common};
  for (const f of ofMod) {
    const diff = {};
    setOutputs[f.id].forEach((answer, index) => {
      if (JSON.stringify(answer) !== JSON.stringify(common[index])) diff[index] = answer;
    });
    setFormats[f.id] = {mod, diff};
  }
}

// M9.2d: `validateTeam` (`baseValidateTeam`) on the learnset seam's empty side. Per mod, a pool
// of the mod's sets (`sets.json`'s, then sets made for the team rules), and teams drawn from it
// with a seeded generator, plus teams made for each rule; the problems under each format.
const TEAM_EXTRAS = [
  ['Kyurem-Black', ['Ice Beam']], ['Kyurem-White', ['Ice Beam']], ['Necrozma-Dusk-Mane', ['Psychic']],
  ['Necrozma-Dawn-Wings', ['Psychic']], ['Calyrex-Ice', ['Psychic']], ['Calyrex-Shadow', ['Psychic']],
  ['Deoxys-Attack', ['Psychic']], ['Deoxys-Defense', ['Psychic']], ['Deoxys', ['Psychic']],
  ['Pikachu-Starter', ['Thunderbolt']], ['Eevee-Starter', ['Tackle']],
  ['Mewtwo', ['Psychic']], ['Lugia', ['Aeroblast']], ['Kyogre', ['Surf']], ['Groudon', ['Earthquake']],
  ['Orbeetle', ['Hypnosis'], {gigantamax: true}], ['Orbeetle', ['Hypnosis']], ['Gengar', ['Hypnosis']],
  ['Ninjask', ['Baton Pass', 'Swords Dance']], ['Smeargle', ['batonpass', 'Agility']],
  ['Vaporeon', ['Baton Pass', 'Acid Armor', 'Agility']], ['Scolipede', ['Baton Pass'], {ability: 'Speed Boost'}],
  ['Celebi', ['Baton Pass', 'Calm Mind']], ['Umbreon', ['Baton Pass', 'Mean Look']],
  ['Pikachu', ['Thunderbolt'], {name: 'Bob'}], ['Raichu', ['Thunderbolt'], {name: 'Bob'}],
  ['Snorlax', ['Body Slam'], {item: 'Leftovers'}], ['Chansey', ['Soft-Boiled'], {item: 'Leftovers'}],
  ['Blissey', ['Soft-Boiled'], {item: 'Leftovers'}], ['Pikachu', ['Surf']], ['Pikachu-Alola', ['Surf']],
  // Spore never misses at 100: Gravity Sleep Clause does not count it.
  ['Orbeetle', ['Spore'], {gigantamax: true}],
];
function teamExtras(dex) {
  const extras = [];
  for (const [species, moves, more] of TEAM_EXTRAS) {
    const s = dex.species.get(species);
    if (!s.exists) continue;
    extras.push({species, name: '', item: '', ability: s.abilities['0'] || '', moves, nature: '', level: 0,
      gender: '', shiny: false, hpType: '', teraType: '', gigantamax: false, ...(more || {})});
  }
  return extras;
}
const teamPools = {};
const teamLists = {};
const teamOutputs = {};
for (const format of targets) {
  const validator = new TeamValidator(format);
  const dex = validator.dex;
  validator.validateMoves = () => [];
  validator.getEventOnlyData = () => null;
  validator.validatePokemonGo = () => null;
  dex.species.getLearnsetData = () => ({});
  if (!teamPools[format.mod]) {
    const extras = teamExtras(dex);
    const pool = [...setCases[format.mod], ...extras];
    const random = mulberry32(7000 + parseInt(format.mod.slice(3)));
    const teams = [];
    for (let i = 0; i < 150; i++) {
      const size = Math.floor(random() * 8);
      teams.push(Array.from({length: size}, () => Math.floor(random() * pool.length)));
    }
    // Teams made for each rule: the extras in pairs and in one team.
    const base = setCases[format.mod].length;
    for (let i = 0; i + 1 < extras.length; i++) teams.push([base + i, base + i + 1]);
    teams.push(extras.map((_, i) => base + i).slice(0, 6));
    teams.push([base, base, base]);
    // Two passers of one boost each, one writing `batonpass`: One Boost Passer Clause reads the
    // move as written. Gigantamax Orbeetle with Spore alone.
    const at = (species, move) => extras.findIndex(e => e.species === species && e.moves.includes(move));
    for (const pair of [[at('Celebi', 'Calm Mind'), at('Smeargle', 'Agility')], [at('Orbeetle', 'Spore')]]) {
      if (pair.every(i => i >= 0)) teams.push(pair.map(i => base + i));
    }
    teamPools[format.mod] = extras;
    teamLists[format.mod] = teams;
  }
  const pool = [...setCases[format.mod], ...teamPools[format.mod]];
  teamOutputs[format.id] = teamLists[format.mod].map(team => {
    const sets = team.map(i => JSON.parse(JSON.stringify(pool[i])));
    return validator.validateTeam(sets) || [];
  });
  delete dex.species.getLearnsetData;
}
const teamMods = {};
const teamFormats = {};
for (const mod of Object.keys(teamLists)) {
  const ofMod = targets.filter(f => f.mod === mod);
  const common = teamLists[mod].map((_, index) => {
    const votes = new Map();
    for (const f of ofMod) {
      const answer = JSON.stringify(teamOutputs[f.id][index]);
      votes.set(answer, (votes.get(answer) || 0) + 1);
    }
    return JSON.parse([...votes].sort((a, b) => b[1] - a[1])[0][0]);
  });
  teamMods[mod] = {extras: teamPools[mod], teams: teamLists[mod], common};
  for (const f of ofMod) {
    const diff = {};
    teamOutputs[f.id].forEach((answer, index) => {
      if (JSON.stringify(answer) !== JSON.stringify(common[index])) diff[index] = answer;
    });
    teamFormats[f.id] = {mod, diff};
  }
}

// M9.3: `validateSet` with the learnsets, the egg-move father search and Pokémon GO. Per mod,
// sets whose moves come mostly from the species' own family learnset (level-up, egg, event,
// TM, tutor), event-only species with their events' moves, Sketch, the HM transfers, egg move
// combinations, Pokémon GO origins; under each of the mod's formats.
const LEARNSET_SETS_PER_MOD = 250;
const EVENT_ONLY = ['Mew', 'Celebi', 'Jirachi', 'Deoxys', 'Manaphy', 'Darkrai', 'Shaymin', 'Arceus',
  'Victini', 'Keldeo', 'Meloetta', 'Genesect', 'Diancie', 'Hoopa', 'Volcanion', 'Magearna', 'Marshadow',
  'Zeraora', 'Meltan', 'Zarude', 'Pecharunt'];
function drawLearnsetSets(dex, seed) {
  const random = mulberry32(seed);
  const pick = list => list[Math.floor(random() * list.length)];
  const chance = p => random() < p;
  const data = dex.data;
  const withLearnset = Object.keys(data.Pokedex).filter(id => dex.species.getFullLearnset(id).length);
  const moveNames = Object.keys(data.Moves).map(id => dex.moves.get(id).name);
  const itemNames = Object.keys(data.Items).map(id => dex.items.get(id).name);
  const natureNames = Object.keys(data.Natures).map(id => dex.natures.get(id).name);
  const stat = f => ({hp: f(), atk: f(), def: f(), spa: f(), spd: f(), spe: f()});
  const sets = [];
  for (let i = 0; i < LEARNSET_SETS_PER_MOD; i++) {
    const eventOnly = chance(0.15);
    const species = dex.species.get(eventOnly ? pick(EVENT_ONLY) : pick(withLearnset));
    const family = dex.species.getFullLearnset(species.id);
    const familyMoves = [...new Set(family.flatMap(({learnset}) => Object.keys(learnset)))];
    const events = dex.species.getLearnsetData(species.id).eventData || [];
    const eventMoves = events.length ? pick(events).moves || [] : [];
    const count = 1 + Math.floor(random() * 4);
    const moves = [];
    for (let m = 0; m < count; m++) {
      const id = eventOnly && eventMoves.length && chance(0.6) ? pick(eventMoves)
        : chance(0.8) && familyMoves.length ? pick(familyMoves) : Dex.toID(pick(moveNames));
      const name = dex.moves.get(id).name;
      if (!moves.includes(name)) moves.push(name);
    }
    const abilities = Object.values(species.abilities || {});
    sets.push({
      species: species.name, name: '', item: chance(0.3) ? pick(itemNames) : '',
      ability: abilities.length ? pick(abilities) : '', moves,
      nature: chance(0.5) ? pick(natureNames) : '',
      level: chance(0.4) ? 0 : pick([1, 5, 10, 20, 30, 40, 50, 60, 70, 80, 100]),
      gender: pick(['', '', 'M', 'F']), shiny: chance(0.1), hpType: '', teraType: '', gigantamax: false,
      ...(chance(0.3) ? {ivs: chance(0.5) ? stat(() => 31) : stat(() => Math.floor(random() * 32))} : {}),
      ...(chance(0.3) ? {evs: stat(() => pick([0, 4, 84, 252]))} : {}),
    });
  }
  const base = {name: '', item: '', nature: '', level: 0, gender: '', shiny: false, hpType: '', teraType: '', gigantamax: false};
  // Branches a draw rarely reaches, found in the data: an Alolan evolution with a move only its
  // prevolution learns from Gen 8; Gen 9's HOME relearner (a level-up move learned earlier in
  // Brilliant Diamond / Shining Pearl or Legends: Arceus); the learnset domain (a prevolution's
  // old-generation move with the species' own new one); an underleveled evolution whose
  // prevolution has events.
  const targeted = [];
  const own = id => dex.species.getLearnsetData(id).learnset || {};
  const gens = list => list.map(source => parseInt(source.charAt(0)));
  for (const id of withLearnset) {
    const sp = dex.species.get(id);
    if (sp.isNonstandard) continue;
    const ability = Object.values(sp.abilities || {})[0] || '';
    const prevo = sp.prevo ? dex.species.get(sp.prevo) : null;
    if (sp.evoRegion === 'Alola' && prevo && targeted.filter(t => t.why === 'alola').length < 4) {
      const mine = own(id);
      const theirs = own(prevo.id);
      const only = Object.keys(theirs).find(m => !mine[m] && gens(theirs[m]).some(g => g >= 8 && g <= dex.gen));
      if (only) targeted.push({why: 'alola', set: {...base, species: sp.name, ability, moves: [dex.moves.get(only).name]}});
    }
    // HOME: a species that cannot hatch (no egg move to fall back on), a move SV teaches it only
    // by level-up.
    if (dex.gen === 9 && !['nincada', 'spinda'].includes(id) && (sp.gender === 'N' || !sp.canHatch) &&
        targeted.filter(t => t.why === 'home').length < 6) {
      for (const external of ['gen8bdsp', 'gen8legends']) {
        const other = Dex.mod(external);
        if (other.species.get(id).isNonstandard) continue;
        const theirs = other.species.getLearnsetData(id).learnset || {};
        for (const [move, sources] of Object.entries(own(id))) {
          if (sources.some(source => source.startsWith('9') && !source.startsWith('9L'))) continue;
          const sv = sources.find(source => source.startsWith('9L'));
          const old = (theirs[move] || []).find(source => source.startsWith('8L'));
          if (sv && old && parseInt(old.slice(2)) < parseInt(sv.slice(2)) && parseInt(old.slice(2)) >= 1) {
            targeted.push({why: 'home', set: {...base, species: sp.name, ability, level: parseInt(old.slice(2)), moves: [dex.moves.get(move).name]}});
            break;
          }
        }
        if (targeted.filter(t => t.why === 'home').length >= 6) break;
      }
    }
    // The domain: first a move only the species itself learns, in an old generation; then one
    // only its prevolution learns, later and not as a Gen 8+ egg move.
    if (prevo && targeted.filter(t => t.why === 'domain').length < 8) {
      const mine = own(id);
      const theirs = own(prevo.id);
      const ownOld = Object.keys(mine).find(m => !theirs[m] && Math.max(...gens(mine[m])) <= Math.min(6, dex.gen) && Math.max(...gens(mine[m])) >= 3);
      const prevoNew = Object.keys(theirs).find(m => !mine[m] && theirs[m].every(source => source.charAt(1) !== 'E') &&
        Math.min(...gens(theirs[m])) >= 7 && Math.min(...gens(theirs[m])) <= dex.gen);
      if (ownOld && prevoNew) {
        targeted.push({why: 'domain', set: {...base, species: sp.name, ability, level: 100, moves: [dex.moves.get(ownOld).name, dex.moves.get(prevoNew).name]}});
      }
    }
    // Underleveled: a species not evolved by level whose prevolution is, and has events — the
    // prevolution's events are where Showdown looks.
    if (prevo && !sp.evoLevel && prevo.evoLevel && targeted.filter(t => t.why === 'underleveled').length < 6) {
      const events = dex.species.getLearnsetData(prevo.id).eventData || [];
      const event = events.find(e => e.level && e.level < prevo.evoLevel && e.generation <= dex.gen);
      if (event) {
        targeted.push({why: 'underleveled', set: {...base, species: sp.name, ability, level: event.level,
          moves: (event.moves || []).slice(0, 2).map(m => dex.moves.get(m).name)}});
      }
    }
  }
  // Egg move combinations (Gen 3-7: the father search) and Pokémon GO origins (Gen 8-9).
  for (const id of withLearnset) {
    if (targeted.filter(t => t.why === 'egg').length >= 10 || dex.gen < 3 || dex.gen > 7) break;
    const sp = dex.species.get(id);
    if (sp.isNonstandard || sp.prevo) continue;
    const eggMoves = Object.entries(own(id)).filter(([, sources]) =>
      sources.some(source => source.charAt(1) === 'E' && parseInt(source.charAt(0)) <= Math.min(5, dex.gen))).map(([m]) => m);
    if (eggMoves.length >= 3 && random() < 0.2) {
      targeted.push({why: 'egg', set: {...base, species: sp.name, ability: Object.values(sp.abilities)[0] || '', level: 100,
        moves: eggMoves.slice(0, 2 + Math.floor(random() * 2)).map(m => dex.moves.get(m).name)}});
    }
  }
  if (dex.gen >= 8) {
    const goSpecies = Object.keys(Dex.data.PokemonGoData).filter(id => dex.species.get(id).exists && !dex.species.get(id).isNonstandard);
    for (let i = 0; i < 12; i++) {
      const sp = dex.species.get(pick(goSpecies));
      const odd = chance(0.6);
      const ivs = odd ? {hp: 31, atk: pick([21, 25, 31]), def: pick([21, 31]), spa: 0, spd: 0, spe: Math.floor(random() * 32)}
        : stat(() => Math.floor(random() * 32));
      if (odd) { ivs.spa = ivs.atk; ivs.spd = ivs.def; }
      targeted.push({why: 'go', set: {...base, species: sp.name, ability: Object.values(sp.abilities)[0] || '',
        level: pick([1, 8, 12, 15, 20, 30, 50]), shiny: chance(0.2), ivs,
        moves: [dex.moves.get(pick(Object.keys(own(sp.id)).length ? Object.keys(own(sp.id)) : ['tackle'])).name]}});
    }
    targeted.push(
      {why: 'go', set: {...base, species: 'Mew', ability: 'Synchronize', level: 30, ivs: stat(() => 31), moves: ['Barrier', 'Psywave']}},
      {why: 'go', set: {...base, species: 'Mew', ability: 'Synchronize', level: 50, ivs: stat(() => 31), moves: ['Pound', 'Amnesia', 'Swift', 'Barrier', 'Psywave'].slice(0, 4)}},
      {why: 'go', set: {...base, species: 'Mewtwo', ability: 'Pressure', level: 10, ivs: {hp: 3, atk: 3, def: 3, spa: 3, spd: 3, spe: 3}, moves: ['Amnesia', 'Swift']}},
      {why: 'go', set: {...base, species: 'Meltan', ability: 'Magnet Pull', level: 5, ivs: stat(() => 30), moves: ['Thunder Shock']}},
      {why: 'go', set: {...base, species: 'Zapdos', ability: 'Pressure', level: 20, ivs: {hp: 21, atk: 21, def: 21, spa: 21, spd: 21, spe: 0}, moves: ['Thunderbolt']}},
      // Let's Go's restrictive level-up chain (Barrier and Mimic six apart) and a purified
      // Mewtwo (under level 12) with Let's Go-only moves.
      {why: 'go', set: {...base, species: 'Mew', ability: 'Synchronize', level: 50, ivs: stat(() => 31), moves: ['Barrier', 'Mimic']}},
      {why: 'go', set: {...base, species: 'Mew', ability: 'Synchronize', level: 10, ivs: stat(() => 31), moves: ['Confusion', 'Mimic']}},
      {why: 'go', set: {...base, species: 'Mewtwo', ability: 'Pressure', level: 10, ivs: {hp: 3, atk: 3, def: 3, spa: 3, spd: 3, spe: 3}, moves: ['Psybeam', 'Confuse Ray']}},
      {why: 'go', set: {...base, species: 'Mewtwo', ability: 'Pressure', level: 10, ivs: {hp: 3, atk: 3, def: 3, spa: 3, spd: 3, spe: 3}, moves: ['Mist', 'Psywave']}},
    );
  }
  sets.push(...targeted.map(t => t.set));
  sets.push(
    {...base, species: 'Smeargle', ability: 'Own Tempo', moves: ['Spore', 'Shell Smash']},
    {...base, species: 'Smeargle', ability: 'Own Tempo', moves: ['Spore', 'Sketch']},
    {...base, species: 'Smeargle', ability: 'Own Tempo', moves: ['V-create']},
    {...base, species: 'Tentacruel', ability: 'Clear Body', moves: ['Surf', 'Defog', 'Whirlpool']},
    {...base, species: 'Snorlax', ability: 'Thick Fat', moves: ['Strength', 'Body Slam']},
    {...base, species: 'Pikachu', ability: 'Static', moves: ['Volt Tackle', 'Thunderbolt']},
    {...base, species: 'Pikachu', ability: 'Lightning Rod', moves: ['Surf', 'Fly']},
  );
  return sets;
}
const learnsetCases = {};
const learnsetOutputs = {};
for (const format of targets) {
  const validator = new TeamValidator(format);
  if (!learnsetCases[format.mod]) learnsetCases[format.mod] = drawLearnsetSets(validator.dex, 5000 + parseInt(format.mod.slice(3)));
  learnsetOutputs[format.id] = learnsetCases[format.mod].map(input => {
    const set = JSON.parse(JSON.stringify(input));
    const problems = validator.validateSet(set, {}) || [];
    return [problems, {species: set.species, name: set.name, item: set.item, ability: set.ability, moves: set.moves,
      nature: set.nature, gender: set.gender, level: set.level, evs: set.evs, ivs: set.ivs, hpType: set.hpType ?? '',
      teraType: set.teraType ?? '', shiny: !!set.shiny}];
  });
}
const learnsetMods = {};
const learnsetFormats = {};
for (const mod of Object.keys(learnsetCases)) {
  const ofMod = targets.filter(f => f.mod === mod);
  const common = learnsetCases[mod].map((_, index) => {
    const votes = new Map();
    for (const f of ofMod) {
      const answer = JSON.stringify(learnsetOutputs[f.id][index]);
      votes.set(answer, (votes.get(answer) || 0) + 1);
    }
    return JSON.parse([...votes].sort((a, b) => b[1] - a[1])[0][0]);
  });
  learnsetMods[mod] = {sets: learnsetCases[mod], common};
  for (const f of ofMod) {
    const diff = {};
    learnsetOutputs[f.id].forEach((answer, index) => {
      if (JSON.stringify(answer) !== JSON.stringify(common[index])) diff[index] = answer;
    });
    learnsetFormats[f.id] = {mod, diff};
  }
}

// Each mod's common answers, and each format's differences from them.
const TABLES = ['species', 'pairs', 'items', 'moves', 'abilities', 'natures'];
const mods = {};
const formats = {};
for (const mod of [...new Set(targets.map(f => f.mod))]) {
  const ofMod = targets.filter(f => f.mod === mod).map(f => checks[f.id]);
  const common = {};
  for (const table of TABLES) {
    common[table] = {};
    for (const key of Object.keys(ofMod[0][table])) {
      const votes = new Map();
      for (const c of ofMod) votes.set(c[table][key], (votes.get(c[table][key]) || 0) + 1);
      const [answer] = [...votes].sort((a, b) => b[1] - a[1])[0];
      if (answer !== null) common[table][key] = answer;
    }
  }
  // The pairs are asked from data the legality data does not hold (Mega Stones): their keys.
  common.pair_keys = Object.keys(ofMod[0].pairs).sort();
  mods[mod] = common;
}
let count = 0;
for (const format of targets) {
  const c = checks[format.id];
  const diff = {mod: c.mod, set_has: c.set_has};
  for (const table of TABLES) {
    diff[table] = {};
    for (const [key, answer] of Object.entries(c[table])) {
      count++;
      if ((mods[c.mod][table][key] ?? null) !== answer) diff[table][key] = answer;
    }
  }
  formats[format.id] = diff;
}
// Custom formats for the branches no target format reaches: whitelists (`-All …`), a tag's
// unban against Nonexistent (`+Mythical` in Gen 1 does not bring Shaymin), `+CAP`, `+Past`,
// an unbanned base species and its nonstandard formes, Megas banned by their tag. Each against
// its mod's common answers.
const CUSTOM = [
  ['gen9ou', ['-All Pokemon', '+Pikachu', '+Raichu']],
  ['gen9ou', ['-All Items', '+Leftovers']],
  ['gen9ou', ['-All Moves', '+Tackle']],
  ['gen9ou', ['-All Abilities', '+Static']],
  ['gen9ou', ['-All Natures', '+Adamant']],
  ['gen1ou', ['+Mythical']],
  ['gen9ou', ['+CAP']],
  ['gen9ou', ['+Past']],
  ['gen9ou', ['+Unobtainable']],
  ['gen4ou', ['+Pichu']],
  ['gen7ubers', ['-Mega']],
  ['gen9ou', ['+Uber']],
];
const custom = [];
for (const [id, rules] of CUSTOM) {
  const format = Dex.formats.get(Dex.formats.validate(`${id}@@@${rules.join(',')}`), true);
  const c = rulings(format, false);
  const diff = {format: id, rules, mod: format.mod};
  for (const table of TABLES) {
    diff[table] = {};
    for (const [key, answer] of Object.entries(c[table])) {
      count++;
      if ((mods[format.mod][table][key] ?? null) !== answer) diff[table][key] = answer;
    }
  }
  custom.push(diff);
}
fs.writeFileSync(path.join(fixtureDir, 'checks.json'), JSON.stringify({
  schema: 1,
  provenance: {sim: VERSION['@pkmn/sim']},
  set_name: SET_NAME,
  mods,
  formats,
  custom,
}) + '\n');
fs.writeFileSync(path.join(fixtureDir, 'formes.json'), JSON.stringify({
  schema: 1,
  provenance: {sim: VERSION['@pkmn/sim']},
  forme,
  validation_species: validationSpecies,
}) + '\n');
// Each mod's cases and the answers most of its formats give; each format's differences, by
// the case's index.
const statsMods = {};
const statsFormats = {};
for (const mod of [...new Set(targets.map(f => f.mod))]) {
  const ofMod = targets.filter(f => f.mod === mod);
  const cases = Object.keys(statsCases[ofMod[0].id]);
  const common = cases.map(key => {
    const votes = new Map();
    for (const f of ofMod) {
      const answer = JSON.stringify(statsCases[f.id][key]);
      votes.set(answer, (votes.get(answer) || 0) + 1);
    }
    return JSON.parse([...votes].sort((a, b) => b[1] - a[1])[0][0]);
  });
  statsMods[mod] = {cases, common};
  for (const f of ofMod) {
    const diff = {};
    cases.forEach((key, index) => {
      if (JSON.stringify(statsCases[f.id][key]) !== JSON.stringify(common[index])) diff[index] = statsCases[f.id][key];
    });
    statsFormats[f.id] = {mod, diff};
  }
}
for (const c of setCustom) {
  const diff = {};
  c.outputs.forEach((answer, index) => {
    if (JSON.stringify(answer) !== JSON.stringify(setMods[c.mod].common[index])) diff[index] = answer;
  });
  delete c.outputs;
  c.diff = diff;
}
// A team's sets are indices into the mod's `sets.json` sets, then into its `extras`.
fs.writeFileSync(path.join(fixtureDir, 'teams.json'), JSON.stringify({
  schema: 1,
  provenance: {sim: VERSION['@pkmn/sim']},
  mods: teamMods,
  formats: teamFormats,
}) + '\n');
fs.writeFileSync(path.join(fixtureDir, 'learnset-sets.json'), JSON.stringify({
  schema: 1,
  provenance: {sim: VERSION['@pkmn/sim']},
  mods: learnsetMods,
  formats: learnsetFormats,
}) + '\n');
fs.writeFileSync(path.join(fixtureDir, 'sets.json'), JSON.stringify({
  schema: 1,
  provenance: {sim: VERSION['@pkmn/sim']},
  mods: setMods,
  formats: setFormats,
  custom: setCustom,
}) + '\n');
fs.writeFileSync(path.join(fixtureDir, 'stats.json'), JSON.stringify({
  schema: 1,
  provenance: {sim: VERSION['@pkmn/sim']},
  mods: statsMods,
  formats: statsFormats,
}) + '\n');
console.log(`${targets.length} formats, ${count} checks`);
