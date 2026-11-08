// Where the engine follows the cartridge rather than Showdown (issues.md), the oracle is patched
// to follow it too, so the differential replay keeps comparing everything else. Every battle the
// oracle plays (corpus.mjs, showdown.mjs) imports this module first. A patch set on a format's
// hooks or `actions` covers every format of its generations: the custom games and the real
// formats alike (`/^gen4/`, DESIGN.md R-M9-20), as the engine follows the cartridge in both.
// A patch may only change values no random draw depends on: Showdown must consume its PRNG
// exactly as unpatched — with the exceptions the user allowed, whose cartridge behavior changes
// how many draws there are
// (TESTING.md §7): I20 (Gen 4 Beat Up's hits), I35 (Gen 3 Twineedle's one poison roll), I40,
// I41, I43 (a Gen 4 Custap Berry holder with Stall), I57 (a Gen 4 confusion berry at full HP), I61
// (Gen 3-4 Thrash's count), I63 (Gen 3-4 Uproar), I65 (Gen 3 locked turns' PP), I66 (Gen 3-4
// BeforeMove order), I67 (Gen 3-4 last move after a stopped move), I82 (a Gen 4 ability given back),
// I91 (a berry a Gen 4 Fling throws at a target with Klutz), I68 (Gen 3-4 Lock-On), I69 (Gen 2
// Lock-On), I84 (Gen 4 Trace), I49 (a Gen 2-3 PP berry at the end of the turn: a speed tie
// there), I78 (the Gen 4 Rage bug), I77 (a Gen 2 Rage / Fury Cutter damage of 1 multiplied), I85
// (Gen 4 Conversion with Curse its only type), I106 (Gen 4 Truant's parity), I107 (Gen 3 Truant's
// flag cleared as a Pokémon comes in), I108 (Gen 2 Thrash's count and fatigue before the hit test),
// I109 (a Gen 2 rampager asleep stays locked), I87 (Gen 2 Foresight's accuracy stages), I110
// (Gen 2-3 Future Sight / Doom Desire's random factor at the strike), I111 (Gen 3 strikes by
// battler), I166 (Gen 3 Nature Power's accuracy roll), I168 (a Gen 3-4 copied move's target in a
// double battle), I176 (Gen 2 Conversion 2's type draw), I180 (a Gen 7+ instructed Counter, Mirror
// Coat or Pursuit used again), I182 (a Gen 3 lock Encore does not override), I8 (Poison Touch
// rolls whoever holds Protective Pads), I189 (a Gen 1 called multi-hit move past an earlier broken doll),
// I190 (a target-tracking user's locked move after a called one, in a double battle), I196 (a Flower
// Sweet flung at 10), I173 (a Gen 3 failed move is no attack for Mirror Move), I197 (a Gen 4 Transform into
// Arceus without Multitype keeps the target's forme), I199 (Big Root raises a Gen 5+ Liquid Ooze's damage), I202 (a Gen 6
// Normalize user's move Ion Deluge or Electrify turned Electric stays Electric), I198 (a strong weather's holder
// leaving ends only its strong weather), I200 (Magician after a split Dragon Darts weighs both darts' targets); I201
// (Pressure's extra PP for a combined Pledge goes to the Pledge its user uses) changes no draw but the PP left;
// I203 (a Gen 2 multi-hit move a sleeping user's Sleep Talk called through Metronome hits every time), I205
// (a Gen 3 entrant's switch-in still runs after an earlier entrant falls to the hazards), I207 (a Focus
// Punch user keeps its focus through a heal: Present's, Pollen Puff's on an ally), I209 (a Gen 3+ Counter or Mirror Coat another
// move calls strikes back). I206 (Dragon Darts' first target is checked for
// Emergency Exit), I210 (Gen 3 Lightning Rod names itself as it takes a move), I211 (a White Herb
// its holder may not use stays held) and I212 (Gen 7's Rollout storage kept until it is used) change
// no draw. I213 (a Gen 5+ called move's own result for Dancer and the Fusion moves) changes whether
// Dancer dances, and so its draws; I214 (Gen 4 Safeguard stops every confusion berry's confusion) takes
// the confusion's duration draw away; I215 (an Infiltrator's Max Move effects past Mist and Safeguard)
// lets a sleep's or a confusion's duration be drawn; I216 (a Gen 3 sun Weather Ball thaws its target)
// takes the thawed Pokémon's later thaw rolls away; I217 (a Gen 7+ Dancer stopped before its dance
// leaves the dance the battle's last move) lets a later Copycat call it, with its draws; I218 (Gen
// 3's damage as pokeemerald computes it) takes Spit Up's critical-hit and damage-roll draws away; I186
// (Crafty Shield lets a move its user uses on itself through) lets an Acupressure on itself draw its
// stat; I219 (a Gen 2 drag's entrant meets the Spikes within the move) moves a speed tie its switch-in
// sorts into the move.
// `SHOWDOWN_UNPATCHED=1` plays Showdown as published: to reproduce a battle recorded before a
// patch existed, and to check that a patch's scenario fails without it.
import {Battle, Dex, Pokemon} from '@pkmn/sim';

const patched = !process.env.SHOWDOWN_UNPATCHED;

// I4 — Gen 3-4 Beat Up. Showdown carries the base-stat damage in a `beatup` volatile on the user
// (`duration: 1`) that only ends with the turn's residuals, so its `onFoeModifyDef` (Gen 4) /
// `onFoeModifySpD` (Gen 3) keeps replacing the user's foes' Defense with their base Defense in
// every later damage calculation that turn, and mid-turn frames show it. On the cartridge the
// base stats belong to Beat Up's own damage: the volatile ends with the move.
for (const mod of patched ? ['gen3', 'gen4'] : []) {
  const condition = Dex.mod(mod).data.Moves.beatup.condition;
  condition.onAfterMove = pokemon => {
    pokemon.removeVolatile('beatup');
  };
}

// I5 — Gen 3-4 accuracy. Showdown multiplies the move's accuracy by the accuracy stage and then
// the evasion stage as floats (gen3 `tryMoveHit`, gen4 `hitStepAccuracy`) and applies its
// `chainModify` modifiers (Compound Eyes, Sand Veil, Snow Cloak, Hustle, Tangled Feet) only when
// that product is a whole number; its Gen 4 items return floats (`accuracy * 0.9`). The cartridge
// (pokeemerald `Cmd_accuracycheck`, pokeplatinum `BattleControllerPlayer_CheckMoveHitAccuracy`)
// takes the net stage `6 + accuracy - evasion` (0..12) to a fraction, then each modifier's
// percentage in a fixed order, floored at every step. The patch: those abilities' and items'
// accuracy handlers contribute nothing (they still run, so the handler list — and any speed tie
// Showdown shuffles in it — is unchanged), and a last `ModifyAccuracy` handler on the Gen 3-4
// formats returns the cartridge's value, which `randomChance(value, 100)` then rolls as before.
// Unaware sets a stage aside as the cartridge does (the target's: the user's accuracy stage; the
// user's: the target's evasion stage). Gen 4 Gravity is the cartridge's last step (×10 / 6, after
// Micle Berry), its Showdown handler quiet like the others. Gen 4 Simple doubles each stage
// before Unaware and only the sum is clamped (DESIGN.md R176). Gen 4 Micle Berry comes after the
// items, before Gravity (×120 / 100; its own handler only marks the move, I42). Not covered (the
// engine has none of it yet). Foresight / Miracle Eye: below.
const STAGE_RATIOS = [
  [33, 100], [36, 100], [43, 100], [50, 100], [60, 100], [75, 100], [1, 1],
  [133, 100], [166, 100], [2, 1], [233, 100], [133, 50], [3, 1],
];
const PHYSICAL_TYPES = ['Normal', 'Fighting', 'Flying', 'Poison', 'Ground', 'Rock', 'Bug', 'Ghost', 'Steel'];
const quiet = () => undefined;
for (const mod of patched ? ['gen3', 'gen4'] : []) {
  const data = Dex.mod(mod).data;
  for (const id of ['compoundeyes', 'hustle']) {
    if (data.Abilities[id]?.onSourceModifyAccuracy) data.Abilities[id].onSourceModifyAccuracy = quiet;
  }
  for (const id of ['sandveil', 'snowcloak', 'tangledfeet']) {
    if (data.Abilities[id]?.onModifyAccuracy) data.Abilities[id].onModifyAccuracy = quiet;
  }
  for (const id of ['brightpowder', 'laxincense']) {
    if (data.Items[id]?.onModifyAccuracy) data.Items[id].onModifyAccuracy = quiet;
  }
  for (const id of ['widelens', 'zoomlens']) {
    if (data.Items[id]?.onSourceModifyAccuracy) data.Items[id].onSourceModifyAccuracy = quiet;
  }
  const gravity = data.Moves.gravity?.condition;
  if (gravity?.onModifyAccuracy) gravity.onModifyAccuracy = quiet;
}
function cartridgeAccuracy(accuracy, target, source, move) {
  if (typeof accuracy !== 'number' || typeof move.accuracy !== 'number') return;
  const unaware = pokemon => pokemon.hasAbility('unaware') &&
    !(pokemon === target && this.suppressingAbility(target));
  const targetAbility = id => target.hasAbility(id) && !this.suppressingAbility(target);
  // Gen 4 Simple (pokeplatinum): the attacker's accuracy stage and the target's evasion stage
  // doubled, before Unaware; only their sum is clamped.
  const simple = this.gen === 4;
  let accuracyStage = source.boosts.accuracy * (simple && source.hasAbility('simple') ? 2 : 1);
  let evasionStage = target.boosts.evasion * (simple && targetAbility('simple') ? 2 : 1);
  if (move.ignoreAccuracy || unaware(target)) accuracyStage = 0;
  if (move.ignoreEvasion || unaware(source)) evasionStage = 0;
  // An identified target (Foresight, Odor Sleuth, Miracle Eye): Gen 4 a raised evasion stage
  // counts as 0 (pokeplatinum `MON_IS_IDENTIFIED`, as Showdown's `onModifyBoost`, which this
  // value replaces); Gen 3 none counts (pokeemerald `Cmd_accuracycheck`, issues.md I88).
  if (target.volatiles['foresight'] || target.volatiles['miracleeye']) {
    if (this.gen === 3) evasionStage = 0;
    else if (evasionStage > 0) evasionStage = 0;
  }
  const [numerator, denominator] = STAGE_RATIOS[Math.max(0, Math.min(12, 6 + accuracyStage - evasionStage))];
  let calc = Math.floor(move.accuracy * numerator / denominator);
  const percent = p => { calc = Math.floor(calc * p / 100); };
  if (source.hasAbility('compoundeyes')) percent(130);
  if (this.field.isWeather('sandstorm') && targetAbility('sandveil')) percent(80);
  if (this.field.isWeather('hail') && targetAbility('snowcloak')) percent(80);
  const physical = this.gen === 3 ? PHYSICAL_TYPES.includes(move.type) : move.category === 'Physical';
  if (source.hasAbility('hustle') && physical) percent(80);
  if (targetAbility('tangledfeet') && target.volatiles['confusion']) percent(50);
  if (target.hasItem('brightpowder')) percent(90);
  if (target.hasItem('laxincense')) percent(this.gen === 3 ? 95 : 90);
  if (source.hasItem('widelens')) percent(110);
  if (source.hasItem('zoomlens') && !this.queue.willMove(target)) percent(120);
  if (move.micleBoosted) percent(120);
  if (this.field.getPseudoWeather('gravity')) calc = Math.floor(calc * 10 / 6);
  return calc;
}
for (const [id, format] of patched ? Object.entries(Dex.data.Rulesets) : []) {
  if (!/^gen[34]/.test(id)) continue;
  format.onModifyAccuracyPriority = -100;
  format.onModifyAccuracy = cartridgeAccuracy;
  const cached = Dex.formats.get(id);
  cached.onModifyAccuracyPriority = -100;
  cached.onModifyAccuracy = cartridgeAccuracy;
}

// I30 — Unaware and the confusion self-hit, Gen 5-6. Unaware's `onAnyModifyBoost` reads
// `this.activeTarget`, which during a self-hit is still the foe the confused Pokémon aimed at: an
// Unaware foe there zeroes the confused Pokémon's own Attack and Defense stages. On the cartridge
// Unaware looks at the damage's attacker and defender, both the confused Pokémon. The patch does
// what the Gen 7+ condition already does (`this.activeTarget = pokemon`) for the self-hit; the
// gen5 mod shares the gen6 condition. Gen 4's self-hit, and I31's Slow Start with it, is the
// cartridge's through the Gen 4 damage patch (I14-I20).
for (const mod of patched ? ['gen6'] : []) {
  const condition = Dex.mod(mod).data.Conditions.confusion;
  const beforeMove = condition.onBeforeMove;
  condition.onBeforeMove = function (pokemon, ...rest) {
    const aimedAt = this.activeTarget;
    this.activeTarget = pokemon;
    try {
      return beforeMove.call(this, pokemon, ...rest);
    } finally {
      this.activeTarget = aimedAt;
    }
  };
}

// I10 — gems. Showdown's gem adds a `gem` volatile (`duration: 1`, `onBasePower`) that lasts
// until the turn's residuals, so any later move its holder uses that turn (Dancer, Instruct) is
// boosted too. On the cartridge the boost belongs to the move that used the gem up: the volatile
// ends with the move, as I4's `beatup` does.
for (const mod of patched ? ['gen5', 'gen6', 'gen7', 'gen8', 'gen9'] : []) {
  const condition = Dex.mod(mod).data.Conditions.gem;
  condition.onAfterMove = pokemon => {
    pokemon.removeVolatile('gem');
  };
}
// I10, continued: a foreseen attack's strike (Gen 5+: the move itself, which may use its user's
// gem) is no move use: no `onAfterMove` ends that gem, which would boost its user's next move.
// The volatile ends with the strike (`futuremove`'s `onEnd`, DESIGN.md R277).
for (const mod of patched ? ['gen5', 'gen6', 'gen7', 'gen8', 'gen9'] : []) {
  const condition = Dex.mod(mod).data.Conditions.futuremove;
  if (condition.gemEndsWithStrike) continue;
  const onEnd = condition.onEnd;
  condition.onEnd = function (target) {
    const source = this.effectState.source;
    const result = onEnd.call(this, target);
    source?.removeVolatile('gem');
    return result;
  };
  condition.gemEndsWithStrike = true;
}

// I10, continued: Bide's release (Gen 5+) is struck from the volatile's `onBeforeMove`, which then
// stops the move: no `onAfterMove` ends the gem the release used, which would outlast it into the
// turn's frames (DESIGN.md R-M8-132). The volatile ends with the release. Gen 5-9 share the object.
for (const mod of patched ? ['gen5', 'gen6', 'gen7', 'gen8', 'gen9'] : []) {
  const condition = Dex.mod(mod).data.Moves.bide.condition;
  if (condition.gemEndsWithRelease) continue;
  const onBeforeMove = condition.onBeforeMove;
  condition.onBeforeMove = function (pokemon, target, move) {
    try {
      return onBeforeMove.call(this, pokemon, target, move);
    } finally {
      pokemon.removeVolatile('gem');
    }
  };
  condition.gemEndsWithRelease = true;
}

// I11 — Gen 3 one-hit KO accuracy. pokeemerald `Cmd_tryKO` hits when `Random() % 100 + 1 <
// chance`, one point less than Showdown's `randomChance(chance, 100)` (Gen 4's
// `BtlCmd_TryOHKOMove` is `% 100 <`, as Showdown). A last `Accuracy` handler on the Gen 3 formats
// takes the point off; Lock-On and No Guard (`true`) are left alone.
function gen3OhkoAccuracy(accuracy, target, source, move) {
  if (!move.ohko || typeof accuracy !== 'number') return;
  return accuracy - 1;
}
for (const [id, format] of patched ? Object.entries(Dex.data.Rulesets) : []) {
  if (!/^gen3/.test(id)) continue;
  format.onAccuracyPriority = -100;
  format.onAccuracy = gen3OhkoAccuracy;
  const cached = Dex.formats.get(id);
  cached.onAccuracyPriority = -100;
  cached.onAccuracy = gen3OhkoAccuracy;
}

// I12 — Gen 3-4 King's Rock / Razor Fang and Serene Grace. Showdown's items push a 10 % flinch
// onto the move's secondaries (`onModifyMove`, priority -1) before Serene Grace (-2) doubles
// every secondary, so a Serene Grace holder flinches 20 % of the time. The cartridge rolls the
// item apart from the move's effect (pokeemerald `ITEMEFFECT_KINGSROCK_SHELLBELL`, pokeplatinum
// `CheckExtraFlinch`: `Random() % 100 < 10`), which Serene Grace does not touch. The patch marks
// the item's secondary and Serene Grace leaves marked ones alone.
for (const mod of patched ? ['gen3', 'gen4'] : []) {
  const data = Dex.mod(mod).data;
  for (const id of ['kingsrock', 'razorfang']) {
    const item = data.Items[id];
    if (!item?.onModifyMove) continue;
    const push = item.onModifyMove;
    item.onModifyMove = function (move, ...rest) {
      const before = move.secondaries?.length ?? 0;
      push.call(this, move, ...rest);
      if ((move.secondaries?.length ?? 0) > before) move.secondaries[move.secondaries.length - 1].kingsrock = true;
    };
  }
  const grace = data.Abilities.serenegrace;
  grace.onModifyMove = function (move) {
    if (move.secondaries && move.id !== 'chatter') {
      this.debug('doubling secondary chance');
      for (const secondary of move.secondaries) {
        if (secondary.chance && !secondary.kingsrock) secondary.chance *= 2;
      }
    }
  };
}

// I14-I20 — Gen 4 damage. Showdown's gen4 `getDamage` / `modifyDamage` chain each stage's
// modifiers 4096-based and round once, apply the stat modifiers after the stages, and put some
// multipliers elsewhere than the cartridge. pokeplatinum (c248fb3: `BattleSystem_CalcMoveDamage`,
// `BattleScript_CalcMoveDamage`, `BattleSystem_CalcDamageVariance`, `BattleSystem_ApplyTypeChart`,
// the move scripts' `BTLVAR_POWER_MULTI`, `BtlCmd_BeatUp`, `CALC_SELF_HIT`,
// `subscript_type_resist_berry`) multiplies one modifier at a time and floors each: I14 the power
// and the stats (their modifiers before the stage), I15 the screens, spread, weather, Solar Beam
// and Life Orb, I16 Sniper's ×3 critical hit, I17 the move's own ×2, I18 the type chart one type
// at a time and what follows it, the resist berry last, I19 the confusion self-hit and Struggle,
// I20 Beat Up. The patch lets Showdown run its calculation as published — every event, message
// and random draw stays — and returns the cartridge's number from the same power callback,
// critical hit, damage roll and eaten berry instead.
const TYPE_ITEMS = {
  silkscarf: 'Normal', charcoal: 'Fire', mysticwater: 'Water', magnet: 'Electric',
  miracleseed: 'Grass', nevermeltice: 'Ice', blackbelt: 'Fighting', poisonbarb: 'Poison',
  softsand: 'Ground', sharpbeak: 'Flying', twistedspoon: 'Psychic', silverpowder: 'Bug',
  hardstone: 'Rock', spelltag: 'Ghost', dragonfang: 'Dragon', blackglasses: 'Dark',
  metalcoat: 'Steel', seaincense: 'Water', waveincense: 'Water', oddincense: 'Psychic',
  rockincense: 'Rock', roseincense: 'Grass', flameplate: 'Fire', splashplate: 'Water',
  zapplate: 'Electric', meadowplate: 'Grass', icicleplate: 'Ice', fistplate: 'Fighting',
  toxicplate: 'Poison', earthplate: 'Ground', skyplate: 'Flying', mindplate: 'Psychic',
  insectplate: 'Bug', stoneplate: 'Rock', spookyplate: 'Ghost', dracoplate: 'Dragon',
  dreadplate: 'Dark', ironplate: 'Steel',
};
const ORBS = {
  adamantorb: ['Dialga', ['Dragon', 'Steel']],
  lustrousorb: ['Palkia', ['Dragon', 'Water']],
  griseousorb: ['Giratina', ['Dragon', 'Ghost']],
};
const PINCH = {overgrow: 'Grass', blaze: 'Fire', torrent: 'Water', swarm: 'Bug'};
const RESIST_BERRIES = new Set([
  'occaberry', 'passhoberry', 'wacanberry', 'rindoberry', 'yacheberry', 'chopleberry', 'kebiaberry',
  'shucaberry', 'cobaberry', 'payapaberry', 'tangaberry', 'chartiberry', 'kasibberry', 'habanberry',
  'colburberry', 'babiriberry', 'chilanberry',
]);
// pokeplatinum `sTypeMatchupMultipliers`: for each attacking type, its defending types that are
// not neutral, in the table's order — a dual type meets them in this order, one at a time.
const CHART_ORDER = {
  Normal: ['Rock', 'Steel', 'Ghost'],
  Fire: ['Fire', 'Water', 'Grass', 'Ice', 'Bug', 'Rock', 'Dragon', 'Steel'],
  Water: ['Fire', 'Water', 'Grass', 'Ground', 'Rock', 'Dragon'],
  Electric: ['Water', 'Electric', 'Grass', 'Ground', 'Flying', 'Dragon'],
  Grass: ['Fire', 'Water', 'Grass', 'Poison', 'Ground', 'Flying', 'Bug', 'Rock', 'Dragon', 'Steel'],
  Ice: ['Water', 'Grass', 'Ice', 'Ground', 'Flying', 'Dragon', 'Steel', 'Fire'],
  Fighting: ['Normal', 'Ice', 'Poison', 'Flying', 'Psychic', 'Bug', 'Rock', 'Dark', 'Steel', 'Ghost'],
  Poison: ['Grass', 'Poison', 'Ground', 'Rock', 'Ghost', 'Steel'],
  Ground: ['Fire', 'Electric', 'Grass', 'Poison', 'Flying', 'Bug', 'Rock', 'Steel'],
  Flying: ['Electric', 'Grass', 'Fighting', 'Bug', 'Rock', 'Steel'],
  Psychic: ['Fighting', 'Poison', 'Psychic', 'Dark', 'Steel'],
  Bug: ['Fire', 'Grass', 'Fighting', 'Poison', 'Flying', 'Psychic', 'Ghost', 'Dark', 'Steel'],
  Rock: ['Fire', 'Ice', 'Fighting', 'Ground', 'Flying', 'Bug', 'Steel'],
  Ghost: ['Normal', 'Psychic', 'Dark', 'Steel', 'Ghost'],
  Dragon: ['Dragon', 'Steel'],
  Dark: ['Fighting', 'Psychic', 'Ghost', 'Dark', 'Steel'],
  Steel: ['Fire', 'Water', 'Electric', 'Ice', 'Rock', 'Steel'],
};
const STAGE_FRACTIONS = [[10, 40], [10, 35], [10, 30], [10, 25], [10, 20], [10, 15], [10, 10],
  [15, 10], [20, 10], [25, 10], [30, 10], [35, 10], [40, 10]];
const times = (value, numerator, denominator) => Math.trunc(value * numerator / denominator);
// `BattleSystem_Divide`: never 0 from a value that was not.
const divide = (value, divisor) => (value ? Math.max(1, Math.trunc(value / divisor)) : 0);
const staged = (stat, stage) => times(stat, ...STAGE_FRACTIONS[6 + Math.max(-6, Math.min(6, stage))]);
function cartridgeGen4Damage(source, target, move, seen) {
  const tr = Math.trunc;
  const ability = (pokemon, id) => pokemon.hasAbility(id);
  // `Battler_IgnorableAbility`: the target's ability, unless Mold Breaker sets it aside.
  const foeAbility = (pokemon, id) => pokemon.hasAbility(id) && !this.suppressingAbility(pokemon);
  // `Battler_HeldItemEffect`: nothing while Klutz or Embargo sets the item aside.
  const item = pokemon => (pokemon.ignoringItem() ? '' : pokemon.getItem().id);
  const crit = target.getMoveHitData(move).crit;
  const critMul = crit ? (ability(source, 'sniper') ? 3 : 2) : 1;
  const variance = damage => (damage ? Math.max(1, tr(damage * (100 - seen.roll) / 100)) : 0);
  const helpingHand = !!source.volatiles['helpinghand'];
  // Beat Up (`BtlCmd_BeatUp`): the member's base Attack and level (its party data) against the
  // target's base Defense, power 10, the critical hit and Helping Hand, nothing else. The target's
  // is its battle data's (`DEFENDING_MON.species`): a transformed target's is the species it
  // became (R-M11-111).
  if (seen.member) {
    const attack = this.dex.species.get(seen.member.set.species).baseStats.atk;
    const defense = target.species.baseStats.def;
    let damage = tr(tr(attack * move.basePower * tr(tr(seen.member.level * 2 / 5) + 2) / defense) / 50);
    damage = (damage + 2) * critMul;
    if (helpingHand) damage = times(damage, 15, 10);
    return variance(damage);
  }
  // The confusion self-hit borrows Struggle (`CALC_SELF_HIT(MOVE_STRUGGLE, 40)`): its data type,
  // Normal, and no screens, weather, critical hit or `BattleScript_CalcMoveDamage` boosts.
  const selfHit = seen.selfHit;
  const struggle = selfHit || move.id === 'struggle';
  const type = move.type === '???' ? 'Normal' : move.type;
  const physical = move.category === 'Physical';
  const weather = selfHit ? '' : this.field.effectiveWeather();
  // Power: the move's own multiplier (`BTLVAR_POWER_MULTI`), then the cartridge's list.
  let power = seen.power;
  const hiding = id => !!target.volatiles[id];
  const substitute = !!target.volatiles['substitute'];
  if ((move.recoil || move.hasCrashDamage) && ability(source, 'reckless')) power = times(power, 12, 10);
  if (move.id === 'facade' && source.status && source.status !== 'slp') power *= 2;
  if (move.id === 'brine' && target.hp * 2 <= target.maxhp) power *= 2;
  if (['gust', 'twister'].includes(move.id) && (hiding('fly') || hiding('bounce'))) power *= 2;
  if (['earthquake', 'magnitude'].includes(move.id) && hiding('dig')) power *= 2;
  if (['surf', 'whirlpool'].includes(move.id) && hiding('dive')) power *= 2;
  if (move.flags['minimize'] && hiding('minimize')) power *= 2;
  // Showdown's callbacks double Smelling Salts / Wake-Up Slap; the scripts skip a Substitute.
  if (['smellingsalts', 'wakeupslap'].includes(move.id) && substitute) power = move.basePower;
  if (source.volatiles['charge'] && type === 'Electric') power *= 2;
  if (helpingHand) power = times(power, 15, 10);
  if (ability(source, 'technician') && !struggle && power <= 60) power = times(power, 15, 10);
  const held = item(source);
  if (TYPE_ITEMS[held] === type) power = times(power, 120, 100);
  if (held === 'lightball' && source.species.baseSpecies === 'Pikachu') power *= 2;
  const orb = ORBS[held];
  if (orb && source.species.baseSpecies === orb[0] && orb[1].includes(type) &&
    !(held === 'griseousorb' && source.transformed)) power = times(power, 120, 100);
  if (held === 'muscleband' && physical) power = times(power, 110, 100);
  if (held === 'wiseglasses' && !physical) power = times(power, 110, 100);
  if (foeAbility(target, 'thickfat') && ['Fire', 'Ice'].includes(type)) power = tr(power / 2);
  const active = this.getAllActive();
  if (type === 'Electric' && active.some(p => p.volatiles['mudsport'])) power = tr(power / 2);
  if (type === 'Fire' && active.some(p => p.volatiles['watersport'])) power = tr(power / 2);
  for (const [id, pinchType] of Object.entries(PINCH)) {
    if (type === pinchType && ability(source, id) && source.hp <= tr(source.maxhp / 3)) power = times(power, 150, 100);
  }
  if (type === 'Fire' && foeAbility(target, 'heatproof')) power = tr(power / 2);
  if (type === 'Fire' && foeAbility(target, 'dryskin')) power = times(power, 125, 100);
  if (ability(source, 'rivalry') && source.gender && target.gender) {
    power = source.gender === target.gender ? times(power, 125, 100) : times(power, 75, 100);
  }
  if (move.flags['punch'] && ability(source, 'ironfist')) power = times(power, 12, 10);
  // The stats: the stored stat times each modifier, floored, before the stage.
  const sunny = weather === 'sunnyday';
  // Flower Gift (`BattleSystem_CountAbility(…, COUNT_ALIVE_BATTLERS_OUR_SIDE, …)`): a holder on the
  // side with HP left (`curHP`) — not a Self-Destruct user, whose HP its effect script sets to 0
  // before `CalcDamage`, though Showdown's `fainted` waits for `faintMessages` (R-M11-115).
  const flowerGift = side => sunny && side.active.some(p => p && p.hp && p.hasAbility('flowergift'));
  let attack;
  let defense;
  if (physical) {
    attack = source.storedStats.atk;
    if (ability(source, 'hugepower') || ability(source, 'purepower')) attack *= 2;
    if (ability(source, 'slowstart') && source.abilityState.counter) attack = tr(attack / 2);
    if (held === 'choiceband') attack = times(attack, 150, 100);
    if (held === 'thickclub' && ['Cubone', 'Marowak'].includes(source.species.baseSpecies)) attack *= 2;
    if (ability(source, 'hustle')) attack = times(attack, 150, 100);
    if (ability(source, 'guts') && source.status) attack = times(attack, 150, 100);
    if (flowerGift(source.side)) attack = times(attack, 15, 10);
    defense = target.storedStats.def;
    if (item(target) === 'metalpowder' && target.species.name === 'Ditto' && !target.transformed) defense *= 2;
    if (foeAbility(target, 'marvelscale') && target.status) defense = times(defense, 150, 100);
    if (['explosion', 'selfdestruct'].includes(move.id)) defense = tr(defense / 2);
  } else {
    attack = source.storedStats.spa;
    if (held === 'choicespecs') attack = times(attack, 150, 100);
    if (held === 'souldew' && ['Latios', 'Latias'].includes(source.species.baseSpecies)) attack = times(attack, 150, 100);
    if (held === 'deepseatooth' && source.species.baseSpecies === 'Clamperl') attack *= 2;
    const partner = id => source.allies().some(p => p.hasAbility(id));
    if ((ability(source, 'plus') && partner('minus')) || (ability(source, 'minus') && partner('plus'))) {
      attack = times(attack, 150, 100);
    }
    if (sunny && ability(source, 'solarpower')) attack = times(attack, 15, 10);
    defense = target.storedStats.spd;
    const targetItem = item(target);
    if (targetItem === 'souldew' && ['Latios', 'Latias'].includes(target.species.baseSpecies)) defense = times(defense, 150, 100);
    if (targetItem === 'deepseascale' && target.species.baseSpecies === 'Clamperl') defense *= 2;
    if (weather === 'sandstorm' && target.hasType('Rock')) defense = times(defense, 15, 10);
    if (flowerGift(target.side) && !this.suppressingAbility(target)) defense = times(defense, 15, 10);
  }
  let attackStage = source.boosts[physical ? 'atk' : 'spa'];
  let defenseStage = target.boosts[physical ? 'def' : 'spd'];
  if (ability(source, 'simple')) attackStage = Math.max(-6, Math.min(6, attackStage * 2));
  if (foeAbility(target, 'simple')) defenseStage = Math.max(-6, Math.min(6, defenseStage * 2));
  if (foeAbility(target, 'unaware')) attackStage = 0;
  if (ability(source, 'unaware')) defenseStage = 0;
  if (critMul > 1 && attackStage < 0) attackStage = 0;
  if (critMul > 1 && defenseStage > 0) defenseStage = 0;
  const level = tr(tr(source.level * 2 / 5) + 2);
  let damage = tr(tr(staged(attack, attackStage) * power * level / staged(defense, defenseStage)) / 50);
  if (physical && source.status === 'brn' && !ability(source, 'guts')) damage = tr(damage / 2);
  const screen = target.side.getSideCondition(physical ? 'reflect' : 'lightscreen');
  if (screen && !selfHit && critMul === 1 && move.id !== 'brickbreak') {
    damage = target.alliesAndSelf().length > 1 ? times(damage, 2, 3) : tr(damage / 2);
  }
  if (move.spreadHit) damage = times(damage, 3, 4);
  if (weather === 'raindance') {
    if (type === 'Fire') damage = tr(damage / 2);
    if (type === 'Water') damage = times(damage, 15, 10);
  }
  if (move.id === 'solarbeam' && ['raindance', 'sandstorm', 'hail'].includes(weather)) damage = tr(damage / 2);
  if (weather === 'sunnyday') {
    if (type === 'Fire') damage = times(damage, 15, 10);
    if (type === 'Water') damage = tr(damage / 2);
  }
  if (source.volatiles['flashfire'] && type === 'Fire') damage = times(damage, 15, 10);
  damage = (damage + 2) * critMul;
  if (!selfHit) {
    if (held === 'lifeorb') damage = times(damage, 130, 100);
    if (held === 'metronome' && source.volatiles['metronome']) {
      damage = times(damage, 10 + Math.min(10, source.volatiles['metronome'].numConsecutive), 10);
    }
    if (source.volatiles['mefirst']) damage = times(damage, 15, 10);
  }
  damage = variance(damage);
  if (move.type !== '???') {
    if (source.hasType(move.type)) damage = ability(source, 'adaptability') ? damage * 2 : times(damage, 15, 10);
    const order = CHART_ORDER[move.type] ?? [];
    const types = [...new Set(target.getTypes())]
      .map(defending => [defending, this.dex.getEffectiveness(move.type, defending)])
      .filter(([, effect]) => effect)
      .sort(([a], [b]) => order.indexOf(a) - order.indexOf(b));
    for (const [, effect] of types) damage = divide(damage * (effect > 0 ? 20 : 5), 10);
    const typeMod = target.getMoveHitData(move).typeMod;
    if (typeMod > 0) {
      if (foeAbility(target, 'filter') || foeAbility(target, 'solidrock')) damage = divide(damage * 3, 4);
      if (held === 'expertbelt') damage = times(damage, 120, 100);
    }
    if (typeMod < 0 && ability(source, 'tintedlens')) damage *= 2;
  }
  if (seen.berry) damage = divide(damage, 2);
  return Math.max(1, damage);
}
function cartridgeGetDamage(source, target, move, suppressMessages = false) {
  const battle = this.battle;
  const seen = {power: undefined, roll: undefined, berry: false, member: null, selfHit: typeof move === 'number'};
  if (typeof move === 'object' && move.id === 'beatup' && move.allies?.length) [seen.member] = move.allies;
  const heldBefore = target.item;
  const {runEvent, randomizer} = battle;
  battle.runEvent = function (eventid, ...rest) {
    if (eventid === 'BasePower' && seen.power === undefined) [, , , seen.power] = rest;
    return runEvent.call(this, eventid, ...rest);
  };
  battle.randomizer = function (damage) {
    seen.roll = this.random(16);
    return this.trunc(this.trunc(damage * (100 - seen.roll)) / 100);
  };
  let result;
  try {
    result = Object.getPrototypeOf(this).getDamage.call(this, source, target, move, suppressMessages);
  } finally {
    battle.runEvent = runEvent;
    battle.randomizer = randomizer;
  }
  if (seen.roll === undefined || typeof result !== 'number') return result;
  seen.berry = RESIST_BERRIES.has(heldBefore) && !target.item;
  const active = typeof move === 'number' ? battle.dex.getActiveMove('struggle') : move;
  if (seen.selfHit) {
    active.type = '???';
    active.basePower = move;
    seen.power = move;
  }
  return cartridgeGen4Damage.call(battle, source, target, seen.selfHit ? Object.assign(active, {moveHitData: {}}) : move, seen);
}
for (const [id, format] of patched ? Object.entries(Dex.data.Rulesets) : []) {
  if (!/^gen4/.test(id)) continue;
  format.actions = {...format.actions, getDamage: cartridgeGetDamage};
  const cached = Dex.formats.get(id);
  cached.actions = {...cached.actions, getDamage: cartridgeGetDamage};
}

