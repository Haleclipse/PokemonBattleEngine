use crate::ids::{AbilityId, ItemId, MoveId, MoveSlot, PokemonId, SideId, SpeciesId};
use serde::{Deserialize, Serialize};

use crate::dex::PokemonType;

use super::{PokemonHistory, PokemonTypes, VolatileState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleStats {
    pub attack: u16,
    pub defense: u16,
    pub special_attack: u16,
    pub special_defense: u16,
    pub speed: u16,
}

impl BattleStats {
    /// One of the five in-battle stats (`None` for accuracy / evasion).
    pub const fn get(&self, stat: Stat) -> Option<u16> {
        match stat {
            Stat::Attack => Some(self.attack),
            Stat::Defense => Some(self.defense),
            Stat::SpecialAttack => Some(self.special_attack),
            Stat::SpecialDefense => Some(self.special_defense),
            Stat::Speed => Some(self.speed),
            Stat::Accuracy | Stat::Evasion => None,
        }
    }

    pub fn set(&mut self, stat: Stat, value: u16) {
        match stat {
            Stat::Attack => self.attack = value,
            Stat::Defense => self.defense = value,
            Stat::SpecialAttack => self.special_attack = value,
            Stat::SpecialDefense => self.special_defense = value,
            Stat::Speed => self.speed = value,
            Stat::Accuracy | Stat::Evasion => {}
        }
    }
}

impl Default for BattleStats {
    fn default() -> Self {
        Self {
            attack: 100,
            defense: 100,
            special_attack: 100,
            special_defense: 100,
            speed: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stat {
    Attack,
    Defense,
    SpecialAttack,
    SpecialDefense,
    Speed,
    Accuracy,
    Evasion,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatStages {
    pub attack: i8,
    pub defense: i8,
    pub special_attack: i8,
    pub special_defense: i8,
    pub speed: i8,
    pub accuracy: i8,
    pub evasion: i8,
}

impl StatStages {
    pub fn get(&self, stat: Stat) -> i8 {
        match stat {
            Stat::Attack => self.attack,
            Stat::Defense => self.defense,
            Stat::SpecialAttack => self.special_attack,
            Stat::SpecialDefense => self.special_defense,
            Stat::Speed => self.speed,
            Stat::Accuracy => self.accuracy,
            Stat::Evasion => self.evasion,
        }
    }

    pub fn set(&mut self, stat: Stat, value: i8) {
        let slot = match stat {
            Stat::Attack => &mut self.attack,
            Stat::Defense => &mut self.defense,
            Stat::SpecialAttack => &mut self.special_attack,
            Stat::SpecialDefense => &mut self.special_defense,
            Stat::Speed => &mut self.speed,
            Stat::Accuracy => &mut self.accuracy,
            Stat::Evasion => &mut self.evasion,
        };
        *slot = value;
    }
}

/// A Pokémon's gender (Showdown `gender`: `M`, `F`, or `''` for a genderless one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gender {
    Male,
    Female,
    Genderless,
}

impl Gender {
    /// One male and one female (Attract, Captivate, Cute Charm's infatuation).
    pub fn opposite(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Male, Self::Female) | (Self::Female, Self::Male)
        )
    }

    /// Both have a gender (Rivalry compares them only then).
    pub fn both_gendered(self, other: Self) -> bool {
        self != Self::Genderless && other != Self::Genderless
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NonVolatileStatus {
    Burn,
    Paralysis,
    Poison,
    Toxic,
    Sleep,
    Freeze,
}

/// Showdown's staleness (DESIGN.md §45.7): the Pokémon may be keeping the battle from ending.
/// Endless Battle Clause reads it at each turn's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Staleness {
    /// Its own doing: it ate a restorative berry that an item change had given it.
    Internal,
    /// A foe's doing: a foe healed it, it ate a Leppa Berry that came from another Pokémon, or a
    /// foe gave it an ability.
    External,
}

/// Runtime metadata for a major status condition.
///
/// Keeping lifecycle state beside the status prevents Toxic/Sleep counters from becoming unrelated
/// fields on `PokemonState`. More generation-specific counters can be added here without changing
/// the public meaning of `PokemonState::status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusState {
    #[serde(with = "crate::snapshot::wire")]
    pub kind: NonVolatileStatus,
    pub turns_active: u8,
    pub toxic_counter: u8,
    pub sleep_turns_remaining: Option<u8>,
    /// The sleep time rolled when the status was applied (Showdown `startTime`).
    pub sleep_turns_rolled: Option<u8>,
    /// Gen 3: the turns asleep spent on Snore / Sleep Talk since sleep last stopped the
    /// Pokémon (Showdown `skippedTime`), given back as it switches in.
    pub sleep_turns_skipped: u8,
    /// The side of the Pokémon whose doing the status is (Showdown `statusState.source`):
    /// Sleep Clause Mod counts a sleep a foe caused, not a Rest (DESIGN.md §45.7).
    #[serde(with = "crate::snapshot::wire")]
    pub inflicted_by: Option<crate::ids::SideId>,
}

impl StatusState {
    pub const fn new(kind: NonVolatileStatus) -> Self {
        Self {
            kind,
            turns_active: 0,
            toxic_counter: 0,
            sleep_turns_remaining: None,
            sleep_turns_rolled: None,
            sleep_turns_skipped: 0,
            inflicted_by: None,
        }
    }

    /// End-of-turn bookkeeping. The sleep counter is *not* touched here: Showdown decrements it
    /// at BeforeMove (`before_move.rs`), and so does the engine; the toxic counter (Showdown
    /// `stage`) advances when the poison damage itself runs, so a status set late in the
    /// residual schedule (Toxic Orb) starts at 1/16 the next turn.
    pub fn advance_turn(&mut self) {
        self.turns_active = self.turns_active.saturating_add(1);
    }
}

impl From<NonVolatileStatus> for StatusState {
    fn from(value: NonVolatileStatus) -> Self {
        Self::new(value)
    }
}

/// A hard lock (`PokemonState::hard_lock`): the move, the condition holding the Pokémon to it,
/// and the position the move aims at (a two-turn move's charge, a Rollout chain's first target).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveLock {
    pub move_id: MoveId,
    pub by: super::VolatileConditionKind,
    pub target: Option<crate::ids::PositionId>,
}

/// Mimic's rewrite of the move slots (Showdown `moveSlots` after `mimic`, DESIGN.md §42.3): the
/// slots the Pokémon battles with. Only the slots in `rewritten` are new in Showdown
/// (`moveSlots[i] = {…}`); the others are the own slots themselves — `clearVolatile`'s
/// `baseMoveSlots.slice()` shares them — so what is spent from them this stint stays when the
/// overlay goes (`PokemonState::drop_move_overlay`, DESIGN.md R-M8-54).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveOverlay {
    pub slots: Vec<MoveSlotState>,
    pub rewritten: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveSlotState {
    #[serde(with = "crate::snapshot::wire")]
    pub move_id: MoveId,
    pub pp: u8,
    pub max_pp: u8,
    /// Showdown `moveSlot.used`: the Pokémon spent PP on this move since it came in (Last
    /// Resort).
    pub used: bool,
    /// Showdown `moveSlot.target` where it is not the move's own: a Mimic's or Sketch's copy
    /// of a use whose `onModifyMove` aimed it elsewhere (a non-Ghost's Curse: its user). The
    /// request reads it — whether a target is chosen —, the move itself does not.
    #[serde(with = "crate::snapshot::wire")]
    pub target: Option<crate::rules::TargetPattern>,
}

impl MoveSlotState {
    pub const fn new(move_id: MoveId, pp: u8) -> Self {
        Self {
            move_id,
            pp,
            used: false,
            max_pp: pp,
            target: None,
        }
    }
}

/// Showdown `abilityState`: what the Pokémon's ability keeps between events. Entering the field
/// starts it over (`switchIn`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityState {
    /// Gorilla Tactics: the move it locked the Pokémon into (`choiceLock`).
    #[serde(with = "crate::snapshot::wire")]
    pub choice_lock: Option<MoveId>,
    /// Trace: still looking for an ability to copy (`seek`); every `Update` looks again.
    pub trace_seek: bool,
    /// Gen 9 Protean / Libero changed the holder's type this stint (`abilityState.protean`).
    pub protean_used: bool,
    /// Supreme Overlord: the side's fainted Pokémon when it came in, at most 5 (`fallen`).
    pub fallen: u8,
    /// Slow Start: the turn ends left before it wears off (`counter`); 0 once over.
    pub slow_start: u8,
    /// Unnerve, As One: started since the Pokémon came in (`unnerved`); while it is, foes eat
    /// no berries.
    pub unnerved: bool,
    /// Berserk, Anger Shell: a move that strikes once has hurt the Pokémon and its
    /// `AfterMoveSecondary` has not come yet (`!checkedBerserk`, `!checkedAngerShell`); until
    /// it does, the Pokémon eats no healing berry.
    pub hp_berries_wait: bool,
    /// Neutralizing Gas: its `End` has run (`ending`) — it suppresses nothing more until the
    /// holder comes in again.
    pub ending: bool,
    /// Gluttony: a Neutralizing Gas ending started it again and took its flag down
    /// (`abilityState.gluttony = false`): its pinch berries wait for a quarter of the HP until the
    /// holder takes damage (`onDamage` sets the flag) or the ability starts anew (R-M11-41).
    pub gluttony_waits: bool,
    /// Cud Chew: the berry the Pokémon ate and the turn ends left before it eats it again
    /// (`berry`, `counter`).
    #[serde(with = "crate::snapshot::wire")]
    pub cud_chew: Option<(ItemId, u8)>,
    /// Ripen: the berry the Pokémon last ate was a resist berry (`berryWeaken`); the next hit
    /// it takes is halved again, which uses the note up.
    pub berry_weaken: bool,
    /// Disguise / Ice Face (Showdown `effectState.busted`): a move has broken the forme that
    /// takes hits whole, and the `Update` has not put it back (DESIGN.md §43.4).
    pub busted: bool,
    /// Embody Aspect (Showdown `abilityState.embodied`, DESIGN.md §44.6): the Ogerpon `-Tera`
    /// forme has raised its stat this stint on the field — the entry resets the ability state
    /// (`switchIn`'s `initEffectState`), so coming back raises it again.
    pub embodied: bool,
    /// Opportunist: the raises its foes got since it last copied them (`boosts`).
    pub opportunist: KeptRaises,
    /// Showdown `abilityState.effectOrder`: when this state was started, in the battle's order
    /// (`BattleState::restart_ability_state`). Handlers of equal speed in a fast-exit event
    /// (`compareRedirectOrder`: Lightning Rod, Follow Me) run the earlier-started first.
    pub effect_order: u32,
}

