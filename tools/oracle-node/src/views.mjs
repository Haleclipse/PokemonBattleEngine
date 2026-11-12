// What each observer of a Showdown battle knows (DESIGN.md §47.0 items 9 and 12, M10.4c): the
// battle's raw log split into each player's and the spectators' protocol streams (@pkmn/sim's
// `extractChannelMessages`), each read by an @pkmn/client `Battle` — Showdown's client-side
// tracker, an implementation of its own — the players' with their requests too. At every
// checkpoint the recorder takes each observer's knowledge, normalized, as `views`.
import {Dex, Teams, extractChannelMessages, toID} from '@pkmn/sim';
import {Battle as ClientBattle, Pokemon as ClientPokemon, Side as ClientSide} from '@pkmn/client';
import {Generations} from '@pkmn/data';
import {Protocol} from '@pkmn/protocol';

const generations = new Generations(Dex);
// @pkmn/client's weather names → Showdown's ids.
const WEATHER_IDS = {
  'Sand': 'sandstorm', 'Sun': 'sunnyday', 'Rain': 'raindance', 'Hail': 'hail', 'Snow': 'snowscape',
  'Harsh Sunshine': 'desolateland', 'Heavy Rain': 'primordialsea', 'Strong Winds': 'deltastream',
};
// Showdown's channels: a player's number, 0 for the spectators.
const OBSERVERS = [['p1', 1], ['p2', 2], ['spectator', 0]];
// The notation the views are written in, bumped whenever it changes (`tests/support/views.rs`
// compares this one alone): 2 from R-M10-19 (Showdown's ids for the weather and the terrain,
// cosmetic formes as their base species, the three markers); 3 from R-M10-22 (items by their
// own ids, `item_from_z_move`, Anger Point's ability taken from `-setboost`, an item seen at work
// after another was used up, a base ability kept through another's naming); 4 from R-M10-23 (the
// moves' uses without Pressure's, `uses_uncompared`, `moves_copied`, `moves_from_request`,
// `transformed`; the moves seen as a transformed Pokémon's; a generic Z-Move's or a Max Move's name
// and a `cant` that names another's move left out; Beak Blast's charge, Forewarn, a Leppa Berry, a
// Round brought forward); 5 from R-M10-26 (the volatiles: a reflection keeps the reflector's
// `-singlemove` ones, a Yawn stays on through a sleep); 6 from R-M10-30 (a side's `preview`, the
// Team Preview lines and the open team sheets, at the first checkpoint after the start; the sheets
// read as Showdown's own client reads them); 7 from R-M10-39 (a suppressed ability, `-endability`,
// kept as the ability it is rather than blanked). The views R-M10-16 to R-M10-18 recorded have none.
const VERSION = 7;

// The engine's views count the uses seen of a move, not the PP it paid (DESIGN.md §47.0 item 13):
// @pkmn/client's `useMove` adds the PP each Pressure takes (`getPressurePP`), left out here.
ClientPokemon.prototype.getPressurePP = function (pp) {
  return pp;
};
// A transformed Pokémon's move is the one it transformed into's own (from Gen 2 none transforms into
// one that has): @pkmn/client's `rememberMove` passes it on along a chain of Transforms, to the one
// that Pokémon transformed into since. It stops at the first here.
const rememberMove = ClientPokemon.prototype.rememberMove;
ClientPokemon.prototype.rememberMove = function (moveName, ppUsed = 1, recursionSource) {
  const transform = this.volatiles.transform;
  if (!recursionSource || !transform || this.side.battle.gen.num < 2) {
    return rememberMove.call(this, moveName, ppUsed, recursionSource);
  }
  delete this.volatiles.transform;
  try {
    return rememberMove.call(this, moveName, ppUsed, recursionSource);
  } finally {
    this.volatiles.transform = transform;
  }
};

// A transformed Pokémon shows the ability Transform gave it: @pkmn/client files one named then as
// its base ability too (when it knew none), and puts it back as the Pokémon leaves the field. It is
// the one in effect, never the base.
const rememberAbility = ClientPokemon.prototype.rememberAbility;
ClientPokemon.prototype.rememberAbility = function (ability, isNotBase) {
  return rememberAbility.call(this, ability, isNotBase || !!this.volatiles.transform);
};

// Baton Pass hands on what Showdown's `copyVolatileFrom` copies: no condition marked `noCopy` (from
// Gen 5 Mean Look's and Block's `trapped`; Gen 5 Mud Sport and Water Sport), which @pkmn/client's
// own list leaves in — and every other, which its list may take out: Laser Focus, no `noCopy`, the
// one of them the simulator hands on (a Gen 7 Delcatty's to a Paras, R-M10-71).
const copyVolatileFrom = ClientPokemon.prototype.copyVolatileFrom;
ClientPokemon.prototype.copyVolatileFrom = function (pokemon, copySource = 'batonpass') {
  const passed = {...pokemon.volatiles};
  copyVolatileFrom.call(this, pokemon, copySource);
  if (copySource !== 'batonpass') return;
  const dex = Dex.forGen(this.side.battle.gen.num);
  for (const id of Object.keys(this.volatiles)) {
    if (dex.conditions.get(id).noCopy) delete this.volatiles[id];
  }
  for (const [id, volatile] of Object.entries(passed)) {
    const condition = dex.conditions.get(id);
    if (!(id in this.volatiles) && condition.exists && !condition.noCopy) this.volatiles[id] = volatile;
  }
};

// A Pokémon coming in that matches a Team Preview entry takes the entry's place (`addPokemon` with
// the entry's slot, `|updatepoke|` too): Showdown's own client keeps what was known of the entry —
// its item, base ability and moves (an open team sheet's) — @pkmn/client the item alone.
const addPokemon = ClientSide.prototype.addPokemon;
ClientSide.prototype.addPokemon = function (details, replaceSlot = -1) {
  const old = replaceSlot >= 0 ? this.team[replaceSlot] : undefined;
  const poke = addPokemon.call(this, details, replaceSlot);
  if (old?.baseAbility) {
    poke.baseAbility = old.baseAbility;
    if (!poke.ability) poke.ability = old.baseAbility;
  }
  if (old?.moveSlots.length) poke.moveSlots = old.moveSlots;
  return poke;
};

export function observers(battle) {
  const clients = OBSERVERS.map(([name, channel]) => ({
    name, channel, client: new ClientBattle(generations), reset: new Set(), copied: new Set(),
    unresolved: new Set(), zItems: new Set(), hiddenSwaps: new Set(), masks: {worn: new Set(), unmasked: new Set(), entryItems: new WeakMap()},
    misplaced: new Set(), renewed: new Set(), previews: {p1: {pokes: [], sheet: null}, p2: {pokes: [], sheet: null}, started: false, taken: false},
    moves: {uncompared: new Set(), reset: new Set(), copied: new Set(), fromRequest: new Set(), zPowered: new Set(), previous: null, dancer: null, called: new Set(), lastMove: new Map(), mover: null, moveName: null, stolen: null, flung: null, snatcher: null},
    changed: new WeakSet(),
    handover: {who: null, used: new Map()},
    fallenTera: new WeakMap(),
  }));
  let cursor = 0;
  return {
    /** Feeds the log since the last call (and the players' requests), then takes the views. */
    take() {
      const raw = battle.log.slice(cursor).join('\n');
      cursor = battle.log.length;
      const views = {version: VERSION};
      for (const {name, channel, client, reset, copied, unresolved, zItems, hiddenSwaps, masks, misplaced, renewed, moves, previews, changed, handover, fallenTera} of clients) {
        const stream = extractChannelMessages(raw, [channel])[channel].join('\n');
        for (const parsed of Protocol.parse(stream)) {
          const kwArgs = parsed.kwArgs;
          const previous = moves.previous;
          const args = movesLine(parsed.args, kwArgs, moves);
          cheekPouchHeal(args, kwArgs);
          clearAmuletFail(client, args, kwArgs);
          obliviousFrom(kwArgs);
          moves.previous = args;
          noteIllusionReset(client, reset, args);
          noteMaskedKnowledge(client, masks, args);
          noteMisplaced(client, misplaced, args);
          const entries = entriesOf(client);
          noteOwnBoostsCopied(client, name, copied, args);
          const twin = twinOnField(client, args) || fallenTwin(client, args);
          const itemBefore = itemOf(client, args);
          const basesBefore = basesOf(client, args, kwArgs);
          const healedItem = itemKeptFromHeal(client, moves, args, kwArgs);
          const micle = micleSpent(client, args, kwArgs);
          const slotsBefore = slotsOf(client, args);
          dropOldCopy(client, moves, args);
          ownAbilityBeforeTransform(client, battle, name, args);
          const hidden = ownSlotsHidden(client, args);
          notePreview(previews, args);
          if (args[0] === 'start') previews.started = true;
          const kept = volatilesKept(client, args, kwArgs);
          const asOne = asOneBefore(client, args);
          noteFallenTera(client, fallenTera, args);
          const wornBefore = args[0] === 'replace' ? activeAt(client, args[1]) : null;
          identifyRevived(client, args, kwArgs);
          client.add(args, kwArgs);
          untangleReplace(client, args, wornBefore);
          restoreRevivedTera(client, fallenTera, args, kwArgs);
          noteEntryItem(client, masks, args, wornBefore);
          restoreOwnSlots(hidden);
          noteShowteam(client, previews, args);
          restoreVolatiles(kept);
          if (healedItem) healedItem.poke.item = healedItem.item;
          if (micle) Object.assign(micle.poke, micle.kept);
          noteMoves(client, moves, args, kwArgs, slotsBefore);
          noteHaze(client, args);
          clampBoost(client, args);
          notePrimalOrb(client, args);
          copyDragonCheer(client, args);
          transformCritVolatiles(client, args);
          noteMimicDraw(client, args, previous);
          keepAsOne(client, args, previous, asOne);
          noteOwnCopy(client, moves, name, battle, args);
          noteUnresolvedIllusion(client, unresolved, args, twin);
          noteZItem(client, zItems, args, itemBefore);
          rememberAbilityFrom(client, args, kwArgs);
          rememberItemAtWork(client, moves, args, kwArgs);
          noteAbilityChange(client, changed, args, kwArgs, basesBefore);
          noteRenewed(client, renewed, entries);
          noteBestowed(client, args, kwArgs);
          noteSymbiosis(client, args, kwArgs);
          noteMover(moves, args, kwArgs);
          noteFlungItem(client, moves, args, kwArgs);
          notePerishBody(client, moves, args, kwArgs);
          noteHiddenSwap(client, hiddenSwaps, args, kwArgs);
          noteUsedOnReceipt(client, handover, args, kwArgs);
        }
        const side = name === 'p1' ? battle.p1 : name === 'p2' ? battle.p2 : null;
        // A player's own Illusion holder that came in disguised is told apart by the request that
        // follows; where none does (the battle ended), the client's own side is its guess
        // (R-M10-46).
        if (side && !side.activeRequest && side.active.some(pokemon => pokemon?.illusion)) {
          unresolved.add(name);
        }
        if (side?.activeRequest) {
          const kept = movesTaken(client, name);
          const request = Protocol.parseRequest(JSON.stringify(side.activeRequest));
          const teraIds = teraSearchIds(client, name, request);
          client.update(request);
          restoreTeraSearchIds(teraIds);
          noteMovesFromRequest(client, moves, name, kept);
        }

        const own = name === 'p1' ? client.p1 : name === 'p2' ? client.p2 : null;
        // What the preview showed never changes once the battle is on: taken once, at the first
        // checkpoint after the battle's start.
        const preview = previews.started && !previews.taken ? previews : null;
        if (preview) previews.taken = true;
        views[name] = view(client, reset, copied, unresolved, zItems, hiddenSwaps, masks, misplaced, renewed, moves, own, preview);
      }
      return views;
    },
  };
}