// I50 / I51 — Gen 2 damage as the cartridge computes it. I50: Minimize doubles the final damage. pokecrystal's Stomp script runs
// `doubleminimizedamage` after `damagevariation` (the random roll): the damage, with the item, the
// critical hit, the 997 cap, the + 2, weather, STAB and type all in, doubles, at most 65535. Showdown's
// gen2 Minimize is its condition's `onSourceModifyDamage` (`chainModify(2)`, in the `ModifyDamage`
// event right after the critical hit, and dropped when a type-boosting item's `damage * 1.1` is not a
// whole number). The patch: the `ModifyDamage` event still runs as before (its handlers, and any
// speed tie among them, unchanged), but its value is the attacker's type-boosting item alone — the
// event's only other handler in Gen 2 — and the damage doubles as `getDamage` returns.
function gen2ItemBoost(damage, attacker, defender, move) {
  const boost = attacker.getItem().onModifyDamage?.call(this, damage, attacker, defender, move);
  return boost === undefined ? damage : boost;
}
// The gen2 mod's own `getDamage` (the format's `actions` replace it on the instance, and the
// prototype's is the modern one).
const gen2GetDamage = Dex.mod('gen2').data.Scripts.actions.getDamage;
//
// I51 — Gen 2 type-boosting items before the critical hit. pokecrystal's `BattleCommand_DamageCalc`
// multiplies the item's × (100 + 10) / 100 into the damage and then doubles it for a critical hit
// (`.CriticalMultiplier`); Showdown's gen2 `getDamage` doubles first and runs `ModifyDamage` on the
// doubled damage. The patch's `ModifyDamage` value (above) takes the item on the damage before the
// doubling — `CriticalHit` runs in `getDamage` exactly when the hit is critical — and doubles after.
//
// I75 / I77 (below): Gen 2 Fury Cutter's doublings and Rage's count multiply the damage after the
// type matchups and before the random factor (pokecrystal `furycutter`, `ragedamage`), at most
// 65535; a damage of 1 so multiplied draws its random factor (`damagevariation` skips only a damage
// below 2). Where a move is multiplied, `getDamage` keeps the type matchups and the random factor
// out (`runEffectiveness` read and answered neutral, `noDamageVariance`) and they follow here, the
// messages where they were.
function gen2DamageMultiplier(source, move) {
  if (move.id === 'furycutter' && move.furyCutterDoublings) return 2 ** move.furyCutterDoublings;
  if (move.id === 'rage') return (source.volatiles['rage']?.counter || 0) + 1;
  return 1;
}
function cartridgeGen2GetDamage(source, target, move, suppressMessages = false) {
  const battle = this.battle;
  const {runEvent} = battle;
  let multiplied = null;
  if (target) {
    target.runEffectiveness = function (active, ...rest) {
      const typeMod = Pokemon.prototype.runEffectiveness.call(this, active, ...rest);
      const multiplier = gen2DamageMultiplier(source, active);
      if (multiplier === 1) return typeMod;
      multiplied = {active, typeMod, multiplier, variance: active.noDamageVariance};
      active.noDamageVariance = true;
      return 0;
    };
  }
  let critical = false;
  battle.runEvent = function (eventid, attacker, defender, active, damage, ...rest) {
    const value = runEvent.call(this, eventid, attacker, defender, active, damage, ...rest);
    if (eventid === 'CriticalHit') critical = true;
    if (eventid !== 'ModifyDamage') return value;
    if (!critical) return gen2ItemBoost.call(this, damage, attacker, defender, active);
    return Math.floor(gen2ItemBoost.call(this, damage / 2, attacker, defender, active)) * 2;
  };
  let result;
  try {
    result = gen2GetDamage.call(this, source, target, move, suppressMessages);
  } finally {
    battle.runEvent = runEvent;
    if (target) delete target.runEffectiveness;
    if (multiplied) multiplied.active.noDamageVariance = multiplied.variance;
  }
  if (multiplied && typeof result === 'number') {
    const {typeMod, multiplier} = multiplied;
    let damage = result;
    if (typeMod > 0) {
      if (!suppressMessages) battle.add('-supereffective', target);
      damage *= typeMod >= 2 ? 4 : 2;
    }
    if (typeMod < 0) {
      if (!suppressMessages) battle.add('-resisted', target);
      damage = Math.floor(damage / 2);
      if (typeMod <= -2) damage = Math.floor(damage / 2);
    }
    damage = Math.min(65535, damage * multiplier);
    if (!multiplied.variance && damage > 1) damage = Math.floor(damage * battle.random(217, 256) / 255);
    result = damage || 1;
  }
  const flags = typeof move === 'object' ? move.flags : typeof move === 'string' ? battle.dex.moves.get(move).flags : {};
  if (typeof result === 'number' && flags?.minimize && target.volatiles['minimize']) {
    result = Math.min(65535, result * 2);
  }
  return result;
}
for (const [id, format] of patched ? Object.entries(Dex.data.Rulesets) : []) {
  if (!/^gen2/.test(id)) continue;
  format.actions = {...format.actions, getDamage: cartridgeGen2GetDamage};
  const cached = Dex.formats.get(id);
  cached.actions = {...cached.actions, getDamage: cartridgeGen2GetDamage};
}

// I21 — Gen 4 Metronome (the item) counts as the cartridge does. Showdown's gen4 `metronome`
// condition counts in `onTryMove`: one more when the move is the one counted before and the
// holder's move last turn succeeded, 0 otherwise, without a cap (a two-turn move's charge stops
// the event before it). pokeplatinum (`BattleSystem_UpdateMetronomeCount` at the end of the
// BeforeMove checks, `BattleSystem_VerifyMetronomeCount` as the action ends) counts the chosen
// move as it starts, up to 10, not on a two-turn move's attacking turn, and takes one step back
// after a move that did no work (it missed, failed, met an immunity or a Protect) or a turn lost
// to anything but sleep or recharging. The patch moves the count to `onModifyMove` (every move
// past BeforeMove, before a two-turn move's charge) and adds the step back to `onAfterMove` and
// `onMoveAborted`. The boost, `× (10 + n) / 10` floored, is `cartridgeGen4Damage`'s (I14-I20).
// While Klutz or Embargo sets the item aside the cartridge's count drops to 0 as the move starts,
// the counted move kept (Showdown's `onTryMove` would end the condition instead; R194).
for (const mod of patched ? ['gen4'] : []) {
  const condition = Dex.mod(mod).data.Items.metronome.condition;
  const stepBack = (state, move) => {
    if (state.lastMove === move.id && state.numConsecutive > 0) state.numConsecutive--;
  };
  condition.onTryMove = () => undefined;
  // Before `mustrecharge` (11): whether the turn is a recharge, and whether it is a two-turn
  // move's attacking turn.
  condition.onBeforeMovePriority = 12;
  condition.onBeforeMove = function (pokemon) {
    this.effectState.recharging = !!pokemon.volatiles['mustrecharge'];
    this.effectState.releasing = !!pokemon.volatiles['twoturnmove'];
  };
  condition.onModifyMove = function (move, pokemon) {
    const state = this.effectState;
    // `Battler_HeldItemEffect` names no Metronome under Klutz or Embargo: the count drops to 0.
    if (!pokemon.hasItem('metronome')) {
      state.numConsecutive = 0;
      return;
    }
    if (state.releasing) return;
    if (state.lastMove === move.id) {
      state.numConsecutive = Math.min(10, state.numConsecutive + 1);
    } else {
      state.lastMove = move.id;
      state.numConsecutive = 0;
    }
  };
  condition.onAfterMove = function (pokemon, target, move) {
    if (this.effectState.releasing) return;
    const protectedFrom = pokemon.moveThisTurnResult === null && move.flags['protect'] &&
      !!target?.volatiles['protect'];
    if (pokemon.moveThisTurnResult === false || protectedFrom) stepBack(this.effectState, move);
  };
  condition.onMoveAborted = function (pokemon, target, move) {
    if (this.effectState.recharging || this.effectState.releasing || pokemon.status === 'slp') return;
    stepBack(this.effectState, move);
  };
}

// I20 — Gen 4 Beat Up's members and immunity are the cartridge's. Showdown's gen4 Beat Up
// (`onModifyMove`) takes the side's Pokémon neither fainted nor statused — the user too only when
// it has no status — and, once Normalize has made it Normal, a Ghost type is immune. pokeplatinum
// `BeatUpEligibleMon` always counts the user's own slot, whatever its status, and effect script
// 0154 sets `SYSCTL_IGNORE_IMMUNITIES`, so the type chart never stops it. The patch keeps the
// original `onModifyMove` and then counts the user in and sets `ignoreImmunity`. The user's decision
// makes it the exception to the RNG rule above: a hit more is a critical-hit roll and a damage roll
// more, so a battle recorded before the patch and one recorded after it part ways at the first Beat
// Up of a statused user.
for (const mod of patched ? ['gen4'] : []) {
  const beatup = Dex.mod(mod).data.Moves.beatup;
  const modifyMove = beatup.onModifyMove;
  beatup.onModifyMove = function (move, pokemon, ...rest) {
    modifyMove.call(this, move, pokemon, ...rest);
    move.allies = pokemon.side.pokemon.filter(ally => ally === pokemon || (!ally.fainted && !ally.status));
    move.multihit = move.allies.length;
    move.ignoreImmunity = true;
  };
}

// I23 — Gen 4-5 Mud Sport / Water Sport. Showdown makes them a volatile on a move that targets
// the field (`target: 'all'`); `tryMoveHit` hands the volatile to `targets[0]`, the first active
// Pokémon of the user's side, so in doubles a Mud Sport from the right lands on the left ally
// (fails when that one has it already, ends when it leaves). On the cartridge the effect is the
// user's own (pokeplatinum `moveEffectsMask` on the attacker): the patch has the moves target
// their user. Gen 3's own `useMove` already hands an 'all' move to its user. No random draw
// depends on the target (`accuracy: true`). Their Pressure keeps the field move's reach (Gen 4
// everyone else's, I72; Gen 5 the foes', Showdown's own): the `getMoveTargets` wrapper below
// (R266).
for (const mod of patched ? ['gen4', 'gen5'] : []) {
  for (const id of ['mudsport', 'watersport']) {
    Dex.mod(mod).data.Moves[id].target = 'self';
  }
}

// I32 — Gen 2's HP berries wait for less than half. pokecrystal `HandleHPHealingItem` feeds Berry,
// Gold Berry and Berry Juice (all `HELD_BERRY`) only when `2 × HP < max HP`; Showdown's
// `onResidual` takes `hp <= maxhp / 2`. The patch keeps each item's own `onResidual` and calls it
// only below half.
for (const mod of patched ? ['gen2'] : []) {
  const items = Dex.mod(mod).data.Items;
  for (const id of ['berry', 'goldberry', 'berryjuice']) {
    const residual = items[id].onResidual;
    items[id].onResidual = function (pokemon, ...rest) {
      if (pokemon.hp * 2 < pokemon.maxhp) return residual.call(this, pokemon, ...rest);
    };
  }
}

// I33 — Gen 2's status berries are eaten where pokecrystal feeds them, never at an `Update`: the
// effect commands that inflict a status or confusion on another Pokémon call
// `UseHeldStatusHealingItem` / `UseConfusionHealingItem` at once (Rest's own sleep does not), and
// `HandleHealingItems` checks them at the end of the turn. The patch takes each berry's `onUpdate`
// away and runs it from an `onAfterSetStatus` (a move of another Pokémon), from the `confusion`
// condition's `onStart` (idem) and from an `onResidual` at the berries' order 10.
const GEN2_STATUS_BERRIES = {};
for (const mod of patched ? ['gen2'] : []) {
  const data = Dex.mod(mod).data;
  for (const id of ['przcureberry', 'psncureberry', 'burntberry', 'iceberry', 'mintberry',
    'bitterberry', 'miracleberry']) {
    const item = data.Items[id];
    const update = item.onUpdate;
    GEN2_STATUS_BERRIES[id] = update;
    item.onUpdate = undefined;
    // A move's own status, or one its secondary's `onHit` sets (gen2 Tri Attack's
    // `trySetStatus(move.statusRoll, source)`: the effect is the secondary, the move still active;
    // pokecrystal's `BattleCommand_TriStatusChance` jumps to `FreezeTarget` & co., which feed the
    // berry at once — R-M8-114).
    item.onAfterSetStatus = function (status, target, source, effect) {
      if (source && source !== target &&
        (effect?.effectType === 'Move' || (this.activeMove && this.activePokemon === source))) {
        update.call(this, target);
      }
    };
    item.onResidualOrder = 10;
    item.onResidual = function (pokemon) {
      update.call(this, pokemon);
    };
  }
  const confusion = data.Conditions.confusion;
  const start = confusion.onStart;
  confusion.onStart = function (target, source, sourceEffect) {
    const result = start.call(this, target, source, sourceEffect);
    const update = GEN2_STATUS_BERRIES[target.item];
    if (update && source && source !== target && sourceEffect?.effectType === 'Move') {
      update.call(this, target);
    }
    return result;
  };
}

// I34 — Gen 3's status berries are eaten before a replacement request. pokeemerald checks them at
// every move's end (`MOVEEND_ITEM_EFFECTS_ALL`), at the end of the turn (`ENDTURN_ITEMS2`, after
// Yawn) and after the replacements, all before it asks for a replacement; Showdown feeds them at
// its `Update`, which Gen 1-4 skip when a switch is asked for. The patch runs the status berries'
// own `onUpdate` for every active Pokémon, fastest first (a tie keeps position order: no draw),
// before a Gen 3 `switch` request.
const GEN3_STATUS_BERRIES = ['cheriberry', 'chestoberry', 'pechaberry', 'rawstberry', 'aspearberry',
  'persimberry', 'lumberry'];
if (patched) {
  const makeRequest = Battle.prototype.makeRequest;
  Battle.prototype.makeRequest = function (type, ...rest) {
    if (this.gen === 3 && type === 'switch') {
      const active = this.getAllActive().map((pokemon, index) => ({pokemon, index}));
      active.sort((a, b) => b.pokemon.speed - a.pokemon.speed || a.index - b.index);
      for (const {pokemon} of active) {
        const item = pokemon.getItem();
        if (GEN3_STATUS_BERRIES.includes(item.id) && pokemon.hp) item.onUpdate?.call(this, pokemon);
      }
    }
    return makeRequest.call(this, type, ...rest);
  };
}

// I37 — Gen 3-4 stat berries need room. pokeemerald `ItemBattleEffects` and pokeplatinum
// `BattleSystem_TriggerHeldItem` feed a pinch berry only while its effect can apply: Liechi …
// Apicot Berry with the stat below +6, Lansat Berry without Focus Energy, Starf Berry with one of
// its five stats below +6; otherwise it stays held. Showdown's `onResidual` (gen3) / `onUpdate`
// (gen4) eat it anyway. The patch calls them only while there is room. Gen 4 shares these items'
// objects with Gen 5 (no gen4 entry), so the wrapper asks the battle's generation.
const ROOM_STATS = {liechiberry: 'atk', ganlonberry: 'def', salacberry: 'spe', petayaberry: 'spa',
  apicotberry: 'spd'};
const hasRoom = (pokemon, id) => {
  if (id in ROOM_STATS) return pokemon.boosts[ROOM_STATS[id]] < 6;
  if (id === 'lansatberry') return !pokemon.volatiles['focusenergy'];
  return ['atk', 'def', 'spa', 'spd', 'spe'].some(stat => pokemon.boosts[stat] < 6);
};
for (const mod of patched ? ['gen3', 'gen4'] : []) {
  const items = Dex.mod(mod).data.Items;
  for (const id of [...Object.keys(ROOM_STATS), 'lansatberry', 'starfberry']) {
    for (const callback of ['onUpdate', 'onResidual']) {
      const original = items[id][callback];
      if (!original || original.needsRoom) continue;
      const wrapped = function (pokemon, ...rest) {
        if (this.gen <= 4 && !hasRoom(pokemon, id)) return;
        return original.call(this, pokemon, ...rest);
      };
      wrapped.needsRoom = true;
      items[id][callback] = wrapped;
    }
  }
}

// I38 — Gen 3-4 Focus Band lasts the move. pokeemerald `Cmd_adjustnormaldamage` rolls on every
// hit and a success sets `focusBanded` until the action is over; pokeplatinum's
// `focusItemActivated` lasts until `MoveEnd`: once the band succeeded, every later hit of the move
// that would knock its holder out leaves it at 1. Showdown rolls each damage on its own. The patch
// draws exactly as Showdown does (the roll comes first, whatever the damage) and remembers a
// success on a hit of the active move for the rest of it. Gen 3-9 share this item's object, so the
// wrapper asks the battle's generation.
if (patched) {
  const band = Dex.mod('gen4').data.Items.focusband;
  const onDamage = band.onDamage;
  band.onDamage = function (damage, target, source, effect) {
    if (this.gen < 3 || this.gen > 4) return onDamage.call(this, damage, target, source, effect);
    const move = this.activeMove;
    const armed = !!move && target.itemState.focusBandMove === move;
    const rolled = this.randomChance(1, 10);
    const ofTheMove = effect?.effectType === 'Move' && !!move && effect.id === move.id;
    if (rolled && ofTheMove) target.itemState.focusBandMove = move;
    if ((rolled || (armed && ofTheMove)) && damage >= target.hp && effect?.effectType === 'Move') {
      this.add('-activate', target, 'item: Focus Band');
      return target.hp - 1;
    }
  };
}

// I39 — Gen 4 Focus Sash and the confusion self-hit. pokeplatinum's
// `subscript_hurt_self_in_confusion` runs `CheckHoldOnWith1HP` on the confused Pokémon: at full HP
// its sash leaves it at 1 and is used up. Showdown's gen4 sash only arms from `onTryHit`, which a
// self-hit never reaches. The patch gives the gen4 item an `onDamage` for the `confused`
// pseudo-move alone (no draw).
if (patched) {
  const sash = Dex.mod('gen4').data.Items.focussash;
  sash.onDamage = function (damage, target, source, effect) {
    if (this.gen === 4 && effect?.id === 'confused' && target.hp === target.maxhp &&
      damage >= target.hp && target.useItem()) {
      return target.hp - 1;
    }
  };
}

// I42 — Gen 4 Micle Berry as pokeplatinum has it. `HOLD_EFFECT_PINCH_ACC_UP` is checked with the
// other pinch berries after every move (`BattleSystem_TriggerHeldItem`), the
// `moveEffectsData.micleBerry` flag it sets lasts until a move checks its accuracy
// (`BattleControllerPlayer_CheckMoveHitAccuracy`: not a move without one) or the holder leaves
// the field, and Baton Pass does not hand it on; Showdown eats it in the residual only, its
// `micleberry` condition lasts two turns, any move's `ModifyAccuracy` (or `Accuracy`) uses it up,
// and it is not `noCopy`. The patch eats it at the gen4 item's `Update` instead of its residual,
// takes the condition's duration away, marks it `noCopy`, and lets only a move with an accuracy
// to check use it (the cartridge accuracy above applies the ×120 / 100). No draw.
if (patched) {
  const micle = Dex.mod('gen4').data.Items.micleberry;
  const eat = micle.onResidual;
  micle.onUpdate = function (pokemon) {
    if (this.gen === 4) return eat.call(this, pokemon);
  };
  micle.onResidual = undefined;
  const condition = micle.condition;
  delete condition.duration;
  condition.noCopy = true;
  condition.onSourceAccuracy = undefined;
  condition.onSourceModifyAccuracy = function (accuracy, target, source, move) {
    if (typeof accuracy !== 'number' || typeof move.accuracy !== 'number') return;
    this.add('-enditem', source, 'Micle Berry');
    source.removeVolatile('micleberry');
    move.micleBoosted = true;
  };
}

// I43 — Gen 4 Custap Berry ahead of Stall. pokeplatinum sets the same `battlerQuickClaw` flag
// for Custap Berry as for a Quick Claw that went off, and that flag is compared before Stall
// (`BattleSystem_CompareBattlerSpeed`); Showdown's gen4 `Custap` event takes the action's priority
// + 0.1, and the action's priority already holds Stall's −0.1. The patch gives the event the
// move's own priority + 0.1. The user allowed this one to change Showdown's draws: an event now
// ahead of other actions can tie a different set of them at `insertChoice`.
if (patched) {
  const custap = Dex.mod('gen4').data.Items.custapberry;
  custap.onBeforeTurn = function (pokemon) {
    if (pokemon.hp <= pokemon.maxhp / 4 || (pokemon.hp <= pokemon.maxhp / 2 && pokemon.ability === 'gluttony')) {
      const action = this.queue.willMove(pokemon);
      if (!action) return;
      this.queue.insertChoice({
        choice: 'event',
        event: 'Custap',
        priority: action.move.priority + 0.1,
        pokemon: action.pokemon,
        move: action.move,
        targetLoc: action.targetLoc,
      });
    }
  };
}

// I44 — Gen 4 put-first actions under Trick Room. pokeplatinum compares two Pokémon whose Quick
// Claw or Custap Berry went off by their Speed before its Trick Room branch
// (`BattleSystem_CompareBattlerSpeed`): the faster goes first, Trick Room or not; Showdown orders
// them by `getActionSpeed`, reversed under Trick Room. The patch gives a gen4 move with a
// positive fractional priority, and the `Custap` event, their Speed unreversed. No draw: a tie is
// a tie either way.
if (patched) {
  const getActionSpeed = Battle.prototype.getActionSpeed;
  Battle.prototype.getActionSpeed = function (action) {
    const result = getActionSpeed.call(this, action);
    const putFirst = (action.choice === 'move' && action.fractionalPriority > 0) ||
      (action.choice === 'event' && action.event === 'Custap');
    if (this.gen === 4 && putFirst && action.pokemon && this.field.getPseudoWeather('trickroom')) {
      action.speed = this.trunc(action.pokemon.getStat('spe', false, false), 13);
    }
    return result;
  };
}

// I35 — Gen 3 Twineedle rolls its poison once. pokeemerald's `BattleScript_MultiHitLoop` deals
// the hits, and `BattleScript_MultiHitEnd` runs `seteffectwithchance` once after them; the gen3
// `tryMoveHit` rolls every hit's secondaries. The patch lets a multi-hit move's secondaries roll on
// its last hit only (as the gen1 / gen2 mods do). The user's decision makes it the second exception
// to the RNG rule above: a Twineedle rolls one chance instead of two, so a battle recorded before
// the patch and one recorded after it part ways at the first Gen 3 Twineedle that hits twice.
for (const mod of patched ? ['gen3'] : []) {
  const actions = Dex.mod(mod).data.Scripts.actions;
  actions.secondaries = function (targets, source, move, ...rest) {
    if (move.multihit && !move.lastHit) return;
    return Object.getPrototypeOf(this).secondaries.call(this, targets, source, move, ...rest);
  };
}

// I40 — Gen 2 Counter / Mirror Coat against a failed attack or another Counter / Mirror Coat.
// pokecrystal's `BattleCommand_Counter` fails when the foe's last counterable move is Counter itself
// (`EFFECT_COUNTER`) or dealt no damage (`wCurDamage` 0); `BattleCommand_MirrorCoat` likewise with
// `EFFECT_MIRROR_COAT`. The gen2 `runMove` records a failed move as a 0-damage attack
// (`gotAttacked(move, false)`), so the foe's Counter returns 2 × 0 = 0 and hits for nothing. The patch
// makes both return `false` (the move fails) in those cases. The user's decision makes it the third
// exception to the RNG rule: a King's Rock holder no longer rolls the flinch of that empty hit.
for (const mod of patched ? ['gen2'] : []) {
  const moves = Dex.mod(mod).data.Moves;
  for (const id of ['counter', 'mirrorcoat']) {
    const damageCallback = moves[id].damageCallback;
    moves[id].damageCallback = function (pokemon, target) {
      const last = pokemon.getLastAttackedBy();
      if (last && (last.move === 'counter' || last.move === 'mirrorcoat')) return false;
      const damage = damageCallback.call(this, pokemon, target);
      return damage === 0 ? false : damage;
    };
  }
}

// I41 — Gen 2-3 Bide's first use never misses. pokecrystal's `Bide` script checks the hit only when
// the energy is released (`storeenergy` returns, `unleashenergy` sets Bide up); pokeemerald's
// `BattleScript_EffectBide` has no `accuracycheck`. The gen3 mod gives Bide accuracy 100 (gen2
// inherits it) and `tryMoveHit` checks it against the user itself, so a user whose own accuracy or
// evasion stage has changed may miss and never start. The patch makes the move's accuracy `true`
// while the user has no `bide` volatile (the release keeps its own accuracy 100). Fourth exception
// to the RNG rule: that check no longer draws.
for (const mod of patched ? ['gen2', 'gen3'] : []) {
  const bide = Dex.mod(mod).data.Moves.bide;
  bide.onModifyMove = function (move, pokemon) {
    if (!pokemon.volatiles['bide']) move.accuracy = true;
  };
}

// I52 — Gen 4 Embargo and the Speed-halving items as the cartridge has them. (1) pokeplatinum's
// Embargo script (`subscript_embargo_start.s`) also fails on a target whose Quick Claw went off as
// the turn's actions were ordered and that has not moved yet (`moveEffectsData.quickClaw`, set by
// `BattleSystem_CompareBattlerSpeed`, cleared as the holder's move begins), and on one whose Custap
// Berry did (`moveEffectsData.custapBerry`, the same life; the berry goes as the move begins): the
// patch notes the turn a Quick Claw went off (its `onFractionalPriority` returned the bonus; the
// data is shared with Gen 5-9, hence the generation check) and the turn a Custap Berry queued its
// event (the gen4 `onBeforeTurn`, as I43 has it), and the gen4 Embargo's `onTryHit` fails a target
// noted this turn whose move is still queued (`priorityItemPending`; I53 reads it too). (2) `BattleSystem_CompareBattlerSpeed` halves the Speed of
// a Macho Brace, Iron Ball or Power item holder from the held item itself, "not ignored by any
// negation effect": while the holder's `ModifySpe` event runs its `ignoringItem()` answers no, so
// the item's `onModifySpe` slows it under Klutz and Embargo too. Neither draws anything.
const SLOWING_ITEMS = ['machobrace', 'ironball', 'poweranklet', 'powerband', 'powerbelt',
  'powerbracer', 'powerlens', 'powerweight'];
