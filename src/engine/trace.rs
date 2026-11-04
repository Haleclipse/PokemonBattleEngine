use crate::dex::{PokemonType, SecondaryEffect};
use crate::ids::{MoveId, PokemonId, PositionId, SideId};
use crate::rules::{DeclaredTarget, Fraction, ResolvedTarget, SemiInvulnerableKind};
use crate::state::{
    BattleOutcome, BattlePhase, FieldConditionKind, NonVolatileStatus, SideConditionKind,
    TerrainKind, VolatileConditionKind, WeatherKind,
};

use super::{
    BeforeMoveBlockReason, CanMoveResult, ConditionApplyError, EffectSource, HitBlockReason,
    MechanicChoice, ModifierSource, ModifierStage, Stat, StatChangeBlockReason, StatusBlockReason,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveFailReason {
    NoLegalTarget,
    /// The dex resolved the move as `PrimaryEffect::Unsupported`; validation normally refuses it.
    Unsupported,
    BeforeMove(BeforeMoveBlockReason),
    Condition(ConditionApplyError),
    /// A status move whose effect changed nothing on any target (Showdown `-fail`).
    NoEffect,
    /// An active Pokémon's ability stopped the move before it started (Damp:
    /// `cant|holder|ability: Damp|move`).
    Ability {
        holder: crate::ids::PokemonId,
        ability: crate::ids::AbilityId,
    },
    /// A strong weather stopped the move before it started (Desolate Land a Water attack,
    /// Primordial Sea a Fire one: `-fail|user|move|[from] Desolate Land`).
    Weather(WeatherKind),
    /// Bide took the turn (§15.10): storing energy (`-activate`), or releasing it — the move
    /// itself then counts as failed (its volatile's `onBeforeMove` returns `false`).
    Bide {
        released: bool,
    },
    /// Powder (`onTryMove`): the user's Fire move blew up in its face (`-activate|user|move:
    /// Powder`).
    Powder,
    /// The move had no PP left as its turn came (Spite or Eerie Spell took it after it was
    /// chosen): Showdown `cant|user|nopp|move`.
    NoPp,
    /// Gen 1's Desync Clause Mod (`PpRules::desync_clause`): the side's last selected move is not
    /// in the slot the action paid from; the move does not go on (`-hint` lines only).
    Desync,
}

/// What the line of an ability change names (`TraceEvent::AbilityChanged`, DESIGN.md §47.0
/// item 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityChangeLine {
    /// None, a forme's for good (`setAbility(species.abilities[0], …, isFromFormeChange)`): the
    /// forme's first ability, which the forme it shows has (`detailschange`), its own from then
    /// on (`baseAbility`, R-M10-40).
    Forme,
    /// None, a Transform's: the target's ability, which `Transformed` tells (DESIGN.md R-M10-40).
    Copied,
    /// Showdown `setAbility` from a move or an ability: `-ability|p|new|old|[from] effect|[of]
    /// source` (Mummy's and Lingering Aroma's `-activate|holder|ability: Mummy|p|[ability] old`):
    /// the old ability and the new, and `source`, where the line names one, has the new one —
    /// Role Play's target, Entrainment's user, Trace's target, a Receiver's fallen partner,
    /// Mummy's holder.
    Named { source: Option<PokemonId> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueReorderReason {
    DynamicSpeed,
    /// Round moved the next Round to the front (`queue.prioritizeAction`).
    Prioritized,
    /// Quash moved a Pokémon's action last.
    Quashed,
}

/// Why a major status ended outside of an explicit cure move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CureReason {
    WokeUp,
    Thawed,
    /// Shed Skin, Natural Cure.
    Ability(crate::ids::AbilityId),
    /// Heal Bell, Aromatherapy, Refresh.
    Move(MoveId),
    /// A status berry the Pokémon ate (Showdown `cureStatus()` from its `onEat`).
    Item(crate::ids::ItemId),
    /// The end-of-turn faint check (Showdown `checkFainted`: the status becomes `fnt`).
    Fainted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    TurnStarted {
        turn: u32,
    },
    ActionOrderResolved {
        actor: PokemonId,
        action_priority: i16,
        move_priority: i8,
        /// Tenths of a priority step (DESIGN.md §12.4).
        fractional_priority: i8,
        effective_speed: u32,
        trick_room: bool,
        tie_break: u64,
    },
    ActionQueueReordered {
        reason: QueueReorderReason,
    },
    /// A switch-in batch (the leads, simultaneous replacements, a drag) drew a SpeedTie
    /// decision for each of these Pokémon, in draw order, with the speed it shares with
    /// another: lets a replay plan the decisions from the order Showdown reported.
    SwitchInTiesDrawn {
        tied: Vec<(PokemonId, u32)>,
    },
    /// A move asked to move a Pokémon's pending action (After You, Quash: `-activate|target|move:
    /// …`); the queue changes once the action is over.
    QueueChangeRequested {
        request: crate::state::QueueRequest,
    },
    /// A faint in a Gen 1-3 singles battle cancelled these Pokémon's remaining actions
    /// (`FaintTurnEffect`).
    ActionsCancelledByFaint {
        actors: Vec<PokemonId>,
    },
    PhaseChanged {
        from: BattlePhase,
        to: BattlePhase,
    },
    /// An ability announced itself (`-ability`, `-activate|…|ability: …`, `[from] ability: …`):
    /// everyone sees it (DESIGN.md §47.0 item 12).
    AbilityActivated {
        source: PokemonId,
        ability: crate::ids::AbilityId,
    },
    /// An ability at work that shows itself to no one but its holder's side: Showdown says
    /// nothing (Magic Guard), marks the line `[silent]` (Natural Cure's cure), or shows it to its
    /// holder's side alone (Gen 3 Pressure); or the line names the ability's effect, not the
    /// ability (As One's Unnerve).
    AbilityAtWork {
        holder: PokemonId,
        ability: crate::ids::AbilityId,
    },
    /// Showdown `setAbility` (DESIGN.md §41.3), and what its line names (§47.0 item 16).
    AbilityChanged {
        pokemon: PokemonId,
        from: crate::ids::AbilityId,
        to: crate::ids::AbilityId,
        line: AbilityChangeLine,
    },
    /// Illusion's `onEnd`: the holder is seen as itself (`replace`, `-end|…|Illusion`).
    IllusionEnded {
        holder: PokemonId,
    },
    /// Showdown `transformInto` (`-transform|user|target`, Imposter's `[from] ability:`).
    Transformed {
        user: PokemonId,
        into: PokemonId,
        by: Option<crate::ids::AbilityId>,
        /// The moves it took, which its side sees in its requests (DESIGN.md §47.0 item 13).
        copied: [Option<MoveId>; 4],
    },
    /// Showdown `formeChange` (DESIGN.md §43.2: `-formechange`, or `detailschange` for a
    /// permanent change). `keeps_ability`: a permanent change Disguise or Ice Face makes, which
    /// leaves the ability as it was (`FormeChangeKind::PermanentKeepingAbility`).
    FormeChanged {
        pokemon: PokemonId,
        species: crate::ids::SpeciesId,
        permanent: bool,
        keeps_ability: bool,
        source: super::EffectSource,
    },
    /// Showdown `battle.skillSwap`: `named` the two abilities the line names before the trade
    /// (the source's, the target's) — Gen 5+ between foes; Gen 4, or two partners, leave them
    /// out (`-activate|source|Skill Swap|||[of] target`).
    AbilitiesSwapped {
        source: PokemonId,
        target: PokemonId,
        named: Option<(crate::ids::AbilityId, crate::ids::AbilityId)>,
    },
    ItemActivated {
        holder: PokemonId,
        item: crate::ids::ItemId,
    },
    MechanicActivated {
        side: SideId,
        pokemon: PokemonId,
        mechanic: MechanicChoice,
    },
    /// Dynamax started or ended (DESIGN.md §44.5): Showdown's `-start|p|Dynamax` / `-end|p|Dynamax`.
    DynamaxStarted {
        pokemon: PokemonId,
    },
    DynamaxEnded {
        pokemon: PokemonId,
    },
    /// A Z-Move broke through a protection (DESIGN.md §44.4): Showdown's `-zbroken|target`, its
    /// damage quartered.
    ProtectBroken {
        target: PokemonId,
    },
    /// A Z-Move (DESIGN.md §44.4): Showdown's `-zpower|p`, and the move it runs as. `crystal`:
    /// the Z-Crystal its user holds, which the Z-Move shows (the one of its type, or the
    /// signature one for its species, R-M10-40).
    ZPower {
        actor: PokemonId,
        base: MoveId,
        z_move: MoveId,
        crystal: Option<crate::ids::ItemId>,
    },
    /// Terastallization (DESIGN.md §44.6): Showdown's `-terastallize|p|Type`.
    Terastallized {
        pokemon: PokemonId,
        tera_type: crate::dex::PokemonType,
    },
    BeforeMoveResolved {
        actor: PokemonId,
        /// The action's move as a line naming it shows it (`OwnMove`): what a `cant` naming the
        /// move reveals (Disable, Taunt, Imprison …).
        own_move: Option<MoveId>,
        result: CanMoveResult,
    },
    /// A sleeping Pokémon's BeforeMove let its `sleepUsable` move go ahead (Snore, Sleep Talk:
    /// `cant|user|slp`, then the move).
    MovesWhileAsleep {
        actor: PokemonId,
    },
    /// An Encore that took hold after the choice replaced the chosen move (Showdown
    /// `OverrideAction`).
    MoveOverridden {
        actor: PokemonId,
        chosen: MoveId,
        used: MoveId,
    },
    /// Magic Coat or Magic Bounce took a move over (`-activate` and the reflected `move` line,
    /// DESIGN.md §42.8).
    MoveReflected {
        reflector: PokemonId,
        by: EffectSource,
    },
    /// Instruct made the target use its last move at once (`-singleturn|target|move: Instruct`,
    /// DESIGN.md §42.9).
    Instructed {
        target: PokemonId,
        instructor: PokemonId,
        move_id: MoveId,
    },
    /// Showdown `faint`'s `formeRegression` (DESIGN.md §44.3): a Mega Evolution, a Primal
    /// Reversion, a `-Tera` forme or Morpeko's Terastallization goes back to the set's species
    /// as the Pokémon faints (the details change is silent).
    FormeRegressed {
        pokemon: PokemonId,
        species: crate::ids::SpeciesId,
    },
    /// Mega Evolution or Ultra Burst (DESIGN.md §44.3): the `detailschange` and `-mega` /
    /// `-burst` the forme change makes, and what the frames' species then read.
    /// `item`: the Mega Stone or Z-Crystal the line names (`-mega|p|species|Audinite`): the
    /// forme's `requiredItem`, none for Mega Rayquaza (R-M10-62).
    MegaEvolved {
        pokemon: PokemonId,
        species: crate::ids::SpeciesId,
        ultra: bool,
        item: Option<crate::ids::ItemId>,
    },
    /// Dancer danced along (`-activate|dancer|ability: Dancer`, DESIGN.md §42.9).
    DancerActivated {
        dancer: PokemonId,
        move_id: MoveId,
    },
    /// A Pledge waited for its ally's (`-waiting`, DESIGN.md §42.9).
    PledgeWaiting {
        waiting: PokemonId,
        move_id: MoveId,
    },
    /// Two Pledges combined (`-combine`): the pair's moves.
    PledgeCombined {
        users: (PokemonId, MoveId),
        move_id: MoveId,
    },
    /// Ally Switch traded two Pokémon's slots (`-swap`, DESIGN.md §42.4).
    PositionsSwapped {
        side: crate::ids::SideId,
    },
    /// Mimic or Sketch rewrote a move slot (DESIGN.md §42.5).
    MoveSlotsRewritten {
        pokemon: PokemonId,
        slot: usize,
        move_id: MoveId,
        by: MoveId,
        /// Gen 1 Mimic drew the move among this Pokémon's (`this.sample(target.moves)`): it is
        /// seen to have it (R-M10-43).
        drawn_from: Option<PokemonId>,
    },
    /// Snatch took a move over (`-activate|snatcher|move: Snatch|[of] user`, DESIGN.md §42.8).
    MoveSnatched {
        snatcher: PokemonId,
        from: PokemonId,
        move_id: MoveId,
    },
    /// Forewarn named one of a foe's moves (`-activate|holder|ability: Forewarn|move|[of] foe`).
    /// Gen 4's line names no foe: observers tell which only when one stands (`identified`).
    MoveForewarned {
        holder: PokemonId,
        foe: PokemonId,
        move_id: MoveId,
        identified: bool,
    },
    /// Gen 3 Sleep Talk's `cant|p|nopp|<move>`: the drawn move has no PP left.
    MovedWithoutPp {
        actor: PokemonId,
        move_id: MoveId,
    },
    MoveUsed {
        actor: PokemonId,
        move_id: MoveId,
        declared_target: Option<DeclaredTarget>,
        /// The effect that made the Pokémon use this move (Metronome, Magic Coat …): the
        /// `[from]` of its log line (DESIGN.md §42.1).
        called_by: Option<crate::state::CallSource>,
        /// The move of its own the action uses, as its line shows it (DESIGN.md §47.0 item 13):
        /// the move itself, or the one a status Z-Move (`Z-…`) or a signature one was made from;
        /// none for a call, a lock's later turn, a two-turn move's attack, Struggle, and a
        /// generic Z-Move or a Max Move, which hide the move they were made from.
        own_move: Option<MoveId>,
    },
    PpSpent {
        actor: PokemonId,
        move_id: MoveId,
        remaining: u8,
    },
    /// The Pokémon's types changed (Forecast's forme, Mimicry: Showdown `-formechange`,
    /// `-start … typechange`).
    TypesChanged {
        pokemon: PokemonId,
        types: crate::state::PokemonTypes,
        source: super::EffectSource,
    },
    /// A type was added to the Pokémon's (Showdown `addType`, `-start … typeadd`: Forest's Curse,
    /// Trick-or-Treat), or copied with its types (Reflect Type).
    TypeAdded {
        pokemon: PokemonId,
        type_: crate::dex::PokemonType,
        source: super::EffectSource,
    },
    /// PP given back to a move slot (Leppa Berry, Mystery Berry).
    PpRestored {
        pokemon: PokemonId,
        move_id: MoveId,
        remaining: u8,
        source: super::EffectSource,
    },
    /// PP taken from a move slot by something else than its use (Grudge).
    PpReduced {
        pokemon: PokemonId,
        move_id: MoveId,
        remaining: u8,
        source: super::EffectSource,
    },
    /// Execution-time targeting changed the declared target: the target was gone (a random
    /// eligible foe), Follow Me, or a type-absorbing ability (Lightning Rod / Storm Drain).
    TargetRedirected {
        actor: PokemonId,
        move_id: MoveId,
        from: Option<PositionId>,
        to: PokemonId,
        by: EffectSource,
    },
    TargetsResolved {
        actor: PokemonId,
        move_id: MoveId,
        targets: Vec<ResolvedTarget>,
    },
    MoveFailed {
        actor: PokemonId,
        move_id: MoveId,
        reason: MoveFailReason,
    },
    /// Showdown `-prepare`: the action is spent charging; the attack comes next turn.
    ChargeStarted {
        actor: PokemonId,
        move_id: MoveId,
    },
    /// The charge started last turn is released: the attack goes out now.
    ChargeReleased {
        actor: PokemonId,
        move_id: MoveId,
    },
    /// Future Sight / Doom Desire foresaw an attack on `target`'s slot (`-start`, §15.22).
    FutureAttackForeseen {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
    },
    /// The foreseen attack strikes whoever stands in the slot (`-end … move: Future Sight`).
    FutureAttackStruck {
        source: PokemonId,
        target: PokemonId,
        move_id: MoveId,
    },
    /// The move attacks on its first turn: the weather (Solar Beam under sun) or a Power Herb
    /// (used up, `ItemConsumed` before this) let it.
    ChargeSkipped {
        actor: PokemonId,
        move_id: MoveId,
        cause: super::two_turn::ChargeSkipCause,
    },
    /// The target is in the air / underground and the move is not one that reaches it
    /// (Showdown `onInvulnerability` → `-miss`, before immunity, TryHit and accuracy).
    SemiInvulnerableDodged {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        kind: SemiInvulnerableKind,
    },
    HitBlocked {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        reason: HitBlockReason,
    },
    AccuracyResolved {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        base_percent: Option<u8>,
        final_basis_points: u16,
    },
    AccuracyChecked {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        chance: u8,
        hit: bool,
    },
    OffensiveStatResolved {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        base: u32,
        /// The stat once the ability's chain modifiers are in (the chain so far).
        after_ability: u32,
        /// The stat the formula uses: the whole `ModifyAtk` / `ModifySpA` chain applied once.
        after_item: u32,
    },
    DefensiveStatResolved {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        value: u32,
    },
    ModifierApplied {
        stage: ModifierStage,
        source: ModifierSource,
        priority: i16,
        from: u32,
        to: u32,
    },
    CriticalChecked {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        chance: crate::rules::Fraction,
        critical: bool,
    },
    WeatherModifierResolved {
        actor: PokemonId,
        move_id: MoveId,
        weather: Option<WeatherKind>,
        modifier: Fraction,
    },
    StabResolved {
        actor: PokemonId,
        move_id: MoveId,
        move_type: PokemonType,
        modifier: Fraction,
    },
    TypeEffectivenessResolved {
        target: PokemonId,
        move_id: MoveId,
        move_type: PokemonType,
        modifier: Fraction,
    },
    DamageCalculated {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        base_damage: u16,
        spread_applied: bool,
        critical: bool,
        weather_modifier: Fraction,
        stab_modifier: Fraction,
        type_effectiveness: Fraction,
        burn_applied: bool,
        random_percent: u16,
        final_damage: u16,
    },
    DamageApplied {
        target: PokemonId,
        amount: u16,
        hp_before: u16,
        hp_after: u16,
        source: EffectSource,
    },
    HealApplied {
        target: PokemonId,
        amount: u16,
        hp_before: u16,
        hp_after: u16,
        source: EffectSource,
    },
    /// Showdown `sethp` (`-sethp`, Pain Split): the HP set outright — no damage or heal event,
    /// nothing to stop or react to it.
    HpSet {
        target: PokemonId,
        hp_before: u16,
        hp_after: u16,
        source: EffectSource,
    },
    /// A move's HP price (`DamageKind::Cost`), committed by `apply_damage` as `DamageApplied`
    /// would be.
    HpCostPaid {
        target: PokemonId,
        amount: u16,
        hp_before: u16,
        hp_after: u16,
        source: EffectSource,
    },
    SubstituteDamaged {
        target: PokemonId,
        amount: u16,
        hp_before: u16,
        hp_after: u16,
        broken: bool,
    },
    StatChanged {
        target: PokemonId,
        stat: Stat,
        from: i8,
        to: i8,
        source: EffectSource,
    },
    /// ChangeBoost stage: the target's ability rewrote the requested stages (Contrary, Simple).
    StatChangeModified {
        target: PokemonId,
        stat: Stat,
        requested: i8,
        resolved: i8,
        by: EffectSource,
    },
    /// TryBoost stage: the request was refused (Clear Body …). `shown_by`: the ability or item
    /// the refusal's line names, with its holder (`-fail|p|unboost|[from] ability: Clear Body`,
    /// `-block|p|ability: Flower Veil|[of] holder`, `-ability|p|Mirror Armor`); none where it
    /// writes nothing (most refusals of a damaging move's secondary effect).
    StatChangeBlocked {
        target: PokemonId,
        stat: Stat,
        stages: i8,
        source: EffectSource,
        reason: StatChangeBlockReason,
        shown_by: Option<EffectSource>,
    },
    /// Damage resolution: a handler lowered the amount before commit (Sturdy, Focus Sash).
    DamageReduced {
        target: PokemonId,
        requested: u16,
        applied: u16,
        by: EffectSource,
    },
    /// Damage resolution: a handler refused the request (Magic Guard).
    DamagePrevented {
        target: PokemonId,
        amount: u16,
        source: EffectSource,
        by: EffectSource,
    },
    /// The holder used its item up (Showdown `useItem`: `-enditem`), or set it aside unseen
    /// (Natural Gift's `setItem('')`, no message: `seen` false).
    ItemConsumed {
        holder: PokemonId,
        item: crate::ids::ItemId,
        seen: bool,
    },
    /// An item taken from its holder (Showdown `takeItem`: Knock Off's `-enditem`, the theft's
    /// `[silent]` one followed by the thief's `-item`); `seen` false where nothing says so
    /// (Sticky Barb passing to a contact move's user).
    ItemTaken {
        holder: PokemonId,
        item: crate::ids::ItemId,
        taker: PokemonId,
        move_id: Option<MoveId>,
        seen: bool,
    },
    /// An item put in a Pokémon's hands (Showdown `setItem`: the thief's `-item`); `seen` false
    /// where nothing says so (Sticky Barb).
    ItemReceived {
        receiver: PokemonId,
        item: crate::ids::ItemId,
        from: PokemonId,
        seen: bool,
    },
    /// The item named itself as it took effect on its holder (Showdown `-item|holder|item`: an
    /// Air Balloon as its holder comes in or receives it).
    ItemAnnounced {
        holder: PokemonId,
        item: crate::ids::ItemId,
    },
    /// A swap left the holder empty-handed, which a line shows (Trick's `-enditem|holder|item|
    /// [silent]|[from] move: Trick` for the side given nothing; its takes say nothing).
    ItemSwappedAway {
        holder: PokemonId,
        item: crate::ids::ItemId,
    },
    /// Frisk named a foe's item (Showdown `-item|holder|item|[from] ability: Frisk|[of] by`).
    /// Gen 4-5's line leaves the holder out: `identified` says whether everyone can tell it
    /// all the same (the one foe standing).
    ItemRevealed {
        holder: PokemonId,
        item: crate::ids::ItemId,
        by: PokemonId,
        identified: bool,
    },
    /// The holder ate its berry (Showdown `eatItem`: `-enditem … [eat]`), or Cud Chew had it
    /// eat the one it ate before again.
    ItemEaten {
        holder: PokemonId,
        item: crate::ids::ItemId,
    },
    /// The holder got an item back (Harvest: `-item … [from] ability: Harvest`).
    ItemRestored {
        holder: PokemonId,
        item: crate::ids::ItemId,
        source: EffectSource,
    },
    StatusApplied {
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
    },
    /// `shown_by`: the ability its line names, with its holder (`-immune|p|[from] ability:
    /// Limber`, `-block|p|ability: Sweet Veil|[of] holder`); none where it names nothing.
    StatusBlocked {
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
        reason: StatusBlockReason,
        shown_by: Option<EffectSource>,
    },
    SecondaryEffectRolled {
        actor: PokemonId,
        affected: PokemonId,
        move_id: MoveId,
        effect: SecondaryEffect,
        chance_basis_points: u16,
        success: bool,
    },
    SecondaryEffectApplied {
        actor: PokemonId,
        affected: PokemonId,
        move_id: MoveId,
        effect: SecondaryEffect,
    },
    StatusCured {
        target: PokemonId,
        status: NonVolatileStatus,
        reason: CureReason,
    },
    /// The confusion self-hit; the HP change itself is the following `DamageApplied`.
    ConfusionSelfHit {
        target: PokemonId,
        damage: u16,
    },
    StatusResidual {
        target: PokemonId,
        status: NonVolatileStatus,
        damage: u16,
    },
    /// The residual scheduler is about to run one instance (ROADMAP §7.3); the order keys are
    /// the generation's, so a trace shows why Leftovers healed before poison ticked.
    ResidualEffect {
        holder: PokemonId,
        kind: crate::rules::ResidualKind,
        order: u8,
        sub_order: u8,
    },
    /// Spikes / Toxic Spikes gained a layer.
    SideConditionLayered {
        side: SideId,
        kind: SideConditionKind,
        layers: u8,
    },
    VolatileStarted {
        target: PokemonId,
        kind: VolatileConditionKind,
    },
    VolatileEnded {
        target: PokemonId,
        kind: VolatileConditionKind,
        /// Whether the observers see it end (DESIGN.md §47.0 item 13): a line (`-end`, `[silent]`
        /// too) or one that implies it (a forme change takes Autotomize's lightening); not a
        /// volatile Showdown removes with no word (Beak Blast's after the move, Snatch's as it
        /// steals, Gen 1 Bide's to a full paralysis or a self-hit).
        seen: bool,
    },
    SideConditionStarted {
        side: SideId,
        kind: SideConditionKind,
    },
    SideConditionEnded {
        side: SideId,
        kind: SideConditionKind,
    },
    /// Court Change: the two sides' conditions (all but the guards) traded places.
    SideConditionsSwapped,
    /// Baton Pass / Shed Tail: `to`, taking `from`'s place, took over its stat stages
    /// (`stages`, Baton Pass) and these volatiles — a set, so that the event is built whatever
    /// the trace keeps: what the observers saw passed on is theirs to know (DESIGN.md R-M10-26).
    PassedOn {
        from: PokemonId,
        to: PokemonId,
        stages: bool,
        volatiles: crate::state::VolatileSet,
        /// The kinds the hand-over lets through (Baton Pass's the generation's, Shed Tail's its
        /// Substitute): what observers saw on `from` goes on by kind, a trap whose trapper left
        /// unseen too (R-M10-41).
        lets_through: crate::state::VolatileSet,
    },
    /// Revival Blessing brought the fainted Pokémon back (`-heal|p2: Noibat|53/107|[from] move:
    /// Revival Blessing`): what observers see of it off the field, its HP, no status, as itself.
    Revived {
        pokemon: PokemonId,
        sighting: crate::state::Sighting,
    },
    /// Showdown `pokemon.faint()` (Explosion, Memento, Final Gambit; Perish Song's count): the
    /// HP went to 0 with no damage dealt.
    FaintedDirectly {
        pokemon: PokemonId,
        source: super::EffectSource,
    },
    /// Showdown `queue.cancelMove` (Gravity grounding a flyer): the actor's move this turn does
    /// not happen.
    MoveCancelled {
        actor: PokemonId,
    },
    FieldConditionStarted {
        kind: FieldConditionKind,
    },
    FieldConditionEnded {
        kind: FieldConditionKind,
    },
    WeatherChanged {
        from: Option<WeatherKind>,
        to: Option<WeatherKind>,
    },
    /// The weather's residual upkeep (Showdown `-weather X|[upkeep]`), after its tick; 0 turns
    /// means it never runs out.
    WeatherUpkeep {
        weather: WeatherKind,
        turns_remaining: u8,
    },
    /// Static / Flame Body / Poison Point rolled against the attacker of a contact hit.
    ContactReactionChecked {
        holder: PokemonId,
        attacker: PokemonId,
        ability: crate::ids::AbilityId,
        triggered: bool,
    },
    /// Shed Skin rolled at the end of the turn.
    ShedSkinChecked {
        holder: PokemonId,
        triggered: bool,
    },
    /// Sandstorm / hail chip about to be requested on a Pokémon that is not immune.
    WeatherDamage {
        target: PokemonId,
        weather: WeatherKind,
        damage: u16,
    },
    /// A type (Rock / Ground / Steel under sandstorm, Ice under hail) or being underground
    /// shields the Pokémon from the weather's chip.
    WeatherImmune {
        target: PokemonId,
        weather: WeatherKind,
    },
    TerrainChanged {
        from: Option<TerrainKind>,
        to: Option<TerrainKind>,
    },
    EntryHazardTriggered {
        side: SideId,
        target: PokemonId,
        kind: SideConditionKind,
        damage: u16,
    },
    /// A self-switch move landed; the user must be replaced before the turn continues.
    /// A multi-hit move decided how many times it will strike (before any hit lands; the
    /// sequence still stops early on a faint).
    MultiHitResolved {
        actor: PokemonId,
        target: PokemonId,
        move_id: MoveId,
        count: u8,
    },
    /// A phazer or Red Card ordered `target` out; the drag happens at the end of the action.
    DragOutRequested {
        target: PokemonId,
        source: EffectSource,
    },
    /// Suction Cups / Ingrain refused a drag-out (Showdown `DragOut` → `-activate`).
    DragOutBlocked {
        target: PokemonId,
        by: EffectSource,
    },
    /// Protect / Detect used right after a use: 1 / counter (Showdown `StallMove`).
    ProtectChecked {
        actor: PokemonId,
        counter: u16,
        success: bool,
    },
    /// The drag: `incoming` was picked at random from the bench and replaces `target`.
    DraggedOut {
        target: PokemonId,
        incoming: PokemonId,
    },
    SwitchOutRequested {
        actor: PokemonId,
        move_id: MoveId,
    },
    Switched {
        side: SideId,
        position: PositionId,
        outgoing: Option<PokemonId>,
        incoming: PokemonId,
        /// The Pokémon observers see come in: `incoming`, or the disguise its Illusion shows.
        seen_as: PokemonId,
    },
    /// A lead takes its place as the battle starts (Showdown `start`'s `|switch|`, DESIGN.md
    /// R-M10-16): what `Switched` is for a Pokémon that comes in later.
    LeadSentOut {
        pokemon: PokemonId,
        position: PositionId,
        /// The Pokémon observers see come in: the lead, or the disguise its Illusion shows.
        seen_as: PokemonId,
    },
    /// The Pokémon is leaving the field (before its `SwitchOut`): what observers last see of it
    /// — under `seen_as`, the disguise, for an Illusion's holder — before a heal it takes as it
    /// goes (Regenerator), which they do not see (DESIGN.md §47.0 item 12, M10.4c).
    LeftTheField {
        pokemon: PokemonId,
        seen_as: PokemonId,
        sighting: crate::state::Sighting,
    },
    Fainted {
        pokemon: PokemonId,
        position: PositionId,
    },
    /// A rule ended the battle at the turn's end (DESIGN.md §45.7); `BattleFinished` follows.
    BattleEndedByRule {
        rule: BattleEndRule,
    },
    BattleFinished {
        outcome: BattleOutcome,
    },
}

impl TraceEvent {
    /// The event's category, for a filtered trace (DESIGN.md §47.0 item 5). Exhaustive: an event
    /// added later is filed here first.
    pub fn category(&self) -> TraceCategory {
        match self {
            Self::TurnStarted { .. }
            | Self::ActionOrderResolved { .. }
            | Self::ActionQueueReordered { .. }
            | Self::QueueChangeRequested { .. }
            | Self::ActionsCancelledByFaint { .. }
            | Self::PhaseChanged { .. }
            | Self::BeforeMoveResolved { .. }
            | Self::ResidualEffect { .. } => TraceCategory::Flow,
            Self::SwitchInTiesDrawn { .. }
            | Self::AccuracyChecked { .. }
            | Self::CriticalChecked { .. }
            | Self::SecondaryEffectRolled { .. }
            | Self::ContactReactionChecked { .. }
            | Self::ShedSkinChecked { .. }
            | Self::MultiHitResolved { .. }
            | Self::ProtectChecked { .. } => TraceCategory::Check,
            Self::TargetsResolved { .. }
            | Self::AccuracyResolved { .. }
            | Self::OffensiveStatResolved { .. }
            | Self::DefensiveStatResolved { .. }
            | Self::ModifierApplied { .. }
            | Self::WeatherModifierResolved { .. }
            | Self::StabResolved { .. }
            | Self::TypeEffectivenessResolved { .. }
            | Self::DamageCalculated { .. } => TraceCategory::Calculation,
            Self::AbilityActivated { .. }
            | Self::AbilityAtWork { .. }
            | Self::AbilityChanged { .. }
            | Self::IllusionEnded { .. }
            | Self::Transformed { .. }
            | Self::FormeChanged { .. }
            | Self::AbilitiesSwapped { .. }
            | Self::ItemActivated { .. }
            | Self::MechanicActivated { .. }
            | Self::DynamaxStarted { .. }
            | Self::DynamaxEnded { .. }
            | Self::ProtectBroken { .. }
            | Self::ZPower { .. }
            | Self::Terastallized { .. }
            | Self::MovesWhileAsleep { .. }
            | Self::MoveOverridden { .. }
            | Self::MoveReflected { .. }
            | Self::Instructed { .. }
            | Self::FormeRegressed { .. }
            | Self::MegaEvolved { .. }
            | Self::DancerActivated { .. }
            | Self::PledgeWaiting { .. }
            | Self::PledgeCombined { .. }
            | Self::PositionsSwapped { .. }
            | Self::MoveSlotsRewritten { .. }
            | Self::MoveSnatched { .. }
            | Self::MovedWithoutPp { .. }
            | Self::MoveForewarned { .. }
            | Self::MoveUsed { .. }
            | Self::PpSpent { .. }
            | Self::TypesChanged { .. }
            | Self::TypeAdded { .. }
            | Self::PpRestored { .. }
            | Self::PpReduced { .. }
            | Self::TargetRedirected { .. }
            | Self::MoveFailed { .. }
            | Self::ChargeStarted { .. }
            | Self::ChargeReleased { .. }
            | Self::FutureAttackForeseen { .. }
            | Self::FutureAttackStruck { .. }
            | Self::ChargeSkipped { .. }
            | Self::SemiInvulnerableDodged { .. }
            | Self::HitBlocked { .. }
            | Self::DamageApplied { .. }
            | Self::HealApplied { .. }
            | Self::HpSet { .. }
            | Self::HpCostPaid { .. }
            | Self::SubstituteDamaged { .. }
            | Self::StatChanged { .. }
            | Self::StatChangeModified { .. }
            | Self::StatChangeBlocked { .. }
            | Self::DamageReduced { .. }
            | Self::DamagePrevented { .. }
            | Self::ItemConsumed { .. }
            | Self::ItemTaken { .. }
            | Self::ItemReceived { .. }
            | Self::ItemRevealed { .. }
            | Self::ItemAnnounced { .. }
            | Self::ItemSwappedAway { .. }
            | Self::ItemEaten { .. }
            | Self::ItemRestored { .. }
            | Self::StatusApplied { .. }
            | Self::StatusBlocked { .. }
            | Self::SecondaryEffectApplied { .. }
            | Self::StatusCured { .. }
            | Self::ConfusionSelfHit { .. }
            | Self::StatusResidual { .. }
            | Self::SideConditionLayered { .. }
            | Self::VolatileStarted { .. }
            | Self::VolatileEnded { .. }
            | Self::SideConditionStarted { .. }
            | Self::SideConditionEnded { .. }
            | Self::SideConditionsSwapped
            | Self::PassedOn { .. }
            | Self::Revived { .. }
            | Self::FaintedDirectly { .. }
            | Self::MoveCancelled { .. }
            | Self::FieldConditionStarted { .. }
            | Self::FieldConditionEnded { .. }
            | Self::WeatherChanged { .. }
            | Self::WeatherUpkeep { .. }
            | Self::WeatherDamage { .. }
            | Self::WeatherImmune { .. }
            | Self::TerrainChanged { .. }
            | Self::EntryHazardTriggered { .. }
            | Self::DragOutRequested { .. }
            | Self::DragOutBlocked { .. }
            | Self::DraggedOut { .. }
            | Self::SwitchOutRequested { .. }
            | Self::Switched { .. }
            | Self::LeadSentOut { .. }
            | Self::LeftTheField { .. }
            | Self::Fainted { .. }
            | Self::BattleEndedByRule { .. }
            | Self::BattleFinished { .. } => TraceCategory::Outcome,
        }
    }
}

/// The rules that end a battle at a turn's end (Showdown `maybeTriggerEndlessBattleClause`,
/// DESIGN.md §45.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleEndRule {
    /// Gen 1: the battle cannot progress (`FaintRules::no_progress_ties`). It is a tie.
    NoProgress,
    /// Turn 1000 is over, and the battle is a tie. This limit is always on.
    TurnLimit,
    /// Endless Battle Clause. `berry_cycling` names each side whose team started with the means
    /// to cycle restorative berries (P1, P2). Exactly one such side loses; otherwise it is a tie.
    EndlessBattle { berry_cycling: [bool; 2] },
}