/**
 * A `cant` that names a move not the Pokémon's own, which @pkmn/client would remember as its
 * (`cantUseMove`): an ability holder's line on another's move (`ability: Damp|move|[of] user`,
 * Dazzling, Armor Tail), a move a call brought refused as it starts (right after the caller's
 * `move` line: `move: Gravity|Fly`), a dance refused. The line is read without its move. Gen 3
 * Sleep Talk's `nopp` names one of its own.
 * A dance is the dancer's first `move` or `cant` after Dancer's `-activate` (`moves.dancer`),
 * its BeforeMove's lines in between (a confusion it gets past); a confusion self-hit ends it with
 * neither. The Choice lock's refusal of one (`choicelock.onBeforeMove`) writes the dance's `move`
 * line with no `[from]`, which @pkmn/client files among the dancer's own moves: it is read as the
 * dance it is, `[from] ability: Dancer` (a Gen 7 Oricorio locked into Roost, R-M10-64).
 */
function movesLine(args, kwArgs, moves) {
  const previous = moves.previous;
  const who = args[0] !== 'debug' && typeof args[1] === 'string' && /^p\d[a-z]?: /.test(args[1])
    ? ident(args[1]) : null;
  if (args[0] === '-activate' && args[2] === 'ability: Dancer') {
    moves.dancer = who;
    return args;
  }
  const dancing = !!moves.dancer && who === moves.dancer;
  const danceOver = dancing && (args[0] === 'move' || args[0] === 'cant' ||
    (args[0] === '-damage' && kwArgs.from === 'confusion'));
  if (danceOver || (who && who !== moves.dancer) || args[0] === 'upkeep' || args[0] === 'turn') {
    moves.dancer = null;
  }
  if (args[0] === 'move' && dancing && !kwArgs.from) kwArgs.from = 'ability: Dancer';
  if (args[0] !== 'cant' || !args[3] || args[2] === 'nopp') return args;
  const same = previous && typeof previous[1] === 'string' && ident(previous[1]) === ident(args[1]);
  const called = (same && previous[0] === 'move') || dancing;
  return kwArgs.of || called ? args.slice(0, 3) : args;
}

/**
 * The gen5 mod's Oblivious (Gen 3-5) names itself without the `ability:` mark (`-immune|p|[from]
 * Oblivious`), which @pkmn/client looks up as a condition and does not remember; Showdown's own
 * client reads any effect's name (`Dex.getEffect`). Read as the ability it names.
 */
function obliviousFrom(kwArgs) {
  if (kwArgs.from === 'Oblivious') kwArgs.from = 'ability: Oblivious';
}

/**
 * Clear Amulet's refusal names it, its holder keeping it (`-fail|holder|unboost|[from] item: Clear
 * Amulet|[of] holder`): @pkmn/client's handler looks for `clearamulet` where the effect's id reads
 * `item:clearamulet`, and files nothing (Showdown's own client takes the item).
 */
function clearAmuletFail(client, args, kwArgs) {
  if (args[0] !== '-fail' || kwArgs.from !== 'item: Clear Amulet') return;
  const poke = client.getPokemon(kwArgs.of || args[1]);
  if (poke) poke.item = 'clearamulet';
}

/**
 * Cheek Pouch heals its own holder, whoever's berry it ate: a berry Fling hands its target reads
 * `-heal|target|…|[from] ability: Cheek Pouch|[of] user` (the heal's source), which @pkmn/client
 * files under the user — @pkmn/protocol leaves the `[of]` out for the absorbing abilities alone
 * (`HEALING`). The line is read without it.
 */
function cheekPouchHeal(args, kwArgs) {
  if (args[0] === '-heal' && kwArgs.from === 'ability: Cheek Pouch') delete kwArgs.of;
}

/**
 * A berry Fling hands its target, eaten at once, heals it with the user's item: `-heal|target|…|
 * [from] item: Iapapa Berry|[of] user` right after the user's Fling. Showdown's `heal` names its
 * source as `[of]` whatever the item: a Shell Bell's heal names its holder's target, the item its
 * holder's.
 */
function flungHeal(moves, args, kwArgs) {
  return args[0] === '-heal' && !!kwArgs.from?.startsWith('item: ') && !!kwArgs.of && !!moves.mover
    && moves.moveName === 'Fling' && ident(kwArgs.of) === ident(moves.mover)
    && ident(kwArgs.of) !== ident(args[1]);
}

/**
 * Bug Bite and Pluck eat the target's berry (`-enditem|target|berry|[from] stealeat|[move] Bug
 * Bite|[of] user`), whose effect then names it on the eater (`-heal|user|…|[from] item: Mago
 * Berry`, no `[of]`): the berry was the target's.
 */
function stolenBerrysEffect(moves, args, kwArgs) {
  const named = `item: ${moves.stolen?.item}`;
  return !!moves.stolen && (kwArgs.from === named || (args[0] === '-activate' && args[2] === named))
    && !!args[1] && ident(args[1]) === moves.stolen.eater;
}

/**
 * A berry Fling hands its target works on the target at once (`-activate|target|item: Leppa
 * Berry|…`, a heal) before the user's `-enditem|user|berry|[from] move: Fling` names it: what the
 * client filed under the target from those lines goes back to what it knew before the Fling
 * (R-M10-41).
 */
function noteFlungItem(client, moves, args, kwArgs) {
  if (args[0] === 'move') {
    const target = args[2] === 'Fling' && args[3] ? client.getPokemon(args[3]) : null;
    moves.flung = target ? {target, item: target.item} : null;
  } else if (args[0] === '-enditem' && kwArgs.from === 'move: Fling' && moves.flung) {
    const {target, item} = moves.flung;
    if (toID(target.item) === toID(args[2]) && toID(item) !== toID(target.item)) target.item = item;
    moves.flung = null;
  }
}

/**
 * The item of the Pokémon a line about another's item names (`flungHeal`, `stolenBerrysEffect`),
 * as the client knew it: @pkmn/client files the berry under it.
 */
function itemKeptFromHeal(client, moves, args, kwArgs) {
  if (!flungHeal(moves, args, kwArgs) && !stolenBerrysEffect(moves, args, kwArgs)) return null;
  const poke = client.getPokemon(args[1]);
  return poke ? {poke, item: poke.item} : null;
}

/**
 * The last `move` line's user and move (`notePerishBody`'s attacker, `flungHeal`'s Fling), and the
 * berry its Bug Bite or Pluck ate (`stolenBerrysEffect`).
 */
function noteMover(moves, args, kwArgs) {
  if (args[0] === 'move') {
    moves.mover = args[1];
    moves.moveName = args[2];
    moves.stolen = null;
  } else if (args[0] === '-enditem' && kwArgs.from === 'stealeat' && kwArgs.of) {
    moves.stolen = {eater: ident(kwArgs.of), item: args[2]};
  }
}

/**
 * A Micle Berry's boost spent on a move (`-enditem|p|Micle Berry`, nothing else on the line: the
 * berry was eaten before, `[eat]`) names a berry long gone, which @pkmn/client reads as the item
 * the Pokémon holds now going (one Bestow handed it since). What it knows of the item is kept.
 * So with a resist berry's `-enditem|p|Chilan Berry|[weaken]`: Showdown writes it after the berry
 * was eaten (`[eat]`), and a Symbiosis partner may have handed the holder its item in between
 * (R-M11-73).
 */
