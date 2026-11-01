// Export one generation-resolved dex snapshot per generation from @pkmn/data (Showdown Dex).
//
// The snapshots under data/dex/ are the engine's canonical data source: ResolvedDex and
// TypeChart are built from them, and every id (MoveId / AbilityId / ItemId) is the Showdown
// `num`. Only fields the engine can consume or must reason about are exported; Showdown
// callback names are exported as a list so the engine can classify a move / ability / item as
// unsupported instead of silently ignoring behaviour it has not implemented.
//
// Usage:  node src/export-dex.mjs ../../data/dex [--new-registry]
import fs from 'node:fs';
import path from 'node:path';
import {Dex} from '@pkmn/sim';
import {Generations} from '@pkmn/data';

const args = process.argv.slice(2);
const outDir = args.find(arg => !arg.startsWith('--')) ?? path.join('..', '..', 'data', 'dex');
fs.mkdirSync(outDir, {recursive: true});

// `@pkmn/data`'s default existence check, but a move no Pokémon can learn that Metronome may still
// call (`isNonstandard: 'Unobtainable'` with the `metronome` flag: Gen 9's Burn Up, Corrosive Gas, Cut,
// Hold Back) stays in the generation's dex — Showdown's Metronome draws among them (DESIGN.md
// R-M8-74).
const callable = d => d.effectType === 'Move' && d.isNonstandard === 'Unobtainable' && d.flags?.metronome;
const gens = new Generations(Dex, d => {
  if (!d.exists) return false;
  if ('isNonstandard' in d && d.isNonstandard && !callable(d)) return false;
  if (d.kind === 'Ability' && d.id === 'noability') return false;
  return !('tier' in d && ['Illegal', 'Unreleased'].includes(d.tier));
});
const VERSION = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url))).dependencies;

// Engine id registry (data/dex/ids.json). Rules (DESIGN.md §27.1, data/dex/README.md):
//   moves, abilities  the in-game index, which is also PokéAPI's primary key
//                     (data/dex/official-index.json, pinned commit). Entries PokéAPI does not have
//                     (the 18 generic Z-Moves, Showdown's per-forme Embody Aspect) go to a reserved
//                     range at 60000+ so they can never collide with a future key.
//   items             no stable in-game index exists, so ids are generation-ordered:
//                     generation * 1000 + alphabetical sequence within the generation the item
//                     first appears in. New items append to their generation's segment.
//   species           national dex number + forme index: the base forme is 0, other formes are
//                     ordered by (first generation, Showdown id) and append thereafter.
// Ids are assigned once and never renumbered. The registry is rebuilt deterministically over all
// nine generations so the result does not depend on iteration order.
const REGISTRY = path.join(outDir, 'ids.json');
const OFFICIAL_INDEX = path.join(outDir, 'official-index.json');
// An output directory without the registry would get every id assigned afresh (the reserved
// range), silently unlike the committed one: refused, unless a new registry is what is wanted.
if (!args.includes('--new-registry')) {
  for (const file of [REGISTRY, OFFICIAL_INDEX]) {
    if (!fs.existsSync(file)) {
      console.error(`${file} is missing: copy data/dex/ids.json and data/dex/official-index.json into ` +
        `the output directory first (ids are never renumbered), or pass --new-registry to start one`);
      process.exit(2);
    }
  }
}
const official = fs.existsSync(OFFICIAL_INDEX)
  ? JSON.parse(fs.readFileSync(OFFICIAL_INDEX, 'utf8'))
  : {abilities: {}, moves: {}};
const registry = fs.existsSync(REGISTRY)
  ? JSON.parse(fs.readFileSync(REGISTRY, 'utf8'))
  : {schema: 2, note: 'engine ids by Showdown id; rules in data/dex/README.md; never renumbered', moves: {}, abilities: {}, items: {}, species: {}};
const ASSIGNED_BASE = 60000;
const SEGMENT = 1000;