/// Raises kept to be copied later (Opportunist's `effectState.boosts`): by stat, in the order
/// the stats first came, each the sum of what was kept of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRaises {
    #[serde(with = "crate::snapshot::wire")]
    raises: [Option<(Stat, i8)>; 7],
}

impl KeptRaises {
    /// Keep `stages` more of `stat`.
    pub fn add(&mut self, stat: Stat, stages: i8) {
        for slot in &mut self.raises {
            match slot {
                Some((kept, sum)) if *kept == stat => {
                    *sum = sum.saturating_add(stages);
                    return;
                }
                None => {
                    *slot = Some((stat, stages));
                    return;
                }
                Some(_) => {}
            }
        }
    }

    /// The raises kept, in order, leaving none.
    pub fn take(&mut self) -> Vec<(Stat, i8)> {
        std::mem::take(&mut self.raises)
            .into_iter()
            .flatten()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.raises[0].is_none()
    }
}

/// Showdown `itemState`: what the Pokémon's held item keeps between events. Entering the field
/// starts it over (`switchIn`), and so does losing the item (`useItem`, `eatItem`:
/// `clearEffectState`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemState {
    /// Mirror Herb: the raises the holder's foes got, by stat in the order they first came
    /// (`boosts`); the herb is ready once it kept one (`ready`).
    #[serde(with = "crate::snapshot::wire")]
    pub kept_raises: Vec<(Stat, i8)>,
    /// Eject Pack: a stage of the holder's went down (`eject`).
    pub eject: bool,
    /// Booster Energy: its `onStart` ran since the holder came in (`started`); until then its
    /// `onUpdate` does nothing.
    pub started: bool,
}

