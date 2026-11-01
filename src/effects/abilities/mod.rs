mod ability_transfer;
mod absorb;
mod ally_boost;
mod bad_dreams;
mod berserk;
mod charge_abilities;
mod clear_body;
mod contrary;
mod cud_chew;
mod drop_guard;
mod entry_effects;
mod faint_boosts;
mod flash_fire;
pub(crate) mod forme;
pub(crate) mod forme_ability;
mod frisk;
mod harvest;
mod hit_boosts;
mod hit_reactions;
mod huge_power;
mod illusion;
mod imposter;
mod intimidate;
mod lightning_rod;
mod liquid_ooze;
mod magic_guard;
mod mold_breaker;
mod moody;
mod move_guards;
mod natural_cure;
mod neutralizing_gas;
mod opportunist;
mod poison_heal;
mod pressure;
mod regenerator;
mod ripen;
mod rough_skin;
mod screen_cleaner;
mod self_faint;
mod shed_skin;
mod slow_start;
mod speed_boost;
mod stage_reactions;
mod status_immunity;
mod status_tricks;
mod sturdy;
mod supreme_overlord;
mod synchronize;
mod technician;
mod telepathy;
pub mod tera;
mod terrain;
mod toxic_debris;
mod trace;
mod truant;
mod types;
mod unnerve;
mod veils;
mod weather;
mod wind;

pub use crate::effects::{ResidualHandler, ResidualParticipant, SwitchHandler};
use crate::engine::{
    BattleContext, EffectSource, OffensiveStatContext, OrderedHandler, TryHitContext, TryHitResult,
    TrySetStatusContext, TrySetStatusResult,
};
pub use crate::engine::{
    DamageResolutionHandler, DamagingHitReactionHandler, ModifyStatChangeHandler,
    TryStatChangeHandler,
};
use crate::ids::{AbilityId, PokemonId};
pub use berserk::HalfHpBoosts;
pub use moody::{lowerable, moody_stats, raisable};

use super::BehaviorComponent;
use crate::state::NonVolatileStatus;

pub type TryHitHandler = for<'a> fn(&mut TryHitContext<'a>) -> TryHitResult;
pub type TrySetStatusHandler = for<'a> fn(&TrySetStatusContext<'a>) -> TrySetStatusResult;
pub type AfterSetStatusHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, NonVolatileStatus, EffectSource);
/// Showdown `onAfterBoost`: (holder, the stat change that just moved one of its stages).
pub type AfterBoostHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, crate::engine::StatChangeRequest);
/// Showdown `onAnyAfterSetStatus`: (holder, the Pokémon that got the status, status, source).
pub type AnyAfterSetStatusHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, PokemonId, NonVolatileStatus, EffectSource);
/// Showdown `onSideConditionStart`: (holder, the condition that just started on its side).
pub type SideConditionStartHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, crate::state::SideConditionKind);
/// Showdown `onAllyTryHitSide`: (holder, the partner using the move, the move) — a move aimed
/// at the holder's own side by one of its partners.
pub type AllyTryHitSideHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, PokemonId, &crate::dex::ResolvedMove);
/// Showdown `onHit` / `onAfterSubDamage` of the target of a move: (holder, the move's user,
/// whether the hit was a critical hit).
pub type TargetHitHandler = for<'a> fn(&mut BattleContext<'a>, PokemonId, PokemonId, bool);
/// Showdown `onTryBoost` ahead of every other TryBoost handler (Guard Dog's priority 2): `true`
/// when the handler took the change over (the change is refused).
pub type TryBoostFirstHandler =
    for<'a> fn(&mut BattleContext<'a>, crate::engine::StatChangeRequest) -> bool;
/// Showdown `onSourceTryHeal`: a heal about to be taken off the holder (a drain, Leech Seed,
/// Strength Sap); `true` when the handler took it over (the heal is 0).
pub type SourceTryHealHandler =
    for<'a> fn(&mut BattleContext<'a>, crate::engine::DrainedHeal) -> bool;
/// Showdown `onAnyFaint`: (holder, who fainted and how) — asked of the active Pokémon as
/// `faintMessages` reaches a faint.
pub type AnyFaintHandler = for<'a> fn(&mut BattleContext<'a>, PokemonId, crate::state::FaintCause);
/// Showdown `onAllyFaint`: (holder, who fainted and how) — asked of the fainted Pokémon's allies
/// on the field in the same `Faint` event.
pub type AllyFaintHandler = for<'a> fn(&mut BattleContext<'a>, PokemonId, crate::state::FaintCause);
/// Showdown `onSourceAfterFaint`: (holder, the last faint of the batch, which the holder caused,
/// how many fainted in it — `length`).
pub type SourceAfterFaintHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, crate::state::FaintCause, i8);
/// Showdown `onSourceTryPrimaryHit`: the holder's move is about to strike `target`
/// (Gulp Missile's Surf; DESIGN.md §43.4).
pub type SourceTryPrimaryHitHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, PokemonId, &crate::dex::ResolvedMove);

/// Showdown `onModifyAtk` / `onModifySpA`: the 4096-based `chainModify` the handler adds to
/// the attacking stat's chain, if any.
pub type ModifyOffenseHandler = for<'a> fn(&mut OffensiveStatContext<'a>) -> Option<u32>;

/// When an ability's boost applies to the move being used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoostWhen {
    /// The move has this flag (Iron Fist: punch).
    Flag(MoveFlag),
    /// The move has recoil or crash damage (Reckless: `move.recoil || move.hasCrashDamage`).
    Recoil,
    /// The move has this type (Steelworker: Steel).
    MoveType(crate::dex::PokemonType),
    /// The move has this type and the holder is at a third of its max HP or below (Blaze).
    MoveTypeInAPinch(crate::dex::PokemonType),
    /// The holder has a major status (Guts).
    HolderStatused,
    /// The holder is at half its max HP or below (Defeatist).
    HolderAtHalfHp,
    /// A physical move by a poisoned holder (Toxic Boost).
    PoisonedAndPhysical,
    /// A special move by a burned holder (Flare Boost).
    BurnedAndSpecial,
    /// A move of one of these types in this weather (Sand Force).
    WeatherAndTypes(
        crate::state::WeatherKind,
        &'static [crate::dex::PokemonType],
    ),
    /// Every other active Pokémon has moved this turn (Analytic: `queue.willMove` over
    /// `getAllActive`; one sent back to the bench before its move — Eject Button — no longer
    /// counts).
    MovesLast,
    /// A move of one of these types (Thick Fat: Fire, Ice).
    MoveTypes(&'static [crate::dex::PokemonType]),
    /// The Pokémon on the other end of the move has not been on the field through a turn
    /// start (Stakeout: `!defender.activeTurns`).
    OpponentEnteredThisTurn,
    /// In this weather (Orichalcum Pulse: sun).
    Weather(crate::state::WeatherKind),
    /// Plus / Minus: an active Pokémon with `other` (or, from Gen 5, with either) —
    /// `DamageRules::plus_minus` says whose.
    PlusMinus { other: AbilityId },
    /// The move lost its secondaries to Sheer Force (`MoveFlags::sheer_force`), or is one Sheer
    /// Force boosts as it is (`MoveFlags::sheer_force_boost`).
    SheerForce,
    /// A physical move while Slow Start's countdown runs. (The Gen 7 mod reads the category the
    /// move's data gives, adding `onModifySpA` for a physical-in-data move made special — a
    /// Z-move; every move the engine has keeps its data category there.)
    SlowStart,
    /// The holder and the Pokémon on the other end of the move both have a gender (Rivalry).
    BothGendered,
    /// Always (Fur Coat).
    Always,
    /// In this terrain (Hadron Engine: Electric, Grass Pelt: Grassy; Showdown `isTerrain`).
    Terrain(crate::state::TerrainKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveFlag {
    Punch,
    Bite,
    Pulse,
    Slicing,
    Contact,
    Sound,
}

/// An ability's `onModifySpe` as data: `modifier` (4096-based, in the Speed chain) while the
/// field is in `when`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeedBoost {
    pub when: SpeedWhen,
    pub modifier: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedWhen {
    /// Surge Surfer: Electric Terrain (`isTerrain`).
    Terrain(crate::state::TerrainKind),
    /// Slow Start: its countdown is running (`AbilityState::slow_start`).
    SlowStart,
    /// Chlorophyll, Swift Swim, Sand Rush, Slush Rush: this weather as the holder sees it
    /// (`engine::weather_for`; Utility Umbrella only hides sun and rain).
    Weather(crate::state::WeatherKind),
    /// Quick Feet: the holder has a status (`pokemon.status`) — whose paralysis then leaves its
    /// Speed alone (`engine::stats::effective_speed`).
    Status,
}

impl SpeedBoost {
    /// The modifier for `holder` when it applies.
    pub fn modifier_for(
        &self,
        state: &crate::state::BattleState,
        rules: &crate::rules::ResolvedRules,
        holder: PokemonId,
    ) -> Option<u32> {
        let applies = match self.when {
            SpeedWhen::Terrain(terrain) => state.field.terrain.map(|t| t.kind) == Some(terrain),
            SpeedWhen::SlowStart => slow_start_running(state, holder),
            SpeedWhen::Weather(weather) => {
                crate::engine::weather_for(state, rules, holder) == Some(weather)
            }
            SpeedWhen::Status => state.pokemon(holder).is_some_and(|p| p.status.is_some()),
        };
        applies.then_some(self.modifier)
    }
}

/// An ability's `onWeather` (run for each active Pokémon in the weather's upkeep) as data: in
/// `weather` as the holder sees it (`engine::weather_for`: Dry Skin, Rain Dish and Solar Power
/// check `target.effectiveWeather()`, which Utility Umbrella changes for sun and rain only; Ice
/// Body and Gen 4 Hydration the weather's id, the same for them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeatherReaction {
    pub weather: crate::state::WeatherKind,
    pub effect: WeatherEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherEffect {
    /// Max HP divided by this, healed.
    Heal(u16),
    /// Max HP divided by this, lost (a non-move damage: Magic Guard stops it).
    Damage(u16),
    /// Its status cured (Hydration, `-activate`).
    CureStatus,
}

/// An ability's `onModifyDamage` as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageBoost {
    pub when: DamageBoostWhen,
    /// 4096-based.
    pub modifier: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageBoostWhen {
    /// A critical hit (Sniper).
    Critical,
    /// A not very effective hit (Tinted Lens: `typeMod < 0`).
    NotVeryEffective,
    /// A super-effective hit (Neuroforce: `typeMod > 0`).
    SuperEffective,
}

/// An accuracy modifier in the form each generation's accuracy check uses
/// (`HitRules::accuracy_order`, DESIGN.md §15.7): Gen 5+ Showdown's 4096-based `chainModify`;
/// Gen 3-4 the cartridge's `hit rate × percent / 100`, floored, at the modifier's place in the
/// cartridge's fixed order (pokeemerald `Cmd_accuracycheck`, pokeplatinum
/// `BattleControllerPlayer_CheckMoveHitAccuracy`); Gen 2 what it takes off the accuracy byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccuracyModifier {
    pub chain: u32,
    pub percent: AccuracyPercent,
    pub step: AccuracyStep,
    /// Gen 2 (the gen2 mod's Bright Powder: `accuracy - 20`, not below 0).
    pub byte_penalty: u32,
}

impl AccuracyModifier {
    /// A modifier with no Gen 2 form (every ability's: Gen 2 has none).
    pub const fn new(chain: u32, percent: u32, step: AccuracyStep) -> Self {
        Self {
            chain,
            percent: AccuracyPercent::Fixed(percent),
            step,
            byte_penalty: 0,
        }
    }

    /// The Gen 3-4 percentage.
    pub fn percent(self, rules: &crate::rules::ResolvedRules) -> u32 {
        match self.percent {
            AccuracyPercent::Fixed(percent) => percent,
            AccuracyPercent::LaxIncense => rules.hits.lax_incense_percent,
        }
    }
}

/// A modifier's Gen 3-4 percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccuracyPercent {
    Fixed(u32),
    /// Lax Incense's hold-effect parameter differs by game (Emerald 5, Platinum 10):
    /// `HitRules::lax_incense_percent`.
    LaxIncense,
}

/// Where the Gen 3-4 cartridge applies a modifier, first to last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AccuracyStep {
    CompoundEyes,
    /// Sand Veil, Snow Cloak.
    WeatherEvasion,
    Hustle,
    TangledFeet,
    /// Bright Powder, Lax Incense.
    TargetItem,
    /// Wide Lens, Zoom Lens.
    UserItem,
}

/// When an accuracy modifier applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccuracyWhen {
    Always,
    /// A physical move (Hustle) — in Gen 3 one of a physical type
    /// (`HitRules::physical_accuracy_by_type`).
    Physical,
    /// In this weather (Sand Veil, Snow Cloak).
    Weather(crate::state::WeatherKind),
    /// The holder is confused (Tangled Feet).
    HolderConfused,
    /// The holder's opponent has already moved this turn (Zoom Lens: Showdown
    /// `!queue.willMove(target)`, the cartridge's `Battler_MovedThisTurn`).
    OpponentMoved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccuracyBoost {
    pub when: AccuracyWhen,
    pub modifier: AccuracyModifier,
}

impl AccuracyBoost {
    /// The modifier for `move_def`, the holder being `holder` and the other side of the check
    /// `opponent`, when it applies.
    pub fn modifier_for(
        &self,
        state: &crate::state::BattleState,
        rules: &crate::rules::ResolvedRules,
        holder: PokemonId,
        opponent: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) -> Option<AccuracyModifier> {
        let applies = match self.when {
            AccuracyWhen::Always => true,
            AccuracyWhen::Physical if rules.hits.physical_accuracy_by_type => {
                move_def.move_type.physical_before_split()
            }
            AccuracyWhen::Physical => move_def.category == crate::dex::MoveCategory::Physical,
            AccuracyWhen::Weather(weather) => {
                crate::engine::field_weather(state, rules) == Some(weather)
            }
            AccuracyWhen::HolderConfused => state.pokemon(holder).is_some_and(|p| {
                p.volatiles
                    .contains(crate::state::VolatileConditionKind::Confusion)
            }),
            AccuracyWhen::OpponentMoved => !state.will_move(opponent),
        };
        applies.then_some(self.modifier)
    }
}

/// An ability's `onSourceModifyDamage` as data: the damage its holder takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageTaken {
    /// ×0.75 on a super-effective hit (Filter, Solid Rock, Prism Armor: `typeMod > 0`).
    SuperEffective,
    /// ×0.5 at full HP (Multiscale, Shadow Shield).
    FullHp,
    /// ×0.5 from special moves (Ice Scales).
    Special,
    /// ×0.5 from sound moves (Punk Rock).
    Sound,
    /// Fluffy: ×2 from Fire moves, ×0.5 from contact moves (both: ×1).
    Fluffy,
}

impl DamageTaken {
    /// The 4096-based modifier on a hit by `move_def`, if any.
    pub fn modifier(
        self,
        holder: &crate::state::PokemonState,
        move_def: &crate::dex::ResolvedMove,
        type_effectiveness: crate::rules::Fraction,
    ) -> Option<u32> {
        match self {
            Self::SuperEffective => {
                (type_effectiveness.numerator > type_effectiveness.denominator).then_some(3072)
            }
            Self::FullHp => (holder.hp >= holder.max_hp).then_some(2048),
            Self::Special => {
                (move_def.category == crate::dex::MoveCategory::Special).then_some(2048)
            }
            Self::Sound => move_def.flags.sound.then_some(2048),
            Self::Fluffy => {
                let fire = move_def.move_type == crate::dex::PokemonType::Fire;
                match (fire, move_def.flags.contact) {
                    (true, true) | (false, false) => None,
                    (true, false) => Some(8192),
                    (false, true) => Some(2048),
                }
            }
        }
    }
}

/// The multiplier of an ability's boost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoostModifier {
    /// 4096-based.
    Fixed(u32),
    /// Transistor's, by generation (`DamageRules::transistor_boost`).
    Transistor,
    /// Supreme Overlord's, by the Pokémon fallen when the holder came in
    /// (`AbilityState::fallen`, `FALLEN_ALLIES_BOOST`); none fallen, no boost.
    FallenAllies,
    /// Rivalry: ×1.25 against the same gender, ×0.75 against the other.
    Rivalry,
}

/// Showdown `onModifyWeight`: the weight times `factor`, truncated, in priority order (an
/// ability before an item on a tie).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightModifier {
    pub priority: i16,
    pub factor: crate::rules::Fraction,
}

/// Showdown `onModifyMove` editing the holder's move as it starts, as data (applied by
/// `engine::modified_move`, which the replay shares).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MoveEdit {
    /// `move.ignoreEvasion` (Keen Eye from Gen 6, Illuminate and Mind's Eye in Gen 9).
    pub ignore_evasion: bool,
    /// `move.ignoreImmunity` for a Normal or Fighting move (Scrappy, Mind's Eye): Ghost types
    /// take it.
    pub normal_fighting_ignore_immunity: bool,
    /// The move loses `contact` (Long Reach).
    pub no_contact: bool,
    /// `move.tracksTarget` (Propeller Tail, Stalwart): no redirection.
    pub tracks_target: bool,
    /// A contact move loses `protect` (Unseen Fist).
    pub contact_ignores_protect: bool,
    /// A multi-hit range strikes its maximum, and no hit after the first checks accuracy again
    /// (Skill Link).
    pub max_hits: bool,
    /// A move with secondaries loses them and its own `self` (`MoveFlags::sheer_force`; Sheer
    /// Force). `selfBoost` stays.
    pub sheer_force: bool,
    /// A damaging move without a flinch secondary gains one with this percent chance (Stench,
    /// `onModifyMovePriority` -1: after the other edits, appended to the secondaries).
    pub flinch_chance: Option<u8>,
    /// `move.ignoreAbility` (Mold Breaker, Teravolt, Turboblaze): while the move is used the
    /// other Pokémon's breakable abilities do nothing (`engine::suppressing_ability`).
    pub ignores_abilities: bool,
    /// `move.ignoreAbility` for a status move only (Mycelium Might).
    pub status_ignores_abilities: bool,
}

/// Which attacking stat an `offense_boost` modifies: Showdown runs `ModifyAtk` for a physical
/// move and `ModifySpA` for a special one, and an ability declares one handler or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffenseStats {
    /// `onModifyAtk` only (Guts).
    Attack,
    /// `onModifySpA` only (Plus, Minus).
    SpecialAttack,
    /// `onModifyAtk` and `onModifySpA` (Steelworker, Defeatist, Gen 5+ Blaze).
    Both,
}

/// Showdown `onModifyAtk` / `onModifySpA` as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffenseBoost {
    pub stats: OffenseStats,
    pub boost: AbilityBoost,
}

impl OffenseBoost {
    /// The boost's modifier for `move_def` used by `holder` against `opponent`, when its stat
    /// is the one the move attacks with and the boost applies.
    pub fn modifier_for(
        &self,
        state: &crate::state::BattleState,
        rules: &crate::rules::ResolvedRules,
        holder: PokemonId,
        opponent: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) -> Option<u32> {
        let covered = match self.stats {
            OffenseStats::Attack => move_def.category == crate::dex::MoveCategory::Physical,
            OffenseStats::SpecialAttack => move_def.category == crate::dex::MoveCategory::Special,
            OffenseStats::Both => true,
        };
        covered
            .then(|| {
                self.boost
                    .modifier_for(state, rules, holder, opponent, move_def)
            })
            .flatten()
    }

    /// The handler's priority: the boost's own, or the generation's for Plus / Minus.
    pub fn priority(&self, rules: &crate::rules::ResolvedRules) -> i16 {
        match self.boost.when {
            BoostWhen::PlusMinus { .. } => rules.damage.plus_minus_priority,
            _ => self.boost.priority,
        }
    }
}

/// An ability's boost as data: `onBasePower` (a BasePower chain participant) or `onModifyAtk`
/// with `onModifySpA` (the attacking stat's chain), at Showdown's priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilityBoost {
    pub when: BoostWhen,
    pub modifier: BoostModifier,
    pub priority: i16,
}

