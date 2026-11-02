//! The stable API's value types in a snapshot (DESIGN.md §48.2.3a): written and read through
//! `Wire`, so they keep no serde impl of their own and serde stays out of the stable API's
//! promise. A field holding one — in an `Option`, a `Vec`, an array or a tuple too — is marked
//! `#[serde(with = "crate::snapshot::wire")]`. Each takes the form serde's derive would give it:
//! an id its number, a unit variant its name, a struct its fields.
//!
//! The unit enums' variants are generated from their sources (`work/m11/r25/gen_wire.py`); the
//! `match` writing one is exhaustive, so a variant added there fails to compile here until the
//! list is generated again.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    AbilityId, ActionChoice, BattleOutcome, BattlePhase, DeclaredTarget, FieldConditionKind,
    Gender, ItemId, MechanicChoice, MoveId, MoveSlot, NonVolatileStatus, PokemonId, PokemonType,
    PositionId, SessionPhase, SideConditionKind, SideId, SpeciesId, Stat, Stats, TargetPattern,
    TeamSet, TeamSlot, TerrainKind, VolatileConditionKind, WeatherKind,
};

/// A value a snapshot writes and reads without a serde impl of its own.
pub(crate) trait Wire: Sized {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error>;
    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
}

/// `#[serde(with = "crate::snapshot::wire")]`.
pub(crate) fn serialize<T: Wire, S: Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.serialize_wire(serializer)
}

/// `#[serde(with = "crate::snapshot::wire")]`.
pub(crate) fn deserialize<'de, T: Wire, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    T::deserialize_wire(deserializer)
}

/// `#[serde(serialize_with = "crate::snapshot::wire::serialize_ref")]`: a borrowed field of a
/// snapshot being written.
pub(crate) fn serialize_ref<T: Wire, S: Serializer>(
    value: &&T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.serialize_wire(serializer)
}

/// A `Wire` value written as a serde one.
struct Out<'a, T>(&'a T);

impl<T: Wire> Serialize for Out<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize_wire(serializer)
    }
}

/// A `Wire` value read as a serde one.
struct In<T>(T);

impl<'de, T: Wire> Deserialize<'de> for In<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize_wire(deserializer).map(In)
    }
}

impl<T: Wire> Wire for Option<T> {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Some(value) => serializer.serialize_some(&Out(value)),
            None => serializer.serialize_none(),
        }
    }

    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<In<T>>::deserialize(deserializer).map(|value| value.map(|In(value)| value))
    }
}

impl<T: Wire> Wire for Vec<T> {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter().map(Out))
    }

    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<In<T>>::deserialize(deserializer)
            .map(|values| values.into_iter().map(|In(value)| value).collect())
    }
}

impl<T: Wire, const N: usize> Wire for [T; N] {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter().map(Out))
    }

    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values: Vec<T> = Vec::<In<T>>::deserialize(deserializer)?
            .into_iter()
            .map(|In(value)| value)
            .collect();
        let found = values.len();
        values
            .try_into()
            .map_err(|_| D::Error::invalid_length(found, &format!("{N} elements").as_str()))
    }
}

impl<A: Wire, B: Wire> Wire for (A, B) {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (Out(&self.0), Out(&self.1)).serialize(serializer)
    }

    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <(In<A>, In<B>)>::deserialize(deserializer).map(|(In(a), In(b))| (a, b))
    }
}

impl<A: Wire, B: Wire, C: Wire> Wire for (A, B, C) {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (Out(&self.0), Out(&self.1), Out(&self.2)).serialize(serializer)
    }

    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <(In<A>, In<B>, In<C>)>::deserialize(deserializer).map(|(In(a), In(b), In(c))| (a, b, c))
    }
}

/// Plain values beside a stable type in a tuple: their own serde form.
macro_rules! wire_serde {
    ($($ty:ty),* $(,)?) => {$(
        impl Wire for $ty {
            fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.serialize(serializer)
            }

            fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                <$ty>::deserialize(deserializer)
            }
        }
    )*};
}

wire_serde!(bool, u8, u16, u32, i8, i16, i32, usize, String);
// Internal types beside a stable one in a tuple (`Vec<(PokemonId, Sighting)>`): their own serde
// form, derived.
wire_serde!(crate::state::Sighting, crate::state::Revealed);

/// An id: its number.
macro_rules! wire_id {
    ($($ty:ident($inner:ty)),* $(,)?) => {$(
        impl Wire for $ty {
            fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.serialize(serializer)
            }

            fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                <$inner>::deserialize(deserializer).map($ty)
            }
        }
    )*};
}

