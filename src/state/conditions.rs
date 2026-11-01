use crate::ids::{ItemId, MoveId, PokemonId, PositionId};

/// Which protecting move stands behind a `Protect` volatile (Showdown: one volatile each,
/// never two at once): whether it stops status moves, and what a contact move that it stops
/// costs the attacker (`engine::protect_punishment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RampageState {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UproarState {
    /// The frame's `uproar`: the turns left, the current one included (Gen 5+ Showdown's
    /// `duration`; Gen 3-4 the cartridge's count), counted down at the holder's turn-end step.
    pub turns: u8,
    /// Gen 3-4: the move failed this turn (`UproarRules::failing_blocks`): the turn-end step
    /// ends it.
    pub failed: bool,
}

/// What Bide keeps (Showdown's `bide` effect state, DESIGN.md §15.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BideState {
    /// Gen 2+ `duration`, ticked at the end of every turn: the release comes at the holder's
    /// move on the turn it reads 1. Gen 1's has none.
    pub turns: Option<u8>,
    /// Gen 1 `time`: counted down at each of the holder's Bide turns, released at 0.
    pub time: u8,
    /// The damage stored (Gen 2+ `totalDamage`, Gen 1 `damage`).
    pub damage: u32,
    /// Gen 2+ `lastDamageSource`: the Pokémon whose move last damaged the holder.
    pub source: Option<PokemonId>,
    /// Gen 3+ `onLockMove`: the holder cannot switch; Gen 1-2's `onSemiLockMove` only keeps its
    /// other moves off the request.
    pub traps: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolatileCondition {
    Protect {
        source: PokemonId,
        started_turn: u32,
        kind: ProtectKind,
    },
    Endure {
        started_turn: u32,
    },
    /// Showdown keeps the seeder's slot, not the seeder: whoever stands there is healed.
    LeechSeed {
        source: PokemonId,
        source_position: PositionId,
    },
    Substitute {
        source: PokemonId,
        hp: u16,
    },
    Flinch {
        source: PokemonId,
    },
    Confusion {
        source: PokemonId,
        /// Showdown `effectState.time`: 2-5 drawn, or Gen 2 Berserk Gene's 256.
        time: u16,
    },
    Taunt {
        turns: u8,
    },
    Encore {
        move_id: MoveId,
        turns: u8,
    },
    Disable {
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
        move_id: MoveId,
        turns: u8,
        charging: bool,
        target: Option<PositionId>,
        hiding: Option<(crate::rules::SemiInvulnerableKind, u8)>,
    },
    PartiallyTrapped {
        source: PokemonId,
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
        source: PokemonId,
    },
    Trapper,
    Octolock {
        source: PokemonId,
    },
    PartialTrappingLock {
        target: PokemonId,
        move_id: MoveId,
        damage: u16,
        turns: u8,
        total: u8,
    },
    FakePartiallyTrapped {
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
        target: PokemonId,
        turns: u8,
    },
    SkyDropped {
        by: PokemonId,
    },
    /// The pursuers, in the order they marked the holder.
    Pursuit {
        sources: [Option<PokemonId>; 3],
    },
    /// Rollout / Ice Ball: the move, the hits that computed their damage so far
    /// (`hitCount`), the doublings the power has taken (`contactHitCount`: one per hit, less
    /// the hits Gen 7 Disguise took), the turns left (`duration`: 1, back to 2 after each of the
    /// first four hits) and the position the lock aims at (`lastMoveTargetLoc`).
    Rolling {
        move_id: MoveId,
        hits: u8,
        doublings: u8,
        turns: u8,
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
        source: PokemonId,
    },
    /// `from_booster`: started by Booster Energy (`fromBooster`), which the terrain or weather
    /// ending leaves be.
    QuarkDrive {
        best: crate::state::Stat,
        from_booster: bool,
    },
    Protosynthesis {
        best: crate::state::Stat,
        from_booster: bool,
    },
    Yawn {
        turns: u8,
        /// The side of the Pokémon that yawned at it (Showdown `effectState.source`): the
        /// sleep is its doing (Sleep Clause Mod).
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VolatileState {
    entries: Vec<VolatileCondition>,
}

impl VolatileState {
    pub fn entries(&self) -> &[VolatileCondition] {
        &self.entries
    }

    pub fn contains(&self, kind: VolatileConditionKind) -> bool {
        self.entries.iter().any(|entry| entry.kind() == kind)
    }

    pub fn set_protect(&mut self, source: PokemonId, turn: u32, kind: ProtectKind) {
        self.remove(VolatileConditionKind::Protect);
        self.entries.push(VolatileCondition::Protect {
            source,
            started_turn: turn,
            kind,
        });
    }

    /// The protecting move behind the `Protect` volatile, if any.
    pub fn protect_kind(&self) -> Option<ProtectKind> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Protect { kind, .. } => Some(*kind),
            _ => None,
        })
    }

    /// Dynamax (DESIGN.md §44.5): the volatile with its counter and the max HP it doubled.
    pub fn set_dynamax(&mut self, base_max_hp: u16) {
        self.remove(VolatileConditionKind::Dynamax);
        self.entries.push(VolatileCondition::Dynamax {
            turns: 0,
            base_max_hp,
        });
    }

    /// The Dynamax volatile's `effectState.turns`, counted at the residual.
    pub fn dynamax_turns(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Dynamax { turns, .. } => Some(*turns),
            _ => None,
        })
    }

    /// The max HP before the doubling (`baseMaxhp`).
    pub fn dynamax_base_max_hp(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Dynamax { base_max_hp, .. } => Some(*base_max_hp),
            _ => None,
        })
    }

    /// A permanent forme change's new `baseMaxhp` under the Dynamax (`updateMaxHp`).
    pub fn set_dynamax_base_max_hp(&mut self, value: u16) {
        for entry in &mut self.entries {
            if let VolatileCondition::Dynamax { base_max_hp, .. } = entry {
                *base_max_hp = value;
            }
        }
    }

    /// The counter's next value; `None` when the volatile is gone.
    pub fn count_a_dynamax_turn(&mut self) -> Option<u8> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::Dynamax { turns, .. } => {
                *turns = turns.saturating_add(1);
                Some(*turns)
            }
            _ => None,
        })
    }

    pub fn set_endure(&mut self, turn: u32) {
        self.remove(VolatileConditionKind::Endure);
        self.entries
            .push(VolatileCondition::Endure { started_turn: turn });
    }

    pub fn set_taunt(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::Taunt) {
            return false;
        }
        self.entries.push(VolatileCondition::Taunt { turns });
        true
    }

    pub fn set_encore(&mut self, move_id: MoveId, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::Encore) {
            return false;
        }
        self.entries
            .push(VolatileCondition::Encore { move_id, turns });
        true
    }

    pub fn set_disable(&mut self, move_id: MoveId, turns: u8) -> bool {
        self.set_disable_in(move_id, turns, None)
    }

    /// `set_disable`, with Gen 1's disabled slot.
    pub fn set_disable_in(&mut self, move_id: MoveId, turns: u8, slot: Option<u8>) -> bool {
        if self.contains(VolatileConditionKind::Disable) {
            return false;
        }
        self.entries.push(VolatileCondition::Disable {
            move_id,
            turns,
            slot,
        });
        true
    }

    pub fn mark_substitute_broken(&mut self, by: MoveId) {
        self.remove(VolatileConditionKind::SubstituteBroken);
        self.entries
            .push(VolatileCondition::SubstituteBroken { by });
    }

    /// Gen 2 Toxic (`residualdmg` `onStart`): the counter starts over at 0, the volatile is
    /// added if missing. Returns whether it was added.
    pub fn start_residual_damage(&mut self) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::ResidualDamage { counter } = entry {
                *counter = 0;
                return false;
            }
        }
        self.entries
            .push(VolatileCondition::ResidualDamage { counter: 0 });
        true
    }

    pub fn residual_damage_counter(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::ResidualDamage { counter } => Some(*counter),
            _ => None,
        })
    }

    /// One after-action tick of a statused holder (`residualdmg` `onAfterMoveSelf`).
    pub fn advance_residual_damage(&mut self) {
        for entry in &mut self.entries {
            if let VolatileCondition::ResidualDamage { counter } = entry {
                *counter = counter.saturating_add(1);
            }
        }
    }

    pub fn substitute_broken_by(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::SubstituteBroken { by } => Some(*by),
            _ => None,
        })
    }

    pub fn set_rage_powder(&mut self) -> bool {
        if self.contains(VolatileConditionKind::RagePowder) {
            return false;
        }
        self.entries.push(VolatileCondition::RagePowder);
        true
    }

    pub fn set_follow_me(&mut self) -> bool {
        if self.contains(VolatileConditionKind::FollowMe) {
            return false;
        }
        self.entries.push(VolatileCondition::FollowMe);
        true
    }

    pub fn set_must_recharge(&mut self, turns: Option<u8>) -> bool {
        if self.contains(VolatileConditionKind::MustRecharge) {
            return false;
        }
        self.entries.push(VolatileCondition::MustRecharge { turns });
        true
    }

    /// Locks (or re-locks onto the same move); a different move never replaces a live lock.
    pub fn set_choice_lock(&mut self, move_id: MoveId) -> bool {
        if self.contains(VolatileConditionKind::ChoiceLock) {
            return false;
        }
        self.entries.push(VolatileCondition::ChoiceLock { move_id });
        true
    }

    /// Starts a two-turn move's charge, out of reach where `hides` says while it lasts; a
    /// Pokémon already in one is left alone.
    pub fn set_two_turn_move(
        &mut self,
        move_id: MoveId,
        turns: u8,
        target: Option<PositionId>,
        hides: Option<crate::rules::SemiInvulnerableKind>,
    ) -> bool {
        if self.contains(VolatileConditionKind::TwoTurnMove) {
            return false;
        }
        self.entries.push(VolatileCondition::TwoTurnMove {
            move_id,
            turns,
            charging: true,
            target,
            hiding: hides.map(|kind| (kind, turns)),
        });
        true
    }

    /// Gen 1 Fly / Dig's hiding starts (`addVolatile('invulnerability')`); false when the
    /// Pokémon is hidden already.
    pub fn set_invulnerability(&mut self, kind: crate::rules::SemiInvulnerableKind) -> bool {
        if self.contains(VolatileConditionKind::Invulnerability) {
            return false;
        }
        self.entries
            .push(VolatileCondition::Invulnerability { kind });
        true
    }

    /// Sky Drop: its user carries `target` up; false when it carries someone already.
    pub fn set_sky_drop(&mut self, target: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::SkyDrop) {
            return false;
        }
        self.entries
            .push(VolatileCondition::SkyDrop { target, turns: 2 });
        true
    }

    /// The Pokémon this one carries up with Sky Drop, if it does.
    pub fn sky_drop_target(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::SkyDrop { target, .. } => Some(*target),
            _ => None,
        })
    }

    /// Sky Drop carries this Pokémon up, by `by`.
    pub fn set_sky_dropped(&mut self, by: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::SkyDropped) {
            return false;
        }
        self.entries.push(VolatileCondition::SkyDropped { by });
        true
    }

    /// The Pokémon carrying this one up with Sky Drop, if any.
    pub fn sky_dropped_by(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::SkyDropped { by } => Some(*by),
            _ => None,
        })
    }

    /// Where Gen 1 Fly / Dig's hiding keeps the Pokémon, if it does.
    pub fn invulnerability(&self) -> Option<crate::rules::SemiInvulnerableKind> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Invulnerability { kind } => Some(*kind),
            _ => None,
        })
    }

    /// The move a two-turn move locks the Pokémon into (charging or already released).
    /// Rollout / Ice Ball as it starts (`addVolatile`): no hit yet, one turn; false when a chain
    /// is already going.
    pub fn start_rolling(&mut self, move_id: MoveId, target: Option<PositionId>) -> bool {
        if self.contains(VolatileConditionKind::Rolling) {
            return false;
        }
        self.entries.push(VolatileCondition::Rolling {
            move_id,
            hits: 0,
            doublings: 0,
            turns: 1,
            target,
        });
        true
    }

    /// The doublings the Rollout / Ice Ball chain's power has taken (`contactHitCount`).
    pub fn rolling_doublings(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rolling { doublings, .. } => Some(*doublings),
            _ => None,
        })
    }

    /// Gen 7 Disguise takes a hit of `move_id`'s chain (`contactHitCount--`): the hit counts,
    /// its doubling does not.
    pub fn delay_rolling(&mut self, move_id: MoveId) {
        for entry in &mut self.entries {
            if let VolatileCondition::Rolling {
                move_id: rolling,
                doublings,
                ..
            } = entry
                && *rolling == move_id
            {
                *doublings = doublings.saturating_sub(1);
            }
        }
    }

    /// The Rollout storage (`next`, `armed`), if the holder has one.
    pub fn rollout_storage(&self) -> Option<(u8, bool)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::RolloutStorage { next, armed } => Some((*next, *armed)),
            _ => None,
        })
    }

    /// A chain over with `doublings` short of its hits: the rest stored, from `doublings` on.
    pub fn store_rollout(&mut self, doublings: u8) {
        self.remove(VolatileConditionKind::RolloutStorage);
        self.entries.push(VolatileCondition::RolloutStorage {
            next: doublings,
            armed: false,
        });
    }

    /// A Rollout / Ice Ball starts on the storage: no chain, and a failure clears it.
    pub fn arm_rollout_storage(&mut self) {
        for entry in &mut self.entries {
            if let VolatileCondition::RolloutStorage { armed, .. } = entry {
                *armed = true;
            }
        }
    }

    /// A power computed on the storage: the doublings it takes, and whether that was the last
    /// stored one (the fifth hit's) — else the next waits for the power after it.
    pub fn take_rollout_storage(&mut self) -> Option<(u8, bool)> {
        let (doublings, _) = self.rollout_storage()?;
        for entry in &mut self.entries {
            if let VolatileCondition::RolloutStorage { next, armed } = entry {
                *next = next.saturating_add(1);
                *armed = false;
            }
        }
        Some((doublings, doublings >= ROLLOUT_STORED_UP_TO))
    }

    /// The Rollout / Ice Ball chain: the move, its hits so far and the position it aims at.
    pub fn rolling(&self) -> Option<(MoveId, u8, Option<PositionId>)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rolling {
                move_id,
                hits,
                target,
                ..
            } => Some((*move_id, *hits, *target)),
            _ => None,
        })
    }

    /// A Rollout / Ice Ball hit computed its damage (`basePowerCallback`): one more hit and
    /// doubling, and before the fifth hit the chain lasts another turn (`duration = 2`).
    pub fn advance_rolling(&mut self, max_hits: u8) {
        for entry in &mut self.entries {
            if let VolatileCondition::Rolling {
                hits,
                doublings,
                turns,
                ..
            } = entry
            {
                *hits = hits.saturating_add(1);
                *doublings = doublings.saturating_add(1);
                if *hits < max_hits {
                    *turns = 2;
                }
            }
        }
    }

    pub fn two_turn_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove { move_id, .. } => Some(*move_id),
            _ => None,
        })
    }

    /// Where the charge the Pokémon is in keeps it out of reach, if it does.
    pub fn charging_hides(&self) -> Option<crate::rules::SemiInvulnerableKind> {
        self.charge_hiding().map(|(kind, _)| kind)
    }

    /// The hiding move's own condition while the Pokémon charges (Showdown `fly`, `dig` …):
    /// where it hides and its count.
    pub fn charge_hiding(&self) -> Option<(crate::rules::SemiInvulnerableKind, u8)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove {
                charging: true,
                hiding,
                ..
            } => *hiding,
            _ => None,
        })
    }

    /// One residual of the hiding move's own condition: `Some(true)` when it ran out (the user
    /// is out of hiding), `None` without one.
    pub fn count_down_charge_hiding(&mut self) -> Option<bool> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove {
                charging: true,
                hiding,
                ..
            } => {
                let (_, turns) = hiding.as_mut()?;
                *turns = turns.saturating_sub(1);
                let ended = *turns == 0;
                if ended {
                    *hiding = None;
                }
                Some(ended)
            }
            _ => None,
        })
    }

    /// The move the Pokémon is charging right now (semi-invulnerable if the move says so).
    pub fn charging_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove {
                move_id,
                charging: true,
                ..
            } => Some(*move_id),
            _ => None,
        })
    }

    pub fn two_turn_target(&self) -> Option<PositionId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::TwoTurnMove { target, .. } => *target,
            _ => None,
        })
    }

    /// The charge is over: the attack goes out this action (Showdown removes the move's own
    /// condition; `twoturnmove` stays until the end of the turn).
    pub fn release_two_turn_move(&mut self) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::TwoTurnMove { charging, .. } = entry
                && *charging
            {
                *charging = false;
                return true;
            }
        }
        false
    }

    pub fn set_partially_trapped(&mut self, source: PokemonId, move_id: MoveId, turns: u8) -> bool {
        self.set_partially_trapped_banded(source, move_id, turns, false)
    }

    /// `set_partially_trapped`, with whether the trapper held Binding Band.
    pub fn set_partially_trapped_banded(
        &mut self,
        source: PokemonId,
        move_id: MoveId,
        turns: u8,
        banded: bool,
    ) -> bool {
        if self.contains(VolatileConditionKind::PartiallyTrapped) {
            return false;
        }
        self.entries.push(VolatileCondition::PartiallyTrapped {
            source,
            move_id,
            turns,
            banded,
        });
        true
    }

    /// Whether the trapper held Binding Band as it bound this Pokémon.
    pub fn partially_trapped_banded(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry,
                VolatileCondition::PartiallyTrapped { banded: true, .. }
            )
        })
    }

    /// The trapper and the move that bound this Pokémon.
    pub fn partially_trapped_by(&self) -> Option<(PokemonId, MoveId)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::PartiallyTrapped {
                source, move_id, ..
            } => Some((*source, *move_id)),
            _ => None,
        })
    }

    /// Showdown `helpinghand` onStart (×1.5) / onRestart (`multiplier *= 1.5`).
    pub fn add_helping_hand(&mut self) {
        for entry in &mut self.entries {
            if let VolatileCondition::HelpingHand { multiplier } = entry {
                *multiplier = *multiplier * 3 / 2;
                return;
            }
        }
        self.entries
            .push(VolatileCondition::HelpingHand { multiplier: 6144 });
    }

    pub fn helping_hand_multiplier(&self) -> Option<u32> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::HelpingHand { multiplier } => Some(*multiplier),
            _ => None,
        })
    }

    pub fn set_ingrain(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Ingrain) {
            return false;
        }
        self.entries.push(VolatileCondition::Ingrain);
        true
    }

    /// Commander's two volatiles (`addVolatile('commanding')` / `('commanded')`): false when
    /// the holder already has one.
    pub fn set_commanding(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Commanding) {
            return false;
        }
        self.entries.push(VolatileCondition::Commanding);
        true
    }

    pub fn set_commanded(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Commanded) {
            return false;
        }
        self.entries.push(VolatileCondition::Commanded);
        true
    }

    /// Zen Mode's volatile (`addVolatile('zenmode')`): false when the holder already has it
    /// (Showdown's `addVolatile` then runs no `onStart`, so the forme stays as it is).
    pub fn set_zen_mode(&mut self) -> bool {
        if self.contains(VolatileConditionKind::ZenMode) {
            return false;
        }
        self.entries.push(VolatileCondition::ZenMode);
        true
    }

    /// Quark Drive's or Protosynthesis's volatile (`kind`) with the stat it raises, started by
    /// Booster Energy or not; false when the holder already has it (`addVolatile` fails: the
    /// stat stays the one picked first) or `kind` is neither.
    pub fn set_paradox_boost(
        &mut self,
        kind: VolatileConditionKind,
        best: crate::state::Stat,
        from_booster: bool,
    ) -> bool {
        let entry = match kind {
            VolatileConditionKind::QuarkDrive => {
                VolatileCondition::QuarkDrive { best, from_booster }
            }
            VolatileConditionKind::Protosynthesis => {
                VolatileCondition::Protosynthesis { best, from_booster }
            }
            _ => return false,
        };
        if self.contains(kind) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// The stat Quark Drive or Protosynthesis raises, with the volatile, while it does.
    pub fn paradox_boost(&self) -> Option<(VolatileConditionKind, crate::state::Stat)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::QuarkDrive { best, .. } => {
                Some((VolatileConditionKind::QuarkDrive, *best))
            }
            VolatileCondition::Protosynthesis { best, .. } => {
                Some((VolatileConditionKind::Protosynthesis, *best))
            }
            _ => None,
        })
    }

    /// Whether the Quark Drive or Protosynthesis volatile came from Booster Energy.
    pub fn paradox_boost_from_booster(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry,
                VolatileCondition::QuarkDrive {
                    from_booster: true,
                    ..
                } | VolatileCondition::Protosynthesis {
                    from_booster: true,
                    ..
                }
            )
        })
    }

    /// Gen 1 Reflect / Light Screen, Gen 1-2 Mist, Gen 3-5 Mud Sport / Water Sport on the
    /// user; false when it already has it (Gen 1's `onTryHit`, `addVolatile` failing) or `kind`
    /// is none of them.
    /// The Metronome item's count, starting at none; false when the holder already has one
    /// (`addVolatile` fails and the count goes on).
    pub fn start_metronome(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Metronome) {
            return false;
        }
        self.entries.push(VolatileCondition::Metronome {
            last_move: None,
            consecutive: 0,
        });
        true
    }

    /// The Metronome item's count: the move it counts and how many times in a row.
    pub fn metronome(&self) -> Option<(Option<MoveId>, u8)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Metronome {
                last_move,
                consecutive,
            } => Some((*last_move, *consecutive)),
            _ => None,
        })
    }

    /// Sets the Metronome item's count, where there is one.
    pub fn set_metronome(&mut self, move_id: Option<MoveId>, count: u8) {
        for entry in &mut self.entries {
            if let VolatileCondition::Metronome {
                last_move,
                consecutive,
            } = entry
            {
                *last_move = move_id;
                *consecutive = count;
            }
        }
    }

    /// Gen 1 Reflect / Light Screen, Gen 1-2 Mist on the user; false when it already has it
    /// (Gen 1's `onTryHit`, `addVolatile` failing) or `kind` is none of them.
    pub fn set_user_guard(&mut self, kind: VolatileConditionKind) -> bool {
        let entry = match kind {
            VolatileConditionKind::Reflect => VolatileCondition::Reflect,
            VolatileConditionKind::LightScreen => VolatileCondition::LightScreen,
            VolatileConditionKind::Mist => VolatileCondition::Mist,
            VolatileConditionKind::MudSport => VolatileCondition::MudSport,
            VolatileConditionKind::WaterSport => VolatileCondition::WaterSport,
            _ => return false,
        };
        if self.contains(kind) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// Yawning for `turns`, at the doing of `source`'s side; false when already yawning.
    pub fn set_yawn(&mut self, turns: u8, source: Option<crate::ids::SideId>) -> bool {
        if self.contains(VolatileConditionKind::Yawn) {
            return false;
        }
        self.entries.push(VolatileCondition::Yawn { turns, source });
        true
    }

    /// The side whose Pokémon yawned at the holder.
    pub fn yawn_source(&self) -> Option<crate::ids::SideId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Yawn { source, .. } => *source,
            _ => None,
        })
    }

    /// Nightmare; false when already having one.
    pub fn set_nightmare(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Nightmare) {
            return false;
        }
        self.entries.push(VolatileCondition::Nightmare);
        true
    }

    /// Magnet Rise for `turns`; false when already there.
    pub fn set_magnet_rise(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::MagnetRise) {
            return false;
        }
        self.entries.push(VolatileCondition::MagnetRise { turns });
        true
    }

    /// Telekinesis for `turns`; false when already there.
    pub fn set_telekinesis(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::Telekinesis) {
            return false;
        }
        self.entries.push(VolatileCondition::Telekinesis { turns });
        true
    }

    /// Embargo for `turns`; false when already there.
    pub fn set_embargo(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::Embargo) {
            return false;
        }
        self.entries.push(VolatileCondition::Embargo { turns });
        true
    }

    /// Heal Block for `turns`; false when already blocked (a second one does not renew it).
    pub fn set_heal_block(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::HealBlock) {
            return false;
        }
        self.entries.push(VolatileCondition::HealBlock { turns });
        true
    }

    /// A volatile without state of its own (Salt Cure, Tar Shot, Defense Curl, Aqua Ring,
    /// Gastro Acid, Sparkling Aria's mark); false
    /// when already there.
    pub fn set_marker(&mut self, kind: VolatileConditionKind) -> bool {
        let entry = match kind {
            VolatileConditionKind::SaltCure => VolatileCondition::SaltCure,
            VolatileConditionKind::TarShot => VolatileCondition::TarShot,
            VolatileConditionKind::Unburden => VolatileCondition::Unburden,
            VolatileConditionKind::DefenseCurl => VolatileCondition::DefenseCurl,
            VolatileConditionKind::AquaRing => VolatileCondition::AquaRing,
            VolatileConditionKind::PowerTrick => VolatileCondition::PowerTrick,
            VolatileConditionKind::GastroAcid => VolatileCondition::GastroAcid,
            VolatileConditionKind::FocusEnergy => VolatileCondition::FocusEnergy,
            VolatileConditionKind::Charge => VolatileCondition::Charge,
            VolatileConditionKind::NoRetreat => VolatileCondition::NoRetreat,
            VolatileConditionKind::Minimize => VolatileCondition::Minimize,
            VolatileConditionKind::Roost => VolatileCondition::Roost,
            VolatileConditionKind::ZenMode => VolatileCondition::ZenMode,
            VolatileConditionKind::Commanding => VolatileCondition::Commanding,
            VolatileConditionKind::Commanded => VolatileCondition::Commanded,
            VolatileConditionKind::Powder => VolatileCondition::Powder,
            VolatileConditionKind::Electrify => VolatileCondition::Electrify,
            VolatileConditionKind::Foresight => VolatileCondition::Foresight,
            VolatileConditionKind::MiracleEye => VolatileCondition::MiracleEye,
            VolatileConditionKind::SmackDown => VolatileCondition::SmackDown,
            VolatileConditionKind::FlashFire => VolatileCondition::FlashFire,
            VolatileConditionKind::SparklingAria => VolatileCondition::SparklingAria,
            VolatileConditionKind::DestinyBond => VolatileCondition::DestinyBond,
            VolatileConditionKind::Grudge => VolatileCondition::Grudge,
            VolatileConditionKind::Spotlight => VolatileCondition::Spotlight,
            VolatileConditionKind::Torment => VolatileCondition::Torment,
            VolatileConditionKind::Imprison => VolatileCondition::Imprison,
            VolatileConditionKind::LockedOn => VolatileCondition::LockedOn,
            VolatileConditionKind::Curse => VolatileCondition::Curse,
            VolatileConditionKind::GlaiveRush => VolatileCondition::GlaiveRush,
            VolatileConditionKind::Truant => VolatileCondition::Truant,
            _ => return false,
        };
        if self.contains(kind) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// Bide as it starts (`addVolatile('bide')`): false when already there.
    pub fn start_bide(&mut self, turns: Option<u8>, time: u8, traps: bool) -> bool {
        if self.contains(VolatileConditionKind::Bide) {
            return false;
        }
        self.entries.push(VolatileCondition::Bide(BideState {
            turns,
            time,
            damage: 0,
            source: None,
            traps,
        }));
        true
    }

    /// Pursuit's mark (`addVolatile('pursuit')`, then `sources.push`): `source` joins the
    /// holder's pursuers.
    pub fn mark_pursuit(&mut self, source: PokemonId) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::Pursuit { sources } = entry {
                return match sources.iter_mut().find(|slot| slot.is_none()) {
                    Some(slot) => {
                        *slot = Some(source);
                        true
                    }
                    None => false,
                };
            }
        }
        self.entries.push(VolatileCondition::Pursuit {
            sources: [Some(source), None, None],
        });
        true
    }

    /// The holder's pursuers, in marking order.
    pub fn pursuers(&self) -> Vec<PokemonId> {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                VolatileCondition::Pursuit { sources } => {
                    Some(sources.iter().flatten().copied().collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Thrash and its kind (`lockedmove`).
    pub fn rampage(&self) -> Option<&RampageState> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rampage(rampage) => Some(rampage),
            _ => None,
        })
    }

    pub fn rampage_mut(&mut self) -> Option<&mut RampageState> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::Rampage(rampage) => Some(rampage),
            _ => None,
        })
    }

    /// Starts the lock; false when the Pokémon already has one.
    pub fn start_rampage(&mut self, rampage: RampageState) -> bool {
        if self.contains(VolatileConditionKind::Rampage) {
            return false;
        }
        self.entries.push(VolatileCondition::Rampage(rampage));
        true
    }

    /// Fury Cutter's chain: its level (Gen 5+ the multiplier, Gen 2-4 the count) and the turns
    /// left (Gen 5+).
    pub fn fury_cutter(&self) -> Option<(u8, Option<u8>)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::FuryCutter { level, turns } => Some((*level, *turns)),
            _ => None,
        })
    }

    /// Starts Fury Cutter's chain at `level`, or sets the one there; true when it started.
    pub fn set_fury_cutter(&mut self, level: u8, turns: Option<u8>) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::FuryCutter {
                level: current,
                turns: left,
            } = entry
            {
                *current = level;
                *left = turns;
                return false;
            }
        }
        self.entries
            .push(VolatileCondition::FuryCutter { level, turns });
        true
    }

    /// Rage's count (Gen 2; 0 elsewhere), while the Pokémon is raging.
    pub fn rage(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rage { counter, .. } => Some(*counter),
            _ => None,
        })
    }

    /// The Pokémon starts raging (its count at 0) — Gen 1 (`locks`) locked into Rage, its
    /// accuracy byte at 255; false when it was already.
    pub fn start_rage(&mut self, locks: bool) -> bool {
        if self.rage().is_some() {
            return false;
        }
        self.entries.push(VolatileCondition::Rage {
            counter: 0,
            accuracy: locks.then_some(255),
        });
        true
    }

    /// Gen 2: a hit builds the raging Pokémon's count, up to 255 (pokecrystal `inc a; ret z`);
    /// false when it is not raging.
    pub fn count_rage(&mut self) -> bool {
        self.entries.iter_mut().any(|entry| match entry {
            VolatileCondition::Rage { counter, .. } => {
                *counter = counter.saturating_add(1);
                true
            }
            _ => false,
        })
    }

    /// Gen 1 Rage's lock: its accuracy byte.
    pub fn rage_lock(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Rage { accuracy, .. } => *accuracy,
            _ => None,
        })
    }

    /// Gen 1 Rage's lock: the byte this use's check ended with.
    pub fn set_rage_lock(&mut self, byte: u8) {
        for entry in &mut self.entries {
            if let VolatileCondition::Rage {
                accuracy: Some(accuracy),
                ..
            } = entry
            {
                *accuracy = byte;
            }
        }
    }

    /// Lock-On on its user: the target and the turns left (Gen 3+).
    pub fn lock_on(&self) -> Option<(PokemonId, u8)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::LockOn { target, turns } => Some((*target, *turns)),
            _ => None,
        })
    }

    /// Sets Lock-On on its user anew, the one there replaced.
    pub fn set_lock_on(&mut self, target: PokemonId, turns: u8) {
        self.remove(VolatileConditionKind::LockOn);
        self.entries
            .push(VolatileCondition::LockOn { target, turns });
    }

    /// A Lock-On on `from` follows it to `to` (Gen 3-4 Baton Pass), its turns kept or set.
    pub fn retarget_lock_on(&mut self, from: PokemonId, to: PokemonId, turns: Option<u8>) {
        for entry in &mut self.entries {
            if let VolatileCondition::LockOn {
                target,
                turns: left,
            } = entry
                && *target == from
            {
                *target = to;
                if let Some(turns) = turns {
                    *left = turns;
                }
            }
        }
    }

    /// Uproar.
    pub fn uproar(&self) -> Option<&UproarState> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Uproar(uproar) => Some(uproar),
            _ => None,
        })
    }

    pub fn uproar_mut(&mut self) -> Option<&mut UproarState> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::Uproar(uproar) => Some(uproar),
            _ => None,
        })
    }

    /// Starts the uproar; false when the Pokémon already makes one.
    pub fn start_uproar(&mut self, uproar: UproarState) -> bool {
        if self.contains(VolatileConditionKind::Uproar) {
            return false;
        }
        self.entries.push(VolatileCondition::Uproar(uproar));
        true
    }

    pub fn bide(&self) -> Option<&BideState> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Bide(bide) => Some(bide),
            _ => None,
        })
    }

    pub fn bide_mut(&mut self) -> Option<&mut BideState> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::Bide(bide) => Some(bide),
            _ => None,
        })
    }

    /// The Pokémon that threw the holder's Syrup Bomb.
    pub fn syrup_bomb_source(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::SyrupBomb { source, .. } => Some(*source),
            _ => None,
        })
    }

    /// Dragon Cheer adding `stages`; false when already there.
    pub fn set_dragon_cheer(&mut self, stages: u8) -> bool {
        if self.contains(VolatileConditionKind::DragonCheer) {
            return false;
        }
        self.entries.push(VolatileCondition::DragonCheer { stages });
        true
    }

    /// The critical-hit stages Dragon Cheer adds (0 without it).
    pub fn dragon_cheer_stages(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                VolatileCondition::DragonCheer { stages } => Some(*stages),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Fling's mark: the holder threw `item` and loses it at the next `Update`.
    pub fn set_fling(&mut self, item: ItemId) {
        self.remove(VolatileConditionKind::Fling);
        self.entries.push(VolatileCondition::Fling { item });
    }

    /// The item Fling's mark says the holder threw.
    pub fn flung_item(&self) -> Option<ItemId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Fling { item } => Some(*item),
            _ => None,
        })
    }

    /// Micle Berry's boost, for `turns` (none: until used); false when it already has one
    /// (`addVolatile` fails, no `onRestart`).
    pub fn set_micle_berry(&mut self, turns: Option<u8>) -> bool {
        if self.contains(VolatileConditionKind::MicleBerry) {
            return false;
        }
        self.entries.push(VolatileCondition::MicleBerry { turns });
        true
    }

    /// Laser Focus for `turns`, started or restarted (Showdown `onRestart`: `duration = 2`).
    pub fn set_laser_focus(&mut self, turns: u8) {
        self.remove(VolatileConditionKind::LaserFocus);
        self.entries.push(VolatileCondition::LaserFocus { turns });
    }

    /// Stockpile's layers (0 without one).
    /// How many times Autotomize took 100 kg off the holder (0 without it).
    pub fn autotomize_count(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                VolatileCondition::Autotomize { count } => Some(*count),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Autotomize once more (the first one starts it): `lighter` counts another 100 kg off.
    pub fn add_autotomize(&mut self, lighter: bool) {
        for entry in &mut self.entries {
            if let VolatileCondition::Autotomize { count } = entry {
                *count += u8::from(lighter);
                return;
            }
        }
        self.entries.push(VolatileCondition::Autotomize {
            count: u8::from(lighter),
        });
    }

    /// Gen 6+ Transform: the holder's Autotomize count becomes `count` (none at 0).
    pub fn set_autotomize_count(&mut self, count: u8) {
        self.remove(VolatileConditionKind::Autotomize);
        if count > 0 {
            self.entries.push(VolatileCondition::Autotomize { count });
        }
    }

    pub fn stockpile_layers(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                VolatileCondition::Stockpile { layers, .. } => Some(*layers),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// One more Stockpile layer (the first one starts it); returns the layers.
    pub fn add_stockpile_layer(&mut self) -> u8 {
        for entry in &mut self.entries {
            if let VolatileCondition::Stockpile { layers, .. } = entry {
                *layers += 1;
                return *layers;
            }
        }
        self.entries.push(VolatileCondition::Stockpile {
            layers: 1,
            defense: 0,
            special_defense: 0,
        });
        1
    }

    /// Stockpile's end will take back one more stage of `defense` / `special_defense`.
    pub fn count_stockpile_stages(&mut self, defense: bool, special_defense: bool) {
        for entry in &mut self.entries {
            if let VolatileCondition::Stockpile {
                defense: def,
                special_defense: spd,
                ..
            } = entry
            {
                *def -= i8::from(defense);
                *spd -= i8::from(special_defense);
            }
        }
    }

    /// Removes Stockpile; the stages its end takes back (Defense, Special Defense).
    pub fn take_stockpile(&mut self) -> Option<(i8, i8)> {
        let taken = self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Stockpile {
                defense,
                special_defense,
                ..
            } => Some((*defense, *special_defense)),
            _ => None,
        })?;
        self.remove(VolatileConditionKind::Stockpile);
        Some(taken)
    }

    /// Syrup Bomb thrown by `source`, for `turns`; false when already there.
    pub fn set_syrup_bomb(&mut self, source: PokemonId, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::SyrupBomb) {
            return false;
        }
        self.entries
            .push(VolatileCondition::SyrupBomb { source, turns });
        true
    }

    /// Throat Chop for `turns`; false when already there (no renewal).
    pub fn set_throat_chop(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::ThroatChop) {
            return false;
        }
        self.entries.push(VolatileCondition::ThroatChop { turns });
        true
    }

    /// The volatiles that last only while their source stays on the field (Showdown's
    /// `onUpdate` checks `effectState.source.isActive`), with that source: Attract, Syrup Bomb.
    pub fn bound(&self) -> Vec<(VolatileConditionKind, PokemonId)> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                VolatileCondition::Attract { source }
                | VolatileCondition::SyrupBomb { source, .. } => Some((entry.kind(), *source)),
                _ => None,
            })
            .collect()
    }

    /// Perish Song's count (Showdown `duration: 4`); false when one is already running.
    pub fn set_perish_song(&mut self) -> bool {
        if self.contains(VolatileConditionKind::PerishSong) {
            return false;
        }
        self.entries
            .push(VolatileCondition::PerishSong { turns: 4 });
        true
    }

    /// One residual of Perish Song: the count left, `Some(0)` when it ran out (the volatile is
    /// then gone); `None` without the volatile.
    pub fn tick_perish_song(&mut self) -> Option<u8> {
        let left = self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::PerishSong { turns } => {
                *turns = turns.saturating_sub(1);
                Some(*turns)
            }
            _ => None,
        })?;
        if left == 0 {
            self.remove(VolatileConditionKind::PerishSong);
        }
        Some(left)
    }

    /// Infatuated with `source`; false when already infatuated.
    pub fn set_attract(&mut self, source: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::Attract) {
            return false;
        }
        self.entries.push(VolatileCondition::Attract { source });
        true
    }

    pub fn stall_counter(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Stall { counter, .. } => Some(*counter),
            _ => None,
        })
    }

    /// Ally Switch's own counter (Gen 9): `addVolatile('allyswitch')` starting at 3 and
    /// multiplying by 3 each consecutive use, capped at 729.
    pub fn ally_switch_counter(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::AllySwitch { counter, .. } => Some(*counter),
            _ => None,
        })
    }

    /// Showdown `allyswitch`'s `onStart` (3) and `onRestart` (×3, at most 729): a fresh counter
    /// or the next one, with the duration reset. False when the chance failed (no counter).
    pub fn bump_ally_switch(&mut self, turns: u8) -> u16 {
        for entry in &mut self.entries {
            if let VolatileCondition::AllySwitch {
                counter,
                turns: left,
            } = entry
            {
                *counter = (*counter * 3).min(729);
                *left = turns;
                return *counter;
            }
        }
        self.entries
            .push(VolatileCondition::AllySwitch { counter: 3, turns });
        3
    }

    /// `onRestart`'s failure: the volatile goes (the move fails with it).
    pub fn remove_ally_switch(&mut self) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| !matches!(entry, VolatileCondition::AllySwitch { .. }));
        self.entries.len() != before
    }

    /// Showdown `stall` onStart / onRestart: a fresh counter (`start`), or the live one
    /// advanced by `next` with its duration reset.
    pub fn bump_stall(&mut self, start: u16, next: impl FnOnce(u16) -> u16, turns: u8) -> u16 {
        for entry in &mut self.entries {
            if let VolatileCondition::Stall {
                counter,
                turns: left,
            } = entry
            {
                *counter = next(*counter);
                *left = turns;
                return *counter;
            }
        }
        self.entries.push(VolatileCondition::Stall {
            counter: start,
            turns,
        });
        start
    }

    pub fn encored_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Encore { move_id, .. } => Some(*move_id),
            _ => None,
        })
    }

    pub fn disabled_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Disable { move_id, .. } => Some(*move_id),
            _ => None,
        })
    }

    /// Gen 1's disabled slot (`VolatileCondition::Disable::slot`).
    pub fn disabled_slot(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Disable { slot, .. } => *slot,
            _ => None,
        })
    }

    pub fn choice_locked_move(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::ChoiceLock { move_id } => Some(*move_id),
            _ => None,
        })
    }

    /// Remaining turns of a timed condition (Taunt / Encore / Disable).
    pub fn turns_left(&self, kind: VolatileConditionKind) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Taunt { turns } if kind == VolatileConditionKind::Taunt => {
                Some(*turns)
            }
            VolatileCondition::Encore { turns, .. } if kind == VolatileConditionKind::Encore => {
                Some(*turns)
            }
            VolatileCondition::Disable { turns, .. } if kind == VolatileConditionKind::Disable => {
                Some(*turns)
            }
            VolatileCondition::TwoTurnMove { turns, .. }
                if kind == VolatileConditionKind::TwoTurnMove =>
            {
                Some(*turns)
            }
            VolatileCondition::PartiallyTrapped { turns, .. }
                if kind == VolatileConditionKind::PartiallyTrapped =>
            {
                Some(*turns)
            }
            VolatileCondition::Stall { turns, .. } if kind == VolatileConditionKind::Stall => {
                Some(*turns)
            }
            VolatileCondition::PartialTrappingLock { turns, .. }
                if kind == VolatileConditionKind::PartialTrappingLock =>
            {
                Some(*turns)
            }
            VolatileCondition::FakePartiallyTrapped { turns, .. }
                if kind == VolatileConditionKind::FakePartiallyTrapped =>
            {
                Some(*turns)
            }
            _ => None,
        })
    }

    /// Showdown `duration--` in the residual schedule: returns the turns left, removing the
    /// condition when it reaches 0 (the caller traces the end).
    pub fn tick_timed(&mut self, kind: VolatileConditionKind) -> Option<u8> {
        let index = self.entries.iter().position(|entry| entry.kind() == kind)?;
        let turns = timed_turns(&mut self.entries[index])?;
        *turns = turns.saturating_sub(1);
        let left = *turns;
        if left == 0 {
            self.entries.remove(index);
        }
        Some(left)
    }

    /// The turns a condition `tick_timed` counts down has left before its next `duration--`:
    /// 1 where the next residual ends it (R-M10-66).
    pub fn timed_turns_left(&self, kind: VolatileConditionKind) -> Option<u8> {
        let mut entry = *self.entries.iter().find(|entry| entry.kind() == kind)?;
        timed_turns(&mut entry).map(|turns| *turns)
    }

    /// Chilly Reception's announcement for the turn (`addVolatile('chillyreception')`).
    pub fn set_chilly_reception(&mut self) -> bool {
        self.set_turn_start(VolatileConditionKind::ChillyReception)
    }

    /// A move's turn-start volatile (`priorityChargeCallback` / `beforeTurnCallback`: Chilly
    /// Reception, Focus Punch, Beak Blast, Shell Trap), for the turn; false when already there.
    pub fn set_turn_start(&mut self, kind: VolatileConditionKind) -> bool {
        if self.contains(kind) {
            return false;
        }
        let entry = match kind {
            VolatileConditionKind::ChillyReception => {
                VolatileCondition::ChillyReception { turns: 1 }
            }
            VolatileConditionKind::FocusPunch => VolatileCondition::FocusPunch {
                lost_focus: false,
                turns: 1,
            },
            VolatileConditionKind::BeakBlast => VolatileCondition::BeakBlast { turns: 1 },
            VolatileConditionKind::ShellTrap => VolatileCondition::ShellTrap {
                got_hit: false,
                turns: 1,
            },
            VolatileConditionKind::Counter => VolatileCondition::Counter { turns: 1 },
            VolatileConditionKind::MirrorCoat => VolatileCondition::MirrorCoat { turns: 1 },
            _ => return false,
        };
        self.entries.push(entry);
        true
    }

    /// Focus Punch's volatile: a damaging hit landed on the holder (`lostFocus`).
    pub fn lose_focus(&mut self) {
        for entry in &mut self.entries {
            if let VolatileCondition::FocusPunch { lost_focus, .. } = entry {
                *lost_focus = true;
            }
        }
    }

    pub fn lost_focus(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry,
                VolatileCondition::FocusPunch {
                    lost_focus: true,
                    ..
                }
            )
        })
    }

    /// Shell Trap's volatile: a foe's physical hit armed it (`gotHit`); false when it already
    /// was, or there is none.
    pub fn arm_shell_trap(&mut self) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::ShellTrap { got_hit, .. } = entry {
                let fresh = !*got_hit;
                *got_hit = true;
                return fresh;
            }
        }
        false
    }

    pub fn shell_trap_armed(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry, VolatileCondition::ShellTrap { got_hit: true, .. }))
    }

    /// Showdown `addVolatile('trapped', source)`: fails while the holder is already trapped.
    pub fn set_trapped(&mut self, source: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::Trapped) {
            return false;
        }
        self.entries.push(VolatileCondition::Trapped { source });
        true
    }

    /// Who keeps the holder trapped (Mean Look's user).
    pub fn trapped_by(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Trapped { source } => Some(*source),
            _ => None,
        })
    }

    /// The holder is now a trapper; returns whether it was not one yet.
    pub fn set_trapper(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Trapper) {
            return false;
        }
        self.entries.push(VolatileCondition::Trapper);
        true
    }

    /// Gen 1's `partialtrappinglock`: the lock a binding move's first hit starts.
    pub fn set_partial_trapping_lock(
        &mut self,
        target: PokemonId,
        move_id: MoveId,
        damage: u16,
        turns: u8,
    ) {
        self.remove(VolatileConditionKind::PartialTrappingLock);
        self.entries.push(VolatileCondition::PartialTrappingLock {
            target,
            move_id,
            damage,
            turns,
            total: turns,
        });
    }

    /// The Gen 1 binding lock: (target, move, damage, turns left, turns rolled).
    pub fn partial_trapping_lock(&self) -> Option<(PokemonId, MoveId, u16, u8, u8)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::PartialTrappingLock {
                target,
                move_id,
                damage,
                turns,
                total,
            } => Some((*target, *move_id, *damage, *turns, *total)),
            _ => None,
        })
    }

    /// Gen 1's `fakepartiallytrapped` (duration 2), facing `counterpart`.
    pub fn set_fake_partially_trapped(&mut self, counterpart: PokemonId) {
        self.remove(VolatileConditionKind::FakePartiallyTrapped);
        self.entries.push(VolatileCondition::FakePartiallyTrapped {
            counterpart,
            turns: 2,
        });
    }

    pub fn fake_partially_trapped_counterpart(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::FakePartiallyTrapped { counterpart, .. } => Some(*counterpart),
            _ => None,
        })
    }

    /// Showdown `addVolatile('octolock', source)`: fails while one is already there.
    pub fn set_octolock(&mut self, source: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::Octolock) {
            return false;
        }
        self.entries.push(VolatileCondition::Octolock { source });
        true
    }

    /// Who octolocked the holder.
    pub fn octolocked_by(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Octolock { source } => Some(*source),
            _ => None,
        })
    }

    /// Gen 2-4 Baton Pass (`copyVolatileFrom` moves the links): the holder is now trapped by
    /// `to`, which took `from`'s place.
    pub fn retarget_trap(&mut self, from: PokemonId, to: PokemonId) {
        for entry in &mut self.entries {
            if let VolatileCondition::Trapped { source } = entry
                && *source == from
            {
                *source = to;
            }
        }
    }

    /// Showdown `addVolatile('toxic')`: fails while one is already there.
    pub fn set_sure_hit(&mut self) -> bool {
        if self.contains(VolatileConditionKind::SureHit) {
            return false;
        }
        self.entries.push(VolatileCondition::SureHit { turns: 1 });
        true
    }

    /// Showdown `addVolatile('mefirst')`: the boost marker for the move Me First calls.
    pub fn set_me_first(&mut self) -> bool {
        if self.contains(VolatileConditionKind::MeFirst) {
            return false;
        }
        self.entries.push(VolatileCondition::MeFirst { turns: 1 });
        true
    }

    /// Showdown `addVolatile('snatch')`: like the coat, for the turn; a second is no failure.
    pub fn set_snatch(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Snatch) {
            return false;
        }
        self.entries.push(VolatileCondition::Snatch { turns: 1 });
        true
    }

    /// Showdown `addVolatile('magiccoat')`: the move announces itself and stays for the turn.
    /// A second Magic Coat is no failure (`addVolatile` restarts it: the duration is the same).
    pub fn set_magic_coat(&mut self, prankster_boosted: bool) -> bool {
        if self.contains(VolatileConditionKind::MagicCoat) {
            return false;
        }
        self.entries.push(VolatileCondition::MagicCoat {
            turns: 1,
            prankster_boosted,
        });
        true
    }

    /// Whether the Magic Coat on this Pokémon was Prankster-boosted.
    pub fn magic_coat_prankster(&self) -> bool {
        self.entries.iter().any(|volatile| {
            matches!(
                volatile,
                VolatileCondition::MagicCoat {
                    prankster_boosted: true,
                    ..
                }
            )
        })
    }

    pub fn set_leech_seed(&mut self, source: PokemonId, source_position: PositionId) -> bool {
        if self.contains(VolatileConditionKind::LeechSeed) {
            return false;
        }
        self.entries.push(VolatileCondition::LeechSeed {
            source,
            source_position,
        });
        true
    }

    pub fn leech_seed_source_position(&self) -> Option<PositionId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::LeechSeed {
                source_position, ..
            } => Some(*source_position),
            _ => None,
        })
    }

    pub fn set_substitute(&mut self, source: PokemonId, hp: u16) {
        self.remove(VolatileConditionKind::Substitute);
        self.entries
            .push(VolatileCondition::Substitute { source, hp });
    }

    pub fn set_flinch(&mut self, source: PokemonId) {
        self.remove(VolatileConditionKind::Flinch);
        self.entries.push(VolatileCondition::Flinch { source });
    }

    /// Returns false when already confused (Showdown: the effect fails to stack).
    pub fn set_confusion(&mut self, source: PokemonId, time: u16) -> bool {
        if self.contains(VolatileConditionKind::Confusion) {
            return false;
        }
        self.entries
            .push(VolatileCondition::Confusion { source, time });
        true
    }

    pub fn confusion_time(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Confusion { time, .. } => Some(*time),
            _ => None,
        })
    }

    /// Decrements the confusion counter. Returns the new value; `Some(0)` means the confusion
    /// just ended and has been removed.
    pub fn tick_confusion(&mut self) -> Option<u16> {
        let index = self
            .entries
            .iter()
            .position(|entry| matches!(entry, VolatileCondition::Confusion { .. }))?;
        let remaining = match &mut self.entries[index] {
            VolatileCondition::Confusion { time, .. } => {
                *time = time.saturating_sub(1);
                *time
            }
            _ => unreachable!(),
        };
        if remaining == 0 {
            self.entries.remove(index);
        }
        Some(remaining)
    }

    pub fn substitute_hp(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Substitute { hp, .. } => Some(*hp),
            _ => None,
        })
    }

    /// Applies damage to a substitute. Returns `(applied, before, after, broken)` when present.
    pub fn damage_substitute(&mut self, amount: u16) -> Option<(u16, u16, u16, bool)> {
        let index = self
            .entries
            .iter()
            .position(|entry| matches!(entry, VolatileCondition::Substitute { .. }))?;

        let (applied, before, after, broken) = match &mut self.entries[index] {
            VolatileCondition::Substitute { hp, .. } => {
                let before = *hp;
                let applied = amount.min(before);
                *hp -= applied;
                (applied, before, *hp, *hp == 0)
            }
            _ => unreachable!(),
        };

        if broken {
            self.entries.remove(index);
        }

        Some((applied, before, after, broken))
    }

    pub fn remove(&mut self, kind: VolatileConditionKind) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.kind() != kind);
        before != self.entries.len()
    }

    /// Volatile state is cleared when a Pokémon leaves the active field.
    /// Baton Pass / Shed Tail (Showdown `copyVolatileFrom`): the entries `passed` lets through,
    /// as they stand (counts, turns, the Substitute's HP).
    pub fn passed_on(
        &self,
        passed: impl Fn(VolatileConditionKind) -> bool,
    ) -> Vec<VolatileCondition> {
        self.entries
            .iter()
            .copied()
            .filter(|entry| passed(entry.kind()))
            .collect()
    }

    /// The entries handed on by the Pokémon this one replaced.
    pub fn take_over(&mut self, entries: &[VolatileCondition]) {
        for entry in entries {
            self.remove(entry.kind());
            self.entries.push(*entry);
        }
    }

    pub fn clear_on_switch(&mut self) -> Vec<VolatileConditionKind> {
        let expired = self.entries.iter().map(|entry| entry.kind()).collect();
        self.entries.clear();
        expired
    }

    /// End-of-turn lifecycle for turn-scoped volatile conditions.
    pub fn expire_end_turn(&mut self, current_turn: u32) -> Vec<VolatileConditionKind> {
        let mut expired = Vec::new();
        self.entries.retain(|entry| {
            let should_expire = matches!(
                entry,
                VolatileCondition::Protect { started_turn, .. }
                    | VolatileCondition::Endure { started_turn }
                    if *started_turn <= current_turn
            ) || matches!(
                entry,
                VolatileCondition::Flinch { .. }
                    | VolatileCondition::FollowMe
                    | VolatileCondition::RagePowder
                    | VolatileCondition::Spotlight
                    | VolatileCondition::Pursuit { .. }
                    | VolatileCondition::HelpingHand { .. }
            );
            if should_expire {
                expired.push(entry.kind());
                false
            } else {
                true
            }
        });
        expired
    }

    /// The volatiles that only count down in the residual (Showdown's `duration`-only
    /// handlers, none with a residual order of its own), in the order they started: the ones
    /// `tick_timed` counts and the ones of a single turn (`expire_end_turn`). The volatiles with
    /// a residual of their own (Taunt, Perish Song …) are residual instances instead.
    pub fn counting_down(&self) -> Vec<VolatileConditionKind> {
        self.entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    VolatileCondition::TwoTurnMove { .. }
                        | VolatileCondition::MustRecharge { turns: Some(_) }
                        | VolatileCondition::Stall { .. }
                        | VolatileCondition::AllySwitch { .. }
                        | VolatileCondition::SureHit { .. }
                        | VolatileCondition::MagicCoat { .. }
                        | VolatileCondition::Snatch { .. }
                        | VolatileCondition::MeFirst { .. }
                        | VolatileCondition::ChillyReception { .. }
                        | VolatileCondition::FocusPunch { .. }
                        | VolatileCondition::BeakBlast { .. }
                        | VolatileCondition::ShellTrap { .. }
                        | VolatileCondition::Counter { .. }
                        | VolatileCondition::MirrorCoat { .. }
                        | VolatileCondition::Rolling { .. }
                        | VolatileCondition::Rampage(_)
                        | VolatileCondition::LockOn { .. }
                        | VolatileCondition::SkyDrop { .. }
                        | VolatileCondition::FuryCutter { turns: Some(_), .. }
                        | VolatileCondition::Bide(BideState { turns: Some(_), .. })
                        | VolatileCondition::Protect { .. }
                        | VolatileCondition::Endure { .. }
                        | VolatileCondition::Flinch { .. }
                        | VolatileCondition::FollowMe
                        | VolatileCondition::RagePowder
                        | VolatileCondition::Spotlight
                        | VolatileCondition::Pursuit { .. }
                        | VolatileCondition::HelpingHand { .. }
                )
            })
            .map(|entry| entry.kind())
            .collect()
    }

    /// One residual count off `kind` (`counting_down`): `Some(true)` when it ran out and is
    /// gone, `Some(false)` while it goes on, `None` when the Pokémon does not have it.
    pub fn count_down(&mut self, kind: VolatileConditionKind) -> Option<bool> {
        let single_turn = matches!(
            kind,
            VolatileConditionKind::Protect
                | VolatileConditionKind::Endure
                | VolatileConditionKind::Flinch
                | VolatileConditionKind::FollowMe
                | VolatileConditionKind::RagePowder
                | VolatileConditionKind::Spotlight
                | VolatileConditionKind::Pursuit
                | VolatileConditionKind::HelpingHand
        );
        if single_turn {
            return self.remove(kind).then_some(true);
        }
        self.tick_timed(kind).map(|left| left == 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum SideConditionKind {
    StealthRock,
    Spikes,
    ToxicSpikes,
    StickyWeb,
    Tailwind,
    /// Wide Guard / Quick Guard: the side is guarded for the turn.
    WideGuard,
    QuickGuard,
    /// Crafty Shield / Mat Block (Gen 6-8): the side is guarded for the turn against status
    /// moves / against every move Protect would stop.
    CraftyShield,
    MatBlock,
    /// Reflect / Light Screen / Aurora Veil (Gen 2+): hits on the side are weakened while the
    /// screen lasts (`SideCondition::Timed`).
    Reflect,
    LightScreen,
    AuroraVeil,
    /// Mist (Gen 3+), Safeguard (Gen 2+), Lucky Chant (Gen 4-7): other Pokémon cannot lower the
    /// stats of the side's Pokémon / give them a status or confusion; no critical hit lands on
    /// them (`SideCondition::Timed`).
    Mist,
    /// The Pledge combo's field effects (DESIGN.md §42.9): Fire Pledge chips the side's
    /// non-Fire types every turn, Grass Pledge quarters their Speed, Water Pledge doubles the
    /// secondaries of the side's moves.
    FirePledge,
    GrassPledge,
    WaterPledge,
    Safeguard,
    LuckyChant,
}

/// Entry hazards and side effects. Entries keep insertion order: entry hazards trigger in the
/// order they were laid (Showdown runs the side's SwitchIn handlers in effect order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideCondition {
    StealthRock {
        source: PokemonId,
    },
    Spikes {
        source: PokemonId,
        layers: u8,
    },
    ToxicSpikes {
        source: PokemonId,
        layers: u8,
    },
    StickyWeb {
        source: PokemonId,
    },
    Tailwind {
        source: PokemonId,
        turns_remaining: u8,
    },
    WideGuard {
        source: PokemonId,
    },
    QuickGuard {
        source: PokemonId,
    },
    CraftyShield {
        source: PokemonId,
    },
    MatBlock {
        source: PokemonId,
    },
    /// A side condition that only lasts a number of turns (Showdown `duration`, counted down
    /// in the residual): the screens, Mist, Safeguard, Lucky Chant.
    Timed {
        kind: SideConditionKind,
        turns_remaining: u8,
    },
}

impl SideCondition {
    pub const fn kind(self) -> SideConditionKind {
        match self {
            Self::Timed { kind, .. } => kind,
            Self::StealthRock { .. } => SideConditionKind::StealthRock,
            Self::Spikes { .. } => SideConditionKind::Spikes,
            Self::ToxicSpikes { .. } => SideConditionKind::ToxicSpikes,
            Self::StickyWeb { .. } => SideConditionKind::StickyWeb,
            Self::Tailwind { .. } => SideConditionKind::Tailwind,
            Self::WideGuard { .. } => SideConditionKind::WideGuard,
            Self::QuickGuard { .. } => SideConditionKind::QuickGuard,
            Self::CraftyShield { .. } => SideConditionKind::CraftyShield,
            Self::MatBlock { .. } => SideConditionKind::MatBlock,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideConditions {
    entries: Vec<SideCondition>,
}

impl SideConditions {
    pub fn entries(&self) -> &[SideCondition] {
        &self.entries
    }

    pub fn contains(&self, kind: SideConditionKind) -> bool {
        self.entries.iter().any(|entry| entry.kind() == kind)
    }

    pub fn set_stealth_rock(&mut self, source: PokemonId) -> bool {
        if self.contains(SideConditionKind::StealthRock) {
            return false;
        }
        self.entries.push(SideCondition::StealthRock { source });
        true
    }

    /// Adds a layer of Spikes (up to `max_layers`); returns the new layer count, or `None`
    /// when the side is already full.
    pub fn add_spikes(&mut self, source: PokemonId, max_layers: u8) -> Option<u8> {
        self.add_layer(source, max_layers, false)
    }

    pub fn add_toxic_spikes(&mut self, source: PokemonId, max_layers: u8) -> Option<u8> {
        self.add_layer(source, max_layers, true)
    }

    fn add_layer(&mut self, source: PokemonId, max_layers: u8, toxic: bool) -> Option<u8> {
        for entry in &mut self.entries {
            match (entry, toxic) {
                (SideCondition::Spikes { layers, .. }, false)
                | (SideCondition::ToxicSpikes { layers, .. }, true) => {
                    if *layers >= max_layers {
                        return None;
                    }
                    *layers += 1;
                    return Some(*layers);
                }
                _ => {}
            }
        }
        self.entries.push(if toxic {
            SideCondition::ToxicSpikes { source, layers: 1 }
        } else {
            SideCondition::Spikes { source, layers: 1 }
        });
        Some(1)
    }

    pub fn spikes_layers(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                SideCondition::Spikes { layers, .. } => Some(*layers),
                _ => None,
            })
            .unwrap_or(0)
    }

    pub fn toxic_spikes_layers(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                SideCondition::ToxicSpikes { layers, .. } => Some(*layers),
                _ => None,
            })
            .unwrap_or(0)
    }

    pub fn set_sticky_web(&mut self, source: PokemonId) -> bool {
        if self.contains(SideConditionKind::StickyWeb) {
            return false;
        }
        self.entries.push(SideCondition::StickyWeb { source });
        true
    }

    /// Removes a condition (a Poison type absorbing Toxic Spikes, Defog …).
    pub fn remove(&mut self, kind: SideConditionKind) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.kind() != kind);
        self.entries.len() != before
    }

    /// Takes out the conditions `moves` picks, in order, leaving the others where they are
    /// (Court Change).
    pub fn take(&mut self, moves: impl Fn(SideConditionKind) -> bool) -> Vec<SideCondition> {
        let (taken, kept) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|entry| moves(entry.kind()));
        self.entries = kept;
        taken
    }

    /// Adds conditions after the ones already there (Court Change).
    pub fn extend(&mut self, entries: Vec<SideCondition>) {
        self.entries.extend(entries);
    }

    pub fn set_tailwind(&mut self, source: PokemonId, turns: u8) -> bool {
        if self.contains(SideConditionKind::Tailwind) {
            return false;
        }
        self.entries.push(SideCondition::Tailwind {
            source,
            turns_remaining: turns.max(1),
        });
        true
    }

    /// Wide Guard / Quick Guard for the turn; false when the side already has it.
    pub fn set_guard(&mut self, kind: SideConditionKind, source: PokemonId) -> bool {
        if self.contains(kind) {
            return false;
        }
        match kind {
            SideConditionKind::WideGuard => {
                self.entries.push(SideCondition::WideGuard { source });
            }
            SideConditionKind::QuickGuard => {
                self.entries.push(SideCondition::QuickGuard { source });
            }
            SideConditionKind::CraftyShield => {
                self.entries.push(SideCondition::CraftyShield { source });
            }
            SideConditionKind::MatBlock => {
                self.entries.push(SideCondition::MatBlock { source });
            }
            _ => return false,
        }
        true
    }

    pub fn tailwind_turns(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            SideCondition::Tailwind {
                turns_remaining, ..
            } => Some(*turns_remaining),
            _ => None,
        })
    }

    /// Starts a timed side condition (a screen) for `turns`; false when the side has it.
    pub fn set_timed(&mut self, kind: SideConditionKind, turns: u8) -> bool {
        if self.contains(kind) {
            return false;
        }
        self.entries.push(SideCondition::Timed {
            kind,
            turns_remaining: turns.max(1),
        });
        true
    }

    /// The timed side conditions' countdown alone (Gen 3-4 count the screens down first in
    /// the residual); returns the ones that ended.
    pub fn tick_timed(&mut self) -> Vec<SideConditionKind> {
        let mut expired = Vec::new();
        self.entries.retain_mut(|entry| match entry {
            SideCondition::Timed {
                kind,
                turns_remaining,
            } => count_down(*kind, turns_remaining, &mut expired),
            _ => true,
        });
        expired
    }

    /// The end-of-turn countdown; `tailwind` false leaves Tailwind to `tick_tailwind` (Gen 4
    /// counts it down first in the residual), `timed` false the screens to `tick_timed`.
    pub fn tick_end_turn(&mut self, tailwind: bool, timed: bool) -> Vec<SideConditionKind> {
        let mut expired = Vec::new();
        self.entries.retain_mut(|entry| match entry {
            SideCondition::Tailwind { .. } if !tailwind => true,
            SideCondition::Timed { .. } if !timed => true,
            SideCondition::Timed {
                kind,
                turns_remaining,
            } => count_down(*kind, turns_remaining, &mut expired),
            SideCondition::StealthRock { .. }
            | SideCondition::Spikes { .. }
            | SideCondition::ToxicSpikes { .. }
            | SideCondition::StickyWeb { .. } => true,
            // Single-turn guards (Showdown duration 1).
            SideCondition::WideGuard { .. } => {
                expired.push(SideConditionKind::WideGuard);
                false
            }
            SideCondition::QuickGuard { .. } => {
                expired.push(SideConditionKind::QuickGuard);
                false
            }
            SideCondition::CraftyShield { .. } => {
                expired.push(SideConditionKind::CraftyShield);
                false
            }
            SideCondition::MatBlock { .. } => {
                expired.push(SideConditionKind::MatBlock);
                false
            }
            SideCondition::Tailwind {
                turns_remaining, ..
            } => {
                if *turns_remaining <= 1 {
                    expired.push(SideConditionKind::Tailwind);
                    false
                } else {
                    *turns_remaining -= 1;
                    true
                }
            }
        });
        expired
    }

    /// Whether `kind` counts down in the residual: everything but the entry hazards.
    pub fn counts_down(&self, kind: SideConditionKind) -> bool {
        !matches!(
            kind,
            SideConditionKind::StealthRock
                | SideConditionKind::Spikes
                | SideConditionKind::ToxicSpikes
                | SideConditionKind::StickyWeb
        )
    }

    /// One residual count off `kind`: `Some(true)` when it ran out and is gone, `Some(false)`
    /// while it goes on, `None` when the side does not have it (or it never counts down).
    pub fn count_down(&mut self, kind: SideConditionKind) -> Option<bool> {
        let index = self.entries.iter().position(|entry| entry.kind() == kind)?;
        let ended = match &mut self.entries[index] {
            SideCondition::StealthRock { .. }
            | SideCondition::Spikes { .. }
            | SideCondition::ToxicSpikes { .. }
            | SideCondition::StickyWeb { .. } => return None,
            SideCondition::WideGuard { .. }
            | SideCondition::QuickGuard { .. }
            | SideCondition::CraftyShield { .. }
            | SideCondition::MatBlock { .. } => true,
            SideCondition::Timed {
                turns_remaining, ..
            }
            | SideCondition::Tailwind {
                turns_remaining, ..
            } => {
                if *turns_remaining <= 1 {
                    true
                } else {
                    *turns_remaining -= 1;
                    false
                }
            }
        };
        if ended {
            self.entries.remove(index);
        }
        Some(ended)
    }

    /// Tailwind's countdown alone; true when it ended.
    pub fn tick_tailwind(&mut self) -> bool {
        let mut ended = false;
        self.entries.retain_mut(|entry| match entry {
            SideCondition::Tailwind {
                turns_remaining, ..
            } => {
                if *turns_remaining <= 1 {
                    ended = true;
                    false
                } else {
                    *turns_remaining -= 1;
                    true
                }
            }
            _ => true,
        });
        ended
    }
}

