mod ability_change;
mod acupressure;
mod ally_switch;
mod bide;
mod calls;
mod copy_move_slots;
mod corrosive_gas;
mod curse;
mod disable;
mod eating;
mod faint_bonds;
mod field;
mod field_boosts;
mod follow_me;
mod grounding;
mod guard;
mod hazards;
mod heal_and_cure;
mod heal_bell;
mod helping_hand;
mod hp_cost_boosts;
mod identify;
mod ingrain;
mod instruct;
mod leech_seed;
mod lock_on;
mod magic_coat;
mod me_first;
mod no_effect;
mod pain_split;
mod protect;
mod queue;
mod refresh;
mod rest;
mod screens;
mod self_boosts;
mod snatch;
mod spikes;
mod spite;
mod stage_swaps;
mod stat_swaps;
mod stealth_rock;
mod stockpile;
mod substitute;
mod switching;
mod tailwind;
mod terrain;
mod transform;
mod trapping;
mod trick;
mod trick_room;
mod types;

use crate::dex::{ChargeSkip, TwoTurnSpec};
use crate::engine::{BasePowerContext, BattleContext, ConditionApplyError};
use crate::ids::{MoveId, PokemonId};
use crate::rules::{DamageBonusStage, Fraction, ResolvedTarget, SemiInvulnerableKind};

use super::BehaviorComponent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BasePower(pub u16);

/// Showdown `onBasePower` / `onAllyBasePower`: a participant in the BasePower chain. Given the
/// base power and the chain so far, it returns its 4096-based modifier (or `None`); the chain is
/// combined with `chainModify` and applied once (DESIGN.md §22.5).
pub type BasePowerChainHandler = for<'a> fn(&BasePowerContext<'a>, BasePower, u32) -> Option<u32>;
/// A custom primary effect's code (Protect, Substitute, Stealth Rock …): the move's effect on
/// the targets that passed the hit gate. The only move behavior that is code, not data.
pub type ExecuteMoveHandler = for<'a> fn(
    &mut BattleContext<'a>,
    PokemonId,
    &[ResolvedTarget],
) -> Result<(), ConditionApplyError>;

/// A custom primary's own `onTry` / `onPrepareHit` on its user: asked by the Move Shell before
/// the `PrepareHit` event's handlers (Protean, Libero), so a move it fails never reaches them
/// (Showdown `trySpreadMoveHit`: `singleEvent('Try')`, the move's `PrepareHit`, then
/// `runEvent('PrepareHit')`).
pub type TryPrimaryHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId) -> Result<(), ConditionApplyError>;

/// A Z-Move's own terrain (Genesis Supernova's `secondary.self.onHit`, DESIGN.md §44.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZTerrain {
    Psychic,
}

/// A Max Move's own `self.onHit` (DESIGN.md §44.5), which runs only while its user is Dynamaxed:
/// the side's boost, the foes' drop, a weather or a terrain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxMoveEffect {
    /// Max Airstream, Max Knuckle, Max Ooze, Max Steelspike, Max Quake: the user and its allies.
    TeamBoost(&'static [(crate::state::Stat, i8)]),
    /// Max Strike, Max Darkness, Max Flutterby, Max Phantasm, Max Wyrmwind: the user's foes.
    FoeDrop(&'static [(crate::state::Stat, i8)]),
    Weather(crate::state::WeatherKind),
    Terrain(crate::state::TerrainKind),
}

/// A move's behavior as data (DESIGN.md §22.6-22.8, §25.1). Every field is a component standing for
/// the Showdown callbacks `COMPONENTS` lists; the dex keeps, per generation, only the components
/// whose callbacks that generation declares (`ResolvedMove.behavior`), and the engine reads nothing
/// else. What a move does that is not data is a custom primary (`custom`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MoveBehavior {
    /// The move charges for a turn (DESIGN.md §15.5); the Shell owns the two-turn structure,
    /// the spec only says whether the charge can be skipped and where the user hides.
    pub two_turn: Option<TwoTurnSpec>,
    /// Showdown `onTry`: checked after targeting, before the hit steps; failing makes the move
    /// fail (`-fail`) with its PP spent (Fake Out after the first action).
    pub try_use: Option<TryUse>,
    /// Belch, Stuff Cheeks (`onDisableMove`): the move is off the request while its `try_use`
    /// does not hold (`ChoiceError::MoveDisabledByItself`).
    pub self_disables: bool,
    /// The move never misses (and reaches a semi-invulnerable target) when its user has this
    /// type, from the generation `ConditionRules::typed_sure_hit` says (Toxic: Poison).
    pub sure_hit_user_type: Option<crate::dex::PokemonType>,
    /// Damage that replaces the formula (Super Fang, Ruination: half the target's HP); the
    /// dex merges it with the snapshot's `damage` key into `ResolvedMove.fixed_damage`.
    /// Showdown: `damageCallback`.
    pub damage_override: Option<crate::dex::FixedDamage>,
    /// Showdown `basePowerCallback` as data (DESIGN.md §22.7).
    pub power_formula: Option<PowerFormula>,
    /// Showdown `onBasePower` as data: a BasePower chain participant when its condition holds.
    pub power_boost: Option<PowerBoost>,
    /// Showdown `onModifyMove` changing the accuracy with the weather (Thunder, Blizzard).
    pub weather_accuracy: Option<WeatherAccuracy>,
    /// Showdown `onModifyMove` doubling the stat changes in sun (Growth).
    pub sun_doubles_boosts: bool,
    /// Showdown `onHit` healing by the weather (Moonlight, Shore Up).
    pub weather_heal: Option<WeatherHeal>,
    /// Showdown `onTryHit` refusing the move (`-fail`) unless this holds, as the hit lands —
    /// after the accuracy check, before a Substitute (Yawn, Gen 4 Embargo).
    pub target_rule: Option<TargetRule>,
    /// Psycho Shift (`onTryHit`, `self.onHit`): the user's status goes to the target — the move
    /// fails when the user has none — and once it took there the user is cured.
    pub status_transfer: bool,
    /// Purify's `onHit`: the target's status is cured, and only then the user heals this share
    /// of its max HP (rounded up).
    pub cure_target: Option<Fraction>,
    /// Gen 1 Recover / Soft-Boiled (`onHit` in place of the `heal` key): the user heals this
    /// share of its max HP, a `StatusMoveEffect::Heal` — failing, as the gen1 mod has it, when
    /// `ConditionRules::heal_fails_255_short` says.
    pub on_hit_heal: Option<Fraction>,
    /// Heal Pulse / Floral Healing (`onHit`): the target heals half its max HP (rounded up),
    /// more when `TargetHeal::more_when` holds.
    pub target_heal: Option<TargetHeal>,
    /// Heal Block's `durationCallback`: a Heal Block this move inflicts lasts this many turns
    /// instead of 5 (Psychic Noise: 2). See `heal_block_turns`.
    pub heal_block_turns: Option<u8>,
    /// Sparkling Aria's `onAfterMove`: once the move is over, each target it hit is cured of a
    /// burn if its secondary reached it (the `SparklingAria` mark) or the move hit several
    /// targets — unless the user is down or Sheer Force took the secondary.
    pub cures_burn_after_move: bool,
    /// Showdown `onTryHit` that only fails against a Dynamax target (Low Kick, Heavy Slam):
    /// the engine never enables Dynamax (`SpecialMechanics::dynamax`), so no target is one.
    pub dynamax_only_try_hit: bool,
    /// Facade: from the generation `DamageRules::move_can_ignore_burn` says, a burned user's
    /// damage is not halved (Showdown's `move.id !== 'facade'` in `modifyDamage`: no callback).
    pub ignores_burn_drop: bool,
    /// Chloroblast: recoil of this fraction of the user's max HP whenever the move dealt
    /// damage; the dex merges it into `ResolvedMove.recoil` (Showdown's `move.id ===
    /// 'chloroblast'` in `spreadMoveHit` / `calcRecoilDamage`: no callback).
    pub max_hp_recoil: Option<Fraction>,
    /// Hydro Steam: sun boosts it like a Fire move instead of weakening it, when the user is in
    /// the sun (Showdown's `move.id === 'hydrosteam'` in `sunnyday`'s `onWeatherModifyDamage`:
    /// no callback).
    pub sun_boosted: bool,
    /// Struggle from Gen 2: typeless (Showdown `onModifyMove`: `move.type = '???'`), so neutral
    /// against every type and never STAB.
    pub typeless: bool,
    /// Showdown `onEffectiveness`: the move's own effectiveness against one defending type.
    pub effectiveness: Option<TypeEffectiveness>,
    /// Showdown `onDamage` (False Swipe, Hold Back, priority -20): a hit that would knock the
    /// target out leaves it at 1 HP.
    pub spares_target: bool,
    /// Showdown `onAfterMove` with `mindBlownRecoil` (Mind Blown, Steel Beam): once the move is
    /// over — hit, missed or failed — the user loses this fraction of its max HP, rounded.
    pub self_damage_after_move: Option<Fraction>,
    /// Showdown `onMoveFail` with `hasCrashDamage` (High Jump Kick, Jump Kick, Axe Kick,
    /// Supercell Slam): hitting no target crashes the user (`HitRules::crash`).
    pub crash_damage: bool,
    /// Showdown `onHit` curing the target of this status (Wake-Up Slap: sleep, Smelling Salts:
    /// paralysis) once the hit lands.
    pub cures_target_status: Option<crate::state::NonVolatileStatus>,
    /// Fling's `onHit` from its item's `fling.effect` (White Herb, Mental Herb), set as the
    /// move is prepared (`engine::prepared_move`, DESIGN.md R-M8-86).
    pub flung_on_hit: Option<crate::effects::items::FlingEffect>,
    /// Clear Smog's `onHit`: the target's stat stages go back to 0 once the hit lands
    /// (`BattleContext::clear_stages`).
    pub clears_target_stages: bool,
    /// Spit Up's `onAfterMove`: once the move is over (hit, missed or failed after BeforeMove),
    /// the user's Stockpile ends (`BattleContext::end_stockpile`).
    pub ends_stockpile: bool,
    /// Trump Card: its PP takes no PP Ups (Showdown's `move.id === 'trumpcard'` in the
    /// `Pokemon` constructor: no callback).
    pub no_pp_boosts: bool,
    /// Showdown `onTryImmunity` as data.
    pub try_immunity: Option<TryImmunity>,
    /// Gen 2 Swagger's `onTryHit`, after the accuracy roll: the move misses a target whose
    /// Attack is maxed out (stage +6, or 999 with its stages).
    pub misses_maxed_attack: bool,
    /// Swagger behind a Gen 2 Substitute: its stat stages go through, its confusion is dropped
    /// (Showdown's gen2 `substitute` names Swagger by id: no callback;
    /// `ConditionRules::substitute_gen2_quirks`).
    pub boosts_through_gen2_substitute: bool,
    /// Mean Look, Spider Web: a Gen 2 Substitute lets them through (Showdown's gen2
    /// `substitute` stops a status move only for a status, stages, confusion or its named few;
    /// pokecrystal's `BattleCommand_ArenaTrap` checks no Substitute, R-M8-115).
    pub through_gen2_substitute: bool,
    /// Fell Stinger's `onAfterMoveSecondarySelf`: the user raises this stat when the move's
    /// target is down after the move (by `HitRules::knock_out_boost_stages`).
    pub knock_out_boost: Option<crate::state::Stat>,
    /// Order Up's `onAfterMoveSecondarySelf` (DESIGN.md §43.4): the commanded Dondozo raises a
    /// stat by the forme of the Tatsugiri in its mouth.
    pub order_up: bool,
    /// Relic Song's `onAfterMoveSecondarySelf` (DESIGN.md §43.4): a user of this species of its
    /// own, not transformed, switches between its two formes once the move did something —
    /// unless Sheer Force kept the event from running.
    pub forme_toggle: Option<(crate::ids::SpeciesId, crate::ids::SpeciesId)>,
    /// A secondary's own `onHit` as data: the effect it applies, on its condition (Burning
    /// Jealousy, Alluring Voice).
    pub secondary_on_hit: Option<SecondaryOnHit>,
    /// Gen 4-5 Chatter's `onModifyMove`: the move's secondaries need the user to be this
    /// species (Chatot).
    pub secondaries_need_user_species: Option<crate::ids::SpeciesId>,
    /// Gen 2 Tri Attack's `onHit` (`move.statusRoll`): its random status is drawn on every hit
    /// before the secondary's chance, and a frozen target thaws when the draw is a burn.
    pub status_pick_before_chance: bool,
    /// A power drawn as the move starts (Showdown `onModifyMove`, announced at
    /// `onUseMoveMessage`): Magnitude.
    pub random_power: Option<RandomPower>,
    /// The move sets its own type as it starts (Showdown `onModifyType`, and Weather Ball's
    /// `onModifyMove` doubling its power): `engine::modified_move`.
    pub own_type: Option<OwnType>,
    /// Showdown `onBasePower` with a chance (Fickle Beam: 3 / 10 to double, announced with
    /// `-activate|user|move: Fickle Beam`), rolled after the critical-hit check.
    pub power_chance: Option<PowerChance>,
    /// A callback that only prints a line and changes nothing in the battle.
    pub announcement: Option<Announcement>,
    /// Ivy Cudgel's animation-only `onPrepareHit` (`TypeAnimation`).
    pub type_animation: Option<TypeAnimation>,
    /// Echoed Voice's `onTryMove` and `condition`: the move starts or renews the Echoed Voice
    /// field condition (`FieldState::echo`) before its hit steps.
    pub echoes: bool,
    /// Round's `onTry`: the next queued action using the same move, either side's, moves to the
    /// front of the queue (Showdown `queue.prioritizeAction`), marked as prioritized by it.
    pub prioritizes_same_move: bool,
    /// Beat Up's `onModifyMove` (and its Gen 3-4 `condition`): one hit per party member taking
    /// part (`engine::beat_up_members`), as `DamageRules::beat_up` says.
    pub beat_up: bool,
    /// Shell Side Arm's `onModifyMove`: physical, with contact, when its physical estimate
    /// against the target beats the special one (`engine::physical_by_estimate`); a tie flips a
    /// coin.
    pub category_by_estimate: bool,
    /// Photon Geyser's `onModifyMove`: physical when the user's staged Attack beats its staged
    /// Special Attack (`engine::modified_move`).
    pub category_by_stats: bool,
    /// Tera Blast (DESIGN.md §44.6): while its user is Terastallized the move takes the Tera
    /// type, is physical when the user's Attack (stages, no modifiers) beats its Special
    /// Attack, and — Stellar — has 100 power and drops the user's Attack and Special Attack by
    /// one. Its type is not changed by a Normal-type ability (`onModifyType`'s Tera Blast
    /// exception).
    pub tera_blast: bool,
    /// Tera Starstorm: for a Terapagos-Stellar its type is Stellar, it aims at all adjacent
    /// foes, and (Terastallized) it is physical when the user's Attack beats its Special
    /// Attack.
    pub tera_starstorm: bool,
    /// Gen 2 Present (Showdown's gen2 `getDamage`, `move.id === 'present'`: no callback): the
    /// cartridge's glitch reads Attack 10, the attacker's last type's index as the Defense and
    /// the defender's as the level (`-hint`). Only the Gen 2 formula reads it.
    pub type_index_damage: bool,
    /// Pollen Puff (`onTryHit`, `onHit`, `onTryMove`): aimed at an ally, it has no power and
    /// heals this fraction of the ally's max HP (rounded down) instead. Its `onTryMove` stops it
    /// when its user is under Heal Block.
    pub ally_heal: Option<Fraction>,
    /// Brick Break, Psychic Fangs, Raging Bull (`onTryHit`, as the hit lands, before a
    /// Substitute can take it): the screens come down (`BattleContext::break_screens`).
    pub breaks_screens: bool,
    /// Grassy Glide (`onModifyPriority`): +1 priority while this terrain is up and the user is
    /// grounded.
    pub terrain_priority: Option<crate::state::TerrainKind>,
    /// Expanding Force (`onModifyMove`): in this terrain, a grounded user's move hits all
    /// adjacent foes (`engine::modified_move`, the targets resolved again).
    pub terrain_spread: Option<crate::state::TerrainKind>,
    /// Secret Power (`onModifyMove`): in a terrain its secondary is the terrain's (30 %
    /// paralysis, sleep, −1 Special Attack, −1 Speed); with none, its own.
    pub terrain_secondaries: bool,
    /// Rapid Spin, Mortal Spin, Stone Axe, Ceaseless Edge (`onAfterHit`, `onAfterSubDamage`):
    /// after a hit that reached the target's HP and its `DamagingHit` reactions, or as a
    /// Substitute takes the hit — while the user stands and Sheer Force has not dropped the
    /// move's secondaries (`!move.hasSheerForce`).
    pub after_hit: Option<AfterHitEffect>,
    /// Knock Off, Thief, Covet (`onAfterHit`, not `onAfterSubDamage`): the item a hit on the
    /// target's body takes (`BattleContext::knock_off`, `steal_item`).
    pub takes_item: Option<ItemTake>,
    /// Incinerate (`onHit` on each body it struck, before the secondaries): the target's berry
    /// — Gen 6+ its gem too — burns (`BattleContext::burn_item`).
    pub burns_item: bool,
    /// Bug Bite, Pluck (`onHit` on the body it struck, before the secondaries): the user takes
    /// the target's berry and eats it (`BattleContext::steal_and_eat_berry`).
    pub steals_berry: bool,
    /// Fling (`onPrepareHit`, its `condition`): the user's item, if it can throw it
    /// (`engine::fling_item`), sets the power and what the hit does — a berry the target eats,
    /// a status or a flinch (`ItemBehavior::fling_effect`) as a secondary — and the user loses
    /// it at the next `Update` (`VolatileCondition::Fling`); the move fails otherwise.
    pub fling: bool,
    /// Natural Gift's `onPrepareHit`: the user's berry (`engine::natural_gift_berry`) sets the
    /// power and is used up — not eaten; the move fails without one. Its type is `own_type`.
    pub natural_gift: bool,
    /// Gen 2-4 Rapid Spin's `self.onHit`: as the move's self effects land on each hit (before
    /// its secondaries and the target's reactions, a Substitute's hit too), the user's side is
    /// cleared as `AfterHitEffect::ClearHazards` does.
    pub self_clears_hazards: bool,
    /// Burn Up, Double Shock: the move's `onTryMove` stops it (`-fail`, neither failed nor
    /// succeeded) for a user without this type; its `self.onHit`, as Rapid Spin's lands on each
    /// hit, turns this type of the user's into `???` (`setType`).
    pub burns_up: Option<crate::dex::PokemonType>,
    /// Plasma Fists (`pseudoWeather: 'iondeluge'` on a damaging move, no callback): each hit on
    /// the target's body starts Ion Deluge (`runMoveEffects`, before the secondaries); the dex
    /// takes the key as consumed.
    pub starts_ion_deluge: bool,
    /// The move's own `onHit` on the target's body (before its secondaries) that is also its
    /// `onAfterSubDamage`, when a Substitute takes the hit (Steel Roller, Core Enforcer, Flame
    /// Burst).
    pub hit_or_sub: Option<HitOrSubEffect>,
    /// Genesis Supernova (DESIGN.md §44.4): the terrain this Z-Move lays on the field after its
    /// hit (`secondary.self.onHit`'s `field.setTerrain`).
    pub z_terrain: Option<ZTerrain>,
    /// A Max Move's `self.onHit` (DESIGN.md §44.5), run only while the user is Dynamaxed.
    pub max_move_effect: Option<MaxMoveEffect>,
    /// Chilly Reception, Focus Punch, Beak Blast, Shell Trap (`priorityChargeCallback`; Gen 3-4
    /// Focus Punch `beforeTurnCallback`): chosen, the move puts this volatile on its user as the
    /// turn begins (DESIGN.md §15.8).
    pub turn_start_volatile: Option<crate::state::VolatileConditionKind>,
    /// Gen 5+ Focus Punch (`beforeMoveCallback`): a user that lost its focus does not move — no
    /// PP spent, the move failed (`cant … Focus Punch`).
    pub lost_focus_stops: bool,
    /// Gen 3-4 Focus Punch (`onTry`): a user that lost its focus stops after spending the PP,
    /// neither succeeding nor failing.
    pub lost_focus_fails: bool,
    /// Beak Blast (`onAfterMove`): the user's turn-start volatile ends once the move is over.
    pub ends_turn_start_volatile: bool,
    /// Shell Trap (`onTryMove`): the move only goes ahead once a foe's physical hit armed it
    /// (`cant … Shell Trap`, neither succeeding nor failing).
    pub needs_armed_trap: bool,
    /// Counter, Mirror Coat (`beforeTurnCallback`, their volatile's `onRedirectTarget`), Metal
    /// Burst, Comeuppance (`onModifyTarget`): the move goes where the attacker it answers stood.
    pub strikes_back_at: Option<crate::dex::Retaliation>,
    /// Explosion, Self-Destruct (Showdown names them by id in `getDamage`, no callback): in the
    /// generations `SelfFaintRules::explosion_halves_defense` says, the target's Defense is
    /// halved.
    pub halves_defense: bool,
    /// Explosion, Self-Destruct, Misty Explosion, Mind Blown (Damp's `onAnyTryMove` names them
    /// by id, no callback of theirs): an active Damp stops the move before it starts.
    pub stopped_by_damp: bool,
    /// Thousand Waves, Jaw Lock (`onHit` on the target's body, before its secondaries): the
    /// target is trapped, with Jaw Lock the user too (DESIGN.md §30.8).
    pub hit_traps: Option<HitTrap>,
    /// Gen 1 Bind / Wrap / Clamp / Fire Spin (`self.volatileStatus: 'partialtrappinglock'`,
    /// `onTryMove`): the hit locks the user into the move, and the move ends its target's
    /// recharge turn even if it misses (DESIGN.md §30.9).
    pub binding_lock: bool,
    /// Pursuit (`beforeTurnCallback`): chosen, it marks the foes (Gen 2-3 the one it chose) as
    /// the turn begins; one of them switching out is struck first (DESIGN.md §15.11).
    pub marks_pursuit: bool,
    /// Gen 3+ Pursuit (`onModifyMove`): it never misses a Pokémon switching out.
    pub sure_hit_on_switching: bool,
    /// Rollout, Ice Ball (`onModifyMove`, `onAfterMove`, their `condition`): the move starts a
    /// chain that locks its user in until it misses or has hit five times (DESIGN.md §15.12).
    /// `onAfterMove` stores the doublings Gen 7 Disguise held back (`rolloutstorage`, issues.md
    /// I212); used on a storage, the move starts no chain.
    pub rolling: bool,
    /// Thrash, Petal Dance, Outrage, Raging Fury (`self.volatileStatus: 'lockedmove'`): the user
    /// is locked into the move for a few turns and confused when they end, as the generation
    /// counts them (`RampageRules`, DESIGN.md §15.12).
    pub rampage: bool,
    /// Gen 2 Thrash and its kind's `onMoveFail` (`addVolatile('lockedmove')` on a use that came
    /// to nothing): the lock does not wait for a hit. On the cartridge it starts as the move
    /// starts, hit or miss (`RampageCounter::Gen2Uses`, pokecrystal `BattleCommand_Rampage`
    /// ahead of `checkhit`), which this stands for.
    pub rampage_lock_at_start: bool,
    /// Gen 1-2 charging moves' `onPrepareHit` (`source.status !== 'slp'`): released while the
    /// user is asleep — only Sleep Talk gets one that far — the move fails (`-fail`).
    pub charge_fails_asleep: bool,
    /// Future Sight, Doom Desire (`onTry`, DESIGN.md §15.22): the use foresees an attack on the
    /// target's slot (`futuremove`) that strikes two turns later; it fails while one waits
    /// there.
    pub future_attack: bool,
    /// Sky Drop (DESIGN.md §15.23): the first turn carries the target up (`onTryHit`, sure to
    /// hit and without contact: `onModifyMove`), the next drops it (`onHit`) — or the drop fails
    /// (`onMoveFail`); the target must stand (`onTry`). Its `condition` is the `skydrop`
    /// volatile (`BattleContext::{lift, end_sky_drop}`).
    pub sky_drop: bool,
    /// Uproar (`self.volatileStatus: 'uproar'`, its `condition`, `onTryHit`): the user makes an
    /// uproar for a few turns, locked into the move, and no one falls asleep meanwhile; who it
    /// wakes, and when, is the generation's (`UproarRules`, DESIGN.md §15.12).
    pub uproar: bool,
    /// Rage (`self.volatileStatus: 'rage'` and its `condition`): a hit leaves the user raging,
    /// so that the hits it takes build its Attack — Gen 2 a count that multiplies Rage's damage —
    /// until the generation ends it (`RageRules`, DESIGN.md §15.19) — Gen 1's never: a lock (its
    /// condition's `onLockMove`).
    pub rage: bool,
    /// Glaive Rush (`self.volatileStatus: 'glaiverush'` and its `condition`: `onAccuracy`,
    /// `onSourceModifyDamage`, `onBeforeMove`): a hit leaves its user open — every move at it
    /// hits and deals double damage — until its next move attempt (DESIGN.md §15.20).
    pub glaive_rush: bool,
    /// Gen 3-4 Imprison (the gen4 mod's `onTryHit`): it fails unless a foe knows one of its
    /// user's moves.
    pub imprison_needs_shared_move: bool,
    /// Fire / Grass / Water Pledge (DESIGN.md §42.9): two allies' different Pledges in one turn
    /// combine — the second waits (`onPrepareHit` → `null`), the first runs at 150 power with the
    /// pair's type and side condition (`basePowerCallback`, `onModifyMove`).
    pub pledge_combo: bool,
}

