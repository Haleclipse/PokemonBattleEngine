use crate::ids::{AbilityId, ItemId, MoveId, PokemonId};
use crate::state::{NonVolatileStatus, TerrainKind, VolatileConditionKind, WeatherKind};

/// A rule stage at which a value may be modified.
///
/// Effects register only for stages they are semantically allowed to touch. This is the central
/// replacement for a single generic event result/value channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierStage {
    Accuracy,
    BasePower,
    OffensiveStat,
    DefensiveStat,
    EffectiveSpeed,
    FinalDamage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierSource {
    Move(MoveId),
    /// An ability on a specific holder. Carrying the holder keeps two identical abilities on
    /// different Pokémon distinguishable in trace and ordering (doubles).
    AbilityInstance {
        holder: PokemonId,
        ability: AbilityId,
    },
    /// An item on a specific holder; same rationale as `AbilityInstance`.
    ItemInstance {
        holder: PokemonId,
        item: ItemId,
    },
    Status(NonVolatileStatus),
    /// A volatile on the user (Helping Hand).
    Volatile(VolatileConditionKind),
    Weather(WeatherKind),
    Terrain(TerrainKind),
    /// A pseudo-weather (Gen 6+ Mud Sport / Water Sport).
    Field(crate::state::FieldConditionKind),
    Rule(&'static str),
}

impl ModifierSource {
    /// Deterministic fallback ordering for equal explicit priorities.
    pub const fn source_order(self) -> u8 {
        match self {
            Self::Move(_) => 10,
            Self::AbilityInstance { .. } => 20,
            Self::ItemInstance { .. } => 30,
            Self::Status(_) => 40,
            Self::Volatile(_) => 45,
            Self::Weather(_) => 50,
            Self::Terrain(_) => 60,
            Self::Field(_) => 65,
            Self::Rule(_) => 70,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModifierOrder {
    pub priority: i16,
    pub source_order: u8,
}

impl ModifierOrder {
    pub const fn new(priority: i16, source: ModifierSource) -> Self {
        Self {
            priority,
            source_order: source.source_order(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct OrderedHandler<F> {
    pub priority: i16,
    pub handler: F,
}

impl<F> OrderedHandler<F> {
    pub const fn new(priority: i16, handler: F) -> Self {
        Self { priority, handler }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierDescriptor {
    pub stage: ModifierStage,
    pub source: ModifierSource,
    pub order: ModifierOrder,
}

impl ModifierDescriptor {
    pub const fn new(stage: ModifierStage, source: ModifierSource, priority: i16) -> Self {
        Self {
            stage,
            source,
            order: ModifierOrder::new(priority, source),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierApplication {
    pub descriptor: ModifierDescriptor,
    pub from: u32,
    pub to: u32,
}

pub fn sort_descriptors(entries: &mut [ModifierDescriptor]) {
    entries.sort_by_key(|entry| entry.order);
}

/// A Showdown event chain's participants in the order its handlers run (`comparePriority`): the
/// highest priority first; equal priorities by source (an ability before an item). For chains
/// whose descriptors carry Showdown's own priorities (`ModifyAtk`, `ModifyDef` …); a
/// `chainModify` rounds at every step, so from three participants the order shows.
pub fn sort_by_showdown_priority<T>(entries: &mut [(ModifierDescriptor, T)]) {
    entries.sort_by_key(|(descriptor, _)| {
        (
            std::cmp::Reverse(descriptor.order.priority),
            descriptor.order.source_order,
        )
    });
}
