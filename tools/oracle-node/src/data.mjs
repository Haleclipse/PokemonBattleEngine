import fs from 'node:fs';
import {Dex} from '@pkmn/sim';
import {Generations} from '@pkmn/data';

const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const gens = new Generations(Dex);
const gen = gens.get(input.generation);

// A name the generation does not know is a valid, reportable fact: the Rust ResolvedDex must
// have filtered the same entry out. Report it instead of crashing so the snapshot can assert it.
const moves = (input.moves ?? []).map(name => {
  const move = gen.moves.get(name);
  if (!move) return {name, exists: false};
  return {
    exists: true,
    name: move.name,
    id: move.id,
    type: move.type,
    category: move.category,
    base_power: move.basePower,
    accuracy: move.accuracy === true ? null : move.accuracy,
    pp: move.pp,
    priority: move.priority,
  };
});

const abilities = (input.abilities ?? []).map(name => {
  const ability = gen.abilities.get(name);
  if (!ability) return {name, exists: false};
  return {exists: true, name: ability.name, id: ability.id};
});

const items = (input.items ?? []).map(name => {
  const item = gen.items.get(name);
  if (!item) return {name, exists: false};
  return {exists: true, name: item.name, id: item.id};
});

process.stdout.write(JSON.stringify({
  schema: 1,
  provenance: {engine: '@pkmn/data', version: '0.10.11'},
  generation: input.generation,
  moves,
  abilities,
  items,
}, null, 2));