function micleSpent(client, args, kwArgs) {
  if (args[0] !== '-enditem') return null;
  const micle = args[2] === 'Micle Berry' && !Object.keys(kwArgs).length;
  if (!micle && !kwArgs.weaken) return null;
  const poke = client.getPokemon(args[1]);
  if (!poke) return null;
  const {item, itemEffect, lastItem, lastItemEffect} = poke;
  return {poke, kept: {item, itemEffect, lastItem, lastItemEffect}};
}

/**
 * The move slots the client knows of the Pokémon a `move` line names, and of the one it stands
 * transformed into (which `rememberMove` passes the move on to), before it reads the line.
 */
function slotsOf(client, args) {
  if (args[0] !== 'move' && !(args[0] === 'cant' && args[3])) return undefined;
  const poke = client.getPokemon(args[1]);
  const slots = new Map();
  for (const p of [poke, poke?.volatiles?.transform?.pokemon]) {
    // Each move by its id, and by its id as a slot of the Pokémon's own or one it took by
    // Transform (`virtual`): a transformed Pokémon may have both.
    if (p) slots.set(p, new Set(p.moveSlots.flatMap(slot => [slot.id, `${slot.virtual ? 'virtual' : 'own'}:${slot.id}`])));
  }
  return slots;
}

/**
 * What @pkmn/client's moves need besides its handlers (DESIGN.md §47.0 item 13):
 * - a Z-Move's name after `-zpower` (a generic one) and a Dynamaxed Pokémon's Max Move hide the
 *   move they were made from; the client takes the name for a move of its own — left out; a
 *   signature Z-Move with no `-zpower` is a custom game's, in its slot, which the client takes
 *   for the move it is made from;
 * - a Round a partner's brought forward is its own action (`[from] move: Round`, PP paid), which
 *   the client takes for a call: counted — so is the move an Encore put in its place;
 * - Beak Blast's charge names the move (`-singleturn|p|move: Beak Blast`), like Focus Punch's;
 * - Forewarn and a Leppa or Mystery Berry name a move, which the client's handlers miss (they look
 *   for `forewarn`, `leppaberry` where the id reads `ability:forewarn`, `item:leppaberry`);
 * - Spite, Eerie Spell, G-Max Depletion and Grudge change a move's PP, which the client adds to its
 *   uses; Gen 2's locked turns and two-turn attacks print no `[from]`, which it counts as uses: the
 *   move is marked `uses_uncompared`; Lunar Dance's PP restored clears all of a Pokémon's counts,
 *   the entry marked `uses_uncompared`;
 * - Roost's volatile, which the client leaves off a user that is not Flying;
 * - Mimic (`-start|p|Mimic|move`; Gen 2-4 `-activate|p|move: Mimic|move`, Gen 4's Sketch too) and
 *   Sketch (`-activate|p|move: Sketch|move`) copy a move the client knows nothing of, then takes
 *   for one of its own: the entry is marked `moves_copied`.
 */
function noteMoves(client, moves, args, kwArgs, slotsBefore) {
  const poke = typeof args[1] === 'string' && /^p[1-4][a-d]?: /.test(args[1]) ? client.getPokemon(args[1]) : null;
  if (!poke) return;
  const key = poke.originalIdent ?? poke.ident;
  if (args[0] === '-zpower') {
    moves.zPowered.add(key);
  } else if (args[0] === 'move') {
    moves.snatcher = null;
    const move = Dex.moves.get(args[2]);
    const drop = id => {
      for (const [p, before] of slotsBefore ?? []) {
        if (!before.has(id)) p.moveSlots = p.moveSlots.filter(slot => slot.id !== id);
      }
    };
    // A Round or a Pledge brought forward (`[from] move: Round`, `[from] move: Grass Pledge`) is
    // the Pokémon's own action, which the client takes for a call — of the other Pledge, or of
    // the Round or Pledge an Encore then put another move in place of (`onOverrideAction`: the
    // action keeps its `sourceEffect`). Neither calls a move of its own.
    // (Gen 1's locks print `[from] Skull Bash`, a Pursuit at a switch `[from] Pursuit`: no `move:`.)
    const caller = kwArgs.from?.startsWith('move:') ? Dex.moves.get(kwArgs.from.slice(5).trim()) : null;
    const broughtForward = caller?.exists &&
      (caller.id === move.id || caller.id === 'round' || PLEDGES.includes(caller.id));
    if (broughtForward) {
      if (caller.id !== move.id) drop(caller.id);
      poke.rememberMove(move.name, 1);
    }
    // A call's own call (Assist → Nature Power → Tri Attack): the line names its caller, which
    // the client remembers as the Pokémon's (`rememberMove(caller)`) — a move itself called on
    // the Pokémon's line before (`moves.called`), no move of its own (a Gen 4 Shelgon, R-M10-70).
    if (caller?.exists && !broughtForward && moves.lastMove.get(key) === caller.id &&
      moves.called.has(key)) {
      drop(caller.id);
    }
    if (caller?.exists && !broughtForward) moves.called.add(key);
    else moves.called.delete(key);
    if (client.gen.num === 2 && (move.flags.charge || GEN2_LOCKS.includes(move.id))) {
      moves.uncompared.add(`${key}|${move.id}`);
      // A lock's later turn or a charge's attack after the Pokémon's line of the same move: the
      // move is not its own if that line's was not (a call's).
      if (!kwArgs.from && moves.lastMove.get(key) === move.id) drop(move.id);
    }
    moves.lastMove.set(key, move.id);
    const zPowered = moves.zPowered.has(key);
    moves.zPowered.delete(key);
    const from = move.isZ ? Dex.items.get(move.isZ).zMoveFrom : undefined;
    if ((move.isZ && zPowered && !from) || (move.isMax && poke.volatiles.dynamax)) {
      drop(move.id);
    } else if (move.isZ && !zPowered && from && !kwArgs.from) {
      // A signature Z-Move a custom game put in a slot, used as it is: the client takes it for
      // the move it is made from — a use of it, if that one was seen before. A transformed
      // Pokémon's is a slot it took (`virtual`), apart from its own of the same move (a Hakamo-o
      // that used Darkest Lariat, transformed, using the Malicious Moonsault it copied, R-M11-57).
      const base = toID(from);
      const virtual = !!poke.volatiles?.transform;
      const key = `${virtual ? 'virtual' : 'own'}:${base}`;
      const remembered = slot => slot.id === base && !!slot.virtual === virtual;
      const own = poke.moveSlots.find(slot => remembered(slot) && slotsBefore?.get(poke)?.has(key));
      if (own && typeof own.ppUsed === 'number') own.ppUsed = Math.max(0, own.ppUsed - 1);
      if (!slotsBefore?.get(poke)?.has(key)) poke.moveSlots = poke.moveSlots.filter(slot => !remembered(slot));
      drop(base);
      poke.rememberMove(move.name, 1);
    }
  } else if (args[0] === '-singleturn' && toID(args[2]) === 'movebeakblast') {
    poke.rememberMove('Beak Blast', 0);
  } else if (args[0] === '-singleturn' && toID(args[2]) === 'moveroost') {
    // Roost is on whatever the user's types (Showdown's `roost` volatile); the client leaves it
    // off one that is not Flying.
    poke.addVolatile('roost', {duration: 'turn'});
  } else if (args[0] === '-activate') {
    const effect = toID(args[2]).replace(/^(move|item|ability)/, '');
    if (['spite', 'eeriespell', 'gmaxdepletion', 'grudge'].includes(effect) && kwArgs.move) {
      moves.uncompared.add(`${key}|${toID(kwArgs.move)}`);
    } else if (['leppaberry', 'mysteryberry'].includes(effect) && kwArgs.move) {
      poke.rememberMove(kwArgs.move, 0);
    } else if (effect === 'forewarn' && kwArgs.move) {
      // Gen 5+ names the foe (`[of]`, read into `args[3]`); Gen 4 only a single foe gives it away.
      const foes = poke.side.foe.active.filter(foe => foe && !foe.fainted);
      const foe = args[3] ? client.getPokemon(args[3]) : foes.length === 1 ? foes[0] : null;
      foe?.rememberMove(kwArgs.move, 0);
    } else if (effect === 'sketch' || effect === 'mimic') {
      moves.copied.add(key);
    } else if (effect === 'snatch') {
      moves.snatcher = key;
    }
  } else if (args[0] === 'cant' && args[3] && moves.snatcher === key) {
    // The move Snatch just took, refused its snatcher (`cant|snatcher|move: Heal Block|Wish`, Heal
    // Block's `onModifyMove`): the client remembers it as the snatcher's; it is its user's
    // (R-M11-100).
    const id = toID(args[3]);
    if (!slotsBefore?.get(poke)?.has(id)) poke.moveSlots = poke.moveSlots.filter(slot => slot.id !== id);
    moves.snatcher = null;
  } else if (args[0] === '-start' && toID(args[2]).replace(/^move/, '') === 'mimic') {
    moves.copied.add(key);
  } else if (args[0] === '-heal' && toID(kwArgs.from ?? '') === 'movelunardance') {
    moves.reset.add(key);
  }
}

/**
 * As One shows itself, then the part at work (`-ability|p|As One`, `-ability|p|Unnerve`): the
 * client takes the part for the ability — of a Pokémon that transformed into an As One holder too
 * (Imposter), whose base ability is another. So it does with the part's raise a knock-out brings
 * (`-ability|p|Chilling Neigh|boost`, Grim Neigh's), which Chilling Neigh's own holder shows the
 * same way: one the client knew to have As One before the line (`asOne`, R-M10-52). The ability
 * stays As One.
 */
function keepAsOne(client, args, previous, asOne) {
  if (args[0] !== '-ability' || !['Unnerve', 'Chilling Neigh', 'Grim Neigh'].includes(args[2])) return;
  const shown = previous?.[0] === '-ability' && previous[2] === 'As One' && ident(previous[1]) === ident(args[1]);
  const poke = shown ? client.getPokemon(args[1]) : asOne;
  if (poke) poke.ability = 'asone';
}