/// What kind of thing a trace event tells (DESIGN.md §47.0 item 5): how the turn ran (`Flow`),
/// what happened in the battle (`Outcome`), a chance and how it fell (`Check`), the values behind
/// an outcome (`Calculation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceCategory {
    /// Turns, phases, the order of actions, the queue's changes, BeforeMove's verdicts, the
    /// residual's schedule.
    Flow,
    /// What happened: moves used, damage and healing, statuses, stat stages, switches, faints,
    /// the field — the battle as its players see it.
    Outcome,
    /// A chance drawn and its result: accuracy, critical hits, secondaries, contact reactions,
    /// Shed Skin, Protect's chain, ties, a multi-hit move's count.
    Check,
    /// The values behind an outcome: the stats, modifiers, effectiveness and damage computed.
    Calculation,
}

/// The categories a filtered trace keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TraceFilter {
    kept: u8,
}

impl TraceFilter {
    /// Keeps nothing yet.
    pub const fn new() -> Self {
        Self { kept: 0 }
    }

    /// Keeps `category` too.
    pub const fn keep(self, category: TraceCategory) -> Self {
        Self {
            kept: self.kept | Self::bit(category),
        }
    }

    pub const fn keeps(self, category: TraceCategory) -> bool {
        self.kept & Self::bit(category) != 0
    }