/// One turn off a timed side condition; false (and `kind` noted as ended) when it runs out.
fn count_down(
    kind: SideConditionKind,
    turns: &mut u8,
    expired: &mut Vec<SideConditionKind>,
) -> bool {
    if *turns <= 1 {
        expired.push(kind);
        false
    } else {
        *turns -= 1;
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum FieldConditionKind {
    TrickRoom,
    EchoedVoice,
    /// Gravity (Gen 4+): every Pokémon grounded, accuracy raised, the airborne moves refused.
    Gravity,
    /// Wonder Room (Gen 5+): every Pokémon's stored Defense and Special Defense trade places.
    WonderRoom,
    /// Ion Deluge (Gen 6-7, and Plasma Fists): Normal moves are Electric for the rest of the
    /// turn.
    IonDeluge,
    /// Gen 6-7 Mud Sport / Water Sport: Electric / Fire moves weakened.
    MudSport,
    WaterSport,
    /// Fairy Lock (Gen 6+): no Pokémon can switch out the next turn.
    FairyLock,
    /// Magic Room (Gen 5+): no Pokémon's item has an effect (`engine::held_item`). Its
    /// duration counts down in the residual (`onFieldResidualOrder` 27.6, `ResidualRules::durations`).
    MagicRoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCondition {
    TrickRoom {
        source: PokemonId,
        turns_remaining: u8,
    },
    /// Echoed Voice's field condition (Showdown `echoedvoice`, duration 2): its `multiplier`
    /// grows by one, up to 5, each turn the move is used again before it runs out.
    EchoedVoice { multiplier: u8, turns_remaining: u8 },
    /// A pseudo-weather that only lasts a number of turns (Showdown `duration`, counted down
    /// at the end of the turn): Gravity, Wonder Room, Ion Deluge, Mud / Water Sport, Fairy
    /// Lock.
    Timed {
        kind: FieldConditionKind,
        turns_remaining: u8,
    },
}

impl FieldCondition {
    pub const fn kind(self) -> FieldConditionKind {
        match self {
            Self::TrickRoom { .. } => FieldConditionKind::TrickRoom,
            Self::EchoedVoice { .. } => FieldConditionKind::EchoedVoice,
            Self::Timed { kind, .. } => kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum WeatherKind {
    Sun,
    Rain,
    Sandstorm,
    Snow,
    /// Delta Stream's strong winds (Showdown `deltastream`), always a strong weather.
    StrongWinds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeatherState {
    pub kind: WeatherKind,
    pub source: PokemonId,
    pub turns_remaining: u8,
    /// Showdown's strong weathers (DESIGN.md §23.5): Desolate Land's extremely harsh sun
    /// (`kind` Sun), Primordial Sea's heavy rain (Rain), Delta Stream's winds. Everything that
    /// reads the sun or the rain reads them too; they never run out, stop the opposite type's
    /// attacks and keep every other weather from starting.
    pub strong: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum TerrainKind {
    Electric,
    Grassy,
    Psychic,
    Misty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainState {
    pub kind: TerrainKind,
    pub source: PokemonId,
    pub turns_remaining: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldState {
    conditions: Vec<FieldCondition>,
    pub weather: Option<WeatherState>,
    pub terrain: Option<TerrainState>,
}

impl FieldState {
    pub fn conditions(&self) -> &[FieldCondition] {
        &self.conditions
    }

    pub fn contains(&self, kind: FieldConditionKind) -> bool {
        self.conditions.iter().any(|entry| entry.kind() == kind)
    }

    /// Trick Room toggles: using it while active removes it.
    /// Returns `true` when the condition becomes active, `false` when it is removed.
    pub fn toggle_trick_room(&mut self, source: PokemonId, turns: u8) -> bool {
        if let Some(index) = self
            .conditions
            .iter()
            .position(|entry| entry.kind() == FieldConditionKind::TrickRoom)
        {
            self.conditions.remove(index);
            return false;
        }

        self.conditions.push(FieldCondition::TrickRoom {
            source,
            turns_remaining: turns.max(1),
        });
        true
    }

    pub fn trick_room_turns(&self) -> Option<u8> {
        self.conditions.iter().find_map(|entry| match entry {
            FieldCondition::TrickRoom {
                turns_remaining, ..
            } => Some(*turns_remaining),
            _ => None,
        })
    }

    /// Echoed Voice's `onTryMove` (`addPseudoWeather('echoedvoice')`): starts the condition at
    /// multiplier 1 for two turns, or — used again once a turn has passed (`onFieldRestart`
    /// with its duration no longer 2) — raises the multiplier, up to 5, for two more turns.
    /// Returns whether the condition started.
    pub fn echo(&mut self) -> bool {
        for entry in &mut self.conditions {
            if let FieldCondition::EchoedVoice {
                multiplier,
                turns_remaining,
            } = entry
            {
                if *turns_remaining != 2 {
                    *turns_remaining = 2;
                    *multiplier = (*multiplier + 1).min(5);
                }
                return false;
            }
        }
        self.conditions.push(FieldCondition::EchoedVoice {
            multiplier: 1,
            turns_remaining: 2,
        });
        true
    }

    /// Starts a pseudo-weather for `turns` (`FieldCondition::Timed`); false when it is already
    /// up (Showdown `addPseudoWeather` without `onFieldRestart`).
    pub fn start_timed(&mut self, kind: FieldConditionKind, turns: u8) -> bool {
        if self.contains(kind) {
            return false;
        }
        self.conditions.push(FieldCondition::Timed {
            kind,
            turns_remaining: turns.max(1),
        });
        true
    }

    /// Ends a pseudo-weather; false when it was not up.
    pub fn remove(&mut self, kind: FieldConditionKind) -> bool {
        let before = self.conditions.len();
        self.conditions.retain(|entry| entry.kind() != kind);
        self.conditions.len() != before
    }

    /// Echoed Voice's power multiplier while its condition lasts.
    pub fn echoed_voice_multiplier(&self) -> Option<u8> {
        self.conditions.iter().find_map(|entry| match entry {
            FieldCondition::EchoedVoice { multiplier, .. } => Some(*multiplier),
            _ => None,
        })
    }

    /// `turns` 0 means the weather never runs out (a weather ability before Gen 6).
    pub fn set_weather(
        &mut self,
        kind: WeatherKind,
        source: PokemonId,
        turns: u8,
    ) -> Option<WeatherState> {
        let previous = self.weather;
        self.weather = Some(WeatherState {
            kind,
            source,
            turns_remaining: turns,
            strong: false,
        });
        previous
    }

    /// A strong weather (Desolate Land, Primordial Sea, Delta Stream): no duration.
    pub fn set_strong_weather(
        &mut self,
        kind: WeatherKind,
        source: PokemonId,
    ) -> Option<WeatherState> {
        let previous = self.weather;
        self.weather = Some(WeatherState {
            kind,
            source,
            turns_remaining: 0,
            strong: true,
        });
        previous
    }

    /// The weather's residual tick (Showdown `duration--`, ending at 0): `Some(kind)` when it
    /// just ended, `None` while it goes on or when there is none. Permanent weather never ticks.
    pub fn tick_weather(&mut self) -> Option<WeatherKind> {
        match self.weather {
            Some(state) if state.turns_remaining == 0 => None,
            Some(state) if state.turns_remaining == 1 => {
                self.weather = None;
                Some(state.kind)
            }
            Some(mut state) => {
                state.turns_remaining -= 1;
                self.weather = Some(state);
                None
            }
            None => None,
        }
    }

    pub fn set_terrain(
        &mut self,
        kind: TerrainKind,
        source: PokemonId,
        turns: u8,
    ) -> Option<TerrainState> {
        let previous = self.terrain;
        self.terrain = Some(TerrainState {
            kind,
            source,
            turns_remaining: turns.max(1),
        });
        previous
    }

    /// One residual count off the field condition `kind`: `Some(true)` when it ran out and is
    /// gone, `Some(false)` while it goes on, `None` when the field does not have it.
    pub fn count_down_condition(&mut self, kind: FieldConditionKind) -> Option<bool> {
        let index = self
            .conditions
            .iter()
            .position(|entry| entry.kind() == kind)?;
        let (FieldCondition::TrickRoom {
            turns_remaining, ..
        }
        | FieldCondition::EchoedVoice {
            turns_remaining, ..
        }
        | FieldCondition::Timed {
            turns_remaining, ..
        }) = &mut self.conditions[index];
        if *turns_remaining <= 1 {
            self.conditions.remove(index);
            Some(true)
        } else {
            *turns_remaining -= 1;
            Some(false)
        }
    }

    /// One residual count off the terrain: `Some((kind, true))` when it ran out and is gone,
    /// `Some((kind, false))` while it goes on, `None` when there is none.
    pub fn count_down_terrain(&mut self) -> Option<(TerrainKind, bool)> {
        let mut state = self.terrain?;
        if state.turns_remaining <= 1 {
            self.terrain = None;
            Some((state.kind, true))
        } else {
            state.turns_remaining -= 1;
            self.terrain = Some(state);
            Some((state.kind, false))
        }
    }

    pub fn tick_end_turn(&mut self) -> FieldExpiry {
        let mut conditions = Vec::new();
        self.conditions.retain_mut(|entry| {
            let kind = entry.kind();
            let (FieldCondition::TrickRoom {
                turns_remaining, ..
            }
            | FieldCondition::EchoedVoice {
                turns_remaining, ..
            }
            | FieldCondition::Timed {
                turns_remaining, ..
            }) = entry;
            if *turns_remaining <= 1 {
                conditions.push(kind);
                false
            } else {
                *turns_remaining -= 1;
                true
            }
        });

        // Weather ticks in the residual schedule (`tick_weather`), not here.
        let terrain = match self.terrain {
            Some(state) if state.turns_remaining <= 1 => {
                self.terrain = None;
                Some(state.kind)
            }
            Some(mut state) => {
                state.turns_remaining -= 1;
                self.terrain = Some(state);
                None
            }
            None => None,
        };

        FieldExpiry {
            conditions,
            terrain,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldExpiry {
    pub conditions: Vec<FieldConditionKind>,
    pub terrain: Option<TerrainKind>,
}

/// The count of a timed condition (Showdown `duration`), the one `VolatileConditions::tick_timed`
/// decrements; `None` for one with no count.
fn timed_turns(entry: &mut VolatileCondition) -> Option<&mut u8> {
    match entry {
        VolatileCondition::Taunt { turns }
        | VolatileCondition::Encore { turns, .. }
        | VolatileCondition::Disable { turns, .. }
        | VolatileCondition::TwoTurnMove { turns, .. }
        | VolatileCondition::MustRecharge { turns: Some(turns) }
        | VolatileCondition::MicleBerry { turns: Some(turns) }
        | VolatileCondition::PartiallyTrapped { turns, .. }
        | VolatileCondition::Stall { turns, .. }
        | VolatileCondition::AllySwitch { turns, .. }
        | VolatileCondition::Yawn { turns, .. }
        | VolatileCondition::HealBlock { turns }
        | VolatileCondition::Embargo { turns }
        | VolatileCondition::MagnetRise { turns }
        | VolatileCondition::Telekinesis { turns }
        | VolatileCondition::SyrupBomb { turns, .. }
        | VolatileCondition::ThroatChop { turns }
        | VolatileCondition::LaserFocus { turns }
        | VolatileCondition::SureHit { turns }
        | VolatileCondition::MagicCoat { turns, .. }
        | VolatileCondition::Snatch { turns }
        | VolatileCondition::MeFirst { turns }
        | VolatileCondition::LockOn { turns, .. }
        | VolatileCondition::SkyDrop { turns, .. }
        | VolatileCondition::FuryCutter {
            turns: Some(turns), ..
        }
        | VolatileCondition::ChillyReception { turns }
        | VolatileCondition::PartialTrappingLock { turns, .. }
        | VolatileCondition::FakePartiallyTrapped { turns, .. }
        | VolatileCondition::FocusPunch { turns, .. }
        | VolatileCondition::BeakBlast { turns }
        | VolatileCondition::ShellTrap { turns, .. }
        | VolatileCondition::Counter { turns }
        | VolatileCondition::MirrorCoat { turns }
        | VolatileCondition::Rolling { turns, .. }
        | VolatileCondition::Bide(BideState {
            turns: Some(turns), ..
        }) => Some(turns),
        _ => None,
    }
}
