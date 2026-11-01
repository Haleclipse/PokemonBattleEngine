import fs from 'node:fs';
import {calculate, Generations, Pokemon, Move, Field} from '@smogon/calc';

const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const gen = Generations.get(input.generation);
// The Rust engine takes final (pre-boost) stats directly instead of deriving them from base
// stats / IVs / EVs / nature. @smogon/calc reads `rawStats` for damage and applies boosts on
// top, so overriding `rawStats` gives both sides byte-identical inputs without inventing a
// species. `calculate` works on clones, and `Pokemon#clone` recomputes the stats from the
// species, so the fixed stats travel with the clone. Keys: hp, atk, def, spa, spd, spe.
const clone = Pokemon.prototype.clone;
Pokemon.prototype.clone = function () {
  const copy = clone.call(this);
  if (this.fixedStats) fixStats(copy, this.fixedStats);
  return copy;
};

function fixStats(pokemon, stats) {
  pokemon.fixedStats = stats;
  Object.assign(pokemon.rawStats, stats);
  Object.assign(pokemon.stats, stats);
  if (stats.hp !== undefined) pokemon.originalCurHP = stats.hp;
}

function build(side) {
  const pokemon = new Pokemon(gen, side.species, side.options ?? {});
  if (side.raw_stats) fixStats(pokemon, side.raw_stats);
  return pokemon;
}

const attacker = build(input.attacker);
const defender = build(input.defender);
const move = new Move(gen, input.move.name, input.move.options ?? {});
const field = new Field(input.field ?? {});
const result = calculate(gen, attacker, defender, move, field);
const range = result.range();

process.stdout.write(JSON.stringify({
  schema: 1,
  provenance: {engine: '@smogon/calc', version: '0.11.0'},
  damage: result.damage,
  range,
  description: result.desc(),
  inputs: {
    attacker: {level: attacker.level, raw_stats: attacker.rawStats, boosts: attacker.boosts, status: attacker.status, item: attacker.item ?? null, ability: attacker.ability ?? null, types: attacker.types},
    defender: {level: defender.level, raw_stats: defender.rawStats, boosts: defender.boosts, status: defender.status, item: defender.item ?? null, ability: defender.ability ?? null, types: defender.types},
    move: {name: move.name, bp: move.bp, type: move.type, category: move.category},
    field: {weather: field.weather ?? null, terrain: field.terrain ?? null, gameType: field.gameType},
  },
}, null, 2));