    const fn bit(category: TraceCategory) -> u8 {
        match category {
            TraceCategory::Flow => 1,
            TraceCategory::Outcome => 2,
            TraceCategory::Check => 4,
            TraceCategory::Calculation => 8,
        }
    }
}

/// What a battle's trace keeps (DESIGN.md §47.0 item 5). The engine never reads its trace
/// (R-M10-2), so the mode changes what is recorded, never the battle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TraceMode {
    /// Every event (rule debugging, the replay against Showdown).
    #[default]
    Full,
    /// The events of the categories the filter keeps.
    Filtered(TraceFilter),
    /// Nothing (rollouts, search).
    Off,
}

impl TraceEvent {
    /// Whether the event is one of the kinds `disclosures` may disclose something for: one test
    /// of the kind, which `push` asks every event before the table.
    #[inline]
    pub fn may_disclose(&self) -> bool {
        matches!(
            self,
            Self::Switched { .. }
                | Self::LeadSentOut { .. }
                | Self::IllusionEnded { .. }
                | Self::LeftTheField { .. }
                | Self::Fainted { .. }
                | Self::Revived { .. }
                | Self::StatusCured { .. }
                | Self::FormeChanged { .. }
                | Self::AbilityActivated { .. }
                | Self::AbilityChanged { .. }
                | Self::AbilitiesSwapped { .. }
                | Self::ItemActivated { .. }
                | Self::ItemRevealed { .. }
                | Self::ItemAnnounced { .. }
                | Self::ItemSwappedAway { .. }
                | Self::ItemReceived { .. }
                | Self::ItemRestored { .. }
                | Self::ItemConsumed { .. }
                | Self::ItemEaten { .. }
                | Self::ItemTaken { .. }
                | Self::DamageApplied { .. }
                | Self::HealApplied { .. }
                | Self::StatChanged { .. }
                | Self::StatChangeBlocked {
                    shown_by: Some(_),
                    ..
                }
                | Self::StatusBlocked {
                    shown_by: Some(_),
                    ..
                }
                | Self::DancerActivated { .. }
                | Self::MegaEvolved { item: Some(_), .. }
                | Self::ZPower {
                    crystal: Some(_),
                    ..
                }
                | Self::StatusApplied { .. }
                | Self::TypesChanged { .. }
                | Self::PpRestored { .. }
                | Self::MoveReflected { .. }
                | Self::TargetRedirected { .. }
                | Self::DragOutBlocked { .. }
                | Self::DamageReduced { .. }
                | Self::MoveUsed { .. }
                | Self::BeforeMoveResolved {
                    result: CanMoveResult::Blocked(_),
                    ..
                }
                | Self::MoveFailed {
                    reason: MoveFailReason::NoPp
                        | MoveFailReason::Ability { .. }
                        | MoveFailReason::BeforeMove(BeforeMoveBlockReason::LostFocus),
                    ..
                }
                | Self::MoveFailed {
                    move_id: MoveId::SHELL_TRAP,
                    reason: MoveFailReason::NoEffect,
                    ..
                }
                | Self::HitBlocked {
                    reason: HitBlockReason::Ability(_) | HitBlockReason::Item(_),
                    ..
                }
                | Self::MovedWithoutPp { .. }
                | Self::MoveForewarned { .. }
                | Self::PpReduced { .. }
                | Self::Transformed { .. }
                | Self::MoveSlotsRewritten { .. }
                | Self::VolatileStarted { .. }
                | Self::VolatileEnded { .. }
                | Self::PassedOn { .. }
                | Self::DynamaxStarted { .. }
                | Self::DynamaxEnded { .. }
                | Self::PhaseChanged {
                    from: BattlePhase::EndTurn,
                    ..
                }
        )
    }