/// What Transform copied from its target (Showdown `transformInto`, DESIGN.md §41.7): read in
/// place of the Pokémon's own values while it stays on the field, and written to by whatever
/// changes them meanwhile (Soak, Skill Swap, Power Split, PP spent); leaving drops it
/// (`clearVolatile`). Boosts, critical-hit volatiles and Gen 1's modified stats are copied into
/// the Pokémon's own fields, which leaving clears anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformState {
    /// The Pokémon it transformed into.
    #[serde(with = "crate::snapshot::wire")]
    pub into: PokemonId,
    #[serde(with = "crate::snapshot::wire")]
    pub species: Option<SpeciesId>,
    /// The target's types before Roost took Flying out (`roost.typeWas`).
    pub types: PokemonTypes,
    /// The five stored stats; HP stays the Pokémon's own.
    pub stats: BattleStats,
    /// Gen 3+ the target's ability; Gen 1-2 the Pokémon's own (no abilities).
    #[serde(with = "crate::snapshot::wire")]
    pub ability: AbilityId,
    /// The target's moves with Transform's PP (`TransformRules::max_pp`).
    pub moves: Vec<MoveSlotState>,
    /// Gen 2-4 the target's Hidden Power; Gen 5+ the Pokémon's own.
    #[serde(with = "crate::snapshot::wire")]
    pub hidden_power: (PokemonType, u8),
}