const priorityItemPending = (battle, pokemon) => battle.gen === 4 &&
  (pokemon.m.quickClawTurn === battle.turn || pokemon.m.custapTurn === battle.turn) &&
  !!battle.queue.willMove(pokemon);
if (patched) {
  const custap = Dex.mod('gen4').data.Items.custapberry;
  const beforeTurn = custap.onBeforeTurn;
  custap.onBeforeTurn = function (pokemon) {
    beforeTurn.call(this, pokemon);
    if (this.queue.list.some(action => action.choice === 'event' && action.event === 'Custap' &&
      action.pokemon === pokemon)) pokemon.m.custapTurn = this.turn;
  };
  const quickclaw = Dex.mod('gen4').data.Items.quickclaw;
  const fractionalPriority = quickclaw.onFractionalPriority;
  quickclaw.onFractionalPriority = function (priority, pokemon, ...rest) {
    const bonus = fractionalPriority.call(this, priority, pokemon, ...rest);
    if (this.gen === 4 && bonus) pokemon.m.quickClawTurn = this.turn;
    return bonus;
  };
  const embargo = Dex.mod('gen4').data.Moves.embargo;
  const tryHit = embargo.onTryHit;
  embargo.onTryHit = function (pokemon, ...rest) {
    if (priorityItemPending(this, pokemon)) return false;
    return tryHit.call(this, pokemon, ...rest);
  };
  const ignoringItem = Pokemon.prototype.ignoringItem;
  Pokemon.prototype.ignoringItem = function (...args) {
    if (this.battle.gen === 4 && this.battle.event?.id === 'ModifySpe' &&
      SLOWING_ITEMS.includes(this.item)) return false;
    return ignoringItem.apply(this, args);
  };
}

// DESIGN.md R205, issues.md I80 / I82: changing abilities in Gen 3-4 as the cartridges do.
if (patched) {
  // Gen 4: the switch-in effects pokeplatinum marks announced for a stint on the field
  // (`battleMons[].…Announced`, cleared by `InitBattleMon`; the weather abilities share one).
  const ANNOUNCED = {
    drizzle: 'weather', drought: 'weather', sandstream: 'weather', snowwarning: 'weather',
    intimidate: 'intimidate', trace: 'trace', download: 'download', anticipation: 'anticipation',
    forewarn: 'forewarn', frisk: 'frisk', slowstart: 'slowstart', moldbreaker: 'moldbreaker',
    pressure: 'pressure',
  };
  // Gen 4: an ability change starts nothing at once; the move-end switch-in check
  // (`BattleSystem_TriggerEffectOnSwitch`) starts the new ability if it is one of these kinds and
  // the kind is new this stint (the `onStart` wrappers below). It starts no other ability:
  // Forecast / Flower Gift's form check and Truant's turn parity there are not modeled (R205).
  // Gluttony's `onStart` is no switch-in effect, only the flag Showdown's berries ask
  // (`abilityState.gluttony`); pokeplatinum's berry checks read the ability itself, so it works
  // at once and its `onStart` still runs (DESIGN.md R236).
  const FLAG_ONLY = new Set(['gluttony']);
  let changingAbility = false;
  const singleEvent = Battle.prototype.singleEvent;
  Battle.prototype.singleEvent = function (eventid, effect, ...rest) {
    if (changingAbility && this.gen === 4 && eventid === 'Start' &&
        effect.effectType === 'Ability' && !(effect.id in ANNOUNCED) && !FLAG_ONLY.has(effect.id)) {
      return;
    }
    return singleEvent.call(this, eventid, effect, ...rest);
  };
  const setAbility = Pokemon.prototype.setAbility;
  Pokemon.prototype.setAbility = function (...args) {
    const outer = changingAbility;
    changingAbility = true;
    try {
      return setAbility.apply(this, args);
    } finally {
      changingAbility = outer;
    }
  };
  // I80 (§41.10 A6): Gen 3-4 Skill Swap takes the same ability (pokeemerald
  // `Cmd_tryswapabilities`, pokeplatinum `subscript_exchange_abilities` have no such check).
  // Showdown's `skillSwap` without its `gen <= 5` same-ability check; a Gen 4 Griseous Orb still
  // refuses it through its own `onSetAbility` (I81).
  const skillSwap = Battle.prototype.skillSwap;
  Battle.prototype.skillSwap = function (source, target) {
    if (this.gen > 4) return skillSwap.call(this, source, target);
    if (source.fainted || target.fainted) return false;
    const sourceAbility = source.getAbility();
    const targetAbility = target.getAbility();
    if (sourceAbility.flags['failskillswap'] || targetAbility.flags['failskillswap']) return false;
    const sourceEffect = this.dex.conditions.get('skillswap');
    const targetCanBeSet = this.runEvent('SetAbility', target, source, sourceEffect, sourceAbility);
    if (!targetCanBeSet) return targetCanBeSet;
    const sourceCanBeSet = this.runEvent('SetAbility', source, source, sourceEffect, targetAbility);
    if (!sourceCanBeSet) return sourceCanBeSet;
    this.add('-activate', source, 'Skill Swap', '', '', `[of] ${target}`);
    this.singleEvent('End', sourceAbility, source.abilityState, source);
    this.singleEvent('End', targetAbility, target.abilityState, target);
    source.ability = targetAbility.id;
    target.ability = sourceAbility.id;
    source.abilityState = this.initEffectState({id: source.ability, target: source});
    target.abilityState = this.initEffectState({id: target.ability, target});
    if (this.gen > 3) {
      changingAbility = true;
      try {
        this.singleEvent('Start', sourceAbility, target.abilityState, target);
        this.singleEvent('Start', targetAbility, source.abilityState, source);
      } finally {
        changingAbility = false;
      }
    }
    return true;
  };
  // I80 for Role Play: no same-ability check in Gen 3-4 (`Cmd_trycopyability`,
  // `subscript_copy_ability`).
  for (const mod of ['gen3', 'gen4']) {
    const roleplay = Dex.mod(mod).data.Moves.roleplay;
    const tryHit = roleplay.onTryHit;
    roleplay.onTryHit = function (target, source, ...rest) {
      if (this.gen > 4) return tryHit.call(this, target, source, ...rest);
      if (target.getAbility().flags['failroleplay'] || source.getAbility().flags['cantsuppress']) return false;
    };
  }
  // Gen 4: a Pokémon's switch-in effect of a kind already announced this stint does not start
  // again — what Skill Swap or Role Play brings starts only if its kind is new.
  for (const [id, kind] of Object.entries(ANNOUNCED)) {
    const ability = Dex.mod('gen4').data.Abilities[id];
    const onStart = ability.onStart;
    ability.onStart = function (pokemon, ...rest) {
      if (this.gen === 4) {
        const announced = (pokemon.m.announced ??= new Set());
        if (announced.has(kind)) return;
        announced.add(kind);
      }
      return onStart.call(this, pokemon, ...rest);
    };
  }
  const clearVolatile = Pokemon.prototype.clearVolatile;
  Pokemon.prototype.clearVolatile = function (...args) {
    this.m.announced = new Set();
    return clearVolatile.apply(this, args);
  };
}

// I46 — Kee Berry and Maranga Berry only for a move that hit the holder from another Pokémon.
// Showdown's `onAfterMoveSecondary` checks the move's category alone, so a user's own Bide
// (physical, aimed at itself) eats its Kee Berry as it starts storing energy; Red Card and Eject
// Button in the same event check `source !== target`, and the games describe the berries as
// going off when the holder is hit by a physical / special move. No cartridge to check (Gen 6+);
// the user's decision treats it as a Showdown bug. The patch returns early when the holder is the
// move's user. No draw.
// The mods loaded above hold their own merged copies of the items, so each is patched (once).
const hitByAnother = new WeakSet();
for (const mod of patched ? ['gen6', 'gen7', 'gen8', 'gen9'] : []) {
  for (const id of ['keeberry', 'marangaberry']) {
    const item = Dex.mod(mod).data.Items[id];
    if (hitByAnother.has(item)) continue;
    hitByAnother.add(item);
    const onAfterMoveSecondary = item.onAfterMoveSecondary;
    item.onAfterMoveSecondary = function (target, source, move) {
      if (source === target) return;
      return onAfterMoveSecondary.call(this, target, source, move);
    };
  }
}

// I45 — Gen 4 Jaboca / Rowap Berry need the attacker standing. pokeplatinum checks them after
// every hit (`AFTER_MOVE_EFFECT_TRIGGER_ITEMS_ON_HIT` in `BattleControllerPlayer_AfterMoveEffects`,
// which runs before `BattleControllerPlayer_LoopMultiHit`) and, for U-turn, before the user leaves
// (`BattleSystem_TriggerHeldItemOnPivotMove`), both only while `ATTACKING_MON.curHP`; the gen6
// mod's `onDamagingHit` that gen4 uses eats the berry whatever, so an attacker Rough Skin has just
// knocked out still costs the holder its berry. The patch returns early in gen4 while the attacker
// has no HP. No draw.
if (patched) {
  for (const id of ['jabocaberry', 'rowapberry']) {
    const item = Dex.mod('gen4').data.Items[id];
    const onDamagingHit = item.onDamagingHit;
    item.onDamagingHit = function (damage, target, source, move) {
      if (this.gen === 4 && !source.hp) return;
      return onDamagingHit.call(this, damage, target, source, move);
    };
  }
}

// I53 — Knock Off, Thief and Covet as the cartridges have them (Gen 2-4). (1) pokeplatinum's
// `subscript_knock_off.s` and `BtlCmd_TryStealItem` leave the item of a target whose Quick Claw or
// Custap Berry went off this turn and that has not moved yet (I52's `priorityItemPending`). (2)
// `BtlCmd_TryStealItem` fails when the user has Multitype too (Showdown only refuses the
// target's). (3) Mail is never stolen (pokecrystal `ItemIsMail`, pokeemerald `IS_ITEM_MAIL`,
// pokeplatinum `BattleSystem_CanStealItem`): Mail's `onTakeItem` lets only Knock Off take it in
// Gen 2-4. (4) pokeemerald `SetMoveEffect` returns for a target at 0 HP before Knock Off: Gen 3
// Knock Off leaves a fainted target its item. Gen 3 and Gen 4 have Knock Off objects of their own;
// Thief and Mail are shared with other generations, hence the generation checks. None of them draws
// anything.
if (patched) {
  for (const mod of ['gen3', 'gen4']) {
    const knockoff = Dex.mod(mod).data.Moves.knockoff;
    const knockOffAfterHit = knockoff.onAfterHit;
    knockoff.onAfterHit = function (target, source, move) {
      if (priorityItemPending(this, target) || (this.gen === 3 && !target.hp)) return;
      return knockOffAfterHit.call(this, target, source, move);
    };
  }
  for (const id of ['thief', 'covet']) {
    const steal = Dex.mod('gen4').data.Moves[id];
    const afterHit = steal.onAfterHit;
    steal.onAfterHit = function (target, source, move) {
      if (priorityItemPending(this, target)) return;
      if (this.gen === 4 && (source.ability === 'multitype' || target.ability === 'multitype')) return;
      return afterHit.call(this, target, source, move);
    };
  }
  const mail = Dex.mod('gen4').data.Items.mail;
  const mailTakeItem = mail.onTakeItem;
  mail.onTakeItem = function (item, ...rest) {
    if (this.gen <= 4 && this.activeMove?.id !== 'knockoff') return false;
    return mailTakeItem.call(this, item, ...rest);
  };
}

// I54 — Gen 3 link battles never check a Knock Off's mark when an item changes hands (pokeemerald
// `MOVE_EFFECT_STEAL_ITEM` and `Cmd_tryswapitems` skip `knockedOffMons` for `BATTLE_TYPE_LINK` and
// the Battle Frontier), but a marked Pokémon enters holding nothing (`Cmd_switchindataupdate`).
// Showdown's `takeItem` refuses a marked holder or taker in Gen 1-4. The patch lifts the marks for
// Gen 3's `takeItem` and empties a marked Pokémon's hands as it switches in — Gen 4 too
// (pokeplatinum `BattleSystem_InitBattleMon` clears `heldItem` for a slot in
// `knockedOffItemsMask`): a marked Pokémon can still get an item back with Recycle, which neither
// cartridge nor Showdown refuses (R-M8-117). No draw.
if (patched) {
  const takeItem = Pokemon.prototype.takeItem;
  Pokemon.prototype.takeItem = function (source) {
    if (this.battle.gen !== 3) return takeItem.call(this, source);
    const marked = [this, source].filter(pokemon => pokemon?.itemKnockedOff);
    for (const pokemon of marked) pokemon.itemKnockedOff = false;
    try {
      return takeItem.call(this, source);
    } finally {
      for (const pokemon of marked) pokemon.itemKnockedOff = true;
    }
  };
  const markedSwitchIn = function (pokemon, ...rest) {
    if (pokemon.itemKnockedOff) pokemon.item = '';
    return Object.getPrototypeOf(this).switchIn.call(this, pokemon, ...rest);
  };
  for (const [id, format] of Object.entries(Dex.data.Rulesets)) {
    if (!/^gen[34]/.test(id)) continue;
    format.actions = {...format.actions, switchIn: markedSwitchIn};
    const cached = Dex.formats.get(id);
    cached.actions = {...cached.actions, switchIn: markedSwitchIn};
  }
}

// I53 (D.3) — Gen 4 Trick and Switcheroo read the same flags as Knock Off (pokeplatinum
// `subscript_exchange_items.s` checks `BATTLEMON_QUICK_CLAW` / `BATTLEMON_CUSTAP_BERRY` before
// `TrySwapItems`): the gen4 moves' `onHit` fails on a target `priorityItemPending` notes. The
// cartridge's other checks there — Multitype or a Griseous Orb on either side, Mail, a Substitute,
// nothing held, a Knock Off's mark — Showdown's `takeItem` already refuses. No draw.
if (patched) {
  for (const id of ['trick', 'switcheroo']) {
    const move = Dex.mod('gen4').data.Moves[id];
    const onHit = move.onHit;
    move.onHit = function (target, source, ...rest) {
      if (priorityItemPending(this, target)) return false;
      return onHit.call(this, target, source, ...rest);
    };
  }
}

// I55 — a Gen 4 Choice lock ends once the item is set aside. pokeplatinum's `UpdateMoveBuffers`
// clears the lock of the move's user and of its target when `Battler_HeldItem` reads no Choice
// item for them — and it reads nothing under Klutz or Embargo — so the move that sets the item
// aside (Embargo, or the Skill Swap / Role Play that brings Klutz) ends the lock, and the holder is
// free after the item comes back. Showdown's `choicelock` keeps the lock, held off while
// `ignoringItem()`, back in force after. The patch removes it at the gen4 `onDisableMove` while
// `ignoringItem()` (the data is shared with Gen 3, which has neither, hence the generation check).
// No draw.
if (patched) {
  const choicelock = Dex.mod('gen4').data.Conditions.choicelock;
  const disableMove = choicelock.onDisableMove;
  choicelock.onDisableMove = function (pokemon, ...rest) {
    if (this.gen === 4 && pokemon.ignoringItem()) {
      pokemon.removeVolatile('choicelock');
      return;
    }
    return disableMove.call(this, pokemon, ...rest);
  };
}

// I184 — a Gen 3 Choice Band locks by the item its holder held before its move handed it one
// (pokeemerald `Cmd_moveend`: `holdEffectAtk` comes from `gBattleMons[attacker].item`, which Trick's
// `Cmd_tryswapitems` and the item steal of Thief / Covet leave `ITEM_NONE` — the item taken waits in
// `changedItems` until `MOVEEND_CHANGED_ITEMS`, after `MOVEEND_CHOICE_MOVE`), and a Trick that swapped
// clears both battlers' `choicedMove`. Showdown's gen3 shares the gen4 Choice items: `onAfterMove`
// locks by the item held then, so a Pokémon that took a Choice Band with its own Trick, Thief or
// Covet is locked into that move, and with no `onStart` a Band-for-Band Trick leaves the target's
// lock. The patch notes the turn a Gen 3 Pokémon took an item during its own move (the data is shared
// with Gen 4-9, hence the generation check), skips that holder's lock, and ends both locks on a Trick
// that swapped. No draw.
if (patched) {
  const setItem = Pokemon.prototype.setItem;
  Pokemon.prototype.setItem = function (item, ...rest) {
    const result = setItem.call(this, item, ...rest);
    if (result && this.battle.gen === 3 && this.battle.activePokemon === this) {
      this.m.itemTakenTurn = this.battle.turn;
    }
    return result;
  };
  const band = Dex.mod('gen4').data.Items.choiceband;
  const afterMove = band.onAfterMove;
  band.onAfterMove = function (pokemon, ...rest) {
    if (this.gen === 3 && pokemon.m.itemTakenTurn === this.turn) return;
    return afterMove.call(this, pokemon, ...rest);
  };
  const trick = Dex.mod('gen4').data.Moves.trick;
  const swap = trick.onHit;
  trick.onHit = function (target, source, ...rest) {
    const result = swap.call(this, target, source, ...rest);
    if (this.gen === 3 && result !== false) {
      target.removeVolatile('choicelock');
      source.removeVolatile('choicelock');
    }
    return result;
  };
}

// I61 — Gen 3-4 Thrash, Petal Dance, Outrage as the cartridge counts them (pokeemerald
// `MOVE_EFFECT_THRASH` / `ENDTURN_THRASH`, `Cmd_attackcanceler`, `WasUnableToUseMove`; pokeplatinum
// `subscript_thrash` / `MON_COND_CHECK_STATE_THRASH`, `BattleContext_MoveFailed`). The first hit sets
// 2-3 turns; every end of turn counts one down (the per-battler end-turn order, after Uproar and
// before Disable: 10.12); a later hit leaves the count alone and a miss does not stop it; at 0 the
// lock ends and confuses its user unless it is confused already. A turn the move failed — full
// paralysis, a flinch, a confusion self-hit, love, Disable, Taunt, Imprison, Gen 4 Gravity, or a
// type immunity — ends it silently at the end of the turn; Gen 3's Protect ends it at once, and a
// Gen 3 user put to sleep or frozen loses it; a Gen 4 user asleep at the end of a turn the lock
// does not end loses it (`SetupNextTurn`). Showdown keeps a hidden length and restarts a visible
// `duration` on every hit, so a miss or an immunity ends the lock (with confusion on its last
// turn), a blocked last turn confuses at the end of the turn, Gen 3's Protect keeps it going, and
// the end comes after every ordered residual. The patch replaces the gen3 / gen4 `lockedmove` with
// the cartridge's count as its `duration` (drawn by `durationCallback`, the same `random(2, 4)`
// where `onStart` drew the length), the failure flag set by the blocking handlers and the moves'
// `onMoveFail`. User's standing decision (2026-10-01): an exception to the RNG rule — a lock that
// goes on after a miss draws its random targets and its confusion on other turns than Showdown's.
const RAMPAGE_MOVES = ['thrash', 'petaldance', 'outrage'];
const rampageFailed = Symbol('rampageFailed');
function failRampageWhenStopped(effect, callback, gens) {
  if (!effect?.[callback] || effect[callback][rampageFailed]) return;
  const original = effect[callback];
  const wrapped = function (pokemon, ...rest) {
    const result = original.call(this, pokemon, ...rest);
    if (result === false && gens.includes(this.gen) && pokemon.volatiles['lockedmove']) {
      pokemon.volatiles['lockedmove'].failed = true;
    }
    return result;
  };
  wrapped[rampageFailed] = true;
  effect[callback] = wrapped;
}
for (const mod of patched ? ['gen3', 'gen4'] : []) {
  const data = Dex.mod(mod).data;
  data.Conditions.lockedmove = {
    name: 'lockedmove',
    durationCallback() {
      return this.random(2, 4);
    },
    onStart(target, source, effect) {
      this.effectState.move = effect.id;
    },
    onResidualOrder: 10,
    onResidualSubOrder: 12,
    onResidual(pokemon) {
      if (this.effectState.failed || pokemon.status === 'slp' ||
        (this.gen === 3 && pokemon.status === 'frz')) {
        delete pokemon.volatiles['lockedmove'];
      }
    },
    onEnd(pokemon) {
      if (this.effectState.failed || (this.gen === 3 && ['slp', 'frz'].includes(pokemon.status))) return;
      pokemon.addVolatile('confusion');
    },
    onLockMove() {
      return this.effectState.move;
    },
  };
  for (const id of ['par', 'flinch', 'confusion']) {
    failRampageWhenStopped(data.Conditions[id], 'onBeforeMove', [3, 4]);
  }
  for (const id of ['attract', 'disable', 'taunt']) {
    failRampageWhenStopped(data.Moves[id]?.condition, 'onBeforeMove', [3, 4]);
  }
  failRampageWhenStopped(data.Moves.imprison?.condition, 'onFoeBeforeMove', [3, 4]);
  failRampageWhenStopped(data.Moves.gravity?.condition, 'onBeforeMove', [4]);
  for (const id of RAMPAGE_MOVES) {
    data.Moves[id].onMoveFail = function (target, source, move) {
      const lock = source.volatiles['lockedmove'];
      if (!lock || !target) return;
      if (this.gen === 3 && target.volatiles['protect']) {
        delete source.volatiles['lockedmove'];
      } else if (!target.runImmunity(move.type)) {
        lock.failed = true;
      }
    };
  }
}

// I63 — Gen 3-4 Uproar as the cartridge has it (pokeemerald `MOVE_EFFECT_UPROAR`,
// `ENDTURN_UPROAR`, `UproarWakeUpCheck`, `AtkCanceller_UnableToUseMove`, `Cmd_attackcanceler`,
// `SetMoveEffect`, `BattleScript_EffectYawn`; pokeplatinum `subscript_uproar`,
// `MON_COND_CHECK_STATE_UPROAR`, `CHECK_STATUS_STATE_SLEEP`, `subscript_fall_asleep`,
// `subscript_yawn`, `BattleContext_MoveFailed`). The move wakes nobody; the holder's turn-end step
// (10.11) wakes every active Pokémon asleep, in battler order, then counts the uproar down and
// ends it at 0 or after a turn its move failed (full paralysis, a confusion self-hit, a type
// immunity; Gen 4 also a flinch, love, Disable, Taunt, Imprison, Gravity); a sleeping Pokémon about
// to move wakes while an uproar goes on. A Soundproof Pokémon is out of it: it may fall asleep and
// is not woken. Gen 3's flinch, love, Disable, Taunt, Imprison, Protect, and a holder put to sleep
// or frozen end it at once, silently (`CancelMultiTurnMoves`). Yawn fails while an uproar keeps its
// target awake. Showdown wakes every sleeper as the move hits, keeps everyone awake, and lets the
// uproar run its `duration` whatever happens. The patch drops the move's `onTryHit`, keeps the
// count drawn by `durationCallback` as the condition's `time` (the frame shows it; Showdown's own
// countdown no longer runs) and counts it in `onResidual`; the wake-ups print
// `-curestatus|p|slp|[from] move: Uproar`. User's standing decision (2026-10-01): an exception to
// the RNG rule — a Pokémon woken before its move acts and draws, a Soundproof one may be put to
// sleep (a sleep roll), and an uproar that ends early lets others sleep.
const UPROAR_WAKE = '[from] move: Uproar';
const uproarOnField = battle => battle.getAllActive().some(pokemon => pokemon.volatiles['uproar']);
function wakeInUproar(battle, pokemon) {
  battle.add('-curestatus', pokemon, 'slp', UPROAR_WAKE);
  if (pokemon.removeVolatile('nightmare')) battle.add('-end', pokemon, 'Nightmare', '[silent]');
  pokemon.setStatus('');
}
// The cartridge's battler order: the player's first slot, the foe's, the player's second, the foe's.
function battlerOrder(battle) {
  const [p1, p2] = battle.sides;
  const order = [];
  for (let slot = 0; slot < Math.max(p1.active.length, p2.active.length); slot++) {
    for (const side of [p1, p2]) {
      const pokemon = side.active[slot];
      if (pokemon && !pokemon.fainted) order.push(pokemon);
    }
  }
  return order;
}
const uproarStopped = Symbol('uproarStopped');
// `react(battle, pokemon)` runs when the handler stops the holder's move (returns `false`).
function whenUproarStopped(effect, callback, react) {
  if (!effect?.[callback] || effect[callback][uproarStopped]) return;
  const original = effect[callback];
  const wrapped = function (pokemon, ...rest) {
    const result = original.call(this, pokemon, ...rest);
    if (result === false && [3, 4].includes(this.gen) && pokemon.volatiles['uproar']) {
      react(this, pokemon);
    }
    return result;
  };
  wrapped[uproarStopped] = true;
  effect[callback] = wrapped;
}
const failUproar = (battle, pokemon) => {
  pokemon.volatiles['uproar'].failed = true;
};
const cancelUproar = (battle, pokemon) => {
  if (battle.gen === 3) {
    delete pokemon.volatiles['uproar'];
  } else {
    failUproar(battle, pokemon);
  }
};
for (const mod of patched ? ['gen3', 'gen4'] : []) {
  const data = Dex.mod(mod).data;
  const uproar = data.Moves.uproar;
  delete uproar.onTryHit;
  const condition = uproar.condition;
  const anySetStatus = condition.onAnySetStatus;
  uproar.condition = {
    ...condition,
    onStart(target) {
      this.effectState.time = this.effectState.duration;
      delete this.effectState.duration;
      this.add('-start', target, 'Uproar');
    },
    onResidualOrder: 10,
    onResidualSubOrder: 11,
    onResidual(target) {
      for (const pokemon of battlerOrder(this)) {
        if (pokemon.status === 'slp' && !pokemon.hasAbility('soundproof')) wakeInUproar(this, pokemon);
      }
      this.effectState.time--;
      if (this.effectState.failed || !this.effectState.time) {
        target.removeVolatile('uproar');
        return;
      }
      this.add('-start', target, 'Uproar', '[upkeep]');
    },
    onAnySetStatus(status, pokemon) {
      if (pokemon.hasAbility('soundproof')) return;
      return anySetStatus.call(this, status, pokemon);
    },
    onAfterSetStatus(status, target) {
      if (this.gen === 3 && ['slp', 'frz'].includes(status.id)) delete target.volatiles['uproar'];
    },
  };
  uproar.onMoveFail = function (target, source, move) {
    const state = source.volatiles['uproar'];
    if (!state || !target) return;
    if (this.gen === 3 && target.volatiles['protect']) {
      delete source.volatiles['uproar'];
    } else if (!target.runImmunity(move.type)) {
      state.failed = true;
    }
  };
  const slp = data.Conditions.slp;
  const sleepBeforeMove = slp.onBeforeMove;
  slp.onBeforeMove = function (pokemon, ...rest) {
    if ([3, 4].includes(this.gen) && !pokemon.hasAbility('soundproof') && uproarOnField(this)) {
      wakeInUproar(this, pokemon);
      return;
    }
    return sleepBeforeMove.call(this, pokemon, ...rest);
  };
  // (Gen 3 shares Gen 4's Yawn.)
  const yawn = data.Moves.yawn;
  if (!yawn.onTryHit[uproarStopped]) {
    const yawnTryHit = yawn.onTryHit;
    yawn.onTryHit = function (target, ...rest) {
      if ([3, 4].includes(this.gen) && !target.hasAbility('soundproof') && uproarOnField(this)) {
        return false;
      }
      return yawnTryHit.call(this, target, ...rest);
    };
    yawn.onTryHit[uproarStopped] = true;
  }
  for (const id of ['par', 'confusion']) {
    whenUproarStopped(data.Conditions[id], 'onBeforeMove', failUproar);
  }
  whenUproarStopped(data.Conditions.flinch, 'onBeforeMove', cancelUproar);
  for (const id of ['attract', 'disable', 'taunt']) {
    whenUproarStopped(data.Moves[id]?.condition, 'onBeforeMove', cancelUproar);
  }
  whenUproarStopped(data.Moves.imprison?.condition, 'onFoeBeforeMove', cancelUproar);
  whenUproarStopped(data.Moves.gravity?.condition, 'onBeforeMove', failUproar);
}

// I65 — Gen 3: a locked turn of Thrash, Petal Dance, Outrage or Uproar that misses or meets
// Protect spends a PP. pokeemerald's scripts skip `ppreduce` (`jumpifstatus2 ...
// STATUS2_MULTIPLETURNS`) only on the way to a hit; a miss or a protected target goes to
// `BattleScript_PrintMoveMissed`, whose `ppreduce` runs (`HITMARKER_NO_PPDEDUCT` is only set for
// Struggle and a Soundproof target). Showdown never deducts PP on a locked turn. The patch deducts
// one in the moves' `onMoveFail` when the lock was there (it only comes with a hit) and the move
// missed (`-miss` is the last line) or the target holds Protect. Not modelled: a locked turn against
// an immune target whose hit test also failed (the cartridge rolls before its type check and spends
// the PP; Showdown reports `-immune` either way). Draws nothing; the PP only matters once it runs
// out.
for (const mod of patched ? ['gen3'] : []) {
  const moves = Dex.mod(mod).data.Moves;
  for (const [id, lock] of [...RAMPAGE_MOVES.map(id => [id, 'lockedmove']), ['uproar', 'uproar']]) {
    const moveFail = moves[id].onMoveFail;
    moves[id].onMoveFail = function (target, source, move) {
      const missed = this.log[this.log.length - 1]?.startsWith('|-miss|');
      if (this.gen === 3 && source.volatiles[lock] && target &&
        (missed || target.volatiles['protect'])) {
        source.deductPP(move.id, 1);
      }
      return moveFail.call(this, target, source, move);
    };
  }
}

// I66 — Gen 3-4 BeforeMove order as the cartridge has it (pokeemerald `AtkCanceller_UnableToUseMove`:
// sleep, freeze, Truant, recharge, flinch, Disable, Taunt, Imprison, confusion, paralysis, love;
// pokeplatinum `CheckStatusState`: the same with Gravity and Heal Block after Imprison). Showdown
// checks the recharge first (`mustrecharge` 11, over sleep and freeze's 10), love (2) before
// paralysis (1), Gen 4's Taunt (5) after Gravity and Heal Block (6), and Gen 3's Taunt last (the
// gen3 mod drops its priority). The patch gives the gen3 / gen4 conditions the cartridge's order:
// `mustrecharge` 9.5 (Gen 3-4 copies: the object is shared with Gen 5-9), Taunt 6.9, paralysis
// 2.5. User's standing decision (2026-10-01): an exception to the RNG rule — a sleeping recharger
// counts its sleep down and a frozen one rolls its thaw, the paralysis roll comes before love's,
// and a taunted status move no longer meets the confusion check first. Truant (not implemented)
// keeps Showdown's 9; Imprison (R229) comes right after Taunt at 6.8, before Gen 4's Gravity and
// Heal Block (6) — only the `cant` reason changes.
for (const mod of patched ? ['gen3', 'gen4'] : []) {
  const data = Dex.mod(mod).data;
  data.Conditions.mustrecharge = {...data.Conditions.mustrecharge, onBeforeMovePriority: 9.5};
  data.Conditions.par = {...data.Conditions.par, onBeforeMovePriority: 2.5};
  data.Moves.taunt = {...data.Moves.taunt,
    condition: {...data.Moves.taunt.condition, onBeforeMovePriority: 6.9}};
  data.Moves.imprison = {...data.Moves.imprison,
    condition: {...data.Moves.imprison.condition, onFoeBeforeMovePriority: 6.8}};
}

