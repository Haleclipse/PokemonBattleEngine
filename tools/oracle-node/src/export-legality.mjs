// Export the format layer's legality data (M9.2, DESIGN.md §45.3): per generation mod, every
// species, move, item, ability and nature the mod's dex knows (`dex.data.*`, Showdown's own
// tables, the ones `validateBanRule` searches), with the facts the team validator reads of
// them — tiers, tags, `isNonstandard` …; and the alias table. Read at run time from the data
// directory (`FormatLibrary`), never embedded (DESIGN.md §46).
//
// As the differential oracle it also writes, per mod, `validateBanRule`'s answer for every
// name (and its `kind:` form), and Showdown's rule tables of a list of custom formats.
//
// Usage:  node src/export-legality.mjs ../../data/legality ../../tests/fixtures/formats
import fs from 'node:fs';
import path from 'node:path';
import {Dex} from '@pkmn/sim';

const dataDir = process.argv[2] ?? path.join('..', '..', 'data', 'legality');
const fixtureDir = process.argv[3] ?? path.join('..', '..', 'tests', 'fixtures', 'formats');
fs.mkdirSync(dataDir, {recursive: true});
fs.mkdirSync(fixtureDir, {recursive: true});
const VERSION = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url))).dependencies;
const provenance = {sim: VERSION['@pkmn/sim']};

const MODS = ['gen1', 'gen2', 'gen3', 'gen4', 'gen5', 'gen6', 'gen7', 'gen8', 'gen9'];
const orNull = value => (value === undefined || value === '' ? null : value);
const byId = rows => rows.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
// Absent is null, false or empty: the engine reads a missing field as its default.
const compact = row => Object.fromEntries(Object.entries(row).filter(([, v]) =>
  v !== null && v !== false && !(Array.isArray(v) && v.length === 0)));