/** The Pokémon a knock-out's `-ability|p|Chilling Neigh|boost` names, where the client has its ability as As One. */
function asOneBefore(client, args) {
  if (args[0] !== '-ability' || args[3] !== 'boost' || !['Chilling Neigh', 'Grim Neigh'].includes(args[2])) return null;
  const poke = client.getPokemon(args[1]);
  return poke?.ability === 'asone' ? poke : null;
}

/**
 * Gen 1 Mimic draws among its target's moves (`this.sample(target.moves)`): the move its `-start`
 * names is one the target has now (R-M10-43). @pkmn/client files it on the user alone; the
 * target of the `move` line before is remembered to have it too (a transformed one passes it on
 * to the one it transformed into, as its own moves are that one's).
 */
function noteMimicDraw(client, args, previous) {
  if (client.gen.num !== 1 || args[0] !== '-start' || args[2] !== 'Mimic' || !args[3]) return;
  if (previous?.[0] !== 'move' || previous[2] !== 'Mimic' || !previous[3]) return;
  client.getPokemon(previous[3])?.rememberMove(args[3], 0);
}

/**
 * Gen 6+ Psych Up hands its user the target's critical-hit volatiles in place of its own
 * (`volatilesToCopy`), which @pkmn/client reads off the `-copyboost` for Focus Energy, G-Max
 * Chi Strike and Laser Focus — Dragon Cheer too in Showdown's list (R-M10-45).
 */
function copyDragonCheer(client, args) {
  if (args[0] !== '-copyboost' || client.gen.num < 9) return;
  const poke = client.getPokemon(args[1]);
  const from = client.getPokemon(args[2]);
  if (!poke || !from) return;
  if (from.volatiles.dragoncheer) poke.addVolatile('dragoncheer');
  else poke.removeVolatile('dragoncheer');
}

/**
 * Gen 6+ Transform hands its user the target's critical-hit volatiles in place of its own
 * (`transformInto`'s `volatilesToCopy`: Dragon Cheer, Focus Energy, G-Max Chi Strike, Laser
 * Focus), with no line of its own; @pkmn/client's `-transform` copies none (a Gen 8 Beheeyem's
 * Focus Energy, gone as it transformed into an Elgyem, R-M10-70).
 */
function transformCritVolatiles(client, args) {
  if (args[0] !== '-transform' || client.gen.num < 6) return;
  const poke = client.getPokemon(args[1]);
  const from = client.getPokemon(args[2]);
  if (!poke || !from) return;
  for (const id of ['dragoncheer', 'focusenergy', 'gmaxchistrike', 'laserfocus']) {
    if (from.volatiles[id]) poke.volatiles[id] = {...from.volatiles[id]};
    else delete poke.volatiles[id];
  }
}

/**
 * The gen1 mod's `boost` names the stages asked for, not those that took (`-boost|p|def|2` at
 * +5), which @pkmn/client adds as they come: a stage stays within -6 to 6 (R-M10-43). So does
 * one set outright: the gen4 mod's Anger Point prints `-setboost|p|atk|12` (R-M11-36).
 */
function clampBoost(client, args) {
  if (args[0] !== '-boost' && args[0] !== '-unboost' && args[0] !== '-setboost') return;
  const poke = client.getPokemon(args[1]);
  const stat = args[2];
  if (!poke || typeof poke.boosts?.[stat] !== 'number') return;
  poke.boosts[stat] = Math.max(-6, Math.min(6, poke.boosts[stat]));
}

/**
 * Primal Reversion names the orb it reverts by (`-primal|p|Blue Orb`), which @pkmn/client, with
 * no `-primal` handler, does not read: the holder's item is known, as a Mega Stone `-mega` names
 * is (R-M11-36).
 */
function notePrimalOrb(client, args) {
  if (args[0] !== '-primal' || !args[2]) return;
  const poke = client.getPokemon(args[1]);
  if (poke) poke.item = toID(args[2]);
}

/**
 * A Terastallized Pokémon stays so for the battle (Showdown never clears `terastallized`), a
 * faint included: one Revival Blessing brings back is still Terastallized. @pkmn/client's
 * `faint` forgets it, as though the Pokémon could not come back: it is kept as it faints and
 * given back as it is revived (`-heal|p2: Braixen|155/310|[from] move: Revival Blessing`,
 * R-M11-52).
 */
function noteFallenTera(client, fallenTera, args) {
  if (args[0] !== 'faint') return;
  const poke = client.getPokemon(args[1]);
  if (poke?.terastallized) fallenTera.set(poke, poke.terastallized);
}

function restoreRevivedTera(client, fallenTera, args, kwArgs) {
  if (args[0] !== '-heal' || kwArgs.from !== 'move: Revival Blessing') return;
  const poke = client.getPokemon(args[1]);
  if (poke && !poke.terastallized && fallenTera.has(poke)) poke.terastallized = fallenTera.get(poke);
}

/**
 * Revival Blessing names the Pokémon it brings back by its own name (`-heal|p1: Dondozo|…|[from]
 * move: Revival Blessing`). One that fell in an Illusion's disguise was never seen under it: its
 * entry is the team preview's, with no ident, and @pkmn/client's `getPokemon` finds nothing (the
 * `-heal` threw). The preview entry of that name takes the ident before the line is read.
 */
function identifyRevived(client, args, kwArgs) {
  if (args[0] !== '-heal' || kwArgs.from !== 'move: Revival Blessing') return;
  if (typeof args[1] !== 'string' || !/^p[1-4]: /.test(args[1]) || client.getPokemon(args[1], true)) return;
  const side = client[args[1].slice(0, 2)];
  const name = args[1].slice(4);
  const entry = side?.team.find(p => !p.originalIdent && p.name === name);
  if (entry) entry.originalIdent = args[1];
}

/**
 * Gen 4 Haze takes Focus Energy off everyone standing with no word of its own (the gen4 mod's
 * `removeVolatile('focusenergy')`): its `-clearallboost` shows it, which the client reads for
 * the stages alone.
 */
function noteHaze(client, args) {
  if (args[0] !== '-clearallboost' || client.gen.num !== 4) return;
  for (const side of [client.p1, client.p2]) {
    for (const poke of side.active) poke?.removeVolatile('focusenergy');
  }
}

const PLEDGES = ['firepledge', 'grasspledge', 'waterpledge'];

/** Gen 2's locks (`lockedmove`, Rollout's, Bide's), whose later turns print no `[from]`. */
const GEN2_LOCKS = ['outrage', 'thrash', 'petaldance', 'rollout', 'bide', 'rage'];

/**
 * A player's Pokémon transformed into one of the other side's: @pkmn/client files the moves its
 * request lists under that Pokémon (`mpoke.moveSlots = …`), all of them, in their slots' order,
 * with the PP the copies used for its uses — none at all for a locked request, which lists the
 * locked move alone with no PP; what it knew of them before is kept (`movesTaken`). The other
 * side's entry is marked `moves_from_request`.
 */
function noteMovesFromRequest(client, moves, observer, kept) {
  for (const [into, slots] of kept) {
    for (const slot of into.moveSlots) {
      // A request's slot is named by `move`, not `name`, which `rememberMove` matches on.
      slot.name ??= Dex.moves.get(slot.move ?? slot.id).name;
    }
    for (const slot of slots) {
      if (!into.moveSlots.some(own => own.name === slot.name && own.virtual === slot.virtual)) into.moveSlots.push(slot);
    }
    moves.fromRequest.add(into.originalIdent ?? into.ident);
  }
}

/** The other side's Pokémon the player's own stand transformed into, with what the client knows of their moves. */
/**
 * A Pokémon's `searchid` (its ident and details) gains `, tera:<type>` with its `-terastallize`
 * (@pkmn/client), while Showdown's request goes on naming its details without it: the client's
 * `update` then finds no such Pokémon among the observer's own and makes a new one, its volatiles
 * and boosts lost (a Magmar's Focus Energy, which a foe's Psych Up then copied as none, R-M11-34).
 * The observer's own are matched without the suffix for the update and given it back after — a
 * later `switch` line names it.
 */
function teraSearchIds(client, observer, request) {
  const own = observer === 'p1' ? client.p1 : observer === 'p2' ? client.p2 : null;
  if (!own || !request.side) return [];
  const restored = [];
  for (const p of request.side.pokemon) {
    const member = own.team.find(m => m.searchid !== p.searchid
      && m.searchid?.replace(/, tera:[^,|]*$/, '') === p.searchid);
    if (!member) continue;
    restored.push([member, member.searchid, member.details]);
    member.searchid = p.searchid;
  }
  return restored;
}

function restoreTeraSearchIds(restored) {
  for (const [member, searchid, details] of restored) {
    member.searchid = searchid;
    member.details = details;
  }
}

function movesTaken(client, observer) {
  const side = observer === 'p1' ? client.p1 : client.p2;
  const kept = new Map();
  for (const poke of side.active) {
    const into = poke?.volatiles?.transform?.pokemon;
    if (into && into.side !== side) kept.set(into, [...into.moveSlots]);
  }
  return kept;
}

/**
 * Entries of one name on a side (an Illusion's disguise filed apart by the details its wearer's
 * level gives it): which one the client puts a line under is its guess.
 */
function filedApart(side, p) {
  return side.team.some(other => other !== p && other.name === p.name);
}

/**
 * The moves an observer knows a Pokémon has now: its own; transformed, the ones it took (the
 * client's `virtual` slots) and those known of the one it transformed into since, which the client
 * copies only as it transforms.
 */