function buildRegistry() {
  const first = {moves: new Map(), abilities: new Map(), items: new Map(), species: new Map()};
  for (let n = 1; n <= 9; n++) {
    const gen = gens.get(n);
    for (const m of gen.moves) if (m.num > 0 && !m.realMove && !first.moves.has(m.id)) first.moves.set(m.id, n);
    for (const a of gen.abilities) if (a.num >= 0 && !first.abilities.has(a.id)) first.abilities.set(a.id, n);
    // Items: num 0 too (Gen 2's Berserk Gene has no number upstream, R-M9-17).
    for (const i of gen.items) if (i.num >= 0 && !first.items.has(i.id)) first.items.set(i.id, n);
    for (const s of gen.species) if (s.num > 0 && !first.species.has(s.id)) first.species.set(s.id, {gen: n, num: s.num, base: !s.forme});
  }
  const sortedIds = map => [...map.keys()].sort((a, b) => a.localeCompare(b));

  for (const kind of ['moves', 'abilities']) {
    const table = registry[kind];
    const used = new Set(Object.values(table).map(e => e.eid));
    for (const id of sortedIds(first[kind])) {
      if (table[id] !== undefined) continue;
      const officialId = official[kind]?.[id];
      if (officialId !== undefined && !used.has(officialId)) {
        table[id] = {eid: officialId, source: 'official', gen: first[kind].get(id)};
        used.add(officialId);
      }
    }
    for (const id of sortedIds(first[kind])) {
      if (table[id] !== undefined) continue;
      const eid = Math.max(ASSIGNED_BASE - 1, ...[...used].filter(v => v >= ASSIGNED_BASE)) + 1;
      table[id] = {eid, source: 'assigned', gen: first[kind].get(id)};
      used.add(eid);
    }
  }

  {
    const table = registry.items;
    const next = {};
    for (const e of Object.values(table)) next[e.gen] = Math.max(next[e.gen] ?? 0, e.eid % SEGMENT);
    for (const id of sortedIds(first.items)) {
      if (table[id] !== undefined) continue;
      const gen = first.items.get(id);
      const seq = (next[gen] ?? 0) + 1;
      if (seq >= SEGMENT) throw new Error(`gen${gen} item segment overflow`);
      next[gen] = seq;
      table[id] = {eid: gen * SEGMENT + seq, source: 'generation-order', gen};
    }
  }

  {
    const table = registry.species;
    const nextForm = {};
    for (const e of Object.values(table)) nextForm[e.dex] = Math.max(nextForm[e.dex] ?? -1, e.form);
    const entries = [...first.species.entries()].sort(([ida, a], [idb, b]) =>
      a.num - b.num || Number(b.base) - Number(a.base) || a.gen - b.gen || ida.localeCompare(idb));
    for (const [id, info] of entries) {
      if (table[id] !== undefined) continue;
      let form;
      if (info.base) {
        if (Object.values(table).some(e => e.dex === info.num && e.form === 0)) throw new Error(`two base formes for #${info.num}: ${id}`);
        form = 0;
      } else {
        form = (nextForm[info.num] ?? 0) + 1;
        if (form > 255) throw new Error(`#${info.num} has more than 255 formes`);
      }
      nextForm[info.num] = Math.max(nextForm[info.num] ?? -1, form);
      table[id] = {dex: info.num, form, source: 'dex-forme', gen: info.gen};
    }
  }
}
buildRegistry();
const engineId = (kind, id) => registry[kind][id].eid;
const speciesEntry = id => registry.species[id];

// Forme nature, borrowed from Pokémon Online's forme flags. Gigantamax formes are not separate
// entries in this data source (`canGigantamax` sits on the base forme).
function formeKind(species) {
  if (!species.forme) return undefined;
  if (species.isMega) return 'mega';
  if (species.isPrimal) return 'primal';
  if (/^(Alola|Galar|Hisui|Paldea)/.test(species.forme)) return 'regional';
  if (species.battleOnly) return 'battle_only';
  return 'other';
}

// Every event handler is behaviour, whether Showdown writes it as a function or as a value
// (`onCriticalHit: false` on Battle Armor, `onTakeItem: false`, `onLockMove: 'recharge'`,
// `onFractionalPriority: -0.1` on Stall); ordering numbers (`onResidualOrder`,
// `onBasePowerPriority`, `onFractionalPriorityPriority` …) are not handlers, and an `undefined`
// override removes an inherited handler. Two events end in "Priority" themselves.
const PRIORITY_EVENTS = new Set(['onFractionalPriority', 'onModifyPriority']);
const isOrdering = k =>
  /^on[A-Z]/.test(k) && !PRIORITY_EVENTS.has(k) && /(Priority|Order|SubOrder)$/.test(k);