const answers = {};
for (const mod of MODS) {
  const dex = Dex.mod(mod);
  const data = dex.data;
  // A species as `dex.species.get` resolves it: what the rulings, the forme validation and the
  // set's structure read of it.
  const speciesRow = (s, raw) => compact({
      id: s.id,
      name: s.name,
      base_species: s.baseSpecies,
      forme: orNull(s.forme),
      num: s.num,
      gen: s.gen,
      tier: orNull(s.tier),
      doubles_tier: orNull(s.doublesTier),
      nat_dex_tier: orNull(s.natDexTier),
      nonstandard: orNull(s.isNonstandard),
      tags: s.tags,
      mega: !!s.isMega,
      primal: !!s.isPrimal,
      can_gigantamax: orNull(s.canGigantamax),
      gmax_unreleased: !!s.gmaxUnreleased,
      // `validateBanRule`: a species with formes named without one is its base species.
      has_formes: !!(raw && (raw.otherFormes || raw.cosmeticFormes)),
      exists: s.exists,
      abilities: s.abilities,
      types: s.types,
      gender: orNull(s.gender),
      gender_ratio: s.genderRatio,
      prevo: orNull(s.prevo),
      evo_level: s.evoLevel ?? null,
      nfe: !!s.nfe,
      egg_groups: s.eggGroups,
      base_forme: orNull(s.baseForme),
      battle_only: s.battleOnly ?? null,
      changes_from: orNull(s.changesFrom),
      required_ability: orNull(s.requiredAbility),
      required_item: orNull(s.requiredItem),
      required_items: s.requiredItems ?? null,
      required_move: orNull(s.requiredMove),
      required_tera_type: orNull(s.requiredTeraType),
      unreleased_hidden: s.unreleasedHidden ?? false,
      male_only_hidden: !!s.maleOnlyHidden,
      cosmetic_formes: s.cosmeticFormes ?? null,
      is_cosmetic_forme: !!s.isCosmeticForme,
      evos: s.evos,
      evo_type: orNull(s.evoType),
      evo_move: orNull(s.evoMove),
      evo_region: orNull(s.evoRegion),
      mother: orNull(s.mother),
      can_hatch: !!s.canHatch,
  });
  const species = byId(Object.keys(data.Pokedex).map(id => {
    const s = dex.species.get(id);
    if (s.id !== id) throw new Error(`${mod}: species ${id} resolves to ${s.id}`);
    return speciesRow(s, data.Pokedex[id]);
  }));
  // An alias as a species name resolves through the alias table (`getByID` looks the aliases
  // up before the table), except where it makes a species of its own — a cosmetic forme, a
  // special event species: those, by the alias, in full.
  const extraSpecies = {};
  for (const alias of [...Dex.loadAliases().keys()].sort()) {
    const s = dex.species.get(alias);
    if (s.exists && (!data.Pokedex[s.id] || s.name !== dex.species.get(s.id).name ||
        s.isCosmeticForme)) {
      extraSpecies[alias] = speciesRow(s, null);
    }
  }
  // The cosmetic formes named as themselves (`Gastrodon-East`).
  for (const row of species) {
    for (const name of row.cosmetic_formes || []) {
      const s = dex.species.get(name);
      if (s.exists && !extraSpecies[Dex.toID(name)]) extraSpecies[Dex.toID(name)] = speciesRow(s, null);
    }
  }
  // `key` is the table's (what `validateBanRule` looks up), `id` the thing's own (what a
  // ruling records): Hidden Power Dark's is `hiddenpower`.
  const simple = (table, get, extra = () => ({})) => Object.keys(table).sort().map(key => {
    const thing = get(key);
    return compact({key, id: thing.id, name: thing.name, nonstandard: orNull(thing.isNonstandard), gen: thing.gen, ...extra(thing)});
  });
  // Accuracy Trap Clause's "guaranteed accuracy-lowering move", by its own filter.
  const lowersAccuracy = m => (m.boosts?.accuracy ? m.boosts.accuracy < 0
    : !!m.secondaries?.some(x => x.chance === 100 && x.boosts?.accuracy && x.boosts.accuracy < 0));
  const moves = simple(data.Moves, id => dex.moves.get(id), m => ({
    type: m.type,
    ohko: !!m.ohko,
    status: orNull(m.status),
    accuracy: m.accuracy,
    lowers_accuracy: lowersAccuracy(m),
    nosketch: !!m.flags?.nosketch,
    z: !!m.isZ,
    max: m.isMax === true ? true : orNull(m.isMax),
  }));
  const items = simple(data.Items, id => dex.items.get(id), i => ({
    is_gem: !!i.isGem,
    mega_stone: i.megaStone ?? null,
    forced_forme: orNull(i.forcedForme),
  }));
  const types = Object.keys(data.TypeChart).sort().map(id => {
    const t = dex.types.get(id);
    return {id, name: t.name, nonstandard: orNull(t.isNonstandard), hp_ivs: t.HPivs, hp_dvs: t.HPdvs};
  });
  const abilities = simple(data.Abilities, id => dex.abilities.get(id));
  const natures = simple(data.Natures, id => dex.natures.get(id));
  fs.writeFileSync(path.join(dataDir, `${mod}.json`), JSON.stringify({
    schema: 5, provenance, mod, gen: dex.gen, species, moves, items, abilities, natures, types,
    extra_species: extraSpecies,
    // `dex.species.all()`'s order (the Pokédex's own): the egg-move father search runs in it.
    species_order: Object.keys(data.Pokedex),
  }) + '\n');

  // The oracle: `validateBanRule` for every name, bare and with its kind.
  const inputs = new Set();
  const add = (kind, rows) => {
    for (const row of rows) {
      const key = row.key ?? row.id;
      inputs.add(key);
      inputs.add(`${kind}:${key}`);
    }
  };
  add('pokemon', species);
  add('move', moves);
  add('item', items);
  add('ability', abilities);
  add('nature', natures);
  for (const alias of Dex.loadAliases().keys()) inputs.add(alias);
  for (const tag of ['mythical', 'restrictedlegendary', 'uber', 'ou', 'nduber', 'past', 'nonexistent',
    'unobtainable', 'mega', 'allpokemon', 'allitems', 'physical', 'zmove', 'introducedgen', 'nosuchtag']) {
    inputs.add(tag);
    inputs.add(`pokemontag:${tag}`);
  }
  // `tacklebase`: Showdown strips `base` from the id itself while looking for a species, and
  // the kinds searched after it see the stripped id (move:tackle).
  for (const odd of ['unreleased', 'noitem', 'item:noitem', 'pikachubase', 'pokemon:pikachubase',
    'greninjabond', 'basepokemon:pikachu', 'Rayquaza-Mega', 'Choice Band', '', 'tacklebase',
    'leftoversbase', 'move:tacklebase', 'intimidatebase', 'mencebase', 'adamantbase']) {
    inputs.add(odd);
  }
  const table = {};
  for (const input of [...inputs].sort()) {
    try {
      table[input] = dex.formats.validateBanRule(input);
    } catch (e) {
      const message = e.message.split('\n')[0];
      table[input] = message.startsWith('More than one') ? {ambiguous: message.split(': ')[1].split(', ')}
        : message.startsWith('Nothing matches') ? {nothing: true}
        : {error: message};
    }
  }
  answers[mod] = table;
}