function knownMoves(p) {
  const transformed = p.volatiles?.transform?.pokemon;
  const slots = (p.moveSlots ?? []).filter(slot => !!slot.virtual === !!transformed);
  for (const slot of transformed?.moveSlots ?? []) {
    if (!slot.virtual) slots.push({id: slot.id, ppUsed: 0, virtual: true});
  }
  // One a move: a slot a request filled is named otherwise, and the client adds the move again.
  return slots.filter((slot, index) => slots.findIndex(other => other.id === slot.id) === index);
}

/**
 * Volatiles @pkmn/client drops that Showdown keeps (DESIGN.md §47.0 item 13): a move Magic Coat or
 * Magic Bounce sends back is no action of the reflector's (`useMove` alone, no BeforeMove), yet
 * the client clears its `-singlemove` ones (`clearMovestatuses`); a status takes off a Yawn
 * (`-status`), which stays on until its turns run out (`-end|p|move: Yawn|[silent]`).
 */
function volatilesKept(client, args, kwArgs) {
  const reflected = args[0] === 'move' && ['ability: Magic Bounce', 'move: Magic Coat'].includes(kwArgs.from);
  if (!reflected && args[0] !== '-status') return null;
  const poke = client.getPokemon(args[1]);
  if (!poke) return null;
  const kept = Object.entries(poke.volatiles).filter(([id, volatile]) => (reflected ? volatile.duration === 'move' : id === 'yawn'));
  return kept.length ? [poke, kept] : null;
}

function restoreVolatiles(kept) {
  if (!kept) return;
  const [poke, entries] = kept;
  for (const [id, volatile] of entries) poke.volatiles[id] = volatile;
}

/**
 * Gen 2-4: a transformed Pokémon can transform again (with the Transform it took). @pkmn/client
 * adds the new copy to the old one's slots, which the Pokémon has no more: they go first. Gen 1
 * transforms into a transformed Pokémon, and Transform copies a move Mimic put in place: what the
 * client copies of those is no oracle — the entry is marked `moves_copied` too.
 */
function dropOldCopy(client, moves, args) {
  if (args[0] !== '-transform') return;
  const poke = client.getPokemon(args[1]);
  const into = client.getPokemon(args[2]);
  if (poke?.volatiles?.transform) poke.moveSlots = poke.moveSlots.filter(slot => !slot.virtual);
  if (poke && into && (into.volatiles?.transform || moves.copied.has(into.originalIdent ?? into.ident))) {
    moves.copied.add(poke.originalIdent ?? poke.ident);
    // The client passes what each copies on along the chain, to the other.
    if (into.volatiles?.transform) moves.copied.add(into.originalIdent ?? into.ident);
  }
}

/**
 * A Transform into one of the observer's own Pokémon copies the ability @pkmn/client has filed for
 * it, which it leaves blank at times between requests: the observer knows its own, filled in from
 * the battle before the copy (R-M10-48).
 */
function ownAbilityBeforeTransform(client, battle, name, args) {
  if (args[0] !== '-transform' || !args[2]?.startsWith(name)) return;
  const target = client.getPokemon(args[2]);
  if (!target || target.ability) return;
  const side = name === 'p1' ? battle.p1 : name === 'p2' ? battle.p2 : null;
  const own = side?.active[args[2].charCodeAt(2) - 'a'.charCodeAt(0)];
  if (own) target.ability = own.ability;
}

/**
 * Gen 1: a Pokémon transforming into a transformed one takes the moves that one has now, those it
 * took (its `virtual` slots). @pkmn/client's `-transform` copies its own slots as well, which
 * `rememberMove` passes on along the chain to the one it transformed into, which never had them
 * (R-M10-41): they are out of sight while the client reads the line.
 */
function ownSlotsHidden(client, args) {
  if (args[0] !== '-transform') return null;
  const into = client.getPokemon(args[2]);
  if (!into?.volatiles?.transform) return null;
  const own = into.moveSlots.filter(slot => !slot.virtual);
  into.moveSlots = into.moveSlots.filter(slot => slot.virtual);
  return {into, own};
}

function restoreOwnSlots(hidden) {
  if (hidden) hidden.into.moveSlots = [...hidden.own, ...hidden.into.moveSlots];
}

/**
 * A player knows the moves of its own Pokémon, one transformed into one of the other side's
 * included — which are that Pokémon's (its slots in Showdown): filed under that Pokémon as it
 * transforms (the client waits for a request listing them, which may never come).
 */
function noteOwnCopy(client, moves, observer, battle, args) {
  if (args[0] !== '-transform' || !args[1].startsWith(observer) || args[2]?.startsWith(observer)) return;
  const into = client.getPokemon(args[2]);
  const copied = battle.getPokemon(ident(args[2]));
  if (!into || !copied) return;
  for (const moveSlot of copied.baseMoveSlots ?? copied.moveSlots) {
    if (!into.moveSlots.some(own => own.id === moveSlot.id && !own.virtual)) {
      into.moveSlots.push({id: moveSlot.id, name: moveSlot.move, ppUsed: 0, virtual: false});
    }
  }
  moves.fromRequest.add(into.originalIdent ?? into.ident);
}

/**
 * The item the client holds for the Pokémon a `move`, `-item` or `-enditem` line names, before it
 * reads the line.
 */
function itemOf(client, args) {
  return ['move', '-item', '-enditem'].includes(args[0])
    ? client.getPokemon(args[1])?.item
    : undefined;
}

/**
 * @pkmn/client takes a Z-Move's use for its Z-Crystal (`useMove`: `this.item = move.isZ`), an
 * inference, wrong for a Z-Move a custom game put in a slot: the entry is marked
 * `item_from_z_move` until an item line changes the item the client holds or names it. One that
 * does neither — a flung Micle Berry's boost ending, `-enditem|p|Micle Berry`, while the inferred
 * crystal stays — says nothing of it (R-M11-17).
 */
function noteZItem(client, zItems, args, itemBefore) {
  if (args[0] === 'move') {
    const poke = client.getPokemon(args[1]);
    if (poke && poke.item !== itemBefore && Dex.moves.get(args[2]).isZ) {
      zItems.add(poke.originalIdent ?? poke.ident);
    }
  } else if (args[0] === '-item' || args[0] === '-enditem') {
    const poke = client.getPokemon(args[1]);
    const named = Dex.items.get(args[2]).id;
    if (poke && (poke.item !== itemBefore || named === poke.item)) {
      zItems.delete(poke.originalIdent ?? poke.ident);
    }
  }
}

/**
 * An ability named on a line @pkmn/client's handler reads no ability from (its other boost lines
 * do): Anger Point's `-setboost` (`[from] ability: Anger Point`), and Gen 3-5 Oblivious ending
 * Attract with no `-activate` of its own (`-end|p|move: Attract|[from] ability: Oblivious`, the
 * gen5 mod's; Showdown's own client shows it, `maybeAbility(kwArgs.from, kwArgs.of || pokemon)`,
 * R-M10-50). Taken here.
 */
function rememberAbilityFrom(client, args, kwArgs) {
  if (!kwArgs.from?.startsWith('ability:')) return;
  let holder;
  if (args[0] === '-setboost') holder = args[1];
  else if (args[0] === '-end' && !kwArgs.silent) holder = kwArgs.of || args[1];
  else return;
  client.getPokemon(holder)?.rememberAbility(kwArgs.from.slice('ability:'.length).trim());
}

/**
 * An item seen at work that @pkmn/client does not record: on a `-damage` / `-heal … [from] item:
 * Sticky Barb` line because its holder used up an item before (`CONSUMED.includes(lastItemEffect)`,
 * meant for a berry's heal after it was eaten — the holder got another since, Sticky Barb passing
 * on); on `-activate|p|item: Focus Band` because its handler looks for `focusband` where the id
 * reads `item:focusband` (Quick Claw's, Ability Shield's too), and on the `-block` @pkmn/protocol
 * makes of Protective Pads' and Safety Goggles' `-activate` likewise (R-M10-44). Taken here, but
 * for the item it used up.
 */
function rememberItemAtWork(client, moves, args, kwArgs) {
  let poke, name;
  if ((args[0] === '-damage' || args[0] === '-heal') && kwArgs.from?.startsWith('item:')) {
    // A flung berry's heal is the user's item, used up (`flungHeal`); a berry Bug Bite ate, the
    // target's (`stolenBerrysEffect`).
    if (flungHeal(moves, args, kwArgs) || stolenBerrysEffect(moves, args, kwArgs)) return;
    poke = (args[0] === '-damage' && client.getPokemon(kwArgs.of)) || client.getPokemon(args[1]);
    name = kwArgs.from;
  } else if ((args[0] === '-activate' || args[0] === '-block') && args[2]?.startsWith('item:')) {
    if (stolenBerrysEffect(moves, args, kwArgs)) return;
    poke = client.getPokemon(args[1]);
    name = args[2];
  } else if (args[0] === '-item' && args[1]) {
    // An item the client's data for the generation lacks (a custom game's Strawberry Sweet).
    poke = client.getPokemon(args[1]);
    name = args[2];
  } else {
    return;
  }
  const item = itemId(name);
  if (poke && !poke.item && itemId(poke.lastItem) !== item) poke.item = item;
}

/**
 * A Pokémon's own ability (@pkmn/client's base) where a line changes one (DESIGN.md §47.0 item 16,
 * R-M10-39). Showdown's `setAbility` writes `-ability|p|new|old|[from] effect|[of] source`: the
 * old is its own while its ability has not changed since it came in, and `source` has the new
 * one, its own on the same terms. @pkmn/client tracks no such change: it files the old as the
 * base only when it knows none, and the source's new one likewise (Role Play on a Pokémon that
 * Role Played before), or the new one as the base of a line naming no old (Worry Seed's older
 * form). The bases are set here as the engine keeps them, the changed ones (`changed`) known
 * since they came in — an ability line, Mummy's `-activate`, Skill Swap's, a Transform, a forme's.
 */
