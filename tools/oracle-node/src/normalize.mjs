import {toID} from '@pkmn/sim';

function boostProjection(boosts = {}) {
  return {
    attack: boosts.atk ?? 0,
    defense: boosts.def ?? 0,
    special_attack: boosts.spa ?? 0,
    special_defense: boosts.spd ?? 0,
    speed: boosts.spe ?? 0,
    accuracy: boosts.accuracy ?? 0,
    evasion: boosts.evasion ?? 0,
  };
}

function conditionMap(value = {}) {
  const out = {};
  for (const [key, state] of Object.entries(value)) {
    if (!state) continue;
    // Duration-based conditions report turns left; layered hazards report their layers; a
    // Substitute reports its remaining HP.
    const turns = typeof state.layers === 'number' ? state.layers :
      typeof state.hp === 'number' ? state.hp :
      typeof state.duration === 'number' ? state.duration :
      typeof state.time === 'number' ? state.time : null;
    out[key] = turns;
  }
  return out;
}

function pokemonProjection(pokemon) {
  const gen = pokemon.battle.gen;
  return {
    hp: pokemon.hp,
    max_hp: pokemon.maxhp,
    // `fnt` is Showdown's internal marker for a fainted Pokémon; the frame expresses that as hp 0.
    status: pokemon.status && pokemon.status !== 'fnt' ? {kind: pokemon.status} : null,
    boosts: boostProjection(pokemon.boosts),
    volatiles: conditionMap(pokemon.volatiles),
    // Since 2026-10-01 (SCHEMA.md): the types as the battle reads them, the ability the Pokémon
    // has (Gen 3+) and the item it holds (Gen 2+), as Showdown ids.
    types: pokemon.getTypes().map(type => type.toLowerCase().replace(/[^a-z0-9]/g, '')),
    ...(gen >= 3 ? {ability: pokemon.ability} : {}),
    ...(gen >= 2 ? {item: pokemon.item} : {}),
    // Since 2026-10-04 (SCHEMA.md): the species it battles as (`species.id`: a forme it
    // changed into, or the one it transformed into).
    // A cosmetic forme (Vivillon-Savanna) battles as its base species (DESIGN.md §45.5).
    species: pokemon.species.isCosmeticForme ? toID(pokemon.species.baseSpecies) : pokemon.species.id,
    // Since 2026-10-02 (SCHEMA.md): each move slot's PP, in move order (`moveSlots`: a
    // transformed Pokémon's copied moves).
    pp: pokemon.moveSlots.map(slot => slot.pp),
    // Since 2026-10-04 (DESIGN.md §44.8): the Tera type it took, present only while it has
    // Terastallized (Gen 9). A recording made before the field existed has none and the replay
    // does not compare it.
    ...(gen === 9 && pokemon.terastallized
      ? {terastallized: pokemon.terastallized.toLowerCase()}
      : {}),
  };
}

function requestKind(side) {
  const request = side.activeRequest;
  if (!request || request.wait) return 'wait';
  if (request.teamPreview) return 'team_preview';
  // Revival Blessing's request is a forced switch whose slot is `reviving`: the side names a
  // fainted Pokémon (the engine's `BattleRequest::Revive`).
  if (request.forceSwitch) return request.side?.pokemon?.some(p => p.reviving) ? 'revive' : 'forced_switch';
  if (request.active) return 'turn_choices';
  return 'unknown';
}

// Showdown swaps `side.pokemon` entries on every switch; the frame keeps the party in team
// declaration order (SCHEMA.md), which is what `side.team` still holds — or, once a team choice
// brought some of it (Team Preview, M9.4), the order chosen (`oracleParty`, showdown.mjs).
function sideProjection(side) {
  const party = (side.oracleParty ?? side.team).map(set => side.pokemon.find(pokemon => pokemon.set === set));
  const index = new Map(party.map((pokemon, i) => [pokemon, i]));
  return {
    active: side.active.map(pokemon => pokemon?.isActive ? index.get(pokemon) ?? null : null),
    party: party.map(pokemonProjection),
    conditions: conditionMap(side.sideConditions),
  };
}