// I67 — Gen 3-4: a move stopped before it starts leaves no last move. pokeemerald's
// `MOVEEND_UPDATE_LAST_MOVES` sets `gLastMoves` to `MOVE_UNAVAILABLE` when `HITMARKER_OBEYS` was never
// set (it only is once `AtkCanceller_UnableToUseMove` let the move through); pokeplatinum sets
// `movePrevByBattler` to `MOVE_NONE` without `SYSCTL_MOVE_SUCCEEDED` (set as the move's script is
// loaded, past every BeforeMove check). Showdown's `runMove` returns before `moveUsed`, keeping the
// last move used. The patch clears `lastMove` in the gen3 / gen4 formats' `MoveAborted`. User's
// standing decision (2026-10-01): an exception to the RNG rule — Encore and Disable now fail on a
// Pokémon whose last attempt was stopped (asleep, frozen, recharging, flinched, fully paralysed …).
function clearLastMove(pokemon) {
  if ([3, 4].includes(this.gen)) pokemon.lastMove = null;
}
for (const [id, format] of patched ? Object.entries(Dex.data.Rulesets) : []) {
  if (!/^gen[34]/.test(id)) continue;
  format.onMoveAborted = clearLastMove;
  Dex.formats.get(id).onMoveAborted = clearLastMove;
}

// I56 — Gen 4 Bug Bite and Pluck read the Custap Berry's flag as Knock Off does (pokeplatinum
// `BtlCmd_TryPluck`: no effect on a target holding an item whose `moveEffectsData.custapBerry` is
// set — its Custap Berry went off as the turn's actions were ordered and it has not moved yet).
// Showdown's gen4 Custap Berry waits in the queue until its holder moves, and is stolen and eaten
// meanwhile. The moves' data is shared with Gen 5-9; `priorityItemPending` asks the generation. No
// draw.
if (patched) {
  for (const id of ['bugbite', 'pluck']) {
    const move = Dex.mod('gen4').data.Moves[id];
    const onHit = move.onHit;
    move.onHit = function (target, ...rest) {
      if (priorityItemPending(this, target)) return;
      return onHit.call(this, target, ...rest);
    };
  }
}

// I57 — a Gen 4 confusion berry that Bug Bite or Pluck eats does nothing to an eater at full HP
// (pokeplatinum `BattleSystem_PluckBerry`: `PLUCK_EFFECT_HP_RESTORE_*` only when `curHP != maxHP` —
// no heal, no confusion; `BattleSystem_FlingItem` has no such check). Showdown's `onEat` heals
// nothing then but still confuses an eater that dislikes the flavor. The patch returns from the
// gen4 berries' `onEat` at full HP while Bug Bite or Pluck is the active move (the data is shared
// with Gen 5-6, hence the generation check). The confusion it no longer starts draws its duration
// no more: an exception to the rule above (TESTING.md §7).
if (patched) {
  for (const id of ['figyberry', 'wikiberry', 'magoberry', 'aguavberry', 'iapapaberry']) {
    const berry = Dex.mod('gen4').data.Items[id];
    const onEat = berry.onEat;
    berry.onEat = function (pokemon, ...rest) {
      if (this.gen === 4 && pokemon.hp >= pokemon.maxhp &&
        ['bugbite', 'pluck'].includes(this.activeMove?.id)) return;
      return onEat.call(this, pokemon, ...rest);
    };
  }
}

// I59 — Gen 4 Bug Bite and Pluck: an eater with Klutz gets nothing from the berry it takes, whatever
// it holds (pokeplatinum `BattleSystem_PluckBerry` asks `Battler_Ability(attacker) == ABILITY_KLUTZ`
// or Embargo). Showdown's `singleEvent('Eat')` asks the eater's `ignoringItem()`, which spares a
// Klutz holder of an item marked `ignoreKlutz` (Macho Brace, the Power items). The suppression
// check runs before `singleEvent` sets its own event, inside the move's `Hit`, so the patch answers
// for the move's user there. No draw.
if (patched) {
  const ignoringItem = Pokemon.prototype.ignoringItem;
  Pokemon.prototype.ignoringItem = function (...args) {
    const battle = this.battle;
    if (battle.gen === 4 && battle.event?.id === 'Hit' && battle.event.source === this &&
      ['bugbite', 'pluck'].includes(battle.activeMove?.id) && this.hasAbility('klutz')) return true;
    return ignoringItem.apply(this, args);
  };
}

// I90 — Gen 4 Fling fails for a user with Multitype or holding a Griseous Orb, whatever it holds
// (pokeplatinum `effect_script_0233.s` checks both before `TryFling`). Showdown's gen4 Fling asks
// only the item's own `TakeItem`. No draw.
// I92 — Embargo on a Gen 4 Fling's target keeps the flung item's status or flinch away too
// (`BattleSystem_FlingItem`: `flingScript = 0`). Showdown adds them as secondaries, which Embargo
// does not touch: the patch zeroes the chance of the ones Fling's `onPrepareHit` added (those
// without a chance), so the secondary's roll is still drawn and the PRNG keeps its course.
if (patched) {
  const fling = Dex.mod('gen4').data.Moves.fling;
  const prepareHit = fling.onPrepareHit;
  fling.onPrepareHit = function (target, source, move) {
    if (this.gen === 4 && (source.ability === 'multitype' || source.item === 'griseousorb')) {
      return false;
    }
    const result = prepareHit.call(this, target, source, move);
    if (this.gen === 4 && result !== false && target.volatiles['embargo'] && move.secondaries) {
      for (const secondary of move.secondaries) {
        if (secondary.chance === undefined) secondary.chance = 0;
      }
    }
    return result;
  };
}

// I91 — a berry a Gen 4 Fling throws works on a target with Klutz (pokeplatinum
// `BattleSystem_FlingItem` keeps only Embargo out). Showdown's `singleEvent('Eat')` asks the
// target's `ignoringItem()`, Klutz included. The suppression check runs before `singleEvent` sets
// its own event, inside Fling's `Hit`, so the patch answers for the move's target there: only
// Embargo. A berry whose effect draws (a confusion berry the target dislikes, Starf Berry) draws
// where Showdown drew nothing: an exception to the rule above (TESTING.md §7).
if (patched) {
  const ignoringItem = Pokemon.prototype.ignoringItem;
  Pokemon.prototype.ignoringItem = function (...args) {
    const battle = this.battle;
    if (battle.gen === 4 && battle.event?.id === 'Hit' && battle.event.target === this &&
      battle.activeMove?.id === 'fling' && battle.event.source !== this) {
      return !!this.volatiles['embargo'];
    }
    return ignoringItem.apply(this, args);
  };
}

// I93 — the berry a Gen 4 Bug Bite or Pluck takes is the target's to Recycle (pokeplatinum
// `subscript_pluck.s`: `RemoveItem BTLSCR_DEFENDER`, which writes the target's `recycleItem`).
// Showdown's `takeItem` leaves `lastItem` alone. The patch sets the target's `lastItem` once the
// gen4 moves' `onHit` took its berry. No draw.
if (patched) {
  for (const id of ['bugbite', 'pluck']) {
    const move = Dex.mod('gen4').data.Moves[id];
    const onHit = move.onHit;
    move.onHit = function (target, ...rest) {
      const item = target.item;
      const result = onHit.call(this, target, ...rest);
      if (this.gen === 4 && item && !target.item) target.lastItem = item;
      return result;
    };
  }
}

// I94 — a Gen 4 Trick or Switcheroo fails at once when the user or the target has been Knock Off'd
// (pokeplatinum `BtlCmd_TrySwapItems` checks `knockedOffItemsMask` for both before it looks at the
// items). Showdown's gen4 Switcheroo is the gen7 mod's, which only fails while an item stays held:
// `takeItem`'s `false` for a marked Pokémon holding nothing lets the user's item through to a marked
// target. Both moves also ask the unmarked side's `TakeItem` first, which starts its Unburden. The
// patch fails them before either `takeItem`; Trick's object serves every generation and the gen7
// mod's Switcheroo Gen 4-7, hence the generation check. No draw.
if (patched) {
  for (const id of ['trick', 'switcheroo']) {
    const move = Dex.mod('gen4').data.Moves[id];
    const onHit = move.onHit;
    move.onHit = function (target, source, ...rest) {
      if (this.gen === 4 && (target.itemKnockedOff || source.itemKnockedOff)) return false;
      return onHit.call(this, target, source, ...rest);
    };
  }
}

// I68 — Gen 3-4 Lock-On / Mind Reader as the cartridge keeps the lock (pokeemerald
// `Cmd_setalwayshitflag`, `SwitchInClearSetData`; pokeplatinum `subscript_lock_on`,
// `BattleSystem_UpdateAfterSwitch`): the lock is the target's (`STATUS3_ALWAYS_HITS` /
// `MOVE_EFFECT_LOCK_ON` with the locker's battler), so taking aim again is never refused — it sets the
// lock anew; the locker's Baton Pass hands it on with its 2 turns afresh, and the target's keeps it on
// the replacement. Showdown keeps one `lockon` on the user, refuses the move while it is there, and
// copies it through Baton Pass with the turns it has left. The patch lets the gen3 / gen4 moves'
// `onTryHit` drop the old lock instead of failing, gives the gen3 / gen4 `lockon` an `onCopy` that
// sets its `duration` to 2, and has a Baton Pass replacement take over the locks on the Pokémon it
// replaced. Not modelled: in doubles a locker's two locks at once (the cartridge flags each target).
// User's standing decision (2026-10-01): an exception to the RNG rule — a lock set anew or kept skips
// accuracy rolls Showdown makes.
for (const mod of patched ? ['gen3', 'gen4'] : []) {
  const data = Dex.mod(mod).data;
  for (const id of ['lockon', 'mindreader']) {
    const tryHit = data.Moves[id].onTryHit;
    data.Moves[id].onTryHit = function (target, source, ...rest) {
      if ([3, 4].includes(this.gen) && source.volatiles['lockon']) {
        delete source.volatiles['lockon'];
        return;
      }
      return tryHit.call(this, target, source, ...rest);
    };
  }
  data.Moves.lockon.condition = {
    ...data.Moves.lockon.condition,
    onCopy() {
      if ([3, 4].includes(this.gen)) this.effectState.duration = 2;
    },
  };
}
if (patched) {
  const copyVolatileFrom = Pokemon.prototype.copyVolatileFrom;
  Pokemon.prototype.copyVolatileFrom = function (pokemon, switchCause) {
    copyVolatileFrom.call(this, pokemon, switchCause);
    if (![3, 4].includes(this.battle.gen) || switchCause === 'shedtail') return;
    for (const foe of this.battle.getAllActive()) {
      const lock = foe.volatiles['lockon'];
      if (lock && lock.source === pokemon) lock.source = this;
    }
  };
}

// I69 — Gen 2 Lock-On / Mind Reader as the cartridge has them (pokecrystal `BattleCommand_LockOn`,
// `BattleCommand_CheckHit`'s `.LockOn`): the move flags the target (failing only behind a Substitute or
// on a miss); the flag has no time limit and stays when the user switches out; the next accuracy
// check against the target — after its Protect, before its flight or dig — reads and clears it and
// hits, reaching a digging target, and a flying one with anything but Earthquake, Fissure and
// Magnitude; the target's Baton Pass hands it on. Showdown puts a 2-turn `lockon` on the user that
// only skips the accuracy roll, and fails the move against a target under Foresight or Lock-On. The
// patch makes the gen2 moves add a `lockedon` volatile to the target (no `onTryHit`), whose
// `onInvulnerability` lets the move through (or clears the flag for Earthquake, Fissure and Magnitude
// against a flying target) and whose `onAccuracy` clears it and hits. User's standing decision
// (2026-10-01): an exception to the RNG rule — the lock lasts until used and reaches hidden targets.
if (patched) {
  const data = Dex.mod('gen2').data;
  data.Conditions.lockedon = {
    name: 'lockedon',
    onInvulnerabilityPriority: 1,
    onInvulnerability(target, source, move) {
      if (!move || source.isAlly(target)) return;
      if (target.volatiles['fly'] && ['earthquake', 'fissure', 'magnitude'].includes(move.id)) {
        target.removeVolatile('lockedon');
        return;
      }
      return 0;
    },
    onAccuracy(accuracy, target, source, move) {
      if (!move || source.isAlly(target)) return;
      target.removeVolatile('lockedon');
      return true;
    },
  };
  for (const id of ['lockon', 'mindreader']) {
    const move = data.Moves[id];
    delete move.onTryHit;
    move.onHit = function (target, source, effect) {
      target.addVolatile('lockedon', source, effect);
      this.add('-activate', source, `move: ${effect.name}`, `[of] ${target}`);
    };
  }
}

// I47 — Gen 3-4 Shell Bell as the cartridges count it. pokeemerald heals at every move end
// (`MOVEEND_KINGSROCK_SHELLBELL`): a multi-hit move's after each hit (`moveendto
// MOVEEND_NEXT_TARGET` in `BattleScript_DoMultiHit`), a spread move's after each target, each
// from that hit's damage (`shellBellDmg`, a Substitute's included); pokeplatinum adds the whole
// action's damage to HP and to Substitutes into `shellBellDamageDealt` and heals once after the
// move. Showdown's gen3 and gen4 heal `move.totalDamage / 8` once — Substitute damage left out,
// and in gen3 only the last target's. The patch counts each hit's damage (`moveHit`'s result and
// the source's `AfterSubDamage`) on the move: gen4 heals their sum once, gen3 heals each hit of
// a multi-hit move as it lands and every other hit at the move's end, each rounded on its own.
// No draw.
if (patched) {
  const shellbell = Dex.mod('gen4').data.Items.shellbell;
  const healHit = function (battle, pokemon, damage) {
    if (!damage || !pokemon.hp || pokemon.hp >= pokemon.maxhp) return;
    battle.heal(Math.max(1, Math.floor(damage / 8)), pokemon, pokemon, battle.dex.items.get('shellbell'));
  };
  const record = function (move, target, damage) {
    move.shellBellHits ||= [];
    move.shellBellHits.push({target, damage});
  };
  const onAfterMoveSecondarySelf = shellbell.onAfterMoveSecondarySelf;
  shellbell.onAfterMoveSecondarySelf = function (pokemon, target, move) {
    if (this.gen > 4) return onAfterMoveSecondarySelf.call(this, pokemon, target, move);
    if (pokemon.forceSwitchFlag) return;
    const hits = (move.shellBellHits || []).filter(hit => hit.target !== pokemon);
    if (this.gen === 4) {
      const total = (move.totalDamage || 0) + hits.reduce((sum, hit) => sum + hit.damage, 0);
      if (total) this.heal(total / 8, pokemon);
      return;
    }
    for (const hit of hits) {
      if (!hit.healed) healHit(this, pokemon, hit.damage);
    }
  };
  // Gen 3-4: the damage a Substitute took counts (gen4: on top of `move.totalDamage`).
  shellbell.onSourceAfterSubDamage = function (damage, target, source, move) {
    if (this.gen > 4 || !move) return;
    record(move, target, damage);
    if (this.gen === 3 && move.multihit) {
      move.shellBellHits[move.shellBellHits.length - 1].healed = true;
      healHit(this, source, damage);
    }
  };
  // Gen 3: every hit's damage to HP, on its own (`moveHit` runs once per hit and target).
  const actions = Dex.mod('gen3').data.Scripts.actions;
  actions.moveHit = function (target, pokemon, move, ...rest) {
    const result = Object.getPrototypeOf(this).moveHit.call(this, target, pokemon, move, ...rest);
    if (typeof result === 'number' && result > 0 && target && !Array.isArray(target) &&
      target !== pokemon && pokemon.hasItem('shellbell')) {
      record(move, target, result);
      if (move.multihit) {
        move.shellBellHits[move.shellBellHits.length - 1].healed = true;
        healHit(this.battle, pokemon, result);
      }
    }
    return result;
  };
}

// I48 — Gen 3-4 White Herb after a newcomer's Intimidate. pokeplatinum checks every battler's
// White Herb once the switch-in abilities are done (`SWITCH_IN_CHECK_STATE_HELD_ITEM_STATUS`);
// pokeemerald checks the newcomer's own item (`ITEMEFFECT_ON_SWITCH_IN`) and, after the action,
// the first battler's (`HandleFaintedMonActions`: `ItemBattleEffects(ITEMEFFECT_NORMAL, 0, …)`).
// The gen4 `runSwitch` (gen3 inherits it) runs `SwitchIn` — the herb's `onAnySwitchIn` — before
// the newcomer's ability starts, so an Intimidate it brings stays until the next move's end. The
// patch drops the herb's gen3-4 `onAnySwitchIn` and, after `runSwitch`, starts the herb of every
// Pokémon on the field (gen4) or of the newcomer and P1's first slot (gen3), fastest first (a tie
// keeps position order: no draw).
if (patched) {
  const whiteherb = Dex.data.Items.whiteherb;
  for (const item of new Set([whiteherb, Dex.mod('gen3').data.Items.whiteherb, Dex.mod('gen4').data.Items.whiteherb])) {
    const onAnySwitchIn = item.onAnySwitchIn;
    item.onAnySwitchIn = function (...args) {
      if (this.gen <= 4) return;
      return onAnySwitchIn.apply(this, args);
    };
  }
  for (const mod of ['gen3', 'gen4']) {
    const actions = Dex.mod(mod).data.Scripts.actions;
    const runSwitch = actions.runSwitch;
    actions.runSwitch = function (pokemon) {
      const result = runSwitch.call(this, pokemon);
      const battle = this.battle;
      const first = battle.sides[0].active[0];
      const holders = battle.getAllActive()
        .map((holder, index) => ({holder, index}))
        .filter(({holder}) => holder.hasItem('whiteherb') &&
          (battle.gen === 4 || holder === pokemon || holder === first));
      holders.sort((a, b) => b.holder.speed - a.holder.speed || a.index - b.index);
      for (const {holder} of holders) {
        battle.singleEvent('Start', holder.getItem(), holder.itemState, holder);
      }
      return result;
    };
  }
}

// I84 — Gen 4 Trace as pokeplatinum's switch-in check has it (`ChooseTraceTarget`, §41.10 A5): it
// leaves out the foes it may not copy (Forecast, Trace, Multitype: the gen4 `notrace` ones) and
// those without HP first, then takes the only one left — no draw — or, with two, one at random;
// with none it looks again at the next check. Showdown's gen4 `onUpdate` takes `randomFoe()` first
// (a draw even with one foe) and gives up for the time being on one it may not copy. An exception
// to the RNG rule. (Gen 3, I83, needs no patch: what the cartridge does differently cannot be
// reached in Showdown's switch flow.)
if (patched) {
  Dex.mod('gen4').data.Abilities.trace.onUpdate = function (pokemon) {
    if (!this.effectState.seek) return;
    const foes = pokemon.foes().filter(foe => !foe.getAbility().flags['notrace']);
    if (!foes.length) return;
    const target = foes.length > 1 ? foes[this.random(foes.length)] : foes[0];
    pokemon.setAbility(target.getAbility(), target);
  };
}

// I71 — Gen 3 Spite as the cartridge has it (pokeemerald `Cmd_tryspiteppreduce`): the target's last
// move needs more than 1 PP left, and a move left with none ends the target's multi-turn move
// (`CancelMultiTurnMoves`: a two-turn move's charge, Thrash's lock without its confusion, Uproar,
// Bide, Rollout and Ice Ball — silently; Ice Ball rolls under its own `iceball` volatile, which the
// patch first left out, R-M8-147). Showdown fails only at 0 PP and lets the lock go on. The patch
// replaces the gen3 `onHit`, drawing first as Showdown does (the cartridge draws after its checks,
// as the engine does: a draw the replay never sees), so it changes no draw. The message still
// shows the draw. The gen3 `spite` is also Gen 2's (the gen2 mod inherits the object), whose
// cartridge fails only at 0 PP (pokecrystal `BattleCommand_Spite`): Gen 2 keeps Showdown's `onHit`.
const cancelMultiTurnMoves = pokemon => {
  const twoTurn = pokemon.volatiles['twoturnmove'];
  if (twoTurn) {
    delete pokemon.volatiles[twoTurn.move];
    delete pokemon.volatiles['twoturnmove'];
  }
  for (const id of ['lockedmove', 'uproar', 'bide', 'rollout', 'iceball']) delete pokemon.volatiles[id];
  pokemon.m.furyCutter = 0;
};
if (patched) {
  const spite = Dex.mod('gen3').data.Moves.spite;
  const onHit = spite.onHit;
  spite.onHit = function (target, ...rest) {
    if (this.gen !== 3) return onHit.call(this, target, ...rest);
    const roll = this.random(2, 6);
    const move = target.lastMove;
    const slot = move && target.getMoveData(move.id);
    if (!slot || slot.pp < 2) return false;
    target.deductPP(move.id, roll);
    this.add('-activate', target, 'move: Spite', move.id, roll);
    if (!slot.pp) cancelMultiTurnMoves(target);
  };
}

// I72 — Gen 3-4 Pressure's reach as the cartridge counts it (pokeemerald `Cmd_ppreduce`,
// `PressurePPLoseOnUsingImprison`, `PressurePPLoseOnUsingPerishSong`; pokeplatinum
// `BattleControllerPlayer_DecrementPP`). Gen 3 counts nothing for a move aimed at its user —
// a field move (Haze, the weathers), one for the user's side (Reflect, Heal Bell), Helping Hand
// — but the foes for Imprison and everyone else for Perish Song. Gen 4 counts nothing for a move
// for the user's side, the user or an ally, nor for Counter / Mirror Coat / Metal Burst (aimed
// at their user as they start), the foes for Imprison, and everyone else standing for a field
// move. Both count the foes for a foe-side move and only Pokémon still standing. Showdown's
// `pressureTargets` are the move's targets: its gen3 / gen4 field and own-side moves count the
// user's allies (`onDeductPP` spares only the user) and a fainted foe still in its slot, and
// Imprison nobody. The patch replaces the gen3 / gen4 `pressureTargets`; no draw changes.
const SPORTS = ['mudsport', 'watersport'];
const cartridgePressureTargets = (pokemon, move, targets) => {
  const gen = pokemon.battle.gen;
  const foes = pokemon.foes();
  const others = [...pokemon.allies(), ...foes];
  if (move.id === 'imprison') return foes;
  if (gen === 3 && move.id === 'perishsong') return others;
  // I23 aims the gen4 sports at their user; their reach is still the field's (pokeplatinum
  // `RANGE_FIELD`).
  switch (gen === 4 && SPORTS.includes(move.id) ? 'all' : move.target) {
  case 'all': return gen === 4 ? others : [];
  case 'allySide': case 'allyTeam': case 'allies': case 'self': case 'adjacentAlly':
  case 'adjacentAllyOrSelf':
    return [];
  case 'foeSide': return foes;
  case 'scripted': return gen === 4 ? [] : targets;
  default: return targets;
  }
};
if (patched) {
  const getMoveTargets = Pokemon.prototype.getMoveTargets;
  Pokemon.prototype.getMoveTargets = function (move, target) {
    const result = getMoveTargets.call(this, move, target);
    // Gen 5: the sports I23 aims at their user meet the foes' Pressure as Showdown's field move
    // (`target: 'all'`) does.
    if (this.battle.gen === 5 && SPORTS.includes(move.id)) {
      return {targets: result.targets, pressureTargets: this.foes()};
    }
    if (this.battle.gen !== 3 && this.battle.gen !== 4) return result;
    return {targets: result.targets, pressureTargets: cartridgePressureTargets(this, move, result.targets)};
  };
}

// I73 — Gen 2 Curse by a user that is not a Ghost type, as the cartridge has it (pokecrystal
// `BattleCommand_Curse`): with Attack and Defense both at +6 it fails, Speed untouched. Showdown's
// `onTryHit` (base, which Gen 2 keeps) still lowers the Speed. (Its order, Speed first, is the
// cartridge's already: the base `onTryHit` replaces the gen4 `onModifyMove`'s `self`.) The patch
// fails the gen2 move in that case. No draw changes.
if (patched) {
  const curse = Dex.mod('gen2').data.Moves.curse;
  const onTryHit = curse.onTryHit;
  curse.onTryHit = function (target, source, move) {
    if (!source.hasType('Ghost') && source.boosts.atk >= 6 && source.boosts.def >= 6) return false;
    return onTryHit.call(this, target, source, move);
  };
}

// I75 — Gen 2-4 Fury Cutter as the cartridges count it (pokecrystal `BattleCommand_FuryCutter`;
// pokeemerald `Cmd_furycuttercalc`, `CancelMultiTurnMoves`; pokeplatinum
// `BtlCmd_CalcFuryCutterPower`, `Battler_UnlockMoveChoice`): a count of the uses that landed (a
// Substitute's too), with no time limit, doubling `min(count, 5) − 1` times — Gen 2 the damage
// after the type matchups and before the random factor, Gen 3-4 the power. A use that does not
// land ends it, and so does a switch; Gen 2 also any other move; Gen 3 recharging, a flinch,
// Disable, Taunt, Imprison, love, and sleep or freeze put on the user; Gen 4 a flinch, full
// paralysis, a confusion self-hit, love, Taunt, Disable, Imprison, Gravity, recharging, and sleep
// or freeze put on the user. Showdown keeps a 2-turn `furycutter` volatile, renewed by each use
// that reaches the damage calculation (Gen 2-3: only by its `onHit`, which a Substitute skips),
// doubling the power up to 160. The patch keeps the count on `pokemon.m.furyCutter`; Gen 2's
// doubling multiplies the damage before the random factor (`cartridgeGen2GetDamage`; a damage of 1
// so doubled draws the factor, I77).
const endFuryCutter = pokemon => {
  if (pokemon) pokemon.m.furyCutter = 0;
};
const endFuryCutterWhenStopped = (effect, callback, gens) => {
  if (!effect?.[callback]) return;
  const original = effect[callback];
  effect[callback] = function (pokemon, ...rest) {
    const result = original.call(this, pokemon, ...rest);
    if ((result === false || result === null) && gens.includes(this.gen)) endFuryCutter(pokemon);
    return result;
  };
};
for (const mod of patched ? ['gen2', 'gen3', 'gen4'] : []) {
  const data = Dex.mod(mod).data;
  const furycutter = data.Moves.furycutter;
  furycutter.basePowerCallback = function (pokemon, target, move) {
    const count = Math.min((pokemon.m.furyCutter || 0) + 1, 5);
    pokemon.m.furyCutter = count;
    if (this.gen === 2) {
      move.furyCutterDoublings = count - 1;
      return move.basePower;
    }
    return move.basePower * 2 ** (count - 1);
  };
  delete furycutter.onHit;
  furycutter.onMoveFail = function (target, source) {
    endFuryCutter(source);
  };
  if (mod === 'gen3') {
    for (const id of ['mustrecharge', 'flinch']) endFuryCutterWhenStopped(data.Conditions[id], 'onBeforeMove', [3]);
    for (const id of ['disable', 'taunt', 'attract']) {
      endFuryCutterWhenStopped(data.Moves[id]?.condition, 'onBeforeMove', [3]);
    }
    endFuryCutterWhenStopped(data.Moves.imprison?.condition, 'onFoeBeforeMove', [3]);
  }
  if (mod === 'gen4') {
    for (const id of ['mustrecharge', 'flinch', 'par', 'confusion']) {
      endFuryCutterWhenStopped(data.Conditions[id], 'onBeforeMove', [4]);
    }
    for (const id of ['disable', 'taunt', 'attract', 'gravity']) {
      endFuryCutterWhenStopped(data.Moves[id]?.condition, 'onBeforeMove', [4]);
    }
    endFuryCutterWhenStopped(data.Moves.imprison?.condition, 'onFoeBeforeMove', [4]);
  }
}
if (patched) {
  const clearVolatile = Pokemon.prototype.clearVolatile;
  Pokemon.prototype.clearVolatile = function (...args) {
    if (this.battle.gen <= 4) endFuryCutter(this);
    return clearVolatile.apply(this, args);
  };
  const setStatus = Pokemon.prototype.setStatus;
  Pokemon.prototype.setStatus = function (status, ...rest) {
    const result = setStatus.call(this, status, ...rest);
    if (result && [3, 4].includes(this.battle.gen) && ['slp', 'frz'].includes(this.status)) endFuryCutter(this);
    return result;
  };
  const actions = Dex.mod('gen2').data.Scripts.actions;
  const runMove = actions.runMove;
  actions.runMove = function (moveOrMoveName, pokemon, ...rest) {
    // The move the turn uses: an Encore's (`encore.onOverrideAction` swaps the chosen move
    // inside `runMove`), so an encored Fury Cutter after a chosen Encore keeps counting
    // (R-M8-106).
    const chosen = this.dex.moves.get(moveOrMoveName).id;
    const used = pokemon.volatiles['encore']?.move || chosen;
    if (used !== 'furycutter') endFuryCutter(pokemon);
    return runMove.call(this, moveOrMoveName, pokemon, ...rest);
  };
  // Gen 2's damage doubling: `cartridgeGen2GetDamage` (I51 above) multiplies the damage.
}

// I76 — Gen 2 Bide's storing turns are turns like any other for the residual damage (pokecrystal:
// `ResidualDamage` runs after each side's turn whatever it did). Showdown's gen2 `runMove` returns
// at once when the move's `beforeMoveCallback` (Bide's, while it stores) says so, before
// `AfterMoveSelf` — a poisoned, burned or seeded Bide user takes nothing on those turns. The patch
// runs `AfterMoveSelf` there as the end of `runMove` does. No draw.
if (patched) {
  const bide = Dex.mod('gen2').data.Moves.bide;
  const beforeMoveCallback = bide.beforeMoveCallback;
  bide.beforeMoveCallback = function (pokemon, target, move) {
    const result = beforeMoveCallback.call(this, pokemon, target, move);
    if (result && pokemon.side.foe.active[0]?.hp) this.runEvent('AfterMoveSelf', pokemon, target, move);
    return result;
  };
}

// I186 — Crafty Shield does not stop a move on the user's whole party. Bulbapedia (Howl, from Gen 8 aimed at
// the user and its allies: "not blocked by protection moves, including Max Guard and Crafty Shield"; the
// Sword / Shield research found Coaching goes through it too): it guards against status moves aimed at the
// Pokémon on its side, not the ones a member uses on the whole party. Showdown's `craftyshield` condition
// stops any status move but those aimed at `self` or `all`, so an `allies` move (Howl, Jungle Healing, Life
// Dew, Lunar Blessing) fails on the user and its partner alike. The patch lets `allies` through. No draw.
// Nor does it stop a move the user uses on itself (Bulbapedia: "an ally can still use Acupressure ... on
// itself"; R-M11-91): Showdown's check reads the move's target type, and Acupressure's `adjacentAllyOrSelf`
// aimed at the user fails. The patch lets a move through whose target is its user; the Acupressure that
// goes ahead draws its stat.
if (patched) {
  const condition = Dex.data.Moves.craftyshield.condition;
  const tryHit = condition.onTryHit;
  condition.onTryHit = function (target, source, move) {
    if (move.target === 'allies' || target === source) return;
    return tryHit.call(this, target, source, move);
  };
}