function basesOf(client, args, kwArgs) {
  if (args[0] === '-ability' && kwArgs.from) {
    const poke = client.getPokemon(args[1]);
    const of = kwArgs.of ? client.getPokemon(kwArgs.of) : null;
    return {poke, base: poke?.baseAbility, of, ofBase: of?.baseAbility};
  }
  if (args[0] === '-activate' && ['ability: Mummy', 'ability: Lingering Aroma'].includes(args[2])) {
    const poke = client.getPokemon(args[3]);
    return {poke, base: poke?.baseAbility, of: null, ofBase: undefined};
  }
  // A suppressed ability (`-endability`: Gastro Acid, Core Enforcer), which the client blanks.
  if (args[0] === '-endability') {
    const poke = client.getPokemon(args[1]);
    return {poke, base: poke?.baseAbility, ability: poke?.ability, ended: true};
  }
  // Baton Pass and Shed Tail leave the outgoing one's ability as the client found it.
  if (args[0] === 'switch' && ['Baton Pass', 'Shed Tail'].includes(kwArgs.from)) {
    const {player, position} = Protocol.parsePokemonIdent(args[1]);
    const side = client[player];
    const slot = position ? position.charCodeAt(0) - 'a'.charCodeAt(0) : 0;
    const outgoing = side?.active[slot];
    return outgoing ? {poke: null, passed: outgoing} : undefined;
  }
  if (args[0] === '-activate' && ['Skill Swap', 'move: Skill Swap'].includes(args[2])) {
    const poke = client.getPokemon(args[1]);
    const of = kwArgs.of ? client.getPokemon(kwArgs.of) : null;
    return {poke, base: poke?.baseAbility, of, ofBase: of?.baseAbility, swap: true};
  }
  return undefined;
}

function noteAbilityChange(client, changed, args, kwArgs, before) {
  if (['switch', 'drag'].includes(args[0])) {
    // Showdown's `clearVolatile` on the one leaving resets its ability to its own, a Baton
    // Pass's too, which @pkmn/client's `switchOut` skips.
    if (before?.passed && before.passed !== client.getPokemon(args[1])) {
      before.passed.ability = before.passed.baseAbility;
      changed.delete(before.passed);
    }
    const poke = client.getPokemon(args[1]);
    if (poke) changed.delete(poke);
    return;
  }
  // The Pokémon has the ability still, suppressed (the view names it).
  if (before?.ended) {
    if (before.poke) before.poke.ability = toID(args[2] ?? '') || before.ability;
    return;
  }
  if (['-transform', 'detailschange'].includes(args[0])) {
    const poke = client.getPokemon(args[1]);
    if (poke) changed.add(poke);
    return;
  }
  if (!before?.poke) return;
  const {poke, base, of, ofBase} = before;
  if (before.swap) {
    // Gen 5+ between foes the line names the two; the client files each one's as its base.
    for (const [p, b] of [[poke, base], [of, ofBase]]) {
      if (!p) continue;
      if (changed.has(p)) p.baseAbility = b;
      changed.add(p);
    }
    return;
  }
  if (args[0] === '-activate') {
    // Mummy, Lingering Aroma (`-activate|holder|ability: Mummy|attacker|[ability] old`): the
    // attacker has it now, its old one its own while unchanged. @pkmn/client's handler never
    // runs — it looks the effect up as `ability:mummy`, its case is `mummy` — where Showdown's
    // own client (`Dex.getEffect`) does.
    poke.ability = toID(args[2].replace('ability: ', ''));
    poke.baseAbility = changed.has(poke) ? base : (toID(kwArgs.ability ?? '') || base);
    changed.add(poke);
    return;
  }
  const old = args[3] ? toID(args[3]) : '';
  if (!old) {
    // A line that names no old one changes nothing it shows of the Pokémon's own, but the
    // `[fail]` of a weather ability a strong weather keeps off (`-ability|p|Drought|[from]
    // desolateland|[fail]`), which names its own while unchanged.
    poke.baseAbility = kwArgs.fail && !changed.has(poke) ? toID(args[2]) || base : base;
    return;
  }
  poke.baseAbility = changed.has(poke) ? base : old;
  changed.add(poke);
  if (of && of !== poke) of.baseAbility = changed.has(of) ? ofBase : toID(args[2]);
}


/**
 * Bestow gives the giver's item away (`-item|target|item|[from] move: Bestow|[of] giver`): the
 * giver holds nothing after, which @pkmn/client's handler leaves out.
 */
function noteBestowed(client, args, kwArgs) {
  if (args[0] !== '-item' || kwArgs.from !== 'move: Bestow') return;
  const giver = client.getPokemon(kwArgs.of);
  if (!giver) return;
  giver.item = '';
  giver.lastItem = itemId(args[2]);
}

/**
 * What Team Preview showed of each side (`|poke|side|details|item`), in order: the species (a
 * hidden forme, `Urshifu-*`, as its base species, `forme_hidden`; a cosmetic forme as its base
 * species, as the engine names it), level, gender and item mark.
 */
function notePreview(previews, args) {
  if (args[0] === 'clearpoke') {
    for (const side of ['p1', 'p2']) previews[side].pokes = [];
  } else if (args[0] === 'poke' && ['p1', 'p2'].includes(args[1])) {
    const details = Protocol.parseDetails('', '', args[2]);
    const hidden = details.speciesForme.endsWith('-*');
    previews[args[1]].pokes.push({
      species: battleSpecies(hidden ? details.speciesForme.slice(0, -2) : details.speciesForme),
      forme_hidden: hidden,
      level: details.level ?? 100,
      gender: details.gender ?? '',
      item: args[3] ?? '',
    });
  }
}

/**
 * Open team sheets (`|showteam|side|packed team`, Showdown's `showOpenTeamSheets`; DESIGN.md
 * §47.0 item 14), which @pkmn/client has no handler for: read as Showdown's own client reads them
 * (`case 'showteam'`) — the side's members filed anew from the sheets, each with its item, ability
 * and moves, none seen used. The packed team's own `|` split the line: its fields are joined back.
 * The sheets are kept as they were shown (`preview.sheet`).
 */
function noteShowteam(client, previews, args) {
  if (args[0] !== 'showteam' || client.turn !== 0 || !['p1', 'p2'].includes(args[1])) return;
  // The packed names (`AlcremieRubyCream`, unpacked `Alcremie Ruby Cream`) as Showdown names them.
  const team = Teams.unpack(args.slice(2).join('|'))?.map(set => ({
    ...set,
    species: Dex.species.get(set.species).name,
    item: set.item ? Dex.items.get(set.item).name : '',
    ability: set.ability ? Dex.abilities.get(set.ability).name : '',
    moves: set.moves.map(move => Dex.moves.get(move).name),
  }));
  if (!team?.length) return;
  previews[args[1]].sheet = team.map(set => ({
    species: battleSpecies(set.species),
    item: itemId(set.item),
    ability: toID(set.ability),
    moves: set.moves.map(toID),
    gender: set.gender && set.gender !== 'N' ? set.gender : '',
    level: set.level || 100,
    tera_type: set.teraType ?? '',
  }));
  client.getSide(args[1]).clearPokemon();
  for (const set of team) {
    // A forme that battles under its base species' name (`getUpdatedDetails`: Greninja-Bond,
    // Rockruff-Dusk) filed so, or the client files it apart from the Pokémon that comes in
    // (R-M10-40).
    const species = HIDDEN_FORMES.includes(set.species) ? Dex.species.get(set.species).baseSpecies : set.species;
    const details = species + (!set.level || set.level === 100 ? '' : `, L${set.level}`) +
      (!set.gender || set.gender === 'N' ? '' : `, ${set.gender}`);
    const pokemon = client.rememberTeamPreviewPokemon(args[1], details);
    if (set.item) pokemon.item = set.item;
    if (set.ability) pokemon.rememberAbility(set.ability);
    for (const move of set.moves) pokemon.rememberMove(move, 0);
  }
}

/**
 * Perish Body (`-ability|holder|Perish Body`, "Both Pokémon will faint in three turns!") starts the
 * perish count of the Pokémon whose move made contact — the last `move` line's — and of the holder
 * still standing, with no `-start` line of its own (Showdown's `addVolatile('perishsong')`, not the
 * move's `-start|…|perish3`), which @pkmn/client leaves to the next turn's end: the line says it.
 */
function notePerishBody(client, moves, args, kwArgs) {
  if (args[0] !== '-ability' || args[2] !== 'Perish Body' || kwArgs.from) return;
  for (const ident of [moves.mover, args[1]]) {
    const poke = ident && client.getPokemon(ident);
    if (poke && poke.hp > 0 && !poke.volatiles.perishsong) poke.addVolatile('perishsong', {duration: 3});
  }
}

/**
 * An item Trick or Switcheroo hands over, Bestow gives, or Thief, Covet, Magician or Pickpocket
 * takes, that is used at once (a White Herb on lowered stats, a Seed on its terrain: the item's start, inside
 * `setItem`): Showdown writes its `-enditem` and what it did before the `-item|p|item|[from] move:
 * Trick` of the handover, which @pkmn/client reads as holding it again. The Pokémon holds nothing.
 * A handover runs from the swap's `-activate`, or a move's `move` line (its user the one Thief,
 * Covet or Magician hands the item, R-M10-59; its target the one Pickpocket or Bestow does,
 * R-M11-134, R-M11-144), to the next one or the next move. A berry eaten in it (`[eat]`) is the
 * Pokémon's own, eaten at the hit's `Update` before the handover: no berry is used inside
 * `setItem` (R-M11-145).
 */