impl AbilityBoost {
    /// The boost's 4096-based modifier for `move_def` when it applies; `holder` has the
    /// ability, `opponent` is the Pokémon on the other end of the move (the target for the
    /// user's boosts, the user for the target's).
    pub fn modifier_for(
        &self,
        state: &crate::state::BattleState,
        rules: &crate::rules::ResolvedRules,
        holder: PokemonId,
        opponent: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) -> Option<u32> {
        let flags = move_def.flags;
        let applies = match self.when {
            BoostWhen::Flag(MoveFlag::Punch) => flags.punch,
            BoostWhen::Flag(MoveFlag::Bite) => flags.bite,
            BoostWhen::Flag(MoveFlag::Pulse) => flags.pulse,
            BoostWhen::Flag(MoveFlag::Slicing) => flags.slicing,
            BoostWhen::Flag(MoveFlag::Contact) => flags.contact,
            BoostWhen::Flag(MoveFlag::Sound) => flags.sound,
            BoostWhen::Recoil => {
                matches!(move_def.recoil, Some(crate::dex::Recoil::Dealt(_)))
                    || move_def.behavior.crash_damage
            }
            BoostWhen::MoveType(move_type) => move_def.move_type == move_type,
            BoostWhen::MoveTypeInAPinch(move_type) => {
                move_def.move_type == move_type
                    && state
                        .pokemon(holder)
                        .is_some_and(|p| u32::from(p.hp) * 3 <= u32::from(p.max_hp))
            }
            BoostWhen::HolderStatused => state.pokemon(holder).is_some_and(|p| p.status.is_some()),
            BoostWhen::HolderAtHalfHp => state
                .pokemon(holder)
                .is_some_and(|p| u32::from(p.hp) * 2 <= u32::from(p.max_hp)),
            BoostWhen::PoisonedAndPhysical => {
                move_def.category == crate::dex::MoveCategory::Physical
                    && state.pokemon(holder).is_some_and(|p| {
                        p.has_status(NonVolatileStatus::Poison)
                            || p.has_status(NonVolatileStatus::Toxic)
                    })
            }
            BoostWhen::BurnedAndSpecial => {
                move_def.category == crate::dex::MoveCategory::Special
                    && state
                        .pokemon(holder)
                        .is_some_and(|p| p.has_status(NonVolatileStatus::Burn))
            }
            BoostWhen::WeatherAndTypes(weather, types) => {
                crate::engine::field_weather(state, rules) == Some(weather)
                    && types.contains(&move_def.move_type)
            }
            BoostWhen::MoveTypes(types) => types.contains(&move_def.move_type),
            BoostWhen::OpponentEnteredThisTurn => {
                state.pokemon(opponent).is_some_and(|p| p.active_turns == 0)
            }
            BoostWhen::Weather(weather) => {
                crate::engine::weather_for(state, rules, holder) == Some(weather)
            }
            BoostWhen::PlusMinus { other } => {
                use crate::rules::PlusMinusPartners;
                let rule = rules.damage.plus_minus;
                let side = state.pokemon(holder).map(|p| p.side);
                state
                    .sides
                    .iter()
                    .flat_map(|s| s.active.iter().flatten().copied())
                    .filter(|id| *id != holder)
                    // Gen 3 (`getAllActive`: `!active.fainted`) still counts a Pokémon at 0 HP
                    // whose faint is not processed yet — the first target a spread move knocked
                    // out, at the next target's hit (pokeemerald's `ABILITY_ON_FIELD2` reads no
                    // HP at all); Gen 4+ `allies()` goes by HP (R-M8-107).
                    .filter(|id| {
                        state.pokemon(*id).is_some_and(|p| !p.is_fainted())
                            || (rule == PlusMinusPartners::AnyActiveOpposite
                                && state.faint_queue.contains(id)
                                && !state.announced_faints.contains(id))
                    })
                    .filter_map(|id| state.pokemon(id))
                    .filter(|p| {
                        rule == PlusMinusPartners::AnyActiveOpposite || Some(p.side) == side
                    })
                    .any(|p| {
                        let partner = crate::engine::ability_active(state, rules, p);
                        partner == Some(other)
                            || (rule == PlusMinusPartners::AlliesEither
                                && matches!(partner, Some(AbilityId::PLUS | AbilityId::MINUS)))
                    })
            }
            BoostWhen::SheerForce => move_def.flags.sheer_force || move_def.flags.sheer_force_boost,
            BoostWhen::SlowStart => {
                let category = if rules.damage.slow_start_reads_data_category {
                    move_def.data_category
                } else {
                    move_def.category
                };
                category == crate::dex::MoveCategory::Physical && slow_start_running(state, holder)
            }
            BoostWhen::BothGendered => state
                .pokemon(holder)
                .zip(state.pokemon(opponent))
                .is_some_and(|(holder, opponent)| holder.gender.both_gendered(opponent.gender)),
            BoostWhen::Always => true,
            BoostWhen::Terrain(terrain) => state.field.terrain.map(|t| t.kind) == Some(terrain),
            BoostWhen::MovesLast => state
                .sides
                .iter()
                .flat_map(|side| side.active.iter().flatten().copied())
                .filter(|id| *id != holder && state.pokemon(*id).is_some_and(|p| !p.is_fainted()))
                .all(|id| !state.will_move(id)),
        };
        if !applies {
            return None;
        }
        match self.modifier {
            BoostModifier::Fixed(modifier) => Some(modifier),
            BoostModifier::Transistor => Some(rules.damage.transistor_boost),
            BoostModifier::FallenAllies => state
                .pokemon(holder)
                .map(|p| usize::from(p.ability_state.fallen))
                .filter(|fallen| *fallen > 0)
                .map(|fallen| FALLEN_ALLIES_BOOST[fallen]),
            BoostModifier::Rivalry => {
                let same = state.pokemon(holder).map(|p| p.gender)
                    == state.pokemon(opponent).map(|p| p.gender);
                Some(if same { 5120 } else { 3072 })
            }
        }
    }
}
pub use crate::effects::moves::BasePowerChainHandler;

/// Dark Aura / Fairy Aura's BasePower modifier ([5448, 4096]).
pub const AURA_BOOST: u32 = 5448;
/// The auras' modifier while an Aura Break is active ([3072, 4096]).
pub const AURA_BROKEN: u32 = 3072;
/// The Ruin abilities' stat modifier (`chainModify(0.75)`).
pub const RUIN: u32 = 3072;
/// Whether `holder`'s Slow Start is still counting down.
fn slow_start_running(state: &crate::state::BattleState, holder: PokemonId) -> bool {
    state
        .pokemon(holder)
        .is_some_and(|p| p.ability_state.slow_start > 0)
}

/// Supreme Overlord's BasePower modifier by the Pokémon fallen (at most 5).
pub const FALLEN_ALLIES_BOOST: [u32; 6] = [4096, 4506, 4915, 5325, 5734, 6144];
/// The place of an `onDamagingHit` without `onDamagingHitOrder` (Gooey, Cotton Down): after
/// every ordered reaction (Showdown sorts a missing order last).
pub(crate) const UNORDERED_REACTION: i16 = i16::MAX;

/// A random reaction to a hit that reached the holder's HP, after the ordered ones (Showdown
/// `onDamagingHit` without an order). Data: the hit sequence rolls it and the replay reads it
/// from the table (`engine::effect_spore_roll`, `engine::cursed_body_rolls`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitReaction {
    /// Effect Spore: a contact hit may put the attacker to sleep, paralyse or poison it
    /// (`HitRules::effect_spore`).
    EffectSpore,
    /// Cursed Body: 3 in 10 to Disable the move that hit, unless the attacker already has a
    /// Disable or the move is Struggle.
    CursedBody,
    /// Cute Charm: a contact hit may infatuate the attacker (`StatusLifecycleRules::
    /// contact_status_chance`, as Static; Gen 3-4 a hit that dealt damage) — rolled before the
    /// infatuation checks the two genders.
    CuteCharm,
}

/// Showdown `onSourceDamagingHit` as data (Poison Touch, Toxic Chain): a hit the holder deals
/// has a 3 in 10 chance to inflict `status` on its target — never on one whose Shield Dust or
/// Covert Cloak keeps secondaries off (`engine::shields_secondaries`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DealtHitStatus {
    pub status: NonVolatileStatus,
    /// Poison Touch needs a contact move; Toxic Chain does not.
    pub contact_only: bool,
}

/// Forecast's formes (DESIGN.md §43.2): the holding species' forme for each weather, its own
/// otherwise (Castform-Sunny / -Rainy / -Snowy, whose numbers are the base forme's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forecast {
    pub holder: crate::ids::SpeciesId,
    pub sun: crate::ids::SpeciesId,
    pub rain: crate::ids::SpeciesId,
    pub snow: crate::ids::SpeciesId,
}

#[derive(Clone, Copy, Default)]
pub struct AbilityBehavior {
    /// The holder entered the field (Intimidate, the weather setters).
    /// Showdown: `onStart`.
    pub switch_in: Option<SwitchHandler>,
    /// Showdown `onSwitchInPriority`: where `switch_in` sorts among the switch-in handlers from
    /// Gen 5 (`SwitchInOrder::ByPriority`; Unnerve, As One: 1; Costar, Hospitality,
    /// Protosynthesis, Quark Drive: -2). Data with the handler, not a callback of its own.
    pub switch_in_priority: i8,
    /// Showdown `onSwitchIn` alone (Imposter): run as the holder enters the field, never as a
    /// new or restored ability starts on it.
    pub entry: Option<SwitchHandler>,
    /// Illusion: `onBeforeSwitchIn` picks the disguise (`BattleContext::take_disguise`); its
    /// `onEnd` lifts it (`BattleContext::end_illusion`), `onFaint` drops it silently.
    pub illusion: bool,
    pub try_hit: Option<TryHitHandler>,
    /// Magic Bounce (`onTryHit`, priority 1): a reflectable move aimed at the holder is used
    /// again against its user (DESIGN.md §42.8). Read by `MovePipeline::reflect_move`.
    pub magic_bounce: bool,
    /// Dancer (Showdown's `runMove`, no callback of its own): a dance another Pokémon used is
    /// danced along by the holder, slowest first (DESIGN.md §42.9).
    pub dancer: bool,
    pub try_set_status: Option<TrySetStatusHandler>,
    pub after_set_status: Option<AfterSetStatusHandler>,
    pub modify_offense: Option<OrderedHandler<ModifyOffenseHandler>>,
    /// `modify_offense` rounds the stat itself (Showdown `return this.modify(atk, 1.5)`: Hustle)
    /// rather than `chainModify`: from Gen 5 the other participants' chain applies to what it
    /// leaves, not to the stat with it in (R-M10-65). Gen 3-4's formula takes every modifier
    /// in turn anyway.
    pub offense_on_stat: bool,
    /// Showdown `onBasePower` as data (Iron Fist, Tough Claws; Gen 3-4 Blaze).
    pub base_power_boost: Option<AbilityBoost>,
    /// Showdown `onModifyAtk` / `onModifySpA` as data (Guts; Steelworker; Gen 5+ Blaze).
    pub offense_boost: Option<OffenseBoost>,
    /// Guts: a burned holder's physical damage is not halved (Showdown's `!hasAbility('guts')`
    /// in the damage formula: no callback).
    pub ignores_burn_drop: bool,
    /// Showdown `onImmunity` for a weather: no residual damage from it (Sand Force: sandstorm).
    pub weather_immunity: Option<crate::state::WeatherKind>,
    /// Showdown `onModifyDamage` as data: a ModifyDamage chain participant (Sniper, Tinted Lens).
    pub damage_boost: Option<DamageBoost>,
    /// Showdown `onModifySTAB`: the holder's STAB (Adaptability: ×2).
    pub stab_modifier: Option<crate::rules::Fraction>,
    /// Showdown `onSourceModifyDamage`: the damage the holder takes (Filter, Multiscale …).
    pub damage_taken: Option<DamageTaken>,
    /// Showdown `onSourceModifyAtk` + `onSourceModifySpA`: the attacking stat of a move aimed at
    /// the holder (Thick Fat, Heatproof: ×0.5).
    pub foe_offense: Option<AbilityBoost>,
    /// Showdown `onSourceBasePower`: the power of a move aimed at the holder (Gen 3-4 Thick Fat).
    pub foe_base_power: Option<AbilityBoost>,
    /// Showdown `onModifyDef`: the holder's Defense (Marvel Scale, Fur Coat).
    pub defense_boost: Option<AbilityBoost>,
    /// Showdown `onDamage` halving the residual damage of this status (Heatproof: burn).
    pub halves_status_damage: Option<NonVolatileStatus>,
    /// Showdown `onSourceModifyAccuracy`: the accuracy of the holder's moves (Compound Eyes,
    /// Hustle).
    pub accuracy_boost: Option<AccuracyBoost>,
    /// Showdown `onModifyAccuracy`: the accuracy of moves aimed at the holder (Sand Veil, Snow
    /// Cloak, Tangled Feet).
    pub evasion_boost: Option<AccuracyBoost>,
    /// Showdown `onAnyModifyAccuracy`: the accuracy of the holder's side's moves, 4096-based
    /// (Victory Star, Gen 5+).
    pub ally_accuracy_boost: Option<u32>,
    /// Wonder Skin (`onModifyAccuracy`, priority 10): a status move aimed at the holder has
    /// this accuracy, before any other modifier.
    pub status_accuracy: Option<u8>,
    /// No Guard (`onAnyInvulnerability`, `onAnyAccuracy`): every move the holder uses or is
    /// aimed at hits, a semi-invulnerable target included.
    pub no_guard: bool,
    /// Showdown `onBasePower` on the user's ability (Technician, priority 30).
    pub base_power: Option<OrderedHandler<BasePowerChainHandler>>,
    /// Showdown `onAllyBasePower` (Power Spot, Battery, priority 22): a participant in a
    /// partner's BasePower chain, never the holder's own.
    pub ally_base_power: Option<OrderedHandler<BasePowerChainHandler>>,
    /// Showdown `onAllyBasePower` as data, for the holder's own moves and its partners' (Steely
    /// Spirit: Steel ×1.5, priority 22).
    pub team_base_power: Option<AbilityBoost>,
    /// Showdown `onAnyBasePower` (Dark Aura, Fairy Aura, priority 20): a damaging move of this
    /// type, whoever uses it, gets `AURA_BOOST` — `AURA_BROKEN` while an Aura Break is active —
    /// once, however many holders are active. Its `onStart` only announces it.
    pub aura: Option<crate::dex::PokemonType>,
    /// Aura Break (`onAnyTryPrimaryHit`): the auras weaken moves instead. Its `onStart` only
    /// announces it.
    pub aura_break: bool,
    /// Showdown `onAnyModifyAtk` / `onAnyModifyDef` / `onAnyModifySpA` / `onAnyModifySpD` (the
    /// Ruin abilities, priority 0): that stat of every Pokémon without this ability is `RUIN` in
    /// a damage calculation, once however many holders are active. Its `onStart` only announces
    /// it.
    pub ruin: Option<crate::state::Stat>,
    /// Showdown `onAnyModifyDamage` for hits on the holder's partners (Friend Guard 3072 =
    /// ×0.75), 4096-based, chained into ModifyDamage.
    pub ally_damage_modifier: Option<u32>,
    /// Reaction to a hit that reached the holder's HP; the priority is Showdown's
    /// `onDamagingHitOrder` (abilities 1, items 2; none: `UNORDERED_REACTION`, last).
    /// Showdown: `onDamagingHit`.
    pub damaging_hit_reaction: Option<OrderedHandler<DamagingHitReactionHandler>>,
    /// The Resolve stage of a DamageRequest; the priority is the execution order, lowest
    /// first: Magic Guard 0, Sturdy 30, Focus Sash 40 (Showdown's `onDamagePriority` negated).
    /// Showdown: `onDamage`.
    pub damage_resolution: Option<OrderedHandler<DamageResolutionHandler>>,
    /// ChangeBoost stage of a stat change (Contrary, Simple).
    pub modify_stat_change: Option<ModifyStatChangeHandler>,
    /// Showdown `onModifyBoost` (Gen 4 Simple): the holder's stages count double wherever they
    /// are read (`engine::stats::read_stage`, the accuracy check), not as they change.
    pub doubles_read_stages: bool,
    /// TryBoost stage of a stat change (Clear Body).
    pub try_stat_change: Option<TryStatChangeHandler>,
    /// Showdown `onModifyMove` as data (Keen Eye, Scrappy, Long Reach, Skill Link …).
    pub move_edit: Option<MoveEdit>,
    /// Parental Bond (`onPrepareHit`): the holder's move strikes twice (`engine::prepared_move`).
    /// Its `onSourceModifySecondaries` only concerns Secret Power.
    pub parental_bond: bool,
    /// Gorilla Tactics (`onModifyMove`, `onDisableMove`, `onBeforeMove`): the first move the
    /// holder uses after coming in, Struggle aside, locks it in (`AbilityState::choice_lock`,
    /// which `onStart` / `onEnd` clear: coming in starts it over); the request offers no other
    /// and BeforeMove refuses one (`-fail`, no PP).
    pub choice_lock: bool,
    /// Gorilla Tactics' `onModifyAtk`: `if (pokemon.volatiles['dynamax']) return;` — its Attack
    /// boost does not apply while the holder is Dynamaxed (DESIGN.md §44.5).
    pub boost_suspended_by_dynamax: bool,
    /// Showdown `onModifyWeight` (Heavy Metal ×2 at priority 1, Light Metal ½ truncated).
    pub weight_modifier: Option<WeightModifier>,
    /// Rock Head (`onDamage` returning null for `recoil`): a move's recoil does not reach the
    /// holder; Struggle's, an HP cost, still does.
    pub blocks_recoil: bool,
    /// Unaware (`onAnyModifyBoost`): the holder's moves read the target's Defense, Special
    /// Defense and evasion at stage 0; a move aimed at the holder reads its user's Attack,
    /// Defense, Special Attack and accuracy at stage 0 (`engine::unaware_ignores`).
    pub unaware: bool,
    /// Mirror Armor (its `onTryBoost`): a drop another Pokémon causes is refused and sent back
    /// to that Pokémon, when it still stands on the field — never a drop Mirror Armor itself
    /// sent (`effect.name === 'Mirror Armor'`).
    pub reflects_stat_drops: bool,
    /// Showdown `onTryAddVolatile` returning `null` for these volatiles: they are never added
    /// to the holder (Inner Focus: flinch). The move that tried still rolled its chance.
    pub blocks_volatiles: &'static [crate::state::VolatileConditionKind],
    /// Showdown `onFlinch` (Steadfast): a flinch that stops the holder's move raises this stat
    /// by one stage.
    pub flinch_boost: Option<crate::state::Stat>,
    /// Showdown `onModifySecondaries` (Shield Dust): a move hitting the holder loses its
    /// target-directed secondaries, which then roll nothing; user-directed ones stay.
    pub shields_secondaries: bool,
    /// Effect Spore, Cursed Body (`HitReaction`).
    /// Showdown: `onDamagingHit`.
    pub hit_reaction: Option<HitReaction>,
    /// Poison Touch, Toxic Chain (`DealtHitStatus`).
    /// Showdown: `onSourceDamagingHit`.
    pub dealt_hit_status: Option<DealtHitStatus>,
    /// Showdown `onAnyRedirectTarget` (Gen 3 Lightning Rod: `onFoeRedirectTarget`): single-target
    /// moves of this type are pulled onto the holder in doubles (Lightning Rod: Electric, Storm
    /// Drain: Water).
    pub redirect_type: Option<crate::dex::PokemonType>,
    /// Runs after this ability's `try_hit` refused a move: Lightning Rod / Storm Drain raise
    /// Special Attack, Water Absorb / Volt Absorb restore HP.
    /// Showdown: part of `onTryHit`.
    pub absorb_reaction: Option<AbsorbReactionHandler>,
    /// Static / Flame Body / Poison Point: a contact hit that reached the holder's HP may
    /// status the attacker (`StatusLifecycleRules::contact_status_chance`, a `ContactReaction`
    /// decision). Data, not a function: the hit sequence rolls and applies it, and the replay
    /// can tell from the table that a decision is due.
    pub contact_status: Option<NonVolatileStatus>,
    /// Shed Skin, Speed Boost: take part in the residual schedule.
    /// Showdown: `onResidual`.
    pub residual: Option<ResidualParticipant>,
    /// The holder is about to leave by a switch, not a faint (Natural Cure).
    /// Showdown: `onSwitchOut`.
    pub switch_out: Option<SwitchHandler>,
    /// Showdown `onDragOut` returning null (Suction Cups): Roar / Whirlwind / Dragon Tail / Red
    /// Card cannot force the holder out.
    pub blocks_drag_out: bool,
    /// Serene Grace (Showdown `onModifyMove`): every secondary chance is multiplied.
    pub secondary_chance_multiplier: Option<u16>,
    /// Infiltrator (Showdown `onModifyMove`: `move.infiltrates`): the holder's moves go through
    /// screens, a foe's Mist and Safeguard, and from Gen 6 a Substitute
    /// (`engine::infiltrates`).
    pub infiltrates: bool,
    /// Quark Drive (`onStart`, `onTerrainChange`, `onEnd`): the `quarkdrive` volatile while
    /// Electric Terrain lasts (`BattleContext::quark_drive`).
    pub quark_drive: bool,
    /// Showdown `onModifySpe` as data (Surge Surfer: ×2 in Electric Terrain; Chlorophyll …).
    pub speed_boost: Option<SpeedBoost>,
    /// Air Lock, Cloud Nine (`suppressWeather`; `onSwitchIn`, `onStart`, `onEnd` tell the
    /// `WeatherChange` listeners): the weather is suppressed while the holder is active
    /// (`engine::weather`).
    pub suppresses_weather: bool,
    /// Showdown `onWeather` as data (Dry Skin, Rain Dish, Ice Body, Solar Power, Gen 4
    /// Hydration), in the weather's upkeep, before its chip.
    pub weather_reactions: &'static [WeatherReaction],
    /// Protosynthesis (`onStart`, `onWeatherChange`, `onEnd`): the `protosynthesis` volatile
    /// while the sun is up (`BattleContext::protosynthesis`).
    pub protosynthesis: bool,
    /// Damp (`onAnyTryMove`, `onAnyDamage`): while the holder is active, no Pokémon's
    /// Explosion, Self-Destruct, Misty Explosion or Mind Blown goes ahead
    /// (`MoveBehavior::stopped_by_damp`), and Aftermath deals nothing.
    pub damp: bool,
    /// Overcoat (Showdown `onImmunity` sandstorm / hail / powder + `onTryHit` for powder
    /// moves): no weather chip, and powder moves are refused from Gen 6.
    pub weather_and_powder_immune: bool,
    /// Showdown `onModifyCritRatio` (Super Luck +1).
    pub crit_ratio_boost: u8,
    /// Showdown `onCriticalHit: false` (Battle Armor, Shell Armor): the roll still happens,
    /// the hit is never critical.
    pub blocks_critical_hit: bool,
    /// Sturdy's `onTryHit`: one-hit KO moves do not affect the holder (`-immune`).
    pub ohko_immune: bool,
    /// Showdown `onFractionalPriority` (Stall, Quick Draw).
    pub fractional_priority: Option<crate::effects::FractionalPriority>,
    /// Showdown `onModifyPriority` (Prankster, Gale Wings, Triage).
    pub modify_priority: Option<crate::effects::PriorityBoost>,
    /// Showdown `onModifyType` (Gen 4-6 Normalize: `onModifyMove`): the holder's moves change
    /// type (`engine::modified_move`).
    pub modify_type: Option<TypeChange>,
    /// Showdown `onBasePower` on `typeChangerBoosted`: a move whose type this ability changed
    /// gets `DamageRules::type_change_boost`.
    pub type_change_boost: bool,
    /// Showdown `onAnyAfterSetStatus` (Poison Puppeteer): after a status is set on any
    /// Pokémon, once the target's own `after_set_status` has run.
    pub any_after_set_status: Option<AnyAfterSetStatusHandler>,
    /// Merciless (`onModifyCritRatio`): the holder's hits on a poisoned target are critical
    /// (the ratio becomes 5).
    pub crit_against_poisoned: bool,
    /// Emergency Exit, Wimp Out (`onEmergencyExit`): once the holder's HP falls from above half
    /// to half or less, it switches out (`BattleContext::emergency_exit`).
    pub emergency_exit: bool,
    /// Showdown `onAfterBoost` (Rattled from Gen 8): after one of the holder's stages moved.
    pub after_boost: Option<AfterBoostHandler>,
    /// Showdown `onAfterEachBoost` (Defiant, Competitive): after each of the holder's stages
    /// moved, right after that one stat (before the rest of a multi-stat change).
    pub after_each_boost: Option<AfterBoostHandler>,
    /// Showdown `onAnyFaint` (Soul-Heart): a Pokémon fainted (`BattleContext::faint_event`).
    pub any_faint: Option<AnyFaintHandler>,
    /// Showdown `onAllyFaint` (Receiver, Power of Alchemy): an ally fainted
    /// (`BattleContext::faint_event`).
    pub ally_faint: Option<AllyFaintHandler>,
    /// Showdown `onSetStatus` refusing these statuses to the holder, whoever causes them
    /// (Immunity, Pastel Veil: poison; Insomnia, Vital Spirit: sleep; Limber: paralysis; Water
    /// Veil, Water Bubble, Thermal Exchange: burn — DESIGN.md R270).
    pub blocks_statuses: &'static [NonVolatileStatus],
    /// Shields Down (`onSetStatus`, `onTryAddVolatile`, DESIGN.md §43.4): while the holder
    /// battles as this species (its Meteor forme) it takes none of these statuses or volatiles.
    pub in_forme_blocks: Option<FormeGuard>,
    /// Showdown `onImmunity` (`runStatusImmunity`, ahead of every `SetStatus` /
    /// `TryAddVolatile` handler): Magma Armor's freeze, Oblivious's infatuation.
    pub status_immunity: Option<StatusImmunity>,
    /// Showdown `onUpdate` curing the holder (`BattleContext::run_update`).
    pub update: Option<AbilityUpdate>,

