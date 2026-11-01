import fs from 'node:fs';
import {Dex, PRNG} from '@pkmn/sim';

// Showdown draws a gender (`prng.sample(['M', 'F'])`, one frame) for every Pokémon whose set
// names none and whose species has no fixed one, as the teams are built — before the battle
// starts. Deleting such a Pokémon therefore shifts every later random number. This pins each
// drawn gender in its set and advances the seed past those draws, so the battle plays exactly
// as before and team members can then be deleted freely (tools/minimize.py).
const scenario = JSON.parse(fs.readFileSync(0, 'utf8'));
const dex = Dex.forGen(scenario.battle.generation);
const prng = new PRNG(scenario.random.seed);
for (const side of ['p1', 'p2']) {
  for (const set of scenario.teams[side]) {
    if (['M', 'F', 'N'].includes(set.gender)) continue;
    const fixed = dex.species.get(set.species).gender;
    if (fixed) continue;
    set.gender = prng.sample(['M', 'F']);
  }
}
scenario.random.seed = prng.getSeed().split(',').map(Number);
process.stdout.write(JSON.stringify(scenario));
