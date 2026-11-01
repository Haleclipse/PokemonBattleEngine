# Validation protocol v1 (frozen 2026-09-25)

These shapes are contracts between the Rust engine, the Node oracle adapters and the committed
fixtures. Changing any of them means bumping `schema`, updating both sides in the same commit and
regenerating every pinned output. Pinned outputs are never regenerated silently.

## Scenario v1 — `tests/fixtures/cases/<id>.json`

| Field | Type | Meaning |
|---|---|---|
| `schema` | 1 | |
| `id` | string | case id; also the failure-bundle directory name |
| `battle.generation` | 1..9 | |
| `battle.mode` | `singles` \| `doubles` | |
| `battle.format` | string, optional | Showdown format id; default `gen<N>customgame`. Rust always uses the `custom` format |
| `teams.p1[]`, `teams.p2[]` | set | `label?`, `species`, `ability?`, `item?`, `level` (default 100), `nature?`, `evs?`, `ivs?` (maps of `hp/atk/def/spa/spd/spe`), `moves[]`, `gender?` (`M` / `F`; Showdown only — no gender mechanic is modelled — pinned by `tools/minimize.py` so that deleting a team member keeps the random numbers) |
| `script[]` | `{p1?, p2?}` | Showdown choice syntax, one entry per request boundary: `move N`, `switch N`, `pass`. An entry answers whatever both engines are waiting for, so a turn suspended mid-way (U-turn) takes two entries: the turn choices, then the replacement (`switch N`; the side not asked sends nothing). `switch N` counts in Showdown's current party order (a switch swaps the two Pokémon's positions). Doubles: one comma-separated action per active slot, `move N T` with T = foe slot (1..) or own slot (-1..). Also accepted, as in Showdown: `switch <label>` (the Pokémon named, whatever the party order) and `move <name> [T]` (the move by its `toID`; a locked Pokémon's request lists only the locked move, so it keeps `move 1`) — the forms `tools/minimize.py` writes, added 2026-09-27 without a schema bump since every earlier script means the same |
| `random` | object, optional | `mode`: `oracle-guided` (default) or `semantic`; `seed` for Showdown |

Names resolve through Showdown's `toID`. Sets are consumed unchanged by `Teams.pack` on the
Showdown side and by `build_pokemon` on the Rust side.

## SemanticFrame v1 — one per request boundary

| Field | Type | Meaning |
|---|---|---|
| `schema` | 1 | |
| `checkpoint` | string | `team_preview` (Showdown only), `initial`, `boundary_<n>` |
| `turn` | int | |
| `request` | string \| null | `turn_choices`, `forced_switch`, `revive` (Revival Blessing: a forced switch whose slot is `reviving`, added 2026-09-29 without a schema bump — no earlier frame has one), `finished`. Showdown emits `p1|p2`; the comparator keeps the first |
| `outcome` | string \| null | `winner(p1)`, `winner(p2)`, `draw`; compared |
| `sides[2].active[]` | party index \| null | per active slot |
| `sides[2].party[]` | `{hp, max_hp, status: {kind} \| null, boosts{...}, volatiles{name: turns \| null}, types[], ability, item, species, pp[]}` | `types` are the types the battle reads (Showdown `getTypes`), `ability` the ability the Pokémon has (Gen 3+), `item` the item it holds (Gen 2+, `""` for none), all Showdown ids; recorded since 2026-10-01 (DESIGN.md §41.8), and a recording without them is replayed without comparing them; `pp` is each move slot's PP in move order (Showdown `moveSlots[].pp`), recorded since 2026-10-02 (DESIGN.md §15.15) and likewise not compared where a recording has none; `species` is the species it battles as (Showdown `pokemon.species.id`: a forme it changed into, or the Pokémon it transformed into), recorded since 2026-10-04 (DESIGN.md §43.5) and likewise not compared where a recording has none; `terastallized` is the Tera type the Pokémon took (Gen 9, Showdown `pokemon.terastallized`), written only while it has Terastallized (recorded since 2026-10-04, DESIGN.md §44.8); status kinds are Showdown abbreviations (`brn par psn tox slp frz`); party order is the team declaration order — after a Team Preview choice, the Pokémon brought in the order chosen (M9.4, DESIGN.md §45.5) — never the engine's internal order; a cosmetic forme's `species` is its base species' (Vivillon-Savanna as `vivillon`); volatile / side / field condition names are Showdown ids (`leechseed`, `taunt`, `encore`, `disable`, `mustrecharge`, `choicelock`, `twoturnmove`, `stealthrock`, `spikes`, `toxicspikes`, `stickyweb`, `trickroom`); layered hazards report their layers, a `substitute` its remaining HP, timed volatiles their remaining turns (mustrecharge is 1 in a frame: Showdown's duration 2 already ticked once). A charging two-turn move reports `twoturnmove` and the move's own id: with the same turns when the move has a condition (`fly`, `dig`: semi-invulnerable), null otherwise (`solarbeam`) |
| `sides[2].conditions` | `{name: turns \| null}` | side conditions |
| `field` | `{conditions, weather: {kind, turns_remaining} \| null, terrain: ... \| null}` | |

Compared with `compare_json` after normalisation; `checkpoint` is ignored, `events` (below) is
stripped from the oracle frame.

## Event stream v1 — `frames[].events` in `<id>.showdown.json`

Normalised from Showdown's protocol log between two boundaries, `|split|` deduplicated.
Pokémon are `{side: 0|1, slot: 0.., name}`.

Every Pokémon an event names is named as it is (`pokemon`, `user`, `target`, `of`, and the raw `args`): Showdown names an Illusion holder by its disguise, and `showdown.mjs` puts the true name back, line by line (DESIGN.md R-K.10c; the `debug_log` keeps Showdown's lines).

| kind | Fields |
|---|---|
| `turn` | `turn` |
| `residual` | (no fields) the end-of-turn residual schedule starts here; later events belong to no action |
| `move` | `user`, `move`, `target` (also for a move that failed with `[still]`, whose target Showdown erases from its log: `showdown.mjs` keeps it — recordings from 2026-09-27 on) |
| `switch` | `pokemon`, `details`, `hp`, `max_hp`, `status`; `illusion` (the entrant's own details) when it comes in disguised — `details` are then the disguise's |
| `drag` | same fields; a forced entry (Roar, Dragon Tail, Red Card) |
| `replace` | `pokemon`, `details`: Illusion lifted in place, the Pokémon's own details (before 2026-10-04 parsed as a `switch`; no earlier recording has one). From R-N.1 until R-M8-75 (2026-10-04 – 08) the normalizer dropped both this kind and `switch` / `drag`'s `illusion` (DESIGN.md R-M8-75); the campaign recordings of that span with an Illusion user were re-recorded |
| `damage`, `heal`, `sethp` | `pokemon`, `hp`, `max_hp`, `status`, `from`, `of` (the `[of] …` field: the Pokémon whose ability / item dealt it) |
| `hitcount` | `pokemon`, `count` (hits of a multi-hit move that landed) |
| `prepare` | `pokemon`, `move`: the charging turn of a two-turn move (Fly, Dig, Solar Beam); an attack in the same action means the charge was skipped (sun) |
| `singleturn` | `pokemon`, `move`: Protect / Detect succeeded (a failed one is a `fail`) |
| `crit`, `miss`, `fail`, `immune`, `faint`, `cant`, `supereffective`, `resisted`, `block`, `notarget` | `pokemon`, `detail` |
| `status`, `curestatus` | `pokemon`, `status`, `from` (`[from] ability: Static`, `[from] item: Toxic Orb`, `[from] ability: Natural Cure`, `[msg]` / `[silent]` on a cure), `of` |
| `boost`, `unboost`, `setboost` | `pokemon`, `stat`, `amount` |
| `weather`, `fieldstart`, `fieldend`, `sidestart`, `sideend`, `start`, `end`, `activate`, `ability`, `item`, `enditem` | `args` |

The replay driver derives per-action decisions from it (TESTING.md §6.3): `miss`/`immune`/`fail`
→ Accuracy false, `crit` → Critical true for the hit it precedes, `hitcount` → MultiHitCount,
`status`/`boost` on the damaged target → Secondary true, `status` on the user `[from] ability:`
→ ContactReaction true, `activate … ability: Shed Skin` at the end of the turn → ShedSkin true,
`singleturn` Protect right after a Protect → ProtectSuccess true, `activate … move: Bind|[of] …`
→ the bind's Duration (the boundary frame's `partiallytrapped` plus its first tick),
`damage` / `heal` (move damage, recoil, reactions, drain) → the HP sequence the recovered damage
rolls must reproduce.

## Oracle output v1 — `<id>.showdown.json`

`{schema, provenance: {engine, version, formatid, seed}, frames[], debug_log}`.

## Failure bundle v1 — `target/oracle-failures/<id>/`

`scenario.json`, `rust.frames.json`, `oracle.frames.json`, `diff.json`, `rust.trace.txt`,
`showdown.log.txt`, `provenance.json`, `reproduce.txt`. Written by `tests/showdown_oracle.rs`
whenever a scenario diverges; layout checked by `failure_bundle_has_the_documented_layout`.

## Ids

Engine ids are the registry contract in `data/dex/README.md`; fixtures never contain them.