    /// Showdown `onAnySwitchIn` and its priority (Pastel Veil: its `onStart` again, 0;
    /// Opportunist −3), after any Pokémon's switch-in handlers of a higher priority
    /// (`BattleEngine::run_any_switch_in`).
    pub any_switch_in: Option<(i8, SwitchHandler)>,
    /// Showdown `onFoeAfterBoost` (Opportunist): after a foe's stage changed, with what changed.
    pub foe_after_boost: Option<crate::effects::items::ItemAfterBoostHandler>,
    /// Showdown `onAnyAfterMove` (Opportunist): after any Pokémon's move
    /// (`MovePipeline::any_after_move`).
    pub any_after_move: Option<SwitchHandler>,
    /// Showdown `onAnyAfterTerastallization` (Opportunist): after any Pokémon Terastallized
    /// (`BattleEngine::run_after_terastallization`, DESIGN.md §44.6).
    pub any_after_terastallization: Option<SwitchHandler>,
    /// Showdown `onAnyAfterMega` (DESIGN.md §44.3): the same handler as `onAnyAfterMove` for
    /// Opportunist (the raises kept while the foe Mega Evolves are copied).
    pub any_after_mega: Option<SwitchHandler>,
    /// Showdown `onAfterTerastallization` (Teraform Zero): the holder Terastallized
    /// (`BattleEngine::run_after_terastallization`, DESIGN.md §44.6).
    pub after_terastallization: Option<SwitchHandler>,
    /// Dazzling, Queenly Majesty, Armor Tail (`onFoeTryMove`): a foe's move of raised priority
    /// aimed at the holder's side (or Perish Song, Flower Shield, Rototiller) fails
    /// (`engine::priority_guard`).
    pub blocks_priority_moves: bool,
    /// Own Tempo's `onHit`: a move whose `volatileStatus` is confusion is announced as refused
    /// (`-immune|p|confusion`); the confusion itself is its `blocks_volatiles`.
    pub announces_blocked_confusion: bool,
    /// Trace (`onStart`, `onUpdate`): it copies a foe's ability as it comes in, and at every
    /// `Update` while it found none (`BattleContext::start_trace`). One component for both: the
    /// gen3 data has no `onUpdate`, but the Gen 3 cartridge looks again too (issues.md I83).
    pub trace: bool,
    /// Showdown `onSourceAfterFaint` (Moxie, Beast Boost): Pokémon fainted, the last of them to
    /// the holder (`BattleContext::after_faint`).
    pub source_after_faint: Option<SourceAfterFaintHandler>,
    /// Showdown `onSideConditionStart` (Wind Power, Wind Rider): a side condition started on
    /// the holder's side (`BattleContext::side_condition_started`).
    pub side_condition_start: Option<SideConditionStartHandler>,
    /// Showdown `onAllyTryHitSide` (Sap Sipper): a partner's move aimed at the holder's side.
    pub ally_try_hit_side: Option<AllyTryHitSideHandler>,
    /// Showdown `onHit` (Anger Point): a damaging hit on the holder's HP, after the move's own
    /// `onHit` (`runMoveEffects`' `runEvent('Hit')`), before the secondaries.
    pub on_hit: Option<TargetHitHandler>,
    /// Showdown `onAfterSubDamage` (Gen 4 Anger Point): a hit the holder's Substitute took,
    /// after its recoil and drain.
    pub after_sub_damage: Option<TargetHitHandler>,
    /// Klutz: the holder's item has no effect (`engine::held_item`), but for one Showdown marks
    /// `ignoreKlutz`. Its `onStart` sends `End` to the item, which no implemented item answers.
    pub suppresses_own_item: bool,
    /// Sticky Hold (`onTakeItem`): the holder's item is not taken by another, nor knocked off
    /// (`BattleContext::take_item`; Gen 5+ a fainted holder's or a Sticky Barb aside,
    /// `ItemTransferRules::sticky_hold_spares_fainted_or_barb`).
    pub sticky_hold: bool,
    /// Gen 4 Multitype (the gen4 mod's `onTakeItem: false`): the holder's item is never taken.
    pub keeps_item: bool,
    /// Gen 4 Multitype (`onSetAbility: false`): the holder's ability is never changed
    /// (DESIGN.md §43.4).
    pub refuses_ability_change: bool,
    /// Unburden (`onTakeItem`, `onAfterUseItem`, `onEnd`): an attempt to take the holder's item,
    /// or its own use of it, gives it the `unburden` volatile, which doubles its Speed while it
    /// holds nothing; the volatile goes as the holder leaves.
    pub unburden: bool,
    /// Magician (`onAfterMoveSecondarySelf`): after its damaging move, an empty-handed holder
    /// takes the item of the first Pokémon it hit, fastest first (`BattleContext::magician`).
    pub magician: bool,
    /// Pickpocket (`onAfterMoveSecondary`): an empty-handed holder takes the item of the
    /// Pokémon whose contact move hit it (`BattleContext::pickpocket`).
    pub pickpocket: bool,
    /// Symbiosis (`onAllyAfterUseItem`): once a partner uses up its item, the holder hands it
    /// its own (`BattleContext::after_use_item`).
    pub symbiosis: bool,
    /// Showdown `onTryBoost` at `onTryBoostPriority` 2 (Guard Dog): a change to the holder's
    /// stages, capped, before the item's and the other abilities' TryBoost.
    pub try_boost_first: Option<TryBoostFirstHandler>,
    /// Showdown `onEnd` that ends a volatile the ability started (Flash Fire's `flashfire`). An
    /// ability ends here only as its holder leaves the field, which ends the volatile anyway
    /// (it is `noCopy`), so this is the data alone.
    pub ends_volatile: Option<crate::state::VolatileConditionKind>,
    /// Showdown `onSourceTryHeal` (Liquid Ooze): a heal taken off the holder
    /// (`BattleContext::drain_heal`).
    pub source_try_heal: Option<SourceTryHealHandler>,
    /// Showdown `onAllySetStatus` (Flower Veil, Sweet Veil): asked as a status is set on the
    /// holder or one of its active allies (Showdown's allies include the holder).
    pub ally_try_set_status: Option<TrySetStatusHandler>,
    /// Showdown `onAllyTryBoost` (Flower Veil): asked as a stat of the holder or one of its
    /// active allies changes, after the Pokémon's own item and ability.
    pub ally_try_stat_change: Option<TryStatChangeHandler>,
    /// Showdown `onTryAddVolatile` / `onAllyTryAddVolatile` with a condition or for allies
    /// (Leaf Guard, Sweet Veil, Flower Veil, Aroma Veil); see `VolatileGuard`.
    pub volatile_guard: Option<VolatileGuard>,
    /// Comatose: the holder counts as asleep where Showdown asks `hasAbility('comatose')`
    /// (Dream Eater, Hex, Infernal Parade, Wake-Up Slap, Nightmare:
    /// `engine::counts_as_asleep`). Its `onStart` only announces it.
    pub counts_as_asleep: bool,
    /// Unnerve, As One (`onFoeTryEatItem`): once the holder's `switch_in` has started it
    /// (`AbilityState::unnerved`, reset as it leaves: `onEnd`), no foe eats a berry.
    pub unnerve: bool,
    /// The kind of switch-in effect it starts, as the Gen 4 cartridge marks it announced
    /// (`StartNewAbility::Unannounced`); data, read in no other generation.
    pub announces: Option<crate::state::SwitchInAnnouncement>,
    /// Neutralizing Gas (DESIGN.md §41.2): while the holder is on the field every other ability
    /// is set aside (`engine::ignoring_ability`); its `onEnd` starts them again
    /// (`BattleContext::end_ability`).
    pub neutralizing_gas: bool,
    /// Pressure (`onDeductPP`): a move that meets the holder costs its user one more PP
    /// (`engine::pressure_holders`, DESIGN.md §15.15).
    pub pressure: bool,
    /// Gluttony: the holder eats a pinch berry at half HP (`HpShare::Pinch`). Showdown sets
    /// `abilityState.gluttony` as the ability starts and whenever the holder takes damage; the
    /// berries read the ability and that flag, which is always set for a holder on the field
    /// (the engine has no ability change without a start).
    pub gluttony: bool,
    /// Ripen (`onTryHeal`, `onTryEatItem`, `onEatItem`, `onSourceModifyDamage`): a berry's heal
    /// is doubled (Leftovers' and Berry Juice's only announce it), eating a berry announces it,
    /// and a hit a resist berry halved is halved again. Its `onChangeBoost` is the
    /// `modify_stat_change` handler `ripen::change_boost`.
    pub ripen: bool,
    /// Cheek Pouch (`onEatItem`): once a berry's own effect is over, the holder heals this share
    /// of its HP.
    pub eaten_berry_heal: Option<crate::rules::Fraction>,
    /// Cud Chew (`onEatItem`; its `onResidual` is its `residual`): a berry the holder eats it
    /// eats again at the second turn's end after (`AbilityState::cud_chew`).
    pub chews_the_cud: bool,
    /// Berserk, Anger Shell (`onDamage`, `onTryEatItem`, `onAfterMoveSecondary`): a move that
    /// strikes once and hurts the holder keeps its healing berries waiting for the move's end,
    /// where a drop from above half its HP to half or below changes these stats.
    pub half_hp_boosts: Option<HalfHpBoosts>,
    /// Protean, Libero (`onPrepareHit`): the holder takes its move's type before the move hits
    /// (Gen 9 once a stint, `AbilityRules`-free: `ConditionRules::protean_once_per_entry`).
    pub retypes_before_hit: bool,
    /// Gen 5+ Color Change (`onAfterMoveSecondary`): a damaging move that reached the holder
    /// gives it the move's type, once the move is over.
    pub retypes_after_move: bool,
    /// Flower Gift (`onAllyModifyAtk` priority 3, `onAllyModifySpD` priority 4): in the sun the
    /// holder's side — itself included — has ×1.5 Attack and Special Defense
    /// (`WeatherRules::flower_gift`, DESIGN.md §23.6). Its `onStart` / `onWeatherChange` switch
    /// Cherrim between its two formes, which have the same numbers
    /// (`BattleContext::flower_gift`, §43.2).
    pub flower_gift: bool,
    /// Desolate Land, Primordial Sea, Delta Stream (`onStart`, `onAnySetWeather`, `onEnd`): the
    /// strong weather the holder sets as it enters and ends as it leaves (DESIGN.md §23.5).
    pub strong_weather: Option<crate::state::WeatherKind>,
    /// Forecast (`onStart`, `onWeatherChange`): the holding species takes the forme of the
    /// weather it sees — only these formes' types differ (DESIGN.md §23.6, §43.2).
    pub forecast: Option<Forecast>,
    /// Mimicry (`onStart`, `onTerrainChange`): the holder takes the terrain's type, and its own
    /// types back when there is none (DESIGN.md §23.6).
    pub mimicry: bool,
    /// Shadow Tag, Arena Trap, Magnet Pull (`onFoeTrapPokemon`, `onFoeMaybeTrapPokemon`): an
    /// adjacent foe the filter takes cannot switch (`engine::trapped_by_ability`).
    pub traps_foes: Option<TrapFilter>,
    /// Gen 3 Magnet Pull (`onAnyTrapPokemon`, `onAnyMaybeTrapPokemon`): any adjacent Pokémon the
    /// filter takes, an ally too.
    pub traps_any: Option<TrapFilter>,
    /// Truant (`onBeforeMove`, priority 9; DESIGN.md §15.21): every other turn the holder loafs
    /// instead of moving, as the generation keeps count (`TruantRules`, BeforeMove's
    /// `BeforeMoveBlockReason::Truant`). Its `onStart` / Gen 3 `onSwitchIn` and Gen 3
    /// `onResidual` are the `switch_in` and `residual` handlers of `truant.rs`.
    pub truant: bool,
    /// Gen 6 Stance Change (`onBeforeMove`, priority 11): the holder's forme for the move it is
    /// about to use (`BattleContext::stance_change`, DESIGN.md §43.4).
    pub before_move_stance: Option<StanceChange>,
    /// Gen 7+ Stance Change (`onModifyMove`, priority 1): the same as the move starts — a called
    /// move too.
    pub move_stance: Option<StanceChange>,
    /// Zen Mode, Schooling, Shields Down, Hunger Switch, Power Construct (`onStart`,
    /// `onResidual` order 29): the forme the holder takes by itself (DESIGN.md §43.4).
    pub self_forme: Option<SelfForme>,
    /// Disguise, Ice Face (`onDamage` priority 1, `onCriticalHit`, `onEffectiveness`,
    /// `onUpdate`): the forme that takes one hit whole (DESIGN.md §43.4).
    pub absorbing: Option<AbsorbingForme>,
    /// Gulp Missile (`onSourceTryPrimaryHit`, `onDamagingHit`): the Cramorant that dives with
    /// Surf or Dive and spits its catch out when it is hit (DESIGN.md §43.4).
    pub gulp_missile: bool,
    /// Showdown `onSourceTryPrimaryHit`: the holder's move, as it is about to strike another
    /// Pokémon (Gulp Missile's Surf).
    pub source_try_primary_hit: Option<SourceTryPrimaryHitHandler>,
    /// Showdown `onWeatherChange` (Ice Face): the weather around the holder changed.
    pub weather_change: Option<SwitchHandler>,
    /// Battle Bond (`onSourceAfterFaint`): the Greninja that grows on a knock-out
    /// (DESIGN.md §43.4).
    pub battle_bond: bool,
    /// Zero to Hero (`onSwitchOut`, `onSwitchIn`): the Palafin that becomes its Hero forme as
    /// it leaves (DESIGN.md §43.4).
    pub zero_to_hero: bool,
    /// Commander (`onStart`, `onAnySwitchIn` priority −2, `onUpdate`): a Tatsugiri beside a
    /// Dondozo takes it into its mouth, which is the `commanding` / `commanded` volatiles
    /// (DESIGN.md §43.4).
    pub commander: bool,
}

pub use forme::{HpDenominator, SelfForme, SelfFormeRule};
pub use forme_ability::AbsorbingForme;