export function frame(battle, checkpoint) {
  return {
    schema: 1,
    checkpoint,
    turn: battle.turn,
    request: battle.ended ? 'finished' : `${requestKind(battle.p1)}|${requestKind(battle.p2)}`,
    // Same encoding as the Rust projection: winner(p1) / winner(p2) / draw.
    outcome: battle.ended
      ? (battle.winner === 'P1' ? 'winner(p1)' : battle.winner === 'P2' ? 'winner(p2)' : 'draw')
      : null,
    sides: [sideProjection(battle.p1), sideProjection(battle.p2)],
    field: {
      // A format rule with battle handlers (Sleep Clause Mod's `onSetStatus`) sits among the
      // pseudo-weathers to receive its events; it is the format's, not a field condition.
      conditions: conditionMap(Object.fromEntries(Object.entries(battle.field?.pseudoWeather ?? {})
        .filter(([id]) => !battle.ruleTable.has(id)))),
      // Showdown ids (`sunnyday`, `raindance`, `sandstorm`, `snowscape` / `hail`); the duration
      // is what its residual has left to tick (5 when set by a move, 4 after the first end of turn).
      weather: battle.field?.weather
        ? {kind: battle.field.weather, turns_remaining: battle.field.weatherState?.duration ?? null}
        : null,
      terrain: battle.field?.terrain
        ? {kind: battle.field.terrain, turns_remaining: battle.field.terrainState?.duration ?? null}
        : null,
    },
  };
}

// ---------------------------------------------------------------------------------------------
// Protocol events between two request boundaries, normalised to the checkpoints TESTING.md §2
// lists (hit / no-hit, damage committed, status, boosts, faint, switch). Pokémon are identified
// by side index and active slot so the Rust side can map them without Showdown's object ids.
// ---------------------------------------------------------------------------------------------

function who(field) {
  const m = /^p([12])([a-d]?): (.*)$/.exec(field ?? '');
  if (!m) return null;
  return {side: Number(m[1]) - 1, slot: m[2] ? m[2].charCodeAt(0) - 97 : null, name: m[3]};
}

function hpField(field) {
  if (!field) return {};
  const [num, ...rest] = field.split(' ');
  const status = rest.join(' ') || null;
  if (num === '0' || status === 'fnt') return {hp: 0, max_hp: null, status: status === 'fnt' ? null : status};
  const [hp, max_hp] = num.split('/').map(Number);
  return {hp, max_hp: Number.isFinite(max_hp) ? max_hp : null, status};
}