// `getAlias`'s table (alias id → target id), the same for every mod.
const aliases = Object.fromEntries([...Dex.loadAliases()]
  .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
fs.writeFileSync(path.join(dataDir, 'aliases.json'), JSON.stringify({schema: 5, provenance, aliases}) + '\n');
// Pokémon GO's encounters and Let's Go restrictive moves (M9.3, optional data).
fs.writeFileSync(path.join(dataDir, 'pokemongo.json'), JSON.stringify({
  schema: 1, provenance,
  pokemongo: Object.fromEntries(Object.keys(Dex.data.PokemonGoData).sort().map(id => {
    const go = Dex.data.PokemonGoData[id];
    return [id, compact({encounters: go.encounters ?? null, lgpe_restrictive_moves: go.LGPERestrictiveMoves ?? null})];
  })),
}) + '\n');

// The learnsets (M9.3, optional data): the base dex's (`learnsets.json`); each mod's entries
// that differ from it (`learnsets-<mod>.json`); and, for Gen 9's HOME relearner, Brilliant
// Diamond / Shining Pearl's and Legends: Arceus' learnsets of the species standard there
// (`getExternalLearnsetData`).
const learnsetRow = data => compact({
  learnset: data.learnset ?? null,
  event_data: data.eventData ?? null,
  event_only: !!data.eventOnly,
  encounters: data.encounters ?? null,
});
const baseLearnsets = Dex.data.Learnsets;
fs.writeFileSync(path.join(dataDir, 'learnsets.json'), JSON.stringify({
  schema: 1, provenance,
  learnsets: Object.fromEntries(Object.keys(baseLearnsets).sort().map(id => [id, learnsetRow(baseLearnsets[id])])),
}) + '\n');
for (const mod of MODS) {
  const own = Dex.mod(mod).data.Learnsets;
  const differing = Object.keys(own).sort().filter(id => own[id] !== baseLearnsets[id]);
  fs.writeFileSync(path.join(dataDir, `learnsets-${mod}.json`), JSON.stringify({
    schema: 1, provenance, mod,
    learnsets: Object.fromEntries(differing.map(id => [id, learnsetRow(own[id])])),
  }) + '\n');
}
for (const external of ['gen8bdsp', 'gen8legends']) {
  const dex = Dex.mod(external);
  const own = dex.data.Learnsets;
  const standard = Object.keys(own).sort().filter(id => !dex.species.get(id).isNonstandard && own[id].learnset);
  fs.writeFileSync(path.join(dataDir, `learnsets-${external}.json`), JSON.stringify({
    schema: 1, provenance, mod: external,
    learnsets: Object.fromEntries(standard.map(id => [id, {learnset: own[id].learnset}])),
  }) + '\n');
}

// Name resolution (`dex.species.get(name)` …) for every key, alias, cosmetic forme, guessed
// regional or Mega forme and a few names that are nothing: what each resolves to, as
// [id, name, exists]. Three mods: the getters are the same code over each mod's tables.
const names = {};
for (const mod of ['gen1', 'gen4', 'gen9']) {
  const dex = Dex.mod(mod);
  const data = dex.data;
  const aliasIds = [...Dex.loadAliases().keys()];
  const junk = ['', 'nothing', 'constructor', 'mpikachu', 'pikachum', 'alolanzzz', 'a', 'pokemon'];
  const answer = thing => [thing.id, thing.name, !!thing.exists];
  const resolve = (inputs, get) => Object.fromEntries([...new Set(inputs)].sort().map(i => [i, answer(get(i))]));
  const formeKeys = Object.keys(data.Pokedex).filter(k => /(alola|galar|hisui|paldea|mega|megax|megay|primal)$/.test(k));
  const guesses = [];
  for (const key of formeKeys) {
    const s = dex.species.get(key);
    const base = Dex.toID(s.baseSpecies);
    for (const [forme, prefixes] of [['alola', ['a', 'alola', 'alolan']], ['galar', ['g', 'galar', 'galarian']],
      ['hisui', ['h', 'hisui', 'hisuian']], ['paldea', ['p', 'paldea', 'paldean']], ['mega', ['m', 'mega']], ['primal', ['p', 'primal']]]) {
      for (const p of prefixes) guesses.push(p + base, base + p);
    }
  }
  const cosmetic = Object.keys(data.Pokedex).flatMap(k => dex.species.get(k).cosmeticFormes || []);
  // A kind's own aliases, and a hundred of the others' (an alias to another kind is a thing of
  // this kind that does not exist, by the alias's target).
  const aliasMap = Dex.loadAliases();
  const aliasesOf = table => {
    const own = aliasIds.filter(a => Object.prototype.hasOwnProperty.call(table, aliasMap.get(a)));
    const other = aliasIds.filter(a => !own.includes(a)).filter((_, i) => i % 25 === 0).slice(0, 100);
    return [...own, ...other];
  };
  names[mod] = {
    species: resolve([...Object.keys(data.Pokedex), ...aliasIds, ...cosmetic, ...guesses, ...junk,
      'Nidoran♀', 'Nidoran♂', 'nidoran', ' Pikachu ', 'Charizard-Mega-X', 'Raichu-Alola'], i => dex.species.get(i)),
    items: resolve([...Object.keys(data.Items), ...aliasesOf(data.Items), ...junk,
      ...Object.keys(data.Items).filter(k => k.endsWith('berry')).map(k => k.slice(0, -5))], i => dex.items.get(i)),
    moves: resolve([...Object.keys(data.Moves), ...aliasesOf(data.Moves), ...junk, 'hiddenpowerfire60', 'hiddenpower60',
      'Hidden Power [Fire]', 'hiddenpowerfire'], i => dex.moves.get(i)),
    abilities: resolve([...Object.keys(data.Abilities), ...aliasesOf(data.Abilities), ...junk], i => dex.abilities.get(i)),
    natures: resolve([...Object.keys(data.Natures), ...aliasesOf(data.Natures), ...junk], i => dex.natures.get(i)),
    types: resolve([...Object.keys(data.TypeChart), 'Fire', 'stellar', '???', 'bird', ...junk], i => dex.types.get(i)),
  };
}
fs.writeFileSync(path.join(fixtureDir, 'names.json'), JSON.stringify({schema: 1, provenance, names}) + '\n');

// Custom formats (`format@@@rules`): their rule tables, or Showdown's error, item by item.
const CUSTOM = [
  ['gen9ou', ['!Species Clause']],
  ['gen9ou', ['!Sleep Clause Mod']],
  ['gen9ou', ['-Pikachu']],
  ['gen9ou', ['-pokemon:pikachu']],
  ['gen9ou', ['-Pikachu-Original']],
  ['gen9ou', ['+Kyogre', '-Choice Band']],
  ['gen9ou', ['-Tackle + Leftovers']],
  ['gen9ou', ['-Drizzle ++ Swift Swim']],
  ['gen9ou', ['*Mythical', 'Limit One Restricted']],
  ['gen9ou', ['+Unreleased']],
  ['gen9ou', ['-Metronome']],
  ['gen9ou', ['-Nosuchthing']],
  ['gen9vgc2025regi', ['Adjust Level = 50']],
  ['gen9vgc2025regi', ['Adjust Level = 60']],
  ['gen9vgc2025regi', ['!! Adjust Level = 60']],
  ['gen9vgc2025regi', ['!Team Preview']],
  ['gen7ubers', ['-Rayquaza-Mega']],
  ['gen6ubers', ['-Rayquaza-Mega']],
  ['gen7ubers', ['-Rayquaza-Mega', '-Rayquaza']],
  ['gen9ubers', ['-Rayquaza-Mega']],
  ['gen3ou', ['-Mence']],
  ['gen1ou', ['-Lucario']],
  ['gen4ou', ['+Garchomp']],
  ['gen8ou', ['-pokemontag:mythical']],
  ['gen9ou', ['Species Clause']],
  ['gen9ou', ['-Pikachu', '-All Pokemon']],
  ['gen9ou', ['-All Pokemon', '+Pikachu']],
  ['gen9ou', ['-Tackle > 1']],
  ['gen9ou', ['+Baton Pass + Speed Boost']],
  // The implicit Mega Rayquaza Clause, where no ruleset writes it: Rayquaza allowed and its Mega
  // banned; and Rayquaza-Mega banned while Rayquaza stays banned by its Uber tag.
  ['gen7ou', ['+Rayquaza', '-Rayquaza-Mega']],
  ['gen7ou', ['-Rayquaza-Mega']],
  ['gen3ou', ['+Rayquaza', '-Rayquaza-Mega']],
];
const custom = [];
for (const [id, rules] of CUSTOM) {
  const entry = {format: id, rules};
  try {
    const name = Dex.formats.validate(`${id}@@@${rules.join(',')}`);
    const table = Dex.formats.getRuleTable(Dex.formats.get(name, true));
    entry.table = {
      entries: [...table.entries()],
      value_rules: [...table.valueRules.entries()],
      tag_rules: table.tagRules,
      complex_bans: table.complexBans,
      complex_team_bans: table.complexTeamBans,
    };
  } catch (e) {
    entry.error = e.message.split('\n')[0];
  }
  custom.push(entry);
}
// `isBannedSpecies` of every species of the mod, for every target format: the ids banned.
const TARGET = /^gen[1-9](ou|ubers|doublesou|vgc\d+.*)$/;
const banned = {};
for (const format of Dex.formats.all().filter(f => f.effectType === 'Format' && TARGET.test(f.id))) {
  const dex = Dex.mod(format.mod);
  const table = Dex.formats.getRuleTable(format);
  banned[format.id] = Object.keys(dex.data.Pokedex).sort()
    .filter(id => table.isBannedSpecies(dex.species.get(id)));
}
fs.writeFileSync(path.join(fixtureDir, 'banned-species.json'), JSON.stringify({schema: 1, provenance, banned}) + '\n');
fs.writeFileSync(path.join(fixtureDir, 'banrules.json'), JSON.stringify({schema: 1, provenance, answers}) + '\n');
fs.writeFileSync(path.join(fixtureDir, 'custom.json'), JSON.stringify({schema: 1, provenance, custom}, null, 1) + '\n');
console.log(`${MODS.length} mods, ${Object.keys(aliases).length} aliases, ` +
  `${Object.values(answers).reduce((n, t) => n + Object.keys(t).length, 0)} ban rule answers, ${custom.length} custom formats`);
