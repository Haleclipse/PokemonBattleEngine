//! Volatile conditions' kinds and values (DESIGN.md §30): `VolatileConditionKind`, the
//! condition a Pokémon holds with its data (`VolatileCondition`), the states of the conditions
//! that count (Thrash's rampage, Uproar, Bide), Protect's kinds, `VolatileSet`.

use crate::ids::{ItemId, MoveId, PokemonId, PositionId};
use serde::{Deserialize, Serialize};

/// Which protecting move stands behind a `Protect` volatile (Showdown: one volatile each,
/// never two at once): whether it stops status moves, and what a contact move that it stops
/// costs the attacker (`engine::protect_punishment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtectKind {
    /// Protect, Detect.
    Protect,
    KingsShield,
    SpikyShield,
    BanefulBunker,
    Obstruct,
    SilkTrap,
    BurningBulwark,
    /// Max Guard (DESIGN.md §44.5): a Dynamaxed Pokémon's Protect variant, which blocks every
    /// move but a fixed list (Z-Moves and Max Moves included).
    MaxGuard,
}

impl ProtectKind {
    /// Showdown `checkMoveBypassesProtect(…, blockStatus)`: King's Shield, Obstruct, Silk Trap
    /// and Burning Bulwark let status moves through.
    pub fn blocks_status(self) -> bool {
        matches!(
            self,
            Self::Protect | Self::SpikyShield | Self::BanefulBunker
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum VolatileConditionKind {
    Protect,
    /// Endure: this turn, a move's damage leaves the holder at 1 HP at least.
    Endure,
    Substitute,
    /// Leech Seed: drained at the end of every turn towards the seeder's position.
    LeechSeed,
    /// Set by a secondary effect; consumed at the target's next BeforeMove or cleared at end
    /// of turn, whichever comes first.
    Flinch,
    /// Showdown `time` convention: decremented at BeforeMove, ends at 0.
    Confusion,
    /// Taunt: no status moves; `turns` counts down in the residual schedule (order 15).
    Taunt,
    /// Encore: only the encored move; counts down in the residual schedule (order 16), ends
    /// early when that move runs out of PP.
    Encore,
    /// Disable: the disabled move cannot be used; counts down in the residual schedule (17).
    Disable,
    /// The turn after Hyper Beam: the next action is spent recharging (BeforeMove, first).
    MustRecharge,
    /// A Choice item locked the holder into the first move it used (BeforeMove, last). A
    /// request refused by Gorilla Tactics' lock (`AbilityState::choice_lock`) names it too.
    ChoiceLock,
    /// Follow Me: foes' single-target moves are redirected to the holder this turn.
    FollowMe,
    /// A two-turn move in progress (Fly, Dig, Solar Beam): the user is locked into it for the
    /// next request, cannot switch, and may be out of reach while charging.
    TwoTurnMove,
    /// Bind / Wrap / Fire Spin …: chipped every turn and unable to switch while the trapper
    /// stays on the field (Showdown `partiallytrapped`).
    PartiallyTrapped,
    /// Ingrain: rooted (cannot switch or be dragged out), healed every turn.
    Ingrain,
    /// Protect / Detect's consecutive-use counter (Showdown `stall`, duration 2).
    Stall,
    /// Ally Switch's consecutive-use counter (Showdown `allyswitch`, Gen 9's `onPrepareHit`,
    /// duration 2).
    AllySwitch,
    /// Helping Hand on this Pokémon for the turn: its move's BasePower chain gets the multiplier.
    HelpingHand,
    /// Rage Powder: like Follow Me for the turn, but a powder-immune attacker ignores it.
    RagePowder,
    /// Gen 3-4 marker left by a broken Substitute (`substitutebroken`).
    SubstituteBroken,
    /// Gen 2's bad-poison counter (`residualdmg`), started by Toxic and gone on switch-out.
    ResidualDamage,
    /// Showdown `toxic` volatile (Gen 6+): a Poison type using Toxic cannot miss until the end
    /// of the turn (duration 1, ticked last in the residual).
    SureHit,
    /// Magic Coat (DESIGN.md §42.8): this turn, a reflectable move aimed at the holder is used
    /// again against its user (duration 1). Gen 4 ends it as it reflects, Gen 3 / 5+ at the end
    /// of the turn.
    MagicCoat,
    /// Snatch (DESIGN.md §42.8): this turn, a move carrying the `snatch` flag used by anyone is
    /// taken over by the holder (duration 1, gone as it steals).
    Snatch,
    /// Me First (DESIGN.md §42.8): the holder uses the move its target had queued, ×1.5 (the
    /// volatile is the boost's marker; duration 1).
    MeFirst,
    /// Gen 1 Reflect / Light Screen (`volatileStatus`, no duration): the holder's Defense /
    /// Special counts double in the damage formula until it leaves the field.
    Reflect,
    LightScreen,
    /// Gen 1-2 Mist (`volatileStatus`, no duration): other Pokémon cannot lower the holder's
    /// stats until it leaves the field.
    Mist,
    /// Attract (infatuation): half the holder's moves fail (BeforeMove); ends when the Pokémon
    /// it is infatuated with leaves the field.
    Attract,
    /// Quark Drive's volatile (`quarkdrive`): the holder's best stat, picked as it starts, is
    /// raised while Electric Terrain lasts.
    QuarkDrive,
    /// Protosynthesis's (`protosynthesis`): the same, while the sun is up.
    Protosynthesis,
    /// Gen 3-5 Mud Sport / Water Sport (`volatileStatus`, no duration): Electric / Fire moves
    /// are weakened while the holder stays in.
    MudSport,
    WaterSport,
    /// Yawn: the holder falls asleep when its two turns run out (residual).
    Yawn,
    /// Nightmare: a sleeping holder loses a quarter of its max HP every turn; ends when it
    /// wakes.
    Nightmare,
    /// Heal Block: the holder cannot use moves that heal, and heals fail (Gen 4: only a drain's
    /// and Leech Seed's).
    HealBlock,
    /// Embargo (Gen 4-7): the holder's item has no effect (`engine::held_item`).
    Embargo,
    /// Salt Cure: the holder loses an eighth of its max HP every turn (a quarter for a Water or
    /// Steel type).
    SaltCure,
    /// Syrup Bomb: the holder's Speed drops by one every turn for three turns, while the
    /// Pokémon that threw it stays on the field.
    SyrupBomb,
    /// Throat Chop: the holder cannot use sound moves for two turns.
    ThroatChop,
    /// Tar Shot: Fire moves are doubly effective against the holder.
    TarShot,
    /// Unburden's (`unburden`, R211): an Unburden holder's item was taken or used up; while it
    /// holds nothing its Speed doubles. The ability's `onEnd` removes it as the holder leaves,
    /// before Baton Pass could hand it on.
    Unburden,
    /// Defense Curl's mark (`defensecurl`): nothing of its own; Rollout and Ice Ball read it.
    DefenseCurl,
    /// Aqua Ring: the holder heals a sixteenth of its max HP every turn (residual).
    AquaRing,
    /// Stockpile: up to three layers, for Spit Up and Swallow; the stages it raised come back
    /// down as it ends.
    Stockpile,
    /// Power Trick: the holder's stored Attack and Defense are swapped while it lasts.
    PowerTrick,
    /// Gastro Acid (Gen 4+): the holder's ability has no effect (`engine::ignoring_ability`)
    /// until it leaves the field; Baton Pass hands it on.
    GastroAcid,
    /// Focus Energy: the holder's critical-hit ratio rises (`ConditionRules::focus_energy_stages`).
    FocusEnergy,
    /// Dragon Cheer: +1 critical-hit ratio, +2 for a Dragon type as it started.
    DragonCheer,
    /// Laser Focus: the holder's next hits this turn and the next are critical.
    LaserFocus,
    /// Micle Berry's boost (`micleberry`): the holder's next move that checks accuracy is more
    /// accurate.
    MicleBerry,
    /// Fling's (`fling`): the holder threw this item; it loses it at the next `Update`.
    Fling,
    /// Charge: the holder's next Electric move has double power.
    Charge,
    /// No Retreat: the holder cannot switch out.
    NoRetreat,
    /// Minimize's mark (`minimize`): `minimize` moves are doubled against the holder, and from
    /// Gen 6 sure to hit it.
    Minimize,
    /// Roost's (`roost`, `duration: 1`): the holder's types leave out Flying until the end of
    /// the turn (`onType`, `engine::types_of`).
    Roost,
    /// Zen Mode's (`zenmode`, no duration): the holder wears its Zen forme (Showdown's ability
    /// adds it at the end of the turn and drops it again; DESIGN.md §43.4).
    ZenMode,
    /// Commander's `commanding` (Tatsugiri in its ally's mouth, DESIGN.md §43.4): the holder
    /// acts not at all and cannot be dragged out; it is not invulnerable.
    Commanding,
    /// Commander's `commanded` (the Dondozo that swallowed its Tatsugiri): the five stats rose
    /// as it started, and it cannot be dragged out.
    Commanded,
    /// Powder's (`powder`, `duration: 1`): a Fire move the holder uses this turn blows up in its
    /// face instead (`MovePipeline` TryMove, DESIGN.md R275).
    Powder,
    /// Electrify's (`electrify`, `duration: 1`): the holder's move this turn is Electric
    /// (`onModifyType`, `engine::modified_move`).
    Electrify,
    /// Foresight / Odor Sleuth's identification (`foresight`): Normal and Fighting moves hit the
    /// Ghost holder, and its evasion counts as the generation says (`IdentifyRules`).
    Foresight,
    /// Miracle Eye's identification (`miracleeye`): Psychic moves hit the Dark holder, and its
    /// evasion counts as Foresight's does.
    MiracleEye,
    /// Smack Down / Thousand Arrows (`smackdown`): the holder is on the ground
    /// (`environment::grounded`) until it leaves the field.
    SmackDown,
    /// Magnet Rise (`magnetrise`, `duration: 5`): the holder is off the ground.
    MagnetRise,
    /// Telekinesis (`telekinesis`, `duration: 3`): the holder is off the ground, and moves at it
    /// (but a one-hit KO) never miss.
    Telekinesis,
    /// Autotomize: the holder is lighter by 100 kg per use.
    Autotomize,
    /// Flash Fire's boost (`flashfire`): the holder's Fire moves are stronger.
    FlashFire,
    /// Sparkling Aria's mark on a target its secondary reached, read (and removed) as the move
    /// ends.
    SparklingAria,
    /// The Metronome item's count (Showdown `metronome`, added as the item starts): the move
    /// it counts and how many times in a row (DESIGN.md R126).
    Metronome,
    /// Perish Song (Showdown `perishsong`, duration 4): the holder faints when the count, ticked
    /// in the residual, reaches 0.
    PerishSong,
    /// Chilly Reception's `priorityChargeCallback` (duration 1): the user announced it as the
    /// turn began; it only shows (`-prepare`).
    ChillyReception,
    /// Mean Look, Block, Jaw Lock … (Showdown `trapped`, linked to the trapper's `trapper`): the
    /// holder cannot switch; it goes free when its trapper leaves the field.
    Trapped,
    /// Showdown `trapper`: the holder keeps some Pokémon `Trapped`, and stays one while it does.
    Trapper,
    /// Octolock (Showdown `octolock`, not linked): the holder cannot switch while its source is
    /// on the field, and loses Defense and Special Defense in the residual until it leaves.
    Octolock,
    /// Gen 1 Bind / Wrap / Clamp / Fire Spin (Showdown `partialtrappinglock`): the user repeats
    /// the move on the Pokémon it caught, for the turns it rolled, with the first hit's damage.
    PartialTrappingLock,
    /// Gen 1 (Showdown `fakepartiallytrapped`): the lock ended this turn; both Pokémon show it
    /// for one more turn (it changes nothing the engine models).
    FakePartiallyTrapped,
    /// Focus Punch's turn-start volatile (duration 1): a damaging hit on the holder makes it
    /// lose its focus; it cannot flinch meanwhile.
    FocusPunch,
    /// Beak Blast's turn-start volatile (duration 1): a contact hit on the holder burns the
    /// attacker, until the holder's Beak Blast is over.
    BeakBlast,
    /// Shell Trap's turn-start volatile (duration 1): a physical hit from a foe arms the trap and
    /// moves the holder's action to the front.
    ShellTrap,
    /// Gen 3+ Counter's / Mirror Coat's turn-start volatile (`beforeTurnCallback`, duration 1):
    /// the hit it answers is the attack record's (DESIGN.md §15.8); it only shows.
    Counter,
    MirrorCoat,
    /// Destiny Bond (Showdown `destinybond`, no duration, `noCopy`): a foe's move knocking the
    /// holder out knocks that foe out too; gone at the holder's next move attempt.
    DestinyBond,
    /// Grudge (Showdown `grudge`, no duration): a move knocking the holder out loses all its
    /// PP; gone at the holder's next move attempt.
    Grudge,
    /// Spotlight (Showdown `spotlight`, duration 1, `noCopy`): the holder's foes' single-target
    /// moves turn to it, ahead of Follow Me.
    Spotlight,
    /// Bide (Showdown `bide`): the holder stores the damage it takes, then strikes back with
    /// twice as much (DESIGN.md §15.10).
    Bide,
    /// Pursuit's mark (Showdown `pursuit`, duration 1): the pursuers that strike the holder if
    /// it switches out this turn (DESIGN.md §15.11).
    Pursuit,
    /// Rollout / Ice Ball (Showdown `rollout` / `iceball`, duration 1): the hits so far, the
    /// user locked into the move while the chain goes on (DESIGN.md §15.12).
    Rolling,
    /// Gen 7's Rollout storage glitch (Showdown `rolloutstorage`, issues.md I212): the doublings
    /// a chain lost to Disguise, waiting for the holder's next moves that compute a power.
    RolloutStorage,
    /// Dynamax (its own `onResidual`, priority −100).
    Dynamax,
    /// Thrash, Petal Dance, Outrage, Raging Fury (Showdown `lockedmove`, DESIGN.md §15.12): the
    /// user is locked into the move for its turns and is confused when they end.
    Rampage,
    /// Uproar (Showdown `uproar`, DESIGN.md §15.12): the user is locked into the move for its
    /// turns, and no one on the field falls asleep meanwhile.
    Uproar,
    /// Torment (Showdown `torment`): the holder cannot choose the move it used last.
    Torment,
    /// Imprison (Showdown `imprison`, on its user): the holder's foes cannot use the moves it
    /// knows.
    Imprison,
    /// Lock-On / Mind Reader on their user (Gen 3+, Showdown `lockon`): its moves against the
    /// target skip the accuracy check (DESIGN.md §15.14).
    LockOn,
    /// Gen 2 Lock-On / Mind Reader on the target (the cartridge's flag): the next accuracy check
    /// against it is skipped and clears it.
    LockedOn,
    /// A Ghost type's Curse on the holder (Showdown `curse`): a quarter of its max HP at every
    /// end of the turn (Gen 2: after its own move) — DESIGN.md §15.16.
    Curse,
    /// Fury Cutter's chain (DESIGN.md §15.18): Gen 5+ Showdown's `furycutter` (a multiplier, 2
    /// turns), Gen 2-4 the cartridge's count (no time limit).
    FuryCutter,
    /// Rage on its user (Showdown `rage`, the cartridges' flag; DESIGN.md §15.19): a hit builds
    /// its Attack — Gen 2 a count that multiplies Rage's damage.
    Rage,
    /// Glaive Rush's drawback on its user (Showdown `glaiverush`, DESIGN.md §15.20): every move
    /// at it hits and deals double damage, until its next move attempt.
    GlaiveRush,
    /// Gen 5+ Truant (Showdown `truant`, DESIGN.md §15.21): the holder's next move attempt that
    /// reaches the check loafs, and the volatile goes. Baton Pass hands it on (no `noCopy`).
    Truant,
    /// Gen 1 Fly / Dig's hiding (Showdown gen1 `invulnerability`, pokered `INVULNERABLE`,
    /// `ChargeModel::Gen1`): out of reach of all but Swift and Transform, outliving a charge
    /// that full paralysis or Disable ended.
    Invulnerability,
    /// Sky Drop on its user (Showdown `skydrop`, `duration: 2`, DESIGN.md §15.23): it carries
    /// `target` in the air until the next turn's drop.
    SkyDrop,
    /// Sky Drop on the Pokémon carried up (no Showdown condition: `isSkyDropped`): it cannot move,
    /// switch or be dragged out, and is out of reach as its carrier is.
    SkyDropped,
}

/// Showdown `lockedmove` (DESIGN.md §15.12), as the generation counts it
/// (`RampageRules::counter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RampageState {
    #[serde(with = "crate::snapshot::wire")]
    pub move_id: MoveId,
    /// The frame's `lockedmove`: Gen 1 the uses left (`time`, counted down as the move starts
    /// again); Gen 3-4 the cartridge's counter (2-3, counted down at the end of every turn);
    /// Gen 5+ Showdown's `duration` (2, back to 2 after every hit while `left` allows it, 1 at
    /// the end of a turn).
    pub turns: u8,
    /// Gen 5+ Showdown's hidden `trueDuration` (2-3, counted down by the end of the turn); 0
    /// elsewhere.
    pub left: u8,
    /// Gen 1: the accuracy byte carried from use to use (Showdown `lockedmove.accuracy`).
    pub accuracy: u8,
    /// Gen 3-4: the move failed this turn (pokeemerald `WasUnableToUseMove`, pokeplatinum
    /// `BattleContext_MoveFailed`): the lock ends at the end of the turn without confusion.
    pub failed: bool,
}

/// Showdown `uproar` (DESIGN.md §15.12), as the generation counts it (`UproarRules`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UproarState {
    /// The frame's `uproar`: the turns left, the current one included (Gen 5+ Showdown's
    /// `duration`; Gen 3-4 the cartridge's count), counted down at the holder's turn-end step.
    pub turns: u8,
    /// Gen 3-4: the move failed this turn (`UproarRules::failing_blocks`): the turn-end step
    /// ends it.
    pub failed: bool,
}

/// What Bide keeps (Showdown's `bide` effect state, DESIGN.md §15.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BideState {
    /// Gen 2+ `duration`, ticked at the end of every turn: the release comes at the holder's
    /// move on the turn it reads 1. Gen 1's has none.
    pub turns: Option<u8>,
    /// Gen 1 `time`: counted down at each of the holder's Bide turns, released at 0.
    pub time: u8,
    /// The damage stored (Gen 2+ `totalDamage`, Gen 1 `damage`).
    pub damage: u32,
    /// Gen 2+ `lastDamageSource`: the Pokémon whose move last damaged the holder.
    #[serde(with = "crate::snapshot::wire")]
    pub source: Option<PokemonId>,
    /// Gen 3+ `onLockMove`: the holder cannot switch; Gen 1-2's `onSemiLockMove` only keeps its
    /// other moves off the request.
    pub traps: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolatileCondition {
    Protect {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
        started_turn: u32,
        kind: ProtectKind,
    },
    Endure {
        started_turn: u32,
    },
    /// Showdown keeps the seeder's slot, not the seeder: whoever stands there is healed.
    LeechSeed {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
        #[serde(with = "crate::snapshot::wire")]
        source_position: PositionId,
    },
    Substitute {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
        hp: u16,
    },
    Flinch {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
    },
    Confusion {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
        /// Showdown `effectState.time`: 2-5 drawn, or Gen 2 Berserk Gene's 256.
        time: u16,
    },
    Taunt {
        turns: u8,
    },
    Encore {
        #[serde(with = "crate::snapshot::wire")]
        move_id: MoveId,
        turns: u8,
    },
    Disable {
        #[serde(with = "crate::snapshot::wire")]
        move_id: MoveId,
        turns: u8,
        /// Gen 1: the move slot the request greys out (Showdown gen1 `slotIndex`, pokered's
        /// `wPlayerDisabledMove` high nibble); the move itself is refused by id at BeforeMove,
        /// in whatever slot. Later generations disable by id alone (`None`).
        slot: Option<u8>,
    },
    /// Showdown `mustrecharge` (duration 2, ticked at the end of the turn; Gen 1 none): normally
    /// spent at the next BeforeMove.
    MustRecharge {
        turns: Option<u8>,
    },
    ChoiceLock {
        #[serde(with = "crate::snapshot::wire")]
        move_id: MoveId,
    },
    FollowMe,
    /// Showdown `twoturnmove` (duration 2, ticked at the end of the turn) together with the
    /// move's own condition (`fly`, `dig`): `charging` is the turn between `-prepare` and the
    /// attack; `target` is the position declared when the charge began, which the attack aims
    /// at again. `hiding` is the move's condition where it hides its user (the move's
    /// `TwoTurnSpec::invulnerable`, Showdown `isSemiInvulnerable`) with its own count: a
    /// duration handler of its own in the residual (`DurationHandler::ChargeHiding`), tied with
    /// `twoturnmove`'s.
    TwoTurnMove {
        #[serde(with = "crate::snapshot::wire")]
        move_id: MoveId,
        turns: u8,
        charging: bool,
        #[serde(with = "crate::snapshot::wire")]
        target: Option<PositionId>,
        hiding: Option<(crate::rules::SemiInvulnerableKind, u8)>,
    },
    PartiallyTrapped {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
        #[serde(with = "crate::snapshot::wire")]
        move_id: MoveId,
        turns: u8,
        /// The trapper held Binding Band as it bound (Showdown `boundDivisor`, set at `onStart`).
        banded: bool,
    },
    Ingrain,
    /// Zen Mode's (`zenmode`): no state of its own.
    ZenMode,
    /// Commander's (`commanding`, `commanded`): no state of its own — the volatile on the
    /// Dondozo names the Tatsugiri that took it in (`commanded`'s source).
    Commanding,
    Commanded,
    Stall {
        counter: u16,
        turns: u8,
    },
    AllySwitch {
        counter: u16,
        turns: u8,
    },
    /// 4096-based multiplier: 6144 (×1.5), ×1.5 again for each further Helping Hand.
    HelpingHand {
        multiplier: u32,
    },
    RagePowder,
    SubstituteBroken {
        #[serde(with = "crate::snapshot::wire")]
        by: MoveId,
    },
    ResidualDamage {
        counter: u8,
    },
    SureHit {
        turns: u8,
    },
    MagicCoat {
        turns: u8,
        /// Magic Coat itself was Prankster-boosted (`effectState.pranksterBoosted`): the move it
        /// sends back keeps the boost (DESIGN.md R-M8-88).
        prankster_boosted: bool,
    },
    Snatch {
        turns: u8,
    },
    MeFirst {
        turns: u8,
    },
    Reflect,
    LightScreen,
    Mist,
    PerishSong {
        turns: u8,
    },
    ChillyReception {
        turns: u8,
    },
    Trapped {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
    },
    Trapper,
    Octolock {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
    },
    PartialTrappingLock {
        #[serde(with = "crate::snapshot::wire")]
        target: PokemonId,
        #[serde(with = "crate::snapshot::wire")]
        move_id: MoveId,
        damage: u16,
        turns: u8,
        total: u8,
    },
    FakePartiallyTrapped {
        #[serde(with = "crate::snapshot::wire")]
        counterpart: PokemonId,
        turns: u8,
    },
    FocusPunch {
        lost_focus: bool,
        turns: u8,
    },
    BeakBlast {
        turns: u8,
    },
    ShellTrap {
        got_hit: bool,
        turns: u8,
    },
    Counter {
        turns: u8,
    },
    MirrorCoat {
        turns: u8,
    },
    DestinyBond,
    Grudge,
    Spotlight,
    Bide(BideState),
    Rampage(RampageState),
    Uproar(UproarState),
    Torment,
    Imprison,
    LockOn {
        #[serde(with = "crate::snapshot::wire")]
        target: PokemonId,
        turns: u8,
    },
    LockedOn,
    Curse,
    /// `level`: Gen 5+ the multiplier, Gen 2-4 the count of uses that landed; `turns`: Gen 5+
    /// the turns left, none for the cartridge's count.
    FuryCutter {
        level: u8,
        turns: Option<u8>,
    },
    /// `counter`: Gen 2 the hits that built it (pokecrystal `wPlayerRageCounter`), 0 elsewhere;
    /// `accuracy`: Gen 1's lock, its accuracy byte (Showdown `rage.accuracy`), none elsewhere.
    Rage {
        counter: u8,
        accuracy: Option<u8>,
    },
    GlaiveRush,
    Truant,
    Invulnerability {
        kind: crate::rules::SemiInvulnerableKind,
    },
    SkyDrop {
        #[serde(with = "crate::snapshot::wire")]
        target: PokemonId,
        turns: u8,
    },
    SkyDropped {
        #[serde(with = "crate::snapshot::wire")]
        by: PokemonId,
    },
    /// The pursuers, in the order they marked the holder.
    Pursuit {
        #[serde(with = "crate::snapshot::wire")]
        sources: [Option<PokemonId>; 3],
    },
    /// Rollout / Ice Ball: the move, the hits that computed their damage so far
    /// (`hitCount`), the doublings the power has taken (`contactHitCount`: one per hit, less
    /// the hits Gen 7 Disguise took), the turns left (`duration`: 1, back to 2 after each of the
    /// first four hits) and the position the lock aims at (`lastMoveTargetLoc`).
    Rolling {
        #[serde(with = "crate::snapshot::wire")]
        move_id: MoveId,
        hits: u8,
        doublings: u8,
        turns: u8,
        #[serde(with = "crate::snapshot::wire")]
        target: Option<PositionId>,
    },
    /// Gen 7's Rollout storage glitch (issues.md I212): a chain that ran its five hits with
    /// fewer doublings left the rest stored — `next` is the doubling count the holder's next
    /// power takes, up to the fifth hit's (4). `armed`: a Rollout / Ice Ball is using it now,
    /// one that fails clears it.
    RolloutStorage {
        next: u8,
        armed: bool,
    },
    /// Dynamax (DESIGN.md §44.5): the HP is doubled and every move runs as its Max Move; the
    /// counter reaches 3 at the end of the third turn.
    Dynamax {
        /// Showdown `effectState.turns`, counted at the residual.
        turns: u8,
        /// Showdown `baseMaxhp`: the max HP before the doubling.
        base_max_hp: u16,
    },
    MudSport,
    WaterSport,
    Attract {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
    },
    /// `from_booster`: started by Booster Energy (`fromBooster`), which the terrain or weather
    /// ending leaves be.
    QuarkDrive {
        #[serde(with = "crate::snapshot::wire")]
        best: crate::state::Stat,
        from_booster: bool,
    },
    Protosynthesis {
        #[serde(with = "crate::snapshot::wire")]
        best: crate::state::Stat,
        from_booster: bool,
    },
    Yawn {
        turns: u8,
        /// The side of the Pokémon that yawned at it (Showdown `effectState.source`): the
        /// sleep is its doing (Sleep Clause Mod).
        #[serde(with = "crate::snapshot::wire")]
        source: Option<crate::ids::SideId>,
    },
    Nightmare,
    HealBlock {
        turns: u8,
    },
    Embargo {
        turns: u8,
    },
    SaltCure,
    SyrupBomb {
        #[serde(with = "crate::snapshot::wire")]
        source: PokemonId,
        turns: u8,
    },
    ThroatChop {
        turns: u8,
    },
    TarShot,
    Unburden,
    DefenseCurl,
    AquaRing,
    PowerTrick,
    GastroAcid,
    FocusEnergy,
    Charge,
    NoRetreat,
    Minimize,
    Roost,
    Powder,
    Electrify,
    Foresight,
    MiracleEye,
    SmackDown,
    MagnetRise {
        turns: u8,
    },
    Telekinesis {
        turns: u8,
    },
    FlashFire,
    /// How many times it took off 100 kg (Showdown's gen5 `effectState.multiplier`; later
    /// `weighthg` itself goes down).
    Autotomize {
        count: u8,
    },
    /// The critical-hit stages it adds (Showdown `effectState.hasDragonType`: 2, else 1).
    DragonCheer {
        stages: u8,
    },
    /// Showdown `duration`: 2 as it starts or restarts.
    LaserFocus {
        turns: u8,
    },
    /// Showdown `duration` (Gen 5+: 2); none where it lasts until used (Gen 4, issues.md I42).
    MicleBerry {
        turns: Option<u8>,
    },
    Fling {
        #[serde(with = "crate::snapshot::wire")]
        item: ItemId,
    },
    /// `layers`, and the stages its end takes back (Showdown `effectState.def` / `.spd`: 0 or
    /// negative).
    Stockpile {
        layers: u8,
        defense: i8,
        special_defense: i8,
    },
    SparklingAria,
    Metronome {
        #[serde(with = "crate::snapshot::wire")]
        last_move: Option<MoveId>,
        consecutive: u8,
    },
}

impl VolatileCondition {
    pub const fn kind(self) -> VolatileConditionKind {
        match self {
            Self::Protect { .. } => VolatileConditionKind::Protect,
            Self::Endure { .. } => VolatileConditionKind::Endure,
            Self::LeechSeed { .. } => VolatileConditionKind::LeechSeed,
            Self::Substitute { .. } => VolatileConditionKind::Substitute,
            Self::Flinch { .. } => VolatileConditionKind::Flinch,
            Self::Confusion { .. } => VolatileConditionKind::Confusion,
            Self::Taunt { .. } => VolatileConditionKind::Taunt,
            Self::Encore { .. } => VolatileConditionKind::Encore,
            Self::Disable { .. } => VolatileConditionKind::Disable,
            Self::MustRecharge { .. } => VolatileConditionKind::MustRecharge,
            Self::ChoiceLock { .. } => VolatileConditionKind::ChoiceLock,
            Self::FollowMe => VolatileConditionKind::FollowMe,
            Self::TwoTurnMove { .. } => VolatileConditionKind::TwoTurnMove,
            Self::PartiallyTrapped { .. } => VolatileConditionKind::PartiallyTrapped,
            Self::Ingrain => VolatileConditionKind::Ingrain,
            Self::ZenMode => VolatileConditionKind::ZenMode,
            Self::Commanding => VolatileConditionKind::Commanding,
            Self::Commanded => VolatileConditionKind::Commanded,
            Self::Stall { .. } => VolatileConditionKind::Stall,
            Self::AllySwitch { .. } => VolatileConditionKind::AllySwitch,
            Self::HelpingHand { .. } => VolatileConditionKind::HelpingHand,
            Self::RagePowder => VolatileConditionKind::RagePowder,
            Self::SubstituteBroken { .. } => VolatileConditionKind::SubstituteBroken,
            Self::ResidualDamage { .. } => VolatileConditionKind::ResidualDamage,
            Self::SureHit { .. } => VolatileConditionKind::SureHit,
            Self::MagicCoat { .. } => VolatileConditionKind::MagicCoat,
            Self::Snatch { .. } => VolatileConditionKind::Snatch,
            Self::MeFirst { .. } => VolatileConditionKind::MeFirst,
            Self::Reflect => VolatileConditionKind::Reflect,
            Self::LightScreen => VolatileConditionKind::LightScreen,
            Self::Mist => VolatileConditionKind::Mist,
            Self::Attract { .. } => VolatileConditionKind::Attract,
            Self::QuarkDrive { .. } => VolatileConditionKind::QuarkDrive,
            Self::Protosynthesis { .. } => VolatileConditionKind::Protosynthesis,
            Self::MudSport => VolatileConditionKind::MudSport,
            Self::WaterSport => VolatileConditionKind::WaterSport,
            Self::Yawn { .. } => VolatileConditionKind::Yawn,
            Self::Nightmare => VolatileConditionKind::Nightmare,
            Self::HealBlock { .. } => VolatileConditionKind::HealBlock,
            Self::Embargo { .. } => VolatileConditionKind::Embargo,
            Self::SaltCure => VolatileConditionKind::SaltCure,
            Self::SyrupBomb { .. } => VolatileConditionKind::SyrupBomb,
            Self::ThroatChop { .. } => VolatileConditionKind::ThroatChop,
            Self::TarShot => VolatileConditionKind::TarShot,
            Self::Unburden => VolatileConditionKind::Unburden,
            Self::DefenseCurl => VolatileConditionKind::DefenseCurl,
            Self::AquaRing => VolatileConditionKind::AquaRing,
            Self::PowerTrick => VolatileConditionKind::PowerTrick,
            Self::GastroAcid => VolatileConditionKind::GastroAcid,
            Self::FocusEnergy => VolatileConditionKind::FocusEnergy,
            Self::Charge => VolatileConditionKind::Charge,
            Self::NoRetreat => VolatileConditionKind::NoRetreat,
            Self::Minimize => VolatileConditionKind::Minimize,
            Self::Roost => VolatileConditionKind::Roost,
            Self::Powder => VolatileConditionKind::Powder,
            Self::Electrify => VolatileConditionKind::Electrify,
            Self::Foresight => VolatileConditionKind::Foresight,
            Self::MiracleEye => VolatileConditionKind::MiracleEye,
            Self::SmackDown => VolatileConditionKind::SmackDown,
            Self::MagnetRise { .. } => VolatileConditionKind::MagnetRise,
            Self::Telekinesis { .. } => VolatileConditionKind::Telekinesis,
            Self::FlashFire => VolatileConditionKind::FlashFire,
            Self::Autotomize { .. } => VolatileConditionKind::Autotomize,
            Self::DragonCheer { .. } => VolatileConditionKind::DragonCheer,
            Self::LaserFocus { .. } => VolatileConditionKind::LaserFocus,
            Self::MicleBerry { .. } => VolatileConditionKind::MicleBerry,
            Self::Fling { .. } => VolatileConditionKind::Fling,
            Self::Stockpile { .. } => VolatileConditionKind::Stockpile,
            Self::SparklingAria => VolatileConditionKind::SparklingAria,
            Self::Metronome { .. } => VolatileConditionKind::Metronome,
            Self::PerishSong { .. } => VolatileConditionKind::PerishSong,
            Self::ChillyReception { .. } => VolatileConditionKind::ChillyReception,
            Self::Trapped { .. } => VolatileConditionKind::Trapped,
            Self::Trapper => VolatileConditionKind::Trapper,
            Self::Octolock { .. } => VolatileConditionKind::Octolock,
            Self::PartialTrappingLock { .. } => VolatileConditionKind::PartialTrappingLock,
            Self::FakePartiallyTrapped { .. } => VolatileConditionKind::FakePartiallyTrapped,
            Self::FocusPunch { .. } => VolatileConditionKind::FocusPunch,
            Self::BeakBlast { .. } => VolatileConditionKind::BeakBlast,
            Self::ShellTrap { .. } => VolatileConditionKind::ShellTrap,
            Self::Counter { .. } => VolatileConditionKind::Counter,
            Self::MirrorCoat { .. } => VolatileConditionKind::MirrorCoat,
            Self::DestinyBond => VolatileConditionKind::DestinyBond,
            Self::Grudge => VolatileConditionKind::Grudge,
            Self::Spotlight => VolatileConditionKind::Spotlight,
            Self::Bide(_) => VolatileConditionKind::Bide,
            Self::Pursuit { .. } => VolatileConditionKind::Pursuit,
            Self::Rolling { .. } => VolatileConditionKind::Rolling,
            Self::RolloutStorage { .. } => VolatileConditionKind::RolloutStorage,
            Self::Dynamax { .. } => VolatileConditionKind::Dynamax,
            Self::Rampage(_) => VolatileConditionKind::Rampage,
            Self::Uproar(_) => VolatileConditionKind::Uproar,
            Self::Torment => VolatileConditionKind::Torment,
            Self::Imprison => VolatileConditionKind::Imprison,
            Self::LockOn { .. } => VolatileConditionKind::LockOn,
            Self::LockedOn => VolatileConditionKind::LockedOn,
            Self::Curse => VolatileConditionKind::Curse,
            Self::FuryCutter { .. } => VolatileConditionKind::FuryCutter,
            Self::Rage { .. } => VolatileConditionKind::Rage,
            Self::GlaiveRush => VolatileConditionKind::GlaiveRush,
            Self::Truant => VolatileConditionKind::Truant,
            Self::Invulnerability { .. } => VolatileConditionKind::Invulnerability,
            Self::SkyDrop { .. } => VolatileConditionKind::SkyDrop,
            Self::SkyDropped { .. } => VolatileConditionKind::SkyDropped,
        }
    }
}

/// A set of volatile kinds (Baton Pass's hand-over, DESIGN.md R-M10-26): one bit a kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct VolatileSet([u32; 4]);

/// The doublings a Rollout storage holds up to: the fifth hit's (×16).
pub const ROLLOUT_STORED_UP_TO: u8 = 4;

// Every kind has its bit (the last one declared is the highest).
const _: () = assert!((VolatileConditionKind::SkyDropped as usize) < 128);

impl VolatileSet {
    /// Every kind.
    pub const ALL: Self = Self([u32::MAX; 4]);

    pub fn insert(&mut self, kind: VolatileConditionKind) {
        let bit = kind as usize;
        self.0[bit / 32] |= 1 << (bit % 32);
    }

    pub fn remove(&mut self, kind: VolatileConditionKind) {
        let bit = kind as usize;
        self.0[bit / 32] &= !(1 << (bit % 32));
    }

    pub fn contains(&self, kind: VolatileConditionKind) -> bool {
        let bit = kind as usize;
        self.0[bit / 32] & (1 << (bit % 32)) != 0
    }
}

impl FromIterator<VolatileConditionKind> for VolatileSet {
    fn from_iter<I: IntoIterator<Item = VolatileConditionKind>>(kinds: I) -> Self {
        let mut set = Self::default();
        for kind in kinds {
            set.insert(kind);
        }
        set
    }
}
