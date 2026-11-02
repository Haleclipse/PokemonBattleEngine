//! Transform and the type-changing effects (`TransformRules`, `TypeRules`, `PlateRules`).

use super::*;

/// Transform (Showdown `pokemon.transformInto`, DESIGN.md §41.7): when it fails and what it
/// copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransformRules {
    /// Gen 2+: a target that has transformed already cannot be copied.
    pub fails_into_transformed: bool,
    /// Gen 5+: a user that has transformed already cannot transform again.
    pub fails_when_transformed: bool,
    /// Gen 5+: a target behind a Substitute cannot be copied (Gen 2-4 the move passes it).
    pub fails_behind_substitute: bool,
    /// Gen 5+: each copied move's max PP is its PP, at most 5; Gen 1-4 the user's own full
    /// max PP for the move (`calculatePP` with its PP Ups).
    pub max_pp_capped: bool,
    /// Gen 2-4: Hidden Power's type and power are the target's (Gen 5+ the user keeps its own).
    pub copies_hidden_power: bool,
    /// Gen 3+ (`gen > 2`): the ability is the target's.
    pub copies_ability: bool,
    /// Gen 5+ (`setAbility`'s `Start`): the copied ability starts at once when it is a
    /// different one. Gen 4's starts at the move's end with any other gained ability
    /// (`StartNewAbility::Unannounced`); Gen 3's never.
    pub starts_different_ability: bool,
    /// Gen 4 on the cartridge (pokeplatinum `BtlCmd_Transform` clears every `…Announced` flag):
    /// the switch-in effects announced this stint count as unannounced again, so the move's
    /// end starts the copied Intimidate even after the user's own (issues.md I112).
    pub clears_announcements: bool,
    /// Gen 6+: Focus Energy, Dragon Cheer and Laser Focus are dropped, then the target's copied.
    pub copies_crit_volatiles: bool,
    /// Gen 6+: Autotomize lightens `weighthg` itself, which Transform overwrites with the
    /// target's: the user's own uses go, the target's come along. Gen 5's lightening is the
    /// user's volatile (`onModifyWeight`), which stays.
    pub copies_autotomize: bool,
    /// Gen 3-4 on the cartridge (pokeemerald `Cmd_transformdataexecution`, pokeplatinum
    /// `BtlCmd_Transform` clear `disabledMove`): the user's Disable ends, without a word
    /// (issues.md I112; Showdown keeps it).
    pub ends_disable: bool,
}

/// How a Pokémon's types change in battle (DESIGN.md §41.4): Showdown `setType` / `getTypes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeRules {
    /// Whose types `setType` (without `enforce`) refuses to change.
    pub lock: TypeLock,
    /// The `arceus` / `silvally` conditions' `this.gen >= 8` guard, negated (Gen 5-7): a holder
    /// whose ability is not Multitype / RKS System is Normal whatever its species, rather than
    /// keeping the types of the forme it was built as.
    pub without_ability_becomes_normal: bool,
    /// What `getTypes` gives when nothing is left (a pure Flying type roosting): Gen 5+ Normal,
    /// before `???`.
    pub empty_becomes: PokemonType,
    /// Gen 6+ Soak fails on a Pokémon that is pure Water already (the gen5 `onHit` only asks
    /// `setType`).
    pub soak_fails_if_same: bool,
    /// The type Camouflage gives its user.
    pub camouflage: Camouflage,
    /// How Conversion picks the user's new type.
    pub conversion: Conversion,
    /// What Conversion 2 reads: Gen 2 the target's `lastMove`, later its `lastMoveUsed` (with
    /// the type it was used as).
    pub conversion2_reads_last_move: bool,
    /// Gen 4+ Conversion 2 leaves out the user's own types.
    pub conversion2_skips_own_types: bool,
    /// Gen 2-4 (the gen2 / gen3 / gen4 mods' `lastMove.id === 'struggle' ? 'Normal'`): Conversion
    /// 2 reads a Struggle as Normal. Gen 5+ reads the type it was used as, `???`, which nothing
    /// resists: the move fails.
    pub conversion2_struggle_as_normal: bool,
    /// Gen 9 Protean / Libero change the holder's type once a stint (`abilityState.protean`;
    /// the gen8 mod's every time).
    pub protean_once_per_entry: bool,
}

/// Arceus's Plate-as-type (`arceus` species condition, `onType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlateRules {
    /// Gen 4 only (the gen4 mod's `arceus` condition): the holder takes the forme of its Plate
    /// as it comes in (Showdown has no forme change from Gen 5: the type alone follows).
    pub switch_in_forme: bool,
    /// Gen 4 only (the same condition's `onBeforeSwitchIn`): an Arceus entering takes the
    /// Normal type before anything else reads it (`setType('Normal')`; Multitype refuses it),
    /// so a forme built as another type comes in Normal and can be retyped from there.
    pub before_switch_in_normal: bool,
}

/// Conversion (`TypeRules::conversion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Conversion {
    /// Gen 1 (`target: normal`): the user takes the target's types.
    CopyTarget,
    /// Gen 2-5: the type of one of the user's moves it does not have, drawn over the slots
    /// (`sample`). Gen 4-5 leave Conversion itself out. A `???` move (Curse before Gen 5):
    /// `typeless_counts` (Gen 3-4 on the cartridge, issues.md I85 — §41.10 A1) counts it as
    /// Ghost for a Ghost user and Normal for any other; otherwise (Gen 2, Showdown's gen3
    /// `move.id !== 'curse'`) it is left out.
    RandomMoveType {
        skips_conversion: bool,
        typeless_counts: bool,
    },
    /// Gen 6+: the first slot's type, failing for a user that has it.
    FirstSlot,
}

/// Camouflage's type (`TypeRules::camouflage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Camouflage {
    /// Gen 3-4 (the gen4 mod): Normal, failing for a user that has Normal already.
    Normal,
    /// Gen 5: Ground (only `setType` can fail).
    Ground,
    /// Gen 6-7: the terrain's (Electric, Grass, Fairy for Misty, Psychic), Normal without one;
    /// failing for a user that is that type alone already.
    ByTerrain,
}

/// Whose types cannot be changed (`TypeRules::lock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeLock {
    /// Gen 1-3: nobody's.
    None,
    /// Gen 4: a Pokémon with Multitype (`hasAbility('multitype')`).
    MultitypeHolder,
    /// Gen 5+: these species, every forme (`species.num`: Arceus, Silvally).
    Species(&'static [u16]),
}
