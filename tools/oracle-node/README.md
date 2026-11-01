# Oracle Node Tooling

This directory is a development-only boundary. The Rust engine has no runtime dependency on Node.

The package versions are exact on purpose. Updating an oracle must be an explicit reviewable change together with regenerated fixture provenance.

## Showdown semantic oracle

Input is JSON on stdin:

```json
{
  "formatid": "gen9customgame",
  "seed": [1, 2, 3, 4],
  "p1": [{"species":"Pikachu","ability":"Static","moves":["Nuzzle"]}],
  "p2": [{"species":"Mew","ability":"Synchronize","moves":["Splash"]}],
  "choices": [
    {"p1":"move 1","p2":"move 1"}
  ]
}
```

Run:

```bash
npm ci
node src/showdown.mjs < scenario.json > oracle.json
```

Output contains normalized request-boundary frames plus the raw Showdown debug log.

Team Preview is answered by the scenario's `preview` (`{"p1": "team 3142", "p2": …}`), or
`default` (declared order) when it has none — the custom games' answer, whose first frame
compared is `initial`; a real format's replay compares the `team_preview` frame too (M9.4,
DESIGN.md §45.5). From the choice on, a frame lists the Pokémon brought in the order chosen. A
real format's teams reach the battle as its validator leaves them (`battleTeam` in `sets.mjs`:
the server's path).

`format-cases.mjs` writes real-format battles: per format, two legal teams (`legal-teams.mjs`,
every gender named), a Team Preview choice per side, the battle played by `corpus.mjs` and
recorded by `showdown.mjs`:

```bash
node src/format-cases.mjs ../../tests/fixtures/cases 3 gen9vgc2025regi gen9ou
```

A format's battle-time rules (Sleep Clause Mod, Freeze Clause Mod, …) receive their events as
pseudo-weathers on Showdown's field. The frames leave them out of `field.conditions`: they
are the format's rules, not field conditions, and the engine reads them as `BattleClauses`
(M9.5, DESIGN.md §45.7). `-message` lines become `message` events (`{kind, text}`). The
replay reads `Sleep Clause Mod activated.` / `Freeze Clause activated.` as a status secondary
that hit and was refused.

`-transform` lines become `transform` events (`{kind, args}`: user, target, source). The replay
needs them to plan a later lead's Trace after an Imposter (R-M9-23).

## Cartridge patches

`src/cartridge.mjs`, imported by `showdown.mjs` and `corpus.mjs` before any battle starts,
patches Showdown where it is known to differ from the cartridge and the engine follows the
cartridge (`issues.md`, TESTING.md §7). Patches never change what Showdown draws from its PRNG.
`SHOWDOWN_UNPATCHED=1` runs Showdown as published. A patch set on a format's hooks or `actions`
covers every format of its generations (`/^gen4/`): the custom games and the real formats
alike. Until R-M9-20 these patches covered `…customgame` only, so the real formats played
unpatched Showdown.

## Corpus generator

`corpus.mjs` plays a whole battle from a scenario without a `script` with a deterministic policy
and prints the scenario back with the choices as its script (TESTING.md §6.4):

```bash
node src/corpus.mjs < teams.json > ../../tests/fixtures/cases/<id>.json
node src/showdown.mjs < ../../tests/fixtures/cases/<id>.json > ../../tests/fixtures/cases/<id>.showdown.json
```

`corpus: {switchEvery, maxTurns}` in the input tunes the policy (defaults 7 and 80).

## Damage oracle

`damage.mjs` is a minimal JSON wrapper around `@smogon/calc`. Keep damage oracle cases separate from full battle cases where possible: they fail closer to the actual arithmetic bug.

## Dex export (canonical data)

`export-dex.mjs` writes the committed snapshots under `data/dex/` (one per generation) and
maintains the engine id registry `data/dex/ids.json`. See `data/dex/README.md` for the rules.

```bash
node src/export-dex.mjs ../../data/dex
```

## Format export (M9)

`export-formats.mjs` writes the format layer's data, `data/formats/formats.json` (read at run
time from the data directory, not embedded; the target
formats, each generation mod's rules, and `validateRule`'s answer for every rule string), and the
oracle `tests/fixtures/formats/ruletables.json` (Showdown's resolved rule table per target
format). See DESIGN.md §45.2.

```bash
node src/export-formats.mjs ../../data/formats ../../tests/fixtures/formats
```

`export-legality.mjs` writes the legality data, `data/legality/gen{1..9}.json` (each mod's
species, moves, items, abilities and natures with their tiers, tags and `isNonstandard`) and
`aliases.json`, also read at run time; and the oracles of name resolution, species bans and
custom formats, and name resolution (`names.json`) under `tests/fixtures/formats/`. See
DESIGN.md §45.3.

```bash
node src/export-legality.mjs ../../data/legality ../../tests/fixtures/formats
```

`export-validator.mjs` asks Showdown's `TeamValidator` one ruling at a time (the five
`check*` functions) about every species, item, move, ability and nature, under every target
format and a list of custom ones, and writes `tests/fixtures/formats/checks.json`; and
`validateForme` / `getValidationSpecies` on sets made to reach their branches (`formes.json`);
`validateStats` likewise (`stats.json`); and `validateSet` with the learnset stubbed out
(`validateMoves`, `getEventOnlyData`, `validatePokemonGo`, `getLearnsetData`) on seeded and
targeted sets (`sets.json`), and `validateTeam` on seeded and targeted teams (`teams.json`);
and `validateSet` with the learnsets, the father search and Pokémon GO (`learnset-sets.json`,
DESIGN.md §45.4). `export-legality.mjs` also writes the learnsets
(`data/legality/learnsets*.json`) and Pokémon GO's data (`pokemongo.json`).
See DESIGN.md §45.3.

```bash
node src/export-validator.mjs ../../tests/fixtures/formats
```

`export-builder.mjs` is the pre-battle session's oracle (M9.4, DESIGN.md §45.5): per target
format, what its battle reads of the rules (`gameType`, `maxTeamSize`, `pickedTeamSize`, the
Team Preview request, Dynamax / Tera after the clauses), and four teams Showdown admits — drawn
by `legal-teams.mjs` (a seeded generator of legal sets: species the format allows, moves from
the family's learnsets, each set validated alone and then the team) — with the Pokémon
Showdown's `Side` builds from them once validated; for the first two of them, what Team Preview
shows (the `|poke|` lines, `|teampreview|N`), the open team sheets where the format has them, and
eleven team choices with the slots each brings or Showdown's refusal; and hand-made sets reaching
each generation's hidden formes. Writes `tests/fixtures/formats/built.json`.

```bash
node src/export-builder.mjs ../../tests/fixtures/formats
```

## Data oracle

`data.mjs` emits a generation-resolved subset of move / ability / item data through `@pkmn/data`. The normal workflow snapshots this JSON into the repository; CI compares Rust `ResolvedDex` against the snapshot and does not query a website live.

Names the generation does not know are reported as `{"name": ..., "exists": false}` rather than
crashing: "this move did not exist in Gen 1" is a fact the Rust resolver must reproduce.

Regenerate the committed snapshots (only together with a provenance bump):

```bash
for g in 1 4 9; do
  echo "{\"generation\":$g,\"moves\":[...],\"abilities\":[...],\"items\":[...]}" \
    | node src/data.mjs > ../../tests/fixtures/oracle/data-gen$g.json
done
```