/// What a forme of the holder refuses (DESIGN.md §43.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormeGuard {
    pub forme: crate::ids::SpeciesId,
    pub statuses: &'static [NonVolatileStatus],
    pub volatiles: &'static [crate::state::VolatileConditionKind],
}

impl FormeGuard {
    /// Whether the guard holds for a Pokémon battling as `battling`.
    pub fn holds(self, battling: Option<crate::ids::SpeciesId>) -> bool {
        battling == Some(self.forme)
    }
}

/// Disguise's forme: Mimikyu's own, whole until a damaging move breaks it (DESIGN.md §43.4).
fn disguise() -> AbsorbingForme {
    AbsorbingForme {
        whole: crate::ids::SpeciesId::MIMIKYU,
        broken: crate::ids::SpeciesId::MIMIKYU_BUSTED,
        totem: Some((
            crate::ids::SpeciesId::MIMIKYU_TOTEM,
            crate::ids::SpeciesId::MIMIKYU_BUSTED_TOTEM,
        )),
        physical_only: false,
        break_cost: Some(crate::rules::Fraction::new(1, 8)),
    }
}

/// Ice Face's: only a physical move breaks it.
fn ice_face() -> AbsorbingForme {
    AbsorbingForme {
        whole: crate::ids::SpeciesId::EISCUE,
        broken: crate::ids::SpeciesId::EISCUE_NOICE,
        totem: None,
        physical_only: true,
        break_cost: None,
    }
}

/// A self-changing ability's residual (`onResidualOrder` 29).
fn forme_residual() -> ResidualParticipant {
    ResidualParticipant {
        kind: crate::rules::ResidualKind::FormeChange,
        handler: forme::residual as ResidualHandler,
    }
}

/// Zen Mode (DESIGN.md §43.4): Darmanitan's Zen forme, or the Galar one, decided by which base
/// forme the holder is.
fn zen_mode() -> SelfForme {
    SelfForme {
        holder: crate::ids::SpeciesId::DARMANITAN,
        rule: SelfFormeRule::ZenMode {
            at: HpDenominator::Half,
            other: crate::ids::SpeciesId::DARMANITAN_ZEN,
            galar: Some((
                crate::ids::SpeciesId::DARMANITAN_GALAR,
                crate::ids::SpeciesId::DARMANITAN_GALAR_ZEN,
            )),
        },
    }
}

/// Stance Change's formes (DESIGN.md §43.4): a holding species of its own, not transformed,
/// takes `attack` for a damaging move and `shield` for `shield_move`; another status move
/// changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StanceChange {
    pub attack: crate::ids::SpeciesId,
    pub shield: crate::ids::SpeciesId,
    pub shield_move: crate::ids::MoveId,
}

/// Showdown `onImmunity` for a status or a volatile (`AbilityBehavior::status_immunity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusImmunity {
    /// Magma Armor: `frz`.
    Status(NonVolatileStatus),
    /// Oblivious: `attract`.
    Volatile(crate::state::VolatileConditionKind),
}

/// What an ability's `onUpdate` cures on its holder (`AbilityBehavior::update`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityUpdate {
    /// Disguise, Ice Face: the forme that took a hit whole breaks into its other one
    /// (`forme_ability::AbsorbingForme::update`).
    BustedForme(AbsorbingForme),
    /// Commander: a Tatsugiri beside a Dondozo takes it into its mouth
    /// (`forme_ability::commander`).
    Commander,
    /// `-activate|p|ability: X` and `cureStatus()` while the holder has one of these.
    CureStatus(&'static [NonVolatileStatus]),
    /// Own Tempo (confusion), Oblivious (infatuation; Gen 6+ Taunt too:
    /// `ConditionRules::oblivious_covers_taunt`): these volatiles end.
    EndVolatiles(&'static [crate::state::VolatileConditionKind]),
}

/// Showdown `onTryAddVolatile` (the holder) or `onAllyTryAddVolatile` (the holder and its
/// active allies) returning `null`: these volatiles are never added while `when` holds for the
/// Pokémon they would go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolatileGuard {
    pub kinds: &'static [crate::state::VolatileConditionKind],
    pub allies: bool,
    pub when: VolatileGuardWhen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolatileGuardWhen {
    Always,
    /// Leaf Guard: in sun.
    Sun,
    /// Flower Veil: the Pokémon is Grass-type.
    GrassType,
}

/// Whom a trapping ability keeps from switching (DESIGN.md §30.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapFilter {
    /// Shadow Tag: anyone — from Gen 4 not another Shadow Tag
    /// (`SwitchingRules::shadow_tag_spares_its_like`).
    Anyone,
    /// Arena Trap: a grounded Pokémon (`engine::environment::grounded`).
    Grounded,
    /// Magnet Pull: a Steel type.
    SteelType,
}

/// Which of the holder's moves an ability retypes, and to what.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeChange {
    pub scope: TypeChangeScope,
    pub to: crate::dex::PokemonType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeChangeScope {
    /// Aerilate, Galvanize, Pixilate, Refrigerate: Normal-type moves, but not those that set
    /// their own type (Weather Ball, Revelation Dance …).
    NormalMoves,
    /// Liquid Voice: sound moves.
    SoundMoves,
    /// Normalize: every move but Struggle (Gen 7+: and those that set their own type).
    AllMoves,
}

/// An absorbing ability's side effect after refusing a move (`absorb_reaction`).
/// Showdown: part of `onTryHit`.
pub type AbsorbReactionHandler = for<'a> fn(&mut BattleContext<'a>, PokemonId);

/// A reaction to a damaging hit with no `onDamagingHitOrder` (after the ordered ones).
fn unordered_reaction(handler: crate::engine::DamagingHitReactionHandler) -> AbilityBehavior {
    AbilityBehavior {
        damaging_hit_reaction: Some(OrderedHandler::new(UNORDERED_REACTION, handler)),
        ..AbilityBehavior::default()
    }
}

/// An ability that keeps these statuses off its holder (`onSetStatus`) and cures them at the
/// next `Update` (`onUpdate`): Immunity, Insomnia, Limber … (DESIGN.md R270).
fn cures(statuses: &'static [NonVolatileStatus]) -> AbilityBehavior {
    AbilityBehavior {
        blocks_statuses: statuses,
        update: Some(AbilityUpdate::CureStatus(statuses)),
        ..AbilityBehavior::default()
    }
}

/// The ability behavior components (`effects::BehaviorComponent`, DESIGN.md §25.1).
const COMPONENTS: &[BehaviorComponent<AbilityBehavior>] = &[
    BehaviorComponent {
        source_callbacks: |h| match h.accuracy_boost {
            Some(_) => &["onSourceModifyAccuracy"],
            None => &[],
        },
        disable: |h| h.accuracy_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match (h.evasion_boost, h.status_accuracy) {
            (None, None) => &[],
            _ => &["onModifyAccuracy"],
        },
        disable: |h| {
            h.evasion_boost = None;
            h.status_accuracy = None;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ally_accuracy_boost {
            Some(_) => &["onAnyModifyAccuracy"],
            None => &[],
        },
        disable: |h| h.ally_accuracy_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.no_guard {
                &["onAnyInvulnerability", "onAnyAccuracy"]
            } else {
                &[]
            }
        },
        disable: |h| h.no_guard = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.damage_taken {
            Some(_) => &["onSourceModifyDamage"],
            None => &[],
        },
        disable: |h| h.damage_taken = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.foe_offense {
            Some(_) => &["onSourceModifyAtk", "onSourceModifySpA"],
            None => &[],
        },
        disable: |h| h.foe_offense = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.foe_base_power {
            Some(_) => &["onSourceBasePower"],
            None => &[],
        },
        disable: |h| h.foe_base_power = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.defense_boost {
            Some(_) => &["onModifyDef"],
            None => &[],
        },
        disable: |h| h.defense_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.halves_status_damage {
            Some(_) => &["onDamage"],
            None => &[],
        },
        disable: |h| h.halves_status_damage = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.weather_immunity {
            Some(_) => &["onImmunity"],
            None => &[],
        },
        disable: |h| h.weather_immunity = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.damage_boost {
            Some(_) => &["onModifyDamage"],
            None => &[],
        },
        disable: |h| h.damage_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.stab_modifier {
            Some(_) => &["onModifySTAB"],
            None => &[],
        },
        disable: |h| h.stab_modifier = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.base_power_boost {
            // Supreme Overlord's `onEnd` only hides its count (a silent `-end`).
            Some(AbilityBoost {
                modifier: BoostModifier::FallenAllies,
                ..
            }) => &["onBasePower", "onEnd"],
            Some(_) => &["onBasePower"],
            None => &[],
        },
        disable: |h| h.base_power_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.offense_boost {
            Some(OffenseBoost {
                stats: OffenseStats::Attack,
                ..
            }) => &["onModifyAtk"],
            Some(OffenseBoost {
                stats: OffenseStats::SpecialAttack,
                ..
            }) => &["onModifySpA"],
            Some(OffenseBoost {
                stats: OffenseStats::Both,
                ..
            }) => &["onModifyAtk", "onModifySpA"],
            None => &[],
        },
        disable: |h| h.offense_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.modify_type {
            Some(_) => &["onModifyType", "onModifyMove"],
            None => &[],
        },
        disable: |h| h.modify_type = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.type_change_boost {
                &["onBasePower"]
            } else {
                &[]
            }
        },
        disable: |h| h.type_change_boost = false,
    },
    BehaviorComponent {
        // Showdown's `onStart`, or an `onSwitchIn` (Neutralizing Gas; sorted by
        // `onSwitchInPriority`): either runs as the holder comes in.
        source_callbacks: |h| match h.switch_in {
            Some(_) => &["onStart", "onSwitchIn"],
            None => &[],
        },
        disable: |h| h.switch_in = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.illusion {
                &["onBeforeSwitchIn", "onEnd", "onFaint"]
            } else {
                &[]
            }
        },
        disable: |h| h.illusion = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.entry {
            Some(_) => &["onSwitchIn"],
            None => &[],
        },
        disable: |h| h.entry = None,
    },
    BehaviorComponent {
        // An absorbing ability's side effect belongs to its `onTryHit`.
        source_callbacks: |h| match (h.try_hit, h.absorb_reaction) {
            (None, None) => &[],
            _ => &["onTryHit"],
        },
        disable: |h| {
            h.try_hit = None;
            h.absorb_reaction = None;
        },
    },
    BehaviorComponent {
        // Magic Bounce declares `onTryHit` and `onAllyTryHitSide`; the reflection itself is the
        // hit gate's (`MovePipeline::reflect_move`, §42.8).
        source_callbacks: |h| {
            if h.magic_bounce {
                &["onTryHit", "onAllyTryHitSide"]
            } else {
                &[]
            }
        },
        disable: |h| h.magic_bounce = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.try_set_status {
            Some(_) => &["onSetStatus"],
            None => &[],
        },
        disable: |h| h.try_set_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.blocks_statuses.is_empty() {
                &[]
            } else {
                &["onSetStatus"]
            }
        },
        disable: |h| h.blocks_statuses = &[],
    },
    BehaviorComponent {
        source_callbacks: |h| match h.status_immunity {
            Some(_) => &["onImmunity"],
            None => &[],
        },
        disable: |h| h.status_immunity = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.update {
            Some(_) => &["onUpdate"],
            None => &[],
        },
        disable: |h| h.update = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_switch_in {
            Some(_) => &["onAnySwitchIn"],
            None => &[],
        },
        disable: |h| h.any_switch_in = None,
    },
    BehaviorComponent {
        // The raises it keeps live in the ability's state, which `End` (a new ability, leaving
        // the field) starts over: Opportunist's `onEnd`.
        source_callbacks: |h| match h.foe_after_boost {
            Some(_) => &["onFoeAfterBoost", "onEnd"],
            None => &[],
        },
        disable: |h| h.foe_after_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_after_move {
            Some(_) => &["onAnyAfterMove"],
            None => &[],
        },
        disable: |h| h.any_after_move = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_after_mega {
            Some(_) => &["onAnyAfterMega"],
            None => &[],
        },
        disable: |h| h.any_after_mega = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_after_terastallization {
            Some(_) => &["onAnyAfterTerastallization"],
            None => &[],
        },
        disable: |h| h.any_after_terastallization = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_terastallization {
            Some(_) => &["onAfterTerastallization"],
            None => &[],
        },
        disable: |h| h.after_terastallization = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.blocks_priority_moves {
                &["onFoeTryMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.blocks_priority_moves = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.announces_blocked_confusion {
                &["onHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.announces_blocked_confusion = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_set_status {
            Some(_) => &["onAfterSetStatus"],
            None => &[],
        },
        disable: |h| h.after_set_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.modify_offense {
            Some(_) => &["onModifyAtk"],
            None => &[],
        },
        disable: |h| h.modify_offense = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.base_power {
            Some(_) => &["onBasePower"],
            None => &[],
        },
        disable: |h| h.base_power = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ally_base_power {
            Some(_) => &["onAllyBasePower"],
            None => &[],
        },
        disable: |h| h.ally_base_power = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.team_base_power {
            Some(_) => &["onAllyBasePower"],
            None => &[],
        },
        disable: |h| h.team_base_power = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.aura {
            Some(_) => &["onAnyBasePower", "onStart"],
            None => &[],
        },
        disable: |h| h.aura = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.aura_break {
                &["onAnyTryPrimaryHit", "onStart"]
            } else {
                &[]
            }
        },
        disable: |h| h.aura_break = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ruin {
            Some(crate::state::Stat::Attack) => &["onAnyModifyAtk", "onStart"],
            Some(crate::state::Stat::Defense) => &["onAnyModifyDef", "onStart"],
            Some(crate::state::Stat::SpecialAttack) => &["onAnyModifySpA", "onStart"],
            Some(crate::state::Stat::SpecialDefense) => &["onAnyModifySpD", "onStart"],
            _ => &[],
        },
        disable: |h| h.ruin = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ally_damage_modifier {
            Some(_) => &["onAnyModifyDamage"],
            None => &[],
        },
        disable: |h| h.ally_damage_modifier = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.damaging_hit_reaction {
            Some(_) => &["onDamagingHit"],
            None => &[],
        },
        disable: |h| h.damaging_hit_reaction = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.damage_resolution {
            Some(_) => &["onDamage"],
            None => &[],
        },
        disable: |h| h.damage_resolution = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.modify_stat_change {
            Some(_) => &["onChangeBoost"],
            None => &[],
        },
        disable: |h| h.modify_stat_change = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.doubles_read_stages {
                &["onModifyBoost"]
            } else {
                &[]
            }
        },
        disable: |h| h.doubles_read_stages = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.weight_modifier {
            Some(_) => &["onModifyWeight"],
            None => &[],
        },
        disable: |h| h.weight_modifier = None,
    },
    BehaviorComponent {
        source_callbacks: |h| if h.blocks_recoil { &["onDamage"] } else { &[] },
        disable: |h| h.blocks_recoil = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.unaware {
                &["onAnyModifyBoost"]
            } else {
                &[]
            }
        },
        disable: |h| h.unaware = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.move_edit {
            Some(_) => &["onModifyMove"],
            None => &[],
        },
        disable: |h| h.move_edit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.parental_bond {
                &["onPrepareHit", "onSourceModifySecondaries"]
            } else {
                &[]
            }
        },
        disable: |h| h.parental_bond = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.choice_lock {
                &[
                    "onStart",
                    "onModifyMove",
                    "onBeforeMove",
                    "onDisableMove",
                    "onEnd",
                ]
            } else {
                &[]
            }
        },
        disable: |h| h.choice_lock = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.try_stat_change {
            Some(_) => &["onTryBoost"],
            None => &[],
        },
        disable: |h| h.try_stat_change = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.reflects_stat_drops {
                &["onTryBoost"]
            } else {
                &[]
            }
        },
        disable: |h| h.reflects_stat_drops = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.blocks_volatiles.is_empty() {
                &[]
            } else {
                &["onTryAddVolatile"]
            }
        },
        disable: |h| h.blocks_volatiles = &[],
    },
    BehaviorComponent {
        source_callbacks: |h| match h.flinch_boost {
            Some(_) => &["onFlinch"],
            None => &[],
        },
        disable: |h| h.flinch_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.shields_secondaries {
                &["onModifySecondaries"]
            } else {
                &[]
            }
        },
        disable: |h| h.shields_secondaries = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.hit_reaction {
            Some(_) => &["onDamagingHit"],
            None => &[],
        },
        disable: |h| h.hit_reaction = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.dealt_hit_status {
            Some(_) => &["onSourceDamagingHit"],
            None => &[],
        },
        disable: |h| h.dealt_hit_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.redirect_type {
            // Gen 3 Lightning Rod: `onFoeRedirectTarget` (`ConditionRules::
            // redirect_ability_foes_only`, `redirect_reads_dex_type`).
            Some(_) => &["onAnyRedirectTarget", "onFoeRedirectTarget"],
            None => &[],
        },
        disable: |h| h.redirect_type = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.contact_status {
            Some(_) => &["onDamagingHit"],
            None => &[],
        },
        disable: |h| h.contact_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.residual {
            Some(_) => &["onResidual"],
            None => &[],
        },
        disable: |h| h.residual = None,
    },
    BehaviorComponent {
        // `onCheckShow` is Natural Cure's client-display bookkeeping, no battle semantics.
        source_callbacks: |h| match h.switch_out {
            Some(_) => &["onSwitchOut", "onCheckShow"],
            None => &[],
        },
        disable: |h| h.switch_out = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.blocks_drag_out {
                &["onDragOut"]
            } else {
                &[]
            }
        },
        disable: |h| h.blocks_drag_out = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.secondary_chance_multiplier {
            Some(_) => &["onModifyMove"],
            None => &[],
        },
        disable: |h| h.secondary_chance_multiplier = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.infiltrates {
                &["onModifyMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.infiltrates = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.quark_drive {
                &["onStart", "onTerrainChange", "onEnd"]
            } else {
                &[]
            }
        },
        disable: |h| h.quark_drive = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.speed_boost {
            // Slow Start's `onEnd` only announces the end (`-end`, silent when called back).
            Some(SpeedBoost {
                when: SpeedWhen::SlowStart,
                ..
            }) => &["onModifySpe", "onEnd"],
            Some(_) => &["onModifySpe"],
            None => &[],
        },
        disable: |h| h.speed_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.suppresses_weather {
                &["onSwitchIn", "onStart", "onEnd"]
            } else {
                &[]
            }
        },
        disable: |h| h.suppresses_weather = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.weather_reactions.is_empty() {
                &[]
            } else {
                &["onWeather"]
            }
        },
        disable: |h| h.weather_reactions = &[],
    },
    BehaviorComponent {
        source_callbacks: |h| match h.forecast {
            Some(_) => &["onStart", "onWeatherChange"],
            None => &[],
        },
        disable: |h| h.forecast = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.mimicry {
                &["onStart", "onTerrainChange"]
            } else {
                &[]
            }
        },
        disable: |h| h.mimicry = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.flower_gift {
                &[
                    "onAllyModifyAtk",
                    "onAllyModifySpD",
                    "onStart",
                    "onWeatherChange",
                ]
            } else {
                &[]
            }
        },
        disable: |h| h.flower_gift = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.strong_weather {
            Some(_) => &["onStart", "onAnySetWeather", "onEnd"],
            None => &[],
        },
        disable: |h| h.strong_weather = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.traps_foes {
            Some(_) => &["onFoeTrapPokemon", "onFoeMaybeTrapPokemon"],
            None => &[],
        },
        disable: |h| h.traps_foes = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.traps_any {
            Some(_) => &["onAnyTrapPokemon", "onAnyMaybeTrapPokemon"],
            None => &[],
        },
        disable: |h| h.traps_any = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.damp {
                &["onAnyTryMove", "onAnyDamage"]
            } else {
                &[]
            }
        },
        disable: |h| h.damp = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.protosynthesis {
                &["onStart", "onWeatherChange", "onEnd"]
            } else {
                &[]
            }
        },
        disable: |h| h.protosynthesis = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.weather_and_powder_immune {
                &["onImmunity", "onTryHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.weather_and_powder_immune = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.crit_ratio_boost > 0 {
                &["onModifyCritRatio"]
            } else {
                &[]
            }
        },
        disable: |h| h.crit_ratio_boost = 0,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.blocks_critical_hit {
                &["onCriticalHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.blocks_critical_hit = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.ohko_immune { &["onTryHit"] } else { &[] }
        },
        disable: |h| h.ohko_immune = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.fractional_priority {
            Some(_) => &["onFractionalPriority"],
            None => &[],
        },
        disable: |h| h.fractional_priority = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.modify_priority {
            Some(_) => &["onModifyPriority"],
            None => &[],
        },
        disable: |h| h.modify_priority = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_after_set_status {
            Some(_) => &["onAnyAfterSetStatus"],
            None => &[],
        },
        disable: |h| h.any_after_set_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_boost {
            Some(_) => &["onAfterBoost"],
            None => &[],
        },
        disable: |h| h.after_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_each_boost {
            Some(_) => &["onAfterEachBoost"],
            None => &[],
        },
        disable: |h| h.after_each_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_faint {
            Some(_) => &["onAnyFaint"],
            None => &[],
        },
        disable: |h| h.any_faint = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.trace {
                &["onStart", "onUpdate"]
            } else {
                &[]
            }
        },
        disable: |h| h.trace = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ally_faint {
            Some(_) => &["onAllyFaint"],
            None => &[],
        },
        disable: |h| h.ally_faint = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.source_after_faint {
            Some(_) => &["onSourceAfterFaint"],
            None => &[],
        },
        disable: |h| h.source_after_faint = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.side_condition_start {
            Some(_) => &["onSideConditionStart"],
            None => &[],
        },
        disable: |h| h.side_condition_start = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ally_try_hit_side {
            Some(_) => &["onAllyTryHitSide"],
            None => &[],
        },
        disable: |h| h.ally_try_hit_side = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.on_hit {
            Some(_) => &["onHit"],
            None => &[],
        },
        disable: |h| h.on_hit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_sub_damage {
            Some(_) => &["onAfterSubDamage"],
            None => &[],
        },
        disable: |h| h.after_sub_damage = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ends_volatile {
            Some(_) => &["onEnd"],
            None => &[],
        },
        disable: |h| h.ends_volatile = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.suppresses_own_item {
                &["onStart"]
            } else {
                &[]
            }
        },
        disable: |h| h.suppresses_own_item = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.sticky_hold { &["onTakeItem"] } else { &[] }
        },
        disable: |h| h.sticky_hold = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.keeps_item { &["onTakeItem"] } else { &[] }
        },
        disable: |h| h.keeps_item = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.refuses_ability_change {
                &["onSetAbility"]
            } else {
                &[]
            }
        },
        disable: |h| h.refuses_ability_change = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.magician {
                &["onAfterMoveSecondarySelf"]
            } else {
                &[]
            }
        },
        disable: |h| h.magician = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.symbiosis {
                &["onAllyAfterUseItem"]
            } else {
                &[]
            }
        },
        disable: |h| h.symbiosis = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.pickpocket {
                &["onAfterMoveSecondary"]
            } else {
                &[]
            }
        },
        disable: |h| h.pickpocket = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.unburden {
                &["onAfterUseItem", "onTakeItem", "onEnd"]
            } else {
                &[]
            }
        },
        disable: |h| h.unburden = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.try_boost_first {
            Some(_) => &["onTryBoost"],
            None => &[],
        },
        disable: |h| h.try_boost_first = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.source_try_heal {
            Some(_) => &["onSourceTryHeal"],
            None => &[],
        },
        disable: |h| h.source_try_heal = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.emergency_exit {
                &["onEmergencyExit"]
            } else {
                &[]
            }
        },
        disable: |h| h.emergency_exit = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.crit_against_poisoned {
                &["onModifyCritRatio"]
            } else {
                &[]
            }
        },
        disable: |h| h.crit_against_poisoned = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ally_try_set_status {
            Some(_) => &["onAllySetStatus"],
            None => &[],
        },
        disable: |h| h.ally_try_set_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.ally_try_stat_change {
            Some(_) => &["onAllyTryBoost"],
            None => &[],
        },
        disable: |h| h.ally_try_stat_change = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.volatile_guard {
            Some(VolatileGuard { allies: true, .. }) => &["onAllyTryAddVolatile"],
            Some(_) => &["onTryAddVolatile"],
            None => &[],
        },
        disable: |h| h.volatile_guard = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.counts_as_asleep {
                &["onStart"]
            } else {
                &[]
            }
        },
        disable: |h| h.counts_as_asleep = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.unnerve {
                &["onFoeTryEatItem", "onEnd"]
            } else {
                &[]
            }
        },
        disable: |h| h.unnerve = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.neutralizing_gas { &["onEnd"] } else { &[] }
        },
        disable: |h| h.neutralizing_gas = false,
    },
    BehaviorComponent {
        source_callbacks: |h| if h.pressure { &["onDeductPP"] } else { &[] },
        disable: |h| h.pressure = false,
    },
    BehaviorComponent {
        source_callbacks: |h| if h.truant { &["onBeforeMove"] } else { &[] },
        disable: |h| h.truant = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.gluttony {
                &["onStart", "onDamage"]
            } else {
                &[]
            }
        },
        disable: |h| h.gluttony = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.ripen {
                &[
                    "onEatItem",
                    "onSourceModifyDamage",
                    "onTryEatItem",
                    "onTryHeal",
                ]
            } else {
                &[]
            }
        },
        disable: |h| h.ripen = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.eaten_berry_heal {
            Some(_) => &["onEatItem"],
            None => &[],
        },
        disable: |h| h.eaten_berry_heal = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.chews_the_cud { &["onEatItem"] } else { &[] }
        },
        disable: |h| h.chews_the_cud = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.half_hp_boosts {
            Some(_) => &["onAfterMoveSecondary", "onDamage", "onTryEatItem"],
            None => &[],
        },
        disable: |h| h.half_hp_boosts = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.retypes_before_hit {
                &["onPrepareHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.retypes_before_hit = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.retypes_after_move {
                &["onAfterMoveSecondary"]
            } else {
                &[]
            }
        },
        disable: |h| h.retypes_after_move = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.before_move_stance {
            Some(_) => &["onBeforeMove"],
            None => &[],
        },
        disable: |h| h.before_move_stance = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.move_stance {
            Some(_) => &["onModifyMove"],
            None => &[],
        },
        disable: |h| h.move_stance = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.self_forme {
            Some(SelfForme {
                rule: SelfFormeRule::ZenMode { .. },
                ..
            }) => &["onStart", "onResidual", "onEnd"],
            Some(_) => &["onStart", "onResidual"],
            None => &[],
        },
        disable: |h| {
            h.self_forme = None;
            h.residual = None;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| match h.absorbing {
            Some(_) => &["onDamage", "onCriticalHit", "onEffectiveness", "onUpdate"],
            None => &[],
        },
        disable: |h| {
            h.absorbing = None;
            h.update = None;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.gulp_missile {
                &["onDamagingHit", "onSourceTryPrimaryHit"]
            } else {
                &[]
            }
        },
        disable: |h| {
            h.gulp_missile = false;
            h.source_try_primary_hit = None;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.battle_bond {
                &["onSourceAfterFaint", "onModifyMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.battle_bond = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.weather_change {
            Some(_) => &["onWeatherChange"],
            None => &[],
        },
        disable: |h| h.weather_change = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.commander {
                &["onAnySwitchIn", "onStart", "onUpdate"]
            } else {
                &[]
            }
        },
        disable: |h| {
            h.commander = false;
            h.update = None;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.zero_to_hero {
                &["onSwitchIn", "onSwitchOut"]
            } else {
                &[]
            }
        },
        disable: |h| h.zero_to_hero = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.in_forme_blocks {
            Some(_) => &["onSetStatus", "onTryAddVolatile"],
            None => &[],
        },
        disable: |h| h.in_forme_blocks = None,
    },
];

