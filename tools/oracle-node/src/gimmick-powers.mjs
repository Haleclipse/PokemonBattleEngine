// Export the power Showdown gives each damaging move's Z-Move (Gen 7, `zMove.basePower`) and Max
// Move (Gen 8, `maxMove.basePower`): the explicit values of the move data and the defaults
// `dex-moves.mjs` derives from the base power. tests/gimmick_powers.rs checks the engine's
// `ResolvedMove::z_power` / `max_power` (data/dex snapshots plus `dex::z_move_power` /
// `dex::max_move_power`, DESIGN.md §44.1) against it.
//
// Usage:  node src/gimmick-powers.mjs > ../../tests/fixtures/oracle/gimmick-powers.json
import {Dex} from '@pkmn/sim';
import {Generations} from '@pkmn/data';

const gens = new Generations(Dex);
const powers = (n, key) => Object.fromEntries([...gens.get(n).moves]
  .filter(m => m.num > 0 && !m.realMove && m.category !== 'Status' && !m.isZ && !m.isMax && m.id !== 'struggle')
  .map(m => [m.id, m[key].basePower])
  .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
console.log(JSON.stringify({
  note: 'Showdown Z-Move power (Gen 7) and Max Move power (Gen 8) per damaging move',
  z: powers(7, 'zMove'),
  max: powers(8, 'maxMove'),
}, null, 1));