// `stillTarget(i)`: the target Showdown blanked in line i's `move` (a move that failed with
// `[still]`: `attrLastMove` erases it so a player never learns it); `showdown.mjs` keeps it.
export function parseEvents(lines, stillTarget = () => undefined, disguised = () => false) {
  const events = [];
  for (let i = 0; i < lines.length; i++) {
    let line = lines[i];
    // `|split|pN` is followed by the private (exact HP) line and then the public copy; keep
    // the private one only.
    if (line.startsWith('|split|')) {
      line = lines[i + 1] ?? '';
      i += 2;
    }
    const parts = line.split('|');
    if (parts.length < 2) continue;
    const kind = parts[1];
    // Showdown adds a bare `|` right before it runs the end-of-turn residual (`runAction
    // 'residual'`): everything after it belongs to the end of the turn, not to the last action.
    // The bare line `turnLoop` adds before its `|t:|` stamp as the actions start is not that
    // (it comes first in a boundary's slice, unless a Quick Claw or Quick Draw announced
    // itself while the choices were resolved), nor the one `win` adds before `|win|` / `|tie`:
    // the action that ended the battle goes on after it (Gen 4's Focus Sash used up at
    // `AfterMoveSecondary`, R-M11-49).
    if (line === '|') {
      const next = lines[i + 1] ?? '';
      const ends = next.startsWith('|win|') || next === '|tie';
      if (events.length > 0 && !next.startsWith('|t:|') && !ends) events.push({kind: 'residual'});
      continue;
    }
    switch (kind) {
      case 'turn': events.push({kind, turn: Number(parts[2])}); break;
      // Gen 1 Disable names the slot it drew only in its debug line (`Disable: disabling move X in
      // slot N`): two slots may hold the move, a Mimic's copy beside the original (R-M8-161).
      case 'debug': {
        const drawn = /^Disable: disabling move .* in slot (\d+)$/.exec(parts.slice(2).join('|'));
        if (drawn) events.push({kind: 'disableslot', slot: Number(drawn[1])});
        break;
      }
      case 'move': {
        // `[from] move: Metronome` (or an ability, or Magic Coat): the move another effect made
        // the Pokémon use (DESIGN.md §42.1). It sits among the line's attributes, after the
        // target (which a `[still]` move leaves blank): find the `[from]` wherever it is.
        // `[anim] Shell Side Arm Physical` (the move's `onPrepareHit`, before a protection can
        // stop it and its `-hint`): `anim`, only on a line that has one (R-M8-166).
        const anim = parts.slice(4).find(part => part && part.startsWith('[anim]'));
        events.push({kind, user: who(parts[2]), move: parts[3], target: who(parts[4] || stillTarget(i) || ''),
          from: parts.slice(4).find(part => part && part.startsWith('[from]')) ?? null,
          ...(anim ? {anim: anim.slice('[anim]'.length).trim()} : {})});
        break;
      }
      // A `switch` into an Illusion shows the disguise's details: `illusion` carries the entrant's
      // own (its gender); `replace` lifts the disguise in place (DESIGN.md §41.7; restored in
      // R-M8-75 after R-N.1 dropped it).
      case 'switch': {
        const actual = disguised(i);
        events.push({kind: 'switch', pokemon: who(parts[2]), details: parts[3], ...hpField(parts[4]),
          ...(actual ? {illusion: actual} : {})}); break;
      }
      case 'replace':
        events.push({kind: 'replace', pokemon: who(parts[2]), details: parts[3]}); break;
      // A permanent forme change (DESIGN.md §43.5): `detailschange` rewrites the Pokémon's own
      // species (`-formechange` is the in-battle one, with the ability or move that caused it).
      case 'detailschange':
        events.push({kind: 'detailschange', pokemon: who(parts[2]), details: parts[3]}); break;
      case '-formechange':
        events.push({kind: 'formechange', pokemon: who(parts[2]), species: parts[3], from: parts[4] ?? null}); break;
      // A forced entry (Roar, Dragon Tail, Red Card): the replay needs to tell it from a chosen one.
      case 'drag': {
        const actual = disguised(i);
        events.push({kind: 'drag', pokemon: who(parts[2]), details: parts[3], ...hpField(parts[4]),
          ...(actual ? {illusion: actual} : {})}); break;
      }
      case '-damage': case '-heal': case '-sethp':
        events.push({kind: kind.slice(1), pokemon: who(parts[2]), ...hpField(parts[3]), from: parts[4] ?? null, of: parts[5] ?? null}); break;
      case '-hitcount': events.push({kind: 'hitcount', pokemon: who(parts[2]), count: Number(parts[3])}); break;
      // Protect / Detect succeeded this turn.
      case '-singleturn': events.push({kind: 'singleturn', pokemon: who(parts[2]), move: parts[3]}); break;
      // A two-turn move's charging turn (Fly, Dig, Solar Beam); the attack follows next turn
      // unless the weather let it go at once (the damage then follows in the same action).
      case '-prepare': events.push({kind: 'prepare', pokemon: who(parts[2]), move: parts[3]}); break;
      case '-crit': case '-miss': case '-fail': case '-immune': case 'faint': case 'cant':
      case '-supereffective': case '-resisted': case '-block': case '-notarget':
        events.push({kind: kind.replace(/^-/, ''), pokemon: who(parts[2]), detail: parts.slice(3).join('|') || null}); break;
      // `[from] ability: Static|[of] p1a: X` names a contact reaction, `[from] item: Toxic Orb` a
      // residual; a cure carries `[from] ability: Natural Cure` or nothing (Heal Bell, Shed Skin
      // announce themselves with a preceding `-activate`).
      case '-status': case '-curestatus':
        events.push({kind: kind.slice(1), pokemon: who(parts[2]), status: parts[3], from: parts[4] ?? null, of: parts[5] ?? null}); break;
      case '-boost': case '-unboost': case '-setboost':
        events.push({kind: kind.slice(1), pokemon: who(parts[2]), stat: parts[3], amount: Number(parts[4])}); break;
      // Gen 1-3 `tryMoveHit` fails an `onTry` silently: Fake Out's hint is the only trace.
      case '-hint': events.push({kind: 'hint', text: parts.slice(2).join('|')}); break;
      // A format clause refusing a status (`Freeze Clause activated.`): the roll that asked for
      // it landed (M9.5, DESIGN.md §45.7).
      case '-message': events.push({kind: 'message', text: parts.slice(2).join('|')}); break;
      // A Pledge waiting for its ally's (`-waiting|user|ally`): that action does nothing.
      case '-waiting':
        events.push({kind: 'waiting', pokemon: who(parts[2]), of: who(parts[3])});
        break;
      case '-weather': case '-fieldstart': case '-fieldend': case '-sidestart': case '-sideend':
      case '-start': case '-end': case '-activate': case '-ability': case '-item': case '-enditem':
      // Gastro Acid (`-endability|POKEMON`): the ability stops counting.
      case '-endability':
      // Transform / Imposter (`-transform|USER|TARGET`): the user takes the target's ability, which
      // a later lead's Trace may copy (DESIGN.md R-M9-23).
      case '-transform':
        events.push({kind: kind.slice(1), args: parts.slice(2)}); break;
      default: break;
    }
  }
  return events;
}