// I188 — Innards Out answers a Bide release. Its handler adds the hit loop's `move.totalDamage` (a
// multi-hit move's earlier hits); Bide's release is a bare `moveData` sent to `tryMoveHit`, which has
// none, so the sum is NaN and the attacker takes nothing. Bulbapedia: a Pokémon with Innards Out
// knocked out by an opponent's attack deals the HP it had left to that opponent. The patch reads a
// missing total as 0. No draw.
if (patched) {
  for (const mod of ['gen7', 'gen8', 'gen9']) {
    const innardsOut = Dex.mod(mod).data.Abilities.innardsout;
    const onDamagingHit = innardsOut.onDamagingHit;
    innardsOut.onDamagingHit = function (damage, target, source, move) {
      if (move && move.totalDamage === undefined) move.totalDamage = 0;
      return onDamagingHit.call(this, damage, target, source, move);
    };
  }
}

// I192 — Mind Blown and Steel Beam cost their user half its HP when Parental Bond made them hit twice
// and nothing was hit. Parental Bond's `onPrepareHit` sets `multihit`, so the move's `onAfterMove`
// leaves the cost to the hit loop (`mindBlownRecoil`), which a target's Protect or a miss never
// reaches. Bulbapedia (Mind Blown): the user takes the damage even if the target is protected or the
// move misses; with Parental Bond, once. The patch pays a cost the hit loop left unpaid in
// `onAfterMove` (a Damp `TryMove` failure clears `mindBlownRecoil` first and still costs nothing).
// No draw.
if (patched) {
  // Each generation's own copy (the gen8 mod lists Mind Blown again; earlier patches loaded the mods).
  const moves = new Set(['mindblown', 'steelbeam'].flatMap(id =>
    [Dex.data.Moves[id], ...['gen7', 'gen8', 'gen9'].map(mod => Dex.mod(mod).data.Moves[id])].filter(Boolean)));
  for (const move of moves) {
    const onAfterMove = move.onAfterMove;
    move.onAfterMove = function (pokemon, target, activeMove) {
      if (activeMove.mindBlownRecoil && activeMove.multihit) {
        const hpBeforeRecoil = pokemon.hp;
        this.damage(Math.round(pokemon.maxhp / 2), pokemon, pokemon, this.dex.conditions.get(activeMove.id), true);
        activeMove.mindBlownRecoil = false;
        if (pokemon.hp <= pokemon.maxhp / 2 && hpBeforeRecoil > pokemon.maxhp / 2) {
          this.runEvent('EmergencyExit', pokemon, pokemon);
        }
        return;
      }
      return onAfterMove.call(this, pokemon, target, activeMove);
    };
  }
}

// I190 — a Stalwart or Propeller Tail user's locked move after a called one goes at a foe. A move
// called by another (Metronome's Outrage) leaves `lastMoveTargetLoc` at the caller's, the user's
// own position (the `twoturnmove` condition says so, and corrects it for two-turn moves); the
// lock's next action aims there, and `getTarget` returns a target-tracking user's `originalTarget`
// — itself — before `randomNormal` draws a foe. Outrage hits its user. Bulbapedia: Outrage,
// Thrash and Petal Dance attack a random opponent; Stalwart and Propeller Tail only ignore
// redirection. The patch tracks no user that its move cannot aim at. Singles: no draw; doubles:
// the draw a random target makes — allowed by the user (2026-10-12, TESTING.md §7).
if (patched) {
  const getTarget = Battle.prototype.getTarget;
  const aimedElsewhere = ['normal', 'any', 'adjacentFoe', 'adjacentAlly', 'randomNormal'];
  Battle.prototype.getTarget = function (pokemon, move, targetLoc, originalTarget) {
    if (originalTarget === pokemon && aimedElsewhere.includes(this.dex.moves.get(move).target)) {
      originalTarget = undefined;
    }
    return getTarget.call(this, pokemon, move, targetLoc, originalTarget);
  };
}

// I189 — a Gen 1 Substitute broken by an earlier move leaves no mark. pokered's `AttackSubstitute` stops a
// multi-hit move by zeroing the attacker's move effect as the doll breaks — this move's, nothing after it.
// Showdown keeps `subFainted` on the Pokémon until a `runMove` aimed at it clears it, so a move Mirror Move
// or Metronome calls at a Pokémon whose doll broke on an earlier turn (the caller aims at itself) stops
// after its first hit. The patch clears the other active Pokémon's flag as a called move starts; a doll
// the called move breaks itself still stops it. User's decision (2026-10-12): an exception to the RNG
// rule — the hits that now follow draw what they draw (Twineedle's poison roll on its last hit).
if (patched) {
  const actions = Dex.mod('gen1').data.Scripts.actions;
  const useMove = actions.useMove;
  actions.useMove = function (moveOrMoveName, pokemon, options) {
    const caller = options?.sourceEffect ?? this.battle.effect;
    if (caller && ['metronome', 'mirrormove'].includes(caller.id)) {
      for (const active of this.battle.getAllActive()) {
        if (active !== pokemon) active.subFainted = null;
      }
    }
    return useMove.call(this, moveOrMoveName, pokemon, options);
  };
}

// I49 — Gen 2-3 PP berries wait for the end of the turn. pokecrystal `HandleMysteryberry` (the
// between-turn effects, after Leftovers) and pokeemerald `ItemBattleEffects(ITEMEFFECT_NORMAL, …,
// FALSE)` (`ENDTURN_ITEMS1`: `HOLD_EFFECT_RESTORE_PP` runs only `!moveTurn`) restore the first
// move with no PP left; Showdown's `onUpdate` eats the berry as soon as a move runs out (Gen 2:
// only when it is the last move's). The patch takes the Gen 2 Mystery Berry's and the Gen 3 Leppa
// Berry's `onUpdate` away (gen3 gets its own copy of the entry it shares with gen4-5) and checks
// at the berries' end-of-turn residual (gen2 order 10, gen3 10.4) for the first slot with 0 PP.
// The new residual handler is the only draw it can add: a speed tie with another Pokémon's
// handler at that step, which Showdown shuffles (the engine draws the same tie).
for (const mod of patched ? ['gen2', 'gen3'] : []) {
  const items = Dex.mod(mod).data.Items;
  const id = mod === 'gen2' ? 'mysteryberry' : 'leppaberry';
  items[id] = {
    ...items[id],
    onUpdate: undefined,
    onResidualOrder: 10,
    ...(mod === 'gen3' ? {onResidualSubOrder: 4} : {}),
    onResidual(pokemon) {
      if (!pokemon.hp) return;
      const moveSlot = pokemon.moveSlots.find(slot => slot.pp === 0);
      if (!moveSlot) return;
      if (id === 'mysteryberry') {
        pokemon.addVolatile('leppaberry');
        pokemon.volatiles['leppaberry'].moveSlot = moveSlot;
      }
      pokemon.eatItem();
    },
  };
}

// I185 — the Gen 3 Leppa Berry reads the party's moves. pokeemerald's `HOLD_EFFECT_RESTORE_PP` looks
// for the first move with no PP left in `GetMonData(mon, MON_DATA_MOVE1 + i)` / `MON_DATA_PP1 + i` —
// the party data, which a mimicked or transformed slot never writes back to (`Cmd_ppreduce`) — and
// restores it up to `CalculatePPWithBonus`, writing the battler's PP only where that slot is the
// party's own move (`MOVE_IS_PERMANENT`). Showdown reads `moveSlots`, so a mimicked move run out of
// PP eats the berry. The patch has the gen3 Leppa Berry (I49's copy) look in `baseMoveSlots` — the
// party's moves, one object with an own slot the Pokémon battles with — at the residual and as it is
// eaten. No draw.
if (patched) {
  const leppa = Dex.mod('gen3').data.Items.leppaberry;
  leppa.onResidual = function (pokemon) {
    if (!pokemon.hp) return;
    if (!pokemon.baseMoveSlots.some(slot => slot.pp === 0)) return;
    pokemon.eatItem();
  };
  leppa.onEat = function (pokemon) {
    const moveSlot = pokemon.baseMoveSlots.find(slot => slot.pp === 0) ||
      pokemon.baseMoveSlots.find(slot => slot.pp < slot.maxpp);
    if (!moveSlot) return;
    moveSlot.pp = Math.min(moveSlot.pp + 10, moveSlot.maxpp);
    this.add('-activate', pokemon, 'item: Leppa Berry', moveSlot.move, '[consumed]');
  };
}

// I100 — Gen 4 Sticky Barb. pokeplatinum `BattleSystem_TriggerHeldItemOnHit` (`…OnPivotMove`
// for U-turn) moves the barb after the contact abilities (Rough Skin) and only (a) to an attacker
// still standing, (b) not for Knock Off — which then knocks it off — and (c) not to an attacker
// a Knock Off marked (`knockedOffItemsMask`). Showdown's `onHit` hands it over before
// `DamagingHit`, for Knock Off too, asking only the holder's mark (`takeItem()`). The patch moves
// the gen4 handover to an `onDamagingHit` after the abilities' (order 2) with the cartridge's
// conditions; no draw changes.
for (const mod of patched ? ['gen4'] : []) {
  const items = Dex.mod(mod).data.Items;
  items.stickybarb = {
    ...items.stickybarb,
    onHit: undefined,
    onDamagingHitOrder: 2,
    onDamagingHit(damage, target, source, move) {
      if (!source || source === target || !source.hp || source.item || source.itemKnockedOff ||
        move.id === 'knockoff' || !this.checkMoveMakesContact(move, source, target)) return;
      const barb = target.takeItem();
      if (!barb) return;
      source.setItem(barb);
    },
  };
}

// I85 — Gen 4 Conversion as pokeplatinum has it (`BtlCmd_TryConversion`, §41.10 A1): a `???` move
// (Curse) counts — as Ghost for a Ghost user, Normal for another — where Showdown's gen4 `onHit`
// leaves Curse out (Conversion itself stays out). The draw is the same `sample` over the slots,
// except where Curse is the only type left: the cartridge draws and changes the type, Showdown fails
// without a draw — an exception to the RNG rule. (Gen 3 needs no patch: there Conversion's own slot
// counts too, its Normal is the type Curse would add, so only how often Normal comes up differs,
// which the replay does not see.)
if (patched) {
  const conversion = Dex.mod('gen4').data.Moves.conversion;
  const onHit = conversion.onHit;
  conversion.onHit = function (target, ...rest) {
    if (this.gen !== 4) return onHit.call(this, target, ...rest);
    const possibleTypes = target.moveSlots.map(moveSlot => {
      const move = this.dex.moves.get(moveSlot.id);
      if (move.id === 'conversion') return '';
      const type = move.type === '???' ? (target.hasType('Ghost') ? 'Ghost' : 'Normal') : move.type;
      return target.hasType(type) ? '' : type;
    }).filter(type => type);
    if (!possibleTypes.length) return false;
    const type = this.sample(possibleTypes);
    if (!target.setType(type)) return false;
    this.add('-start', target, 'typechange', type);
  };
}

// I77 — Gen 2-4 Rage as the cartridges have it (pokecrystal `BattleCommand_Rage`,
// `BattleCommand_BuildOpponentRage`, `BattleCommand_RageDamage`, `ParsePlayerAction`;
// pokeemerald `BattleScript_EffectRage`, `MOVEEND_RAGE`, `TryClearRageStatuses`; pokeplatinum
// `subscript_set_rage_flag`, `BattleControllerPlayer_RageBuilding`, `AFTER_MOVE_HIT_STATE_RAGE`).
// Showdown keeps one `rage` for Gen 2-9: the user stops raging at its next move attempt, Rage's
// too (`onBeforeMove`), and a damaging hit on its body (`onHit`) — an ally's too — raises its
// Attack, at +6 a `-boost … 0`. The cartridges: the user stays raging until the turn it chooses
// something other than Rage (Gen 2-3, as that turn starts — a switch too) or until its own move
// other than Rage is over (Gen 4). Gen 2: a hit by a damaging move that did not miss, its
// Substitute's too, builds a count (up to 255) and no Attack, and Rage deals its damage × (count
// + 1) after the type matchups and before the random factor (`cartridgeGen2GetDamage`). Gen 3-4:
// a hit that took the standing holder's HP — not its Substitute's — raises its Attack below +6,
// after the move's secondaries and before the reactions (the head of `DamagingHit`); Gen 3 only
// a foe's. Gen 3: the flag is the accuracy check's — a Rage that misses or meets Protect ends it,
// one that passes the check starts it even as a type immunity or Wonder Guard stops it, but not
// against a Substitute. Baton Pass leaves it (pokeplatinum's `VOLATILE_CONDITION_BATON_PASSED`).
// The patch gives Gen 2, 3 and 4 a `rage` of their own; Gen 2's count is the volatile's
// `counter`, its build shown as `-activate|holder|move: Rage` (the cartridge's "rage is
// building"). Draws: the multiplied damage of 1 above (I75's too); otherwise none.
const rageChosen = (battle, pokemon) => battle.queue.willMove(pokemon)?.move?.id === 'rage';
const patchRage = (mod, extend) => {
  const data = Dex.mod(mod).data;
  const base = data.Moves.rage;
  const condition = {...base.condition, noCopy: true};
  delete condition.onBeforeMove;
  delete condition.onBeforeMovePriority;
  delete condition.onHit;
  extend(condition);
  data.Moves.rage = {...base, condition};
};
const raiseRage = (battle, target, source) => {
  if (target !== source && target.hp && target.boosts.atk < 6) battle.boost({atk: 1}, target, source);
};
if (patched) {
  patchRage('gen2', condition => {
    condition.onBeforeTurnPriority = 100;
    condition.onBeforeTurn = function (pokemon) {
      if (!rageChosen(this, pokemon)) delete pokemon.volatiles['rage'];
    };
    const count = (battle, target, source, move) => {
      const state = target.volatiles['rage'];
      if (target === source || move.category === 'Status' || !state || (state.counter || 0) >= 255) return;
      state.counter = (state.counter || 0) + 1;
      battle.add('-activate', target, 'move: Rage');
    };
    condition.onHit = function (target, source, move) {
      // Present's heal ends its move first (pokecrystal `.heal_effect`: `EndMoveEffect`).
      if (!move.heal) count(this, target, source, move);
    };
    condition.onAfterSubDamage = function (damage, target, source, move) {
      count(this, target, source, move);
    };
  });
  patchRage('gen3', condition => {
    condition.onBeforeTurnPriority = 100;
    condition.onBeforeTurn = function (pokemon) {
      if (!rageChosen(this, pokemon)) delete pokemon.volatiles['rage'];
    };
    condition.onDamagingHitOrder = -1;
    condition.onDamagingHit = function (damage, target, source) {
      if (!target.isAlly(source)) raiseRage(this, target, source);
    };
  });
  patchRage('gen4', condition => {
    condition.onBeforeTurnPriority = 100;
    condition.onBeforeTurn = function (pokemon) {
      // I78 (below).
      if (!rageChosen(this, pokemon)) {
        for (const id of GEN4_RAGE_WIPED) {
          // Mean Look's hold is a linked volatile: `removeVolatile` (silent, no `onEnd`) unlinks
          // its trapper, which a raw delete leaves holding a stale link.
          if (id === 'trapped') pokemon.removeVolatile(id);
          else delete pokemon.volatiles[id];
        }
      }
    };
    condition.onDamagingHitOrder = -1;
    condition.onDamagingHit = function (damage, target, source) {
      raiseRage(this, target, source);
    };
    condition.onAfterMovePriority = 100;
    condition.onAfterMove = function (pokemon, target, move) {
      if (move.id !== 'rage') delete pokemon.volatiles['rage'];
    };
  });
  // Gen 3: the accuracy check's flag. A target out of reach (`Invulnerability`) is a miss; past
  // it, the check passes when the `Accuracy` event says `true` (or does not run: `alwaysHit`)
  // or the first `randomChance` after it comes out true.
  const actions = Dex.mod('gen3').data.Scripts.actions;
  const tryMoveHit = actions.tryMoveHit;
  actions.tryMoveHit = function (target, pokemon, move) {
    if (move.id !== 'rage' || this.battle.gen !== 3) return tryMoveHit.call(this, target, pokemon, move);
    const battle = this.battle;
    const {runEvent, randomChance} = battle;
    const behindSubstitute = !!target.volatiles['substitute'];
    if (behindSubstitute) move.self = null;
    let reached = false;
    let accuracy;
    let roll;
    battle.runEvent = function (eventid, ...rest) {
      const value = runEvent.call(this, eventid, ...rest);
      if (eventid === 'Invulnerability') reached = value !== false;
      if (eventid === 'Accuracy' && accuracy === undefined) accuracy = value;
      return value;
    };
    battle.randomChance = function (...args) {
      const result = randomChance.apply(this, args);
      if (accuracy !== undefined && accuracy !== true && roll === undefined) roll = result;
      return result;
    };
    let result;
    try {
      result = tryMoveHit.call(this, target, pokemon, move);
    } finally {
      battle.runEvent = runEvent;
      battle.randomChance = randomChance;
    }
    if (result === false) {
      const passed = reached && (accuracy === undefined || accuracy === true || roll === true);
      if (!passed || target.volatiles['protect']) {
        delete pokemon.volatiles['rage'];
      } else if (!behindSubstitute) {
        pokemon.addVolatile('rage');
      }
    }
    return result;
  };
}

// I78 — the Gen 4 Rage bug, as pokeplatinum's source marks it
// (`BattleControllerPlayer_CheckPreMoveActions`: `statusVolatile &= VOLATILE_CONDITION_RAGE`,
// "bug: clears every volatile status except rage"): as the turn starts, a raging Pokémon that did
// not choose Rage loses, silently, every other volatile status bit — of what Showdown models and
// a raging Pokémon can hold then: confusion, a binding move's hold, love, Focus Energy, its
// Substitute, Mean Look's hold, Nightmare, a Ghost's Curse, Defense Curl, Torment — and keeps
// raging. Showdown has no such thing. The patch is the gen4 `rage`'s `onBeforeTurn` (I77 above);
// Mean Look's hold goes through `removeVolatile`, so its trapper (a Showdown link with no cartridge
// counterpart) is no trapper once it holds nobody else (R266). Draws change: a confusion, a binding move, love that is gone draws nothing (TESTING.md §7).
const GEN4_RAGE_WIPED = ['confusion', 'partiallytrapped', 'attract', 'focusenergy', 'substitute', 'trapped',
  'nightmare', 'curse', 'defensecurl', 'torment'];

// I79 — Gen 1 Rage's build as pokered has it (`HandleBuildingRage`). It follows the move's hit,
// `AttackSubstitute`'s too, so a hit on the raging Pokémon's Substitute builds; and the build is the
// holder's own Attack-up effect run with the turn flipped (`StatModifierUpEffect`), whose end cuts
// the paralysed Speed / burned Attack of the Pokémon whose turn it is not — the attacker — again
// (`QuarterSpeedDueToParalysis`, `HalveAttackDueToBurn`), unless the stat was already 999. Showdown's
// gen1 Substitute runs no `Hit` event, and its `boost` cuts nobody's stats (only the gen1 `moveHit`
// does, for stat moves). The patch: the gen1 `rage` builds in `onAfterSubDamage` as in `onHit`; the
// gen1 `boost` with the rage condition as its effect, once the Attack moved, cuts the holder's foe's
// stats as the gen1 `moveHit` does. No draw changes.
if (patched) {
  const condition = Dex.mod('gen1').data.Moves.rage.condition;
  condition.onAfterSubDamage = function (damage, target, source, move) {
    if (target.boosts.atk < 6 && move.category !== 'Status' && !move.selfdestruct) this.boost({atk: 1});
  };
  const scripts = Dex.mod('gen1').data.Scripts;
  const boost = scripts.boost;
  scripts.boost = function (changes, target, source, effect) {
    const holder = target || this.event?.target;
    const cause = effect || (this.event ? this.effect : null);
    const result = boost.call(this, changes, target, source, effect);
    if (result && cause?.id === 'rage' && holder) {
      const foe = holder.side.foe.active[0];
      if (foe?.status === 'par') foe.modifyStat('spe', 0.25);
      if (foe?.status === 'brn') foe.modifyStat('atk', 0.5);
    }
    return result;
  };
}

// I105 / I106 / I107 — Truant on the cartridges (DESIGN.md §15.21).
// I105 (Gen 3-4): the check comes between freeze and the recharge (pokeemerald `CANCELER_TRUANT`,
// pokeplatinum `CHECK_STATUS_STATE_TRUANT`). Showdown's 9 falls below I66's `mustrecharge` 9.5; the
// patch gives the gen3 / gen4 Truant 9.7. A loafing turn then passes the recharge turn with it: the
// recharge is not spent, its own time runs out at the turn's end (pokeemerald `rechargeTimer`,
// pokeplatinum `SetupNextTurn`; Showdown `mustrecharge`'s `duration: 2`). The order alone changes no
// draw: with Gen 3's turn-end flag and Gen 4's parity (I106) the same turns are lost either way, only
// the `cant` reason and when the recharge goes differ.
// I106 (Gen 4): pokeplatinum keeps a parity, not Showdown's volatile flipped by each move attempt
// that reaches the check (`moveEffectsData.truant`; `Battler_CheckTruant`: Truant and `truant !=
// (totalTurns & 1)`). Every Pokémon coming in takes `(totalTurns + 1) & 1` (the battle's leads 0),
// Role Play's user and both Skill Swap Pokémon `totalTurns & 1`; Showdown's turn is `totalTurns + 1`.
// The patch: the gen4 Truant loafs when `truantParity` matches the turn's parity, with no `onStart`
// (the I82 patch already keeps it from starting on an ability change); the parity is set as a
// Pokémon switches in (`switchIn`) and as Role Play / Skill Swap succeed; Pursuit's mark asks it
// (`beforeTurnCallback`, which asked for the volatile). Draws change: sleep, freeze and the recharge
// no longer hold the count back (TESTING.md §7).
// I107 (Gen 3): pokeemerald clears the `DisableStruct` (`truantCounter` with it) on every switch-in
// and sets the counter for a Truant holder only; Showdown's gen3 `truantTurn` lives on the Pokémon
// and keeps its value across switches, which a Pokémon that gains Truant (Trace, Role Play, Skill
// Swap) in two stints reads. The patch clears it as any Pokémon switches in. Draws change with the
// turns it loafs on (TESTING.md §7).
if (patched) {
  const truant3 = {...Dex.mod('gen3').data.Abilities.truant, onBeforeMovePriority: 9.7};
  Dex.mod('gen3').data.Abilities.truant = truant3;
  const truant4 = {
    ...Dex.mod('gen4').data.Abilities.truant,
    onStart: undefined,
    onBeforeMovePriority: 9.7,
    onBeforeMove(pokemon) {
      if ((pokemon.truantParity || 0) === (this.turn & 1)) {
        this.add('cant', pokemon, 'ability: Truant');
        return false;
      }
    },
  };
  Dex.mod('gen4').data.Abilities.truant = truant4;
  // The cartridges' Truant ends what the other stopping checks end (I61, I63, I75): Gen 3 cancels the
  // lock and the uproar at once and the Fury Cutter count; Gen 4 the count alone.
  failRampageWhenStopped(truant3, 'onBeforeMove', [3]);
  whenUproarStopped(truant3, 'onBeforeMove', cancelUproar);
  endFuryCutterWhenStopped(truant3, 'onBeforeMove', [3]);
  endFuryCutterWhenStopped(truant4, 'onBeforeMove', [4]);
  const truantSwitchIn = previous => function (pokemon, ...rest) {
    const result = previous ? previous.call(this, pokemon, ...rest)
      : Object.getPrototypeOf(this).switchIn.call(this, pokemon, ...rest);
    if (result) {
      if (this.battle.gen === 3) pokemon.truantTurn = false;
      if (this.battle.gen === 4) pokemon.truantParity = this.battle.turn & 1;
    }
    return result;
  };
  for (const [id, format] of Object.entries(Dex.data.Rulesets)) {
    if (!/^gen[34]/.test(id)) continue;
    format.actions = {...format.actions, switchIn: truantSwitchIn(format.actions?.switchIn)};
    const cached = Dex.formats.get(id);
    cached.actions = {...cached.actions, switchIn: truantSwitchIn(cached.actions?.switchIn)};
  }
  const moves4 = Dex.mod('gen4').data.Moves;
  for (const [id, holders] of [['roleplay', (target, source) => [source]],
    ['skillswap', (target, source) => [source, target]]]) {
    const onHit = moves4[id].onHit;
    moves4[id] = {...moves4[id], onHit(target, source, ...rest) {
      const result = onHit.call(this, target, source, ...rest);
      if (result !== false && result !== null) {
        for (const pokemon of holders(target, source)) pokemon.truantParity = (this.turn + 1) & 1;
      }
      return result;
    }};
  }
  const pursuit4 = moves4.pursuit;
  const marking = pursuit4.beforeTurnCallback;
  moves4.pursuit = {...pursuit4, beforeTurnCallback(pokemon, ...rest) {
    if (pokemon.hasAbility('truant') && (pokemon.truantParity || 0) === (this.turn & 1)) return;
    return marking.call(this, pokemon, ...rest);
  }};
}

// I87 — Gen 2 Foresight and the accuracy stages (pokecrystal `CheckHit`'s `.StatModifiers`):
// when the identified target's evasion stage is at least the user's accuracy stage, neither stage
// applies. Showdown's gen2 accuracy reads the stages as they are. The patch: the gen2 `foresight`
// condition's `onAccuracy` (the target's, before the stages) marks the move to ignore both. A move
// of 100 % then stays at 255 and draws nothing where Showdown drew — an exception to the RNG rule.
// I89 — Gen 3-4 Foresight, Odor Sleuth and Miracle Eye on the cartridge (pokeemerald
// `Cmd_setforesight`, pokeplatinum `subscript_foresight` / `subscript_miracle_eye`) set their flag
// whatever the target has: they succeed on a target already identified by either. Showdown fails
// them there (`addVolatile`, the moves' `onTryHit`). The patch gives the gen3 / gen4 conditions an
// `onRestart` that announces them again and takes the gen4 moves' `onTryHit` away. No draw
// depends on it.
if (patched) {
  const gen2 = Dex.mod('gen2').data.Moves;
  gen2.foresight = {
    ...gen2.foresight,
    condition: {
      ...gen2.foresight.condition,
      onAccuracy(accuracy, target, source, move) {
        if (typeof accuracy === 'number' && target.boosts.evasion >= source.boosts.accuracy) {
          move.ignoreAccuracy = true;
          move.ignoreEvasion = true;
        }
      },
    },
  };
  for (const mod of ['gen3', 'gen4']) {
    const moves = Dex.mod(mod).data.Moves;
    for (const [id, name] of [['foresight', 'Foresight'], ['miracleeye', 'Miracle Eye']]) {
      if (!moves[id]?.condition) continue;
      moves[id] = {
        ...moves[id],
        condition: {
          ...moves[id].condition,
          onRestart(pokemon) {
            this.add('-start', pokemon, name);
          },
        },
      };
    }
    if (mod === 'gen4') {
      for (const id of ['foresight', 'odorsleuth', 'miracleeye']) {
        moves[id] = {...moves[id]};
        delete moves[id].onTryHit;
      }
    }
  }
}

// I108 / I109 — Gen 2 Thrash, Petal Dance and Outrage as pokecrystal counts them
// (`BattleCommand_CheckRampage`, `BattleCommand_Rampage`, `CantMove`; DESIGN.md §15.12).
// I108: the script opens with the count, ahead of the hit test: the first use sets 1-2 more uses
// (`BattleRandom & 1` + 1), hit or miss; each later use counts one down, and the one that reaches
// 0 ends the lock and confuses its user — for 2-3 turns (`BattleRandom & 1` + 2), a confusion there
// rolled anew, nothing under Safeguard (its confusion left alone) — before it strikes; the lock
// goes once that move is over. Showdown's gen2 `lockedmove` draws 2-3 turns as the move lands
// (`self`, or `onMoveFail` on a miss), confuses in `onAfterMove` once its `duration` reads 1, for
// 2-5 turns, and deletes a confusion there even under Safeguard. The patch: a gen2 `lockedmove`
// of its own (`time`: the uses left, no `duration`), the moves without `self` / `onMoveFail`, the
// count and the last use's release in the gen2 `tryMoveHit`, and the gen2 confusion's `onStart`
// drawing 2-3 for the fatigue. (Showdown's gen2 `onAfterMove` never runs: the gen2 `runMove` sends
// `AfterMove` to the move alone; its lock ended with its `duration`, confusing at the turn's end.)
// I109: every stop ends the lock silently at the user's move (`CantMove`): asleep, the lock waits
// for that move instead of ending at the turn's end (the gen2 `lockedmove`'s `onResidual`). (Waking
// up ends it too, but a Pokémon put to sleep stays asleep at its next move — Gen 2's sleep lasts at
// least a turn — and the lock is gone by then.)
// Draws change: the count and the fatigue are drawn before the hit test instead of after the
// move, the fatigue's turns are 2-3, and a sleeping user stays locked (TESTING.md §7).
if (patched) {
  const data = Dex.mod('gen2').data;
  const lockedmove = data.Conditions.lockedmove;
  data.Conditions.lockedmove = {
    name: 'lockedmove',
    onStart(target, source, effect) {
      this.effectState.move = effect.id;
      this.effectState.time = this.random(2, 4) - 1;
    },
    onLockMove() {
      return this.effectState.move;
    },
    onMoveAborted(pokemon) {
      delete pokemon.volatiles['lockedmove'];
    },
    onBeforeTurn: lockedmove.onBeforeTurn,
  };
  for (const id of RAMPAGE_MOVES) {
    data.Moves[id] = {...data.Moves[id], self: undefined, onMoveFail: undefined};
  }
  const tryMoveHit = data.Scripts.actions.tryMoveHit;
  data.Scripts.actions.tryMoveHit = function (target, pokemon, move) {
    if (this.battle.gen !== 2 || !RAMPAGE_MOVES.includes(move.id)) {
      return tryMoveHit.call(this, target, pokemon, move);
    }
    const lock = pokemon.volatiles['lockedmove'];
    if (!lock) {
      pokemon.addVolatile('lockedmove', pokemon, move);
    } else if (--lock.time <= 0 && !pokemon.side.getSideCondition('safeguard')) {
      delete pokemon.volatiles['confusion'];
      pokemon.addVolatile('confusion', pokemon, this.dex.conditions.get('lockedmove'));
    }
    const result = tryMoveHit.call(this, target, pokemon, move);
    // The gen2 `runMove` runs no `AfterMove` event for volatiles: the last use lets go here.
    if (pokemon.volatiles['lockedmove']?.time <= 0) delete pokemon.volatiles['lockedmove'];
    return result;
  };
  const confusion = data.Conditions.confusion;
  const confusionStart = confusion.onStart;
  data.Conditions.confusion = {...confusion, onStart(target, source, effect) {
    if (this.gen === 2 && effect?.id === 'lockedmove') {
      this.add('-start', target, 'confusion', '[silent]');
      this.effectState.time = this.random(2, 4);
      return;
    }
    return confusionStart.call(this, target, source, effect);
  }};
}