/// One forme the Pokémon's species can take (DESIGN.md §43.1), with the values its set gives
/// it — Showdown's `setSpecies` computes them as the forme changes (`spreadModify(
/// species.baseStats, set)`); the team builder computes them once (`dex::build_pokemon`), so a
/// forme change needs no dex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forme {
    #[serde(with = "crate::snapshot::wire")]
    pub species: SpeciesId,
    pub types: PokemonTypes,
    /// The five stored stats the set gives this forme.
    pub stats: BattleStats,
    /// The max HP the set gives this forme (a permanent forme change's `updateMaxHp`).
    pub max_hp: u16,
    /// The forme's first ability (Showdown `species.abilities['0']`): a permanent forme change
    /// gives it.
    #[serde(with = "crate::snapshot::wire")]
    pub first_ability: AbilityId,
    /// The forme a battle-only forme goes back to (Showdown `battleOnly` / `changesFrom`).
    #[serde(with = "crate::snapshot::wire")]
    pub changes_from: Option<SpeciesId>,
    /// It is a mega forme (Showdown `isMega`, DESIGN.md §44.3): what a Mega Stone's or Mega
    /// Rayquaza's `canMegaEvo` looks for.
    pub mega: bool,
    /// The move this forme needs (Showdown `requiredMove`; Mega Rayquaza only).
    #[serde(with = "crate::snapshot::wire")]
    pub required_move: Option<crate::ids::MoveId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PokemonState {
    /// Read-only: `BattleState` keeps an id → index table, so the id must not change after
    /// construction. Every other field is public on purpose (see DESIGN.md §7.2).
    #[serde(with = "crate::snapshot::wire")]
    id: PokemonId,
    #[serde(with = "crate::snapshot::wire")]
    pub side: SideId,
    pub level: u8,
    pub hp: u16,
    pub max_hp: u16,
    pub stats: BattleStats,
    pub types: PokemonTypes,
    /// The types before something changed them this stint on the field (Forecast, Mimicry):
    /// Showdown's `setSpecies(baseSpecies)` as the Pokémon leaves the field puts them back.
    /// `None` while unchanged.
    pub types_before_change: Option<PokemonTypes>,
    /// Showdown `addedType`: a third type Forest's Curse or Trick-or-Treat added (one at a
    /// time); a type change and leaving the field clear it.
    #[serde(with = "crate::snapshot::wire")]
    pub added_type: Option<PokemonType>,
    #[serde(with = "crate::snapshot::wire")]
    pub ability: AbilityId,
    /// The ability before something changed it this stint on the field (Skill Swap, Role Play
    /// …): Showdown's `clearVolatile` writes `baseAbility` back as the Pokémon leaves the field.
    /// `None` while unchanged.
    #[serde(with = "crate::snapshot::wire")]
    pub ability_before_change: Option<AbilityId>,
    #[serde(with = "crate::snapshot::wire")]
    pub held_item: Option<ItemId>,
    /// From the set (`PokemonSet::gender`); genderless when it names none.
    #[serde(with = "crate::snapshot::wire")]
    pub gender: Gender,
    pub status: Option<StatusState>,
    pub stages: StatStages,
    pub volatiles: VolatileState,
    pub moves: Vec<MoveSlotState>,
    /// The last move this Pokémon actually used (Showdown `lastMove`): Disable locks onto it.
    /// Cleared on switch-out, and in Gen 1-2 whenever anyone enters.
    #[serde(with = "crate::snapshot::wire")]
    pub last_move: Option<MoveId>,
    /// Showdown `lastMoveUsed`: the last move this Pokémon used (a called one too), with the
    /// type it had as it was used (`ModifyType` edits the same object). Conversion 2 reads it.
    /// Cleared on switch-out.
    #[serde(with = "crate::snapshot::wire")]
    pub last_move_used: Option<(MoveId, PokemonType)>,
    /// Showdown `lastMoveTargetLoc`: the position the last move aimed at (Instruct aims the
    /// repeated move there, DESIGN.md §42.9).
    #[serde(with = "crate::snapshot::wire")]
    pub last_move_target: Option<crate::ids::PositionId>,
    /// Where the last move used went as its `onModifyMove` aimed it (Showdown `lastMove`, the
    /// active move: `lastMove.target`), with the move: Mimic and Sketch copy it into the slot.
    #[serde(with = "crate::snapshot::wire")]
    pub last_move_pattern: Option<(MoveId, crate::rules::TargetPattern)>,
    /// The move Encore locks onto (Showdown's Gen 2 `lastMoveEncore`, `lastMove` elsewhere): the
    /// same last move, which the Gen 1-2 entry reset leaves alone. Cleared on switch-out.
    #[serde(with = "crate::snapshot::wire")]
    pub last_move_encore: Option<MoveId>,
    /// Showdown `lastMove.baseMove`: the move a Max Move was made from (only Max Moves carry
    /// one, DESIGN.md §44.5). Encore, Spite, Grudge and Eerie Spell read the base move out of a
    /// `last_move` that is a Max Move. Kept in step with `last_move` at every write.
    #[serde(with = "crate::snapshot::wire")]
    pub last_move_base: Option<MoveId>,
    /// Showdown `lastMove.isZ` for a status move used as a Z-Move: its id is the move itself
    /// (a damaging Z-Move is a move of its own, `ResolvedMove::z_crystal`). Copycat, Encore,
    /// Disable, Mimic, Sketch, Mirror Move, Instruct, Spite and Eerie Spell refuse it as a Z-Move
    /// (DESIGN.md R-M9-36). Written with `last_move` by the move's own use.
    pub last_move_z: bool,
    /// Turns spent on the field since entering (Showdown `activeTurns`), advanced at the end
    /// of every turn; Taunt's duration depends on it.
    pub active_turns: u16,
    /// Showdown `activeMoveActions`: moves this Pokémon has tried since it came in (counted
    /// before BeforeMove); Fake Out only works while it is 1.
    pub active_move_actions: u16,
    /// Truant's bit as the cartridges keep it (`TruantRules`, DESIGN.md §15.21): Gen 3, the
    /// holder loafs at its next move attempt (pokeemerald `truantCounter`); Gen 4, it loafs on
    /// the turns of this parity, odd = true (pokeplatinum `moveEffectsData.truant`). Gen 5+ keep
    /// the `truant` volatile instead.
    pub truant_turn: bool,
    /// The Pokémon's own species (Showdown `baseSpecies`, or `set.species`: the held item's
    /// species check from Gen 5, Gen 1's critical chance, Beat Up's members). What it battles
    /// as is `engine::species_of` (Showdown `species`, DESIGN.md §41.7). `None` for a Pokémon
    /// built by hand.
    #[serde(with = "crate::snapshot::wire")]
    pub species: Option<SpeciesId>,
    /// Gen 1 (`StatRules::modified_stats`, Showdown `modifiedStats`): the in-battle value of
    /// each stat, kept on the Pokémon. `None` in every other generation.
    pub modified_stats: Option<BattleStats>,
    /// The stored stats as the Pokémon came in, while a move has rewritten `stats` (Power
    /// Split, Guard Split, Speed Swap, Power Trick: Showdown `storedStats`); leaving the field
    /// puts them back (`setSpecies` in `clearVolatile`).
    pub unchanged_stats: Option<BattleStats>,
    /// Return's power (Showdown `happiness`, 255 unless the set says otherwise).
    pub happiness: u8,
    /// The set's nature (Showdown `getNature()`): the confusion berries read the stat it lowers.
    pub nature: crate::dex::Nature,
    /// Hidden Power's type and (Gen 2-5) power from the set's IVs (Showdown `hpType`,
    /// `hpPower`): all-31 IVs give Dark 70.
    #[serde(with = "crate::snapshot::wire")]
    pub hidden_power: (crate::dex::PokemonType, u8),
    /// The forme the Pokémon battles as when it is not its own species (Showdown `species` after
    /// a `formeChange` that is not permanent: Stance Change, Zen Mode, Forecast …, DESIGN.md
    /// §43.1); leaving the field drops it (`setSpecies(baseSpecies)`). A permanent forme change
    /// rewrites `species` instead.
    #[serde(with = "crate::snapshot::wire")]
    pub forme: Option<SpeciesId>,
    /// The formes its species can take, with the values its set gives each (DESIGN.md §43.1);
    /// empty for a Pokémon built by hand, which then changes no forme.
    #[serde(with = "crate::snapshot::arc_slice")]
    pub formes: std::sync::Arc<[Forme]>,
    /// Showdown `dynamaxLevel` (0-10, DESIGN.md §44.5): how much HP a Dynamax adds
    /// (`1.5 + 0.05 × level`).
    pub dynamax_level: u8,
    /// The type Terastallization gives (Showdown `teraType`): the set's, else the species'
    /// first type (`PokemonSet::tera_type` is resolved as it is built). Gen 9 only.
    #[serde(with = "crate::snapshot::wire")]
    pub tera_type: PokemonType,
    /// Terastallization (DESIGN.md §44.1): the type it took, `None` while it has not. The types
    /// it reads are then that type alone (`types_of`, unless it is Stellar), and its base types
    /// still count for STAB and for the 60-power floor.
    #[serde(with = "crate::snapshot::wire")]
    pub terastallized: Option<PokemonType>,
    /// Stellar's per-type boost (Showdown `stellarBoostedTypes`): the types whose move already
    /// got the one-time boost; a Terapagos-Stellar keeps none (`getDamage`).
    #[serde(with = "crate::snapshot::wire")]
    pub stellar_boosted_types: Vec<PokemonType>,
    /// Transform's copy (DESIGN.md §41.7), read through the effective-value entries
    /// (`engine::species_of`, `types_of`, `raw_ability`, `move_slots`, `battle_stats`,
    /// `hidden_power_of`). `None` while the Pokémon is itself.
    pub transform: Option<Box<TransformState>>,
    /// Showdown `moveSlots` after Mimic rewrote one (§42.3): the slots the Pokémon battles with,
    /// thrown away on switch-out (Mimic's move is forgotten, the PP spent from the other slots
    /// stays: `MoveOverlay`). `None` while nothing rewrote them. Sketch writes the own slots
    /// instead (it is permanent).
    pub move_overlay: Option<MoveOverlay>,
    /// Illusion's disguise (Showdown `pokemon.illusion`, DESIGN.md §41.7): the party member the
    /// Pokémon appears as to the other side (`BattleEngine::view_for`). Nothing else reads it but
    /// Transform, which fails on either side of one. Chosen as it enters, gone as it is hit,
    /// suppressed or faints; one that leaves keeps it until it comes in again (`beingCalledBack`:
    /// a cure off the field names the disguise, R-M11-51).
    #[serde(with = "crate::snapshot::wire")]
    pub illusion: Option<PokemonId>,
    /// Battle Bond (Showdown `bondTriggered`): it has raised its holder once this battle
    /// (DESIGN.md §43.4).
    pub bond_triggered: bool,
    /// Zero to Hero (Showdown `heroMessageDisplayed`): its Hero forme has announced itself
    /// once since it changed (DESIGN.md §43.4).
    pub hero_message_displayed: bool,
    /// The species the set named (Showdown `pokemon.set.species`): what a forme with
    /// `forme_regression` goes back to as it faints (DESIGN.md §44.3).
    #[serde(with = "crate::snapshot::wire")]
    pub set_species: Option<SpeciesId>,
    /// The ability the set named (Showdown `pokemon.set.ability`, the reverting faint's
    /// `baseAbility`).
    #[serde(with = "crate::snapshot::wire")]
    pub set_ability: AbilityId,
    /// The item the set named (Showdown `pokemon.set.item`): Endless Battle Clause's
    /// berry-cycling check (DESIGN.md §45.7).
    #[serde(with = "crate::snapshot::wire")]
    pub set_item: Option<ItemId>,
    /// The moves the set named (Showdown `pokemon.set.moves`), whatever Sketch writes since.
    #[serde(with = "crate::snapshot::wire")]
    pub set_moves: Vec<MoveId>,
    /// Showdown `staleness` (DESIGN.md §45.7), kept for the whole battle.
    pub staleness: Option<Staleness>,
    /// Showdown `volatileStaleness`: a foe gave the Pokémon its ability (Skill Swap, Wandering
    /// Spirit, Entrainment). It lasts while the Pokémon stays on the field.
    pub volatile_staleness: Option<Staleness>,
    /// Showdown `pendingStaleness`: what the Pokémon holds came from an item change
    /// (`setItem`), and eating a restorative berry would make it stale.
    /// - Taking the item away clears it.
    /// - It is always internal. Showdown's external value needs a `source`, and no caller of
    ///   `setItem` passes one: Trick and Switcheroo call `setItem(item)`.
    pub pending_staleness: bool,
    /// Showdown `formeRegression` (DESIGN.md §44.3): the forme a permanent change took — a
    /// Mega Evolution, a Primal Reversion, a `-Tera` forme, Morpeko's Terastallization — goes
    /// back to the set's species as the Pokémon faints.
    pub forme_regression: bool,
    /// Showdown `canMegaEvo` (DESIGN.md §44.3): the forme this Pokémon may Mega Evolve into,
    /// fixed as the battle is built (the species, its moves and the stone it holds then).
    #[serde(with = "crate::snapshot::wire")]
    pub mega_evolves_into: Option<SpeciesId>,
    /// Showdown `canUltraBurst` (DESIGN.md §44.3): Necrozma's Ultra forme, the same way.
    #[serde(with = "crate::snapshot::wire")]
    pub ultra_bursts_into: Option<SpeciesId>,
    /// What happened to and by the Pokémon recently (DESIGN.md §22.8).
    pub history: PokemonHistory,
    pub ability_state: AbilityState,
    pub item_state: ItemState,
}

