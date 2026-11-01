// Stat-formula micro-oracle: species + level + nature + EVs + IVs → @smogon/calc's rawStats.
// Input: {"cases": [{"id", "generation", "species", "level", "nature", "evs", "ivs"}]}
import fs from 'node:fs';
import {Generations, Pokemon} from '@smogon/calc';

const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const cases = input.cases.map(c => {
  const gen = Generations.get(c.generation);
  const pokemon = new Pokemon(gen, c.species, {level: c.level, nature: c.nature, evs: c.evs, ivs: c.ivs});
  return {
    ...c,
    expected: {raw_stats: pokemon.rawStats, max_hp: pokemon.maxHP()},
    calc_inputs: {base_stats: pokemon.species.baseStats, nature: pokemon.nature, evs: pokemon.evs, ivs: pokemon.ivs, level: pokemon.level},
  };
});
process.stdout.write(JSON.stringify({schema: 1, provenance: {engine: '@smogon/calc', version: '0.11.0'}, cases}, null, 2));
