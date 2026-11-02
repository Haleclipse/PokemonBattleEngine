//! The resolved dex model (DESIGN.md §27): moves, abilities, items and species as the engine
//! reads them, with each generation's behavior and support classification.

use crate::ids::{AbilityId, ItemId, MoveId, SpeciesId};
use crate::rules::{Fraction, SemiInvulnerableKind, TargetPattern};
use crate::state::{NonVolatileStatus, PokemonTypes, Stat, VolatileConditionKind, WeatherKind};
use serde::{Deserialize, Serialize};

use super::PokemonType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectChance(pub Option<u16>);

/// A secondary effect's chance as Showdown declares it, in basis points, or `None` when nothing
/// is rolled (a move's own `self` effect, Bind's trap, a secondary with no `chance`). The two
/// differ in Gen 1-2, where a declared 100 % is still a 255/256 roll (DESIGN.md §31.2).
impl EffectChance {
    pub const CERTAIN: Self = Self(None);

    pub const fn percent(percent: u8) -> Self {
        Self(Some(percent as u16 * 100))
    }

    /// The declared chance, a certain effect counting as 100 %.
    pub const fn basis_points(self) -> u16 {
        match self.0 {
            Some(basis_points) => basis_points,
            None => 10_000,
        }
    }

    /// The chance doubled (a combined Water Pledge's side, DESIGN.md §42.9): capped at 100 %.
    pub const fn doubled(self) -> Self {
        match self.0 {
            Some(basis_points) => Self(Some(if basis_points > 5_000 {
                10_000
            } else {
                basis_points * 2
            })),
            None => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryTarget {
    Target,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryTrigger {
    PerTargetHit,
    /// The move's own `self` (Close Combat's drops), once after the hits.
    OnceAfterMove,
    /// Showdown `selfBoost` (Clanging Scales, Scale Shot): once every hit step is over, when
    /// the move did something (`moveResult`), after the struck targets' item reactions.
    AfterMove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryEffect {
    Status(NonVolatileStatus),
    StatStage {
        stat: Stat,
        delta: i8,
    },
    /// Flinch or Confusion; other volatiles are still unsupported.
    Volatile(VolatileConditionKind),
    /// One of three statuses at random (Showdown `this.sample`: Tri Attack, Dire Claw).
    RandomStatus([NonVolatileStatus; 3]),
    /// Gen 2 Thief's `onHit`: the user takes the target's item (`BattleContext::steal_item`).
    StealItem,
    /// Eerie Spell's `onHit`: the target's last move loses 3 PP (`BattleContext::eerie_spell`).
    EerieSpell,
}

/// A condition a secondary needs when it would roll (Showdown: the secondary's own `onHit`
/// check, or a move's `onModifyMove` that zeroes the chance). Unmet, nothing is rolled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryRequirement {
    /// Burning Jealousy, Alluring Voice: the target raised a stat this turn
    /// (`statsRaisedThisTurn`).
    TargetStatsRaisedThisTurn,
    /// Gen 4-5 Chatter: only this species' confuses (the gen5 mod's `onModifyMove`: Chatot).
    UserSpecies(SpeciesId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecondaryEffectSpec {
    pub chance: EffectChance,
    pub target: SecondaryTarget,
    pub trigger: SecondaryTrigger,
    pub effect: SecondaryEffect,
    pub requires: Option<SecondaryRequirement>,
    /// Decided by the previous spec's roll: one Showdown secondary with several effects (Gen 1
    /// Psychic's Special drop on both stats, Ancient Power's five boosts) rolls once.
    pub shares_roll: bool,
    /// One of Showdown's `secondaries`, which Shield Dust / Covert Cloak (target-directed ones)
    /// and Sheer Force take away; not the move's own `self`, `selfBoost` or `volatileStatus`
    /// (a binding move's trap).
    pub in_secondaries: bool,
    /// Added by the user's item (King's Rock, Razor Fang: `ItemMoveEdit::adds_flinch`), not
    /// the move's own: `ItemFlinchRules` says whether Serene Grace doubles it, and Gen 2's
    /// "no flinch for a sleeping or frozen target" skips only the move's own.
    pub from_item: bool,
}

impl SecondaryEffectSpec {
    pub const fn status(
        chance_percent: u8,
        target: SecondaryTarget,
        status: NonVolatileStatus,
    ) -> Self {
        Self {
            chance: EffectChance::percent(chance_percent),
            target,
            trigger: SecondaryTrigger::PerTargetHit,
            effect: SecondaryEffect::Status(status),
            requires: None,
            shares_roll: false,
            in_secondaries: true,
            from_item: false,
        }
    }

    pub const fn stat_stage(
        chance_percent: u8,
        target: SecondaryTarget,
        stat: Stat,
        delta: i8,
    ) -> Self {
        Self {
            chance: EffectChance::percent(chance_percent),
            target,
            trigger: SecondaryTrigger::OnceAfterMove,
            effect: SecondaryEffect::StatStage { stat, delta },
            requires: None,
            shares_roll: false,
            in_secondaries: false,
            from_item: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveCategory {
    Physical,
    Special,
    Status,
}

/// The primary effect of a status move the engine can express from declarative data alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusMoveEffect {
    /// Thunder Wave, Will-O-Wisp, Hypnosis …
    SetStatus(NonVolatileStatus),
    /// Swords Dance (self), Growl (target): applied to every resolved Pokémon target, so the
    /// target pattern decides whom.
    StatStages(Vec<(Stat, i8)>),
    /// Confuse Ray …
    Volatile(VolatileConditionKind),
    /// Recover, Soft-Boiled …: fraction of max HP.
    Heal(Fraction),
    /// Roar, Whirlwind: the target is dragged out for a random bench Pokémon (DESIGN.md §30.2).
    ForceSwitch,
    /// Sunny Day, Rain Dance, Sandstorm, Hail / Snowscape: sets the weather for five turns
    /// (§23.1); fails under the same weather.
    Weather(WeatherKind),
    /// Moonlight, Morning Sun, Synthesis, Shore Up: heals by the weather (§22.7).
    WeatherHeal(crate::effects::moves::WeatherHeal),
    /// Swagger, Flatter (then confusion), Toxic Thread (then poison): the stat stages first,
    /// then the rest, as Showdown's `runMoveEffects` orders a move's `boosts` before its
    /// `status` and `volatileStatus`.
    StagesThen(Vec<(Stat, i8)>, FollowUp),
    /// Psycho Shift: the user's status to the target; the user is cured once it took.
    TransferStatus,
    /// Purify: the target's status is cured, then the user heals this share of its max HP
    /// (rounded up).
    CureTarget(Fraction),
    /// Heal Pulse, Floral Healing: the target heals (`effects::moves::TargetHeal`).
    TargetHeal(crate::effects::moves::TargetHeal),
    /// Gen 1 Roar, Whirlwind (`forceSwitch: false`): nothing happens, and no accuracy check
    /// comes first (pokered `ResidualEffects1`; issues.md I74).
    Inert,
}

/// What a `StagesThen` status move does after its stat stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUp {
    /// A major status: when it does not take, the move fails on that target (Showdown
    /// `!hitResult && move.status`), the stages staying changed.
    Status(NonVolatileStatus),
    /// A volatile (confusion): the move did something if either part did.
    Volatile(VolatileConditionKind),
}

/// When a two-turn move attacks on its first turn instead of charging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeSkip {
    Never,
    /// Solar Beam / Solar Blade under sun.
    InSun,
    /// Electro Shot under rain (Showdown `effectiveWeather()`: a Utility Umbrella hides it,
    /// Primordial Sea's heavy rain counts), after its charging boost.
    InRain,
}

/// A charging move's shape (DESIGN.md §15.5): declared by the move's behavior, consumed by
/// the Move Shell's TryMove stage and the hit gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwoTurnSpec {
    pub skip: ChargeSkip,
    /// Where the user is while charging, if out of reach at all.
    pub invulnerable: Option<SemiInvulnerableKind>,
    /// Showdown `this.boost(...)` in the move's `onTryMove`: the user's own stat change as it
    /// starts charging, after `-prepare` and before the weather check (Skull Bash's Defense,
    /// Meteor Beam's and Electro Shot's Special Attack).
    pub charge_boost: Option<(crate::state::Stat, i8)>,
    /// Where the doubling against this hiding place applies where the generation does not say
    /// (`TwoTurnRules::bonus_stage_override`): the condition's `onSourceModifyDamage` (Fly, Dig,
    /// Dive) is the final damage, its `onSourceBasePower` (Bounce) the power.
    pub bonus_stage: crate::rules::DamageBonusStage,
}

/// What the engine does when a move's primary effect resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimaryEffect {
    /// Standard typed damage pipeline; every declarative property of the move is implemented.
    Damage,
    /// A status move expressed from its declarative data (§15).
    Status(StatusMoveEffect),
    /// A custom primary (`effects::moves::custom`) owns the primary effect.
    Custom,
    /// The move carries data or callbacks the engine has not implemented. It still resolves in
    /// the dex (names, PP and data stay queryable) but choice validation rejects it, so it can
    /// never silently do nothing in a battle. `ResolvedMove::unsupported` lists why.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveFlags {
    pub protectable: bool,
    /// Showdown `bypasssub` (sound moves, Gen 6+): status moves otherwise fail against a
    /// Substitute and damaging moves hit it.
    pub bypass_substitute: bool,
    pub contact: bool,
    /// Whether the type chart's 0× entries block the move (damaging moves: yes; status moves:
    /// only when Showdown declares `ignoreImmunity: false`, e.g. Thunder Wave vs Ground).
    pub type_immunity: bool,
    /// Showdown `forceSTAB` (the combined Pledge moves, DESIGN.md §42.9): the move counts as
    /// the user's own type for STAB.
    pub force_stab: bool,
    /// Showdown `ignoreImmunity` for the move's type on a move that checks the chart (Scrappy,
    /// Mind's Eye: Normal and Fighting moves): a 0× entry counts as 1×, the rest still applies.
    pub immunity_ignored: bool,
    /// Showdown `hasSheerForce`: Sheer Force took the move's secondaries and its own `self` away
    /// as it started (`engine::modified_move`) — BasePower ×1.3, and none of the
    /// `AfterMoveSecondary` events (a struck target's Red Card / Eject Button, the user's Life
    /// Orb).
    pub sheer_force: bool,
    /// Showdown `hasSheerForceBoost` (Electro Shot, Order Up): Sheer Force boosts the move
    /// without taking anything away from it.
    pub sheer_force_boost: bool,
    /// Parental Bond made the move strike twice, the second hit weakened
    /// (`engine::prepared_move`).
    pub parental_bond: bool,
    /// Showdown `noparentalbond`, `charge` or `futuremove`: Parental Bond leaves the move alone.
    pub parental_bond_exempt: bool,
    /// Showdown `futuremove` (Future Sight, Doom Desire): what its strike knocks out takes no
    /// Destiny Bond or Grudge along.
    pub future_move: bool,
    /// Showdown `powder` (Spore, Rage Powder …): Grass types (Gen 6+), Overcoat and Safety
    /// Goggles are immune.
    pub powder: bool,
    /// Showdown `defrost`: a frozen user of this move thaws (Flame Wheel, Scald …).
    pub defrost: bool,
    /// Showdown `failencore`: Encore cannot lock a target into this move (Encore itself,
    /// Transform, Mimic, Struggle …).
    pub fail_encore: bool,
    /// Showdown `heal`: a healing move (Recover, Drain Punch …), which Triage speeds up.
    pub heal: bool,
    /// Showdown `cantusetwice` (Gigaton Hammer, Blood Moon): off the request right after the
    /// user used it.
    pub cant_use_twice: bool,
    /// Showdown `sound` (Liquid Voice turns these Water; Soundproof refuses them).
    pub sound: bool,
    /// Showdown `bullet` (Shadow Ball, Seed Bomb …): Bulletproof refuses them.
    pub bullet: bool,
    /// Showdown `punch` (Iron Fist), `bite` (Strong Jaw), `pulse` (Mega Launcher), `slicing`
    /// (Sharpness).
    pub punch: bool,
    pub bite: bool,
    pub pulse: bool,
    pub slicing: bool,
    /// Showdown `gravity` (Fly, Bounce, High Jump Kick, Splash …): refused while Gravity is up.
    pub gravity: bool,
    /// Showdown `stallingMove` (Protect and its variants, Endure; Gen 5's Wide / Quick Guard):
    /// the consecutive-use roll of the `stall` counter (`BattleContext::stall_move`).
    pub stalling: bool,
    /// Showdown `wind` (Gust, Hurricane, Tailwind …): Wind Rider absorbs it, Wind Power charges.
    pub wind: bool,
    /// Showdown `minimize` (Stomp, Body Slam …): doubled against a Minimized target, and from
    /// Gen 6 sure to hit it.
    pub minimize: bool,
    /// Showdown `mustpressure` (Spikes, Stealth Rock, Toxic Spikes; Imprison and Snatch from Gen
    /// 5): every foe's Pressure counts, whatever the move aims at (`engine::pressure_holders`).
    pub must_pressure: bool,
    /// Showdown `sleepUsable` (Snore, Sleep Talk): a sleeping user's BeforeMove lets the move
    /// go ahead after its `cant|user|slp` (DESIGN.md §42.1).
    pub sleep_usable: bool,
    /// The copying and calling flags of DESIGN.md §42.2, straight from the snapshot's `flags`:
    /// which moves Metronome may draw (`metronome`), which Assist may draw (`noassist`), which
    /// Sleep Talk may draw (`nosleeptalk`), which Copycat (`failcopycat`), Mimic (`failmimic`)
    /// and Sketch (`nosketch`) refuse, which Instruct (`failinstruct`) and Me First
    /// (`failmefirst`) refuse, and which Mirror Move may copy (`mirror`).
    pub callable: bool,
    pub no_assist: bool,
    pub no_sleep_talk: bool,
    pub fail_copycat: bool,
    pub fail_mimic: bool,
    pub no_sketch: bool,
    pub fail_instruct: bool,
    pub fail_me_first: bool,
    pub mirror: bool,
    /// Showdown `reflectable` (Magic Coat, Magic Bounce), `snatch` and `dance` (Dancer).
    pub reflectable: bool,
    pub snatch: bool,
    pub dance: bool,
    /// Showdown `charge` and `recharge`: a two-turn move; the recharge turn (`Hyper Beam`).
    /// Sleep Talk and Instruct refuse both, and a charged move is no Instruct target.
    pub charge: bool,
    pub recharge: bool,
    /// Showdown `pledgecombo`: the three Pledge moves combine (DESIGN.md §42.4).
    pub pledge_combo: bool,
}

impl MoveFlags {
    pub const STANDARD: Self = Self {
        protectable: true,
        bypass_substitute: false,
        contact: false,
        type_immunity: true,
        force_stab: false,
        immunity_ignored: false,
        sheer_force: false,
        sheer_force_boost: false,
        parental_bond: false,
        parental_bond_exempt: false,
        future_move: false,
        powder: false,
        defrost: false,
        fail_encore: false,
        heal: false,
        cant_use_twice: false,
        sound: false,
        bullet: false,
        punch: false,
        bite: false,
        pulse: false,
        slicing: false,
        gravity: false,
        stalling: false,
        wind: false,
        minimize: false,
        must_pressure: false,
        sleep_usable: false,
        callable: false,
        no_assist: false,
        no_sleep_talk: false,
        fail_copycat: false,
        fail_mimic: false,
        no_sketch: false,
        fail_instruct: false,
        fail_me_first: false,
        mirror: false,
        reflectable: false,
        snatch: false,
        dance: false,
        charge: false,
        recharge: false,
        pledge_combo: false,
    };

    pub const STATUS: Self = Self {
        protectable: false,
        bypass_substitute: false,
        contact: false,
        type_immunity: false,
        force_stab: false,
        immunity_ignored: false,
        sheer_force: false,
        sheer_force_boost: false,
        parental_bond: false,
        parental_bond_exempt: false,
        future_move: false,
        powder: false,
        defrost: false,
        fail_encore: false,
        heal: false,
        cant_use_twice: false,
        sound: false,
        bullet: false,
        punch: false,
        bite: false,
        pulse: false,
        slicing: false,
        gravity: false,
        stalling: false,
        wind: false,
        minimize: false,
        must_pressure: false,
        sleep_usable: false,
        callable: false,
        no_assist: false,
        no_sleep_talk: false,
        fail_copycat: false,
        fail_mimic: false,
        no_sketch: false,
        fail_instruct: false,
        fail_me_first: false,
        mirror: false,
        reflectable: false,
        snatch: false,
        dance: false,
        charge: false,
        recharge: false,
        pledge_combo: false,
    };
}

/// Recoil the user of a move takes once it dealt damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recoil {
    /// Double-Edge: a fraction of the damage dealt (Showdown `recoil`).
    Dealt(Fraction),
    /// Chloroblast: a fraction of the user's max HP, rounded (Showdown's `move.id ===
    /// 'chloroblast'` in `calcRecoilDamage`: no data key).
    UserMaxHp(Fraction),
    /// Struggle from Gen 4 (`struggleRecoil`): a quarter of the user's max HP with the
    /// generation's rounding (`HitRules::struggle_recoil_rounding`), at least 1, straight to HP
    /// (Showdown `directDamage`: no ability or item stands in its way).
    Struggle,
}

/// Damage that bypasses the formula (Showdown `damage` / `damageCallback`): no critical hit,
/// no random roll, no modifier; type immunity still applies unless the move ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixedDamage {
    /// Seismic Toss, Night Shade: the user's level.
    Level,
    /// Dragon Rage (40), Sonic Boom (20).
    Amount(u16),
    /// Super Fang, Ruination: half the target's current HP, at least 1.
    HalfTargetHp,
    /// Guardian of Alola: three quarters of the target's current HP, at least 1 — or a quarter
    /// of that through a protection (`-zbroken`, DESIGN.md §44.4).
    ThreeQuartersTargetHp,
    /// Endeavor: the target's HP less the user's.
    TargetHpMinusUserHp,
    /// Psywave: a random amount by the user's level (`DamageRules::psywave`), drawn as the
    /// hit's damage roll.
    Psywave,
    /// Final Gambit: the user's current HP — and the user faints as it is taken (its
    /// `damageCallback`: `pokemon.faint()`).
    UserHp,
    /// Counter, Mirror Coat, Metal Burst, Comeuppance: the damage the user took this turn,
    /// multiplied (DESIGN.md §15.8); nothing to strike back at, the move fails.
    Retaliation(Retaliation),
    /// Bide's release (DESIGN.md §15.10): twice the damage it stored.
    Bide,
    /// Gen 2-3 Future Sight / Doom Desire striking (DESIGN.md §15.22, issues.md I110): the
    /// damage computed at use, the random factor drawn now (`FutureAttackRules::roll_at_hit`).
    Foreseen(u16),
}

/// What a retaliating move strikes back at (`FixedDamage::Retaliation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retaliation {
    /// Counter: the last physical hit from a foe, ×2.
    Counter,
    /// Mirror Coat: the last special hit from a foe, ×2.
    MirrorCoat,
    /// Metal Burst, Comeuppance: the last damaging hit from a foe, ×1.5.
    MetalBurst,
}

/// Showdown `selfSwitch`: the move orders its user out after it lands (DESIGN.md §30).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfSwitch {
    /// `selfSwitch: true` (U-turn, Volt Switch, Chilly Reception, Teleport): the replacement
    /// takes nothing over.
    Plain,
    /// `'copyvolatile'` / `'shedtail'` (Baton Pass, Shed Tail): the replacement takes over
    /// what the user hands on (§30.7).
    Pass(crate::state::PassKind),
}