impl PokemonState {
    /// A level-100, 100 HP, all-stats-100, mono-Normal Pokémon with no ability, item, status or
    /// moves. Callers set the fields they care about; tests and fixtures stay short.
    pub fn new(id: PokemonId, side: SideId) -> Self {
        Self {
            id,
            side,
            level: 100,
            hp: 100,
            max_hp: 100,
            stats: BattleStats::default(),
            types: PokemonTypes::mono(PokemonType::Normal),
            types_before_change: None,
            last_move_used: None,
            added_type: None,
            ability_before_change: None,
            ability: AbilityId::NONE,
            held_item: None,
            gender: Gender::Genderless,
            status: None,
            stages: StatStages::default(),
            volatiles: VolatileState::default(),
            moves: Vec::new(),
            last_move: None,
            last_move_target: None,
            last_move_pattern: None,
            last_move_encore: None,
            last_move_base: None,
            last_move_z: false,
            active_turns: 0,
            active_move_actions: 0,
            truant_turn: false,
            species: None,
            modified_stats: None,
            unchanged_stats: None,
            happiness: 255,
            nature: crate::dex::Nature::Hardy,
            hidden_power: (crate::dex::PokemonType::Dark, 70),
            forme: None,
            formes: std::sync::Arc::from([]),
            dynamax_level: 10,
            tera_type: PokemonType::Normal,
            terastallized: None,
            stellar_boosted_types: Vec::new(),
            transform: None,
            move_overlay: None,
            illusion: None,
            bond_triggered: false,
            hero_message_displayed: false,
            set_species: None,
            set_ability: AbilityId::NONE,
            set_item: None,
            set_moves: Vec::new(),
            staleness: None,
            volatile_staleness: None,
            pending_staleness: false,
            forme_regression: false,
            mega_evolves_into: None,
            ultra_bursts_into: None,
            history: PokemonHistory::default(),
            ability_state: AbilityState::default(),
            item_state: ItemState::default(),
        }
    }