function noteUsedOnReceipt(client, handover, args, kwArgs) {
  const swap = ['move: Trick', 'move: Switcheroo'];
  const take = ['move: Thief', 'move: Covet', 'move: Bestow', 'ability: Magician', 'ability: Pickpocket'];
  if (args[0] === 'move' || args[0] === '-activate') {
    const starts = args[0] === '-activate' && swap.includes(args[2]) && kwArgs.of;
    const moves = args[0] === 'move' ? [args[1], args[3]].filter(Boolean).map(ident) : null;
    handover.who = starts ? new Set([ident(args[1]), ident(kwArgs.of)]) : moves ? new Set(moves) : null;
    handover.used = new Map();
    return;
  }
  if (!handover.who) return;
  if (args[0] === '-enditem' && !kwArgs.from && !kwArgs.eat && handover.who.has(ident(args[1]))) {
    handover.used.set(ident(args[1]), args[2]);
  } else if (args[0] === '-item' && [...swap, ...take].includes(kwArgs.from) && handover.used.get(ident(args[1])) === args[2]) {
    const poke = client.getPokemon(args[1]);
    if (poke) poke.item = '';
  }
}

/**
 * Symbiosis hands the holder's item to an ally that used its own up (`-activate|holder|ability:
 * Symbiosis|item|[of] ally`, which @pkmn/protocol reads as `kwArgs.item`), which @pkmn/client's
 * handler leaves out: the ally holds it, the holder nothing (Showdown's own client, `case
 * 'symbiosis'`).
 */
function noteSymbiosis(client, args, kwArgs) {
  if (args[0] !== '-activate' || args[2] !== 'ability: Symbiosis' || !kwArgs.item || !kwArgs.of) return;
  const holder = client.getPokemon(args[1]);
  const ally = client.getPokemon(kwArgs.of);
  if (!holder || !ally) return;
  holder.item = '';
  holder.lastItem = itemId(kwArgs.item);
  ally.item = kwArgs.item;
}

/**
 * Gen 3-4 Skill Swap names neither ability (`-activate|p|Skill Swap|||[of] target`), and
 * @pkmn/client skips it: what it knows of either Pokémon's ability is stale from then on, and the
 * abilities it learns after are taken for their own. Both entries are marked
 * `ability_swapped_unseen` — and one that transforms into either, the stale ability the client
 * copies (`-transform`: a Gen 3 Ditto taking a Flame Body unseen swaps had moved on, R-M10-72).
 */
function noteHiddenSwap(client, hiddenSwaps, args, kwArgs) {
  if (args[0] === '-transform') {
    const poke = client.getPokemon(args[1]);
    const into = client.getPokemon(args[2]);
    if (poke && into && hiddenSwaps.has(into.originalIdent ?? into.ident)) {
      hiddenSwaps.add(poke.originalIdent ?? poke.ident);
    }
    return;
  }
  if (args[0] !== '-activate' || !/skill ?swap/i.test(args[2] ?? '') || kwArgs.ability || kwArgs.ability2) return;
  for (const ident of [args[1], kwArgs.of]) {
    const poke = client.getPokemon(ident);
    if (poke) hiddenSwaps.add(poke.originalIdent ?? poke.ident);
  }
}

/** An item id as Showdown's: @pkmn/client files one seen at work (`[from] item: Life Orb`)
 * under the condition's id (`itemlifeorb`). */
function itemId(id) {
  if (!id || Dex.items.get(id).exists) return id ?? '';
  const own = id.startsWith('item') ? Dex.items.get(id.slice(4)) : null;
  return own?.exists ? own.id : id;
}

/**
 * @pkmn/client's `replace` (an Illusion broken) resets the entry the disguise wore to full HP and
 * no status, forgetting what was seen of the Pokémon itself; the entry is marked
 * `illusion_reset` until that Pokémon is seen again.
 */
function noteIllusionReset(client, reset, args) {
  if (!['replace', 'switch', 'drag'].includes(args[0])) return;
  const side = args[1].startsWith('p1') ? client.p1 : client.p2;
  const slot = args[1].charCodeAt(2) - 'a'.charCodeAt(0);
  if (args[0] === 'replace') {
    const worn = side.active[slot];
    if (worn) reset.add(worn.originalIdent ?? worn.ident);
  } else {
    reset.delete(`${args[1].slice(0, 2)}: ${args[1].slice(args[1].indexOf(':') + 2)}`);
  }
}

/**
 * The item a Pokémon's entry was seen to take or lose since it came in (`-item`, `-enditem`): an
 * Illusion's holder that changed items in its disguise (a Trick) keeps what it took when the
 * Illusion breaks, as the view has it (DESIGN.md §47.0 item 11), while @pkmn/client leaves that on
 * the disguise's entry and the holder's with the item it knew before — an open team sheet's
 * (R-M11-61). Kept by entry, not by slot: an Ally Switch moves the disguise with its item
 * (R-M11-66). On `replace`, the holder's entry takes what the disguise's (`worn`) took.
 */
function noteEntryItem(client, masks, args, worn) {
  const position = args[1];
  if (typeof position !== 'string' || !/^p[1-4][a-d]: /.test(position)) return;
  if (args[0] === 'replace') {
    const holder = client.getPokemon(position);
    if (holder && worn && masks.entryItems.has(worn)) holder.item = masks.entryItems.get(worn);
    return;
  }
  const entry = client.getPokemon(position);
  if (!entry) return;
  if (args[0] === 'switch' || args[0] === 'drag') masks.entryItems.delete(entry);
  else if (args[0] === '-item') masks.entryItems.set(entry, toID(args[2]));
  else if (args[0] === '-enditem') masks.entryItems.set(entry, '');
}

/** The entry standing in a position (`p1a: …`), as the client has it before a line. */
function activeAt(client, position) {
  const side = position.startsWith('p1') ? client.p1 : client.p2;
  return side.active[position.charCodeAt(2) - 'a'.charCodeAt(0)] ?? null;
}

/**
 * Two of one Pokémon on the field, @pkmn/client may take the wrong one for the Illusion's holder
 * (`noteUnresolvedIllusion`); the `replace` that breaks the Illusion then finds the holder's
 * entry standing in the other slot and moves it over, leaving it in both. The next switch out of
 * that other slot empties the holder's (`switchOut` goes by the entry's slot), and a line naming
 * the holder there throws. The other slot takes back the entry the disguise wore (`worn`): the
 * Pokémon standing there is the one the holder was disguised as. The side stays unresolved.
 */
function untangleReplace(client, args, worn) {
  if (args[0] !== 'replace') return;
  const side = args[1].startsWith('p1') ? client.p1 : client.p2;
  const slot = args[1].charCodeAt(2) - 'a'.charCodeAt(0);
  const holder = side.active[slot];
  if (!holder || !worn || worn === holder || side.active.includes(worn)) return;
  side.active.forEach((p, i) => {
    if (i !== slot && p === holder) {
      side.active[i] = worn;
      worn.slot = i;
    }
  });
}

/**
 * What an Illusion's holder showed of its item and ability in a disguise @pkmn/client keeps on the
 * disguise's entry when the Illusion breaks (`replace`); the engine gives it back to the holder
 * (DESIGN.md §47.0 item 11). The disguise's entry is marked `illusion_worn`, the holder's
 * `illusion_unmasked`, for the rest of the battle.
 */
function noteMaskedKnowledge(client, masks, args) {
  if (args[0] !== 'replace') return;
  const side = args[1].startsWith('p1') ? client.p1 : client.p2;
  const worn = side.active[args[1].charCodeAt(2) - 'a'.charCodeAt(0)];
  if (worn) masks.worn.add(worn.originalIdent ?? worn.ident);
  masks.unmasked.add(ident(args[1]));
}

/**
 * A line that names a Pokémon in a slot another holds (Emergency Exit's `-activate|p2a: …` as it
 * stands in p2b), or brings one in that already stands in another slot (a Baton Pass or a drag
 * into the partner's slot): @pkmn/client files what follows under the wrong entry, or a new one.
 * The Pokémon named is marked `misplaced` for the rest of the battle.
 */
function noteMisplaced(client, misplaced, args) {
  const position = args[1];
  if (typeof position !== 'string' || !/^p[1-4][a-d]: /.test(position)) return;
  const side = position.startsWith('p1') ? client.p1 : client.p2;
  const slot = position.charCodeAt(2) - 'a'.charCodeAt(0);
  const name = position.slice(position.indexOf(':') + 2);
  if (['switch', 'drag', 'replace'].includes(args[0])) {
    if (side.active.some((p, at) => p && at !== slot && p.name === name)) misplaced.add(ident(position));
  } else if (side.active[slot] && side.active[slot].name !== name) {
    misplaced.add(ident(position));
  }
}

/** Each side's entries by their ident, before a line. */
function entriesOf(client) {
  const entries = new Map();
  for (const side of [client.p1, client.p2]) {
    for (const p of side.team) entries.set(p.originalIdent ?? p.ident, p);
  }
  return entries;
}

/**
 * @pkmn/client may file a Pokémon coming in under a new entry of the same name (it did not match
 * the details it had), forgetting what the old one knew. The ident is marked `entry_renewed`.
 */
function noteRenewed(client, renewed, before) {
  for (const [ident, entry] of entriesOf(client)) {
    if (ident && before.has(ident) && before.get(ident) !== entry) renewed.add(ident);
  }
}

/** `p2a: Haunter` → `p2: Haunter`. */
function ident(position) {
  return `${position.slice(0, 2)}: ${position.slice(position.indexOf(':') + 2)}`;
}