// I95 — Gen 4 Magnet Rise on a Levitate holder (pokeplatinum effect script 0252: `CheckAbility
// CHECK_HAVE … ABILITY_LEVITATE`, through `Battler_Ability`, which a Gastro Acid sets aside).
// Showdown's gen4 `magnetrise` condition reads `target.ability === 'levitate'` itself, so Magnet
// Rise fails on a holder whose Levitate Gastro Acid suppressed. The patch asks `hasAbility`
// instead. No draw depends on it.
if (patched) {
  const moves = Dex.mod('gen4').data.Moves;
  const condition = moves.magnetrise.condition;
  moves.magnetrise = {
    ...moves.magnetrise,
    condition: {
      ...condition,
      onStart(target) {
        if (target.volatiles['ingrain'] || target.hasAbility('levitate')) return false;
        this.add('-start', target, 'Magnet Rise');
      },
    },
  };
}

// I110 — Gen 2-3 Future Sight / Doom Desire: the random factor of the damage computed at use goes
// on at the strike. pokecrystal's Future Sight script computes the damage at use (`damagestats`,
// `damagecalc`) and stores it at `futuresight`; the strike skips to the command after it,
// `damagevariation`, before `checkhit`. pokeemerald's `Cmd_trysetfutureattack` stores
// `CalculateBaseDamage` (no random factor); the strike's script runs `accuracycheck`, then
// `adjustnormaldamage2` (`ApplyRandomDmgMultiplier`: × (100 − random % 16) / 100, at least 1).
// Showdown's gen4 `onTry` (which gen2 and gen3 inherit; gen3's own Doom Desire alike) stores
// `getDamage` with its random factor; pokeplatinum's `BtlCmd_TryFutureSight` does that too (Gen 4
// is unchanged). The patch: the gen2 / gen3 moves store the damage without the factor
// (`noDamageVariance`; Gen 3's `modifyDamage` through an identity `randomizer`), and the strike
// draws it — Gen 2 as the condition ends, before the strike's hit steps (a damage of 1 draws
// nothing), Gen 3 once the strike hit, through a `damageCallback`. Draws change (TESTING.md §7).
const foreseenAtUse = (id, name, accuracy, basePower, category) => function (source, target) {
  if (!target.side.addSlotCondition(target, 'futuremove')) return false;
  const flags = {metronome: 1, futuremove: 1};
  const {randomizer} = this;
  this.randomizer = damage => damage;
  let damage;
  try {
    damage = this.actions.getDamage(source, target,
      {name, basePower, category, flags, willCrit: false, type: '???', noDamageVariance: true}, true);
  } finally {
    this.randomizer = randomizer;
  }
  Object.assign(target.side.slotConditions[target.position]['futuremove'], {
    duration: 3,
    move: id,
    source,
    moveData: {id, name, accuracy, basePower: 0, damage, category, flags, effectType: 'Move', type: '???'},
  });
  this.add('-start', source, name);
  return null;
};
for (const mod of patched ? ['gen2', 'gen3'] : []) {
  const data = Dex.mod(mod).data;
  data.Moves.futuresight = {...data.Moves.futuresight,
    onTry: foreseenAtUse('futuresight', 'Future Sight', 90, 80, 'Special')};
  if (mod === 'gen3') {
    data.Moves.doomdesire = {...data.Moves.doomdesire,
      onTry: foreseenAtUse('doomdesire', 'Doom Desire', 85, 120, 'Physical')};
  }
  const condition = data.Conditions.futuremove;
  const onEnd = condition.onEnd;
  data.Conditions.futuremove = {...condition, onEnd(target) {
    const state = this.effectState;
    const base = state.moveData?.damage;
    if (!target.fainted && target !== state.source && typeof base === 'number') {
      if (this.gen === 2) {
        if (base > 1) state.moveData = {...state.moveData, damage: Math.floor(base * this.random(217, 256) / 255)};
      } else {
        state.moveData = {...state.moveData, damageCallback() {
          return Math.floor(base * (100 - this.random(16)) / 100) || 1;
        }};
      }
    }
    return onEnd.call(this, target);
  }};
}

// I111 — Gen 3 Future Sight / Doom Desire strikes go by battler. pokeemerald's
// `HandleWishPerishSongOnTurnEnd` walks the battlers by number (the player's left 0, the foe's
// left 1, the player's right 2, the foe's right 3) and strikes for each whose count runs out;
// Showdown sorts the `futuremove` handlers by their holder's speed (`speedSort`), and pokeplatinum
// by `monSpeedOrder` (Gen 4 is unchanged). The patch gives the gen3 handlers a speed by battler
// number, distinct, so nothing ties: holders of one speed no longer draw their order (TESTING.md
// §7).
if (patched) {
  const resolvePriority = Battle.prototype.resolvePriority;
  Battle.prototype.resolvePriority = function (handler, callbackName) {
    const resolved = resolvePriority.call(this, handler, callbackName);
    if (this.gen === 3 && callbackName === 'onResidual' && resolved.effect?.id === 'futuremove'
      && resolved.effectHolder?.side) {
      const holder = resolved.effectHolder;
      resolved.speed = 4 - (holder.position * 2 + holder.side.n);
    }
    return resolved;
  };
}

// I112 — Gen 3-4 Transform ends Disable; Gen 4's forgets the announced switch-in effects.
// pokeemerald `Cmd_transformdataexecution` and pokeplatinum `BtlCmd_Transform` clear the user's
// `disabledMove` and its timer (DESIGN.md §41.7, A9 of §41.10); pokeplatinum also clears every
// `…Announced` flag (the weather abilities', Intimidate's, Trace's, Download's, Anticipation's,
// Forewarn's …), so the switch-in check at the move's end starts a copied Intimidate even after
// the user's own. Showdown's `transformInto` leaves the `disable` volatile (a disabled move the
// target also knows stays disabled), and the R205 patch's `announced` set stays. The patch empties
// the set as a Gen 4 Transform starts (back as it was if Transform fails) and drops the volatile,
// without a message, once a Gen 3-4 Transform succeeded. No draw changes.
if (patched) {
  for (const mod of ['gen3', 'gen4']) {
    const moves = Dex.mod(mod).data.Moves;
    const onHit = moves.transform.onHit;
    moves.transform = {...moves.transform, onHit(target, pokemon, ...rest) {
      const announced = pokemon.m.announced;
      if (this.gen === 4) pokemon.m.announced = new Set();
      const result = onHit.call(this, target, pokemon, ...rest);
      if (result) {
        delete pokemon.volatiles['disable'];
      } else {
        pokemon.m.announced = announced;
      }
      return result;
    }};
  }
}

// I113 — Gen 3 Soul Dew and Metal Powder ask the battle species. pokeemerald `CalculateBaseDamage`
// checks `attacker->species` / `defender->species` of the battle structure, which Transform copies
// (`Cmd_transformdataexecution`): a Pokémon transformed into Latias or Latios gets its Soul Dew's
// boost, and Metal Powder asks `SPECIES_DITTO` alone (a Ditto transformed into a Ditto keeps it,
// any other transformed Ditto does not). Showdown's gen3 Soul Dew asks `baseSpecies`, its Metal
// Powder `species.name === 'Ditto' && !transformed`. No draw changes.
if (patched) {
  const items3 = Dex.mod('gen3').data.Items;
  const lati = pokemon => pokemon.species.num === 380 || pokemon.species.num === 381;
  items3.souldew = {...items3.souldew,
    onModifySpA(spa, pokemon) { if (lati(pokemon)) return this.chainModify(1.5); },
    onModifySpD(spd, pokemon) { if (lati(pokemon)) return this.chainModify(1.5); },
  };
  items3.metalpowder = {...items3.metalpowder,
    onModifyDef(def, pokemon) { if (pokemon.species.name === 'Ditto') return this.chainModify(2); },
  };
}

// I114 — Gen 3 Bide's storing turns leave no last move. pokeemerald handles the turns after the
// first in `AtkCanceler_UnableToUseMove` (`CANCELER_BIDE`), so `HITMARKER_OBEYS` is never set and
// `MOVEEND_UPDATE_LAST_MOVES` writes `MOVE_UNAVAILABLE` — what I67 does for the release, which
// Showdown aborts (`MoveAborted`). Showdown stops a storing turn in the move's
// `beforeMoveCallback`, which keeps `lastMove`. The patch clears it there. No draw changes.
if (patched) {
  const bide = Dex.mod('gen3').data.Moves.bide;
  const beforeMoveCallback = bide.beforeMoveCallback;
  bide.beforeMoveCallback = function (pokemon, ...rest) {
    const stopped = beforeMoveCallback.call(this, pokemon, ...rest);
    if (stopped) pokemon.lastMove = null;
    return stopped;
  };
}

// I166 — Gen 3 Nature Power never misses. pokeemerald's `BattleScript_EffectNaturePower` is
// `attackcanceler`, `attackstring`, `callenvironmentattack`: no `accuracycheck`, so the 95 in the
// move table is never read. Showdown's gen3 mod gives the move `accuracy: 95`, and its gen3
// `tryMoveHit` has no exemption for a move aimed at its user, so Nature Power rolls (and, with the
// user's own accuracy and evasion stages, can miss itself). The patch makes it `accuracy: true`.
// The draw changes: Showdown no longer rolls the accuracy (TESTING.md §7).
if (patched) {
  Dex.mod('gen3').data.Moves.naturepower.accuracy = true;
}

// I167 — Gen 3: the Pressure a called move meets comes off the action's PP. pokeemerald's
// Metronome, Assist, Mirror Move and Nature Power scripts have no `ppreduce` of their own: they
// jump into the called move's script, whose `ppreduce` (`Cmd_ppreduce`) takes 1 plus one for each
// Pressure the called move meets from the action's slot (`gCurrMovePos`) — Metronome's when it
// called Nature Power or Mirror Move. Sleep Talk pays its own first and turns the called move's off
// (`HITMARKER_NO_PPDEDUCT`). Showdown's gen3 `useMoveInner` counts Pressure only for a move with
// no `sourceEffect`. The patch counts it, as for a move of the action's own, for a move a move
// called (`callsMove`) other than Sleep Talk, and takes it from the action's move
// (`moveThisTurn`). No draw changes.
if (patched) {
  const actions = Dex.mod('gen3').data.Scripts.actions;
  const useMoveInner = actions.useMoveInner;
  actions.useMoveInner = function (moveOrMoveName, pokemon, options) {
    let caller = options?.sourceEffect;
    if (!caller && this.battle.effect.id) caller = this.battle.effect;
    if (!caller || caller.effectType !== 'Move' || !caller.callsMove || caller.id === 'sleeptalk') {
      return useMoveInner.call(this, moveOrMoveName, pokemon, options);
    }
    // The gen3 `useMoveInner` asks for the targets once, right where it would count Pressure.
    const getMoveTargets = pokemon.getMoveTargets;
    pokemon.getMoveTargets = function (move, target) {
      delete pokemon.getMoveTargets;
      const result = getMoveTargets.call(this, move, target);
      let extraPP = 0;
      for (const source of result.pressureTargets) {
        const ppDrop = this.battle.runEvent('DeductPP', source, pokemon, move);
        if (ppDrop !== true) extraPP += ppDrop || 0;
      }
      if (extraPP > 0) pokemon.deductPP(pokemon.moveThisTurn, extraPP);
      return result;
    };
    try {
      return useMoveInner.call(this, moveOrMoveName, pokemon, options);
    } finally {
      delete pokemon.getMoveTargets;
    }
  };
}

// I168 — Gen 3-4 Mimic and Sketch: the move they copy is chosen with a target like any other.
// pokeemerald (`Cmd_mimicattackcopy`, `Cmd_copymovepermanently`) and pokeplatinum write the move
// into the slot; choosing it reads the move's own target (`gBattleMoves[move].target`). The gen4
// `mimic` / `sketch` (Gen 3 inherits them) build the new slot without `target`, so in a double
// battle the request offers the copied move with none: a target given is a choice error, and the
// move goes to a random adjacent foe (`getTarget`'s `getRandomTarget`). The patch gives the slot
// its move's target. The draw changes: a copied move chosen with its target draws none
// (TESTING.md §7).
if (patched) {
  for (const id of ['mimic', 'sketch']) {
    for (const move of new Set([Dex.mod('gen3').data.Moves[id], Dex.mod('gen4').data.Moves[id]])) {
      const onHit = move.onHit;
      move.onHit = function (target, source, ...rest) {
        const result = onHit.call(this, target, source, ...rest);
        for (const slot of [...source.moveSlots, ...source.baseMoveSlots]) {
          if (slot.target === undefined) slot.target = this.dex.moves.get(slot.id).target;
        }
        return result;
      };
    }
  }
}

// I169 — Gen 3-4: a Pokémon that takes another's move over pays one PP for its Pressure.
// pokeemerald's `Cmd_attackcanceler` runs `PressurePPLose(attacker, taker, MOVE_MAGIC_COAT /
// MOVE_SNATCH)` as Magic Coat bounces a move or Snatch takes one: the slot holding the taking move
// loses a PP when the move's user has Pressure, whatever their sides (none when the taker does not
// know the move — a caller used it). pokeplatinum's `BattleSystem_DecPPForPressure` takes it from
// the slot the taker chose this turn (`moveSlot`), a caller's when one called the taking move.
// Showdown's gen4 Snatch (Gen 3 inherits it) charges the Snatch slot through `DeductPP`; its
// Magic Coat charges nothing. The patch charges the coat's reflection (Gen 3: the Magic Coat slot;
// Gen 4: `lastMove`, the move the action used) and moves Gen 4 Snatch's charge to `lastMove`.
// No draw changes.
if (patched) {
  const conditions = mod => [Dex.mod(mod).data.Moves.magiccoat.condition, Dex.mod(mod).data.Moves.snatch.condition];
  const [coat3, snatch3] = conditions('gen3');
  const [coat4, snatch4] = conditions('gen4');
  for (const coat of new Set([coat3, coat4])) {
    const onTryHit = coat.onTryHit;
    coat.onTryHit = function (target, source, move) {
      const reflects = target !== source && !move.hasBounced && move.flags['reflectable'];
      if (reflects && this.gen <= 4 && source.hasAbility('pressure')) {
        target.deductPP(this.gen === 3 ? 'magiccoat' : target.lastMove?.id ?? 'magiccoat', 1);
      }
      return onTryHit.call(this, target, source, move);
    };
  }
  for (const snatch of new Set([snatch3, snatch4])) {
    const onAnyPrepareHit = snatch.onAnyPrepareHit;
    snatch.onAnyPrepareHit = function (source, target, move) {
      const snatchUser = this.effectState.source;
      if (this.gen !== 4 || !snatchUser.lastMove) return onAnyPrepareHit.call(this, source, target, move);
      const chosen = snatchUser.lastMove.id;
      const deductPP = snatchUser.deductPP;
      snatchUser.deductPP = function (moveid, amount, ...rest) {
        delete snatchUser.deductPP;
        return deductPP.call(this, chosen, amount, ...rest);
      };
      try {
        return onAnyPrepareHit.call(this, source, target, move);
      } finally {
        delete snatchUser.deductPP;
      }
    };
  }
}

// I170 — A Gen 3 Pokémon with the same move in two slots (Sketch copying a Mimic that Mimic's own
// copy hid): the action's PP comes off the slot it chose, and Mimic and Sketch write that slot
// (pokeemerald's `gCurrMovePos` in `Cmd_ppreduce`, `Cmd_mimicattackcopy`, `Cmd_copymovepermanently`;
// issues.md I170). Showdown's gen3 `deductPP` takes the first slot with the move's id (`getMoveData`),
// and Mimic / Sketch write `source.moves.indexOf(...)` — what pokeplatinum does (`Battler_SlotForMove`,
// `BtlCmd_TryMimic`), so Gen 4+ keep it. The patch keeps the chosen slot (`action.moveSlot`) on the
// Pokémon for its move action; `getMoveData` and Mimic / Sketch prefer it when it holds the move.
// (Gen 2 never has a move twice: its Sketch fails, its Mimic refuses a known move.) No draw changes.
if (patched) {
  const runAction = Battle.prototype.runAction;
  Battle.prototype.runAction = function (action) {
    const pokemon = action.pokemon;
    if (this.gen !== 3 || action.choice !== 'move' || action.moveSlot === undefined || !pokemon) {
      return runAction.call(this, action);
    }
    pokemon.chosenMoveSlot = action.moveSlot;
    try {
      return runAction.call(this, action);
    } finally {
      delete pokemon.chosenMoveSlot;
    }
  };
  const getMoveData = Pokemon.prototype.getMoveData;
  Pokemon.prototype.getMoveData = function (move) {
    const slot = this.chosenMoveSlot === undefined ? undefined : this.moveSlots[this.chosenMoveSlot];
    if (slot && slot.id === this.battle.dex.moves.get(move).id) return slot;
    return getMoveData.call(this, move);
  };
  // `source.moves.indexOf(id)` finds the chosen slot: the other slots holding the move read as empty.
  const onTheChosenSlot = (move, id) => {
    const onHit = move.onHit;
    move.onHit = function (target, source, ...rest) {
      const chosen = source?.chosenMoveSlot;
      if (chosen === undefined || source.moveSlots[chosen]?.id !== id) return onHit.call(this, target, source, ...rest);
      const ids = source.moveSlots.map((slot, i) => (slot.id === id && i !== chosen ? '' : slot.id));
      Object.defineProperty(source, 'moves', {get: () => ids, configurable: true});
      try {
        return onHit.call(this, target, source, ...rest);
      } finally {
        delete source.moves;
      }
    };
  };
  for (const id of ['mimic', 'sketch']) {
    const move = Dex.mod('gen3').data.Moves[id];
    if (move?.onHit) onTheChosenSlot(move, id);
  }
}

// I171 — Color Change does not answer its holder's own move. Showdown runs Bide's first turn as a
// hit on its user (`target: 'self'`), so from Gen 5 the holder's Color Change takes Bide's Normal type
// (`onAfterMoveSecondary`); Gen 3-4's handler does not. Bide's storing turn strikes nobody; the user
// decided to treat this as a defect (issues.md I171). The patch skips the holder's own move. No draw
// changes.
if (patched) {
  const abilities = new Set(['gen5', 'gen6', 'gen7', 'gen8', 'gen9']
    .map(mod => Dex.mod(mod).data.Abilities.colorchange).filter(Boolean));
  for (const colorchange of abilities) {
    const onAfterMoveSecondary = colorchange.onAfterMoveSecondary;
    colorchange.onAfterMoveSecondary = function (target, source, move) {
      if (target === source) return;
      return onAfterMoveSecondary.call(this, target, source, move);
    };
  }
}

// I172 — A Pledge does not combine with its own user's. Each Pledge's `onPrepareHit` looks through the
// queue for a Pokémon on the user's side (`action.pokemon.isAlly(source)`) about to use another Pledge;
// `isAlly` is true of the user itself, so a Pledge Instruct makes its user use again waits for the
// other Pledge the user still has queued, even in singles. The user decided to treat this as a defect
// (issues.md I172): while the Pledge looks for its partner, the user is not its own ally. No draw
// changes of its own (the battle goes on differently from there).
if (patched) {
  for (const mod of ['gen5', 'gen6', 'gen7', 'gen8', 'gen9']) {
    for (const id of ['firepledge', 'grasspledge', 'waterpledge']) {
      const pledge = Dex.mod(mod).data.Moves[id];
      const onPrepareHit = pledge?.onPrepareHit;
      if (!onPrepareHit || onPrepareHit.ownPledgeApart) continue;
      const apart = function (target, source, move) {
        const own = Object.prototype.hasOwnProperty.call(source, 'isAlly');
        const isAlly = source.isAlly;
        source.isAlly = function (pokemon) { return pokemon !== this && isAlly.call(this, pokemon); };
        try {
          return onPrepareHit.call(this, target, source, move);
        } finally {
          if (own) source.isAlly = isAlly; else delete source.isAlly;
        }
      };
      apart.ownPledgeApart = true;
      pledge.onPrepareHit = apart;
    }
  }
}

// I176 — Gen 2 Conversion 2 reads the type the move table gives the target's last move.
// pokecrystal's `BattleCommand_Conversion2` looks up `Moves + MOVE_TYPE` for
// `BATTLE_VARS_LAST_COUNTER_MOVE_OPP`: Beat Up is Dark there, Hidden Power Normal. Showdown's gen2
// `conversion2` reads `target.lastMove.type`, the ActiveMove its `onModifyMove` / `onModifyType`
// changed (Beat Up `???`, which nothing resists, so the move fails; Hidden Power its own type).
// The cartridge wins (issues.md I176): the patch reads the move's own type. It draws where
// Showdown failed (the type `sample`).
if (patched) {
  const conversion2 = Dex.mod('gen2').data.Moves.conversion2;
  conversion2.onHit = function (target, source) {
    if (!target.lastMove) return false;
    const possibleTypes = [];
    const attackType = target.lastMove.id === 'struggle' ? 'Normal' : this.dex.moves.get(target.lastMove.id).type;
    for (const typeName of this.dex.types.names()) {
      const typeCheck = this.dex.types.get(typeName).damageTaken[attackType];
      if (typeCheck === 2 || typeCheck === 3) possibleTypes.push(typeName);
    }
    if (!possibleTypes.length) return false;
    const randomType = this.sample(possibleTypes);
    if (!source.setType(randomType)) return false;
    this.add('-start', source, 'typechange', randomType);
  };
}

// I177 — A Gen 4 Slow Start holder's confusion self-hit is halved. pokeplatinum's
// `BattleSystem_CalcMoveDamage` halves the attacking stat of a Slow Start holder whatever the move,
// and the self-hit is Struggle's data (physical). Showdown's gen4 self-hit is a pseudo-move with no
// id (`getDamage(pokemon, pokemon, 40)`), and the gen7 mod's Slow Start (Gen 4-7) asks the data of
// `move.id` for a physical category: it finds none and leaves the Attack alone. The cartridge wins
// (issues.md I177): the patch halves it for the pseudo-move too. No draw changes.
if (patched) {
  const slowstart = Dex.mod('gen4').data.Abilities.slowstart;
  const onModifyAtk = slowstart.onModifyAtk;
  if (onModifyAtk && !onModifyAtk.selfHitHalved) {
    const halved = function (atk, pokemon, target, move) {
      if (this.gen === 4 && move && !move.id && this.effectState.counter) {
        return this.chainModify(0.5);
      }
      return onModifyAtk.call(this, atk, pokemon, target, move);
    };
    halved.selfHitHalved = true;
    slowstart.onModifyAtk = halved;
  }
}

// I178 — A Gen 4 Unaware user of Explosion or Self-Destruct keeps its Unaware for the hit. Showdown
// knocks the user out before the hit (`selfdestruct: 'always'`), and `runEvent` finds `onAny`
// handlers through `foes()` / `alliesAndSelf()`, which skip a Pokémon at 0 HP: the target's stages
// count again. pokeplatinum's `BattleSystem_CalcMoveDamage` and its accuracy check read
// `Battler_Ability(attacker)`, whatever the HP. The cartridge wins (issues.md I178): the patch sets
// aside the target's stages as Unaware's `onAnyModifyBoost` would. No draw changes.
if (patched) {
  const runEvent = Battle.prototype.runEvent;
  Battle.prototype.runEvent = function (eventid, target, ...rest) {
    const result = runEvent.call(this, eventid, target, ...rest);
    const user = this.activePokemon;
    if (this.gen === 4 && eventid === 'ModifyBoost' && user && !user.hp && user !== target &&
        target === this.activeTarget && this.activeMove?.selfdestruct === 'always' &&
        result && typeof result === 'object' && user.hasAbility('unaware')) {
      for (const stat of ['def', 'spd', 'evasion']) {
        if (stat in result) result[stat] = 0;
      }
    }
    return result;
  };
}

// I179 — A Gen 4 Bide release is an attack its target records. pokeplatinum's `subscript_bide_end`
// only sets the damage; the move goes on to `BattleControllerPlayer_UpdateHP`, which records Bide's
// physical damage (`physicalDamageTaken`) as any hit's, so Counter and Metal Burst answer it.
// Showdown's gen4 release (`tryMoveHit` → `moveHit`) never calls `gotAttacked`. The cartridge wins
// (issues.md I179): the patch records the release as the gen3 `tryMoveHit` does, unless it struck a
// Substitute (`UpdateHP` returns before the record). No draw changes of its own.
if (patched) {
  const condition = Dex.mod('gen4').data.Moves.bide.condition;
  const onBeforeMove = condition.onBeforeMove;
  if (onBeforeMove && !onBeforeMove.releaseAttacks) {
    const attacks = function (pokemon, target, move) {
      const actions = this.actions;
      const own = Object.prototype.hasOwnProperty.call(actions, 'tryMoveHit');
      const tryMoveHit = actions.tryMoveHit;
      actions.tryMoveHit = function (hitTarget, user, moveData) {
        const damage = tryMoveHit.call(this, hitTarget, user, moveData);
        if (moveData.id === 'bide' && hitTarget && hitTarget !== user && typeof damage === 'number' && damage > 0) {
          hitTarget.gotAttacked(moveData, damage, user);
        }
        return damage;
      };
      try {
        return onBeforeMove.call(this, pokemon, target, move);
      } finally {
        if (own) actions.tryMoveHit = tryMoveHit; else delete actions.tryMoveHit;
      }
    };
    attacks.releaseAttacks = true;
    condition.onBeforeMove = attacks;
  }
}

// I193 — Gen 4 Life Orb's recoil reads the item held once the move is over. pokeplatinum's
// `BattleController_TriggerAfterMoveHitEffects` runs after the action's whole script and reads the
// attacker's item afresh (`AFTER_MOVE_HIT_STATE_LIFE_ORB`): a tenth of its max HP when a hit of the
// move took real HP (`SYSCTL_MOVE_HIT`, set at the end of `UpdateHP`, which a Substitute's hit
// returns before). Showdown's gen4 item hangs the recoil on a `lifeorb` volatile set at
// `onBasePower`: a fixed-damage, one-hit KO, Counter / Mirror Coat / Metal Burst or Bide damage
// never reaches it, a Life Orb Thief or Covet takes arrives after it, a flung one leaves it behind.
// The cartridge wins (issues.md I193): the patch drops the volatile and gives the item an
// `onAfterMoveSecondarySelf` that pays when the hit loop's `move.totalDamage` (a Substitute's hit
// adds 0) is above 0; Bide's release, outside the hit loop, pays after its `tryMoveHit`. The ×1.3
// stays. No draw.
if (patched) {
  const orb = Dex.mod('gen4').data.Items.lifeorb;
  orb.onBasePower = undefined;
  orb.onAfterMoveSecondarySelf = function (source, target, move) {
    if (move.category !== 'Status' && !move.flags['futuremove'] && move.totalDamage > 0) {
      this.damage(source.baseMaxhp / 10, source, source, this.dex.items.get('lifeorb'));
    }
  };
  const condition = Dex.mod('gen4').data.Moves.bide.condition;
  const onBeforeMove = condition.onBeforeMove;
  if (onBeforeMove && !onBeforeMove.releasePaysLifeOrb) {
    const pays = function (pokemon, target, move) {
      const actions = this.actions;
      const own = Object.prototype.hasOwnProperty.call(actions, 'tryMoveHit');
      const tryMoveHit = actions.tryMoveHit;
      actions.tryMoveHit = function (hitTarget, user, moveData) {
        const damage = tryMoveHit.call(this, hitTarget, user, moveData);
        if (moveData.id === 'bide' && typeof damage === 'number' && damage > 0 &&
          user.hasItem('lifeorb') && !user.ignoringItem()) {
          this.battle.damage(user.baseMaxhp / 10, user, user, this.dex.items.get('lifeorb'));
        }
        return damage;
      };
      try {
        return onBeforeMove.call(this, pokemon, target, move);
      } finally {
        if (own) actions.tryMoveHit = tryMoveHit; else delete actions.tryMoveHit;
      }
    };
    pays.releasePaysLifeOrb = true;
    pays.releaseAttacks = onBeforeMove.releaseAttacks;
    condition.onBeforeMove = pays;
  }
}

// I194 — Gen 6-7 Protean and Toxic. Since Gen 6 a Poison-type user's Toxic cannot miss; Protean
// makes its holder Poison-type before the move (Bulbapedia, Protean: it "will always hit with
// Toxic"; the Showdown bug reports on Smogon were acknowledged). Showdown's gen7 `toxic` asks for
// the type in its own `onPrepareHit`, which runs before the user's `PrepareHit` handlers, so a
// Protean holder only turns Poison-type after the check (Gen 8+ asks in the hit steps, after it).
// Bulbapedia wins (issues.md I194). The patch keeps Showdown's draws: once Gen 6-7 Protean made its
// holder Poison-type for Toxic, a `proteantoxic` volatile (gone with the move) lets the move past a
// semi-invulnerable target (where Showdown drew nothing, the accuracy is `true`) and otherwise turns
// the accuracy it reached into 100 — the draw stays, the move hits, evasion or not. Unlike the gen7
// `toxic` volatile it leaves the user's later moves that turn alone (no draw is skipped). Gen 6-8
// share the ability object.
if (patched) {
  const proteanToxic = {
    name: 'Protean Toxic',
    noCopy: true,
    duration: 1,
    onSourceInvulnerabilityPriority: 1,
    onSourceInvulnerability(target, source, move) {
      if (move?.id === 'toxic' && source === this.effectState.target) return 0;
    },
    onSourceAccuracyPriority: -100,
    onSourceAccuracy(accuracy, target, source, move) {
      if (move?.id !== 'toxic' || source !== this.effectState.target || accuracy === true) return;
      return target.isSemiInvulnerable() ? true : 100;
    },
    onAfterMove(pokemon) {
      pokemon.removeVolatile('proteantoxic');
    },
  };
  for (const mod of ['gen6', 'gen7']) Dex.mod(mod).data.Conditions.proteantoxic = proteanToxic;
  const protean = Dex.mod('gen7').data.Abilities.protean;
  const onPrepareHit = protean.onPrepareHit;
  if (onPrepareHit && !onPrepareHit.toxicSureHit) {
    const prepared = function (source, target, move) {
      const result = onPrepareHit.call(this, source, target, move);
      if (this.gen >= 6 && this.gen <= 7 && move.id === 'toxic' && source.hasType('Poison') &&
        !source.volatiles['toxic']) {
        source.addVolatile('proteantoxic');
      }
      return result;
    };
    prepared.toxicSureHit = true;
    protean.onPrepareHit = prepared;
  }
}