    /// What the event shows everyone (DESIGN.md §47.0 item 12): the one table of the views'
    /// disclosures, at most two an event. An event `push_with` may leave unbuilt shows nothing
    /// (`tests/battle_view.rs`).
    pub fn disclosures(&self) -> [Option<crate::state::Disclosure>; 2] {
        use crate::state::Disclosure;
        let one = |disclosure| [disclosure, None];
        match self {
            Self::Switched {
                incoming: pokemon,
                seen_as,
                ..
            }
            | Self::LeadSentOut {
                pokemon, seen_as, ..
            } => one(Some(Disclosure::Appeared {
                pokemon: *pokemon,
                seen_as: *seen_as,
            })),
            // The holder is seen as itself.
            // `-end|holder|Illusion` names the ability.
            Self::IllusionEnded { holder } => [
                Some(Disclosure::Appeared {
                    pokemon: *holder,
                    seen_as: *holder,
                }),
                Some(Disclosure::AbilityShown(
                    *holder,
                    crate::ids::AbilityId::ILLUSION,
                )),
            ],
            Self::LeftTheField {
                pokemon,
                seen_as,
                sighting,
            } => one(Some(Disclosure::Left {
                pokemon: *pokemon,
                seen_as: *seen_as,
                sighting: *sighting,
            })),
            Self::Fainted { pokemon, .. } => one(Some(Disclosure::Fainted(*pokemon))),
            Self::Revived { pokemon, sighting } => {
                one(Some(Disclosure::Revived(*pokemon, *sighting)))
            }
            // A curing ability shows itself in its own event (Shed Skin, Hydration, Healer — the
            // last cures another); Natural Cure's line is `[silent]`.
            // A faint takes the status with no line of its own: an Illusion's holder fainting in
            // its guise is not seen cured (R-M10-44).
            Self::StatusCured {
                reason: CureReason::Fainted,
                ..
            } => [None, None],
            Self::StatusCured { target, .. } => one(Some(Disclosure::Cured(*target))),
            // A change for good names no ability (`detailschange`); one for the battle names
            // its ability (`-formechange … [from] ability: …`) but Zen Mode's, which goes through
            // its `zenmode` volatile, unnamed, and back `[silent]`, and Gulp Missile's, which the
            // move makes (`formeChange(forme, move)`: Surf, Dive).
            Self::FormeChanged {
                pokemon,
                species,
                permanent: true,
                keeps_ability,
                ..
            } => one(Some(Disclosure::FormeShown {
                pokemon: *pokemon,
                species: *species,
                keeps_ability: *keeps_ability,
            })),
            Self::FormeChanged {
                source:
                    EffectSource::Ability {
                        ability:
                            crate::ids::AbilityId::ZEN_MODE | crate::ids::AbilityId::GULP_MISSILE,
                        ..
                    },
                ..
            } => [None, None],
            Self::FormeChanged { source, .. } => one(shown_by(*source, false)),
            Self::AbilityActivated { source, ability } => {
                one(Some(Disclosure::AbilityShown(*source, *ability)))
            }
            // A change the event does not name (the move or ability that names it shows it
            // after, where it does).
            Self::AbilityChanged {
                pokemon,
                to,
                line: AbilityChangeLine::Forme,
                ..
            } => one(Some(Disclosure::OwnAbilityShown(*pokemon, *to))),
            Self::AbilityChanged {
                line: AbilityChangeLine::Copied,
                ..
            } => one(None),
            Self::AbilityChanged {
                pokemon,
                from,
                to,
                line: AbilityChangeLine::Named { source },
            } => [
                Some(Disclosure::AbilityReplaced {
                    pokemon: *pokemon,
                    old: *from,
                    new: *to,
                }),
                source.map(|source| Disclosure::AbilityShown(source, *to)),
            ],
            Self::AbilitiesSwapped {
                source,
                target,
                named: Some((mine, theirs)),
            } => [
                Some(Disclosure::AbilityReplaced {
                    pokemon: *source,
                    old: *mine,
                    new: *theirs,
                }),
                Some(Disclosure::AbilityReplaced {
                    pokemon: *target,
                    old: *theirs,
                    new: *mine,
                }),
            ],
            Self::AbilitiesSwapped {
                source,
                target,
                named: None,
            } => [
                Some(Disclosure::AbilityChanged(*source)),
                Some(Disclosure::AbilityChanged(*target)),
            ],
            // Eject Button, Red Card, a Custap Berry go off as they are used up.
            Self::ItemActivated { holder, item } => {
                one(Some(Disclosure::ItemActed(*holder, *item)))
            }
            // Frisk's line names the frisker's ability too (`[from] ability: Frisk|[of] by`), and
            // the holder where it can be told (Gen 4-5 leave it out among two foes).
            Self::ItemRevealed {
                holder,
                item,
                by,
                identified,
            } => [
                identified.then_some(Disclosure::ItemShown(*holder, *item)),
                Some(Disclosure::AbilityShown(*by, crate::ids::AbilityId::FRISK)),
            ],
            Self::ItemAnnounced { holder, item } => {
                one(Some(Disclosure::ItemShown(*holder, *item)))
            }
            // Harvest's `-item|holder|berry|[from] ability: Harvest` names it (R-M10-40).
            Self::ItemRestored {
                holder,
                item,
                source,
            } => [
                Some(Disclosure::ItemShown(*holder, *item)),
                shown_by(*source, false),
            ],
            Self::ItemReceived {
                receiver,
                item,
                seen: true,
                ..
            } => one(Some(Disclosure::ItemShown(*receiver, *item))),
            Self::ItemConsumed {
                holder,
                item,
                seen: true,
            }
            | Self::ItemSwappedAway { holder, item }
            | Self::ItemEaten { holder, item }
            | Self::ItemTaken {
                holder,
                item,
                seen: true,
                ..
            } => one(Some(Disclosure::ItemLost(*holder, *item))),
            // An item changing hands or used up unannounced (Sticky Barb passing on, Natural
            // Gift's berry, a swap's takes) shows nothing: what was seen stands (R-M10-40).
            // Regenerator heals in silence as its holder leaves.
            Self::HealApplied {
                source:
                    EffectSource::Ability {
                        ability: crate::ids::AbilityId::REGENERATOR,
                        ..
                    },
                ..
            } => [None, None],
            // An item at work the holder keeps (`[from] item: …`): Life Orb's, Rocky Helmet's,
            // Sticky Barb's damage, a Toxic Orb's status, Leftovers', Black Sludge's, Shell
            // Bell's heal. A berry's effect names it too, but the berry is gone by then — or
            // never was the eater's (Bug Bite, Pluck, Fling) — and its loss shows what there is.
            // Sticky Barb's residual names the holder's own, whatever it lost before (it came
            // back unannounced through a contact move, R-M10-40).
            Self::DamageApplied {
                target,
                source:
                    EffectSource::Item {
                        pokemon,
                        item: item @ crate::ids::ItemId::STICKY_BARB,
                    },
                ..
            } if pokemon == target => one(Some(Disclosure::ItemShown(*target, *item))),
            Self::DamageApplied { source, .. } => one(shown_by(*source, true)),
            // A status an ability inflicts names it in its own event, where the line does
            // (`BattleContext::inflict_status_by_ability`); an item's (Toxic Orb) is at work.
            Self::StatusApplied {
                source: source @ EffectSource::Item { .. },
                ..
            } => one(shown_by(*source, true)),
            Self::HealApplied { source, .. } => one(shown_by(
                *source,
                matches!(
                    source,
                    EffectSource::Item {
                        item: crate::ids::ItemId::LEFTOVERS
                            | crate::ids::ItemId::BLACK_SLUDGE
                            | crate::ids::ItemId::SHELL_BELL,
                        ..
                    }
                ),
            )),
            Self::StatChanged { source, .. } | Self::TypesChanged { source, .. } => {
                one(shown_by(*source, false))
            }
            Self::ZPower {
                actor,
                crystal: Some(crystal),
                ..
            }
            | Self::MegaEvolved {
                pokemon: actor,
                item: Some(crystal),
                ..
            } => one(Some(Disclosure::ItemShown(*actor, *crystal))),
            Self::DancerActivated { dancer, .. } => one(Some(Disclosure::AbilityShown(
                *dancer,
                crate::ids::AbilityId::DANCER,
            ))),
            // The refusal's line names the ability, or the item the holder keeps (Clear Amulet).
            Self::StatChangeBlocked {
                shown_by: Some(by), ..
            }
            | Self::StatusBlocked {
                shown_by: Some(by), ..
            } => one(shown_by(*by, true)),
            // A Leppa Berry names the move it restores.
            Self::PpRestored {
                pokemon,
                move_id,
                source,
                ..
            } => [
                shown_by(*source, false),
                Some(Disclosure::MoveShown(*pokemon, *move_id)),
            ],
            // Its own action's move (DESIGN.md §47.0 item 13); a move Sleep Talk calls is one of
            // the user's own. A `move` line is an action but a reflection's (Magic Coat, Magic
            // Bounce: no BeforeMove).
            Self::MoveUsed {
                actor,
                own_move: Some(own),
                ..
            } => [
                Some(Disclosure::MoveUsed(*actor, *own)),
                Some(Disclosure::Acted(*actor)),
            ],
            Self::MoveUsed {
                actor,
                move_id,
                called_by: Some(crate::state::CallSource::Move(MoveId::SLEEP_TALK)),
                ..
            } => [
                Some(Disclosure::MoveShown(*actor, *move_id)),
                Some(Disclosure::Acted(*actor)),
            ],
            Self::MoveUsed {
                called_by:
                    Some(
                        crate::state::CallSource::Move(MoveId::MAGIC_COAT)
                        | crate::state::CallSource::Ability(crate::ids::AbilityId::MAGIC_BOUNCE),
                    ),
                ..
            } => [None, None],
            Self::MoveUsed { actor, .. } => one(Some(Disclosure::Acted(*actor))),
            Self::MovedWithoutPp { actor, move_id }
            | Self::MoveFailed {
                actor,
                move_id,
                reason: MoveFailReason::NoPp,
            } => one(Some(Disclosure::MoveShown(*actor, *move_id))),
            // A `cant` naming the action's move; any `cant` is an action (Sky Drop's carried
            // Pokémon and Commander's Tatsugiri stay silent, and a confused Pokémon hitting
            // itself shows `-activate|p|confusion`, no `cant`).
            Self::BeforeMoveResolved {
                actor,
                own_move: Some(own),
                result:
                    CanMoveResult::Blocked(
                        BeforeMoveBlockReason::Disabled
                        | BeforeMoveBlockReason::Gravity
                        | BeforeMoveBlockReason::Taunt
                        | BeforeMoveBlockReason::Imprison
                        | BeforeMoveBlockReason::HealBlock,
                    ),
            } => [
                Some(Disclosure::MoveShown(*actor, *own)),
                Some(Disclosure::Acted(*actor)),
            ],
            // A Choice lock's refusal (Gen 7+ `choicelock.onBeforeMove`) is no `cant`: a `move`
            // line, `[still]`, then `-fail` — a use as anyone sees it, though no PP goes (that
            // it went nowhere would tell of the Choice item, R-M10-53).
            Self::BeforeMoveResolved {
                actor,
                own_move: Some(own),
                result: CanMoveResult::Blocked(BeforeMoveBlockReason::ChoiceLock),
            } => [
                Some(Disclosure::MoveUsed(*actor, *own)),
                Some(Disclosure::Acted(*actor)),
            ],
            // Focus Punch's lost focus, past BeforeMove (`beforeMoveCallback`: `cant|p|Focus
            // Punch|Focus Punch`, R-M10-41), an action; its charge named the move. The focus is
            // seen gone (the client's `cantUseMove`), though it lasts the turn (R-M10-43).
            Self::MoveFailed {
                actor,
                reason: MoveFailReason::BeforeMove(BeforeMoveBlockReason::LostFocus),
                ..
            } => [
                Some(Disclosure::Acted(*actor)),
                Some(Disclosure::VolatileEnded(
                    *actor,
                    VolatileConditionKind::FocusPunch,
                )),
            ],
            // Shell Trap not sprung (`onTryMove`: `cant|p|Shell Trap|Shell Trap`): the trap is
            // seen gone the same way, though it too lasts the turn.
            Self::MoveFailed {
                actor,
                move_id: MoveId::SHELL_TRAP,
                reason: MoveFailReason::NoEffect,
            } => one(Some(Disclosure::VolatileEnded(
                *actor,
                VolatileConditionKind::ShellTrap,
            ))),
            // Truant names itself (`cant|p|ability: Truant`).
            Self::BeforeMoveResolved {
                actor,
                result: CanMoveResult::Blocked(BeforeMoveBlockReason::Truant),
                ..
            } => [
                Some(Disclosure::AbilityShown(
                    *actor,
                    crate::ids::AbilityId::TRUANT,
                )),
                Some(Disclosure::Acted(*actor)),
            ],
            // An ability that refuses a move as it starts names itself and its holder (Damp:
            // `cant|user|ability: Damp|move|[of] holder`), or as it would hit (`-immune|target|[from]
            // ability: Levitate`, Volt Absorb's heal, Flash Fire's `-start`, Telepathy's
            // `-activate` …).
            Self::MoveFailed {
                reason: MoveFailReason::Ability { holder, ability },
                ..
            } => one(Some(Disclosure::AbilityShown(*holder, *ability))),
            Self::HitBlocked {
                target,
                reason: HitBlockReason::Ability(ability),
                ..
            } => one(Some(Disclosure::AbilityShown(*target, *ability))),
            // Safety Goggles' (`-activate|p|item: Safety Goggles|move`) and Ability Shield's
            // (`-block|p|item: Ability Shield`) refusals name the item (R-M10-46).
            Self::HitBlocked {
                target,
                reason: HitBlockReason::Item(item),
                ..
            } => one(Some(Disclosure::ItemShown(*target, *item))),
            Self::BeforeMoveResolved {
                result:
                    CanMoveResult::Blocked(
                        BeforeMoveBlockReason::SkyDropped
                        | BeforeMoveBlockReason::Commanding
                        | BeforeMoveBlockReason::Confusion
                        | BeforeMoveBlockReason::WakingTurn,
                    ),
                ..
            } => [None, None],
            Self::BeforeMoveResolved {
                actor,
                result: CanMoveResult::Blocked(_),
                ..
            } => one(Some(Disclosure::Acted(*actor))),
            // A volatile that shows itself as it starts (`-start`, `-singleturn`, `-singlemove`
            // …, DESIGN.md §47.0 item 13), and goes; a turn-start charge names the move too
            // (`-singleturn|p|move: Focus Punch`).
            Self::VolatileStarted { target, kind } => [
                Some(Disclosure::VolatileStarted(*target, *kind)),
                match kind {
                    VolatileConditionKind::FocusPunch => Some(MoveId::FOCUS_PUNCH),
                    VolatileConditionKind::ShellTrap => Some(MoveId::SHELL_TRAP),
                    VolatileConditionKind::BeakBlast => Some(MoveId::BEAK_BLAST),
                    _ => None,
                }
                .map(|move_id| Disclosure::MoveShown(*target, move_id)),
            ],
            Self::VolatileEnded {
                target,
                kind,
                seen: true,
            } => one(Some(Disclosure::VolatileEnded(*target, *kind))),
            // The turn's end is over (Showdown's `upkeep`), unless the battle ended in it.
            Self::PhaseChanged {
                from: BattlePhase::EndTurn,
                to,
            } if *to != BattlePhase::Finished => one(Some(Disclosure::TurnEnded)),
            // Dynamax has events of its own (`-start|p|Dynamax`, `-end|p|Dynamax`).
            Self::DynamaxStarted { pokemon } => one(Some(Disclosure::VolatileStarted(
                *pokemon,
                VolatileConditionKind::Dynamax,
            ))),
            Self::DynamaxEnded { pokemon } => one(Some(Disclosure::VolatileEnded(
                *pokemon,
                VolatileConditionKind::Dynamax,
            ))),
            Self::PassedOn {
                from,
                to,
                lets_through,
                ..
            } => one(Some(Disclosure::VolatilesPassed {
                from: *from,
                to: *to,
                kinds: *lets_through,
            })),
            Self::MoveForewarned {
                foe,
                move_id,
                identified: true,
                ..
            } => one(Some(Disclosure::MoveShown(*foe, *move_id))),
            // Pressure's PP goes unseen; Spite's, Eerie Spell's, Grudge's names the move.
            Self::PpReduced {
                source: EffectSource::Ability { .. },
                ..
            } => [None, None],
            Self::PpReduced {
                pokemon, move_id, ..
            } => one(Some(Disclosure::MoveShown(*pokemon, *move_id))),
            // Imposter names itself (`-transform … [from] ability: Imposter`).
            // Imposter's own name first: the ability is the target's after.
            Self::Transformed {
                user,
                into,
                by,
                copied,
            } => [
                by.map(|ability| Disclosure::AbilityShown(*user, ability)),
                Some(Disclosure::Transformed {
                    pokemon: *user,
                    into: *into,
                    copied: *copied,
                }),
            ],
            Self::MoveSlotsRewritten {
                pokemon,
                move_id,
                by,
                drawn_from,
                ..
            } => [
                Some(Disclosure::MoveCopied {
                    pokemon: *pokemon,
                    move_id: *move_id,
                    by: *by,
                }),
                drawn_from.map(|from| Disclosure::MoveShown(from, *move_id)),
            ],
            Self::MoveReflected { by, .. }
            | Self::TargetRedirected { by, .. }
            | Self::DragOutBlocked { by, .. }
            | Self::DamageReduced { by, .. } => one(shown_by(*by, true)),
            _ => [None, None],
        }
    }
}