impl AbilityBehavior {
    /// Every Showdown callback some component of these behavior stands for.
    pub fn covered_callbacks(&self) -> Vec<&'static str> {
        super::covered_callbacks(COMPONENTS, self)
    }

    /// These behavior without the components none of whose callbacks `declared` lists.
    pub fn restricted_to(self, declared: &[String]) -> Self {
        super::restricted_to(COMPONENTS, self, declared)
    }
}

/// Sand Veil, Snow Cloak: ×0.8 to the accuracy of moves aimed at the holder in their weather,
/// and no damage from it.
/// Embody Aspect's four variants (the `-Tera` forme's own ability, `onStart`): the stat each
/// raises, once.
fn embody_aspect_teal(ctx: &mut crate::engine::BattleContext<'_>, holder: crate::ids::PokemonId) {
    tera::embody_aspect(
        ctx,
        holder,
        crate::ids::SpeciesId::OGERPON_TEAL_TERA,
        crate::state::Stat::Speed,
    );
}

fn embody_aspect_wellspring(
    ctx: &mut crate::engine::BattleContext<'_>,
    holder: crate::ids::PokemonId,
) {
    tera::embody_aspect(
        ctx,
        holder,
        crate::ids::SpeciesId::OGERPON_WELLSPRING_TERA,
        crate::state::Stat::SpecialDefense,
    );
}

fn embody_aspect_hearthflame(
    ctx: &mut crate::engine::BattleContext<'_>,
    holder: crate::ids::PokemonId,
) {
    tera::embody_aspect(
        ctx,
        holder,
        crate::ids::SpeciesId::OGERPON_HEARTHFLAME_TERA,
        crate::state::Stat::Attack,
    );
}

fn embody_aspect_cornerstone(
    ctx: &mut crate::engine::BattleContext<'_>,
    holder: crate::ids::PokemonId,
) {
    tera::embody_aspect(
        ctx,
        holder,
        crate::ids::SpeciesId::OGERPON_CORNERSTONE_TERA,
        crate::state::Stat::Defense,
    );
}

fn evasive(weather: crate::state::WeatherKind) -> AbilityBehavior {
    AbilityBehavior {
        evasion_boost: Some(AccuracyBoost {
            when: AccuracyWhen::Weather(weather),
            modifier: AccuracyModifier::new(3277, 80, AccuracyStep::WeatherEvasion),
        }),
        weather_immunity: Some(weather),
        ..AbilityBehavior::default()
    }
}

/// Hustle's Attack: `this.modify(atk, 1.5)` on the stat rather than a `chainModify`
/// (`offense_on_stat`): beside a Choice Band the Attack is rounded twice, 145 → 217 → 325, not
/// once at ×2.25 (326, R-M10-65).
fn hustle_offense(ctx: &mut OffensiveStatContext<'_>) -> Option<u32> {
    (ctx.move_category() == crate::dex::MoveCategory::Physical).then_some(6144)
}

/// A BasePower boost (`onBasePower` at `priority`).
fn power(when: BoostWhen, modifier: u32, priority: i16) -> AbilityBehavior {
    AbilityBehavior {
        base_power_boost: Some(AbilityBoost {
            when,
            modifier: BoostModifier::Fixed(modifier),
            priority,
        }),
        ..AbilityBehavior::default()
    }
}

/// An attacking-stat boost for moves of one type (`onModifyAtk` / `onModifySpA`, priority 5).
fn offense(move_type: crate::dex::PokemonType, modifier: BoostModifier) -> AbilityBehavior {
    AbilityBehavior {
        offense_boost: Some(OffenseBoost {
            stats: OffenseStats::Both,
            boost: AbilityBoost {
                when: BoostWhen::MoveType(move_type),
                modifier,
                priority: 5,
            },
        }),
        ..AbilityBehavior::default()
    }
}

/// Blaze, Overgrow, Torrent, Swarm: ×1.5 to moves of their type at a third of max HP or below
/// — Gen 3-4 on the base power (the gen4 mod's `onBasePower`, priority 2), Gen 5+ on the
/// attacking stat (priority 5); each generation's snapshot keeps the one it declares.
fn pinch(move_type: crate::dex::PokemonType) -> AbilityBehavior {
    let boost = |priority| AbilityBoost {
        when: BoostWhen::MoveTypeInAPinch(move_type),
        modifier: BoostModifier::Fixed(6144),
        priority,
    };
    AbilityBehavior {
        base_power_boost: Some(boost(2)),
        offense_boost: Some(OffenseBoost {
            stats: OffenseStats::Both,
            boost: boost(5),
        }),
        ..AbilityBehavior::default()
    }
}

/// An ability whose only effect edits the holder's moves (`onModifyMove`).
fn move_edit(edit: MoveEdit) -> AbilityBehavior {
    AbilityBehavior {
        move_edit: Some(edit),
        ..AbilityBehavior::default()
    }
}

/// Plus / Minus: ×1.5 Special Attack beside a Pokémon with `other` (`DamageRules::plus_minus`).
fn plus_minus(other: AbilityId) -> AbilityBehavior {
    AbilityBehavior {
        offense_boost: Some(OffenseBoost {
            stats: OffenseStats::SpecialAttack,
            boost: AbilityBoost {
                when: BoostWhen::PlusMinus { other },
                modifier: BoostModifier::Fixed(6144),
                priority: 5,
            },
        }),
        ..AbilityBehavior::default()
    }
}

/// A Ruin ability: `stat` of every Pokémon without it is ×0.75 in a damage calculation.
fn ruin(stat: crate::state::Stat) -> AbilityBehavior {
    AbilityBehavior {
        switch_in: Some(entry_effects::announce),
        ruin: Some(stat),
        ..AbilityBehavior::default()
    }
}

/// An -ate ability: Normal-type moves become `to`, boosted.
fn type_changer(to: crate::dex::PokemonType) -> AbilityBehavior {
    AbilityBehavior {
        modify_type: Some(TypeChange {
            scope: TypeChangeScope::NormalMoves,
            to,
        }),
        type_change_boost: true,
        ..AbilityBehavior::default()
    }
}