/// Showdown `selfdestruct`: when the user of the move faints (DESIGN.md §30.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfDestruct {
    /// Explosion, Self-Destruct, Misty Explosion: whatever the move does, at the generation's
    /// point (`SelfFaintRules::timing`).
    Always,
    /// Memento, Final Gambit, Healing Wish, Lunar Dance: once the move reached a target
    /// (`runMoveEffects`).
    IfHit,
}

/// A one-hit KO move (Showdown `ohko`): `immune_type` is Sheer Cold's `'Ice'` from Gen 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ohko {
    pub immune_type: Option<PokemonType>,
}

/// Where the damage formula reads its stats (Showdown `overrideOffensivePokemon`,
/// `overrideOffensiveStat`, `overrideDefensiveStat`): Foul Play uses the target's Attack,
/// Body Press the user's Defense, Psyshock the target's Defense. `None` = by category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatSources {
    pub offense_from_target: bool,
    pub offense_stat: Option<Stat>,
    pub defense_stat: Option<Stat>,
    /// Showdown `ignoreDefensive` (Chip Away, Sacred Sword, Darkest Lariat): the target's
    /// defending stat is read at stage 0, raised or lowered.
    pub ignore_defensive_stage: bool,
    /// Showdown `ignoreEvasion` (the same moves): the accuracy check leaves out the target's
    /// evasion stage.
    pub ignore_evasion: bool,
}