/**
 * @pkmn/client's own team is no oracle (`tests/support/views.rs`): stages a Pokémon of the other
 * side copies or swaps with one of the player's own (Psych Up, Heart Swap, Spectral Thief…), or
 * takes with the rest as it transforms into one (`-transform|user|target`), are copied from that.
 * Such an entry is marked `boosts_from_own` until it leaves the field or its stages are cleared.
 */
function noteOwnBoostsCopied(client, observer, copied, args) {
  if (['-copyboost', '-swapboost'].includes(args[0]) && args[2]) {
    for (const [target, source] of [[args[1], args[2]], [args[2], args[1]]]) {
      if (source.startsWith(observer) && !target.startsWith(observer)) copied.add(ident(target));
    }
  } else if (args[0] === '-transform' && args[2]) {
    if (args[2].startsWith(observer) && !args[1].startsWith(observer)) copied.add(ident(args[1]));
  } else if (['-clearallboost'].includes(args[0])) {
    copied.clear();
  } else if (['-clearboost'].includes(args[0])) {
    copied.delete(ident(args[1]));
  } else if (['switch', 'drag', 'replace'].includes(args[0])) {
    const side = args[1].startsWith('p1') ? client.p1 : client.p2;
    const leaving = side.active[args[1].charCodeAt(2) - 'a'.charCodeAt(0)];
    if (leaving) copied.delete(leaving.originalIdent ?? leaving.ident);
  }
}

/** Whether `args` brings a Pokémon in under the name of one standing on the field already. */
function twinOnField(client, args) {
  if (!['switch', 'drag'].includes(args[0])) return false;
  const side = args[1].startsWith('p1') ? client.p1 : client.p2;
  const slot = args[1].charCodeAt(2) - 'a'.charCodeAt(0);
  const name = args[1].slice(args[1].indexOf(':') + 2);
  return side.active.some((p, i) => p && i !== slot && p.name === name);
}

/**
 * Whether `args` brings a Pokémon in under the name of one seen fainting: that one was an
 * Illusion's holder in its guise, and @pkmn/client guesses which member fainted (a Zoroark or
 * Zorua, a known Illusion, else the first left standing) — right or wrong (R-M10-47).
 */
function fallenTwin(client, args) {
  if (!['switch', 'drag'].includes(args[0])) return false;
  const side = args[1].startsWith('p1') ? client.p1 : client.p2;
  const name = args[1].slice(args[1].indexOf(':') + 2);
  return side.team.some(p => p.fainted && p.name === name);
}

/**
 * Which Pokémon came in, when an Illusion's holder may have: the observers cannot tell. Two of one
 * Pokémon on the field at once tell them that one is the holder, not which (both came in as
 * it); one coming in under a name the holder was once seen through in may be the holder again,
 * or not. @pkmn/client guesses — the newcomer is the known holder of its level, or the entry it
 * last filed that name under — right or wrong, and with no request to go by (a spectator's) it
 * takes a twin for the one standing there and muddles the side. A side where it guessed (a twin,
 * or a newcomer filed under another name) is marked `unresolved_illusion` for the rest of the
 * battle.
 */
function noteUnresolvedIllusion(client, unresolved, args, twin) {
  if (!['switch', 'drag'].includes(args[0])) return;
  const side = args[1].startsWith('p1') ? client.p1 : client.p2;
  const entrant = side.active[args[1].charCodeAt(2) - 'a'.charCodeAt(0)];
  const name = args[1].slice(args[1].indexOf(':') + 2);
  if (twin || (entrant && entrant.name !== name)) unresolved.add(args[1].slice(0, 2));
}

/**
 * Whether `p` stands transformed into an Illusion's holder the client saw unmasked, into the
 * Pokémon an Illusion wore (`illusion_worn`: the moves its holder showed in the disguise the client
 * keeps on that entry, which a Transform into it copies — V-create from a Misdreavus disguised as
 * Groudon, R-M11-90), into one whose moves the client knows no oracle of (`moves_copied`), into an
 * entry filed apart from
 * another of its name (`filed_apart`: a disguise's moves the client keeps on the disguise's level,
 * the engine on the one it shows, R-M10-58), or into an entry the client made anew
 * (`entry_renewed`: it forgot the moves the old one knew — a Roselia whose name an Illusion's
 * holder fainted under, R-M11-37).
 */
function transformedIntoNoOracle(p, masks, moves, renewed) {
  const into = p.volatiles?.transform?.pokemon;
  const key = into && (into.originalIdent ?? into.ident);
  return !!into && (masks.unmasked.has(key) || masks.worn.has(key) || moves.copied.has(key)
    || filedApart(into.side, into) || renewed.has(key));
}

/**
 * Whether `p` stands transformed into an entry whose ability the client knows nothing true of:
 * one it made anew (`entry_renewed`: a Fezandipiti Roar dragged out and back, whose Toxic Chain
 * the old entry knew), filed wrong (`misplaced`) or apart (`filed_apart`), or an Illusion's
 * (`illusion_worn`, `illusion_unmasked`). `-transform` copies that entry's ability (R-M11-148).
 */
function transformedIntoStale(p, masks, misplaced, renewed) {
  const into = p.volatiles?.transform?.pokemon;
  const key = into && (into.originalIdent ?? into.ident);
  return !!into && (renewed.has(key) || misplaced.has(key) || masks.unmasked.has(key)
    || masks.worn.has(key) || filedApart(into.side, into));
}

/** One observer's knowledge, normalized. */
function view(client, reset, copied, unresolved, zItems, hiddenSwaps, masks, misplaced, renewed, moves, own, previews) {
  return {
    sides: [client.p1, client.p2].map(side => ({
      unresolved_illusion: unresolved.has(`p${side.n + 1}`),
      // Of a side the observer does not know, at the first checkpoint after the start alone.
      ...(previews && side !== own ? {preview: previews[`p${side.n + 1}`]} : {}),
      active: side.active.map(pokemon => pokemon ? side.team.indexOf(pokemon) : null),
      team: side.team.map(p => ({
        ...pokemon(p),
        illusion_reset: reset.has(p.originalIdent ?? p.ident),
        boosts_from_own: copied.has(p.originalIdent ?? p.ident),
        item_from_z_move: zItems.has(p.originalIdent ?? p.ident),
        ability_swapped_unseen: hiddenSwaps.has(p.originalIdent ?? p.ident),
        ability_copied_stale: transformedIntoStale(p, masks, misplaced, renewed),
        illusion_worn: masks.worn.has(p.originalIdent ?? p.ident),
        illusion_unmasked: masks.unmasked.has(p.originalIdent ?? p.ident),
        misplaced: misplaced.has(p.originalIdent ?? p.ident),
        entry_renewed: renewed.has(p.originalIdent ?? p.ident),
        // Transformed into an Illusion's holder unmasked (the moves it showed in the disguise the
        // client keeps on the disguise, R-M10-44), or into one whose moves are no oracle (a Mimic
        // it used since, which the client counts among that one's moves, R-M10-46): what the
        // client knows of the copy is no oracle either.
        moves_copied: moves.copied.has(p.originalIdent ?? p.ident) || transformedIntoNoOracle(p, masks, moves, renewed),
        moves_from_request: moves.fromRequest.has(p.originalIdent ?? p.ident),
        filed_apart: filedApart(side, p),
        transformed: !!p.volatiles?.transform,
        // Transformed into one of the observer's own: what the client knows of its moves is a copy
        // of its own team's, no oracle.
        transformed_into_own: !!own && p.volatiles?.transform?.pokemon?.side === own,
        uses_uncompared: moves.reset.has(p.originalIdent ?? p.ident),
        moves: knownMoves(p).map(slot => ({
          id: slot.id,
          pp_used: Number.isFinite(slot.ppUsed) ? slot.ppUsed : null,
          uses_uncompared: moves.uncompared.has(`${p.originalIdent ?? p.ident}|${slot.id}`),
        })),
      })),
      conditions: Object.keys(side.sideConditions ?? {}).sort(),
    })),
    // Showdown's ids, as the engine's frame names them (@pkmn/client keeps names of its own:
    // `Rain`, `Electric`).
    field: {
      weather: client.field.weather ? WEATHER_IDS[client.field.weather] ?? toID(client.field.weather) : null,
      terrain: client.field.terrain ? `${toID(client.field.terrain)}terrain` : null,
      pseudo_weather: Object.keys(client.field.pseudoWeather ?? {}).sort(),
    },
  };
}

/** The species it appears as: a forme it changed into, or the Pokémon it transformed into. */
function appearance(p) {
  return p.volatiles?.formechange?.speciesForme ?? p.speciesForme;
}

/** The formes that battle under their base species' name (Showdown `getUpdatedDetails`). */
const HIDDEN_FORMES = ['Greninja-Bond', 'Rockruff-Dusk'];

function battleSpecies(name) {
  const species = Dex.species.get(name);
  return species.exists ? (species.isCosmeticForme ? species.baseSpecies : species.name) : name;
}

function pokemon(p) {
  return {
    ident: p.originalIdent ?? p.ident ?? null,
    // As it appears, named as the engine's frame names it (`normalize.mjs`): a cosmetic forme
    // (Vivillon-Savanna) battles as its base species.
    species: appearance(p) ? battleSpecies(appearance(p)) : null,
    level: p.level ?? null,
    gender: p.gender ?? null,
    hp: p.hp,
    max_hp: p.maxhp,
    fainted: !!p.fainted,
    status: p.status || null,
    boosts: {...(p.boosts ?? {})},
    item: itemId(p.item),
    item_effect: p.itemEffect ?? '',
    last_item: itemId(p.lastItem),
    ability: p.ability ?? '',
    base_ability: p.baseAbility ?? '',
    volatiles: Object.keys(p.volatiles ?? {}).sort(),
    terastallized: p.terastallized ?? null,
  };
}