/// One lookup per ability yields its typed behavior. As the Dex grows this mapping should be
/// generated from effect metadata rather than maintained manually.
pub fn definition(ability: AbilityId) -> AbilityBehavior {
    match ability {
        AbilityId::AERILATE => type_changer(crate::dex::PokemonType::Flying),
        AbilityId::GALVANIZE => type_changer(crate::dex::PokemonType::Electric),
        AbilityId::PIXILATE => type_changer(crate::dex::PokemonType::Fairy),
        AbilityId::REFRIGERATE => type_changer(crate::dex::PokemonType::Ice),
        AbilityId::LIQUID_VOICE => AbilityBehavior {
            modify_type: Some(TypeChange {
                scope: TypeChangeScope::SoundMoves,
                to: crate::dex::PokemonType::Water,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::NORMALIZE => AbilityBehavior {
            modify_type: Some(TypeChange {
                scope: TypeChangeScope::AllMoves,
                to: crate::dex::PokemonType::Normal,
            }),
            type_change_boost: true,
            ..AbilityBehavior::default()
        },
        AbilityId::ILLUSION => AbilityBehavior {
            illusion: true,
            ..unordered_reaction(illusion::damaging_hit)
        },
        AbilityId::IMPOSTER => AbilityBehavior {
            entry: Some(imposter::switch_in),
            ..AbilityBehavior::default()
        },
        AbilityId::INTIMIDATE => AbilityBehavior {
            switch_in: Some(intimidate::switch_in),
            announces: Some(crate::state::SwitchInAnnouncement::Intimidate),
            ..AbilityBehavior::default()
        },
        AbilityId::SCREEN_CLEANER => AbilityBehavior {
            switch_in: Some(screen_cleaner::switch_in),
            ..AbilityBehavior::default()
        },
        AbilityId::LIMBER => cures(&[NonVolatileStatus::Paralysis]),
        AbilityId::IMMUNITY => cures(&[NonVolatileStatus::Poison, NonVolatileStatus::Toxic]),
        AbilityId::WATER_VEIL => cures(&[NonVolatileStatus::Burn]),
        AbilityId::INSOMNIA | AbilityId::VITAL_SPIRIT => AbilityBehavior {
            blocks_volatiles: &[crate::state::VolatileConditionKind::Yawn],
            ..cures(&[NonVolatileStatus::Sleep])
        },
        AbilityId::MAGMA_ARMOR => AbilityBehavior {
            status_immunity: Some(StatusImmunity::Status(NonVolatileStatus::Freeze)),
            update: Some(AbilityUpdate::CureStatus(&[NonVolatileStatus::Freeze])),
            ..AbilityBehavior::default()
        },
        // Water moves at twice the attacking stat (`onModifyAtk` / `SpA`, no priority), Fire
        // moves against it at half (`onSourceModifyAtk` / `SpA`, priority 5).
        AbilityId::WATER_BUBBLE => AbilityBehavior {
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::Both,
                boost: AbilityBoost {
                    when: BoostWhen::MoveType(crate::dex::PokemonType::Water),
                    modifier: BoostModifier::Fixed(8192),
                    priority: 0,
                },
            }),
            foe_offense: Some(AbilityBoost {
                when: BoostWhen::MoveType(crate::dex::PokemonType::Fire),
                modifier: BoostModifier::Fixed(2048),
                priority: 5,
            }),
            ..cures(&[NonVolatileStatus::Burn])
        },
        AbilityId::THERMAL_EXCHANGE => AbilityBehavior {
            damaging_hit_reaction: unordered_reaction(status_immunity::thermal_exchange)
                .damaging_hit_reaction,
            ..cures(&[NonVolatileStatus::Burn])
        },
        AbilityId::TERAFORM_ZERO => AbilityBehavior {
            after_terastallization: Some(teraform_zero),
            ..AbilityBehavior::default()
        },
        AbilityId::OPPORTUNIST => AbilityBehavior {
            foe_after_boost: Some(opportunist::keep_a_foes_raise),
            any_switch_in: Some((-3, opportunist::copy_kept_raises)),
            any_after_move: Some(opportunist::copy_kept_raises),
            any_after_terastallization: Some(opportunist::copy_kept_raises),
            any_after_mega: Some(opportunist::copy_kept_raises),
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::Opportunist,
                handler: opportunist::copy_kept_raises,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::PASTEL_VEIL => AbilityBehavior {
            switch_in: Some(status_immunity::pastel_veil_start),
            any_switch_in: Some((0, status_immunity::pastel_veil_start)),
            ally_try_set_status: Some(status_immunity::pastel_veil_ally),
            ..cures(&[NonVolatileStatus::Poison, NonVolatileStatus::Toxic])
        },
        AbilityId::OBLIVIOUS => AbilityBehavior {
            status_immunity: Some(StatusImmunity::Volatile(
                crate::state::VolatileConditionKind::Attract,
            )),
            try_hit: Some(status_immunity::oblivious_try_hit),
            try_stat_change: Some(drop_guard::oblivious),
            update: Some(AbilityUpdate::EndVolatiles(&[
                crate::state::VolatileConditionKind::Attract,
                crate::state::VolatileConditionKind::Taunt,
            ])),
            ..AbilityBehavior::default()
        },
        AbilityId::OWN_TEMPO => AbilityBehavior {
            blocks_volatiles: &[crate::state::VolatileConditionKind::Confusion],
            announces_blocked_confusion: true,
            try_stat_change: Some(drop_guard::own_tempo),
            update: Some(AbilityUpdate::EndVolatiles(&[
                crate::state::VolatileConditionKind::Confusion,
            ])),
            ..AbilityBehavior::default()
        },
        AbilityId::BLAZE => pinch(crate::dex::PokemonType::Fire),
        AbilityId::OVERGROW => pinch(crate::dex::PokemonType::Grass),
        AbilityId::TORRENT => pinch(crate::dex::PokemonType::Water),
        AbilityId::SWARM => pinch(crate::dex::PokemonType::Bug),
        AbilityId::IRON_FIST => power(BoostWhen::Flag(MoveFlag::Punch), 4915, 23),
        AbilityId::RECKLESS => power(BoostWhen::Recoil, 4915, 23),
        AbilityId::TOUGH_CLAWS => power(BoostWhen::Flag(MoveFlag::Contact), 5325, 21),
        AbilityId::STRONG_JAW => power(BoostWhen::Flag(MoveFlag::Bite), 6144, 19),
        AbilityId::MEGA_LAUNCHER => power(BoostWhen::Flag(MoveFlag::Pulse), 6144, 19),
        AbilityId::SHARPNESS => power(BoostWhen::Flag(MoveFlag::Slicing), 6144, 19),
        AbilityId::STEELWORKER => {
            offense(crate::dex::PokemonType::Steel, BoostModifier::Fixed(6144))
        }
        AbilityId::TRANSISTOR => {
            offense(crate::dex::PokemonType::Electric, BoostModifier::Transistor)
        }
        AbilityId::DRAGONS_MAW => {
            offense(crate::dex::PokemonType::Dragon, BoostModifier::Fixed(6144))
        }
        AbilityId::ROCKY_PAYLOAD => {
            offense(crate::dex::PokemonType::Rock, BoostModifier::Fixed(6144))
        }
        AbilityId::COMPOUND_EYES => AbilityBehavior {
            accuracy_boost: Some(AccuracyBoost {
                when: AccuracyWhen::Always,
                modifier: AccuracyModifier::new(5325, 130, AccuracyStep::CompoundEyes),
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::HUSTLE => AbilityBehavior {
            modify_offense: Some(OrderedHandler::new(
                5,
                hustle_offense as ModifyOffenseHandler,
            )),
            offense_on_stat: true,
            accuracy_boost: Some(AccuracyBoost {
                when: AccuracyWhen::Physical,
                modifier: AccuracyModifier::new(3277, 80, AccuracyStep::Hustle),
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::SAND_VEIL => evasive(crate::state::WeatherKind::Sandstorm),
        AbilityId::SNOW_CLOAK => evasive(crate::state::WeatherKind::Snow),
        AbilityId::TANGLED_FEET => AbilityBehavior {
            evasion_boost: Some(AccuracyBoost {
                when: AccuracyWhen::HolderConfused,
                modifier: AccuracyModifier::new(2048, 50, AccuracyStep::TangledFeet),
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::VICTORY_STAR => AbilityBehavior {
            ally_accuracy_boost: Some(4506),
            ..AbilityBehavior::default()
        },
        AbilityId::WONDER_SKIN => AbilityBehavior {
            status_accuracy: Some(50),
            ..AbilityBehavior::default()
        },
        AbilityId::NO_GUARD => AbilityBehavior {
            no_guard: true,
            ..AbilityBehavior::default()
        },
        AbilityId::FILTER | AbilityId::SOLID_ROCK | AbilityId::PRISM_ARMOR => AbilityBehavior {
            damage_taken: Some(DamageTaken::SuperEffective),
            ..AbilityBehavior::default()
        },
        AbilityId::MULTISCALE | AbilityId::SHADOW_SHIELD => AbilityBehavior {
            damage_taken: Some(DamageTaken::FullHp),
            ..AbilityBehavior::default()
        },
        AbilityId::ICE_SCALES => AbilityBehavior {
            damage_taken: Some(DamageTaken::Special),
            ..AbilityBehavior::default()
        },
        AbilityId::FLUFFY => AbilityBehavior {
            damage_taken: Some(DamageTaken::Fluffy),
            ..AbilityBehavior::default()
        },
        AbilityId::PUNK_ROCK => AbilityBehavior {
            damage_taken: Some(DamageTaken::Sound),
            ..power(BoostWhen::Flag(MoveFlag::Sound), 5325, 7)
        },
        AbilityId::THICK_FAT => {
            // Gen 3-4 on the power (the gen4 mod's `onSourceBasePower`), Gen 5+ on the
            // attacking stat; each generation's snapshot keeps the one it declares.
            let halved = |priority| AbilityBoost {
                when: BoostWhen::MoveTypes(&[
                    crate::dex::PokemonType::Fire,
                    crate::dex::PokemonType::Ice,
                ]),
                modifier: BoostModifier::Fixed(2048),
                priority,
            };
            AbilityBehavior {
                foe_base_power: Some(halved(1)),
                foe_offense: Some(halved(6)),
                ..AbilityBehavior::default()
            }
        }
        AbilityId::HEATPROOF => {
            // Gen 4-8 on the power (the gen8 mod's `onSourceBasePower`), Gen 9 on the attacking
            // stat; each generation's snapshot keeps the one it declares.
            let halved = |priority| AbilityBoost {
                when: BoostWhen::MoveType(crate::dex::PokemonType::Fire),
                modifier: BoostModifier::Fixed(2048),
                priority,
            };
            AbilityBehavior {
                foe_base_power: Some(halved(18)),
                foe_offense: Some(halved(6)),
                halves_status_damage: Some(NonVolatileStatus::Burn),
                ..AbilityBehavior::default()
            }
        }
        AbilityId::MARVEL_SCALE => AbilityBehavior {
            defense_boost: Some(AbilityBoost {
                when: BoostWhen::HolderStatused,
                modifier: BoostModifier::Fixed(6144),
                priority: 6,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::FUR_COAT => AbilityBehavior {
            defense_boost: Some(AbilityBoost {
                when: BoostWhen::Always,
                modifier: BoostModifier::Fixed(8192),
                priority: 6,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::GUTS => AbilityBehavior {
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::Attack,
                boost: AbilityBoost {
                    when: BoostWhen::HolderStatused,
                    modifier: BoostModifier::Fixed(6144),
                    priority: 5,
                },
            }),
            ignores_burn_drop: true,
            ..AbilityBehavior::default()
        },
        AbilityId::DEFEATIST => AbilityBehavior {
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::Both,
                boost: AbilityBoost {
                    when: BoostWhen::HolderAtHalfHp,
                    modifier: BoostModifier::Fixed(2048),
                    priority: 5,
                },
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::TOXIC_BOOST => power(BoostWhen::PoisonedAndPhysical, 6144, 19),
        AbilityId::FLARE_BOOST => power(BoostWhen::BurnedAndSpecial, 6144, 19),
        AbilityId::SAND_FORCE => AbilityBehavior {
            base_power_boost: Some(AbilityBoost {
                when: BoostWhen::WeatherAndTypes(
                    crate::state::WeatherKind::Sandstorm,
                    &[
                        crate::dex::PokemonType::Rock,
                        crate::dex::PokemonType::Ground,
                        crate::dex::PokemonType::Steel,
                    ],
                ),
                modifier: BoostModifier::Fixed(5325),
                priority: 21,
            }),
            weather_immunity: Some(crate::state::WeatherKind::Sandstorm),
            ..AbilityBehavior::default()
        },
        AbilityId::ANALYTIC => power(BoostWhen::MovesLast, 5325, 21),
        AbilityId::SNIPER => AbilityBehavior {
            damage_boost: Some(DamageBoost {
                when: DamageBoostWhen::Critical,
                modifier: 6144,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::TINTED_LENS => AbilityBehavior {
            damage_boost: Some(DamageBoost {
                when: DamageBoostWhen::NotVeryEffective,
                modifier: 8192,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::ADAPTABILITY => AbilityBehavior {
            stab_modifier: Some(crate::rules::Fraction::new(2, 1)),
            ..AbilityBehavior::default()
        },
        AbilityId::HUGE_POWER | AbilityId::PURE_POWER => AbilityBehavior {
            modify_offense: Some(OrderedHandler::new(
                5,
                huge_power::modify_offense as ModifyOffenseHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::TECHNICIAN => AbilityBehavior {
            base_power: Some(OrderedHandler::new(
                30,
                technician::base_power as BasePowerChainHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::WIND_POWER => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                1,
                wind::wind_power as DamagingHitReactionHandler,
            )),
            side_condition_start: Some(wind::wind_power_tailwind),
            ..AbilityBehavior::default()
        },
        AbilityId::WIND_RIDER => AbilityBehavior {
            switch_in: Some(wind::wind_rider_entry),
            try_hit: Some(wind::wind_rider),
            absorb_reaction: Some(wind::wind_rider_absorb),
            side_condition_start: Some(wind::wind_rider_tailwind),
            ..AbilityBehavior::default()
        },
        AbilityId::SAP_SIPPER => AbilityBehavior {
            try_hit: Some(hit_boosts::sap_sipper),
            absorb_reaction: Some(hit_boosts::sap_sipper_absorb),
            ally_try_hit_side: Some(hit_boosts::sap_sipper_ally),
            ..AbilityBehavior::default()
        },
        AbilityId::ELECTROMORPHOSIS => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                1,
                charge_abilities::electromorphosis as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::ROUGH_SKIN => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                1,
                rough_skin::rough_skin as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::IRON_BARBS => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                1,
                rough_skin::iron_barbs as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::STURDY => AbilityBehavior {
            damage_resolution: Some(OrderedHandler::new(
                30,
                sturdy::damage_resolution as DamageResolutionHandler,
            )),
            ohko_immune: true,
            ..AbilityBehavior::default()
        },
        AbilityId::SUPER_LUCK => AbilityBehavior {
            crit_ratio_boost: 1,
            ..AbilityBehavior::default()
        },
        AbilityId::BATTLE_ARMOR | AbilityId::SHELL_ARMOR => AbilityBehavior {
            blocks_critical_hit: true,
            ..AbilityBehavior::default()
        },
        AbilityId::MAGIC_GUARD => AbilityBehavior {
            damage_resolution: Some(OrderedHandler::new(
                0,
                magic_guard::damage_resolution as DamageResolutionHandler,
            )),
            try_set_status: Some(magic_guard::try_set_status),
            ..AbilityBehavior::default()
        },
        // `onDamagePriority` 1.
        AbilityId::POISON_HEAL => AbilityBehavior {
            damage_resolution: Some(OrderedHandler::new(
                -1,
                poison_heal::damage_resolution as DamageResolutionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::PICKUP => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::Pickup,
                handler: pickup_residual as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::MOODY => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::Moody,
                handler: moody::residual as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::BAD_DREAMS => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::BadDreams,
                handler: bad_dreams::residual as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::QUICK_FEET => AbilityBehavior {
            speed_boost: Some(SpeedBoost {
                when: SpeedWhen::Status,
                modifier: 6144,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::CLEAR_BODY => AbilityBehavior {
            try_stat_change: Some(clear_body::try_stat_change),
            ..AbilityBehavior::default()
        },
        AbilityId::WHITE_SMOKE => AbilityBehavior {
            try_stat_change: Some(drop_guard::white_smoke),
            ..AbilityBehavior::default()
        },
        AbilityId::FULL_METAL_BODY => AbilityBehavior {
            try_stat_change: Some(drop_guard::full_metal_body),
            ..AbilityBehavior::default()
        },
        AbilityId::HYPER_CUTTER => AbilityBehavior {
            try_stat_change: Some(drop_guard::hyper_cutter),
            ..AbilityBehavior::default()
        },
        AbilityId::BIG_PECKS => AbilityBehavior {
            try_stat_change: Some(drop_guard::big_pecks),
            ..AbilityBehavior::default()
        },
        AbilityId::MIRROR_ARMOR => AbilityBehavior {
            reflects_stat_drops: true,
            ..AbilityBehavior::default()
        },
        // Gen 3-7 declare only `onTryAddVolatile`: the Intimidate guard is Gen 8+.
        AbilityId::INNER_FOCUS => AbilityBehavior {
            blocks_volatiles: &[crate::state::VolatileConditionKind::Flinch],
            try_stat_change: Some(drop_guard::inner_focus),
            ..AbilityBehavior::default()
        },
        AbilityId::STEADFAST => AbilityBehavior {
            flinch_boost: Some(crate::state::Stat::Speed),
            ..AbilityBehavior::default()
        },
        AbilityId::SHIELD_DUST => AbilityBehavior {
            shields_secondaries: true,
            ..AbilityBehavior::default()
        },
        AbilityId::EFFECT_SPORE => AbilityBehavior {
            hit_reaction: Some(HitReaction::EffectSpore),
            ..AbilityBehavior::default()
        },
        AbilityId::CURSED_BODY => AbilityBehavior {
            hit_reaction: Some(HitReaction::CursedBody),
            ..AbilityBehavior::default()
        },
        AbilityId::CUTE_CHARM => AbilityBehavior {
            hit_reaction: Some(HitReaction::CuteCharm),
            ..AbilityBehavior::default()
        },
        AbilityId::RIVALRY => AbilityBehavior {
            base_power_boost: Some(AbilityBoost {
                when: BoostWhen::BothGendered,
                modifier: BoostModifier::Rivalry,
                priority: 24,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::POISON_TOUCH => AbilityBehavior {
            dealt_hit_status: Some(DealtHitStatus {
                status: NonVolatileStatus::Poison,
                contact_only: true,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::TOXIC_CHAIN => AbilityBehavior {
            dealt_hit_status: Some(DealtHitStatus {
                status: NonVolatileStatus::Toxic,
                contact_only: false,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::GOOEY | AbilityId::TANGLING_HAIR => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                UNORDERED_REACTION,
                hit_reactions::slow_the_attacker as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::COTTON_DOWN => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                UNORDERED_REACTION,
                hit_reactions::cotton_down as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::CONTRARY => AbilityBehavior {
            modify_stat_change: Some(contrary::contrary),
            ..AbilityBehavior::default()
        },
        // Gen 4 declares `onModifyBoost` (the gen4 mod), later generations `onChangeBoost`.
        AbilityId::SIMPLE => AbilityBehavior {
            modify_stat_change: Some(contrary::simple),
            doubles_read_stages: true,
            ..AbilityBehavior::default()
        },
        AbilityId::LIGHTNING_ROD => AbilityBehavior {
            try_hit: Some(lightning_rod::lightning_rod_try_hit),
            absorb_reaction: Some(lightning_rod::absorb),
            redirect_type: Some(crate::dex::PokemonType::Electric),
            ..AbilityBehavior::default()
        },
        AbilityId::STORM_DRAIN => AbilityBehavior {
            try_hit: Some(lightning_rod::storm_drain_try_hit),
            absorb_reaction: Some(lightning_rod::absorb),
            redirect_type: Some(crate::dex::PokemonType::Water),
            ..AbilityBehavior::default()
        },
        AbilityId::DROUGHT => AbilityBehavior {
            switch_in: Some(weather::drought),
            announces: Some(crate::state::SwitchInAnnouncement::Weather),
            ..AbilityBehavior::default()
        },
        AbilityId::DRIZZLE => AbilityBehavior {
            switch_in: Some(weather::drizzle),
            announces: Some(crate::state::SwitchInAnnouncement::Weather),
            ..AbilityBehavior::default()
        },
        AbilityId::SAND_STREAM => AbilityBehavior {
            switch_in: Some(weather::sand_stream),
            announces: Some(crate::state::SwitchInAnnouncement::Weather),
            ..AbilityBehavior::default()
        },
        AbilityId::SNOW_WARNING => AbilityBehavior {
            switch_in: Some(weather::snow_warning),
            announces: Some(crate::state::SwitchInAnnouncement::Weather),
            ..AbilityBehavior::default()
        },
        AbilityId::STATIC => AbilityBehavior {
            contact_status: Some(NonVolatileStatus::Paralysis),
            ..AbilityBehavior::default()
        },
        AbilityId::FLAME_BODY => AbilityBehavior {
            contact_status: Some(NonVolatileStatus::Burn),
            ..AbilityBehavior::default()
        },
        AbilityId::POISON_POINT => AbilityBehavior {
            contact_status: Some(NonVolatileStatus::Poison),
            ..AbilityBehavior::default()
        },
        AbilityId::SYNCHRONIZE => AbilityBehavior {
            after_set_status: Some(synchronize::after_set_status),
            ..AbilityBehavior::default()
        },
        AbilityId::SHED_SKIN => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::ShedSkin,
                handler: shed_skin::residual as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::FLOWER_GIFT => AbilityBehavior {
            switch_in: Some(weather::flower_gift),
            switch_in_priority: -2,
            flower_gift: true,
            ..AbilityBehavior::default()
        },
        AbilityId::FORECAST => AbilityBehavior {
            switch_in: Some(weather::forecast),
            forecast: Some(Forecast {
                holder: crate::ids::SpeciesId::CASTFORM,
                sun: crate::ids::SpeciesId::CASTFORM_SUNNY,
                rain: crate::ids::SpeciesId::CASTFORM_RAINY,
                snow: crate::ids::SpeciesId::CASTFORM_SNOWY,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::STANCE_CHANGE => {
            let stance = Some(StanceChange {
                attack: crate::ids::SpeciesId::AEGISLASH_BLADE,
                shield: crate::ids::SpeciesId::AEGISLASH,
                shield_move: crate::ids::MoveId::KINGS_SHIELD,
            });
            AbilityBehavior {
                before_move_stance: stance,
                move_stance: stance,
                ..AbilityBehavior::default()
            }
        }
        AbilityId::ZEN_MODE => AbilityBehavior {
            residual: Some(forme_residual()),
            self_forme: Some(zen_mode()),
            ..AbilityBehavior::default()
        },
        AbilityId::SCHOOLING => AbilityBehavior {
            switch_in: Some(forme::enter),
            // `onSwitchInPriority` −1.
            switch_in_priority: -1,
            residual: Some(forme_residual()),
            self_forme: Some(SelfForme {
                holder: crate::ids::SpeciesId::WISHIWASHI,
                rule: SelfFormeRule::Threshold {
                    at: HpDenominator::Quarter,
                    high: crate::ids::SpeciesId::WISHIWASHI_SCHOOL,
                    min_level: 20,
                },
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::SHIELDS_DOWN => AbilityBehavior {
            switch_in: Some(forme::enter),
            // `onSwitchInPriority` −1.
            switch_in_priority: -1,
            residual: Some(forme_residual()),
            self_forme: Some(SelfForme {
                holder: crate::ids::SpeciesId::MINIOR,
                rule: SelfFormeRule::Threshold {
                    at: HpDenominator::Half,
                    high: crate::ids::SpeciesId::MINIOR_METEOR,
                    min_level: 1,
                },
            }),
            // Its Meteor forme takes no status and no Yawn.
            in_forme_blocks: Some(FormeGuard {
                forme: crate::ids::SpeciesId::MINIOR_METEOR,
                statuses: &[
                    NonVolatileStatus::Burn,
                    NonVolatileStatus::Paralysis,
                    NonVolatileStatus::Poison,
                    NonVolatileStatus::Toxic,
                    NonVolatileStatus::Sleep,
                    NonVolatileStatus::Freeze,
                ],
                volatiles: &[crate::state::VolatileConditionKind::Yawn],
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::HUNGER_SWITCH => AbilityBehavior {
            residual: Some(forme_residual()),
            self_forme: Some(SelfForme {
                holder: crate::ids::SpeciesId::MORPEKO,
                rule: SelfFormeRule::Toggle {
                    other: crate::ids::SpeciesId::MORPEKO_HANGRY,
                },
            }),
            ..AbilityBehavior::default()
        },
        // Tera Shift (DESIGN.md §44.6): the switch-in change into Terapagos's Terastal forme;
        // `onSwitchInPriority: 2`.
        AbilityId::TERA_SHIFT => AbilityBehavior {
            switch_in: Some(tera::tera_shift),
            switch_in_priority: 2,
            ..AbilityBehavior::default()
        },
        // Embody Aspect, the four `-Tera` formes' own: the stat its `onStart` raises once.
        AbilityId::EMBODY_ASPECT_TEAL => AbilityBehavior {
            switch_in: Some(embody_aspect_teal),
            ..AbilityBehavior::default()
        },
        AbilityId::EMBODY_ASPECT_WELLSPRING => AbilityBehavior {
            switch_in: Some(embody_aspect_wellspring),
            ..AbilityBehavior::default()
        },
        AbilityId::EMBODY_ASPECT_HEARTHFLAME => AbilityBehavior {
            switch_in: Some(embody_aspect_hearthflame),
            ..AbilityBehavior::default()
        },
        AbilityId::EMBODY_ASPECT_CORNERSTONE => AbilityBehavior {
            switch_in: Some(embody_aspect_cornerstone),
            ..AbilityBehavior::default()
        },
        AbilityId::POWER_CONSTRUCT => AbilityBehavior {
            residual: Some(forme_residual()),
            self_forme: Some(SelfForme {
                holder: crate::ids::SpeciesId::ZYGARDE,
                rule: SelfFormeRule::PowerConstruct {
                    at: HpDenominator::Half,
                    other: crate::ids::SpeciesId::ZYGARDE_COMPLETE,
                },
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::DISGUISE => AbilityBehavior {
            absorbing: Some(disguise()),
            update: Some(AbilityUpdate::BustedForme(disguise())),
            damage_resolution: Some(OrderedHandler {
                // `onDamagePriority` 1: before Sturdy's −30 and Focus Sash's 40.
                priority: 1,
                handler: forme_ability::absorb_damage,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::ICE_FACE => AbilityBehavior {
            absorbing: Some(ice_face()),
            update: Some(AbilityUpdate::BustedForme(ice_face())),
            switch_in: Some(forme_ability::ice_face_weather),
            // `onSwitchInPriority` −2.
            switch_in_priority: -2,
            weather_change: Some(forme_ability::ice_face_weather),
            damage_resolution: Some(OrderedHandler {
                priority: 1,
                handler: forme_ability::absorb_damage,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::GULP_MISSILE => AbilityBehavior {
            gulp_missile: true,
            damaging_hit_reaction: Some(OrderedHandler {
                priority: 0,
                handler: forme_ability::gulp_missile_spit,
            }),
            source_try_primary_hit: Some(forme_ability::gulp_missile_surf),
            ..AbilityBehavior::default()
        },
        AbilityId::BATTLE_BOND => AbilityBehavior {
            battle_bond: true,
            source_after_faint: Some(forme_ability::battle_bond),
            ..AbilityBehavior::default()
        },
        // `onSwitchIn`, not `onStart`: the Hero forme taken as it leaves starts the ability
        // anew, unannounced (R-M10-40).
        AbilityId::ZERO_TO_HERO => AbilityBehavior {
            zero_to_hero: true,
            entry: Some(forme_ability::zero_to_hero_in),
            switch_out: Some(forme_ability::zero_to_hero_out),
            ..AbilityBehavior::default()
        },
        AbilityId::COMMANDER => AbilityBehavior {
            commander: true,
            update: Some(AbilityUpdate::Commander),
            switch_in: Some(forme_ability::commander),
            // `onAnySwitchInPriority` −2.
            any_switch_in: Some((-2, forme_ability::commander)),
            ..AbilityBehavior::default()
        },
        AbilityId::MIMICRY => AbilityBehavior {
            switch_in: Some(terrain::mimicry),
            mimicry: true,
            ..AbilityBehavior::default()
        },
        AbilityId::DESOLATE_LAND => strong_weather(crate::state::WeatherKind::Sun),
        AbilityId::PRIMORDIAL_SEA => strong_weather(crate::state::WeatherKind::Rain),
        AbilityId::DELTA_STREAM => strong_weather(crate::state::WeatherKind::StrongWinds),
        AbilityId::AIR_LOCK | AbilityId::CLOUD_NINE => AbilityBehavior {
            switch_in: Some(weather::suppress_weather),
            entry: Some(entry_effects::announce),
            suppresses_weather: true,
            ..AbilityBehavior::default()
        },
        AbilityId::CHLOROPHYLL => speedy(crate::state::WeatherKind::Sun),
        AbilityId::SWIFT_SWIM => speedy(crate::state::WeatherKind::Rain),
        AbilityId::SLUSH_RUSH => speedy(crate::state::WeatherKind::Snow),
        AbilityId::SAND_RUSH => AbilityBehavior {
            weather_immunity: Some(crate::state::WeatherKind::Sandstorm),
            ..speedy(crate::state::WeatherKind::Sandstorm)
        },
        AbilityId::DRY_SKIN => AbilityBehavior {
            try_hit: Some(absorb::dry_skin),
            absorb_reaction: Some(absorb::heal_quarter),
            foe_base_power: Some(AbilityBoost {
                when: BoostWhen::MoveTypes(&[crate::dex::PokemonType::Fire]),
                modifier: BoostModifier::Fixed(5120),
                priority: 17,
            }),
            weather_reactions: &[
                WeatherReaction {
                    weather: crate::state::WeatherKind::Rain,
                    effect: WeatherEffect::Heal(8),
                },
                WeatherReaction {
                    weather: crate::state::WeatherKind::Sun,
                    effect: WeatherEffect::Damage(8),
                },
            ],
            ..AbilityBehavior::default()
        },
        // Gen 3: a residual (10.3); Gen 4+: an `onWeather`.
        AbilityId::RAIN_DISH => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::RainDish,
                handler: weather::rain_dish as ResidualHandler,
            }),
            weather_reactions: &[WeatherReaction {
                weather: crate::state::WeatherKind::Rain,
                effect: WeatherEffect::Heal(16),
            }],
            ..AbilityBehavior::default()
        },
        AbilityId::ICE_BODY => AbilityBehavior {
            weather_immunity: Some(crate::state::WeatherKind::Snow),
            weather_reactions: &[WeatherReaction {
                weather: crate::state::WeatherKind::Snow,
                effect: WeatherEffect::Heal(16),
            }],
            ..AbilityBehavior::default()
        },
        // Gen 4: an `onWeather`; Gen 5+: a residual (5.3).
        AbilityId::HYDRATION => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::Hydration,
                handler: weather::hydration as ResidualHandler,
            }),
            weather_reactions: &[WeatherReaction {
                weather: crate::state::WeatherKind::Rain,
                effect: WeatherEffect::CureStatus,
            }],
            ..AbilityBehavior::default()
        },
        AbilityId::SOLAR_POWER => AbilityBehavior {
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::SpecialAttack,
                boost: AbilityBoost {
                    when: BoostWhen::Weather(crate::state::WeatherKind::Sun),
                    modifier: BoostModifier::Fixed(6144),
                    priority: 5,
                },
            }),
            weather_reactions: &[WeatherReaction {
                weather: crate::state::WeatherKind::Sun,
                effect: WeatherEffect::Damage(8),
            }],
            ..AbilityBehavior::default()
        },
        AbilityId::PROTOSYNTHESIS => AbilityBehavior {
            switch_in: Some(weather::protosynthesis),
            switch_in_priority: -2,
            protosynthesis: true,
            ..AbilityBehavior::default()
        },
        // Showdown gives `onDamagingHit` no order: after every ordered reaction.
        AbilityId::DAMP => AbilityBehavior {
            damp: true,
            ..AbilityBehavior::default()
        },
        AbilityId::SHADOW_TAG => AbilityBehavior {
            traps_foes: Some(TrapFilter::Anyone),
            ..AbilityBehavior::default()
        },
        AbilityId::ARENA_TRAP => AbilityBehavior {
            traps_foes: Some(TrapFilter::Grounded),
            ..AbilityBehavior::default()
        },
        // Gen 3 declares `onAny…` (allies too), later generations `onFoe…`: the generation's
        // data keeps one of the two.
        AbilityId::MAGNET_PULL => AbilityBehavior {
            traps_foes: Some(TrapFilter::SteelType),
            traps_any: Some(TrapFilter::SteelType),
            ..AbilityBehavior::default()
        },
        AbilityId::AFTERMATH => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                1,
                self_faint::aftermath as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::MUMMY => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                i16::MAX,
                ability_transfer::mummy as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::LINGERING_AROMA => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                i16::MAX,
                ability_transfer::lingering_aroma as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::WANDERING_SPIRIT => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                i16::MAX,
                ability_transfer::wandering_spirit as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::TRUANT => AbilityBehavior {
            truant: true,
            switch_in: Some(truant::switch_in),
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::Truant,
                handler: truant::residual as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::TRACE => AbilityBehavior {
            trace: true,
            announces: Some(crate::state::SwitchInAnnouncement::Trace),
            ..AbilityBehavior::default()
        },
        AbilityId::RECEIVER | AbilityId::POWER_OF_ALCHEMY => AbilityBehavior {
            ally_faint: Some(ability_transfer::receiver),
            ..AbilityBehavior::default()
        },
        AbilityId::PERISH_BODY => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                i16::MAX,
                self_faint::perish_body as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::INNARDS_OUT => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                1,
                self_faint::innards_out as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::SAND_SPIT => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                i16::MAX,
                weather::sand_spit as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::TOXIC_DEBRIS => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                i16::MAX,
                toxic_debris::damaging_hit as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::GOOD_AS_GOLD => AbilityBehavior {
            try_hit: Some(status_tricks::good_as_gold),
            ..AbilityBehavior::default()
        },
        AbilityId::CURIOUS_MEDICINE => AbilityBehavior {
            switch_in: Some(status_tricks::curious_medicine),
            ..AbilityBehavior::default()
        },
        AbilityId::POISON_PUPPETEER => AbilityBehavior {
            any_after_set_status: Some(status_tricks::poison_puppeteer),
            ..AbilityBehavior::default()
        },
        AbilityId::HOSPITALITY => AbilityBehavior {
            switch_in: Some(entry_effects::hospitality),
            switch_in_priority: -2,
            ..AbilityBehavior::default()
        },
        AbilityId::COSTAR => AbilityBehavior {
            switch_in: Some(entry_effects::costar),
            switch_in_priority: -2,
            ..AbilityBehavior::default()
        },
        AbilityId::SUPERSWEET_SYRUP => AbilityBehavior {
            switch_in: Some(entry_effects::supersweet_syrup),
            ..AbilityBehavior::default()
        },
        AbilityId::DOWNLOAD => AbilityBehavior {
            switch_in: Some(stage_reactions::download),
            announces: Some(crate::state::SwitchInAnnouncement::Download),
            ..AbilityBehavior::default()
        },
        AbilityId::INTREPID_SWORD => AbilityBehavior {
            switch_in: Some(stage_reactions::intrepid_sword),
            ..AbilityBehavior::default()
        },
        AbilityId::DAUNTLESS_SHIELD => AbilityBehavior {
            switch_in: Some(stage_reactions::dauntless_shield),
            ..AbilityBehavior::default()
        },
        AbilityId::DEFIANT => AbilityBehavior {
            after_each_boost: Some(stage_reactions::defiant),
            ..AbilityBehavior::default()
        },
        AbilityId::COMPETITIVE => AbilityBehavior {
            after_each_boost: Some(stage_reactions::competitive),
            ..AbilityBehavior::default()
        },
        AbilityId::MOXIE | AbilityId::CHILLING_NEIGH => AbilityBehavior {
            source_after_faint: Some(faint_boosts::moxie),
            ..AbilityBehavior::default()
        },
        AbilityId::GRIM_NEIGH => AbilityBehavior {
            source_after_faint: Some(faint_boosts::grim_neigh),
            ..AbilityBehavior::default()
        },
        AbilityId::GLUTTONY => AbilityBehavior {
            gluttony: true,
            ..AbilityBehavior::default()
        },
        AbilityId::RIPEN => AbilityBehavior {
            ripen: true,
            modify_stat_change: Some(ripen::change_boost),
            ..AbilityBehavior::default()
        },
        AbilityId::CHEEK_POUCH => AbilityBehavior {
            eaten_berry_heal: Some(crate::rules::Fraction::new(1, 3)),
            ..AbilityBehavior::default()
        },
        AbilityId::CUD_CHEW => AbilityBehavior {
            chews_the_cud: true,
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::CudChew,
                handler: cud_chew::residual as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::HARVEST => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::Harvest,
                handler: harvest::residual as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::PROTEAN | AbilityId::LIBERO => AbilityBehavior {
            retypes_before_hit: true,
            ..AbilityBehavior::default()
        },
        // Gen 5+ after the move (`onAfterMoveSecondary`), Gen 3-4 on each hit (`onDamagingHit`):
        // each generation keeps the component it declares.
        AbilityId::COLOR_CHANGE => AbilityBehavior {
            retypes_after_move: true,
            damaging_hit_reaction: Some(OrderedHandler::new(
                UNORDERED_REACTION,
                types::color_change_on_hit as crate::engine::DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::BERSERK => AbilityBehavior {
            half_hp_boosts: Some(berserk::BERSERK),
            ..AbilityBehavior::default()
        },
        AbilityId::ANGER_SHELL => AbilityBehavior {
            half_hp_boosts: Some(berserk::ANGER_SHELL),
            ..AbilityBehavior::default()
        },
        AbilityId::UNNERVE => AbilityBehavior {
            switch_in: Some(unnerve::switch_in),
            switch_in_priority: 1,
            unnerve: true,
            ..AbilityBehavior::default()
        },
        // Mold Breaker, Teravolt, Turboblaze: announced as the holder comes in; every move it uses
        // ignores the others' breakable abilities.
        AbilityId::MOLD_BREAKER | AbilityId::TERAVOLT | AbilityId::TURBOBLAZE => AbilityBehavior {
            switch_in: Some(mold_breaker::switch_in),
            move_edit: Some(MoveEdit {
                ignores_abilities: true,
                ..MoveEdit::default()
            }),
            announces: Some(crate::state::SwitchInAnnouncement::MoldBreaker),
            ..AbilityBehavior::default()
        },
        // Pressure: announced as the holder comes in (Gen 4's announced flag,
        // `SWITCH_IN_CHECK_STATE_PRESSURE`); a move that meets it costs one more PP.
        AbilityId::PRESSURE => AbilityBehavior {
            switch_in: Some(pressure::switch_in),
            announces: Some(crate::state::SwitchInAnnouncement::Pressure),
            pressure: true,
            ..AbilityBehavior::default()
        },
        AbilityId::NEUTRALIZING_GAS => AbilityBehavior {
            switch_in: Some(neutralizing_gas::switch_in),
            switch_in_priority: 2,
            neutralizing_gas: true,
            ..AbilityBehavior::default()
        },
        // As One: Unnerve, plus Chilling Neigh's / Grim Neigh's boost on a knockout.
        AbilityId::AS_ONE_GLASTRIER => AbilityBehavior {
            switch_in: Some(unnerve::as_one),
            switch_in_priority: 1,
            unnerve: true,
            source_after_faint: Some(faint_boosts::moxie),
            ..AbilityBehavior::default()
        },
        AbilityId::AS_ONE_SPECTRIER => AbilityBehavior {
            switch_in: Some(unnerve::as_one),
            switch_in_priority: 1,
            unnerve: true,
            source_after_faint: Some(faint_boosts::grim_neigh),
            ..AbilityBehavior::default()
        },
        AbilityId::BEAST_BOOST => AbilityBehavior {
            source_after_faint: Some(faint_boosts::beast_boost),
            ..AbilityBehavior::default()
        },
        AbilityId::SOUL_HEART => AbilityBehavior {
            any_faint: Some(faint_boosts::soul_heart),
            ..AbilityBehavior::default()
        },
        AbilityId::JUSTIFIED => unordered_reaction(hit_boosts::justified),
        AbilityId::RATTLED => AbilityBehavior {
            after_boost: Some(hit_boosts::rattled_after_boost),
            ..unordered_reaction(hit_boosts::rattled)
        },
        AbilityId::STAMINA => unordered_reaction(hit_boosts::stamina),
        AbilityId::STEAM_ENGINE => unordered_reaction(hit_boosts::steam_engine),
        AbilityId::WATER_COMPACTION => unordered_reaction(hit_boosts::water_compaction),
        AbilityId::WEAK_ARMOR => unordered_reaction(hit_boosts::weak_armor),
        AbilityId::MOTOR_DRIVE => AbilityBehavior {
            try_hit: Some(hit_boosts::motor_drive),
            absorb_reaction: Some(hit_boosts::motor_drive_absorb),
            ..AbilityBehavior::default()
        },
        AbilityId::WELL_BAKED_BODY => AbilityBehavior {
            try_hit: Some(hit_boosts::well_baked_body),
            absorb_reaction: Some(hit_boosts::well_baked_body_absorb),
            ..AbilityBehavior::default()
        },
        AbilityId::EARTH_EATER => AbilityBehavior {
            try_hit: Some(hit_boosts::earth_eater),
            absorb_reaction: Some(absorb::heal_quarter),
            ..AbilityBehavior::default()
        },
        AbilityId::EMERGENCY_EXIT | AbilityId::WIMP_OUT => AbilityBehavior {
            emergency_exit: true,
            ..AbilityBehavior::default()
        },
        AbilityId::MERCILESS => AbilityBehavior {
            crit_against_poisoned: true,
            ..AbilityBehavior::default()
        },
        // Gen 3-4 declare no callback: no battle effect.
        AbilityId::STENCH => AbilityBehavior {
            move_edit: Some(MoveEdit {
                flinch_chance: Some(10),
                ..MoveEdit::default()
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::HEALER => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::Healer,
                handler: veils::healer as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::COMATOSE => AbilityBehavior {
            switch_in: Some(entry_effects::announce),
            try_set_status: Some(veils::comatose),
            counts_as_asleep: true,
            ..AbilityBehavior::default()
        },
        AbilityId::PURIFYING_SALT => AbilityBehavior {
            try_set_status: Some(veils::purifying_salt),
            blocks_volatiles: &[crate::state::VolatileConditionKind::Yawn],
            // Ghost moves at half the attacking stat (`onSourceModifyAtk` 6 / `SpA` 5).
            foe_offense: Some(AbilityBoost {
                when: BoostWhen::MoveType(crate::dex::PokemonType::Ghost),
                modifier: BoostModifier::Fixed(2048),
                priority: 6,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::LEAF_GUARD => AbilityBehavior {
            try_set_status: Some(veils::leaf_guard),
            volatile_guard: Some(VolatileGuard {
                kinds: &[crate::state::VolatileConditionKind::Yawn],
                allies: false,
                when: VolatileGuardWhen::Sun,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::SWEET_VEIL => AbilityBehavior {
            ally_try_set_status: Some(veils::sweet_veil),
            volatile_guard: Some(VolatileGuard {
                kinds: &[crate::state::VolatileConditionKind::Yawn],
                allies: true,
                when: VolatileGuardWhen::Always,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::FLOWER_VEIL => AbilityBehavior {
            ally_try_set_status: Some(veils::flower_veil_status),
            ally_try_stat_change: Some(veils::flower_veil_drops),
            volatile_guard: Some(VolatileGuard {
                kinds: &[crate::state::VolatileConditionKind::Yawn],
                allies: true,
                when: VolatileGuardWhen::GrassType,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::AROMA_VEIL => AbilityBehavior {
            volatile_guard: Some(VolatileGuard {
                kinds: &[
                    crate::state::VolatileConditionKind::Attract,
                    crate::state::VolatileConditionKind::Disable,
                    crate::state::VolatileConditionKind::Encore,
                    crate::state::VolatileConditionKind::HealBlock,
                    crate::state::VolatileConditionKind::Taunt,
                    crate::state::VolatileConditionKind::Torment,
                ],
                allies: true,
                when: VolatileGuardWhen::Always,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::NATURAL_CURE => AbilityBehavior {
            switch_out: Some(natural_cure::switch_out),
            ..AbilityBehavior::default()
        },
        AbilityId::REGENERATOR => AbilityBehavior {
            switch_out: Some(regenerator::switch_out),
            ..AbilityBehavior::default()
        },
        AbilityId::ANGER_POINT => AbilityBehavior {
            on_hit: Some(hit_boosts::anger_point),
            after_sub_damage: Some(hit_boosts::anger_point_substitute),
            ..AbilityBehavior::default()
        },
        AbilityId::FLASH_FIRE => AbilityBehavior {
            try_hit: Some(flash_fire::flash_fire),
            absorb_reaction: Some(flash_fire::flash_fire_absorb),
            ends_volatile: Some(crate::state::VolatileConditionKind::FlashFire),
            ..AbilityBehavior::default()
        },
        AbilityId::GUARD_DOG => AbilityBehavior {
            blocks_drag_out: true,
            try_boost_first: Some(hit_boosts::guard_dog),
            ..AbilityBehavior::default()
        },
        AbilityId::BULLETPROOF => AbilityBehavior {
            try_hit: Some(move_guards::bulletproof),
            ..AbilityBehavior::default()
        },
        AbilityId::SOUNDPROOF => AbilityBehavior {
            try_hit: Some(move_guards::soundproof),
            ally_try_hit_side: Some(move_guards::soundproof_side),
            ..AbilityBehavior::default()
        },
        AbilityId::WONDER_GUARD => AbilityBehavior {
            try_hit: Some(move_guards::wonder_guard),
            ..AbilityBehavior::default()
        },
        AbilityId::DAZZLING | AbilityId::QUEENLY_MAJESTY | AbilityId::ARMOR_TAIL => {
            AbilityBehavior {
                blocks_priority_moves: true,
                ..AbilityBehavior::default()
            }
        }
        AbilityId::ANTICIPATION => AbilityBehavior {
            switch_in: Some(move_guards::anticipation),
            announces: Some(crate::state::SwitchInAnnouncement::Anticipation),
            ..AbilityBehavior::default()
        },
        AbilityId::FOREWARN => AbilityBehavior {
            switch_in: Some(move_guards::forewarn),
            announces: Some(crate::state::SwitchInAnnouncement::Forewarn),
            ..AbilityBehavior::default()
        },
        AbilityId::FRISK => AbilityBehavior {
            switch_in: Some(frisk::switch_in),
            announces: Some(crate::state::SwitchInAnnouncement::Frisk),
            ..AbilityBehavior::default()
        },
        AbilityId::STICKY_HOLD => AbilityBehavior {
            sticky_hold: true,
            ..AbilityBehavior::default()
        },
        // Gen 4's Multitype refuses both: its holder's item is never taken, and its ability is
        // never changed (`onTakeItem: false`, `onSetAbility: false`; the second is the callback
        // this family added, DESIGN.md §43.4).
        AbilityId::MULTITYPE => AbilityBehavior {
            keeps_item: true,
            refuses_ability_change: true,
            ..AbilityBehavior::default()
        },
        AbilityId::MAGICIAN => AbilityBehavior {
            magician: true,
            ..AbilityBehavior::default()
        },
        AbilityId::SYMBIOSIS => AbilityBehavior {
            symbiosis: true,
            ..AbilityBehavior::default()
        },
        AbilityId::PICKPOCKET => AbilityBehavior {
            pickpocket: true,
            ..AbilityBehavior::default()
        },
        AbilityId::UNBURDEN => AbilityBehavior {
            unburden: true,
            ..AbilityBehavior::default()
        },
        AbilityId::KLUTZ => AbilityBehavior {
            suppresses_own_item: true,
            ..AbilityBehavior::default()
        },
        AbilityId::LIQUID_OOZE => AbilityBehavior {
            source_try_heal: Some(liquid_ooze::liquid_ooze),
            ..AbilityBehavior::default()
        },
        AbilityId::SUCTION_CUPS => AbilityBehavior {
            blocks_drag_out: true,
            ..AbilityBehavior::default()
        },
        AbilityId::SPEED_BOOST => AbilityBehavior {
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::SpeedBoost,
                handler: speed_boost::residual as ResidualHandler,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::SERENE_GRACE => AbilityBehavior {
            secondary_chance_multiplier: Some(2),
            ..AbilityBehavior::default()
        },
        AbilityId::INFILTRATOR => AbilityBehavior {
            infiltrates: true,
            ..AbilityBehavior::default()
        },
        AbilityId::ELECTRIC_SURGE => AbilityBehavior {
            switch_in: Some(terrain::electric_surge),
            ..AbilityBehavior::default()
        },
        AbilityId::GRASSY_SURGE => AbilityBehavior {
            switch_in: Some(terrain::grassy_surge),
            ..AbilityBehavior::default()
        },
        AbilityId::MISTY_SURGE => AbilityBehavior {
            switch_in: Some(terrain::misty_surge),
            ..AbilityBehavior::default()
        },
        AbilityId::PSYCHIC_SURGE => AbilityBehavior {
            switch_in: Some(terrain::psychic_surge),
            ..AbilityBehavior::default()
        },
        AbilityId::HADRON_ENGINE => AbilityBehavior {
            switch_in: Some(terrain::hadron_engine),
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::SpecialAttack,
                boost: AbilityBoost {
                    when: BoostWhen::Terrain(crate::state::TerrainKind::Electric),
                    modifier: BoostModifier::Fixed(5461),
                    priority: 5,
                },
            }),
            ..AbilityBehavior::default()
        },
        // Showdown gives `onDamagingHit` no order: after every ordered reaction.
        AbilityId::SEED_SOWER => AbilityBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                i16::MAX,
                terrain::seed_sower as DamagingHitReactionHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::QUARK_DRIVE => AbilityBehavior {
            switch_in: Some(terrain::quark_drive),
            switch_in_priority: -2,
            quark_drive: true,
            ..AbilityBehavior::default()
        },
        AbilityId::SURGE_SURFER => AbilityBehavior {
            speed_boost: Some(SpeedBoost {
                when: SpeedWhen::Terrain(crate::state::TerrainKind::Electric),
                modifier: 8192,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::GRASS_PELT => AbilityBehavior {
            defense_boost: Some(AbilityBoost {
                when: BoostWhen::Terrain(crate::state::TerrainKind::Grassy),
                modifier: BoostModifier::Fixed(6144),
                priority: 6,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::POWER_SPOT => AbilityBehavior {
            ally_base_power: Some(OrderedHandler::new(
                22,
                ally_boost::power_spot as BasePowerChainHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::BATTERY => AbilityBehavior {
            ally_base_power: Some(OrderedHandler::new(
                22,
                ally_boost::battery as BasePowerChainHandler,
            )),
            ..AbilityBehavior::default()
        },
        AbilityId::STEELY_SPIRIT => AbilityBehavior {
            team_base_power: Some(AbilityBoost {
                when: BoostWhen::MoveType(crate::dex::PokemonType::Steel),
                modifier: BoostModifier::Fixed(6144),
                priority: 22,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::DARK_AURA => AbilityBehavior {
            switch_in: Some(entry_effects::announce),
            aura: Some(crate::dex::PokemonType::Dark),
            ..AbilityBehavior::default()
        },
        AbilityId::FAIRY_AURA => AbilityBehavior {
            switch_in: Some(entry_effects::announce),
            aura: Some(crate::dex::PokemonType::Fairy),
            ..AbilityBehavior::default()
        },
        AbilityId::AURA_BREAK => AbilityBehavior {
            switch_in: Some(entry_effects::announce),
            aura_break: true,
            ..AbilityBehavior::default()
        },
        AbilityId::TABLETS_OF_RUIN => ruin(crate::state::Stat::Attack),
        AbilityId::SWORD_OF_RUIN => ruin(crate::state::Stat::Defense),
        AbilityId::VESSEL_OF_RUIN => ruin(crate::state::Stat::SpecialAttack),
        AbilityId::BEADS_OF_RUIN => ruin(crate::state::Stat::SpecialDefense),
        AbilityId::KEEN_EYE => AbilityBehavior {
            try_stat_change: Some(drop_guard::keen_eye),
            move_edit: Some(MoveEdit {
                ignore_evasion: true,
                ..MoveEdit::default()
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::ILLUMINATE => AbilityBehavior {
            try_stat_change: Some(drop_guard::illuminate),
            move_edit: Some(MoveEdit {
                ignore_evasion: true,
                ..MoveEdit::default()
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::MINDS_EYE => AbilityBehavior {
            try_stat_change: Some(drop_guard::minds_eye),
            move_edit: Some(MoveEdit {
                ignore_evasion: true,
                normal_fighting_ignore_immunity: true,
                ..MoveEdit::default()
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::SCRAPPY => AbilityBehavior {
            try_stat_change: Some(drop_guard::scrappy),
            move_edit: Some(MoveEdit {
                normal_fighting_ignore_immunity: true,
                ..MoveEdit::default()
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::LONG_REACH => move_edit(MoveEdit {
            no_contact: true,
            ..MoveEdit::default()
        }),
        AbilityId::PROPELLER_TAIL | AbilityId::STALWART => move_edit(MoveEdit {
            tracks_target: true,
            ..MoveEdit::default()
        }),
        AbilityId::UNSEEN_FIST => move_edit(MoveEdit {
            contact_ignores_protect: true,
            ..MoveEdit::default()
        }),
        AbilityId::SKILL_LINK => move_edit(MoveEdit {
            max_hits: true,
            ..MoveEdit::default()
        }),
        AbilityId::HEAVY_METAL => AbilityBehavior {
            weight_modifier: Some(WeightModifier {
                priority: 1,
                factor: crate::rules::Fraction::new(2, 1),
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::LIGHT_METAL => AbilityBehavior {
            weight_modifier: Some(WeightModifier {
                priority: 0,
                factor: crate::rules::Fraction::new(1, 2),
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::ROCK_HEAD => AbilityBehavior {
            blocks_recoil: true,
            ..AbilityBehavior::default()
        },
        AbilityId::UNAWARE => AbilityBehavior {
            unaware: true,
            ..AbilityBehavior::default()
        },
        AbilityId::PLUS => plus_minus(AbilityId::MINUS),
        AbilityId::MINUS => plus_minus(AbilityId::PLUS),
        AbilityId::PARENTAL_BOND => AbilityBehavior {
            parental_bond: true,
            ..AbilityBehavior::default()
        },
        AbilityId::SHEER_FORCE => AbilityBehavior {
            move_edit: Some(MoveEdit {
                sheer_force: true,
                ..MoveEdit::default()
            }),
            base_power_boost: Some(AbilityBoost {
                when: BoostWhen::SheerForce,
                modifier: BoostModifier::Fixed(5325),
                priority: 21,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::SLOW_START => AbilityBehavior {
            switch_in: Some(slow_start::switch_in),
            residual: Some(ResidualParticipant {
                kind: crate::rules::ResidualKind::SlowStart,
                handler: slow_start::residual as ResidualHandler,
            }),
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::Both,
                boost: AbilityBoost {
                    when: BoostWhen::SlowStart,
                    modifier: BoostModifier::Fixed(2048),
                    priority: 5,
                },
            }),
            speed_boost: Some(SpeedBoost {
                when: SpeedWhen::SlowStart,
                modifier: 2048,
            }),
            announces: Some(crate::state::SwitchInAnnouncement::SlowStart),
            ..AbilityBehavior::default()
        },
        AbilityId::GORILLA_TACTICS => AbilityBehavior {
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::Attack,
                boost: AbilityBoost {
                    when: BoostWhen::Always,
                    modifier: BoostModifier::Fixed(6144),
                    priority: 1,
                },
            }),
            choice_lock: true,
            boost_suspended_by_dynamax: true,
            ..AbilityBehavior::default()
        },
        AbilityId::SUPREME_OVERLORD => AbilityBehavior {
            switch_in: Some(supreme_overlord::switch_in),
            base_power_boost: Some(AbilityBoost {
                when: BoostWhen::Always,
                modifier: BoostModifier::FallenAllies,
                priority: 21,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::STAKEOUT => AbilityBehavior {
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::Both,
                boost: AbilityBoost {
                    when: BoostWhen::OpponentEnteredThisTurn,
                    modifier: BoostModifier::Fixed(8192),
                    priority: 5,
                },
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::ORICHALCUM_PULSE => AbilityBehavior {
            switch_in: Some(weather::orichalcum_pulse),
            offense_boost: Some(OffenseBoost {
                stats: OffenseStats::Attack,
                boost: AbilityBoost {
                    when: BoostWhen::Weather(crate::state::WeatherKind::Sun),
                    modifier: BoostModifier::Fixed(5461),
                    priority: 5,
                },
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::NEUROFORCE => AbilityBehavior {
            damage_boost: Some(DamageBoost {
                when: DamageBoostWhen::SuperEffective,
                modifier: 5120,
            }),
            ..AbilityBehavior::default()
        },
        AbilityId::FRIEND_GUARD => AbilityBehavior {
            ally_damage_modifier: Some(3072),
            ..AbilityBehavior::default()
        },
        AbilityId::WATER_ABSORB => AbilityBehavior {
            try_hit: Some(absorb::water_absorb),
            absorb_reaction: Some(absorb::heal_quarter),
            ..AbilityBehavior::default()
        },
        AbilityId::VOLT_ABSORB => AbilityBehavior {
            try_hit: Some(absorb::volt_absorb),
            absorb_reaction: Some(absorb::heal_quarter),
            ..AbilityBehavior::default()
        },
        AbilityId::OVERCOAT => AbilityBehavior {
            weather_and_powder_immune: true,
            ..AbilityBehavior::default()
        },
        // Mycelium Might: a status move goes last in its bracket and ignores the breakable
        // abilities of the Pokémon it meets.
        AbilityId::MYCELIUM_MIGHT => AbilityBehavior {
            fractional_priority: Some(crate::effects::FractionalPriority::DelayStatus),
            move_edit: Some(MoveEdit {
                status_ignores_abilities: true,
                ..MoveEdit::default()
            }),
            ..AbilityBehavior::default()
        },
        // Magic Bounce: `onTryHit` (priority 1, after Magic Coat's 2); the reflection itself is
        // the hit gate's (`MovePipeline::reflect_move`).
        AbilityId::MAGIC_BOUNCE => AbilityBehavior {
            magic_bounce: true,
            ..AbilityBehavior::default()
        },
        // Dancer: no callbacks; the Shell's post-move step (DESIGN.md §42.9).
        AbilityId::DANCER => AbilityBehavior {
            dancer: true,
            ..AbilityBehavior::default()
        },
        AbilityId::STALL => AbilityBehavior {
            fractional_priority: Some(crate::effects::FractionalPriority::Delay),
            ..AbilityBehavior::default()
        },
        AbilityId::PRANKSTER => AbilityBehavior {
            modify_priority: Some(crate::effects::PriorityBoost::Prankster),
            ..AbilityBehavior::default()
        },
        AbilityId::GALE_WINGS => AbilityBehavior {
            modify_priority: Some(crate::effects::PriorityBoost::GaleWings),
            ..AbilityBehavior::default()
        },
        AbilityId::TRIAGE => AbilityBehavior {
            modify_priority: Some(crate::effects::PriorityBoost::Triage),
            ..AbilityBehavior::default()
        },
        AbilityId::QUICK_DRAW => AbilityBehavior {
            fractional_priority: Some(crate::effects::FractionalPriority::QuickDraw),
            ..AbilityBehavior::default()
        },
        AbilityId::TELEPATHY => AbilityBehavior {
            try_hit: Some(telepathy::try_hit),
            ..AbilityBehavior::default()
        },
        _ => AbilityBehavior::default(),
    }
}

/// Pickup's `onResidual` (`BattleContext::pickup`).
fn pickup_residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    ctx.pickup(holder);
}

/// Showdown `onEatItem` (the berry eaten, still held): Cheek Pouch heals its share of the
/// holder's HP (`heal`: truncated, at least 1; `TryHeal` refuses it under Heal Block); Ripen notes
/// whether the berry was a resist berry (`berryWeaken`), which halves the next hit the holder
/// takes; Cud Chew remembers the berry unless it was `taken` from another's hand (its `onEatItem`
/// skips Bug Bite and Pluck).
pub fn ate_item(
    ctx: &mut BattleContext<'_>,
    behavior: &AbilityBehavior,
    holder: PokemonId,
    item: crate::ids::ItemId,
    taken: bool,
) {
    if behavior.chews_the_cud && !taken {
        ctx.remember_for_cud_chew(holder, item);
    }
    if behavior.ripen {
        ctx.note_berry_weaken(holder, item);
    }
    if let Some(share) = behavior.eaten_berry_heal
        && let (Some(ability), Some(max_hp)) = (ctx.ability_of(holder), ctx.base_max_hp(holder))
    {
        let amount = share.apply_u16(max_hp).max(1);
        ctx.apply_heal(crate::engine::HealRequest {
            target: holder,
            amount,
            source: EffectSource::Ability {
                pokemon: holder,
                ability,
            },
            direct: false,
        });
    }
}

/// Showdown `onDamage` without a say in the damage: Berserk / Anger Shell note whether a move's
/// single strike hurt the holder.
pub fn damaged(
    ctx: &mut BattleContext<'_>,
    behavior: &AbilityBehavior,
    request: &crate::engine::DamageRequest,
) {
    if behavior.half_hp_boosts.is_some() {
        berserk::damaged(ctx, request);
    }
}

/// What a move dealt, for an `onAfterMoveSecondary` handler on one of the Pokémon it reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveDamage {
    pub attacker: PokemonId,
    /// The move's type and category as it was used.
    pub move_type: crate::dex::PokemonType,
    pub category: crate::dex::MoveCategory,
    /// What the move took from the holder's HP over all its hits.
    pub to_holder: u16,
    /// What it took over all its targets (`move.totalDamage`).
    pub total: u16,
    /// The move struck several times (`move.multihit`).
    pub multi_hit: bool,
}

/// Showdown `onAfterMoveSecondary` on a Pokémon a move reached (not after a move Sheer Force
/// boosted): Color Change (Gen 5+), Berserk / Anger Shell.
pub fn after_move_secondary(
    ctx: &mut BattleContext<'_>,
    behavior: &AbilityBehavior,
    holder: PokemonId,
    dealt: MoveDamage,
) {
    if behavior.retypes_after_move {
        types::color_change(ctx, holder, dealt.move_type, dealt.category);
    }
    if let Some(boosts) = behavior.half_hp_boosts {
        let damage = if boosts.counts_every_target && dealt.multi_hit {
            dealt.total
        } else {
            dealt.to_holder
        };
        berserk::after_move_secondary(ctx, holder, dealt.attacker, boosts, damage, dealt.total);
    }
}

pub fn switch_out(ctx: &mut BattleContext<'_>, behavior: &AbilityBehavior, source: PokemonId) {
    if let Some(handler) = behavior.switch_out {
        handler(ctx, source);
    }
}

/// Chlorophyll, Swift Swim, Sand Rush, Slush Rush: ×2 Speed in their weather.
fn speedy(weather: crate::state::WeatherKind) -> AbilityBehavior {
    AbilityBehavior {
        speed_boost: Some(SpeedBoost {
            when: SpeedWhen::Weather(weather),
            modifier: 8192,
        }),
        ..AbilityBehavior::default()
    }
}

fn strong_weather(kind: crate::state::WeatherKind) -> AbilityBehavior {
    AbilityBehavior {
        strong_weather: Some(kind),
        ..AbilityBehavior::default()
    }
}

/// Teraform Zero's `onAfterTerastallization`: for Terapagos-Stellar, the weather and the terrain
/// go (announced only when either was up).
fn teraform_zero(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let stellar = ctx.is_species(holder, crate::ids::SpeciesId::TERAPAGOS_STELLAR);
    if !stellar {
        return;
    }
    if ctx.field_weather().is_none() && ctx.terrain().is_none() {
        return;
    }
    ctx.ability_activated(holder, AbilityId::TERAFORM_ZERO);
    ctx.clear_weather();
    ctx.clear_terrain();
}

pub fn switch_in(ctx: &mut BattleContext<'_>, behavior: &AbilityBehavior, source: PokemonId) {
    if let Some(kind) = behavior.strong_weather
        && let Some(ability) = ctx.ability_of(source)
    {
        ctx.set_strong_weather(kind, source, ability);
    }
    if let Some(handler) = behavior.switch_in {
        handler(ctx, source);
    }
    if behavior.trace {
        trace::start(ctx, source);
    }
}

pub fn try_hit(behavior: &AbilityBehavior, ctx: &mut TryHitContext<'_>) -> TryHitResult {
    behavior
        .try_hit
        .map_or(TryHitResult::Allow, |handler| handler(ctx))
}

pub fn try_set_status(
    behavior: &AbilityBehavior,
    ctx: &TrySetStatusContext<'_>,
) -> TrySetStatusResult {
    behavior
        .try_set_status
        .map_or(TrySetStatusResult::Allow, |handler| handler(ctx))
}

pub fn after_set_status(
    behavior: &AbilityBehavior,
    ctx: &mut BattleContext<'_>,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
) {
    if let Some(handler) = behavior.after_set_status {
        handler(ctx, target, status, source);
    }
}

/// The handler's `chainModify` for the move, if any.
pub fn modify_offense(
    behavior: &AbilityBehavior,
    ctx: &mut OffensiveStatContext<'_>,
) -> Option<u32> {
    behavior
        .modify_offense
        .and_then(|ordered| (ordered.handler)(ctx))
}