    /// Gen 1 `modifyStat`: the modified stat times `numerator / denominator`, floored, at
    /// least 1. No-op outside Gen 1's model.
    pub fn modify_stat(&mut self, stat: Stat, numerator: u32, denominator: u32) {
        if let Some(modified) = &mut self.modified_stats
            && let Some(value) = modified.get(stat)
        {
            let next = (u32::from(value) * numerator / denominator.max(1)).max(1);
            modified.set(stat, next.min(u32::from(u16::MAX)) as u16);
        }
    }

    /// The stats the Pokémon battles with (Showdown `storedStats`, DESIGN.md §41.7): the
    /// damage, the speed order and the stat-swapping moves read them. Its own are `stats`.
    pub fn battle_stats(&self) -> &BattleStats {
        self.transform
            .as_ref()
            .map_or(&self.stats, |transform| &transform.stats)
    }

    /// A move rewrites one stored stat (Showdown `storedStats[stat] = value`), keeping the
    /// stats it came in with for `restore_stored_stats` — or, transformed, Transform's copy.
    pub fn set_stored_stat(&mut self, stat: Stat, value: u16) {
        if let Some(transform) = &mut self.transform {
            transform.stats.set(stat, value);
            return;
        }
        self.unchanged_stats.get_or_insert(self.stats);
        self.stats.set(stat, value);
    }

    /// Showdown `setSpecies(baseSpecies)` as the Pokémon leaves the field: stored stats a move
    /// rewrote go back.
    pub fn restore_stored_stats(&mut self) {
        if let Some(stats) = self.unchanged_stats.take() {
            self.stats = stats;
        }
    }

    /// Gen 1 (Showdown `setSpecies` on switch-out, and the start of the battle): the modified
    /// stats return to the stored ones, then paralysis quarters Speed and burn halves Attack.
    pub fn reset_modified_stats(&mut self) {
        if self.modified_stats.is_none() {
            return;
        }
        self.modified_stats = Some(*self.battle_stats());
        self.apply_status_stat_cut();
    }

    /// Gen 1: paralysis quarters the modified Speed, burn halves the modified Attack.
    pub fn apply_status_stat_cut(&mut self) {
        match self.status.map(|status| status.kind) {
            Some(NonVolatileStatus::Paralysis) => self.modify_stat(Stat::Speed, 1, 4),
            Some(NonVolatileStatus::Burn) => self.modify_stat(Stat::Attack, 1, 2),
            _ => {}
        }
    }

    /// Showdown `baseMaxhp`: the max HP without a Dynamax doubling.
    pub fn base_max_hp(&self) -> u16 {
        self.volatiles.dynamax_base_max_hp().unwrap_or(self.max_hp)
    }

    /// Showdown `getUndynamaxedHP(amount)`: the HP as it would read without the Dynamax
    /// doubling (DESIGN.md §44.5) — `ceil(hp × baseMaxhp / maxhp)`.
    pub fn undynamaxed_hp(&self, amount: Option<u16>) -> u16 {
        let hp = amount.unwrap_or(self.hp);
        match self.volatiles.dynamax_base_max_hp() {
            Some(base) if self.max_hp > 0 => {
                let scaled = u32::from(hp) * u32::from(base);
                scaled.div_ceil(u32::from(self.max_hp)) as u16
            }
            _ => hp,
        }
    }

    /// The move slots the Pokémon battles with (Showdown `moveSlots`, DESIGN.md §41.7): the
    /// request offers them, PP is spent from them, Disable and Encore name them. Its own
    /// (`baseMoveSlots`) are `moves`; Transform's copy comes first, then Mimic's overlay
    /// (`moveSlots` after `mimic`, §42.3 — Showdown keeps them apart from `baseMoveSlots`).
    pub fn move_slots(&self) -> &[MoveSlotState] {
        if let Some(transform) = &self.transform {
            return &transform.moves;
        }
        match &self.move_overlay {
            Some(overlay) => &overlay.slots,
            None => &self.moves,
        }
    }

    pub fn move_slots_mut(&mut self) -> &mut [MoveSlotState] {
        if let Some(transform) = &mut self.transform {
            return &mut transform.moves;
        }
        match &mut self.move_overlay {
            Some(overlay) => &mut overlay.slots,
            None => &mut self.moves,
        }
    }

    /// The move slots Mimic's overlay holds, if any (§42.3).
    pub fn move_overlay(&self) -> Option<&[MoveSlotState]> {
        self.move_overlay
            .as_ref()
            .map(|overlay| overlay.slots.as_slice())
    }

    /// Gen 1: Mimic's move shares its slot's PP byte (Showdown gen1 `deductPP`: a `virtual` slot
    /// syncs its PP into `baseMoveSlots` unless the Pokémon has transformed), so the own slot
    /// keeps what was spent from the copy once the overlay goes.
    pub fn sync_mimicked_pp(&mut self, slot: usize) {
        if self.transform.is_some() {
            return;
        }
        if let Some(overlay) = &self.move_overlay
            && overlay.rewritten.contains(&slot)
            && let (Some(copy), Some(own)) = (overlay.slots.get(slot), self.moves.get_mut(slot))
        {
            own.pp = copy.pp;
        }
    }

    /// `clearVolatile` (`moveSlots = baseMoveSlots.slice()`): Mimic's overlay goes. The slots it
    /// did not rewrite were the own ones all along in Showdown, so the PP spent from them this
    /// stint stays with the Pokémon.
    /// Showdown `baseMoveSlots[index]`: the own slot — or, under Mimic's overlay, the overlay's
    /// copy of an own slot Mimic did not rewrite (one object with it in Showdown,
    /// `moveSlots = baseMoveSlots.slice()`), which `drop_move_overlay` writes back (R-M8-75).
    pub fn base_slot_mut(&mut self, index: usize) -> Option<&mut MoveSlotState> {
        match &mut self.move_overlay {
            Some(overlay) if !overlay.rewritten.contains(&index) => overlay.slots.get_mut(index),
            _ => self.moves.get_mut(index),
        }
    }