/// What an effect at work shows of its source: an ability names itself (`[from] ability: …`);
/// an item, where `held` says the holder keeps it, is seen at work (`[from] item: …`).
fn shown_by(source: EffectSource, held: bool) -> Option<crate::state::Disclosure> {
    use crate::state::Disclosure;
    match source {
        EffectSource::Ability { pokemon, ability } => {
            Some(Disclosure::AbilityShown(pokemon, ability))
        }
        EffectSource::Item { pokemon, item } if held => Some(Disclosure::ItemActed(pokemon, item)),
        _ => None,
    }
}

impl TraceMode {
    /// Whether this mode keeps the events of `category`.
    pub const fn keeps(self, category: TraceCategory) -> bool {
        match self {
            Self::Full => true,
            Self::Filtered(filter) => filter.keeps(category),
            Self::Off => false,
        }
    }

    internal!(
        pub fn records(self, event: &TraceEvent) -> bool {
            self.keeps(event.category())
        }
    );
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BattleTrace {
    events: Vec<TraceEvent>,
    mode: TraceMode,
    /// What the events since control last went back to the caller disclosed, whatever the mode
    /// keeps (DESIGN.md §47.0 item 12): the engine moves it into the state's knowledge then.
    disclosed: Vec<crate::state::Disclosure>,
}

impl BattleTrace {
    /// An empty trace that keeps what `mode` says.
    pub fn with_mode(mode: TraceMode) -> Self {
        Self {
            events: Vec::new(),
            mode,
            disclosed: Vec::new(),
        }
    }

