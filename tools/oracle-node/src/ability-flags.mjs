// Export Showdown's ability `flags` per generation (Gen 3 on): the engine's rule values
// (`ConditionRules::cant_suppress`, …) are checked against it by tests/ability_flags.rs, one flag
// at a time as the engine comes to read it (DESIGN.md §41.4).
//
// Usage:  node src/ability-flags.mjs > ../../tests/fixtures/oracle/ability-flags.json
import {Dex} from '@pkmn/sim';

const FLAGS = ['breakable', 'cantsuppress', 'failroleplay', 'noreceiver', 'noentrain', 'notrace',
  'failskillswap', 'notransform'];
const out = {};
for (const flag of FLAGS) {
  out[flag] = {};
  for (let gen = 3; gen <= 9; gen++) out[flag][`gen${gen}`] = [];
}
for (let gen = 3; gen <= 9; gen++) {
  const dex = Dex.mod(`gen${gen}`);
  for (const ability of dex.abilities.all()) {
    // The generation's own abilities, as the dex snapshots hold them: none from a later
    // generation or not yet released (`Future`), no CAP.
    if (ability.gen > gen || ['CAP', 'Future'].includes(ability.isNonstandard) || ability.num <= 0) continue;
    for (const flag of FLAGS) {
      if (ability.flags[flag]) out[flag][`gen${gen}`].push(ability.id);
    }
  }
}
for (const flag of FLAGS) {
  for (let gen = 3; gen <= 9; gen++) out[flag][`gen${gen}`].sort();
}
console.log(JSON.stringify({
  note: 'Showdown ability flags per generation: the ids of the abilities that carry each flag',
  flags: out,
}, null, 1));