    /// The own move slots (Showdown `baseMoveSlots`, the party's moves and PP): Transform's
    /// copy and Mimic's move left out, an own slot under Mimic's overlay as it stands now
    /// (`base_slot_mut`).
    pub fn base_slots(&self) -> impl Iterator<Item = &MoveSlotState> {
        self.moves
            .iter()
            .enumerate()
            .map(|(index, own)| match &self.move_overlay {
                Some(overlay) if !overlay.rewritten.contains(&index) => {
                    overlay.slots.get(index).unwrap_or(own)
                }
                _ => own,
            })
    }

    pub fn drop_move_overlay(&mut self) {
        let Some(overlay) = self.move_overlay.take() else {
            return;
        };
        for (index, slot) in overlay.slots.into_iter().enumerate() {
            if !overlay.rewritten.contains(&index)
                && let Some(own) = self.moves.get_mut(index)
            {
                *own = slot;
            }
        }
    }

    /// Showdown `lastMove` with `baseMove` resolved (DESIGN.md §44.5): a Max Move reads as the
    /// move it was made from — what Spite, Grudge and Eerie Spell act on.
    pub fn last_move_resolved(&self) -> Option<MoveId> {
        self.last_move_base.or(self.last_move)
    }

    /// The same for Encore's own read (`lastMove`, or `lastMoveEncore` in Gen 1-2).
    pub fn last_move_encore_resolved(&self) -> Option<MoveId> {
        self.last_move_base.or(self.last_move_encore)
    }

    /// Mimic wrote `slot` (Showdown `moveSlots[mimicIndex]`): the overlay starts as a copy of
    /// the own slots, so the other moves stay where they are. A slot rewritten for the first time
    /// under an existing overlay was still the own one in Showdown: what was spent from it so far
    /// goes back to the own slot before the new move takes its place.
    pub fn set_move_overlay(&mut self, slot: usize, entry: MoveSlotState) {
        // Transformed (Gen 1 Mimic, which goes ahead then): the slots it battles with are the
        // copies Transform made (Showdown `moveSlots`), gone with it (R-M8-75).
        if let Some(transform) = &mut self.transform {
            if let Some(target) = transform.moves.get_mut(slot) {
                *target = entry;
            }
            return;
        }
        let overlay = self.move_overlay.get_or_insert_with(|| MoveOverlay {
            slots: self.moves.clone(),
            rewritten: Vec::new(),
        });
        let Some(target) = overlay.slots.get_mut(slot) else {
            return;
        };
        if !overlay.rewritten.contains(&slot) {
            if let Some(own) = self.moves.get_mut(slot) {
                *own = *target;
            }
            overlay.rewritten.push(slot);
        }
        *target = entry;
    }

    /// Sketch (Showdown writes `moveSlots` and `baseMoveSlots`, §42.5): the move is learned for
    /// good — the overlay goes with it when it holds the slot.
    pub fn set_learned_move(&mut self, slot: usize, entry: MoveSlotState) {
        if let Some(target) = self.moves.get_mut(slot) {
            *target = entry;
        }
        if let Some(overlay) = self.move_overlay.as_mut()
            && let Some(target) = overlay.slots.get_mut(slot)
        {
            *target = entry;
        }
    }

    /// The stats the Pokémon's own set gives it (Showdown `baseStoredStats`): neither Transform's
    /// copy nor a move that rewrote the stored stats this stint is seen. Gen 1's critical hits
    /// read it (`getStat(stat, unmodified)`, DESIGN.md R279).
    pub fn own_stats(&self) -> &BattleStats {
        self.unchanged_stats.as_ref().unwrap_or(&self.stats)
    }

    /// The types the Pokémon battles with before Roost and an added type (Showdown `types`):
    /// its own, or Transform's copy.
    pub fn battle_types(&self) -> PokemonTypes {
        self.transform
            .as_ref()
            .map_or(self.types, |transform| transform.types)
    }

    /// The ability the Pokémon has (Showdown `ability`): its own, or Transform's copy.
    pub fn battle_ability(&self) -> AbilityId {
        self.transform
            .as_ref()
            .map_or(self.ability, |transform| transform.ability)
    }

    /// The Hidden Power type and power the Pokémon uses (Showdown `hpType` / `hpPower`).
    pub fn hidden_power_of(&self) -> (PokemonType, u8) {
        self.transform
            .as_ref()
            .map_or(self.hidden_power, |transform| transform.hidden_power)
    }

    pub fn move_slot_of(&self, move_id: MoveId) -> Option<&MoveSlotState> {
        self.move_slots()
            .iter()
            .find(|slot| slot.move_id == move_id)
    }

    /// A forme's type change (Forecast, Mimicry): false when the types are already these —
    /// with no added type (Showdown compares `getTypes()`, which counts Trick-or-Treat's or
    /// Forest's Curse's; `setType` clears it, R-M8-114).
    pub fn change_types(&mut self, types: PokemonTypes) -> bool {
        if self.battle_types() == types && self.added_type.is_none() {
            return false;
        }
        self.set_types(types);
        true
    }

    /// Showdown `setType`'s write: the types become these, the ones it entered with are kept
    /// for its leaving, and an added type goes (`addedType = ''`). A transformed Pokémon's
    /// change is to Transform's copy.
    pub fn set_types(&mut self, types: PokemonTypes) {
        if let Some(transform) = &mut self.transform {
            transform.types = types;
        } else {
            self.types_before_change.get_or_insert(self.types);
            self.types = types;
        }
        self.added_type = None;
    }

    /// The types the Pokémon entered the field with (Showdown `baseSpecies.types`).
    pub fn base_types(&self) -> PokemonTypes {
        self.types_before_change.unwrap_or(self.types)
    }

    /// An ability change (Showdown `setAbility`): the new ability, the old one returned.
    pub fn change_ability(&mut self, ability: AbilityId) -> AbilityId {
        if let Some(transform) = &mut self.transform {
            return std::mem::replace(&mut transform.ability, ability);
        }
        let old = self.ability;
        self.ability_before_change.get_or_insert(old);
        self.ability = ability;
        old
    }

