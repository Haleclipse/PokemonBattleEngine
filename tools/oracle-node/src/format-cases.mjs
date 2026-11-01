// Real-format battles for the replay (M9.4c, DESIGN.md §45.5): per format, two teams it admits
// (legal-teams.mjs, every gender named so that no draw stands between the two engines), a Team
// Preview choice per side, a whole battle played by the corpus policy (corpus.mjs) and its
// Showdown recording (showdown.mjs). Writes `<out>/m9-<format>-<n>.json` and `.showdown.json`.
//
// Usage:  node src/format-cases.mjs <out-dir> <count> <format> [<format> …]
import {execFileSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import {Dex} from '@pkmn/sim';
import {formatBattle, formatHash} from './format-teams.mjs';

const [outDir, countArg, ...formats] = process.argv.slice(2);
const count = Number(countArg ?? 1);
fs.mkdirSync(outDir, {recursive: true});
const here = path.dirname(new URL(import.meta.url).pathname);

for (const formatid of formats) {
  const format = Dex.formats.get(formatid);
  const own = formatHash(formatid);
  for (let n = 1; n <= count; n++) {
    const drawn = formatBattle(formatid, n, {p1: [own, n, 5, 7], p2: [own, n, 13, 17], battle: [own, n, 31, 7]});
    const id = `m9-${formatid}-${n}`;
    const scenario = {
      schema: 1,
      id,
      notes: [`M9.4c: a ${format.name} battle from Team Preview on (format-cases.mjs)`],
      battle: drawn.battle,
      teams: drawn.teams,
      preview: drawn.preview,
      random: drawn.random,
      corpus: {mechanics: true, maxTurns: 40},
    };
    const played = execFileSync('node', [path.join(here, 'corpus.mjs')], {input: JSON.stringify(scenario)});
    const recorded = execFileSync('node', [path.join(here, 'showdown.mjs')], {input: played});
    fs.writeFileSync(path.join(outDir, `${id}.json`), played);
    fs.writeFileSync(path.join(outDir, `${id}.showdown.json`), recorded);
    process.stderr.write(`${id}\n`);
  }
}