wire_id!(
    AbilityId(u16),
    ItemId(u16),
    MoveId(u16),
    PokemonId(u8),
    TeamSlot(u8)
);

impl Wire for PositionId {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.index() as u8).serialize(serializer)
    }

    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // A position's range is the battle's topology's: restoring checks it.
        u8::deserialize(deserializer).map(PositionId::new_unchecked)
    }
}

impl Wire for MoveSlot {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.index() as u8).serialize(serializer)
    }

    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let index = u8::deserialize(deserializer)?;
        MoveSlot::new(index).ok_or_else(|| D::Error::custom(format!("no move slot {index}")))
    }
}

impl Wire for DeclaredTarget {
    fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize_wire(serializer)
    }

    fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PositionId::deserialize_wire(deserializer).map(DeclaredTarget)
    }
}

/// A unit enum: its variant's name. The `match` is exhaustive.
macro_rules! wire_unit_enum {
    ($ty:ident { $($variant:ident),* $(,)? }) => {
        impl Wire for $ty {
            fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(match self {
                    $($ty::$variant => stringify!($variant),)*
                })
            }

            fn deserialize_wire<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                const VARIANTS: &[&str] = &[$(stringify!($variant)),*];
                let name = String::deserialize(deserializer)?;
                match name.as_str() {
                    $(stringify!($variant) => Ok($ty::$variant),)*
                    other => Err(D::Error::unknown_variant(other, VARIANTS)),
                }
            }
        }
    };
}

wire_unit_enum!(BattlePhase {
    TurnChoice,
    TurnResolution,
    ForcedSwitch,
    Revive,
    EndTurn,
    Finished,
});

wire_unit_enum!(FieldConditionKind {
    TrickRoom,
    EchoedVoice,
    Gravity,
    WonderRoom,
    IonDeluge,
    MudSport,
    WaterSport,
    FairyLock,
    MagicRoom,
});

wire_unit_enum!(Gender {
    Male,
    Female,
    Genderless,
});

wire_unit_enum!(MechanicChoice {
    MegaEvolve,
    UltraBurst,
    ZMove,
    Dynamax,
    Terastallize,
});

wire_unit_enum!(NonVolatileStatus {
    Burn,
    Paralysis,
    Poison,
    Toxic,
    Sleep,
    Freeze,
});

wire_unit_enum!(PokemonType {
    Normal,
    Fire,
    Water,
    Electric,
    Grass,
    Ice,
    Fighting,
    Poison,
    Ground,
    Flying,
    Psychic,
    Bug,
    Rock,
    Ghost,
    Dragon,
    Dark,
    Steel,
    Fairy,
    Stellar,
    Typeless,
});

wire_unit_enum!(SessionPhase {
    Registering,
    Choosing,
    InBattle,
});

wire_unit_enum!(SideConditionKind {
    StealthRock,
    Spikes,
    ToxicSpikes,
    StickyWeb,
    Tailwind,
    WideGuard,
    QuickGuard,
    CraftyShield,
    MatBlock,
    Reflect,
    LightScreen,
    AuroraVeil,
    Mist,
    FirePledge,
    GrassPledge,
    WaterPledge,
    Safeguard,
    LuckyChant,
});

wire_unit_enum!(SideId { P1, P2 });

wire_unit_enum!(Stat {
    Attack,
    Defense,
    SpecialAttack,
    SpecialDefense,
    Speed,
    Accuracy,
    Evasion,
});

wire_unit_enum!(TargetPattern {
    SelfOnly,
    SelectedAny,
    SelectedAdjacent,
    SelectedFoe,
    SelectedAdjacentFoe,
    RandomAdjacentFoe,
    Ally,
    AllyOrSelf,
    AllFoes,
    AllAdjacentFoes,
    AllAdjacent,
    UserSide,
    UserTeam,
    UserAndAllies,
    OpponentSide,
    Field,
});

wire_unit_enum!(TerrainKind {
    Electric,
    Grassy,
    Psychic,
    Misty,
});