    pub fn push(&mut self, event: TraceEvent) {
        // The events that may disclose something, asked first: most push nothing here.
        debug_assert!(
            event.may_disclose() || event.disclosures() == [None, None],
            "{event:?}"
        );
        if event.may_disclose() {
            self.disclosed
                .extend(event.disclosures().into_iter().flatten());
        }
        if self.mode.records(&event) {
            self.events.push(event);
        }
    }

    /// What the events disclosed and the state's knowledge has not taken in yet.
    pub fn disclosed(&self) -> &[crate::state::Disclosure] {
        &self.disclosed
    }

    /// Takes the disclosures out, for the state's knowledge (the list kept for the next ones).
    pub fn drain_disclosed(&mut self) -> std::vec::Drain<'_, crate::state::Disclosure> {
        self.disclosed.drain(..)
    }

    /// `push` for an event that allocates to be built (DESIGN.md §47.5 item 5): built only
    /// when the mode keeps `category`, the event's own (checked in debug builds).
    pub fn push_with(&mut self, category: TraceCategory, event: impl FnOnce() -> TraceEvent) {
        if self.mode.keeps(category) {
            let event = event();
            debug_assert_eq!(event.category(), category, "{event:?} filed elsewhere");
            self.events.push(event);
        }
    }

    /// Whether the trace keeps the events of `category` (what building one is worth).
    pub fn keeps(&self, category: TraceCategory) -> bool {
        self.mode.keeps(category)
    }

    pub fn mode(&self) -> TraceMode {
        self.mode
    }

    /// What the trace keeps from now on; what it holds stays.
    pub fn set_mode(&mut self, mode: TraceMode) {
        self.mode = mode;
    }

    /// The events recorded so far, leaving the trace empty: a host that streams a battle's log
    /// takes them after each step.
    pub fn take_events(&mut self) -> Vec<TraceEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }
}