/// Who a damaging move's hit traps (`MoveBehavior::hit_traps`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTrap {
    /// Thousand Waves: the target, by the user.
    Target,
    /// Jaw Lock: the user by the target, then the target by the user.
    Mutual,
}

/// What a hit on the body does with the target's item (`MoveBehavior::takes_item`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemTake {
    /// Knock Off: the item is gone.
    KnockOff,
    /// Thief, Covet: the user takes it.
    Steal,
}

/// What a move does after its hit (`MoveBehavior::after_hit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterHitEffect {
    /// Rapid Spin, Mortal Spin: the user's Leech Seed, its side's entry hazards and its bind
    /// end (`BattleContext::spin_away`).
    ClearHazards,
    /// Stone Axe, Ceaseless Edge: this entry hazard is laid on the user's foes' side (Stealth
    /// Rock, a layer of Spikes).
    LayHazard(crate::state::SideConditionKind),
    /// Ice Spinner: the terrain ends (`BattleContext::clear_terrain`).
    ClearTerrain,
}

/// Heal Pulse / Floral Healing's heal of their target (`StatusMoveEffect::TargetHeal`): half its
/// max HP rounded up, or `modify(maxhp, modifier)` (4096-based) when `more_when` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetHeal {
    pub more_when: TargetHealWhen,
    pub modifier: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetHealWhen {
    /// Heal Pulse: the user has this ability (`hasAbility('megalauncher')`: 0.75).
    UserAbility(crate::ids::AbilityId),
    /// Floral Healing: this terrain is up (`field.isTerrain`, grounded or not: 0.667).
    Terrain(crate::state::TerrainKind),
}

/// Showdown callbacks that only announce something: the engine records the fact, it has nothing
/// to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Announcement {
    /// Gen 1-4 Pay Day's `onHit`: `-fieldactivate|move: Pay Day` (the coins are outside the
    /// battle).
    Coins,
    /// Poltergeist's `onTryHit`: `-activate|target|move: Poltergeist|<the target's item>`.
    TargetItem,
    /// Shell Side Arm's `onPrepareHit` (the animation), `onHit` and `onAfterSubDamage`
    /// (`-hint|<category> Shell Side Arm`).
    Category,
}

/// Ivy Cudgel's `onPrepareHit` is its animation only: `attrLastMove('[anim] Ivy Cudgel
/// <type>')` tells the client which type it turned out as, and no frame, decision or damage
/// reads it. The marker keeps the callback accounted for (DESIGN.md §43.4).
pub type TypeAnimation = bool;

/// A BasePower participant that applies by chance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerChance {
    pub numerator: u16,
    pub denominator: u16,
    /// 4096-based.
    pub modifier: u32,
}

/// How a move sets its own type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnType {
    /// Weather Ball: Fire in sun, Water in rain, Rock in a sandstorm, Ice in hail or snow,
    /// with its power doubled under any of them.
    Weather,
    /// Revelation Dance: the user's first type.
    UserFirstType,
    /// Raging Bull: the type its user's form gives it (Tauros-Paldea).
    UserSpecies(&'static [(crate::ids::SpeciesId, crate::dex::PokemonType)]),
    /// Terrain Pulse: the terrain's type, its power doubled, for a grounded user in one.
    Terrain,
    /// Hidden Power: the user's Hidden Power type (`PokemonState::hidden_power`).
    HiddenPower,
    /// Judgment, Multi-Attack, Techno Blast: the type the held Plate / Memory / Drive gives
    /// (`engine::plate_type` / `memory_type` / `drive_type`; DESIGN.md §43.4). The move's own
    /// type while it holds none.
    HeldItemType(ItemGift),
    /// Natural Gift: the type of the user's berry (`engine::natural_gift_berry`); Normal
    /// while it has none it can use.
    NaturalGift,
}

impl OwnType {
    /// Whether the type-changing abilities leave the move alone (Showdown's `noModifyType`: the
    /// `-ate` abilities' Judgment, Multi-Attack, Natural Gift, Revelation Dance, Techno Blast,
    /// Terrain Pulse and Weather Ball; Gen 7+ Normalize's adds Hidden Power). Raging Bull is in
    /// neither list: used by another than Tauros-Paldea it is a Normal move like any other
    /// (R-M9-49).
    pub fn spares_type_changers(&self) -> bool {
        !matches!(self, OwnType::UserSpecies(_))
    }
}

/// Which type gift of the held item `OwnType::HeldItemType` reads (Showdown `onPlate`,
/// `onMemory`, `onDrive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemGift {
    /// `onPlate`: Judgment reads it; a type Z-Crystal's `onPlate` types Arceus but never the
    /// move (Judgment's own `!item.zMove`).
    Plate,
    /// `onMemory`: Multi-Attack.
    Memory,
    /// `onDrive`: Techno Blast.
    Drive,
}

/// A move's power drawn once as it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomPower {
    /// Magnitude 4-10 at 5 / 10 / 20 / 30 / 20 / 10 / 5 %, power 10 / 30 / 50 / 70 / 90 / 110 /
    /// 150 (`-activate|user|move: Magnitude|N`).
    Magnitude,
    /// Present's `random(10)`: 0-1 heal the target a quarter of its max HP (`move.heal`,
    /// `infiltrates`), 2-5 power 40, 6-8 power 80, 9 power 120.
    Present,
}

/// What a drawn value makes of the move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawnPower {
    Power(u16),
    /// No damage: the target is healed this fraction of its max HP instead.
    HealTarget(Fraction),
}

