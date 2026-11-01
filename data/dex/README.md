# data/dex — canonical dex snapshots

The engine's single source of static data. One complete, generation-resolved snapshot per
generation, exported from `@pkmn/data` (the Pokémon Showdown Dex, pinned in
`tools/oracle-node/package.json`) and committed here.

| File | Content |
|---|---|
| `gen1.json` … `gen9.json` | moves, species, abilities, items, type chart as that generation resolves them |
| `ids.json` | engine id registry (see below) |
| `names-zh.json` | official Simplified Chinese names (moves, abilities, items, species, types, stats, natures, damage classes) from PokéAPI, keyed by Showdown id; the documents' Chinese terms come from it (`tools/build-zh-names.py`) |

What a generation holds is `@pkmn/data`'s default existence check (no `isNonstandard` entry), with
one exception: a move no Pokémon of the generation can learn (`isNonstandard: 'Unobtainable'`) that
still carries the `metronome` flag stays in, because Showdown's Metronome draws among it — Gen 9's
Burn Up, Corrosive Gas, Cut and Hold Back (`tools/oracle-node/src/export-dex.mjs`, DESIGN.md
R-M8-74).

`ResolvedDex` and `TypeChart` are built from these files (`src/data`, `src/dex`,
`src/rules/type_chart.rs`); the crate embeds them with `include_str!` and parses each generation
lazily on first use. Nothing in the engine reads a website or a file at runtime.

## Regenerating

```bash
cd tools/oracle-node && npm ci
node src/export-dex.mjs ../../data/dex
```

Regenerate only together with an explicit oracle version bump (TESTING.md §8) — or, under the
same oracle version, to add a field the exporter derives by a stated rule from data already in
the snapshot's generation, or an upstream field it used to drop; the regeneration must then
change nothing but that field (compare the files with the field removed). Upstream fields no
longer dropped:

| Field | Since |
|---|---|
| move `hasSheerForceBoost` | DESIGN.md R-EF.25 (user decision 01-Q5, 2026-09-30). It was listed among the presentation keys, but it is a mechanic: Sheer Force boosts the move without taking anything from it (Electro Shot, Order Up — the only two moves with it, both Gen 9). Exported only where `true` (`false`, the `dex-moves` default, is a default value). The regeneration changed the two Gen 9 entries and nothing else. |
| move `zMove` (Gen 7), `maxMove` (Gen 8); item `z_move` (now the upstream value: `true`, or the signature Z-Move's name), `z_move_from`, `z_move_type`, `item_user` (Z-Crystals), `plate` (`onPlate`: the Plates, the type Z-Crystals); species `required_move` (Mega formes: Mega Rayquaza's Dragon Ascent), `cannot_dynamax` (Gen 8) | DESIGN.md R-O.1 (§44.1): the gimmicks' data. Each only in the generation whose gimmick reads it, and only what the upstream data declares: a move's `zMove` / `maxMove` as its data entry writes it (`Dex.forGen(n).data.Moves`), not the defaults `dex-moves.mjs` derives from the base power, which are the engine's tables (`dex::z_move_power`, `dex::max_move_power`; `tests/gimmick_powers.rs` checks them against Showdown's). The regeneration changed these fields and nothing else (Gen 4-9). |

Entries no longer dropped:

| Entry | Since |
|---|---|
| items with `num` 0 | DESIGN.md R-M9-17. The exporter took items with `num > 0` only, which dropped one item: Gen 2's Berserk Gene (Showdown gives it no number). A real-format Gen 2 Ubers battle carried it. Now items with `num >= 0` are taken; the regeneration added Berserk Gene to `gen2.json` and its id (2053, appended to the Gen 2 segment) to `ids.json`, and nothing else. |

Derived fields:

| Field | Rule |
|---|---|
| species `nfe` | not fully evolved: the species / forme's own evolution data (`evos`) names an evolution that exists in the same generation's dex. Judged per species / forme, not per individual; each generation's snapshot uses only its own dex, so an evolution added later never reaches back (Gen 1 Chansey has no `nfe`, Gen 2's has; Bisharp only from Gen 9). Not an upstream field of its own; Eviolite reads it (Showdown `baseSpecies.nfe`, identical over all nine generations). |
| species `gender` | the species / forme's fixed gender as the generation's battle dex (`Dex.forGen(n)`) has it: `M`, `F`, or `N` for genderless; absent when the species has a gender ratio. Gen 1 is `N` everywhere (the gen1 mod's `init` sets every species' gender to `N`; `@pkmn/data` drops the field there, the battle dex does not). A set that names no gender takes it (`build_pokemon`, as Showdown `genders[set.gender] \|\| species.gender \|\| sample(['M', 'F'])`); a species with a ratio then stays genderless, where Showdown draws one (a replay reads that draw from the log). |

A regenerated
snapshot changes the dex fingerprint, so battle snapshots taken against the old data are refused
by `BattleEngine::restore`. Never edit the JSON by hand; fix the exporter or the upstream data.

## Engine ids (`ids.json`)

Every id is assigned once, in this registry, and never renumbered. Each entry records its
`source` and the generation the entry first appears in. The rules differ per object because the
outside world offers a stable index for some objects and not for others:

| Object | Id | Rule |
|---|---|---|
| moves, abilities | `u16` | the in-game index, which is also PokéAPI's primary key (`official-index.json`, built from the PokéAPI repository CSVs at a pinned commit). Entries PokéAPI does not have (the 18 generic Z-Moves, Showdown's per-forme Embody Aspect abilities) go to the reserved range from 60000 so they can never collide with a future key. |
| items | `u16` | no stable in-game index exists (Choice Band is 186 in Gen 3 and 220 from Gen 4), so ids are generation-ordered: `generation * 1000 + n`, `n` counting alphabetically by Showdown id within the generation the item first appears in. New items append to their generation's segment. Choice Band = 3008. |
| species | `{dex, form}` | national dex number plus forme index. The base forme is 0; other formes are numbered in order of (first generation, Showdown id) and append thereafter. Charizard = 6:0, Charizard-Mega-X = 6:1, Charizard-Mega-Y = 6:2, Raichu-Alola = 26:1. |
| types | enum | hand-written, 18 entries |

This follows Pokémon Online's long-standing layout: in-game indexes where they exist, a
self-assigned order where they do not, and `dex:forme` for formes. Showdown's own `num` is not
used as an id anywhere: for ten Gen 9 abilities it differs from the official index and three of
them collide on 284 upstream.

Forme nature (`forme_kind`: mega, primal, regional, battle_only, other) is exported per species
entry; Gigantamax formes are not separate entries in this data source. Spelling differences
between Showdown and PokéAPI (Vise Grip / vice-grip, Pretty Feather / pretty-wing) are aliases
written into `official-index.json` by its builder, so no consumer needs its own alias table.

The test `engine_ids_follow_the_registry_contract` checks all of the above over all nine
generations.

| File | Content |
|---|---|
| `official-index.json` | PokéAPI primary keys for abilities, moves and items; provenance records the pinned commit. Moves and abilities take their id from it; the items table is kept for cross-referencing only |

Regenerate the index only together with bumping the pinned commit:

```bash
tools/fetch-pokeapi.sh                 # sparse checkout of data/v2/csv into tools/pokeapi (ignored)
python3 tools/build-official-index.py  # → data/dex/official-index.json
```

## What the exporter keeps

- Move data the engine consumes (type, category, power, accuracy, PP, priority, target, flags,
  secondaries, self effects) plus every other declarative mechanic key and the names of
  Showdown callback functions. The resolver turns anything it cannot express into an
  `Unsupported` classification with a reason, so unimplemented behaviour is never silent.
- Secondary / self effect tables keep their callback names; JSON would otherwise drop an
  `onHit`-only secondary and make it look harmless.
- Hidden Power's per-type pseudo-moves (`realMove`) are skipped; Status moves' default
  `ignoreImmunity: true` is omitted.

`cargo run --example support_matrix` prints how much of each generation the engine implements.