// I195 — Unaware behind a Substitute against a spread move. A Substitute works out the hit's damage
// in its own `onTryPrimaryHit` (`getDamage`), before `getSpreadDamage` sets `activeTarget` target by
// target, so `activeTarget` is still the move's chosen target: Unaware's `onAnyModifyBoost`, which
// asks whether its holder is `activeTarget`, ignores the attacker's stages only for that one. A
// spread move into an Unaware Kangaskhan's doll, aimed at its partner, used the attacker's -1
// Attack. Unaware ignores the attacker's stages whichever target it is (Bulbapedia; the engine
// works the damage out target by target). The patch sets `activeTarget` to the doll's holder
// around the Substitute's `onTryPrimaryHit`. No draw (issues.md I195).
if (patched) {
  const conditions = new Set(['gen4', 'gen5', 'gen6', 'gen7', 'gen8', 'gen9']
    .map(mod => Dex.mod(mod).data.Moves.substitute?.condition).concat(Dex.data.Moves.substitute.condition)
    .filter(Boolean));
  for (const condition of conditions) {
    const onTryPrimaryHit = condition.onTryPrimaryHit;
    if (!onTryPrimaryHit || onTryPrimaryHit.ownTarget) continue;
    const own = function (target, source, move) {
      const activeTarget = this.activeTarget;
      this.activeTarget = target;
      try {
        return onTryPrimaryHit.call(this, target, source, move);
      } finally {
        this.activeTarget = activeTarget;
      }
    };
    own.ownTarget = true;
    condition.onTryPrimaryHit = own;
  }
}

// I165 — A Gen 4 foreseen strike sets off none of the target's on-hit items. pokeplatinum strikes
// Future Sight and Doom Desire at the end of the turn through `subscript_future_sight_damage`, which
// takes the HP (`UpdateHP`) and never reaches `BattleSystem_TriggerHeldItemOnHit`: Jaboca, Rowap and
// Enigma Berry stay, Sticky Barb stays put. Showdown runs the strike's `DamagingHit` and `Hit` events
// as any hit's. The cartridge wins (issues.md I165): the patch skips those items' handlers for a move
// with the `futuremove` flag. No draw.
if (patched) {
  const items = Dex.mod('gen4').data.Items;
  // `onDamagingHit(damage, target, source, move)`, `onHit(target, source, move)`.
  for (const [id, event, at] of [['jabocaberry', 'onDamagingHit', 3], ['rowapberry', 'onDamagingHit', 3],
    ['enigmaberry', 'onHit', 2], ['stickybarb', 'onHit', 2]]) {
    const handler = items[id]?.[event];
    if (!handler || handler.foreseenSkipped) continue;
    const skipped = function (...args) {
      const move = args[at];
      if (this.gen === 4 && move?.flags?.['futuremove']) return;
      return handler.apply(this, args);
    };
    skipped.foreseenSkipped = true;
    items[id][event] = skipped;
  }
}


// I180 — Instruct uses the move again, Counter, Mirror Coat and Pursuit included. Showdown's
// `instruct.onHit` prioritizes `resolveAction({choice: 'move', …})[0]`; for a move with a
// `beforeTurnCallback` that first action is its `beforeTurnMove`, so the instructed Pokémon only
// re-runs the callback and never uses the move. The user decided to treat this as a defect
// (issues.md I180, no cartridge for Gen 7+): the patch prioritizes the move's own action, and for
// Counter / Mirror Coat with no volatile from this turn's start, sets one up with what it would have
// recorded — the last hit of the right category a foe dealt this turn (`attackedBy`). The move then
// draws as any move does: the user allowed the change in draws.
if (patched) {
  const retaliations = {counter: 'Physical', mirrorcoat: 'Special'};
  for (const mod of ['gen7', 'gen8', 'gen9']) {
    const instruct = Dex.mod(mod).data.Moves.instruct;
    const onHit = instruct?.onHit;
    if (!onHit || onHit.usesTheMove) continue;
    const uses = function (target, source) {
      const queue = this.queue;
      const own = Object.prototype.hasOwnProperty.call(queue, 'resolveAction');
      const resolveAction = queue.resolveAction;
      queue.resolveAction = function (action, midTurn) {
        const actions = resolveAction.call(this, action, midTurn);
        if (action.choice !== 'move') return actions;
        return [...actions.filter(a => a.choice === 'move'), ...actions.filter(a => a.choice !== 'move')];
      };
      let result;
      try {
        result = onHit.call(this, target, source);
      } finally {
        if (own) queue.resolveAction = resolveAction; else delete queue.resolveAction;
      }
      const id = target.lastMove?.id;
      const category = retaliations[id];
      if (result !== false && category && !target.volatiles[id] && target.addVolatile(id)) {
        const hit = [...target.attackedBy].reverse().find(entry => entry.thisTurn &&
          !entry.source.isAlly(target) && typeof entry.damageValue === 'number' &&
          this.dex.moves.get(entry.move).category === category);
        if (hit) {
          target.volatiles[id].slot = hit.slot;
          target.volatiles[id].damage = 2 * hit.damageValue;
        }
      }
      return result;
    };
    uses.usesTheMove = true;
    instruct.onHit = uses;
  }
}

// I182 — Gen 3 Encore against a lock. Showdown's `encore.onOverrideAction` turns any action into
// the encored move, a locked one included: a Pokémon charging Dive that Metronome called, encored
// on Metronome, uses Metronome at its own diving self and misses. pokeemerald
// `HandleAction_UseMove` takes the locked move first (`STATUS2_MULTIPLETURNS` or
// `STATUS2_RECHARGE`: `gLockedMoves`) and only then the encored one: the lock goes on. The patch
// leaves a locked action alone (`getLockedMove()`: a charge, Thrash, Uproar, Rollout, Bide, a
// recharge). The locked move then draws as it does: the user allowed the change in draws.
if (patched) {
  const condition = Dex.mod('gen3').data.Moves.encore?.condition;
  const override = condition?.onOverrideAction;
  if (override && !override.spareLocks) {
    const spares = function (pokemon, target, move) {
      if (pokemon.getLockedMove()) return;
      return override.call(this, pokemon, target, move);
    };
    spares.spareLocks = true;
    condition.onOverrideAction = spares;
  }
}

// I183 — Emergency Exit / Wimp Out after a move's hits. The end of Showdown's
// `hitStepMoveHitLoop` checks every target with `(hurtThisTurn || 0) + curDamage` as its HP before
// the move. `hurtThisTurn` is the HP left by the last `spreadDamage`, which a Belly Drum's,
// Substitute's or Curse's HP cost (`directDamage`) and a Pain Split (`sethp`) leave alone, and a
// status move's `curDamage` is 0: a Pokémon hit earlier in the turn but still above half, that
// pays a Belly Drum down below half, switches out. The user decided (issues.md I183, no cartridge
// for Gen 7-8): the ability answers the damage the move dealt, not an HP cost — the patch lets
// those checks through only for a target this move's own damage struck (its `spreadDamage` with
// the move as the effect). The checks elsewhere (recoil, Rough Skin on the user, the residual)
// are not touched; a confusion self-hit has none. No draw changes.
//
// I206 — Dragon Darts' two darts on two targets: the first target's check. The smart-target loop
// rebuilds `damage` as a one-entry array each dart (`damage = [damage[hit - 1]]`), so after the
// second dart the first target's entry is `undefined` and its check is skipped: an Emergency Exit
// or Wimp Out holder the first dart took below half stays in. The user decided (issues.md I206, no
// cartridge for Gen 8): each dart's target is checked with its own dart's damage, in target order —
// the first target's check runs before the second's, or after the loop when the second has none.
// No draw changes.
if (patched) {
  function hitStepMoveHitLoop(targets, pokemon, move, ...rest) {
    const battle = this.battle;
    const inner = this.dex.data.Scripts.actions?.hitStepMoveHitLoop ||
      Object.getPrototypeOf(this).hitStepMoveHitLoop;
    const struck = new Set();
    let checking = false;
    // I206: the darts' targets when the move splits them, and the damage each one's dart dealt.
    const darts = move.smartTarget && targets.length > 1 ? targets.slice() : null;
    const dartDamage = new Map();
    let firstChecked = false;
    const ownSpread = Object.prototype.hasOwnProperty.call(battle, 'spreadDamage');
    const ownRun = Object.prototype.hasOwnProperty.call(battle, 'runEvent');
    const ownAfter = Object.prototype.hasOwnProperty.call(this, 'afterMoveSecondaryEvent');
    const spreadDamage = battle.spreadDamage;
    const runEvent = battle.runEvent;
    const afterMoveSecondaryEvent = this.afterMoveSecondaryEvent;
    battle.spreadDamage = function (damage, targetArray, source, effect, ...more) {
      const dealt = spreadDamage.call(this, damage, targetArray, source, effect, ...more);
      if (effect === move && Array.isArray(targetArray)) {
        targetArray.forEach((target, i) => {
          if (target && typeof dealt?.[i] === 'number' && dealt[i] > 0) struck.add(target);
          if (target && darts && typeof dealt?.[i] === 'number') dartDamage.set(target, dealt[i]);
        });
      }
      return dealt;
    };
    // The first dart's target, as the end of the loop would check it with its own damage.
    const checkFirst = () => {
      if (firstChecked || !darts || move.hit < 2) return;
      firstChecked = true;
      if (move.hasSheerForce && pokemon.hasAbility('sheerforce')) return;
      const first = darts[0];
      const dealt = dartDamage.get(first);
      if (typeof dealt !== 'number' || !first.hp || !struck.has(first)) return;
      const before = (first.hurtThisTurn || 0) + dealt;
      if (first.hp <= first.maxhp / 2 && before > first.maxhp / 2) {
        runEvent.call(battle, 'EmergencyExit', first, pokemon);
      }
    };
    // The checks come right after the move's own `AfterMoveSecondary` events.
    this.afterMoveSecondaryEvent = function (eventTargets, source, eventMove, ...more) {
      const result = afterMoveSecondaryEvent.call(this, eventTargets, source, eventMove, ...more);
      if (eventMove === move) checking = true;
      return result;
    };
    battle.runEvent = function (eventid, target, ...more) {
      if (checking && eventid === 'EmergencyExit') {
        if (darts && target !== darts[0]) checkFirst();
        if (!struck.has(target)) return undefined;
      }
      return runEvent.call(this, eventid, target, ...more);
    };
    try {
      const result = inner.call(this, targets, pokemon, move, ...rest);
      if (checking) checkFirst();
      return result;
    } finally {
      if (ownSpread) battle.spreadDamage = spreadDamage; else delete battle.spreadDamage;
      if (ownRun) battle.runEvent = runEvent; else delete battle.runEvent;
      if (ownAfter) this.afterMoveSecondaryEvent = afterMoveSecondaryEvent;
      else delete this.afterMoveSecondaryEvent;
    }
  }
  for (const [id, format] of Object.entries(Dex.data.Rulesets)) {
    if (!/^gen[789]/.test(id)) continue;
    format.actions = {...format.actions, hitStepMoveHitLoop};
    const cached = Dex.formats.get(id);
    cached.actions = {...cached.actions, hitStepMoveHitLoop};
  }
}

// I8 — Poison Touch and Protective Pads. Showdown's `poisontouch.onSourceDamagingHit` asks
// `checkMoveMakesContact(move, target, source)`, the arguments the other way round from every
// other contact effect: the Pads it reads are the target's, so a target holding them is never
// poisoned while the holder's own Pads change nothing. The Sun / Moon research (Smogon's battle
// mechanics thread, #423-#424) found Pads do neither: they keep neither their holder from being
// poisoned nor its own Poison Touch from triggering. The patch asks the move's own contact flag
// (Long Reach and Punching Glove take it away). Against a target holding Pads the 3/10 is now
// drawn: the user allowed the change in draws (issues.md I8).
if (patched) {
  for (const mod of ['gen5', 'gen6', 'gen7', 'gen8', 'gen9']) {
    const ability = Dex.mod(mod).data.Abilities.poisontouch;
    if (!ability?.onSourceDamagingHit || ability.onSourceDamagingHit.ignoresPads) continue;
    const touches = function (damage, target, source, move) {
      if (target.hasAbility('shielddust') || target.hasItem('covertcloak')) return;
      if (move.flags['contact'] && this.randomChance(3, 10)) {
        target.trySetStatus('psn', source);
      }
    };
    touches.ignoresPads = true;
    ability.onSourceDamagingHit = touches;
  }
}

// I196 — Fling with a Flower Sweet. The Sword / Shield item table gives every Sweet a Fling power
// of 10, the Flower Sweet too (Smogon's Sword & Shield battle mechanics research: Marty's datamine,
// "a power of 0 means it can't be flung"); Showdown's data has `fling: {basePower: 0}`, so its Fling
// throws the Sweet for nothing (`getDamage` returns at a power of 0, before the critical-hit and
// damage rolls). The patch gives it 10: the throw now draws its critical hit and damage roll — the
// draws change, under the standing decision to follow the cartridge (TESTING.md §7).
if (patched) {
  for (const mod of ['gen8', 'gen9']) {
    const sweet = Dex.mod(mod).data.Items.flowersweet;
    if (sweet?.fling) sweet.fling = {...sweet.fling, basePower: 10};
  }
}

// I173 (its Gen 3 part) — a Gen 3 move that failed is no attack Mirror Move can copy. pokeemerald's
// `MOVEEND_MIRROR_MOVE` writes the taken move only without `MOVE_RESULT_NO_EFFECT`, which a failure
// sets (`BattleScript_ButItFailed`: `MOVE_RESULT_FAILED` — a Yawn at a sleeper, a Mimic with nothing
// to copy, a Growl at -6). Showdown's gen3 `tryMoveHit` calls `gotAttacked` after `moveHit` whatever
// it returned, so such a failure (`moveHit` gave false or null) became the last attack. The patch
// records none in Gen 3: Mirror Move copies the attack before it, or fails, and draws accordingly —
// the draws change, under the standing decision to follow the cartridge (TESTING.md §7).
if (patched) {
  const gotAttacked = Pokemon.prototype.gotAttacked;
  Pokemon.prototype.gotAttacked = function (move, damage, source) {
    if (this.battle.gen === 3 && (damage === false || damage === null)) return;
    return gotAttacked.call(this, move, damage, source);
  };
}

// I197 — Gen 4: Transform into Arceus re-formes by the transformer's own plate only with Multitype.
// pokeplatinum's Transform (`BtlCmd_Transform`) copies `BattleMon` up to and including `ability`:
// the target's species, types, `formNum` and ability. The form-change check (`battle_lib.c`, its
// Arceus branch) sets the form from the held item only for a battler whose ability is Multitype
// (`Battler_Ability(…) == ABILITY_MULTITYPE`). Showdown's `transformInto` (its Gen 4 block) re-formes
// a Pokémon transformed into Arceus by its held item whatever the ability it copied: one holding a
// Toxic Plate became Arceus-Poison after copying a White Smoke Arceus. The patch lets that forme
// change through only for a Multitype holder. A type the transformer no longer takes may change
// what a move draws against it (an immunity) — under the standing decision (TESTING.md §7).
if (patched) {
  const transformInto = Pokemon.prototype.transformInto;
  Pokemon.prototype.transformInto = function (pokemon, effect) {
    if (this.battle.gen !== 4 || pokemon.species.num !== 493) return transformInto.call(this, pokemon, effect);
    const own = Object.prototype.hasOwnProperty.call(this, 'formeChange');
    const formeChange = this.formeChange;
    this.formeChange = function (...args) {
      if (this.ability !== 'multitype') return false;
      return formeChange.apply(this, args);
    };
    try {
      return transformInto.call(this, pokemon, effect);
    } finally {
      if (own) this.formeChange = formeChange;
      else delete this.formeChange;
    }
  };
}

// I199 — Big Root raises the damage its holder takes from Liquid Ooze (Bulbapedia's Big Root: "The Big Root
// also increases the damage taken by the holder from Liquid Ooze"; Gen 4 already does: the gen4 mod's Big Root
// returns `Math.floor(damage * 1.3)` before Liquid Ooze reads the heal, as pokeplatinum boosts it before the
// ooze check). From Gen 5 Showdown's Big Root `chainModify`s, a modifier applied only once `TryHeal` is over,
// so Liquid Ooze, a later handler, deals the heal as it was before the boost. The patch has Liquid Ooze deal the
// boosted amount (the same 5324 / 4096 `modify`) where the healer's Big Root raises that heal. No draw.
if (patched) {
  const boosted = ['drain', 'leechseed', 'ingrain', 'aquaring', 'strengthsap'];
  const oozes = new Set(['gen5', 'gen6', 'gen7', 'gen8', 'gen9']
    .map(mod => Dex.mod(mod).data.Abilities.liquidooze).filter(Boolean));
  for (const ooze of oozes) {
    const onSourceTryHeal = ooze.onSourceTryHeal;
    ooze.onSourceTryHeal = function (damage, target, source, effect) {
      if (this.gen >= 5 && boosted.includes(effect?.id) && target.hasItem('bigroot')) {
        damage = this.modify(damage, 5324, 4096);
      }
      return onSourceTryHeal.call(this, damage, target, source, effect);
    };
  }
}

// I202 — Gen 6 Normalize under Ion Deluge or Electrify. Bulbapedia's Ion Deluge (Generation VI): "If the move
// Flying Press used by a Pokémon with Normalize is affected by Ion Deluge, it deals Electric/Flying-type
// damage" — Normalize makes the move Normal, then Ion Deluge makes it Electric (its Normalize page says the
// same of Electrify and Ion Deluge from Generation VII). Showdown's gen6 Normalize keeps the base `onModifyType`
// (Normal, priority 1, before Ion Deluge's -2 and Electrify's) and adds an `onModifyMove` that makes the move
// Normal again after them. The patch leaves a move they turned Electric as it is. The type decides immunities
// (a Ground type's to Electric, a Ghost's to Normal), and with them which draws a hit makes (TESTING.md §7).
if (patched) {
  const normalize = Dex.mod('gen6').data.Abilities.normalize;
  const onModifyMove = normalize.onModifyMove;
  normalize.onModifyMove = function (move, pokemon, target) {
    if (move.type === 'Electric'
      && (this.field.pseudoWeather['iondeluge'] || pokemon.volatiles['electrify'])) {
      return;
    }
    return onModifyMove.call(this, move, pokemon, target);
  };
}

// I198 — A strong weather's holder leaving ends only its strong weather. Showdown's Delta Stream, Primordial Sea and
// Desolate Land `onEnd` ask only whose the weather is (`weatherState.source`), not which it is: a rain the holder's
// own Rain Dance started while Gastro Acid kept its ability quiet ended with it. The user's decision (2026-10-17):
// the weather ends — or passes to another holder of the same ability, as before — only when it is that ability's
// strong weather and the holder is its source (the strong weathers still replace one another, and an old one does
// not come back: Nugget Bridge's 2015 VGC research). The patch has the `onEnd` leave any other weather alone. The
// weather that stays changes what later moves draw (Thunder's accuracy in the rain …: TESTING.md §7).
if (patched) {
  const seen = new Set();
  for (const mod of ['gen6', 'gen7', 'gen8', 'gen9']) {
    for (const id of ['deltastream', 'primordialsea', 'desolateland']) {
      const ability = Dex.mod(mod).data.Abilities[id];
      if (!ability || seen.has(ability)) continue;
      seen.add(ability);
      const onEnd = ability.onEnd;
      ability.onEnd = function (pokemon) {
        if (this.field.weather !== id) return;
        return onEnd.call(this, pokemon);
      };
    }
  }
}

// I201 — Pressure's extra PP for a combined Pledge goes to the Pledge its user uses. Showdown's `useMoveInner` charges
// Pressure to `sourceEffect` when it has PP (`callerMoveForPressure`), meant for a move another move called (Sleep
// Talk, Metronome); a combined Pledge's `sourceEffect` is the ally's Pledge that moved its action up, so the extra PP
// went to the user's move of that name — or nowhere. The user's decision (2026-10-17, a presumption of the executor's
// ownership, not a cartridge-verified rule): the Pokémon that strikes pays it on the Pledge it uses. No draw; the PP
// left can change a later choice.
if (patched) {
  const deductPP = Pokemon.prototype.deductPP;
  Pokemon.prototype.deductPP = function (move, amount, target) {
    const active = this.battle.activeMove;
    if (amount && active?.flags?.pledgecombo && this.battle.activePokemon === this) {
      const charged = this.battle.dex.moves.get(move);
      if (charged.flags?.pledgecombo && charged.id !== active.id && charged.id === active.sourceEffect) {
        move = active.id;
      }
    }
    return deductPP.call(this, move, amount, target);
  };
}

// I200 — Magician after a split Dragon Darts weighs both darts' targets. Showdown's hit loop rebuilds its damage
// list at each dart of a split (`damage = [damage[hit - 1]]`), so the list it returns names the second dart alone
// and `move.hitTargets` keeps that target only: Magician's speed order (`speedSort(hitTargets)`) never sees the
// first. The user's decision (2026-10-17): Magician weighs every target a dart struck, faster first (Smogon's
// Magician: a move that hits several targets takes from the fastest that has an item). The patch keeps the
// targets the darts were thrown at (`move.dartTargets`: as many as the loop ran) and hands Magician those as its
// `hitTargets`. Two targets of the same speed are now shuffled (`speedSort`): TESTING.md §7.
if (patched) {
  const actions = Object.getPrototypeOf(new Battle({formatid: 'gen9customgame'}).actions);
  const hitLoop = actions.hitStepMoveHitLoop;
  actions.hitStepMoveHitLoop = function (targets, pokemon, move) {
    const darts = move.smartTarget && targets.length > 1 ? targets.slice() : null;
    const result = hitLoop.call(this, targets, pokemon, move);
    if (darts) move.dartTargets = darts.slice(0, move.hit);
    return result;
  };
  const magicians = new Set(['gen8', 'gen9'].map(mod => Dex.mod(mod).data.Abilities.magician).filter(Boolean));
  for (const magician of magicians) {
    const handler = magician.onAfterMoveSecondarySelf;
    magician.onAfterMoveSecondarySelf = function (source, target, move) {
      if (!move?.dartTargets) return handler.call(this, source, target, move);
      const hitTargets = move.hitTargets;
      move.hitTargets = move.dartTargets.slice();
      try {
        return handler.call(this, source, target, move);
      } finally {
        move.hitTargets = hitTargets;
      }
    };
  }
}

// I203 — Gen 2: a sleeping user's multi-hit move hits every time. pokecrystal's `BattleCommand_EndLoop`
// checks no sleep at all, so Sleep Talk calling Metronome calling Bone Rush lands each hit; Showdown's
// gen2 loop reads `move.sourceEffect`'s `sleepUsable` (Metronome's: none) and stops before the first.
// (pokeemerald's `BattleScript_MultiHitLoop` reads `gChosenMove` — Sleep Talk — to the same effect,
// but Showdown's Gen 3 Sleep Talk never calls a calling move: Metronome is `nosleeptalk`.) The patch
// marks the move sleep-usable for its hit loop while its user sleeps. The hit count is drawn either
// way; the hits' draws (critical hits, damage) come in (TESTING.md §7).
for (const mod of patched ? ['gen2'] : []) {
  const actions = Dex.mod(mod).data.Scripts.actions;
  const tryMoveHit = actions.tryMoveHit;
  actions.tryMoveHit = function (target, pokemon, move) {
    if (this.battle.gen === 2 && move.multihit && !move.sleepUsable && pokemon.status === 'slp') {
      move.sleepUsable = true;
    }
    return tryMoveHit.call(this, target, pokemon, move);
  };
}

// I204 — Gen 3: an Air Lock or Cloud Nine holder brought to 0 HP stops suppressing the weather at
// once. pokeemerald's `ABILITYEFFECT_CHECK_ON_FIELD` counts a holder with `hp != 0`; Showdown waits
// for its `faint` (`!pokemon.fainted`). A Gen 3 spread move hits target by target (the gen3
// `useMoveInner`), so a holder knocked out by the first hit no longer keeps the rain or sun off the
// next one. Nothing drawn changes.
if (patched) {
  const field = Object.getPrototypeOf(new Battle({formatid: 'gen3customgame'}).field);
  const suppressingWeather = field.suppressingWeather;
  field.suppressingWeather = function () {
    if (this.battle.gen !== 3) return suppressingWeather.call(this);
    return this.battle.sides.some(side => side.active.some(pokemon => pokemon && pokemon.hp &&
      !pokemon.fainted && !pokemon.ignoringAbility() && pokemon.getAbility().suppressWeather &&
      !pokemon.abilityState.ending));
  };
}

// I205 — Gen 3 singles: an entrant already on the field still runs its switch-in after an earlier
// entrant falls to the hazards. Two replacements come in together (both sides' last Pokémon fell at
// the turn's end); the first one's Spikes knock it out. Showdown's gen 3 faint rule then cancels
// every active Pokémon's queued actions (`queue.cancelAction`), the other entrant's `runSwitch`
// with them: no Spikes, no switch-in ability. pokeemerald runs each fainted battler's
// `switchineffects` in turn (`BattleScript_LinkHandleFaintedMonLoop`). The patch keeps a queued
// `runSwitch` through that rule. (The cartridge's order differs further: the fallen entrant's own
// replacement comes in before the other entrant's `switchineffects`; not modelled.) The kept
// switch-in's draws (Trace's foe) come in (TESTING.md §7).
if (patched) {
  const faintMessages = Battle.prototype.faintMessages;
  Battle.prototype.faintMessages = function (...args) {
    if (this.gen !== 3 || this.gameType !== 'singles') return faintMessages.apply(this, args);
    const queue = this.queue;
    queue.cancelAction = function (pokemon) {
      const before = this.list.length;
      this.list = this.list.filter(action => action.pokemon !== pokemon || action.choice === 'runSwitch');
      return this.list.length !== before;
    };
    try {
      return faintMessages.apply(this, args);
    } finally {
      delete queue.cancelAction;
    }
  };
}


// I207 — Focus Punch and a heal. Showdown's `focuspunch` condition (`onHit`) costs the user its
// focus for any move that is not a status move hitting it, a heal included: Present's
// (`move.heal`), Pollen Puff's on an ally. Only damage does: pokeemerald's `jumpifnodamage` reads
// `physicalDmg` / `specialDmg`, which `datahpupdate` sets on its damage branch alone (a negative
// damage, Present's heal, only adds HP); pokeplatinum's Focus Punch script reads the damage masks
// `BattleControllerPlayer_UpdateHP` sets for a damaging hit (`damage < 0`), never for a heal; and
// Bulbapedia: "The move fails if the user has already taken damage from a move in the same turn"
// (Gen 3-4 by the cartridges, Gen 5+ by Bulbapedia; issues.md I207). The patch keeps the focus
// through a heal. The condition is one object every generation shares. Focus Punch then goes
// ahead and draws (TESTING.md §7).
if (patched) {
  const condition = Dex.data.Moves.focuspunch?.condition;
  const onHit = condition?.onHit;
  if (onHit && !onHit.keepsFocusThroughAHeal) {
    const keeps = function (pokemon, source, move) {
      if (move?.heal || (move?.id === 'pollenpuff' && source?.isAlly(pokemon))) return;
      return onHit.call(this, pokemon, source, move);
    };
    keeps.keepsFocusThroughAHeal = true;
    condition.onHit = keeps;
  }
}

// I208 — Gen 3-4: a Cloud Nine or Air Lock coming in turns Castform (and, Gen 4, Cherrim) back to
// its base forme. pokeemerald's `ABILITYEFFECT_ON_SWITCHIN` runs `CastformDataTypeChange` over
// every battler for an entering Cloud Nine / Air Lock (`!WEATHER_HAS_EFFECT`: back to Normal), and
// the fainted-actions step after every action runs `ABILITYEFFECT_FORECAST` again; pokeplatinum's
// `BattleSystem_TriggerFormChange` turns Castform and Cherrim back while a Cloud Nine holder stands
// (`!NO_CLOUD_NINE`). Showdown's gen4 mod (Gen 3 inherits it) drops the base `onStart`'s
// `eachEvent('WeatherChange')`, so the forme stays until the weather next changes. The patch runs
// it again; the holder still announces nothing (issues.md I208). Nothing drawn changes.
if (patched) {
  for (const mod of ['gen3', 'gen4']) {
    for (const id of ['cloudnine', 'airlock']) {
      const ability = Dex.mod(mod).data.Abilities[id];
      ability.onStart = function (pokemon) {
        pokemon.abilityState.ending = false;
        this.eachEvent('WeatherChange', this.effect);
      };
    }
  }
}

// I209 — Gen 3+: a Counter or Mirror Coat another move calls strikes back too. Showdown records the
// hit to answer only in the `counter` / `mirrorcoat` volatile the move's `beforeTurnCallback` adds
// as the turn begins, and only for the move chosen: Sleep Talk calling it (Gen 3-9), Copycat (Gen 9
// Mirror Coat) or Encore turning an action into it finds no volatile, and `onTry` fails.
// pokeemerald `Cmd_mirrorcoatdamagecalculator` / pokeplatinum `BtlCmd_MirrorCoat` read the hits of
// the turn (`specialDmg`, `specialDamageTakenFrom`), whatever the user chose; the user decided Gen 5+
// follows them (issues.md I209, as I180 for Instruct). The patch: before such a move picks its
// target (the volatile's `onRedirectTarget`), it gets the volatile with what it would have recorded
// — the last hit of the turn a foe dealt it whose move's category its condition answers (Gen 3:
// Hidden Power counts for Counter, never for Mirror Coat; `gotAttacked` keeps the category the hit
// had) — and loses it once the move is over. With no such hit it fails as before, untouched. The move
// then draws as any move does: the user allowed the change in draws.
if (patched) {
  const gotAttacked = Pokemon.prototype.gotAttacked;
  Pokemon.prototype.gotAttacked = function (move, damage, source) {
    gotAttacked.call(this, move, damage, source);
    const entry = this.attackedBy[this.attackedBy.length - 1];
    if (entry) entry.category = this.battle.dex.moves.get(move).category;
  };
  const answers = {
    counter: (gen, entry) => entry.category === 'Physical' || (gen === 3 && entry.move === 'hiddenpower'),
    mirrorcoat: (gen, entry) => entry.category === 'Special' && !(gen === 3 && entry.move === 'hiddenpower'),
  };
  const actions = Object.getPrototypeOf(new Battle({formatid: 'gen9customgame'}).actions);
  const useMove = actions.useMove;
  actions.useMove = function (move, pokemon, options) {
    const id = this.dex.moves.get(move).id;
    const answer = this.battle.gen >= 3 && !pokemon.volatiles[id] && answers[id];
    const hit = answer && [...pokemon.attackedBy].reverse().find(entry => entry.thisTurn &&
      !entry.source.isAlly(pokemon) && typeof entry.damageValue === 'number' && answer(this.battle.gen, entry));
    if (!hit || !pokemon.addVolatile(id)) return useMove.call(this, move, pokemon, options);
    pokemon.volatiles[id].slot = hit.slot;
    pokemon.volatiles[id].damage = 2 * hit.damageValue;
    try {
      return useMove.call(this, move, pokemon, options);
    } finally {
      pokemon.removeVolatile(id);
    }
  };
}