/// How many times a damaging move strikes one target (ROADMAP §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiHit {
    Single,
    /// Double Kick (2), Surging Strikes (3), Population Bomb (10).
    Fixed(u8),
    /// Bullet Seed …: `[2, 5]` uses the generation's weighted distribution; any other range
    /// is uniform.
    Range {
        min: u8,
        max: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMove {
    pub id: MoveId,
    pub name: String,
    pub base_power: u16,
    pub accuracy: Option<u8>,
    pub priority: i8,
    pub category: MoveCategory,
    /// The category the move's data gives it (Showdown `dex.moves.get(move.id).category`),
    /// which a change as the move is used leaves alone: Photon Geyser made physical, an
    /// ordinary Z-Move taking its base move's (Gen 4-7 Slow Start reads it).
    pub data_category: MoveCategory,
    pub move_type: PokemonType,
    pub target: TargetPattern,
    pub is_spread: bool,
    pub max_pp: u8,
    pub primary_effect: PrimaryEffect,
    pub flags: MoveFlags,
    pub secondaries: Vec<SecondaryEffectSpec>,
    /// Showdown `move.secondaries` is set: any secondary, `secondary: {}` included (Sheer
    /// Force's condition).
    pub has_secondaries: bool,
    pub multi_hit: MultiHit,
    /// Recoil the user takes once the move dealt damage (ROADMAP §6.5).
    pub recoil: Option<Recoil>,
    /// Giga Drain: fraction of the damage dealt healed to the user.
    pub drain: Option<Fraction>,
    /// U-turn / Volt Switch / Baton Pass: the user is ordered out after the move lands
    /// (DESIGN.md §30).
    pub self_switch: Option<SelfSwitch>,
    /// Dragon Tail / Circle Throw: a damaging move that drags its target out after the hit
    /// (§30.2). Status phazing (Roar) is `StatusMoveEffect::ForceSwitch`.
    pub force_switch: bool,
    /// Showdown `critRatio` (1 for an ordinary move, 2 for Slash …).
    pub crit_ratio: u8,
    /// Showdown `willCrit`: `Some(true)` every hit is a critical hit (Frost Breath, Surging
    /// Strikes), `Some(false)` none is (Gen 2 Flail / Reversal), `None` the ratio decides.
    pub will_crit: Option<bool>,
    /// Showdown `noDamageVariance`: no random roll (Gen 2 Flail / Reversal).
    pub no_damage_variance: bool,
    /// Showdown `multiaccuracy` (Triple Kick, Population Bomb): every hit after the first
    /// checks accuracy again, and a miss ends the sequence.
    pub multi_accuracy: bool,
    /// Showdown `tracksTarget` (Snipe Shot): the move keeps its target — no `RedirectTarget`
    /// (Follow Me, Storm Drain). Showdown also follows the original target to its new position
    /// (Ally Switch), which the engine does not model yet.
    pub tracks_target: bool,
    /// Showdown `callsMove` (Metronome, Copycat, Sleep Talk …): the move's work is calling
    /// another. The Metronome item neither counts it nor starts over on it (`onTryMove`), so the
    /// move it calls carries the chain on (DESIGN.md R-M8-59).
    pub calls_move: bool,
    /// Showdown `breaksProtect` (Feint, Hyperspace Hole): once through the hit steps, the move
    /// takes down its target's protection and its side's guards (`hitStepBreakProtect`).
    pub breaks_protect: bool,
    /// Showdown `stealsBoosts` (Spectral Thief): once through the hit steps, the target's
    /// positive stages go to the user (`hitStepStealBoosts`).
    pub steals_boosts: bool,
    /// Showdown `smartTarget` (Dragon Darts): in doubles its hits split between the target and
    /// the partner beside it (`engine::smart_targets`).
    pub smart_target: bool,
    /// Showdown `nonGhostTarget` (Curse): what the move aims at when its user is not a Ghost
    /// type — on the request and as it starts (`engine::modified_move`).
    pub non_ghost_target: Option<crate::rules::TargetPattern>,
    pub fixed_damage: Option<FixedDamage>,
    /// Showdown `selfdestruct`.
    pub self_destruct: Option<SelfDestruct>,
    pub ohko: Option<Ohko>,
    pub stat_sources: StatSources,
    /// Showdown `thawsTarget` (Scald): a frozen target it hits thaws.
    pub thaws_target: bool,
    /// Showdown `ignoreAbility` (Sunsteel Strike, Moongeist Beam, Photon Geyser): while the
    /// move is used, the other Pokémon's breakable abilities do nothing
    /// (`engine::suppressing_ability`); a user's Mold Breaker sets it too
    /// (`engine::modified_move`).
    pub ignores_ability: bool,
    /// Showdown `isZ`: the Z-Crystal this move *is*, as the item's id (`None` for an ordinary
    /// move). Whether it is a generic Z-Move or a signature one is the crystal's business
    /// (`ResolvedItem::z_move_from`, DESIGN.md §44.4). A Z-Move in a move slot is an ordinary move that happens to be one — the crystal
    /// that turns another move into it is the Z-Move mechanic itself (DESIGN.md §44.4).
    pub z_crystal: Option<crate::ids::ItemId>,
    /// Showdown `isMax`: the move *is* a Max Move.
    pub is_max: bool,
    /// Gen 7 (DESIGN.md §44.1): the power of the damaging move's Z-Move — its data's
    /// `zMove.basePower`, else `dex::z_move_power`. `None` for a status move, a Z-Move or Max
    /// Move, Struggle, and in a generation without Z-Moves.
    pub z_power: Option<u16>,
    /// Gen 7: what the status move's Z version adds (`zMove.boost` / `zMove.effect`); `None`
    /// when it adds nothing, and outside Gen 7.
    pub z_status: Option<super::ZStatusEffect>,
    /// Gen 8 (DESIGN.md §44.1): the power of the damaging move's Max Move — its data's
    /// `maxMove.basePower`, else `dex::max_move_power`. `None` as `z_power` is.
    pub max_power: Option<u16>,
    /// The move's data behavior as this generation has them (DESIGN.md §25.1): each component is
    /// kept only when the generation's snapshot declares the Showdown callback it stands for
    /// (Gen 2-4 Growth has no `onModifyMove`, so no sun doubling; Gen 3 Blizzard has none, so
    /// no hail accuracy). The engine reads move behaviour from here, never from the global
    /// table. Fly / Dig / Solar Beam's charge is `behavior.two_turn` (§15.5).
    pub behavior: crate::effects::moves::MoveBehavior,
    /// Why `primary_effect` is `Unsupported`; empty otherwise. Each entry names one mechanic,
    /// callback or target pattern the engine has not implemented.
    pub unsupported: Vec<String>,
}

impl ResolvedMove {
    /// Showdown `move.multihit` set: a multi-hit move, or Beat Up (its `onModifyMove` sets it to
    /// the party members it calls on). Such a move's hits are a sequence: each one's own
    /// `Update` (Gen 2-3), Gen 3 Shell Bell, the hit count shown.
    pub fn multi_strike(&self) -> bool {
        self.multi_hit != MultiHit::Single || self.behavior.beat_up
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAbility {
    pub id: AbilityId,
    pub name: String,
    /// Every Showdown callback the snapshot declares is covered by a behavior component (or there is
    /// none to cover). A partially implemented ability is *not* implemented.
    pub implemented: bool,
    /// Showdown callbacks the snapshot declares that no behavior component covers.
    pub missing_callbacks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedItem {
    pub id: ItemId,
    pub name: String,
    /// Showdown `isBerry` (the snapshot's `is_berry`): what Harvest can bring back.
    pub is_berry: bool,
    /// Showdown `zMove` (DESIGN.md §44.4): `Some(None)` for a type Z-Crystal (any move of
    /// `z_move_type`), `Some(Some(move))` for a signature crystal (that move alone).
    pub z_crystal: Option<Option<MoveId>>,
    /// Showdown `zMoveFrom`: the move a signature crystal turns into its Z-Move.
    pub z_move_from: Option<MoveId>,
    /// Showdown `zMoveType`: the type a type Z-Crystal converts.
    pub z_move_type: Option<PokemonType>,
    /// Showdown `onPlate` (the snapshot's `plate`): the type a Plate gives its holder's
    /// Multitype (Arceus) and Judgment (DESIGN.md §43.4).
    pub plate: Option<PokemonType>,
    /// Showdown `onMemory` (the snapshot's `memory`): the type a Memory gives its holder's RKS
    /// System (Silvally) and Multi-Attack (DESIGN.md §43.4).
    pub memory: Option<PokemonType>,
    /// Showdown `onDrive` (the snapshot's `drive`): the type a Drive gives Techno Blast
    /// (DESIGN.md §43.4).
    pub drive: Option<PokemonType>,
    /// Showdown `itemUser`: the species a signature Z-Crystal or Mega Stone belongs to.
    pub item_user: Vec<crate::ids::SpeciesId>,
    /// Showdown `megaStone` (DESIGN.md §44.3): the mega forme this stone gives — the species it
    /// belongs to (`species.baseSpecies`, what the `onTakeItem` check reads) and the forme
    /// (what `canMegaEvo` returns).
    pub mega_stone: Option<(crate::ids::SpeciesId, crate::ids::SpeciesId)>,
    /// Every Showdown callback the snapshot declares is covered by a behavior component (or there is
    /// none to cover). A partially implemented item is *not* implemented.
    pub implemented: bool,
    /// Showdown callbacks the snapshot declares that no behavior component covers.
    pub missing_callbacks: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseStats {
    pub hp: u16,
    pub attack: u16,
    pub defense: u16,
    pub special_attack: u16,
    pub special_defense: u16,
    pub speed: u16,
}

/// What kind of forme a species entry is (Pokémon Online's forme flags, mapped from Showdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormeKind {
    Base,
    Mega,
    Primal,
    Regional,
    /// Exists only inside a battle (Ash-Greninja, Zen Mode, Ultra Necrozma).
    BattleOnly,
    Other,
}

/// Static species data. Battles never hold a species; the scenario compiler uses it to derive
/// `PokemonState` stats and types from a set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSpecies {
    pub id: SpeciesId,
    /// Showdown id, e.g. `pikachuhoenn`; the name-lookup key.
    pub key: String,
    pub name: String,
    pub forme_kind: FormeKind,
    pub types: PokemonTypes,
    pub base_stats: BaseStats,
    /// Legal abilities in slot order (0, 1, H, S); empty before Gen 3.
    pub abilities: Vec<AbilityId>,
    pub weight_hg: u32,
    pub base_species: Option<String>,
    pub forme: Option<String>,
    /// The forme a battle-only forme goes back to (Showdown `changesFrom`, or `battleOnly` when
    /// it names one: Darmanitan-Zen → Darmanitan, Minior-Meteor → Minior).
    pub changes_from: Option<String>,
    /// Not fully evolved in this generation (Eviolite).
    pub nfe: bool,
    /// The fixed gender every Pokémon of the species has (Showdown `species.gender`), or
    /// `None` when the species has a gender ratio.
    pub gender: Option<crate::state::Gender>,
    /// Showdown `cannotDynamax` (DESIGN.md §44.5): Zacian, Zamazenta, Eternatus.
    pub cannot_dynamax: bool,
    /// Showdown `requiredMove` (DESIGN.md §44.3): the move a forme needs — Mega Rayquaza
    /// (a Dragon Ascent in the moves is its whole `canMegaEvo`).
    pub required_move: Option<crate::ids::MoveId>,
}

/// Support counts of one generation's dex (`ResolvedDex::support_summary`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportSummary {
    pub moves: usize,
    pub damage: usize,
    pub status: usize,
    pub custom: usize,
    pub unsupported: usize,
    pub abilities: usize,
    pub abilities_implemented: usize,
    pub items: usize,
    pub items_implemented: usize,
    pub species: usize,
}