const isHandler = (obj, k) => /^on[A-Z]/.test(k) && !isOrdering(k) && obj[k] !== undefined;
const callbacks = obj => Object.keys(obj).filter(k => typeof obj[k] === 'function' || isHandler(obj, k)).sort();
const defined = pairs => Object.fromEntries(pairs.filter(([, v]) => v !== undefined && v !== null && v !== false && v !== ''));

// Every move property is exported unless it is presentation / bookkeeping, or a field Showdown
// sets on every move whose value is still its default: an unknown key must reach the snapshot,
// where the engine's classifier marks the move unsupported until something consumes it
// (`willCrit: false` on Gen 2 Flail, `noDamageVariance`, `ignoreAbility`, `tracksTarget` …).
// An explicit `false` on a sparse key is data too and is kept.
const PRESENTATION_KEYS = new Set([
  'id', 'num', 'name', 'fullname', 'desc', 'shortDesc', 'contestType', 'gen', 'isNonstandard',
  'kind', 'effectType', 'exists', 'zMove', 'maxMove', 'realMove', 'noPPBoosts', 'baseMoveType',
  'sourceEffect', 'spreadHit',
  // exported separately below
  'type', 'category', 'basePower', 'accuracy', 'pp', 'priority', 'target', 'flags',
  'secondary', 'secondaries', 'self', 'critRatio', 'condition', 'ignoreImmunity',
]);
const FIELD_DEFAULTS = {
  affectsFainted: false, forceSTAB: false, ignoreAbility: false, ignoreDefensive: false,
  ignoreNegativeOffensive: false, ignoreOffensive: false, ignorePositiveDefensive: false,
  isMax: false, isZ: false, noCopy: false, nonGhostTarget: '', overrideDefensivePokemon: undefined,
  overrideDefensiveStat: undefined, overrideOffensivePokemon: undefined,
  overrideOffensiveStat: undefined, selfSwitch: undefined, sideCondition: undefined,
  status: undefined, volatileStatus: undefined, weather: undefined, damage: undefined,
  duration: undefined, hasSheerForceBoost: false,
};
const isDefault = (k, v) => v === undefined || v === null || v === '' ||
  (k in FIELD_DEFAULTS && FIELD_DEFAULTS[k] === v);
// Handler ordering numbers (`onDamagePriority`, `isOrdering` above) are not properties of the
// move.

// Secondary / self effect tables. Callback names are kept: JSON.stringify would silently drop
// them and a secondary with only an `onHit` would look like an empty, harmless effect.
function effectTable(effect) {
  if (!effect) return undefined;
  const cbs = callbacks(effect);
  return defined([
    ['chance', effect.chance], ['status', effect.status], ['boosts', effect.boosts],
    ['volatileStatus', effect.volatileStatus], ['sideCondition', effect.sideCondition],
    ['weather', effect.weather], ['pseudoWeather', effect.pseudoWeather],
    ['self', effectTable(effect.self)], ['callbacks', cbs.length ? cbs : undefined],
  ]);
}

// The gimmicks' per-move data, exported only in the generation whose mechanic reads it
// (data/dex/README.md): Gen 7 the upstream `zMove` (a Z-Move's power, or a status move's Z effect:
// `boost` / `effect`), Gen 8 the upstream `maxMove` power. Only what the move's data entry
// declares; the defaults `dex-moves.mjs` derives from the base power are the engine's tables
// (`dex::z_move_power`, `dex::max_move_power`, DESIGN.md §44.1).
function gimmickData(gen, move) {
  const raw = Dex.forGen(gen.num).data.Moves[move.id] ?? {};
  return defined([
    ['zMove', gen.num === 7 && raw.zMove ? {...raw.zMove} : undefined],
    ['maxMove', gen.num === 8 && raw.maxMove ? {...raw.maxMove} : undefined],
  ]);
}