    /// Leaving the field: the ability it entered with comes back (`baseAbility`).
    pub fn restore_ability(&mut self) {
        if let Some(ability) = self.ability_before_change.take() {
            self.ability = ability;
        }
    }

    /// Leaving the field: the types it entered with come back, an added type goes.
    pub fn restore_types(&mut self) {
        if let Some(types) = self.types_before_change.take() {
            self.types = types;
        }
        self.added_type = None;
    }

    /// Showdown `setSpecies` for a forme the Pokémon changes into for the stint (DESIGN.md
    /// §43.2): the types become the forme's, an added type goes, the stored stats become its,
    /// and the ones to go back to as it leaves are `own`'s.
    pub fn change_into_forme(&mut self, forme: &Forme, own: Option<&Forme>) {
        self.forme = (self.species != Some(forme.species)).then_some(forme.species);
        self.set_types(forme.types);
        self.unchanged_stats = own.map(|own| own.stats).or(self.unchanged_stats);
        self.stats = forme.stats;
    }

    /// Showdown `setSpecies` for a permanent forme change (DESIGN.md §43.2): the forme becomes
    /// the Pokémon's own — species, types, stored stats — and `updateMaxHp` leaves it the HP it
    /// had lost. `updateMaxHp` compares the forme's max HP with `baseMaxhp` and keeps a Dynamax's
    /// doubling: a Dynamaxed Mimikyu busting its Disguise keeps its max HP (DESIGN.md R-M8-91).
    pub fn take_forme(&mut self, forme: &Forme) {
        self.species = Some(forme.species);
        self.forme = None;
        self.types = forme.types;
        self.types_before_change = None;
        self.added_type = None;
        self.stats = forme.stats;
        self.unchanged_stats = None;
        if forme.max_hp != self.base_max_hp() {
            let dynamaxed = self.volatiles.dynamax_base_max_hp().is_some();
            let max_hp = if dynamaxed {
                forme.max_hp.saturating_mul(2)
            } else {
                forme.max_hp
            };
            let lost = self.max_hp.saturating_sub(self.hp);
            self.hp = if self.hp == 0 {
                0
            } else {
                max_hp.saturating_sub(lost).max(1)
            };
            self.max_hp = max_hp;
            if dynamaxed {
                self.volatiles.set_dynamax_base_max_hp(forme.max_hp);
            }
        }
    }

    /// The ability a permanent forme change gives (`setAbility(…, isFromFormeChange)`): the
    /// ability it leaves the field with is this one, and no `ability_before_change` keeps the
    /// old. Returns the old ability.
    pub fn take_ability(&mut self, ability: AbilityId) -> AbilityId {
        let old = self.ability;
        self.ability = ability;
        self.ability_before_change = None;
        old
    }

    /// The values its set gives `species`, one of the formes its species can take.
    pub fn forme_of(&self, species: SpeciesId) -> Option<&Forme> {
        self.formes.iter().find(|forme| forme.species == species)
    }

    pub const fn id(&self) -> PokemonId {
        self.id
    }

    pub fn is_fainted(&self) -> bool {
        self.hp == 0
    }

    /// Showdown `getLockedMove` (`priorityEvent('LockMove')`, DESIGN.md §15.12): a multi-turn
    /// move that removes every other choice — no other move, no switch, no mechanic — with the
    /// condition that holds it and the position it aims at. Soft locks (Choice items, Encore)
    /// are not this.
    pub fn hard_lock(&self) -> Option<MoveLock> {
        if let Some(move_id) = self.volatiles.two_turn_move() {
            return Some(MoveLock {
                move_id,
                by: super::VolatileConditionKind::TwoTurnMove,
                target: self.volatiles.two_turn_target(),
            });
        }
        if let Some((move_id, _, target)) = self.volatiles.rolling() {
            return Some(MoveLock {
                move_id,
                by: super::VolatileConditionKind::Rolling,
                target,
            });
        }
        // Thrash and its kind aim anew every turn (`randomNormal`).
        if let Some(rampage) = self.volatiles.rampage() {
            return Some(MoveLock {
                move_id: rampage.move_id,
                by: super::VolatileConditionKind::Rampage,
                target: None,
            });
        }
        // Gen 1 Rage: for good, at its target as any other turn's (§15.19).
        if self.volatiles.rage_lock().is_some() {
            return Some(MoveLock {
                move_id: MoveId::RAGE,
                by: super::VolatileConditionKind::Rage,
                target: None,
            });
        }
        // So does Uproar.
        if self.volatiles.uproar().is_some() {
            return Some(MoveLock {
                move_id: MoveId::UPROAR,
                by: super::VolatileConditionKind::Uproar,
                target: None,
            });
        }
        self.volatiles
            .bide()
            .filter(|bide| bide.traps)
            .map(|_| MoveLock {
                move_id: MoveId::BIDE,
                by: super::VolatileConditionKind::Bide,
                target: None,
            })
    }

    /// The move of the hard lock, if any (`hard_lock`).
    pub fn locked_move(&self) -> Option<MoveId> {
        self.hard_lock().map(|lock| lock.move_id)
    }

    /// Showdown `getSemiLockedMove`: Gen 1-2 Bide keeps every other move off the request but
    /// lets the Pokémon switch.
    pub fn semi_locked_move(&self) -> Option<MoveId> {
        self.volatiles
            .bide()
            .filter(|bide| !bide.traps)
            .map(|_| MoveId::BIDE)
    }

    pub fn has_status(&self, status: NonVolatileStatus) -> bool {
        self.status.is_some_and(|current| current.kind == status)
    }

    pub fn move_slot(&self, slot: MoveSlot) -> Option<&MoveSlotState> {
        self.move_slots().get(slot.index())
    }

    pub fn move_slot_mut(&mut self, slot: MoveSlot) -> Option<&mut MoveSlotState> {
        self.move_slots_mut().get_mut(slot.index())
    }
}