impl RandomPower {
    /// The values drawn (Magnitude's magnitudes) with their weights.
    pub const fn table(self) -> (&'static [u16], &'static [u16]) {
        match self {
            Self::Magnitude => (&[4, 5, 6, 7, 8, 9, 10], &[5, 10, 20, 30, 20, 10, 5]),
            Self::Present => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9], &[1; 10]),
        }
    }

    /// What a drawn value does.
    pub const fn effect(self, drawn: u16) -> DrawnPower {
        match self {
            Self::Magnitude => DrawnPower::Power(match drawn {
                4 => 10,
                5 => 30,
                6 => 50,
                7 => 70,
                8 => 90,
                9 => 110,
                _ => 150,
            }),
            Self::Present => match drawn {
                0 | 1 => DrawnPower::HealTarget(Fraction::new(1, 4)),
                2..=5 => DrawnPower::Power(40),
                6..=8 => DrawnPower::Power(80),
                _ => DrawnPower::Power(120),
            },
        }
    }
}

/// What a secondary's `onHit` does (the secondary table declares only its chance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecondaryOnHit {
    pub effect: crate::dex::SecondaryEffect,
    pub requires: Option<crate::dex::SecondaryRequirement>,
}

/// A move's own effectiveness against each defending type (Showdown `onEffectiveness`, run
/// per type after the chart; immunities are the move type's, checked apart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeEffectiveness {
    /// Flying Press: the chart's modifier times that of this second type.
    AlsoAs(crate::dex::PokemonType),
    /// Freeze-Dry: super effective against this type whatever the chart says.
    SuperEffectiveOn(crate::dex::PokemonType),
    /// Thousand Arrows (with `ignoreImmunity: {Ground: true}`): a Ground move that reaches a
    /// target off the ground, a Flying type's immunity counting as neutral (its
    /// `onEffectiveness` returns 0 where `runImmunity('Ground')` fails).
    GroundReachesTheAirborne,
}

/// Showdown `onTry` as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryUse {
    /// Fake Out, First Impression: only the user's first move action since it came in
    /// (`activeMoveActions`).
    FirstAction,
    /// Gen 3-4 Endeavor: only while the user has less HP than the target (`-fail`).
    UserHpBelowTarget,
    /// Gen 7+ Dark Void: only this species can use it (`-fail|user|move: Dark Void`). Showdown
    /// also lets a bounced one through (`move.hasBounced`); Magic Coat and Magic Bounce are not
    /// modelled yet.
    UserSpecies(crate::ids::SpeciesId),
    /// Aura Wheel: any forme of this species (`source.species.baseSpecies === 'Morpeko'`: the
    /// name its battling species is based on).
    UserSpeciesFamily(crate::ids::SpeciesId),
    /// Poltergeist: only against a target holding an item (`-fail`).
    TargetHoldsItem,
    /// Aurora Veil: only in this weather (hail / snow, `-fail`).
    InWeather(crate::state::WeatherKind),
    /// Gen 4 Feint: only against a target that is protecting itself (`-fail`).
    TargetProtected,
    /// Steel Roller: only while a terrain is up (`!isTerrain('')`).
    InTerrain,
    /// Sucker Punch, Thunderclap: only against a target that still has to move this turn, with
    /// a damaging move (or Me First), and is not recharging (`queue.willMove`).
    TargetAttacksNext,
    /// Upper Hand: only against a target that still has to move this turn, with a damaging
    /// move of raised priority (`move.priority > 0.1` as ordered).
    TargetUsesPriorityMove,
    /// Last Resort: the user knows it and at least one other move, and has used every other
    /// since it came in (`moveSlot.used`).
    OtherMovesUsed,
    /// Counter, Mirror Coat (Gen 3+), Metal Burst, Comeuppance: only with a hit this turn to
    /// strike back at (`engine::retaliation_record`).
    StrikesBack(crate::dex::Retaliation),
    /// Spit Up: only with a Stockpile (`-fail`).
    Stockpiled,
    /// Stuff Cheeks: only while the user holds a berry (`getItem().isBerry`: set aside or not).
    UserHoldsBerry,
    /// Belch: only once the user has eaten a berry this battle (`ateBerry`).
    UserAteBerry,
    /// Snore, Sleep Talk: only while the user is asleep or has Comatose (`source.status ===
    /// 'slp' || source.hasAbility('comatose')`).
    UserAsleep,
}

/// Rollout / Ice Ball: the fifth hit ends the chain (`hitCount < 5` keeps it going).
pub const ROLLING_MAX_HITS: u8 = 5;

/// How long a Heal Block `move` inflicts lasts (Showdown `durationCallback`): 5 turns unless the
/// move says otherwise.
pub fn heal_block_turns(behavior: &MoveBehavior) -> u8 {
    behavior.heal_block_turns.unwrap_or(5)
}

/// A move's `onHit` that is also its `onAfterSubDamage` (`MoveBehavior::hit_or_sub`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitOrSubEffect {
    /// Steel Roller: the terrain ends.
    ClearTerrain,
    /// Core Enforcer (Gen 7-8): a target that has already moved this turn has its ability
    /// suppressed (`BattleContext::core_enforcer`).
    SuppressAbilityOfMoved,
    /// Flame Burst (Gen 5-7): the target's adjacent allies lose 1/16 of their max HP
    /// (`BattleContext::flame_burst`).
    SplashAdjacentAllies,
}

/// A move's own `onTryHit` refusing it (`-fail`), as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRule {
    /// Yawn: the target has no major status (nor sleep immunity, which nothing gives).
    TargetUnstatused,
    /// Gen 4 Embargo (the gen4 mod's `onTryHit`): not on a Pokémon whose ability is Multitype
    /// or that holds a Griseous Orb.
    ItemBoundTarget,
    /// Gastro Acid: not on an ability that cannot be suppressed (`cantsuppress`); a target
    /// holding Ability Shield blocks it (`-block`).
    SuppressibleAbility,
    /// Gen 2 Roar, Whirlwind (the gen2 mod's `onTryHit`): only as the turn's last action — no
    /// move left in the queue (DESIGN.md §15.17).
    UserMovesLast,
}

/// Showdown `onTryImmunity`: the target is immune (`-immune`) unless this holds — a hit step of
/// its own, placed by the generation (`HitRules::move_immunity_after_try_hit`; Gen 3 reports it
/// with the type immunity, Gen 4 after the accuracy check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryImmunity {
    /// Endeavor: the user has less HP than the target.
    UserHpBelowTarget,
    /// Dream Eater: the target is asleep (or Comatose); Gen 1-4 not behind a Substitute
    /// (`HitRules::dream_eater_substitute_immune`).
    TargetAsleep,
    /// Synchronoise: the target shares one of the user's types.
    SharesUserType,
    /// Attract, Captivate: one of the two is male and the other female.
    OppositeGender,
    /// Octolock (`getImmunity('trapped', target)`): no Ghost type from Gen 6
    /// (`ghost_escapes_trapping`).
    Trappable,
    /// Trick, Switcheroo: the target does not have Sticky Hold (`hasAbility('stickyhold')`).
    NoStickyHold,
    /// Worry Seed: the target's ability is neither Truant nor Insomnia already (`target.ability`,
    /// suppressed or not).
    NotTruantOrInsomnia,
    /// Leech Seed: the target is not a Grass type (`!target.hasType('Grass')`) — a hit step, so
    /// a Grass target stops the move before its accuracy check where the generation checks the
    /// move's immunity first (a Gen 2 Lock-On on it stays, R-M8-107).
    NotGrass,
}

/// A move's own base power formula (Showdown `basePowerCallback`), evaluated by
/// `engine::formula_power`; per-generation constants live in `DamageRules`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerFormula {
    /// Hidden Power (Gen 2-5 `basePowerCallback`): the user's Hidden Power power.
    HiddenPower,
    /// Beat Up: Gen 5+ `5 + ⌊base Attack / 10⌋` of the hit's member; Gen 2-4 its base power.
    BeatUp,
    /// Echoed Voice: `base × the Echoed Voice multiplier` while the condition lasts.
    Echoed,
    /// Round: doubled when another Round moved this action up the queue (Showdown
    /// `move.sourceEffect === 'round'`).
    DoubledWhenPrioritized,
    /// Triple Kick (10), Triple Axel (20): `step × hit` (Showdown `move.hit`, 1 on the first).
    PerHit(u16),
    /// Eruption, Water Spout, Dragon Energy: `base × HP / max HP`.
    UserHpScaled,
    /// Flail, Reversal: the user's HP in bands (`DamageRules::hp_bands`).
    UserHpBands,
    /// Electro Ball: `[40, 60, 80, 120, 150][min(⌊user speed / target speed⌋, 4)]`.
    SpeedRatioBands,
    /// Gyro Ball: `min(150, ⌊25 × target speed / user speed⌋ + 1)`.
    InverseSpeedRatio,
    /// Stored Power, Power Trip: `base + 20 × the user's positive stages`.
    UserPositiveBoosts,
    /// Punishment: `min(200, 60 + 20 × the target's positive stages)`.
    TargetPositiveBoosts,
    /// Crush Grip, Wring Out (120), Hard Press (100): the target's HP scaled
    /// (`DamageRules::target_hp_power`).
    TargetHpScaled { max: u16 },
    /// Hex, Infernal Parade: doubled against a statused target.
    DoubledAgainstStatus,
    /// Pursuit: doubled against a Pokémon switching out (`beingCalledBack || switchFlag`).
    DoubledAgainstSwitching,
    /// Rollout, Ice Ball: `base × 2^hits` of the chain so far, doubled after Defense Curl.
    Rolling,
    /// Fury Cutter (§15.18): the chain's multiplier (`FuryCutterRules`) — Gen 5+ `base ×
    /// multiplier` up to 160, Gen 3-4 `base × 2^(count − 1)`, Gen 2 the base (its doubling is the
    /// damage's).
    FuryCutter,
    /// Rising Voltage: doubled in Electric Terrain against a grounded target.
    DoubledInElectricTerrain,
    /// Return: `⌊happiness × 10 / 25⌋`.
    Happiness,
    /// Frustration: `⌊(255 − happiness) × 10 / 25⌋`.
    Unhappiness,
    /// Trump Card: by the PP its slot has left once this use is paid (0 → 200, 1 → 80, 2 → 60,
    /// 3 → 50, else 40).
    PpLeftBands,
    /// Water Shuriken: 5 more for Greninja-Ash with Battle Bond.
    AshGreninjaBonus,
    /// Spit Up: 100 per Stockpile layer.
    StockpileLayers,
    /// Wake-Up Slap (a sleeping or Comatose target), Smelling Salts (a paralysed one): doubled.
    DoubledAgainst(crate::state::NonVolatileStatus),
    /// Low Kick, Grass Knot: the target's weight in bands.
    TargetWeightBands,
    /// Heavy Slam, Heat Crash: the user's weight against the target's.
    WeightRatioBands,
    /// Acrobatics: doubled when the user holds no item.
    DoubledWithoutItem,
    // Event history (DESIGN.md §22.8).
    /// Payback: doubled once the target has moved this turn — it is not about to move and,
    /// unless `DamageRules::newcomer_counts_as_moved`, did not just switch in.
    DoubledIfTargetMoved,
    /// Bolt Beak, Fishious Rend: doubled while the target has yet to move (it is about to,
    /// or just switched in).
    DoubledIfTargetYetToMove,
    /// Revenge, Avalanche: doubled when the target damaged the user this turn.
    DoubledIfDamagedByTarget,
    /// Assurance: doubled when the target lost HP this turn.
    DoubledIfTargetHurt,
    /// Stomping Tantrum, Temper Flare: doubled when the user's move failed last turn.
    DoubledAfterFailure,
    /// Rage Fist: `min(350, 50 + 50 × the hits the user has taken)`.
    TimesAttacked,
    /// Last Respects: `50 + 50 × the user's side's faints`.
    FaintedAllies,
}

/// A move's own BasePower chain participant (Showdown `onBasePower`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerBoost {
    pub condition: PowerCondition,
    /// 4096-based.
    pub modifier: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerCondition {
    /// Facade: the user has a status other than sleep.
    UserStatusedAwake,
    /// Brine: the target is at half HP or below.
    TargetAtHalfHp,
    /// Venoshock, Barb Barrage: the target is poisoned or badly poisoned.
    TargetPoisoned,
    /// Collision Course, Electro Drift: the hit is super effective.
    SuperEffective,
    /// Psyblade: Electric Terrain is up.
    ElectricTerrain,
    /// Expanding Force: this terrain is up and the user is grounded.
    GroundedInTerrain(crate::state::TerrainKind),
    /// Retaliate: a Pokémon on the user's side fainted last turn.
    AllyFaintedLastTurn,
    /// Lash Out: the user's stats were lowered this turn.
    UserStatsLoweredThisTurn,
    /// Fusion Flare / Fusion Bolt: the turn's last successful move is the partner move.
    AfterMove(MoveId),
    /// Solar Beam: rain, sandstorm or snow (anything but sun) is up.
    WeakeningWeather,
    /// Grav Apple: this pseudo-weather (Gravity) is up.
    FieldCondition(crate::state::FieldConditionKind),
    /// Gen 6+ Knock Off: the target holds an item its own `onTakeItem` would let this move take
    /// (Showdown `singleEvent('TakeItem')` on the item alone, so Sticky Hold does not stop it).
    TargetItemTakable(MoveId),
}

/// How the weather changes a move's accuracy (Showdown `onModifyMove`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherAccuracy {
    /// Thunder, Hurricane: never miss in rain, 50 % in sun.
    RainSureSunHalf,
    /// Bleakwind / Wildbolt / Sandsear Storm: never miss in rain.
    RainSure,
    /// Blizzard: never misses in hail / snow.
    SnowSure,
}

/// A healing move whose amount follows the weather (Showdown `onHit`); the amounts are the
/// generation's `ConditionRules::weather_heal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherHeal {
    /// Moonlight, Morning Sun, Synthesis: more in sun, less in any other weather.
    Sunlight,
    /// Shore Up: more in a sandstorm.
    Sand,
}

