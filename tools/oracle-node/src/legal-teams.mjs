// Teams a format admits (M9.4, DESIGN.md §45.5), drawn from a seeded generator and kept only
// when Showdown's validator finds no problem: random species the format does not ban, abilities
// of the species, moves from the family's learnsets, items, natures, EVs, IVs, levels, genders,
// Tera types — each set validated alone, then the team. Used by the builder's oracle
// (`export-builder.mjs`) and the real-format battles.
import {Dex, PRNG, TeamValidator} from '@pkmn/sim';

const STATS = ['hp', 'atk', 'def', 'spa', 'spd', 'spe'];

/// The moves the species' family can have in the format's generation (its learnset, its
/// prevolutions', its base forme's), by name.
function familyMoves(dex, species) {
  const moves = new Set();
  const seen = new Set();
  let current = species;
  const visit = s => {
    if (!s?.exists || seen.has(s.id)) return;
    seen.add(s.id);
    const data = dex.species.getLearnsetData(s.id);
    for (const [id, sources] of Object.entries(data?.learnset ?? {})) {
      if (!sources.some(source => Number(source.charAt(0)) <= dex.gen)) continue;
      const move = dex.moves.get(id);
      if (move.exists && !move.isNonstandard && move.gen <= dex.gen) moves.add(move.name);
    }
  };
  while (current?.exists) {
    visit(current);
    if (current.changesFrom) visit(dex.species.get(current.changesFrom));
    if (current.baseSpecies !== current.name) visit(dex.species.get(current.baseSpecies));
    current = current.prevo ? dex.species.get(current.prevo) : null;
  }
  return [...moves].sort();
}

export class LegalTeams {
  constructor(formatid, seed) {
    this.format = Dex.formats.get(formatid);
    this.dex = Dex.forFormat(this.format);
    this.validator = new TeamValidator(this.format);
    this.table = this.dex.formats.getRuleTable(this.format);
    this.prng = new PRNG(seed);
    const dex = this.dex;
    this.species = dex.species.all()
      .filter(s => s.exists && !s.isNonstandard && s.gen <= dex.gen && !s.battleOnly &&
        !this.table.isBannedSpecies(s))
      .sort((a, b) => a.id.localeCompare(b.id));
    this.items = dex.items.all()
      .filter(i => i.exists && !i.isNonstandard && i.gen <= dex.gen && !this.table.isBanned(`item:${i.id}`))
      .sort((a, b) => a.id.localeCompare(b.id));
    this.natures = dex.natures.all().sort((a, b) => a.id.localeCompare(b.id)).map(n => n.name);
    this.types = dex.types.all().filter(t => t.exists && !t.isNonstandard).map(t => t.name).sort();
    this.moveCache = new Map();
  }

  pick(list) {
    return list[this.prng.random(list.length)];
  }

  chance(numerator, denominator) {
    return this.prng.random(denominator) < numerator;
  }

  /// One set of a random species, before validation.
  draw() {
    const dex = this.dex;
    const species = this.pick(this.species);
    if (!this.moveCache.has(species.id)) this.moveCache.set(species.id, familyMoves(dex, species));
    const pool = [...this.moveCache.get(species.id)];
    const moves = [];
    while (moves.length < 4 && pool.length) moves.push(pool.splice(this.prng.random(pool.length), 1)[0]);
    // Hidden Power named with its type half the time (`Hidden Power Fire`).
    const hiddenPower = moves.indexOf('Hidden Power');
    if (hiddenPower >= 0 && this.chance(1, 2)) {
      const kind = this.pick(this.types.filter(t => !['Normal', 'Fairy', 'Stellar'].includes(t)));
      moves[hiddenPower] = `Hidden Power ${kind}`;
    }
    const set = {
      name: this.chance(1, 5) ? `Nick${this.prng.random(100)}` : '',
      species: species.name,
      item: dex.gen >= 2 && this.chance(4, 5) ? this.pick(this.items).name : '',
      ability: dex.gen >= 3 ? this.pick(Object.values(species.abilities)) : '',
      moves,
      nature: dex.gen >= 3 ? this.pick(this.natures) : '',
      gender: '',
      shiny: this.chance(1, 10),
      level: 0,
    };
    const numbers = this.table;
    const maxLevel = numbers.maxLevel;
    set.level = this.chance(3, 5) ? maxLevel :
      Math.max(numbers.minLevel, 1) + this.prng.random(maxLevel - Math.max(numbers.minLevel, 1) + 1);
    if (species.gender === '' || !species.gender) {
      // A species with two genders: one named, or none for the battle to draw.
      set.gender = this.chance(4, 5) ? this.pick(['M', 'F']) : '';
    }
    if (dex.gen >= 3) {
      // EVs in steps of 4 up to the format's limit, 252 a stat at most.
      const evs = {hp: 0, atk: 0, def: 0, spa: 0, spd: 0, spe: 0};
      let left = numbers.evLimit ?? 510;
      for (let i = 0; i < 12 && left >= 4; i++) {
        const stat = this.pick(STATS);
        const add = Math.min(4 * (1 + this.prng.random(32)), 252 - evs[stat], left - (left % 4));
        evs[stat] += add;
        left -= add;
      }
      set.evs = evs;
      const ivs = {hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31};
      if (this.chance(1, 2)) ivs[this.pick(STATS)] = this.prng.random(32);
      set.ivs = ivs;
    }
    if (this.chance(1, 4)) set.happiness = this.prng.random(256);
    if (dex.gen >= 9 && this.chance(4, 5)) set.teraType = this.pick(this.types);
    if (dex.gen >= 7 && set.level === 100 && this.chance(1, 6)) {
      set.hpType = this.pick(this.types.filter(t => !['Normal', 'Fairy', 'Stellar'].includes(t)));
    }
    return set;
  }

  /// A legal set, or null after `tries` draws.
  legalSet(tries = 400) {
    for (let i = 0; i < tries; i++) {
      const set = this.draw();
      if (!set.moves.length) continue;
      const copy = JSON.parse(JSON.stringify(set));
      if (!this.validator.validateSet(copy, {})) return set;
    }
    return null;
  }

  /// A team of `size` legal sets the validator admits together, or null.
  team(size, tries = 40) {
    for (let attempt = 0; attempt < tries; attempt++) {
      const team = [];
      const nums = new Set();
      const items = new Set();
      const names = new Set();
      for (let i = 0; i < 60 && team.length < size; i++) {
        const set = this.legalSet();
        if (!set) return null;
        const species = this.dex.species.get(set.species);
        if (nums.has(species.num)) continue;
        if (set.item && items.has(set.item)) continue;
        // Nicknames unique on a side: the replay names a Pokémon by it, as Showdown's protocol
        // does (DESIGN.md R-M9-22).
        if (set.name && names.has(set.name)) continue;
        nums.add(species.num);
        if (set.item) items.add(set.item);
        if (set.name) names.add(set.name);
        team.push(set);
      }
      if (team.length < size) continue;
      const copy = JSON.parse(JSON.stringify(team));
      if (!this.validator.validateTeam(copy)) return team;
    }
    return null;
  }
}
