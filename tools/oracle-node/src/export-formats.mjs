// Export the format layer's data (M9, DESIGN.md §45): the target formats and every rule they
// reach, per generation mod; and, as the differential oracle, Showdown's resolved rule table for
// each target format and `validateRule`'s answer for every rule or ban string they use.
//
// Usage:  node src/export-formats.mjs ../../data/formats ../../tests/fixtures/formats
//
// The target formats are each generation's OU, Ubers, Doubles OU and VGC formats of the pinned
// @pkmn/sim (the user's decision, 2026-10-19). Rule and ban strings are kept raw, as Showdown's
// data writes them, so the engine's resolver runs the same composition (repeals, `!!`, bans,
// value rules) and resolves the names in a ban with the legality data (export-legality.mjs).
import fs from 'node:fs';
import path from 'node:path';
import {Dex} from '@pkmn/sim';

const dataDir = process.argv[2] ?? path.join('..', '..', 'data', 'formats');
const fixtureDir = process.argv[3] ?? path.join('..', '..', 'tests', 'fixtures', 'formats');
fs.mkdirSync(dataDir, {recursive: true});
fs.mkdirSync(fixtureDir, {recursive: true});
const VERSION = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url))).dependencies;

const TARGET = /^gen[1-9](ou|ubers|doublesou|vgc\d+.*)$/;
const targets = Dex.formats.all()
  .filter(f => f.effectType === 'Format' && TARGET.test(f.id))
  .sort((a, b) => a.id.localeCompare(b.id));

// Every rule (or format used as a rule) a target reaches, per generation mod and by id: a mod
// can redefine a rule (Gen 3's Standard brings Switch Priority Clause Mod, Gen 1-4's Flat Rules
// have no Team Preview), and Showdown reads a format's rules from its own mod's dex.
const rules = new Map(); // mod → Map(id → rule)
const raws = new Map(); // mod → set of raw rule strings met under it
function note(mod, raw) {
  if (!raws.has(mod)) raws.set(mod, new Set());
  raws.get(mod).add(raw);
}
function specsOf(entry) {
  return [
    ...entry.ruleset,
    ...(entry.banlist || []).map(b => '-' + b),
    ...(entry.restricted || []).map(b => '*' + b),
    ...(entry.unbanlist || []).map(b => '+' + b),
  ];
}
function visit(mod, entry) {
  for (const raw of specsOf(entry)) {
    note(mod, raw);
    const first = raw.replace(/^\^/, '').charAt(0);
    if ('+*-'.includes(first)) continue;
    const name = raw.replace(/^\^?!{0,2}/, '').split('=')[0];
    const sub = Dex.mod(mod).formats.get(name);
    if (!sub.exists) throw new Error(`unknown rule ${raw} in ${entry.name}`);
    if (!rules.has(mod)) rules.set(mod, new Map());
    if (!rules.get(mod).has(sub.id)) {
      rules.get(mod).set(sub.id, sub);
    }
    visit(mod, sub);
  }
}
for (const format of targets) visit(format.mod, format);
// And every rule of each mod's dex, so a custom rule (`format@@@Max Level = 50`) can name any
// rule Showdown knows, not only the ones the targets reach.
for (const mod of [...new Set(targets.map(f => f.mod))]) {
  const dex = Dex.mod(mod);
  for (const id of Object.keys(dex.data.Rulesets).sort()) {
    const rule = dex.formats.get(id);
    if (!rule.exists || rule.effectType === 'Format') continue;
    if (!rules.get(mod).has(rule.id)) rules.get(mod).set(rule.id, rule);
    visit(mod, rule);
  }
}

// The hooks a rule has in its mod (a mod can drop one: Gravity Sleep Clause checks the team from
// Gen 8 only): the engine runs a rule's hook only where Showdown's rule has it.
const HOOKS = ['onChangeSet', 'onValidateSet', 'onValidateTeam', 'onValidateRule'];
const entryData = entry => ({
  id: entry.id,
  name: entry.name,
  effect_type: entry.effectType,
  ruleset: entry.ruleset,
  banlist: entry.banlist || [],
  unbanlist: entry.unbanlist || [],
  restricted: entry.restricted || [],
  has_value: entry.hasValue === undefined ? null : entry.hasValue,
  mutually_exclusive_with: entry.mutuallyExclusiveWith || null,
  hooks: HOOKS.filter(hook => typeof entry[hook] === 'function'),
});

const canonical = {};
for (const [mod, set] of [...raws].sort()) {
  const dex = Dex.mod(mod);
  canonical[mod] = {};
  for (const raw of [...set].sort()) {
    canonical[mod][raw] = dex.formats.validateRule(raw);
  }
}

const data = {
  schema: 4,
  provenance: {sim: VERSION['@pkmn/sim']},
  formats: targets.map(f => ({...entryData(f), mod: f.mod, gen: Dex.mod(f.mod).gen, game_type: f.gameType})),
  rules: Object.fromEntries([...rules].sort().map(([mod, byId]) =>
    [mod, [...byId.values()].sort((a, b) => a.id.localeCompare(b.id)).map(entryData)])),
};
fs.writeFileSync(path.join(dataDir, 'formats.json'), JSON.stringify(data, null, 1) + '\n');
// The oracle of the engine's `validateRule`: Showdown's answer for each raw string, per mod.
fs.writeFileSync(path.join(fixtureDir, 'canonical.json'), JSON.stringify({
  schema: 1,
  provenance: {sim: VERSION['@pkmn/sim']},
  canonical,
}, null, 1) + '\n');

// The oracle: Showdown's resolved rule table for every target format.
const tables = {};
for (const format of targets) {
  const table = Dex.formats.getRuleTable(format);
  tables[format.id] = {
    entries: [...table.entries()],
    value_rules: [...table.valueRules.entries()],
    tag_rules: table.tagRules,
    complex_bans: table.complexBans,
    complex_team_bans: table.complexTeamBans,
    numbers: {
      min_team_size: table.minTeamSize,
      max_team_size: table.maxTeamSize,
      picked_team_size: table.pickedTeamSize,
      max_total_level: table.maxTotalLevel,
      max_move_count: table.maxMoveCount,
      min_source_gen: table.minSourceGen,
      min_level: table.minLevel,
      max_level: table.maxLevel,
      default_level: table.defaultLevel,
      adjust_level: table.adjustLevel,
      adjust_level_down: table.adjustLevelDown,
      ev_limit: table.evLimit,
    },
  };
}
fs.writeFileSync(path.join(fixtureDir, 'ruletables.json'), JSON.stringify({
  schema: 1,
  provenance: {sim: VERSION['@pkmn/sim']},
  tables,
}, null, 1) + '\n');
console.log(`${targets.length} formats, ${[...rules.values()].reduce((n, m) => n + m.size, 0)} rules, ${Object.values(canonical).reduce((n, m) => n + Object.keys(m).length, 0)} canonical strings`);