/// The move behavior components (`effects::BehaviorComponent`, DESIGN.md §25.1).
const COMPONENTS: &[BehaviorComponent<MoveBehavior>] = &[
    BehaviorComponent {
        source_callbacks: |h| match h.two_turn {
            Some(_) => &["onTryMove"],
            None => &[],
        },
        disable: |h| h.two_turn = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ally_heal {
            Some(_) => &["onTryHit", "onHit", "onTryMove"],
            None => &[],
        },
        disable: |h| h.ally_heal = None,
    },
    BehaviorComponent {
        source_callbacks: |h| if h.beat_up { &["onModifyMove"] } else { &[] },
        disable: |h| h.beat_up = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.category_by_estimate {
                &["onModifyMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.category_by_estimate = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.category_by_stats {
                &["onModifyMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.category_by_stats = false,
    },
    BehaviorComponent {
        // Genesis Supernova's `secondary.self.onHit` and the Max Moves' `self.onHit`: the
        // snapshot declares them as a `self` callback.
        source_callbacks: |h| {
            if h.z_terrain.is_some() || h.max_move_effect.is_some() {
                &["self:onHit"]
            } else {
                &[]
            }
        },
        disable: |h| {
            h.z_terrain = None;
            h.max_move_effect = None;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.tera_blast {
                &[
                    "basePowerCallback",
                    "onModifyMove",
                    "onModifyType",
                    "onPrepareHit",
                ]
            } else if h.tera_starstorm {
                &["onModifyMove", "onModifyType"]
            } else if h.pledge_combo {
                &["basePowerCallback", "onModifyMove", "onPrepareHit"]
            } else {
                &[]
            }
        },
        disable: |h| {
            h.tera_blast = false;
            h.tera_starstorm = false;
            h.pledge_combo = false;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| if h.echoes { &["onTryMove"] } else { &[] },
        disable: |h| h.echoes = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.prioritizes_same_move {
                &["onTry"]
            } else {
                &[]
            }
        },
        disable: |h| h.prioritizes_same_move = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.type_animation {
            Some(true) => &["onPrepareHit"],
            _ => &[],
        },
        disable: |h| h.type_animation = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.announcement {
            Some(Announcement::Coins) => &["onHit"],
            Some(Announcement::TargetItem) => &["onTryHit"],
            Some(Announcement::Category) => &["onPrepareHit", "onHit", "onAfterSubDamage"],
            None => &[],
        },
        disable: |h| h.announcement = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.try_use {
            Some(_) => &["onTry"],
            None => &[],
        },
        disable: |h| h.try_use = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.self_disables {
                &["onDisableMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.self_disables = false,
    },
    BehaviorComponent {
        // Gen 6-7 implement Toxic's sure hit as an `onPrepareHit` volatile; from Gen 8 Showdown
        // hard-codes it in the hit steps (`gen >= 8 && move.id === 'toxic'`), with no
        // callback. The generation's rule (`ConditionRules::typed_sure_hit`) decides, so the
        // component covers the callback but is never cleared.
        source_callbacks: |h| match h.sure_hit_user_type {
            Some(_) => &["onPrepareHit"],
            None => &[],
        },
        disable: |_| {},
    },
    BehaviorComponent {
        source_callbacks: |h| match h.damage_override {
            Some(_) => &["damageCallback"],
            None => &[],
        },
        disable: |h| h.damage_override = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.power_formula {
            // Gen 2-3 Fury Cutter also doubles in its own `onHit` (§15.18): the same chain.
            Some(PowerFormula::FuryCutter) => &["basePowerCallback", "onHit"],
            Some(_) => &["basePowerCallback"],
            None => &[],
        },
        disable: |h| h.power_formula = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.power_boost {
            Some(_) => &["onBasePower"],
            None => &[],
        },
        disable: |h| h.power_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.weather_accuracy {
            Some(_) => &["onModifyMove"],
            None => &[],
        },
        disable: |h| h.weather_accuracy = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.sun_doubles_boosts {
                &["onModifyMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.sun_doubles_boosts = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.weather_heal {
            Some(_) => &["onHit"],
            None => &[],
        },
        disable: |h| h.weather_heal = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.target_rule {
            Some(_) => &["onTryHit"],
            None => &[],
        },
        disable: |h| h.target_rule = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.status_transfer {
                &["onTryHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.status_transfer = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.cures_burn_after_move {
                &["onAfterMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.cures_burn_after_move = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.cure_target {
            Some(_) => &["onHit"],
            None => &[],
        },
        disable: |h| h.cure_target = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.target_heal {
            Some(_) => &["onHit"],
            None => &[],
        },
        disable: |h| h.target_heal = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.on_hit_heal {
            Some(_) => &["onHit"],
            None => &[],
        },
        disable: |h| h.on_hit_heal = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.dynamax_only_try_hit {
                &["onTryHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.dynamax_only_try_hit = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.typeless { &["onModifyMove"] } else { &[] }
        },
        disable: |h| h.typeless = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.effectiveness {
            Some(_) => &["onEffectiveness"],
            None => &[],
        },
        disable: |h| h.effectiveness = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.spares_target { &["onDamage"] } else { &[] }
        },
        disable: |h| h.spares_target = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.try_immunity {
            Some(_) => &["onTryImmunity"],
            None => &[],
        },
        disable: |h| h.try_immunity = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.misses_maxed_attack {
                &["onTryHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.misses_maxed_attack = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.secondary_on_hit {
            Some(_) => &["secondary:onHit"],
            None => &[],
        },
        disable: |h| h.secondary_on_hit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.own_type {
            Some(OwnType::UserSpecies(_) | OwnType::NaturalGift) => &["onModifyType"],
            Some(_) => &["onModifyType", "onModifyMove"],
            None => &[],
        },
        disable: |h| h.own_type = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.breaks_screens { &["onTryHit"] } else { &[] }
        },
        disable: |h| h.breaks_screens = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.terrain_priority {
            Some(_) => &["onModifyPriority"],
            None => &[],
        },
        disable: |h| h.terrain_priority = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.terrain_spread {
            Some(_) => &["onModifyMove"],
            None => &[],
        },
        disable: |h| h.terrain_spread = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.terrain_secondaries {
                &["onModifyMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.terrain_secondaries = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.burns_item { &["onHit"] } else { &[] }
        },
        disable: |h| h.burns_item = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.steals_berry { &["onHit"] } else { &[] }
        },
        disable: |h| h.steals_berry = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.fling || h.natural_gift {
                &["onPrepareHit"]
            } else {
                &[]
            }
        },
        disable: |h| {
            h.fling = false;
            h.natural_gift = false;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| match h.takes_item {
            Some(_) => &["onAfterHit"],
            None => &[],
        },
        disable: |h| h.takes_item = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_hit {
            Some(_) => &["onAfterHit", "onAfterSubDamage"],
            None => &[],
        },
        disable: |h| h.after_hit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.hit_or_sub {
            Some(_) => &["onHit", "onAfterSubDamage"],
            None => &[],
        },
        disable: |h| h.hit_or_sub = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.binding_lock { &["onTryMove"] } else { &[] }
        },
        disable: |h| h.binding_lock = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.hit_traps {
            Some(_) => &["onHit"],
            None => &[],
        },
        disable: |h| h.hit_traps = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.turn_start_volatile {
            Some(_) => &["priorityChargeCallback", "beforeTurnCallback"],
            None => &[],
        },
        disable: |h| h.turn_start_volatile = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.lost_focus_stops {
                &["beforeMoveCallback"]
            } else {
                &[]
            }
        },
        disable: |h| h.lost_focus_stops = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.lost_focus_fails { &["onTry"] } else { &[] }
        },
        disable: |h| h.lost_focus_fails = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.ends_turn_start_volatile {
                &["onAfterMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.ends_turn_start_volatile = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.strikes_back_at {
            Some(_) => &["onModifyTarget", "beforeTurnCallback"],
            None => &[],
        },
        disable: |h| h.strikes_back_at = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.rolling {
                &["onModifyMove", "onAfterMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.rolling = false,
    },
    BehaviorComponent {
        source_callbacks: |h| if h.uproar { &["onTryHit"] } else { &[] },
        disable: |h| h.uproar = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.charge_fails_asleep {
                &["onPrepareHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.charge_fails_asleep = false,
    },
    BehaviorComponent {
        source_callbacks: |h| if h.future_attack { &["onTry"] } else { &[] },
        disable: |h| h.future_attack = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.sky_drop {
                &["onModifyMove", "onTry", "onTryHit", "onHit", "onMoveFail"]
            } else {
                &[]
            }
        },
        disable: |h| h.sky_drop = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.rampage_lock_at_start {
                &["onMoveFail"]
            } else {
                &[]
            }
        },
        disable: |h| h.rampage_lock_at_start = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.imprison_needs_shared_move {
                &["onTryHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.imprison_needs_shared_move = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.marks_pursuit {
                &["beforeTurnCallback"]
            } else {
                &[]
            }
        },
        disable: |h| h.marks_pursuit = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.sure_hit_on_switching {
                &["onModifyMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.sure_hit_on_switching = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.needs_armed_trap {
                &["onTryMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.needs_armed_trap = false,
    },
    // The move's `self` table's callbacks are declared as `self:<name>`.
    BehaviorComponent {
        source_callbacks: |h| {
            if h.self_clears_hazards {
                &["self:onHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.self_clears_hazards = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.burns_up {
            Some(_) => &["onTryMove", "self:onHit"],
            None => &[],
        },
        disable: |h| h.burns_up = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.power_chance {
            Some(_) => &["onBasePower"],
            None => &[],
        },
        disable: |h| h.power_chance = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.random_power {
            Some(RandomPower::Magnitude) => &["onModifyMove", "onUseMoveMessage"],
            Some(RandomPower::Present) => &["onModifyMove"],
            None => &[],
        },
        disable: |h| h.random_power = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.status_pick_before_chance {
                &["onHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.status_pick_before_chance = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.secondaries_need_user_species {
            Some(_) => &["onModifyMove"],
            None => &[],
        },
        disable: |h| h.secondaries_need_user_species = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.order_up {
                &["onAfterMoveSecondarySelf"]
            } else {
                &[]
            }
        },
        disable: |h| h.order_up = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.knock_out_boost {
            Some(_) => &["onAfterMoveSecondarySelf"],
            None => &[],
        },
        disable: |h| h.knock_out_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.forme_toggle {
            Some(_) => &["onAfterMoveSecondarySelf"],
            None => &[],
        },
        disable: |h| h.forme_toggle = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.cures_target_status {
            Some(_) => &["onHit"],
            None => &[],
        },
        disable: |h| h.cures_target_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.ends_stockpile {
                &["onAfterMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.ends_stockpile = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.clears_target_stages {
                &["onHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.clears_target_stages = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.crash_damage { &["onMoveFail"] } else { &[] }
        },
        disable: |h| h.crash_damage = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.self_damage_after_move {
            Some(_) => &["onAfterMove"],
            None => &[],
        },
        disable: |h| h.self_damage_after_move = None,
    },
];

/// False Swipe / Hold Back's `onDamage`: `damage >= target.hp` becomes `target.hp - 1`.
pub use protect::{endure_damage, punish_contact};

pub fn leave_one_hp(
    ctx: &mut BattleContext<'_>,
    request: &crate::engine::DamageRequest,
) -> crate::engine::DamageDecision {
    match ctx.hp(request.target) {
        Some(hp) if request.amount.0 >= hp => {
            crate::engine::DamageDecision::Reduce(crate::engine::DamageAmount(hp.saturating_sub(1)))
        }
        _ => crate::engine::DamageDecision::Allow,
    }
}

impl MoveBehavior {
    /// Every Showdown callback some component of these behavior stands for.
    pub fn covered_callbacks(&self) -> Vec<&'static str> {
        super::covered_callbacks(COMPONENTS, self)
    }

    /// These behavior without the components none of whose callbacks `declared` lists.
    pub fn restricted_to(self, declared: &[String]) -> Self {
        super::restricted_to(COMPONENTS, self, declared)
    }
}

/// A Max Move's behavior: its `self.onHit` alone.
fn max_effect(effect: MaxMoveEffect) -> MoveBehavior {
    MoveBehavior {
        max_move_effect: Some(effect),
        ..MoveBehavior::default()
    }
}

/// Every move's data behavior, in every generation; the dex restricts them to what each
/// generation declares (`MoveBehavior::restricted_to`). Most moves need none.
pub fn definition(move_id: MoveId) -> MoveBehavior {
    match move_id {
        MoveId::ACROBATICS => formula(PowerFormula::DoubledWithoutItem),
        MoveId::FALSE_SWIPE | MoveId::HOLD_BACK => MoveBehavior {
            spares_target: true,
            ..MoveBehavior::default()
        },
        MoveId::HIGH_JUMP_KICK | MoveId::JUMP_KICK | MoveId::AXE_KICK | MoveId::SUPERCELL_SLAM => {
            MoveBehavior {
                crash_damage: true,
                ..MoveBehavior::default()
            }
        }
        MoveId::MIND_BLOWN => MoveBehavior {
            self_damage_after_move: Some(Fraction::new(1, 2)),
            stopped_by_damp: true,
            ..MoveBehavior::default()
        },
        MoveId::STEEL_BEAM => MoveBehavior {
            self_damage_after_move: Some(Fraction::new(1, 2)),
            ..MoveBehavior::default()
        },
        MoveId::EXPLOSION | MoveId::SELF_DESTRUCT => MoveBehavior {
            halves_defense: true,
            stopped_by_damp: true,
            ..MoveBehavior::default()
        },
        MoveId::MISTY_EXPLOSION => MoveBehavior {
            power_boost: Some(PowerBoost {
                condition: PowerCondition::GroundedInTerrain(crate::state::TerrainKind::Misty),
                modifier: 6144,
            }),
            stopped_by_damp: true,
            ..MoveBehavior::default()
        },
        MoveId::FINAL_GAMBIT => MoveBehavior {
            damage_override: Some(crate::dex::FixedDamage::UserHp),
            ..MoveBehavior::default()
        },
        MoveId::FLYING_PRESS => MoveBehavior {
            effectiveness: Some(TypeEffectiveness::AlsoAs(crate::dex::PokemonType::Flying)),
            ..MoveBehavior::default()
        },
        MoveId::THOUSAND_ARROWS => MoveBehavior {
            effectiveness: Some(TypeEffectiveness::GroundReachesTheAirborne),
            ..MoveBehavior::default()
        },
        MoveId::FREEZE_DRY => MoveBehavior {
            effectiveness: Some(TypeEffectiveness::SuperEffectiveOn(
                crate::dex::PokemonType::Water,
            )),
            ..MoveBehavior::default()
        },
        MoveId::TOXIC => MoveBehavior {
            sure_hit_user_type: Some(crate::dex::PokemonType::Poison),
            ..MoveBehavior::default()
        },
        MoveId::ERUPTION | MoveId::WATER_SPOUT | MoveId::DRAGON_ENERGY => {
            formula(PowerFormula::UserHpScaled)
        }
        MoveId::FLAIL | MoveId::REVERSAL => formula(PowerFormula::UserHpBands),
        MoveId::ELECTRO_BALL => formula(PowerFormula::SpeedRatioBands),
        MoveId::GYRO_BALL => formula(PowerFormula::InverseSpeedRatio),
        MoveId::STORED_POWER | MoveId::POWER_TRIP => formula(PowerFormula::UserPositiveBoosts),
        MoveId::PUNISHMENT => formula(PowerFormula::TargetPositiveBoosts),
        MoveId::CRUSH_GRIP | MoveId::WRING_OUT => {
            formula(PowerFormula::TargetHpScaled { max: 120 })
        }
        MoveId::HARD_PRESS => formula(PowerFormula::TargetHpScaled { max: 100 }),
        MoveId::HEX | MoveId::INFERNAL_PARADE => formula(PowerFormula::DoubledAgainstStatus),
        MoveId::RISING_VOLTAGE => formula(PowerFormula::DoubledInElectricTerrain),
        MoveId::RETURN => formula(PowerFormula::Happiness),
        MoveId::FRUSTRATION => formula(PowerFormula::Unhappiness),
        MoveId::FEINT => MoveBehavior {
            try_use: Some(TryUse::TargetProtected),
            ..MoveBehavior::default()
        },
        MoveId::ENDEAVOR => MoveBehavior {
            damage_override: Some(crate::dex::FixedDamage::TargetHpMinusUserHp),
            try_use: Some(TryUse::UserHpBelowTarget),
            try_immunity: Some(TryImmunity::UserHpBelowTarget),
            ..MoveBehavior::default()
        },
        MoveId::BURNING_JEALOUSY => MoveBehavior {
            secondary_on_hit: Some(SecondaryOnHit {
                effect: crate::dex::SecondaryEffect::Status(crate::state::NonVolatileStatus::Burn),
                requires: Some(crate::dex::SecondaryRequirement::TargetStatsRaisedThisTurn),
            }),
            ..MoveBehavior::default()
        },
        MoveId::ALLURING_VOICE => MoveBehavior {
            secondary_on_hit: Some(SecondaryOnHit {
                effect: crate::dex::SecondaryEffect::Volatile(
                    crate::state::VolatileConditionKind::Confusion,
                ),
                requires: Some(crate::dex::SecondaryRequirement::TargetStatsRaisedThisTurn),
            }),
            ..MoveBehavior::default()
        },
        MoveId::WEATHER_BALL => MoveBehavior {
            own_type: Some(OwnType::Weather),
            ..MoveBehavior::default()
        },
        MoveId::REVELATION_DANCE => MoveBehavior {
            own_type: Some(OwnType::UserFirstType),
            ..MoveBehavior::default()
        },
        MoveId::HIDDEN_POWER => MoveBehavior {
            own_type: Some(OwnType::HiddenPower),
            power_formula: Some(PowerFormula::HiddenPower),
            ..MoveBehavior::default()
        },
        MoveId::BEAT_UP => MoveBehavior {
            beat_up: true,
            power_formula: Some(PowerFormula::BeatUp),
            ..MoveBehavior::default()
        },
        MoveId::PHOTON_GEYSER => MoveBehavior {
            category_by_stats: true,
            ..MoveBehavior::default()
        },
        // Ultra Necrozma's own Z-Move (DESIGN.md §44.4): its `onModifyMove` is Photon Geyser's
        // (the higher of Attack and Special Attack decides), and its data ignores the target's
        // ability.
        MoveId::LIGHT_THAT_BURNS_THE_SKY => MoveBehavior {
            category_by_stats: true,
            ..MoveBehavior::default()
        },
        MoveId::TERA_BLAST => MoveBehavior {
            tera_blast: true,
            ..MoveBehavior::default()
        },
        MoveId::TERA_STARSTORM => MoveBehavior {
            tera_starstorm: true,
            ..MoveBehavior::default()
        },
        MoveId::SHELL_SIDE_ARM => MoveBehavior {
            category_by_estimate: true,
            announcement: Some(Announcement::Category),
            ..MoveBehavior::default()
        },
        MoveId::PRESENT => MoveBehavior {
            random_power: Some(RandomPower::Present),
            type_index_damage: true,
            ..MoveBehavior::default()
        },
        MoveId::POLLEN_PUFF => MoveBehavior {
            ally_heal: Some(Fraction::new(1, 2)),
            ..MoveBehavior::default()
        },
        MoveId::ECHOED_VOICE => MoveBehavior {
            echoes: true,
            power_formula: Some(PowerFormula::Echoed),
            ..MoveBehavior::default()
        },
        MoveId::ROUND => MoveBehavior {
            prioritizes_same_move: true,
            power_formula: Some(PowerFormula::DoubledWhenPrioritized),
            ..MoveBehavior::default()
        },
        MoveId::DARK_VOID => MoveBehavior {
            try_use: Some(TryUse::UserSpecies(crate::ids::SpeciesId::DARKRAI)),
            ..MoveBehavior::default()
        },
        // Hyperspace Fury: Hoopa-Unbound only (`-fail`; Hoopa itself is told `[forme]`).
        MoveId::HYPERSPACE_FURY => MoveBehavior {
            try_use: Some(TryUse::UserSpecies(crate::ids::SpeciesId::HOOPA_UNBOUND)),
            ..MoveBehavior::default()
        },
        // Aura Wheel: Morpeko's own move, Dark in its Hangry forme, Electric otherwise.
        MoveId::AURA_WHEEL => MoveBehavior {
            try_use: Some(TryUse::UserSpeciesFamily(crate::ids::SpeciesId::MORPEKO)),
            own_type: Some(OwnType::UserSpecies(&[(
                crate::ids::SpeciesId::MORPEKO_HANGRY,
                crate::dex::PokemonType::Dark,
            )])),
            ..MoveBehavior::default()
        },
        // Ivy Cudgel: the type of the Ogerpon forme that holds it (its Terastal formes too);
        // its Masks are separate items (`effects::items`).
        MoveId::JUDGMENT => MoveBehavior {
            own_type: Some(OwnType::HeldItemType(ItemGift::Plate)),
            ..MoveBehavior::default()
        },
        // Multi-Attack: the type of the Memory the user holds (`onModifyType`'s Memory event;
        // Normal without one — a Z-Crystal has no `onMemory`). Techno Blast is the same with a
        // Drive (`onDrive`).
        MoveId::MULTI_ATTACK => MoveBehavior {
            own_type: Some(OwnType::HeldItemType(ItemGift::Memory)),
            ..MoveBehavior::default()
        },
        MoveId::TECHNO_BLAST => MoveBehavior {
            own_type: Some(OwnType::HeldItemType(ItemGift::Drive)),
            ..MoveBehavior::default()
        },
        MoveId::IVY_CUDGEL => MoveBehavior {
            type_animation: Some(true),
            own_type: Some(OwnType::UserSpecies(&[
                (
                    crate::ids::SpeciesId::OGERPON_WELLSPRING,
                    crate::dex::PokemonType::Water,
                ),
                (
                    crate::ids::SpeciesId::OGERPON_WELLSPRING_TERA,
                    crate::dex::PokemonType::Water,
                ),
                (
                    crate::ids::SpeciesId::OGERPON_HEARTHFLAME,
                    crate::dex::PokemonType::Fire,
                ),
                (
                    crate::ids::SpeciesId::OGERPON_HEARTHFLAME_TERA,
                    crate::dex::PokemonType::Fire,
                ),
                (
                    crate::ids::SpeciesId::OGERPON_CORNERSTONE,
                    crate::dex::PokemonType::Rock,
                ),
                (
                    crate::ids::SpeciesId::OGERPON_CORNERSTONE_TERA,
                    crate::dex::PokemonType::Rock,
                ),
            ])),
            ..MoveBehavior::default()
        },
        MoveId::POLTERGEIST => MoveBehavior {
            try_use: Some(TryUse::TargetHoldsItem),
            announcement: Some(Announcement::TargetItem),
            ..MoveBehavior::default()
        },
        MoveId::PAY_DAY => MoveBehavior {
            announcement: Some(Announcement::Coins),
            ..MoveBehavior::default()
        },
        MoveId::FICKLE_BEAM => MoveBehavior {
            power_chance: Some(PowerChance {
                numerator: 3,
                denominator: 10,
                modifier: 8192,
            }),
            ..MoveBehavior::default()
        },
        MoveId::MAGNITUDE => MoveBehavior {
            random_power: Some(RandomPower::Magnitude),
            ..MoveBehavior::default()
        },
        MoveId::PSYWAVE => MoveBehavior {
            damage_override: Some(crate::dex::FixedDamage::Psywave),
            ..MoveBehavior::default()
        },
        MoveId::TRIPLE_KICK => formula(PowerFormula::PerHit(10)),
        MoveId::TRIPLE_AXEL => formula(PowerFormula::PerHit(20)),
        MoveId::TRI_ATTACK => MoveBehavior {
            secondary_on_hit: Some(SecondaryOnHit {
                effect: crate::dex::SecondaryEffect::RandomStatus([
                    crate::state::NonVolatileStatus::Burn,
                    crate::state::NonVolatileStatus::Paralysis,
                    crate::state::NonVolatileStatus::Freeze,
                ]),
                requires: None,
            }),
            status_pick_before_chance: true,
            ..MoveBehavior::default()
        },
        MoveId::DIRE_CLAW => MoveBehavior {
            secondary_on_hit: Some(SecondaryOnHit {
                effect: crate::dex::SecondaryEffect::RandomStatus([
                    crate::state::NonVolatileStatus::Poison,
                    crate::state::NonVolatileStatus::Paralysis,
                    crate::state::NonVolatileStatus::Sleep,
                ]),
                requires: None,
            }),
            ..MoveBehavior::default()
        },
        MoveId::CHATTER => MoveBehavior {
            secondaries_need_user_species: Some(crate::ids::SpeciesId::CHATOT),
            ..MoveBehavior::default()
        },
        MoveId::RELIC_SONG => MoveBehavior {
            forme_toggle: Some((
                crate::ids::SpeciesId::MELOETTA,
                crate::ids::SpeciesId::MELOETTA_PIROUETTE,
            )),
            ..MoveBehavior::default()
        },
        MoveId::ORDER_UP => MoveBehavior {
            order_up: true,
            ..MoveBehavior::default()
        },
        MoveId::FELL_STINGER => MoveBehavior {
            knock_out_boost: Some(crate::state::Stat::Attack),
            ..MoveBehavior::default()
        },
        MoveId::DREAM_EATER => MoveBehavior {
            try_immunity: Some(TryImmunity::TargetAsleep),
            ..MoveBehavior::default()
        },
        MoveId::SYNCHRONOISE => MoveBehavior {
            try_immunity: Some(TryImmunity::SharesUserType),
            ..MoveBehavior::default()
        },
        MoveId::ATTRACT | MoveId::CAPTIVATE => MoveBehavior {
            try_immunity: Some(TryImmunity::OppositeGender),
            ..MoveBehavior::default()
        },
        MoveId::KNOCK_OFF => MoveBehavior {
            power_boost: Some(PowerBoost {
                condition: PowerCondition::TargetItemTakable(MoveId::KNOCK_OFF),
                modifier: 6144,
            }),
            takes_item: Some(ItemTake::KnockOff),
            ..MoveBehavior::default()
        },
        // Gen 2 Thief's 100 % secondary `onHit`; Gen 3+ Thief and Covet's `onAfterHit`.
        MoveId::THIEF => MoveBehavior {
            secondary_on_hit: Some(SecondaryOnHit {
                effect: crate::dex::SecondaryEffect::StealItem,
                requires: None,
            }),
            takes_item: Some(ItemTake::Steal),
            ..MoveBehavior::default()
        },
        MoveId::TRICK | MoveId::SWITCHEROO => MoveBehavior {
            try_immunity: Some(TryImmunity::NoStickyHold),
            ..MoveBehavior::default()
        },
        MoveId::WORRY_SEED => MoveBehavior {
            try_immunity: Some(TryImmunity::NotTruantOrInsomnia),
            ..MoveBehavior::default()
        },
        MoveId::LEECH_SEED => MoveBehavior {
            try_immunity: Some(TryImmunity::NotGrass),
            ..MoveBehavior::default()
        },
        MoveId::INCINERATE => MoveBehavior {
            burns_item: true,
            ..MoveBehavior::default()
        },
        MoveId::BUG_BITE | MoveId::PLUCK => MoveBehavior {
            steals_berry: true,
            ..MoveBehavior::default()
        },
        MoveId::FLING => MoveBehavior {
            fling: true,
            ..MoveBehavior::default()
        },
        MoveId::NATURAL_GIFT => MoveBehavior {
            own_type: Some(OwnType::NaturalGift),
            natural_gift: true,
            ..MoveBehavior::default()
        },
        // Belch's `onTry` repeats its `onDisableMove` for a call from outside the request;
        // nothing calls it (no `metronome` flag, and Sleep Talk, Assist, Copycat, Me First,
        // Mimic and Instruct refuse it), so the request is where it is refused.
        MoveId::BELCH => MoveBehavior {
            try_use: Some(TryUse::UserAteBerry),
            self_disables: true,
            ..MoveBehavior::default()
        },
        // Snore's `onTry` (its `sleepUsable` lets it past the sleeping user's BeforeMove,
        // `MoveFlags::sleep_usable`).
        // Sleep Talk's `onTry` (its `sleepUsable` lets it past the sleeping user's BeforeMove);
        // Gen 4's `onTryHit` under a Choice lock or an Encore is its `tries`.
        MoveId::SLEEP_TALK => MoveBehavior {
            try_use: Some(TryUse::UserAsleep),
            ..MoveBehavior::default()
        },
        MoveId::SNORE => MoveBehavior {
            try_use: Some(TryUse::UserAsleep),
            ..MoveBehavior::default()
        },
        MoveId::STUFF_CHEEKS => MoveBehavior {
            try_use: Some(TryUse::UserHoldsBerry),
            self_disables: true,
            ..MoveBehavior::default()
        },
        MoveId::COVET => MoveBehavior {
            takes_item: Some(ItemTake::Steal),
            ..MoveBehavior::default()
        },
        MoveId::YAWN => MoveBehavior {
            target_rule: Some(TargetRule::TargetUnstatused),
            ..MoveBehavior::default()
        },
        // Gen 2 only declares the `onTryHit` (the component is off elsewhere).
        MoveId::ROAR | MoveId::WHIRLWIND => MoveBehavior {
            target_rule: Some(TargetRule::UserMovesLast),
            ..MoveBehavior::default()
        },
        MoveId::EMBARGO => MoveBehavior {
            target_rule: Some(TargetRule::ItemBoundTarget),
            ..MoveBehavior::default()
        },
        MoveId::GASTRO_ACID => MoveBehavior {
            target_rule: Some(TargetRule::SuppressibleAbility),
            ..MoveBehavior::default()
        },
        MoveId::PSYCHO_SHIFT => MoveBehavior {
            status_transfer: true,
            ..MoveBehavior::default()
        },
        MoveId::PURIFY => MoveBehavior {
            cure_target: Some(Fraction::new(1, 2)),
            ..MoveBehavior::default()
        },
        MoveId::PSYCHIC_NOISE => MoveBehavior {
            heal_block_turns: Some(2),
            ..MoveBehavior::default()
        },
        MoveId::THROAT_CHOP => MoveBehavior {
            secondary_on_hit: Some(SecondaryOnHit {
                effect: crate::dex::SecondaryEffect::Volatile(
                    crate::state::VolatileConditionKind::ThroatChop,
                ),
                requires: None,
            }),
            ..MoveBehavior::default()
        },
        MoveId::SPARKLING_ARIA => MoveBehavior {
            cures_burn_after_move: true,
            ..MoveBehavior::default()
        },
        MoveId::SWAGGER => MoveBehavior {
            misses_maxed_attack: true,
            boosts_through_gen2_substitute: true,
            ..MoveBehavior::default()
        },
        MoveId::MEAN_LOOK | MoveId::SPIDER_WEB => MoveBehavior {
            through_gen2_substitute: true,
            ..MoveBehavior::default()
        },
        MoveId::TRUMP_CARD => MoveBehavior {
            power_formula: Some(PowerFormula::PpLeftBands),
            no_pp_boosts: true,
            ..MoveBehavior::default()
        },
        MoveId::WATER_SHURIKEN => formula(PowerFormula::AshGreninjaBonus),
        MoveId::WAKE_UP_SLAP => MoveBehavior {
            power_formula: Some(PowerFormula::DoubledAgainst(
                crate::state::NonVolatileStatus::Sleep,
            )),
            cures_target_status: Some(crate::state::NonVolatileStatus::Sleep),
            ..MoveBehavior::default()
        },
        MoveId::SMELLING_SALTS => MoveBehavior {
            power_formula: Some(PowerFormula::DoubledAgainst(
                crate::state::NonVolatileStatus::Paralysis,
            )),
            cures_target_status: Some(crate::state::NonVolatileStatus::Paralysis),
            ..MoveBehavior::default()
        },
        MoveId::LOW_KICK | MoveId::GRASS_KNOT => MoveBehavior {
            power_formula: Some(PowerFormula::TargetWeightBands),
            dynamax_only_try_hit: true,
            ..MoveBehavior::default()
        },
        MoveId::HEAVY_SLAM | MoveId::HEAT_CRASH => MoveBehavior {
            power_formula: Some(PowerFormula::WeightRatioBands),
            dynamax_only_try_hit: true,
            ..MoveBehavior::default()
        },
        MoveId::FACADE => MoveBehavior {
            power_boost: Some(PowerBoost {
                condition: PowerCondition::UserStatusedAwake,
                modifier: 8192,
            }),
            ignores_burn_drop: true,
            ..MoveBehavior::default()
        },
        MoveId::CHLOROBLAST => MoveBehavior {
            max_hp_recoil: Some(Fraction::new(1, 2)),
            ..MoveBehavior::default()
        },
        MoveId::STRUGGLE => MoveBehavior {
            typeless: true,
            ..MoveBehavior::default()
        },
        MoveId::HYDRO_STEAM => MoveBehavior {
            sun_boosted: true,
            ..MoveBehavior::default()
        },
        MoveId::BRINE => boost(PowerCondition::TargetAtHalfHp, 8192),
        MoveId::VENOSHOCK | MoveId::BARB_BARRAGE => boost(PowerCondition::TargetPoisoned, 8192),
        MoveId::COLLISION_COURSE | MoveId::ELECTRO_DRIFT => {
            boost(PowerCondition::SuperEffective, 5461)
        }
        MoveId::PSYBLADE => boost(PowerCondition::ElectricTerrain, 6144),
        MoveId::PAYBACK => formula(PowerFormula::DoubledIfTargetMoved),
        MoveId::BOLT_BEAK | MoveId::FISHIOUS_REND => {
            formula(PowerFormula::DoubledIfTargetYetToMove)
        }
        MoveId::REVENGE | MoveId::AVALANCHE => formula(PowerFormula::DoubledIfDamagedByTarget),
        MoveId::ASSURANCE => formula(PowerFormula::DoubledIfTargetHurt),
        MoveId::STOMPING_TANTRUM | MoveId::TEMPER_FLARE => {
            formula(PowerFormula::DoubledAfterFailure)
        }
        MoveId::RAGE_FIST => formula(PowerFormula::TimesAttacked),
        MoveId::LAST_RESPECTS => formula(PowerFormula::FaintedAllies),
        MoveId::RETALIATE => boost(PowerCondition::AllyFaintedLastTurn, 8192),
        MoveId::LASH_OUT => boost(PowerCondition::UserStatsLoweredThisTurn, 8192),
        MoveId::FUSION_FLARE => boost(PowerCondition::AfterMove(MoveId::FUSION_BOLT), 8192),
        MoveId::FUSION_BOLT => boost(PowerCondition::AfterMove(MoveId::FUSION_FLARE), 8192),
        MoveId::THUNDER | MoveId::HURRICANE => accuracy(WeatherAccuracy::RainSureSunHalf),
        MoveId::BLEAKWIND_STORM | MoveId::WILDBOLT_STORM | MoveId::SANDSEAR_STORM => {
            accuracy(WeatherAccuracy::RainSure)
        }
        MoveId::BLIZZARD => accuracy(WeatherAccuracy::SnowSure),
        MoveId::GROWTH => MoveBehavior {
            sun_doubles_boosts: true,
            ..MoveBehavior::default()
        },
        MoveId::SPIT_UP => MoveBehavior {
            power_formula: Some(PowerFormula::StockpileLayers),
            try_use: Some(TryUse::Stockpiled),
            ends_stockpile: true,
            ..MoveBehavior::default()
        },
        MoveId::CLEAR_SMOG => MoveBehavior {
            clears_target_stages: true,
            ..MoveBehavior::default()
        },
        MoveId::HEAL_PULSE => MoveBehavior {
            target_heal: Some(TargetHeal {
                more_when: TargetHealWhen::UserAbility(crate::ids::AbilityId::MEGA_LAUNCHER),
                modifier: 3072,
            }),
            ..MoveBehavior::default()
        },
        MoveId::FLORAL_HEALING => MoveBehavior {
            target_heal: Some(TargetHeal {
                more_when: TargetHealWhen::Terrain(crate::state::TerrainKind::Grassy),
                modifier: 2732,
            }),
            ..MoveBehavior::default()
        },
        MoveId::BURN_UP => MoveBehavior {
            burns_up: Some(crate::dex::PokemonType::Fire),
            ..MoveBehavior::default()
        },
        MoveId::DOUBLE_SHOCK => MoveBehavior {
            burns_up: Some(crate::dex::PokemonType::Electric),
            ..MoveBehavior::default()
        },
        MoveId::RECOVER | MoveId::SOFT_BOILED => MoveBehavior {
            on_hit_heal: Some(Fraction::new(1, 2)),
            ..MoveBehavior::default()
        },
        MoveId::MOONLIGHT | MoveId::MORNING_SUN | MoveId::SYNTHESIS => MoveBehavior {
            weather_heal: Some(WeatherHeal::Sunlight),
            ..MoveBehavior::default()
        },
        MoveId::SHORE_UP => MoveBehavior {
            weather_heal: Some(WeatherHeal::Sand),
            ..MoveBehavior::default()
        },
        MoveId::SUPER_FANG | MoveId::RUINATION | MoveId::NATURES_MADNESS => MoveBehavior {
            damage_override: Some(crate::dex::FixedDamage::HalfTargetHp),
            ..MoveBehavior::default()
        },
        MoveId::SUCKER_PUNCH | MoveId::THUNDERCLAP => MoveBehavior {
            try_use: Some(TryUse::TargetAttacksNext),
            ..MoveBehavior::default()
        },
        MoveId::ROLLOUT | MoveId::ICE_BALL => MoveBehavior {
            rolling: true,
            ..formula(PowerFormula::Rolling)
        },
        MoveId::PURSUIT => MoveBehavior {
            marks_pursuit: true,
            sure_hit_on_switching: true,
            ..formula(PowerFormula::DoubledAgainstSwitching)
        },
        MoveId::UPPER_HAND => MoveBehavior {
            try_use: Some(TryUse::TargetUsesPriorityMove),
            ..MoveBehavior::default()
        },
        MoveId::COUNTER => MoveBehavior {
            turn_start_volatile: Some(crate::state::VolatileConditionKind::Counter),
            ..retaliation(crate::dex::Retaliation::Counter)
        },
        MoveId::MIRROR_COAT => MoveBehavior {
            turn_start_volatile: Some(crate::state::VolatileConditionKind::MirrorCoat),
            ..retaliation(crate::dex::Retaliation::MirrorCoat)
        },
        MoveId::METAL_BURST | MoveId::COMEUPPANCE => {
            retaliation(crate::dex::Retaliation::MetalBurst)
        }
        MoveId::FURY_CUTTER => formula(PowerFormula::FuryCutter),
        MoveId::LAST_RESORT => MoveBehavior {
            try_use: Some(TryUse::OtherMovesUsed),
            ..MoveBehavior::default()
        },
        MoveId::FAKE_OUT | MoveId::FIRST_IMPRESSION => MoveBehavior {
            try_use: Some(TryUse::FirstAction),
            ..MoveBehavior::default()
        },
        MoveId::FLY => MoveBehavior {
            two_turn: Some(TwoTurnSpec {
                skip: ChargeSkip::Never,
                invulnerable: Some(SemiInvulnerableKind::Flying),
                charge_boost: None,
                bonus_stage: DamageBonusStage::FinalDamage,
            }),
            charge_fails_asleep: true,
            ..MoveBehavior::default()
        },
        MoveId::DIG => MoveBehavior {
            two_turn: Some(TwoTurnSpec {
                skip: ChargeSkip::Never,
                invulnerable: Some(SemiInvulnerableKind::Underground),
                charge_boost: None,
                bonus_stage: DamageBonusStage::FinalDamage,
            }),
            charge_fails_asleep: true,
            ..MoveBehavior::default()
        },
        // Bounce hides in the air like Fly, but its condition doubles Gust / Twister at the power
        // (`onSourceBasePower`, issues.md I60); Dive hides under water (Surf, Whirlpool reach it
        // and deal double); Phantom Force and Shadow Force vanish (`onInvulnerability: false`)
        // and break through protection as they strike (`breaksProtect`, the snapshot's).
        MoveId::BOUNCE => MoveBehavior {
            two_turn: Some(TwoTurnSpec {
                skip: ChargeSkip::Never,
                invulnerable: Some(SemiInvulnerableKind::Flying),
                charge_boost: None,
                bonus_stage: DamageBonusStage::BasePower,
            }),
            ..MoveBehavior::default()
        },
        MoveId::DIVE => MoveBehavior {
            two_turn: Some(TwoTurnSpec {
                skip: ChargeSkip::Never,
                invulnerable: Some(SemiInvulnerableKind::Underwater),
                charge_boost: None,
                bonus_stage: DamageBonusStage::FinalDamage,
            }),
            ..MoveBehavior::default()
        },
        MoveId::PHANTOM_FORCE | MoveId::SHADOW_FORCE => MoveBehavior {
            two_turn: Some(TwoTurnSpec {
                skip: ChargeSkip::Never,
                invulnerable: Some(SemiInvulnerableKind::Vanished),
                charge_boost: None,
                bonus_stage: DamageBonusStage::FinalDamage,
            }),
            ..MoveBehavior::default()
        },
        // Solar Blade charges and weakens like Solar Beam (its own `onTryMove` / `onBasePower`).
        MoveId::SOLAR_BEAM | MoveId::SOLAR_BLADE => MoveBehavior {
            two_turn: Some(TwoTurnSpec {
                skip: ChargeSkip::InSun,
                invulnerable: None,
                charge_boost: None,
                bonus_stage: DamageBonusStage::FinalDamage,
            }),
            power_boost: Some(PowerBoost {
                condition: PowerCondition::WeakeningWeather,
                modifier: 2048,
            }),
            charge_fails_asleep: true,
            ..MoveBehavior::default()
        },
        // Charging moves with nothing on the charging turn but `-prepare` (§15.5): Razor Wind and
        // Sky Attack (Gen 1-2 also declare `onPrepareHit`), Freeze Shock, Ice Burn, Geomancy (a
        // status move: its boosts come on the second turn).
        MoveId::RAZOR_WIND
        | MoveId::SKY_ATTACK
        | MoveId::FREEZE_SHOCK
        | MoveId::ICE_BURN
        | MoveId::GEOMANCY => MoveBehavior {
            two_turn: Some(TwoTurnSpec {
                skip: ChargeSkip::Never,
                invulnerable: None,
                charge_boost: None,
                bonus_stage: DamageBonusStage::FinalDamage,
            }),
            charge_fails_asleep: true,
            ..MoveBehavior::default()
        },
        // Charging moves that raise the user's stat as they charge (`this.boost` in `onTryMove`,
        // §15.5): Skull Bash's Defense (Gen 1's has none: its gen1 `onTryMove`; Gen 1-2 also
        // declare `onPrepareHit`), Meteor Beam's and Electro Shot's Special Attack — Electro Shot
        // then attacks at once in the rain.
        MoveId::THRASH | MoveId::PETAL_DANCE | MoveId::OUTRAGE | MoveId::RAGING_FURY => {
            MoveBehavior {
                rampage: true,
                rampage_lock_at_start: true,
                ..MoveBehavior::default()
            }
        }
        MoveId::UPROAR => MoveBehavior {
            uproar: true,
            ..MoveBehavior::default()
        },
        MoveId::RAGE => MoveBehavior {
            rage: true,
            ..MoveBehavior::default()
        },
        MoveId::GLAIVE_RUSH => MoveBehavior {
            glaive_rush: true,
            ..MoveBehavior::default()
        },
        MoveId::IMPRISON => MoveBehavior {
            imprison_needs_shared_move: true,
            ..MoveBehavior::default()
        },
        MoveId::SKY_DROP => MoveBehavior {
            sky_drop: true,
            ..MoveBehavior::default()
        },
        MoveId::FUTURE_SIGHT | MoveId::DOOM_DESIRE => MoveBehavior {
            future_attack: true,
            ..MoveBehavior::default()
        },
        MoveId::SKULL_BASH => MoveBehavior {
            charge_fails_asleep: true,
            ..charging_boost(crate::state::Stat::Defense, ChargeSkip::Never)
        },
        MoveId::METEOR_BEAM => charging_boost(crate::state::Stat::SpecialAttack, ChargeSkip::Never),
        MoveId::ELECTRO_SHOT => {
            charging_boost(crate::state::Stat::SpecialAttack, ChargeSkip::InRain)
        }
        MoveId::BRICK_BREAK | MoveId::PSYCHIC_FANGS => MoveBehavior {
            breaks_screens: true,
            ..MoveBehavior::default()
        },
        MoveId::RAGING_BULL => MoveBehavior {
            breaks_screens: true,
            own_type: Some(OwnType::UserSpecies(&[
                (
                    crate::ids::SpeciesId::TAUROS_PALDEA_COMBAT,
                    crate::dex::PokemonType::Fighting,
                ),
                (
                    crate::ids::SpeciesId::TAUROS_PALDEA_BLAZE,
                    crate::dex::PokemonType::Fire,
                ),
                (
                    crate::ids::SpeciesId::TAUROS_PALDEA_AQUA,
                    crate::dex::PokemonType::Water,
                ),
            ])),
            ..MoveBehavior::default()
        },
        MoveId::AURORA_VEIL => MoveBehavior {
            try_use: Some(TryUse::InWeather(crate::state::WeatherKind::Snow)),
            ..MoveBehavior::default()
        },
        MoveId::GRASSY_GLIDE => MoveBehavior {
            terrain_priority: Some(crate::state::TerrainKind::Grassy),
            ..MoveBehavior::default()
        },
        MoveId::TERRAIN_PULSE => MoveBehavior {
            own_type: Some(OwnType::Terrain),
            ..MoveBehavior::default()
        },
        MoveId::EXPANDING_FORCE => MoveBehavior {
            power_boost: Some(PowerBoost {
                condition: PowerCondition::GroundedInTerrain(crate::state::TerrainKind::Psychic),
                modifier: 6144,
            }),
            terrain_spread: Some(crate::state::TerrainKind::Psychic),
            ..MoveBehavior::default()
        },
        MoveId::SECRET_POWER => MoveBehavior {
            terrain_secondaries: true,
            ..MoveBehavior::default()
        },
        // Gen 2-4 (the gen4 mod's `self.onHit`) clear as the self effects land; the inherited
        // `onAfterHit` then finds nothing left.
        MoveId::RAPID_SPIN => MoveBehavior {
            after_hit: Some(AfterHitEffect::ClearHazards),
            self_clears_hazards: true,
            ..MoveBehavior::default()
        },
        MoveId::MORTAL_SPIN => MoveBehavior {
            after_hit: Some(AfterHitEffect::ClearHazards),
            ..MoveBehavior::default()
        },
        MoveId::CHILLY_RECEPTION => MoveBehavior {
            turn_start_volatile: Some(crate::state::VolatileConditionKind::ChillyReception),
            ..MoveBehavior::default()
        },
        // Gen 5+ `priorityChargeCallback` + `beforeMoveCallback`, Gen 3-4 `beforeTurnCallback` +
        // `onTry`: the generation's data keeps one pair.
        MoveId::FOCUS_PUNCH => MoveBehavior {
            turn_start_volatile: Some(crate::state::VolatileConditionKind::FocusPunch),
            lost_focus_stops: true,
            lost_focus_fails: true,
            ..MoveBehavior::default()
        },
        MoveId::BEAK_BLAST => MoveBehavior {
            turn_start_volatile: Some(crate::state::VolatileConditionKind::BeakBlast),
            ends_turn_start_volatile: true,
            ..MoveBehavior::default()
        },
        MoveId::SHELL_TRAP => MoveBehavior {
            turn_start_volatile: Some(crate::state::VolatileConditionKind::ShellTrap),
            needs_armed_trap: true,
            ..MoveBehavior::default()
        },
        MoveId::OCTOLOCK => MoveBehavior {
            try_immunity: Some(TryImmunity::Trappable),
            ..MoveBehavior::default()
        },
        // The gen1 mod's `onTryMove` (a recharging target's turn ends) and `self` lock; later
        // generations bind through the move's `volatileStatus` alone.
        MoveId::BIND | MoveId::WRAP | MoveId::CLAMP | MoveId::FIRE_SPIN => MoveBehavior {
            binding_lock: true,
            ..MoveBehavior::default()
        },
        MoveId::THOUSAND_WAVES => MoveBehavior {
            hit_traps: Some(HitTrap::Target),
            ..MoveBehavior::default()
        },
        MoveId::JAW_LOCK => MoveBehavior {
            hit_traps: Some(HitTrap::Mutual),
            ..MoveBehavior::default()
        },
        // `secondary: { chance: 100, onHit }`: Shield Dust, Covert Cloak and Sheer Force take it
        // (DESIGN.md §15.15).
        MoveId::EERIE_SPELL => MoveBehavior {
            secondary_on_hit: Some(SecondaryOnHit {
                effect: crate::dex::SecondaryEffect::EerieSpell,
                requires: None,
            }),
            ..MoveBehavior::default()
        },
        // `secondary: { chance: 100, onHit }`: Shield Dust, Covert Cloak and Sheer Force take it.
        MoveId::ANCHOR_SHOT | MoveId::SPIRIT_SHACKLE => MoveBehavior {
            secondary_on_hit: Some(SecondaryOnHit {
                effect: crate::dex::SecondaryEffect::Volatile(
                    crate::state::VolatileConditionKind::Trapped,
                ),
                requires: None,
            }),
            ..MoveBehavior::default()
        },
        MoveId::GUARDIAN_OF_ALOLA => MoveBehavior {
            damage_override: Some(crate::dex::FixedDamage::ThreeQuartersTargetHp),
            ..MoveBehavior::default()
        },
        MoveId::MAX_AIRSTREAM => {
            max_effect(MaxMoveEffect::TeamBoost(&[(crate::state::Stat::Speed, 1)]))
        }
        MoveId::MAX_KNUCKLE => {
            max_effect(MaxMoveEffect::TeamBoost(&[(crate::state::Stat::Attack, 1)]))
        }
        MoveId::MAX_OOZE => max_effect(MaxMoveEffect::TeamBoost(&[(
            crate::state::Stat::SpecialAttack,
            1,
        )])),
        MoveId::MAX_STEELSPIKE => max_effect(MaxMoveEffect::TeamBoost(&[(
            crate::state::Stat::Defense,
            1,
        )])),
        MoveId::MAX_QUAKE => max_effect(MaxMoveEffect::TeamBoost(&[(
            crate::state::Stat::SpecialDefense,
            1,
        )])),
        MoveId::MAX_STRIKE => {
            max_effect(MaxMoveEffect::FoeDrop(&[(crate::state::Stat::Speed, -1)]))
        }
        MoveId::MAX_DARKNESS => max_effect(MaxMoveEffect::FoeDrop(&[(
            crate::state::Stat::SpecialDefense,
            -1,
        )])),
        MoveId::MAX_FLUTTERBY => max_effect(MaxMoveEffect::FoeDrop(&[(
            crate::state::Stat::SpecialAttack,
            -1,
        )])),
        MoveId::MAX_PHANTASM => {
            max_effect(MaxMoveEffect::FoeDrop(&[(crate::state::Stat::Defense, -1)]))
        }
        MoveId::MAX_WYRMWIND => {
            max_effect(MaxMoveEffect::FoeDrop(&[(crate::state::Stat::Attack, -1)]))
        }
        MoveId::MAX_FLARE => max_effect(MaxMoveEffect::Weather(crate::state::WeatherKind::Sun)),
        MoveId::MAX_GEYSER => max_effect(MaxMoveEffect::Weather(crate::state::WeatherKind::Rain)),
        MoveId::MAX_HAILSTORM => {
            max_effect(MaxMoveEffect::Weather(crate::state::WeatherKind::Snow))
        }
        MoveId::MAX_ROCKFALL => {
            max_effect(MaxMoveEffect::Weather(crate::state::WeatherKind::Sandstorm))
        }
        MoveId::MAX_LIGHTNING => {
            max_effect(MaxMoveEffect::Terrain(crate::state::TerrainKind::Electric))
        }
        MoveId::MAX_MINDSTORM => {
            max_effect(MaxMoveEffect::Terrain(crate::state::TerrainKind::Psychic))
        }
        MoveId::MAX_OVERGROWTH => {
            max_effect(MaxMoveEffect::Terrain(crate::state::TerrainKind::Grassy))
        }
        MoveId::MAX_STARFALL => {
            max_effect(MaxMoveEffect::Terrain(crate::state::TerrainKind::Misty))
        }
        MoveId::GENESIS_SUPERNOVA => MoveBehavior {
            z_terrain: Some(ZTerrain::Psychic),
            ..MoveBehavior::default()
        },
        MoveId::SPLINTERED_STORMSHARDS => MoveBehavior {
            hit_or_sub: Some(HitOrSubEffect::ClearTerrain),
            ..MoveBehavior::default()
        },
        MoveId::ICE_SPINNER => MoveBehavior {
            after_hit: Some(AfterHitEffect::ClearTerrain),
            ..MoveBehavior::default()
        },
        MoveId::STEEL_ROLLER => MoveBehavior {
            try_use: Some(TryUse::InTerrain),
            hit_or_sub: Some(HitOrSubEffect::ClearTerrain),
            ..MoveBehavior::default()
        },
        MoveId::CORE_ENFORCER => MoveBehavior {
            hit_or_sub: Some(HitOrSubEffect::SuppressAbilityOfMoved),
            ..MoveBehavior::default()
        },
        MoveId::FLAME_BURST => MoveBehavior {
            hit_or_sub: Some(HitOrSubEffect::SplashAdjacentAllies),
            ..MoveBehavior::default()
        },
        MoveId::PLASMA_FISTS => MoveBehavior {
            starts_ion_deluge: true,
            ..MoveBehavior::default()
        },
        MoveId::GRAV_APPLE => MoveBehavior {
            power_boost: Some(PowerBoost {
                condition: PowerCondition::FieldCondition(
                    crate::state::FieldConditionKind::Gravity,
                ),
                modifier: 6144,
            }),
            ..MoveBehavior::default()
        },
        MoveId::STONE_AXE => MoveBehavior {
            after_hit: Some(AfterHitEffect::LayHazard(
                crate::state::SideConditionKind::StealthRock,
            )),
            ..MoveBehavior::default()
        },
        MoveId::FIRE_PLEDGE | MoveId::GRASS_PLEDGE | MoveId::WATER_PLEDGE => MoveBehavior {
            pledge_combo: true,
            ..MoveBehavior::default()
        },
        MoveId::CEASELESS_EDGE => MoveBehavior {
            after_hit: Some(AfterHitEffect::LayHazard(
                crate::state::SideConditionKind::Spikes,
            )),
            ..MoveBehavior::default()
        },
        _ => MoveBehavior::default(),
    }
}

/// A custom primary (DESIGN.md §15.2, §25): the code of the move's PrimaryEffect stage. The
/// Shell still owns everything around it — targeting, TryUse, the hit gate (TryHit, accuracy),
/// secondaries, self-switch and the switch / faint lifecycle. It names the upstream data it
/// consumes; anything else the snapshot declares for the move keeps it Unsupported, so a new
/// Showdown key or callback is never silently claimed.
#[derive(Clone, Copy)]
pub struct CustomPrimary {
    pub handler: ExecuteMoveHandler,
    /// Its `onTry` / `onPrepareHit`, before the `PrepareHit` handlers.
    pub tries: Option<TryPrimaryHandler>,
    /// The Showdown callbacks the handler stands for (upstream terms, data only).
    pub source_callbacks: &'static [&'static str],
    /// The Showdown data keys the handler consumes (`volatileStatus`, `sideCondition`, the
    /// effect's `condition` table …).
    pub source_keys: &'static [&'static str],
}

const fn primary(
    handler: ExecuteMoveHandler,
    source_callbacks: &'static [&'static str],
    source_keys: &'static [&'static str],
) -> Option<CustomPrimary> {
    Some(CustomPrimary {
        handler,
        tries: None,
        source_callbacks,
        source_keys,
    })
}

/// A custom primary whose `onTry` / `onPrepareHit` (`tries`) can fail it before `PrepareHit`.
const fn tried(
    handler: ExecuteMoveHandler,
    tries: TryPrimaryHandler,
    source_callbacks: &'static [&'static str],
    source_keys: &'static [&'static str],
) -> Option<CustomPrimary> {
    Some(CustomPrimary {
        handler,
        tries: Some(tries),
        source_callbacks,
        source_keys,
    })
}

/// Custom primaries whose effect cures other Pokémon's status (Jungle Healing, Lunar Blessing,
/// Worry Seed's wake-up): their `-curestatus` belongs to the move, not to the cured Pokémon's
/// own turn (the replay reads a move's log by this).
pub fn cures_targets(move_id: MoveId) -> bool {
    matches!(
        move_id,
        MoveId::JUNGLE_HEALING | MoveId::LUNAR_BLESSING | MoveId::WORRY_SEED
    )
}

/// Custom primaries whose `onTry` lets a snatched move through (`move.sourceEffect ===
/// 'snatch'`): Swallow, under `ConditionRules::snatched_swallow_skips_stockpile`.
pub fn tries_pass_snatched(move_id: MoveId) -> bool {
    move_id == MoveId::SWALLOW
}

/// The custom primaries, each with the upstream data it consumes (in every generation that
/// has the move; Gen 3 Follow Me is a `slotCondition`, later ones a `volatileStatus`).
pub fn custom(move_id: MoveId) -> Option<CustomPrimary> {
    const SIDE: &[&str] = &["condition", "sideCondition"];
    const VOLATILE: &[&str] = &["condition", "volatileStatus"];
    const TERRAIN: &[&str] = &["condition", "terrain"];
    const PSEUDO_WEATHER: &[&str] = &["condition", "pseudoWeather"];
    const SPORT: &[&str] = &["condition", "pseudoWeather", "volatileStatus"];
    // A stalling move: its `onPrepareHit` (the roll), `onHit` (the counter) and its volatile's
    // `condition` (the block, its contact punishment, Endure's `onDamage`).
    const STALL_CALLBACKS: &[&str] = &["onPrepareHit", "onHit"];
    const STALL_KEYS: &[&str] = &["condition", "stallingMove", "volatileStatus"];
    match move_id {
        MoveId::PROTECT => tried(
            protect::execute,
            protect::stalls,
            &["onPrepareHit", "onHit"],
            &["condition", "stallingMove", "volatileStatus"],
        ),
        // Detect sets Protect's volatile and declares no `condition` of its own.
        MoveId::DETECT => tried(
            protect::execute,
            protect::stalls,
            &["onPrepareHit", "onHit"],
            &["stallingMove", "volatileStatus"],
        ),
        MoveId::KINGS_SHIELD => tried(
            protect::kings_shield,
            protect::stalls,
            STALL_CALLBACKS,
            STALL_KEYS,
        ),
        MoveId::SPIKY_SHIELD => tried(
            protect::spiky_shield,
            protect::stalls,
            STALL_CALLBACKS,
            STALL_KEYS,
        ),
        MoveId::BANEFUL_BUNKER => tried(
            protect::baneful_bunker,
            protect::stalls,
            STALL_CALLBACKS,
            STALL_KEYS,
        ),
        MoveId::OBSTRUCT => tried(
            protect::obstruct,
            protect::stalls,
            STALL_CALLBACKS,
            STALL_KEYS,
        ),
        MoveId::SILK_TRAP => tried(
            protect::silk_trap,
            protect::stalls,
            STALL_CALLBACKS,
            STALL_KEYS,
        ),
        MoveId::BURNING_BULWARK => tried(
            protect::burning_bulwark,
            protect::stalls,
            STALL_CALLBACKS,
            STALL_KEYS,
        ),
        MoveId::MAX_GUARD => tried(
            protect::max_guard,
            protect::stalls,
            STALL_CALLBACKS,
            STALL_KEYS,
        ),
        MoveId::ENDURE => tried(
            protect::endure,
            protect::stalls,
            STALL_CALLBACKS,
            STALL_KEYS,
        ),
        MoveId::INGRAIN => primary(ingrain::execute, &[], VOLATILE),
        MoveId::SUBSTITUTE => primary(substitute::execute, &["onTryHit", "onHit"], VOLATILE),
        MoveId::STEALTH_ROCK => primary(stealth_rock::execute, &[], SIDE),
        MoveId::SPIKES => primary(spikes::spikes, &[], SIDE),
        MoveId::TOXIC_SPIKES => primary(spikes::toxic_spikes, &[], SIDE),
        MoveId::STICKY_WEB => primary(spikes::sticky_web, &[], SIDE),
        MoveId::LEECH_SEED => primary(leech_seed::execute, &[], VOLATILE),
        MoveId::DISABLE => primary(disable::execute, &["onTryHit"], VOLATILE),
        MoveId::FOLLOW_ME => tried(
            follow_me::execute,
            follow_me::tries,
            &["onTry"],
            &["condition", "slotCondition", "volatileStatus"],
        ),
        MoveId::RAGE_POWDER => tried(
            follow_me::rage_powder,
            follow_me::tries,
            &["onTry"],
            VOLATILE,
        ),
        MoveId::SPOTLIGHT => primary(follow_me::spotlight, &["onTryHit"], VOLATILE),
        MoveId::DESTINY_BOND => tried(
            faint_bonds::destiny_bond,
            faint_bonds::destiny_bond_tries,
            &["onPrepareHit"],
            VOLATILE,
        ),
        MoveId::GRUDGE => primary(faint_bonds::grudge, &[], VOLATILE),
        MoveId::BIDE => primary(bide::start, &["beforeMoveCallback"], VOLATILE),
        MoveId::TAILWIND => primary(tailwind::execute, &[], SIDE),
        MoveId::TRICK_ROOM => primary(trick_room::execute, &[], &["condition", "pseudoWeather"]),
        MoveId::ELECTRIC_TERRAIN => primary(terrain::electric, &[], TERRAIN),
        MoveId::GRASSY_TERRAIN => primary(terrain::grassy, &[], TERRAIN),
        MoveId::MISTY_TERRAIN => primary(terrain::misty, &[], TERRAIN),
        MoveId::PSYCHIC_TERRAIN => primary(terrain::psychic, &[], TERRAIN),
        MoveId::WIDE_GUARD => tried(
            guard::wide_guard,
            guard::guard_tries,
            &["onTry", "onHitSide"],
            &["condition", "sideCondition", "stallingMove"],
        ),
        MoveId::QUICK_GUARD => tried(
            guard::quick_guard,
            guard::guard_tries,
            &["onTry", "onHitSide"],
            &["condition", "sideCondition", "stallingMove"],
        ),
        MoveId::CRAFTY_SHIELD => tried(
            guard::crafty_shield,
            guard::crafty_shield_tries,
            &["onTry"],
            SIDE,
        ),
        MoveId::MAT_BLOCK => tried(
            guard::mat_block,
            guard::mat_block_tries,
            &["onTry"],
            &["condition", "sideCondition", "stallingMove"],
        ),
        MoveId::HELPING_HAND => primary(helping_hand::execute, &["onTryHit"], VOLATILE),
        MoveId::HEAL_BELL | MoveId::AROMATHERAPY => primary(heal_bell::execute, &["onHit"], &[]),
        MoveId::SKILL_SWAP => primary(ability_change::skill_swap, &["onHit"], &[]),
        MoveId::ROLE_PLAY => primary(ability_change::role_play, &["onTryHit", "onHit"], &[]),
        MoveId::ENTRAINMENT => primary(ability_change::entrainment, &["onTryHit", "onHit"], &[]),
        MoveId::SIMPLE_BEAM => primary(ability_change::simple_beam, &["onTryHit", "onHit"], &[]),
        MoveId::WORRY_SEED => primary(ability_change::worry_seed, &["onTryHit", "onHit"], &[]),
        MoveId::DOODLE => primary(ability_change::doodle, &["onHit"], &[]),
        MoveId::SOAK => primary(types::soak, &["onHit"], &[]),
        MoveId::MAGIC_POWDER => primary(types::magic_powder, &["onHit"], &[]),
        MoveId::CAMOUFLAGE => primary(types::camouflage, &["onHit"], &[]),
        MoveId::FORESTS_CURSE => primary(types::forests_curse, &["onHit"], &[]),
        MoveId::TRICK_OR_TREAT => primary(types::trick_or_treat, &["onHit"], &[]),
        MoveId::REFLECT_TYPE => primary(types::reflect_type, &["onHit"], &[]),
        MoveId::ELECTRIFY => primary(types::electrify, &["onTryHit"], VOLATILE),
        MoveId::CONVERSION => primary(types::conversion, &["onHit"], &[]),
        MoveId::CONVERSION_2 => primary(types::conversion2, &["onHit"], &[]),
        MoveId::REFRESH => primary(refresh::execute, &["onHit"], &[]),
        MoveId::REST => tried(rest::execute, rest::tries, &["onTry", "onHit"], &[]),
        // Their `onTryHit` (Gen 2 the target's Foresight / Lock-On, Gen 3+ the user's lock) and
        // `onHit`, and the `lockon` condition, as the generation keeps the lock
        // (`LockOnRules`).
        MoveId::LOCK_ON => primary(lock_on::execute, &["onTryHit", "onHit"], &["condition"]),
        MoveId::MIND_READER => primary(lock_on::execute, &["onTryHit", "onHit"], &[]),
        MoveId::SPITE => primary(spite::execute, &["onHit"], &[]),
        // Gen 2's `onTryHit` (the target's own Foresight) and Gen 5+'s (the other move's). Odor
        // Sleuth's `volatileStatus` is Foresight's condition.
        MoveId::FORESIGHT => primary(identify::foresight, &["onTryHit"], VOLATILE),
        MoveId::ODOR_SLEUTH => primary(identify::foresight, &["onTryHit"], &["volatileStatus"]),
        MoveId::MIRACLE_EYE => primary(identify::miracle_eye, &["onTryHit"], VOLATILE),
        // Their `onTry` (Gravity's, for a Z-move: Gravity's own `onBeforeMove` stops the move
        // otherwise) and conditions; Magnet Rise's refusal of a held-down user is its `tries`.
        MoveId::MAGNET_RISE => tried(
            grounding::magnet_rise,
            grounding::magnet_rise_tries,
            &["onTry"],
            VOLATILE,
        ),
        MoveId::TELEKINESIS => primary(grounding::telekinesis, &["onTry"], VOLATILE),
        MoveId::CURSE => primary(
            curse::execute,
            &["onModifyMove", "onTryHit", "onHit"],
            &["condition", "volatileStatus"],
        ),
        MoveId::BELLY_DRUM => primary(hp_cost_boosts::belly_drum, &["onHit"], &[]),
        MoveId::STOCKPILE => tried(
            stockpile::stockpile,
            stockpile::stockpile_tries,
            &["onTry"],
            VOLATILE,
        ),
        MoveId::NO_RETREAT => tried(
            self_boosts::no_retreat,
            self_boosts::no_retreat_tries,
            &["onTry"],
            &["condition", "volatileStatus", "boosts"],
        ),
        MoveId::TIDY_UP => primary(self_boosts::tidy_up, &["onHit"], &[]),
        MoveId::AUTOTOMIZE => primary(
            self_boosts::autotomize,
            &["onTryHit", "onHit"],
            &["boosts", "condition", "volatileStatus"],
        ),
        MoveId::PAIN_SPLIT => primary(pain_split::execute, &["onHit"], &[]),
        MoveId::ACUPRESSURE => primary(acupressure::execute, &["onHit"], &[]),
        MoveId::POWER_SPLIT => primary(stat_swaps::power_split, &["onHit"], &[]),
        MoveId::GUARD_SPLIT => primary(stat_swaps::guard_split, &["onHit"], &[]),
        MoveId::SPEED_SWAP => primary(stat_swaps::speed_swap, &["onHit"], &[]),
        MoveId::POWER_TRICK => primary(stat_swaps::power_trick, &[], VOLATILE),
        MoveId::SWALLOW => tried(
            stockpile::swallow,
            stockpile::swallow_tries,
            &["onTry", "onHit"],
            &[],
        ),
        MoveId::FLOWER_SHIELD => primary(field_boosts::flower_shield, &["onHitField"], &[]),
        MoveId::ROTOTILLER => primary(field_boosts::rototiller, &["onHitField"], &[]),
        MoveId::GEAR_UP => primary(field_boosts::gear_up, &["onHitSide"], &[]),
        MoveId::MAGNETIC_FLUX => primary(field_boosts::magnetic_flux, &["onHitSide"], &[]),
        MoveId::VENOM_DRENCH => primary(field_boosts::venom_drench, &["onHit"], &[]),
        MoveId::HAPPY_HOUR => primary(field_boosts::happy_hour, &["onTryHit"], &[]),
        MoveId::JUNGLE_HEALING => primary(heal_and_cure::jungle_healing, &["onHit"], &[]),
        MoveId::LUNAR_BLESSING => primary(heal_and_cure::lunar_blessing, &["onHit"], &[]),
        MoveId::TAKE_HEART => primary(heal_and_cure::take_heart, &["onHit"], &[]),
        MoveId::STRENGTH_SAP => primary(heal_and_cure::strength_sap, &["onHit"], &[]),
        MoveId::FILLET_AWAY => tried(
            hp_cost_boosts::fillet_away,
            hp_cost_boosts::fillet_away_tries,
            &["onTry", "onTryHit", "onHit"],
            &["boosts"],
        ),
        MoveId::CLANGOROUS_SOUL => tried(
            hp_cost_boosts::clangorous_soul,
            hp_cost_boosts::clangorous_soul_tries,
            &["onTry", "onTryHit", "onHit"],
            &["boosts"],
        ),
        // Gen 1's Haze is an `onHit` on the user (`ConditionRules::haze`).
        MoveId::HAZE => primary(stage_swaps::haze, &["onHitField", "onHit"], &[]),
        MoveId::PSYCH_UP => primary(stage_swaps::psych_up, &["onHit"], &[]),
        MoveId::TRANSFORM => primary(transform::transform, &["onHit"], &[]),
        MoveId::HEART_SWAP => primary(stage_swaps::heart_swap, &["onHit"], &[]),
        MoveId::POWER_SWAP => primary(stage_swaps::power_swap, &["onHit"], &[]),
        MoveId::GUARD_SWAP => primary(stage_swaps::guard_swap, &["onHit"], &[]),
        MoveId::TOPSY_TURVY => primary(stage_swaps::topsy_turvy, &["onHit"], &[]),
        // Gen 1's are volatiles on the user whose `onTryHit` fails when one is already up.
        MoveId::REFLECT => primary(
            screens::reflect,
            &["onTryHit"],
            &["condition", "sideCondition", "volatileStatus"],
        ),
        MoveId::LIGHT_SCREEN => primary(
            screens::light_screen,
            &["onTryHit"],
            &["condition", "sideCondition", "volatileStatus"],
        ),
        MoveId::AURORA_VEIL => primary(screens::aurora_veil, &[], SIDE),
        // Gen 1-2 Mist is a volatile on the user.
        MoveId::MIST => primary(
            screens::mist,
            &[],
            &["condition", "sideCondition", "volatileStatus"],
        ),
        MoveId::SAFEGUARD => primary(screens::safeguard, &[], SIDE),
        MoveId::LUCKY_CHANT => primary(screens::lucky_chant, &[], SIDE),
        MoveId::PERISH_SONG => primary(switching::perish_song, &["onHitField"], &["condition"]),
        // Parting Shot sends its user out itself (Gen 7+: only after a drop).
        MoveId::PARTING_SHOT => primary(switching::parting_shot, &["onHit"], &["selfSwitch"]),
        MoveId::CHILLY_RECEPTION => {
            primary(switching::chilly_reception, &[], &["condition", "weather"])
        }
        // Gen 1-7's `onTry` is `false`.
        MoveId::TELEPORT => tried(
            switching::teleport,
            switching::teleport_tries,
            &["onTry"],
            &[],
        ),
        // Their `self.onHit` only spares the user Pursuit (`skipBeforeSwitchOutEventFlag`).
        MoveId::BATON_PASS => primary(switching::baton_pass, &["onHit", "self:onHit"], &[]),
        MoveId::SHED_TAIL => primary(
            switching::shed_tail,
            &["onTryHit", "onHit", "self:onHit"],
            &["volatileStatus"],
        ),
        MoveId::GRAVITY => primary(field::gravity, &[], PSEUDO_WEATHER),
        MoveId::WISH => primary(field::wish, &[], &["condition", "slotCondition"]),
        // Gen 4's `onAfterMove` is the slot's refill at once (`SlotHealRules::Immediate`).
        MoveId::HEALING_WISH => primary(
            field::healing_wish,
            &["onTryHit", "onAfterMove"],
            &["condition", "slotCondition"],
        ),
        MoveId::LUNAR_DANCE => primary(
            field::lunar_dance,
            &["onTryHit", "onAfterMove"],
            &["condition", "slotCondition"],
        ),
        // Its `selfSwitch` only makes Showdown ask the side (`BattleRequest::Revive`).
        MoveId::REVIVAL_BLESSING => primary(
            field::revival_blessing,
            &["onTryHit"],
            &["condition", "slotCondition", "selfSwitch"],
        ),
        MoveId::WONDER_ROOM => primary(field::wonder_room, &[], PSEUDO_WEATHER),
        MoveId::MAGIC_ROOM => primary(field::magic_room, &[], PSEUDO_WEATHER),
        MoveId::CORROSIVE_GAS => primary(corrosive_gas::corrosive_gas, &["onHit"], &[]),
        MoveId::TRICK => primary(trick::trick, &["onHit"], &[]),
        MoveId::SWITCHEROO => primary(trick::switcheroo, &["onHit"], &[]),
        MoveId::BESTOW => primary(trick::bestow, &["onHit"], &[]),
        MoveId::RECYCLE => primary(trick::recycle, &["onHit"], &[]),
        MoveId::STUFF_CHEEKS => primary(eating::stuff_cheeks, &["onHit"], &[]),
        MoveId::TEATIME => primary(eating::teatime, &["onHitField"], &[]),
        MoveId::ION_DELUGE => primary(field::ion_deluge, &[], PSEUDO_WEATHER),
        MoveId::FAIRY_LOCK => primary(field::fairy_lock, &[], PSEUDO_WEATHER),
        // Gen 3-5 a `volatileStatus` on the user, Gen 6-7 a `pseudoWeather`.
        MoveId::MUD_SPORT => primary(field::mud_sport, &[], SPORT),
        MoveId::WATER_SPORT => primary(field::water_sport, &[], SPORT),
        MoveId::DEFOG => primary(hazards::defog, &["onHit"], &[]),
        MoveId::MEAN_LOOK | MoveId::BLOCK | MoveId::SPIDER_WEB => {
            primary(trapping::mean_look, &["onHit"], &[])
        }
        MoveId::OCTOLOCK => primary(trapping::octolock, &[], VOLATILE),
        MoveId::COURT_CHANGE => primary(hazards::court_change, &["onHitField"], &[]),
        // Magic Coat's reflection hooks are its `condition`'s (`onTryHit`, Gen 5+
        // `onAllyTryHitSide`), which the hit gate runs (§42.8), not callbacks of the move.
        MoveId::MAGIC_COAT => primary(magic_coat::execute, &[], VOLATILE),
        // Snatch's steal is its `condition`'s `onAnyPrepareHit`, which the Shell runs
        // (§42.8), not a callback of the move.
        MoveId::SNATCH => primary(snatch::execute, &[], VOLATILE),
        // Me First's `onTryHit` (`callsMove`, its `condition` is the boost marker).
        MoveId::ME_FIRST => primary(me_first::execute, &["onTryHit"], &["condition"]),
        // Ally Switch's `onPrepareHit` (Gen 9's chance) and `onHit` (the swap); its
        // `condition` is the counter volatile.
        MoveId::ALLY_SWITCH => tried(
            ally_switch::execute,
            ally_switch::try_switch,
            &["onHit", "onPrepareHit"],
            &["condition"],
        ),
        // The Pledges need no custom primary: their behavior is data (the combo table in
        // `engine::pledges` and the Shell's hooks).
        MoveId::FIRE_PLEDGE | MoveId::GRASS_PLEDGE | MoveId::WATER_PLEDGE => None,
        MoveId::MIMIC => primary(copy_move_slots::mimic, &["onHit"], &[]),
        MoveId::INSTRUCT => primary(instruct::execute, &["onHit"], &[]),
        MoveId::SKETCH => primary(copy_move_slots::sketch, &["onHit"], &[]),
        MoveId::SPLASH => primary(no_effect::nothing, &["onTry", "onTryHit"], &[]),
        MoveId::CELEBRATE => primary(no_effect::nothing, &["onTryHit"], &[]),
        MoveId::HOLD_HANDS => primary(no_effect::nothing, &[], &[]),
        MoveId::METRONOME => primary(calls::metronome, &["onHit"], &[]),
        MoveId::ASSIST => primary(calls::assist, &["onHit"], &[]),
        MoveId::SLEEP_TALK => tried(
            calls::sleep_talk,
            calls::sleep_talk_tries,
            &["onHit", "onTry", "onTryHit"],
            &[],
        ),
        MoveId::COPYCAT => primary(calls::copycat, &["onHit"], &[]),
        MoveId::NATURE_POWER => primary(calls::nature_power, &["onHit", "onTryHit"], &[]),
        MoveId::MIRROR_MOVE => primary(calls::mirror_move, &["onTryHit", "onHit"], &[]),
        MoveId::AFTER_YOU => primary(queue::after_you, &["onHit"], &[]),
        MoveId::QUASH => primary(queue::quash, &["onHit"], &[]),
        _ => None,
    }
}

/// A retaliating move (DESIGN.md §15.8): its damage (`damageCallback`), its failure with nothing
/// to answer (`onTry`; Gen 2 Counter has none, its `damageCallback` fails) and where it goes.
/// A charging move whose `onTryMove` raises its user's `stat` by 1 as it charges.
fn charging_boost(stat: crate::state::Stat, skip: ChargeSkip) -> MoveBehavior {
    MoveBehavior {
        two_turn: Some(TwoTurnSpec {
            skip,
            invulnerable: None,
            charge_boost: Some((stat, 1)),
            bonus_stage: DamageBonusStage::FinalDamage,
        }),
        ..MoveBehavior::default()
    }
}

fn retaliation(kind: crate::dex::Retaliation) -> MoveBehavior {
    MoveBehavior {
        damage_override: Some(crate::dex::FixedDamage::Retaliation(kind)),
        try_use: Some(TryUse::StrikesBack(kind)),
        strikes_back_at: Some(kind),
        ..MoveBehavior::default()
    }
}

fn formula(power_formula: PowerFormula) -> MoveBehavior {
    MoveBehavior {
        power_formula: Some(power_formula),
        ..MoveBehavior::default()
    }
}

fn boost(condition: PowerCondition, modifier: u32) -> MoveBehavior {
    MoveBehavior {
        power_boost: Some(PowerBoost {
            condition,
            modifier,
        }),
        ..MoveBehavior::default()
    }
}

fn accuracy(weather_accuracy: WeatherAccuracy) -> MoveBehavior {
    MoveBehavior {
        weather_accuracy: Some(weather_accuracy),
        ..MoveBehavior::default()
    }
}

pub fn execute_custom(
    move_id: MoveId,
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    match custom(move_id) {
        Some(primary) => (primary.handler)(ctx, user, targets),
        None => Ok(()),
    }
}