// I210 — Gen 3 Lightning Rod names itself as it takes a move. Showdown's gen3 mod redirects with no
// line (`onFoeRedirectTarget`); pokeemerald's `Cmd_attackcanceler` runs `BattleScript_TookAttack`
// ("{B_DEF_NAME_WITH_PREFIX}'s {B_DEF_ABILITY}\ntook the attack!") and `RecordAbilityBattle`: the
// ability is shown, as Gen 4+ shows it. The patch prints Gen 4+'s line
// (`-activate|holder|ability: Lightning Rod`) when the move turns to the holder. No draw changes.
if (patched) {
  const rod = Dex.mod('gen3').data.Abilities.lightningrod;
  const redirect = rod.onFoeRedirectTarget;
  rod.onFoeRedirectTarget = function (target, source, source2, move) {
    const holder = redirect.call(this, target, source, source2, move);
    if (holder && holder !== target) this.add('-activate', holder, 'ability: Lightning Rod');
    return holder;
  };
}

// I211 — a White Herb its holder may not use stays held. An item's `Start` is the one handler
// `singleEvent` lets through while the holder ignores its item (Magic Room, Embargo, Klutz), and
// White Herb's `onStart` uses itself with no `ignoringItem()` check, where the seeds and Room
// Service check it: a holder with lowered stats that gets one by Trick under Magic Room loses it
// (`useItem`), while its `Use` handler, which restores the stats, stays suppressed. Magic Room
// "suppresses the effect of all items held by the Pokémon on the field" (Bulbapedia): the herb
// stays, to act once the item may (its residual right after Magic Room ends, an `AnyAfterMove`).
// No draw changes.
if (patched) {
  const herbs = new Set(['gen3', 'gen4', 'gen5', 'gen6', 'gen7', 'gen8', 'gen9']
    .map(mod => Dex.mod(mod).data.Items.whiteherb));
  for (const herb of herbs) {
    const start = herb.onStart;
    herb.onStart = function (pokemon) {
      if (!pokemon.ignoringItem()) return start.call(this, pokemon);
    };
  }
}

// I212 — Gen 7's Rollout storage glitch as the cartridge has it. Gen 7 Disguise takes a Rollout /
// Ice Ball hit as a hit of the chain and not as a doubling (the gen7 mod's `onDamage`:
// `contactHitCount--`); a chain that ends its five hits short of doublings stores the rest
// (`onAfterMove`: `rolloutstorage`). Showdown keeps one stored doubling for two turns
// (`duration: 2`), and a Rollout used on it locks its user in again. The cartridge (Bulbapedia's
// "Rollout storage glitch"; Smogon's USUM research: "Rollout Multiplier does not reset unless a
// Rollout/Ice Ball fails; Pokemon using Rollout/Ice Ball during this state will not be locked
// into" it) keeps the stored doublings until moves that compute a power use them, one each, up to
// the fifth hit's; a Rollout or Ice Ball used on them starts no chain, and one that fails clears
// them. The patch: no duration; `onBasePower`, ahead of every other participant, takes the next
// stored doubling and keeps the rest; Rollout / Ice Ball used on the storage add no `rollout` /
// `iceball` volatile, and one that computed no power clears it; Disguise's decrement asks for the
// chain it decrements (a Rollout with none — on the storage, or called asleep — would throw). No
// draw changes.
if (patched) {
  const data = Dex.mod('gen7').data;
  const storage = data.Conditions.rolloutstorage;
  delete storage.duration;
  storage.onBasePowerPriority = 31;
  storage.onBasePower = function (basePower, source, target, move) {
    const state = source.volatiles['rolloutstorage'];
    let power = Math.max(1, move.basePower) * 2 ** state.contactHitCount;
    if (source.volatiles['defensecurl']) power *= 2;
    move.rolloutStorageUsed = true;
    if (state.contactHitCount >= 4) {
      source.removeVolatile('rolloutstorage');
    } else {
      state.contactHitCount++;
    }
    return power;
  };
  storage.onAfterMove = function (source, target, move) {
    if (move.rolloutStorageArmed && !move.rolloutStorageUsed) source.removeVolatile('rolloutstorage');
  };
  for (const id of ['rollout', 'iceball']) {
    const rolling = data.Moves[id];
    const modify = rolling.onModifyMove;
    rolling.onModifyMove = function (move, pokemon, target) {
      if (pokemon.volatiles['rolloutstorage'] && pokemon.status !== 'slp' && target) {
        move.rolloutStorageArmed = true;
        return;
      }
      return modify.call(this, move, pokemon, target);
    };
  }
  const disguise = data.Abilities.disguise;
  disguise.onDamage = function (damage, target, source, effect) {
    if (effect && effect.effectType === 'Move' &&
      ['mimikyu', 'mimikyutotem'].includes(target.species.id) && !target.transformed) {
      if (['rollout', 'iceball'].includes(effect.id) && source.volatiles[effect.id]) {
        source.volatiles[effect.id].contactHitCount--;
      }
      this.add('-activate', target, 'ability: Disguise');
      this.effectState.busted = true;
      return 0;
    }
  };
}

// I213 — Gen 5+: what a called move did is what the action did. Showdown's `runMove` reads
// `moveDidSomething` — what the action's own move's `useMove` returned — for Dancer (`if
// (move.flags['dance'] && moveDidSomething …)`, `move` being the called move by then) and for
// `lastSuccessfulMoveThisTurn` (Fusion Flare's and Fusion Bolt's doubling). A move called in the
// caller's `onHit` (Assist, Metronome, Sleep Talk, Copycat) always did something; one called in its
// `onTryHit`, which returns `null` (Me First, Gen 5+ Mirror Move, Gen 6+ Nature Power), never did:
// Dancer follows a Swords Dance Assist calls at +6, but not a Feather Dance Mirror Move calls that
// lands. The cartridge research has a dance that failed — Swords Dance at +6 included — go
// uncopied, and Fusion Flare's boost needs a Fusion Bolt "successfully used"; the user decided the
// move executed is what counts (issues.md I213). The patch: the outermost `useMove` of a Pokémon
// returns the result of the last move it executed itself — a move it called in turn — while every
// `useMove` still sets `moveThisTurnResult` as before; another Pokémon's move inside (Magic Coat's,
// Magic Bounce's, Snatch's) is that Pokémon's own and leaves the user's result alone. Only
// `runMove` reads the value. Dancer then dances (or not) as its draws go: the user allowed the
// change in draws.
if (patched) {
  const actions = Object.getPrototypeOf(new Battle({formatid: 'gen9customgame'}).actions);
  const useMove = actions.useMove;
  const executing = new WeakMap();
  actions.useMove = function (move, pokemon, options) {
    if (this.battle.gen < 5) return useMove.call(this, move, pokemon, options);
    const enclosing = executing.get(pokemon);
    const record = {called: false, result: undefined};
    executing.set(pokemon, record);
    let result;
    try {
      result = useMove.call(this, move, pokemon, options);
    } finally {
      if (enclosing) executing.set(pokemon, enclosing);
      else executing.delete(pokemon);
    }
    const executed = record.called ? record.result : result;
    if (!enclosing) return executed;
    enclosing.called = true;
    enclosing.result = executed;
    return result;
  };
}

// I214 — Gen 4: Safeguard stops every confusion berry's confusion. pokeplatinum runs a confusion
// berry the eater dislikes through `subscript_held_item_dislike_flavor`, whose `subscript_confuse`
// (`SIDE_EFFECT_TYPE_HELD_ITEM`) ends silently under Own Tempo, an existing confusion or Safeguard on
// the eater's side — the berry it held, one Bug Bite or Pluck took (`BattleSystem_PluckBerry`), one
// flung at it (`BattleSystem_FlingItem`) alike. Showdown's `onTryAddVolatile` asks `target !== source`:
// only the flung berry, the flinger the event's source, is stopped. The patch: the gen4 Safeguard
// stops a confusion whose effect is a berry, silently, before the duration is drawn — so the draw
// goes too (TESTING.md §7).
if (patched) {
  const condition = Dex.mod('gen4').data.Moves.safeguard.condition;
  const tryAddVolatile = condition.onTryAddVolatile;
  condition.onTryAddVolatile = function (status, target, source, effect) {
    if (this.gen === 4 && status.id === 'confusion' && effect?.effectType === 'Item' && effect.isBerry) {
      return null;
    }
    return tryAddVolatile.call(this, status, target, source, effect);
  };
}

// I215 — a Max or G-Max Move's effects on the foes (`self.onHit`: Max Strike's Speed drop, Max
// Phantasm's Defense drop, G-Max Stun Shock's status, G-Max Gold Rush's confusion …) are its
// user's move's: an Infiltrator user's go through the foes' Mist and Safeguard, as its moves' own
// effects do. Showdown hands `boost` / `trySetStatus` / `addVolatile` the move's `self` data as the
// effect, which is no Move and carries no `infiltrates`, so Mist's `onTryBoost` and Safeguard's
// `onSetStatus` / `onTryAddVolatile` stopped them. No cartridge evidence either way; the user
// decided the move's semantics count (issues.md I215). The patch: those handlers let through what
// the active Max Move's Infiltrator user does to a foe. A sleep or a confusion let through draws
// its duration (TESTING.md §7).
if (patched) {
  const seen = new Set();
  for (const mod of ['gen8', 'gen9']) {
    const moves = Dex.mod(mod).data.Moves;
    for (const [id, names] of [['mist', ['onTryBoost']], ['safeguard', ['onSetStatus', 'onTryAddVolatile']]]) {
      const condition = moves[id]?.condition;
      if (!condition || seen.has(condition)) continue;
      seen.add(condition);
      for (const name of names) {
        const handler = condition[name];
        if (!handler) continue;
        condition[name] = function (value, target, source, effect) {
          const move = this.activeMove;
          if (move?.isMax && move.infiltrates && effect !== move && source && this.activePokemon === source &&
            target && !target.isAlly(source)) {
            return;
          }
          return handler.call(this, value, target, source, effect);
        };
      }
    }
  }
}

// I216 — Gen 3 thawing by a Fire move. pokeemerald's `MOVEEND_DEFROST` thaws a target the move
// dealt special damage to (`specialDmg`) when the move's type, its dynamic type included
// (`GET_MOVE_TYPE`), is Fire. Weather Ball in the sun is Fire (`Cmd_setweatherballtype`), and so
// special: it thaws. (Hidden Power's dynamic type carries `F_DYNAMIC_TYPE_IGNORE_PHYSICALITY`: its
// damage counts as Normal, physical, and it never thaws, as Showdown has it.) Showdown's gen3
// `frz.onDamagingHit` reads the dex type, Weather Ball's Normal. The patch: a Fire Weather Ball
// thaws too. The thawed Pokémon draws no thaw roll after (TESTING.md §7).
if (patched) {
  const frz = Dex.mod('gen3').data.Conditions.frz;
  const onDamagingHit = frz.onDamagingHit;
  frz.onDamagingHit = function (damage, target, source, move) {
    if (move.id === 'weatherball' && move.type === 'Fire' && move.category !== 'Status') {
      target.cureStatus();
      return;
    }
    return onDamagingHit.call(this, damage, target, source, move);
  };
}

// I217 — Gen 7+: a Dancer stopped before its dance (its confusion, paralysis …) leaves the battle's
// last move the dance. Showdown writes `battle.lastMove` only as the action ends (`runAction`'s
// `clearActiveMove()`), from the active move; the dancer's `runMove` makes its copy the active move
// (`setActiveMove`) and the block's `clearActiveMove(true)` leaves none, so the dance the action's
// user did use is never recorded, and a later Copycat calls the move before it. Bulbapedia's
// Copycat, Gen 5+: "The last move used is now unaffected if a Pokémon tries to use a move but is
// prevented by a status condition or effect … the last-used move remains what it was before the
// Pokémon's turn" — the dance (issues.md I217). The patch: a dancer's `runMove` (`externalMove`)
// that leaves no active move puts back the one before it. Copycat then calls the dance, and draws
// as it goes (TESTING.md §7).
if (patched) {
  const actions = Object.getPrototypeOf(new Battle({formatid: 'gen9customgame'}).actions);
  const runMove = actions.runMove;
  actions.runMove = function (moveOrMoveName, pokemon, targetLoc, options) {
    const battle = this.battle;
    const {activeMove, activePokemon, activeTarget} = battle;
    const result = runMove.call(this, moveOrMoveName, pokemon, targetLoc, options);
    if (options?.externalMove && activeMove && !battle.activeMove) {
      battle.activeMove = activeMove;
      battle.activePokemon = activePokemon;
      battle.activeTarget = activeTarget;
    }
    return result;
  };
}

// I218 — Gen 3 damage (R-M11-84). Showdown's gen3 `getDamage` / `modifyDamage` chain the stat
// modifiers 4096-based and round once, apply the stages before them, and double Facade, Smelling
// Salt, Revenge, Pursuit, Weather Ball and the moves that hit a hiding or minimized target in the
// power or the final damage. pokeemerald (work/pret: `CalculateBaseDamage`, `Cmd_damagecalc`,
// `Cmd_typecalc`, `ApplyRandomDmgMultiplier`, the scripts' `sDMG_MULTIPLIER`, `Cmd_trydobeatup`,
// `Cmd_stockpiletobasedamage`, `Cmd_trysetfutureattack`, the confusion self-hit) multiplies one
// modifier at a time and floors each: the stats' modifiers before their stages (the type item
// first, then the held items, Thick Fat, Hustle, Plus / Minus — beside any Pokémon in a slot, a
// fainted one no replacement took the place of too (`ABILITY_ON_FIELD2`) —, Guts; Marvel Scale),
// the power's (Mud Sport, Water Sport, the pinch abilities), then burn, the screens (`2 × (d / 3)`
// while the target's side counts two not yet fainted), the spread (the same count), the weather
// and Flash Fire, + 2, and after the critical hit the
// doubling (Bulbapedia's DoubleDmg), Charge and Helping Hand, then STAB and the type chart entry
// by entry, the random factor last. Beat Up adds Helping Hand before its critical hit; Spit Up
// never lands a critical hit nor draws the random factor; Future Sight and Doom Desire store the
// formula's damage of their own type, Helping Hand in (the strike draws the factor, I110). The
// patch lets Showdown run its calculation — events, messages and draws — and returns the
// cartridge's number from the same power callback, critical hit and damage roll; Spit Up's
// critical hit and roll are not drawn (TESTING.md §7).
const GEN3_TYPE_ITEMS = {
  silkscarf: ['Normal', 10], charcoal: ['Fire', 10], mysticwater: ['Water', 10], seaincense: ['Water', 5],
  magnet: ['Electric', 10], miracleseed: ['Grass', 10], nevermeltice: ['Ice', 10], blackbelt: ['Fighting', 10],
  poisonbarb: ['Poison', 10], softsand: ['Ground', 10], sharpbeak: ['Flying', 10], twistedspoon: ['Psychic', 10],
  silverpowder: ['Bug', 10], hardstone: ['Rock', 10], spelltag: ['Ghost', 10], dragonfang: ['Dragon', 10],
  blackglasses: ['Dark', 10], metalcoat: ['Steel', 10],
};
// `IS_TYPE_PHYSICAL`: the types before `TYPE_MYSTERY`.
const GEN3_PHYSICAL = new Set(['Normal', 'Fighting', 'Flying', 'Poison', 'Ground', 'Rock', 'Bug', 'Ghost', 'Steel']);
const GEN3_MINIMIZE_DOUBLED = new Set(['stomp', 'needlearm', 'astonish', 'extrasensory']);
function cartridgeGen3Damage(source, target, move, seen) {
  const tr = Math.trunc;
  const ability = (pokemon, id) => pokemon.hasAbility(id);
  const item = pokemon => (pokemon.ignoringItem() ? '' : pokemon.getItem().id);
  const helpingHand = !!source.volatiles['helpinghand'];
  const variance = damage => (damage ? Math.max(1, tr(damage * (100 - seen.roll) / 100)) : 0);
  const selfHit = seen.selfHit;
  const crit = !selfHit && !seen.foreseen && move.id !== 'spitup' && target.getMoveHitData(move).crit;
  // Beat Up (`Cmd_trydobeatup`): the member's base Attack and level against the target's base
  // Defense at the move's power, + 2, Helping Hand, then the critical hit and the roll.
  if (seen.member) {
    const attack = this.dex.species.get(seen.member.set.species).baseStats.atk;
    const defense = target.species.baseStats.def;
    let damage = tr(tr(attack * move.basePower * tr(tr(seen.member.level * 2 / 5) + 2) / defense) / 50) + 2;
    if (helpingHand) damage = times(damage, 15, 10);
    if (crit) damage *= 2;
    return variance(damage);
  }
  const data = this.dex.moves.get(seen.foreseen || move.id);
  // The confusion self-hit is Pound at 40 against the user itself; a foreseen attack is its own
  // move's type (`futureSightMove`), not the typeless strike Showdown stores.
  const type = selfHit ? 'Normal' : seen.foreseen ? data.type : move.type === '???' ? 'Normal' : move.type;
  const physical = GEN3_PHYSICAL.has(type);
  const weather = selfHit ? '' : this.field.effectiveWeather();
  const defender = selfHit ? source : target;
  // The power, and the doubling the cartridge leaves to the damage (`sDMG_MULTIPLIER`).
  let power = selfHit ? 40 : seen.power;
  let doubled = false;
  if (['smellingsalts', 'revenge', 'pursuit', 'weatherball'].includes(move.id)) {
    doubled = seen.power === data.basePower * 2;
    power = data.basePower;
    // The Smelling Salt script hits a Substitute without its doubling (`jumpifstatus2 … SUBSTITUTE`).
    if (move.id === 'smellingsalts' && target.volatiles['substitute']) doubled = false;
  }
  if (move.id === 'facade' && ['psn', 'tox', 'brn', 'par'].includes(source.status)) doubled = true;
  if (['gust', 'twister'].includes(move.id) && (target.volatiles['fly'] || target.volatiles['bounce'])) doubled = true;
  if (['earthquake', 'magnitude'].includes(move.id) && target.volatiles['dig']) doubled = true;
  if (['surf', 'whirlpool'].includes(move.id) && target.volatiles['dive']) doubled = true;
  if (GEN3_MINIMIZE_DOUBLED.has(move.id) && target.volatiles['minimize']) doubled = true;
  if (selfHit || seen.foreseen || move.id === 'spitup') doubled = false;
  // Spit Up (`Cmd_stockpiletobasedamage`): the formula at the move's own power (100 in
  // `gBattleMoves`; Showdown's data has 0 and a `basePowerCallback`), times the layers after it.
  const layers = move.id === 'spitup' ? tr(seen.power / 100) : 1;
  if (move.id === 'spitup') power = 100;
  const active = this.getAllActive();
  if (type === 'Electric' && active.some(p => p.volatiles['mudsport'])) power = tr(power / 2);
  if (type === 'Fire' && active.some(p => p.volatiles['watersport'])) power = tr(power / 2);
  for (const [id, pinchType] of Object.entries(PINCH)) {
    if (type === pinchType && ability(source, id) && source.hp <= tr(source.maxhp / 3)) power = times(power, 150, 100);
  }
  // The stats: the stored stat through each modifier in turn, floored, before the stage.
  const held = item(source);
  const heldType = GEN3_TYPE_ITEMS[held];
  let attack;
  let defense;
  if (physical) {
    attack = source.storedStats.atk;
    if (ability(source, 'hugepower') || ability(source, 'purepower')) attack *= 2;
    if (heldType?.[0] === type) attack = times(attack, 100 + heldType[1], 100);
    if (held === 'choiceband') attack = times(attack, 150, 100);
    if (held === 'thickclub' && ['Cubone', 'Marowak'].includes(source.species.name)) attack *= 2;
    if (ability(source, 'hustle')) attack = times(attack, 150, 100);
    if (ability(source, 'guts') && source.status) attack = times(attack, 150, 100);
    defense = defender.storedStats.def;
    if (item(defender) === 'metalpowder' && defender.species.name === 'Ditto') defense *= 2;
    if (ability(defender, 'marvelscale') && defender.status) defense = times(defense, 150, 100);
    if (!selfHit && data.selfdestruct === 'always') defense = tr(defense / 2);
  } else {
    attack = source.storedStats.spa;
    if (heldType?.[0] === type) attack = times(attack, 100 + heldType[1], 100);
    if (held === 'souldew' && ['Latios', 'Latias'].includes(source.species.name)) attack = times(attack, 150, 100);
    if (held === 'deepseatooth' && source.species.name === 'Clamperl') attack *= 2;
    if (held === 'lightball' && source.species.name === 'Pikachu') attack *= 2;
    if (ability(defender, 'thickfat') && ['Fire', 'Ice'].includes(type)) attack = tr(attack / 2);
    // Plus / Minus (`ABILITY_ON_FIELD2`): any Pokémon on the field with the other one.
    const field = id => this.getAllActive(true).some(p => p.hasAbility(id));
    if ((ability(source, 'plus') && field('minus')) || (ability(source, 'minus') && field('plus'))) {
      attack = times(attack, 150, 100);
    }
    defense = defender.storedStats.spd;
    const defenderItem = item(defender);
    if (defenderItem === 'souldew' && ['Latios', 'Latias'].includes(defender.species.name)) defense = times(defense, 150, 100);
    if (defenderItem === 'deepseascale' && defender.species.name === 'Clamperl') defense *= 2;
  }
  let attackStage = source.boosts[physical ? 'atk' : 'spa'];
  let defenseStage = defender.boosts[physical ? 'def' : 'spd'];
  if (crit && attackStage < 0) attackStage = 0;
  if (crit && defenseStage > 0) defenseStage = 0;
  const level = tr(tr(source.level * 2 / 5) + 2);
  let damage = tr(tr(staged(attack, attackStage) * power * level / staged(defense, defenseStage)) / 50);
  // Both defenders standing (`CountAliveMonsInBattle(BATTLE_ALIVE_DEF_SIDE) == 2`).
  const pair = !selfHit && this.gameType === 'doubles' && defender.side.active.filter(p => p && !p.fainted).length === 2;
  const screen = !selfHit && !crit && defender.side.getSideCondition(physical ? 'reflect' : 'lightscreen');
  if (physical) {
    if (source.status === 'brn' && !ability(source, 'guts')) damage = tr(damage / 2);
    if (screen) damage = pair ? 2 * tr(damage / 3) : tr(damage / 2);
    if (pair && data.target === 'allAdjacentFoes') damage = tr(damage / 2);
    if (!damage) damage = 1;
  } else {
    if (screen) damage = pair ? 2 * tr(damage / 3) : tr(damage / 2);
    if (pair && data.target === 'allAdjacentFoes') damage = tr(damage / 2);
    if (weather === 'raindance') {
      if (type === 'Fire') damage = tr(damage / 2);
      if (type === 'Water') damage = times(damage, 15, 10);
    }
    if (move.id === 'solarbeam' && ['raindance', 'sandstorm', 'hail'].includes(weather)) damage = tr(damage / 2);
    if (weather === 'sunnyday') {
      if (type === 'Fire') damage = times(damage, 15, 10);
      if (type === 'Water') damage = tr(damage / 2);
    }
    if (source.volatiles['flashfire'] && type === 'Fire') damage = times(damage, 15, 10);
  }
  damage += 2;
  if (selfHit) return variance(damage);
  // `Cmd_trysetfutureattack`: the formula's damage and Helping Hand, stored; the strike draws.
  if (seen.foreseen) return helpingHand ? times(damage, 15, 10) : damage;
  damage *= (crit ? 2 : 1) * (doubled ? 2 : 1) * layers;
  // Charge reads the move's data type (`gBattleMoves[gCurrentMove].type`).
  if (move.id !== 'spitup' && source.volatiles['charge'] && data.type === 'Electric') damage *= 2;
  if (helpingHand) damage = times(damage, 15, 10);
  // `Cmd_typecalc` (not Struggle): STAB, then the chart entry by entry, 0 -> 1 but for an immunity.
  if (move.type !== '???') {
    if (source.hasType(type)) damage = times(damage, 15, 10);
    const order = CHART_ORDER[type] ?? [];
    const types = [...new Set(target.getTypes())]
      .map(defending => [defending, this.dex.getEffectiveness(type, defending)])
      .filter(([, effect]) => effect)
      .sort(([a], [b]) => order.indexOf(a) - order.indexOf(b));
    for (const [, effect] of types) damage = Math.max(1, tr(damage * (effect > 0 ? 20 : 5) / 10));
  }
  return move.id === 'spitup' ? damage : variance(damage);
}
function cartridgeGen3GetDamage(source, target, move, suppressMessages = false) {
  const battle = this.battle;
  const selfHit = typeof move === 'number';
  const foreseen = !selfHit && move.flags?.futuremove && move.noDamageVariance
    ? move.name.toLowerCase().replace(/[^a-z0-9]/g, '') : '';
  const seen = {power: undefined, roll: undefined, member: null, selfHit, foreseen};
  if (!selfHit && move.id === 'beatup' && move.allies?.length) [seen.member] = move.allies;
  const spitUp = !selfHit && move.id === 'spitup';
  if (spitUp) move.willCrit = false;
  const {runEvent, randomizer} = battle;
  battle.runEvent = function (eventid, ...rest) {
    if (eventid === 'BasePower' && seen.power === undefined) [, , , seen.power] = rest;
    return runEvent.call(this, eventid, ...rest);
  };
  battle.randomizer = function (damage) {
    if (spitUp || foreseen) {
      seen.roll = 0;
      return damage;
    }
    seen.roll = this.random(16);
    return this.trunc(this.trunc(damage * (100 - seen.roll)) / 100);
  };
  let result;
  try {
    result = Object.getPrototypeOf(this).getDamage.call(this, source, target, move, suppressMessages);
  } finally {
    battle.runEvent = runEvent;
    battle.randomizer = randomizer;
  }
  if (seen.roll === undefined || typeof result !== 'number') return result;
  if (selfHit) {
    seen.power = move;
    return cartridgeGen3Damage.call(battle, source, source, battle.dex.getActiveMove('pound'), seen);
  }
  return cartridgeGen3Damage.call(battle, source, target, move, seen);
}
for (const [id, format] of patched ? Object.entries(Dex.data.Rulesets) : []) {
  if (!/^gen3/.test(id)) continue;
  format.actions = {...format.actions, getDamage: cartridgeGen3GetDamage};
  const cached = Dex.formats.get(id);
  cached.actions = {...cached.actions, getDamage: cartridgeGen3GetDamage};
}

// I219 — Gen 2 a Pokémon Roar or Whirlwind drags in meets the Spikes within the move. The gen2
// `moveHit` drags in at once (`dragIn`), but `switchIn` queues the entrant's `runSwitch` (its Spikes)
// as an action of its own for any generation before 5, after the whole dragging action — the
// user's `onAfterMoveSelf` residual (poison, burn, Leech Seed) first. pokecrystal's
// `BattleCommand_ForceSwitch` calls `SpikesDamage` itself, inside the move, and the battle core runs
// `ResidualDamage` only once the move is over (R-M11-97). The patch runs a Gen 2 drag's queued
// `runSwitch` at once: a speed tie its `speedSort` breaks is drawn there.
function cartridgeGen2SwitchIn(pokemon, pos, sourceEffect = null, isDrag) {
  const result = Object.getPrototypeOf(this).switchIn.call(this, pokemon, pos, sourceEffect, isDrag);
  if (result && isDrag) {
    const queue = this.battle.queue;
    const index = queue.list.findIndex(action => action.choice === 'runSwitch' && action.pokemon === pokemon);
    if (index >= 0) {
      queue.list.splice(index, 1);
      this.runSwitch(pokemon);
    }
  }
  return result;
}
for (const [id, format] of patched ? Object.entries(Dex.data.Rulesets) : []) {
  if (!/^gen2/.test(id)) continue;
  format.actions = {...format.actions, switchIn: cartridgeGen2SwitchIn};
  const cached = Dex.formats.get(id);
  cached.actions = {...cached.actions, switchIn: cartridgeGen2SwitchIn};
}

// I220 — Gen 2 Berserk Gene at the top of the turn. pokecrystal's `BattleTurn` calls
// `HandleBerserkGene` before every turn's choices, the player's Pokémon then the enemy's — its only
// call (`engine/battle/core.asm`): a holder that comes in mid-turn (a switch, Baton Pass, a drag)
// uses the gene as the next turn starts, and one that faints first never does. Showdown's
// `onUpdate` uses it at the first `Update` after the holder is in. The patch moves the gene's
// handler to the start of `endTurn` — p1's active Pokémon, then p2's — run as the item's own event
// (the confusion's 256 turns ask for it as the source). No draw of its own.
if (patched) {
  const gene = Dex.mod('gen2').data.Items.berserkgene;
  gene.onCartridgeTurnStart = gene.onUpdate;
  delete gene.onUpdate;
  const endTurn = Battle.prototype.endTurn;
  Battle.prototype.endTurn = function (...args) {
    if (this.gen === 2) {
      for (const side of this.sides) {
        for (const pokemon of side.active) {
          if (pokemon?.hp && pokemon.item === 'berserkgene') {
            this.singleEvent('CartridgeTurnStart', pokemon.getItem(), pokemon.itemState, pokemon);
          }
        }
      }
    }
    return endTurn.apply(this, args);
  };
}

// I221 — Gen 1 a damage the type chart takes to 0 makes the move miss. pokered's
// `AdjustDamageForMoveType` multiplies the damage by each matching type pair and, "if damage is 0,
// make the move miss" (`wMoveMissed`): a 2 or 3 at 0.25× misses — no critical-hit or effectiveness
// message, nothing to the body or its Substitute, no drain, recoil or secondary, and a crash move
// crashes. Showdown's gen1 `getDamage` returns the 0 ("the move is made to miss", its comment says)
// and `moveHit` deals it as a hit; a Substitute's `onTryHit` drains 1 off it. The patch takes back
// the lines the computation wrote and ends the move as an accuracy miss does: `-miss`, no last
// damage, and `null` to the caller (`moveHit` and the Substitute's `onTryHit` both stop on it).
// A secondary the 0 would have rolled draws nothing (TESTING.md §7).
if (patched) {
  const actions = Dex.mod('gen1').data.Scripts.actions;
  const getDamage = actions.getDamage;
  actions.getDamage = function (source, target, move, suppressMessages) {
    const formula = typeof move === 'object' && move.category !== 'Status' && move.basePower > 0 &&
      !move.ohko && !move.damageCallback && move.damage == null &&
      !(move.self?.volatileStatus === 'partialtrappinglock' && move.type === 'Normal' &&
        target.hasType('Ghost')) &&
      !(source.volatiles['partialtrappinglock'] &&
        target === source.volatiles['partialtrappinglock'].locked);
    const start = this.battle.log.length;
    const damage = getDamage.call(this, source, target, move, suppressMessages);
    if (!formula || damage !== 0) return damage;
    this.battle.log.splice(start);
    this.battle.attrLastMove('[miss]');
    this.battle.add('-miss', source);
    this.battle.lastDamage = 0;
    return null;
  };
}
