// Export Showdown's `noCopy` for every condition a Pokémon can hold, per generation: whether
// Baton Pass (`copyVolatileFrom`) leaves it behind. tests/baton_pass_tables.rs checks the
// engine's `SwitchingRules::not_passed` against it.
//
// Usage:  node src/nocopy.mjs > ../../tests/fixtures/oracle/nocopy.json
import {Dex} from '@pkmn/sim';

const out = {};
for (let gen = 1; gen <= 9; gen++) {
  const dex = Dex.mod(`gen${gen}`);
  const conditions = new Map();
  for (const id of Object.keys(dex.data.Conditions)) conditions.set(id, dex.conditions.getByID(id));
  for (const move of dex.moves.all()) if (move.condition) conditions.set(move.id, move.condition);
  for (const item of dex.items.all()) if (item.condition) conditions.set(item.id, item.condition);
  for (const ability of dex.abilities.all()) if (ability.condition) conditions.set(ability.id, ability.condition);
  for (const [id, condition] of conditions) {
    (out[id] ??= Array(9).fill(null))[gen - 1] = !!condition.noCopy;
  }
}
const sorted = Object.fromEntries(Object.keys(out).sort().map(id => [id, out[id]]));
console.log(JSON.stringify({
  note: 'Showdown noCopy per generation (index 0 = Gen 1); null where the generation has no such condition',
  noCopy: sorted,
}, null, 1));