wire_unit_enum!(VolatileConditionKind {
    Protect,
    Endure,
    Substitute,
    LeechSeed,
    Flinch,
    Confusion,
    Taunt,
    Encore,
    Disable,
    MustRecharge,
    ChoiceLock,
    FollowMe,
    TwoTurnMove,
    PartiallyTrapped,
    Ingrain,
    Stall,
    AllySwitch,
    HelpingHand,
    RagePowder,
    SubstituteBroken,
    ResidualDamage,
    SureHit,
    MagicCoat,
    Snatch,
    MeFirst,
    Reflect,
    LightScreen,
    Mist,
    Attract,
    QuarkDrive,
    Protosynthesis,
    MudSport,
    WaterSport,
    Yawn,
    Nightmare,
    HealBlock,
    Embargo,
    SaltCure,
    SyrupBomb,
    ThroatChop,
    TarShot,
    Unburden,
    DefenseCurl,
    AquaRing,
    Stockpile,
    PowerTrick,
    GastroAcid,
    FocusEnergy,
    DragonCheer,
    LaserFocus,
    MicleBerry,
    Fling,
    Charge,
    NoRetreat,
    Minimize,
    Roost,
    ZenMode,
    Commanding,
    Commanded,
    Powder,
    Electrify,
    Foresight,
    MiracleEye,
    SmackDown,
    MagnetRise,
    Telekinesis,
    Autotomize,
    FlashFire,
    SparklingAria,
    Metronome,
    PerishSong,
    ChillyReception,
    Trapped,
    Trapper,
    Octolock,
    PartialTrappingLock,
    FakePartiallyTrapped,
    FocusPunch,
    BeakBlast,
    ShellTrap,
    Counter,
    MirrorCoat,
    DestinyBond,
    Grudge,
    Spotlight,
    Bide,
    Pursuit,
    Rolling,
    RolloutStorage,
    Dynamax,
    Rampage,
    Uproar,
    Torment,
    Imprison,
    LockOn,
    LockedOn,
    Curse,
    FuryCutter,
    Rage,
    GlaiveRush,
    Truant,
    Invulnerability,
    SkyDrop,
    SkyDropped,
});

wire_unit_enum!(WeatherKind {
    Sun,
    Rain,
    Sandstorm,
    Snow,
    StrongWinds,
});

/// A type with fields: serde's derive on a mirror of it (`remote`), checked field by field and
/// variant by variant against the type itself.
macro_rules! wire_remote {
    ($ty:ident, $def:ident) => {
        impl Wire for $ty {
            fn serialize_wire<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                $def::serialize(self, serializer)
            }

            fn deserialize_wire<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                $def::deserialize(deserializer)
            }
        }
    };
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SpeciesId")]
struct SpeciesIdDef {
    dex: u16,
    form: u8,
}
wire_remote!(SpeciesId, SpeciesIdDef);

#[derive(Serialize, Deserialize)]
#[serde(remote = "BattleOutcome")]
enum BattleOutcomeDef {
    Winner(#[serde(with = "crate::snapshot::wire")] SideId),
    Draw,
}
wire_remote!(BattleOutcome, BattleOutcomeDef);

#[derive(Serialize, Deserialize)]
#[serde(remote = "ActionChoice")]
enum ActionChoiceDef {
    Move {
        #[serde(with = "crate::snapshot::wire")]
        slot: MoveSlot,
        #[serde(with = "crate::snapshot::wire")]
        target: Option<DeclaredTarget>,
        #[serde(with = "crate::snapshot::wire")]
        mechanic: Option<MechanicChoice>,
    },
    Switch {
        #[serde(with = "crate::snapshot::wire")]
        pokemon: PokemonId,
    },
    Pass,
}
wire_remote!(ActionChoice, ActionChoiceDef);

#[derive(Serialize, Deserialize)]
#[serde(remote = "Stats")]
struct StatsDef {
    hp: i32,
    atk: i32,
    def: i32,
    spa: i32,
    spd: i32,
    spe: i32,
}
wire_remote!(Stats, StatsDef);

#[derive(Serialize, Deserialize)]
#[serde(remote = "TeamSet")]
struct TeamSetDef {
    name: String,
    species: String,
    item: String,
    ability: String,
    moves: Vec<String>,
    nature: String,
    gender: String,
    #[serde(with = "crate::snapshot::wire")]
    evs: Option<Stats>,
    #[serde(with = "crate::snapshot::wire")]
    ivs: Option<Stats>,
    level: u32,
    shiny: bool,
    happiness: Option<f64>,
    hp_type: String,
    gigantamax: bool,
    tera_type: String,
}
wire_remote!(TeamSet, TeamSetDef);

/// An `Option<Option<T>>` of a stable type, without losing `Some(None)`, which serde writes as
/// `null` like `None`: `None` is `null`, `Some(inner)` a one-element array, `[null]` or `[value]`
/// (`#[serde(with = "crate::snapshot::wire::option_option")]`).
pub(crate) mod option_option {
    use super::*;

    pub(crate) fn serialize<S: Serializer, T: Wire>(
        value: &Option<Option<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            None => serializer.serialize_none(),
            Some(inner) => serializer.collect_seq(std::iter::once(Out(inner))),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>, T: Wire>(
        deserializer: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<[In<Option<T>>; 1]>::deserialize(deserializer)
            .map(|value| value.map(|[In(inner)]| inner))
    }
}