function exportMove(move, gen) {
  const extra = Object.fromEntries(Object.entries(move).filter(([k, v]) =>
    typeof v !== 'function' && !PRESENTATION_KEYS.has(k) && !isOrdering(k) && !isHandler(move, k) &&
    !isDefault(k, v)).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
  // `condition` is a nested effect table; only its callback names matter to the engine.
  if (move.condition) extra.condition = callbacks(move.condition);
  // A high critical-hit stage is a mechanic; the default stage 1 is not worth a key.
  if (move.critRatio && move.critRatio !== 1) extra.critRatio = move.critRatio;
  const secondaries = (move.secondaries ?? []).map(effectTable);
  if (secondaries.length) extra.secondaries = secondaries;
  const self = effectTable(move.self);
  if (self) extra.self = self;
  // `ignoreImmunity` defaults to true for Status moves and false for damaging ones; only the
  // overrides matter (Thunder Wave's `false` against Ground, Gen 1 Seismic Toss's `true`).
  const immunity = move.ignoreImmunity;
  if (move.category === 'Status' ? immunity !== true && immunity !== undefined
    : immunity !== false && immunity !== undefined) {
    extra.ignoreImmunity = immunity;
  }
  return {
    id: move.id, eid: engineId('moves', move.id), num: move.num, name: move.name,
    type: move.type, category: move.category,
    bp: move.basePower, acc: move.accuracy === true ? null : move.accuracy, pp: move.pp,
    pri: move.priority, target: move.target, flags: Object.keys(move.flags ?? {}).sort(),
    ...extra,
    ...gimmickData(gen, move),
    callbacks: callbacks(move),
  };
}

// `nfe` is derived, not an upstream field (data/dex/README.md): a species / forme is not fully
// evolved when its evolution data names an evolution that exists in this generation's own dex, so
// an evolution added later never reaches back into an older snapshot.
function notFullyEvolved(gen, species) {
  const evos = Dex.forGen(gen.num).species.get(species.id).evos ?? [];
  return evos.some(evo => gen.species.get(evo) !== undefined);
}

// `gender` is derived too (data/dex/README.md): the species / forme's fixed gender as this
// generation's battle dex has it (Showdown `species.gender`: `M`, `F`, or `N` for genderless; the
// gen1 mod makes every species `N`). A species with a gender ratio has none.
function fixedGender(gen, species) {
  const gender = Dex.forGen(gen.num).species.get(species.id).gender;
  return ['M', 'F', 'N'].includes(gender) ? gender : undefined;
}

function exportSpecies(species, gen) {
  const slots = species.abilities ?? {};
  return defined([
    ['id', species.id], ['num', species.num], ['form', speciesEntry(species.id)?.form], ['name', species.name],
    ['types', species.types],
    ['base_stats', species.baseStats],
    ['abilities', Object.keys(slots).length ? {...slots} : undefined],
    ['weight_kg', species.weightkg],
    ['base_species', species.baseSpecies !== species.name ? species.baseSpecies : undefined],
    ['forme', species.forme || undefined], ['forme_kind', formeKind(species)],
    ['battle_only', species.battleOnly ? true : undefined], ['changes_from', species.changesFrom],
    ['required_item', species.requiredItem], ['required_ability', species.requiredAbility],
    ['nfe', notFullyEvolved(gen, species) ? true : undefined],
    // Read by the gimmicks only (DESIGN.md §44.1): Mega Rayquaza's move (Gen 6-7), the species
    // that cannot Dynamax (Gen 8).
    ['required_move', species.isMega ? species.requiredMove : undefined],
    ['cannot_dynamax', gen.num === 8 && species.cannotDynamax ? true : undefined],
    ['gender', fixedGender(gen, species)],
  ]);
}

function exportAbility(ability) {
  return {id: ability.id, eid: engineId('abilities', ability.id), num: ability.num, name: ability.name, callbacks: callbacks(ability)};
}

function exportItem(item) {
  return defined([
    ['id', item.id], ['eid', engineId('items', item.id)], ['gen', registry.items[item.id].gen], ['num', item.num], ['name', item.name],
    ['is_choice', item.isChoice], ['is_berry', item.isBerry], ['is_gem', item.isGem],
    ['mega_stone', item.megaStone], ['z_move', item.zMove],
    // A Z-Crystal's own data (DESIGN.md §44.4): the move it turns into its signature Z-Move, the
    // type of moves it turns into Z-Moves, the species that may use it; and the type a Plate or
    // Z-Crystal gives Arceus (`onPlate`, also listed as a callback).
    ['z_move_from', item.zMoveFrom], ['z_move_type', item.zMoveType],
    ['item_user', item.zMove ? item.itemUser : undefined], ['plate', item.onPlate],
    // A Memory's / Drive's type (DESIGN.md §43.4, `onMemory` / `onDrive`, also listed as
    // callbacks): Silvally's RKS System type and Multi-Attack / Techno Blast's type.
    ['memory', item.onMemory], ['drive', item.onDrive],
    ['fling_bp', item.fling?.basePower], ['natural_gift', item.naturalGift],
    ['ignore_klutz', item.ignoreKlutz],
    ['callbacks', callbacks(item)],
  ]);
}

// Attacking type -> defending type -> multiplier, only non-neutral entries.
function exportTypes(gen) {
  const names = [...gen.types].map(t => t.name).filter(n => n !== '???' && n !== 'Stellar');
  const chart = {};
  for (const attack of names) {
    const row = {};
    for (const defense of names) {
      const mult = gen.types.totalEffectiveness(attack, [defense]);
      if (mult !== 1) row[defense] = mult;
    }
    chart[attack] = row;
  }
  return {names, chart};
}

const rows = list => '[\n' + list.map(x => '    ' + JSON.stringify(x)).join(',\n') + '\n  ]';

for (let n = 1; n <= 9; n++) {
  const gen = gens.get(n);
  // Hidden Power's per-type variants are pseudo-moves sharing the real move's num (`realMove`).
  const moves = [...gen.moves].filter(m => m.num > 0 && !m.realMove).map(m => exportMove(m, gen));
  const species = [...gen.species].filter(s => s.num > 0).map(s => exportSpecies(s, gen));
  const abilities = [...gen.abilities].filter(a => a.num >= 0).map(exportAbility);
  const items = [...gen.items].filter(i => i.num >= 0).map(exportItem);
  const types = exportTypes(gen);
  if (types.chart.Fire?.Grass !== 2 || types.chart.Ground?.Flying !== 0) {
    throw new Error(`gen${n}: type chart sanity check failed`);
  }
  const body =
    '{\n' +
    `  "schema": 1,\n` +
    `  "provenance": ${JSON.stringify({engine: '@pkmn/data', version: VERSION['@pkmn/data'], sim: VERSION['@pkmn/sim']})},\n` +
    `  "generation": ${n},\n` +
    `  "types": ${JSON.stringify(types)},\n` +
    `  "moves": ${rows(moves)},\n` +
    `  "species": ${rows(species)},\n` +
    `  "abilities": ${rows(abilities)},\n` +
    `  "items": ${rows(items)}\n` +
    '}\n';
  const file = path.join(outDir, `gen${n}.json`);
  fs.writeFileSync(file, body);
  console.log(`${file}: moves=${moves.length} species=${species.length} abilities=${abilities.length} items=${items.length} types=${types.names.length} ${(body.length / 1024).toFixed(0)} KB`);
}

for (const kind of ['moves', 'abilities', 'items', 'species']) {
  registry[kind] = Object.fromEntries(Object.entries(registry[kind]).sort(([a], [b]) => a.localeCompare(b)));
}
fs.writeFileSync(REGISTRY, JSON.stringify(registry, null, 1) + '\n');
const bySource = kind => Object.values(registry[kind]).reduce((acc, e) => ((acc[e.source] = (acc[e.source] ?? 0) + 1), acc), {});
const formes = Object.values(registry.species).filter(e => e.form > 0).length;
console.log(`${REGISTRY}: moves=${JSON.stringify(bySource('moves'))} abilities=${JSON.stringify(bySource('abilities'))} items=${Object.keys(registry.items).length} species=${Object.keys(registry.species).length} (formes=${formes})`);
