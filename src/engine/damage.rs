use crate::effects::moves::BasePower;
use crate::ids::{AbilityId, ItemId, PokemonId};
use crate::rules::{DamageAlgorithm, DamageRoundingPolicy, Fraction};

use super::{ModifierApplication, ModifierDescriptor, ModifierSource, ModifierStage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccuracyChance(pub u16);

impl AccuracyChance {
    pub const ALWAYS: Self = Self(10_000);

    pub const fn from_percent(percent: u8) -> Self {
        Self(percent as u16 * 100)
    }

    pub const fn clamp(self) -> Self {
        Self(if self.0 > 10_000 { 10_000 } else { self.0 })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OffensiveStat(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DefensiveStat(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DamageAmount(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitBlockReason {
    TypeImmunity,
    /// A Ground move against a target off the ground, the line naming nothing (`-immune` alone):
    /// an Air Balloon, Magnet Rise, Telekinesis (Showdown `isGrounded` false), and Gen 3's
    /// Levitate (`HitRules::levitate_immunity_named`).
    Airborne,
    Protect,
    Ability(AbilityId),
    Item(ItemId),
    /// The move's own immunity check (`onTryImmunity`: Endeavor, Dream Eater, Synchronoise).
    MoveRule,
    /// A status move against a Substitute (unless it bypasses it).
    Substitute,
    /// Wide Guard / Quick Guard on the target's side.
    WideGuard,
    QuickGuard,
    /// Crafty Shield on the target's side: a status move.
    CraftyShield,
    /// Mat Block on the target's side: a damaging move Protect would stop.
    MatBlock,
    /// A powder move against a Grass type (Gen 6+).
    Powder,
    /// A one-hit KO move against a target it cannot affect (`OhkoRules`).
    Ohko,
    /// A move with raised priority against a grounded foe in Psychic Terrain.
    PsychicTerrain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryHitResult {
    Allow,
    Blocked(HitBlockReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageApplication {
    Hp {
        target: PokemonId,
        amount: DamageAmount,
    },
    Substitute {
        target: PokemonId,
        amount: DamageAmount,
        broken: bool,
    },
    /// A damage-resolution handler refused the request (Magic Guard); nothing was committed.
    Prevented { target: PokemonId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CriticalHit(pub bool);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageBreakdown {
    pub base_power: BasePower,
    pub offense: OffensiveStat,
    pub defense: DefensiveStat,
    pub base_damage: u16,
    pub spread_applied: bool,
    pub critical: CriticalHit,
    pub weather_modifier: Fraction,
    pub stab_modifier: Fraction,
    pub type_effectiveness: Fraction,
    pub burn_applied: bool,
    pub random_percent: u16,
    pub final_damage: DamageAmount,
}

pub(crate) fn base_damage(
    algorithm: DamageAlgorithm,
    level: u8,
    power: BasePower,
    offense: OffensiveStat,
    defense: DefensiveStat,
) -> u16 {
    match algorithm {
        DamageAlgorithm::Gen1Legacy
        | DamageAlgorithm::Gen2
        | DamageAlgorithm::Cartridge(_)
        | DamageAlgorithm::Modern => {
            // The broad base equation is shared here; generation-specific critical/stat/random
            // behavior is resolved outside this step. The flat `+ 2` is *not* part of the base:
            // it is a `DamageModifierKind::BaseOffset` step whose position the generation
            // profile decides. Keeping the branch explicit prevents a future Gen-I correction
            // from leaking into modern arithmetic.
            let level_term = (2 * u32::from(level) / 5) + 2;
            let raw = level_term
                .saturating_mul(u32::from(power.0))
                .saturating_mul(offense.0)
                / defense.0.max(1)
                / 50;
            raw.min(u32::from(u16::MAX)) as u16
        }
    }
}

/// One step of the final damage formula. Multiplicative steps carry a `Fraction`; the flat
/// `+ 2` of the base formula is an `Add` step so generation profiles can place it (Gen 3–4 apply
/// burn / spread / weather before it, Gen 5+ after it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinalModifierOp {
    Multiply(Fraction),
    Add(u32),
    /// Showdown `modify(value, modifier)` with a 4096-based chained modifier: several
    /// `ModifyDamage` handlers accumulate (`chainModify`) and round once, half down.
    Chain(u32),
    /// Raised to at least this much (Gen 3's physical minimum before the `+ 2`).
    AtLeast(u32),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FinalDamageModifier {
    pub descriptor: ModifierDescriptor,
    pub op: FinalModifierOp,
}

impl FinalDamageModifier {
    pub const fn new(source: ModifierSource, priority: i16, fraction: Fraction) -> Self {
        Self {
            descriptor: ModifierDescriptor::new(ModifierStage::FinalDamage, source, priority),
            op: FinalModifierOp::Multiply(fraction),
        }
    }

    pub const fn offset(source: ModifierSource, priority: i16, amount: u32) -> Self {
        Self {
            descriptor: ModifierDescriptor::new(ModifierStage::FinalDamage, source, priority),
            op: FinalModifierOp::Add(amount),
        }
    }

    pub const fn at_least(source: ModifierSource, priority: i16, minimum: u32) -> Self {
        Self {
            descriptor: ModifierDescriptor::new(ModifierStage::FinalDamage, source, priority),
            op: FinalModifierOp::AtLeast(minimum),
        }
    }

    pub const fn chain(source: ModifierSource, priority: i16, modifier: u32) -> Self {
        Self {
            descriptor: ModifierDescriptor::new(ModifierStage::FinalDamage, source, priority),
            op: FinalModifierOp::Chain(modifier),
        }
    }
}

/// Showdown `Battle.modify`: `tr((tr(value * modifier) + 2048 - 1) / 4096)`.
pub(crate) fn showdown_modify(value: u32, modifier: u32) -> u32 {
    ((u64::from(value) * u64::from(modifier) + 2047) / 4096).min(u64::from(u32::MAX)) as u32
}

/// Showdown `chainModify`: two 4096-based modifiers combined, rounded half up
/// (`((M × M') + 0x800) >> 12`).
pub(crate) fn chain_modifiers(current: u32, next: u32) -> u32 {
    ((u64::from(current) * u64::from(next) + 2048) >> 12).min(u64::from(u32::MAX)) as u32
}

/// The exact fraction a Showdown 4096-based modifier stands for: the smallest denominator (up
/// to 20) whose fraction truncates or rounds to it (4915 → 6 / 5, 2730 → 2 / 3, 5325 → 13 / 10),
/// so the Gen 4 cartridge's integer steps (`× 120 / 100`, DESIGN.md R125) apply to the same
/// data. A modifier no such fraction explains is its own 4096ths.
pub(crate) fn exact_fraction(modifier: u32) -> (u32, u32) {
    for denominator in 1..=20u32 {
        let numerator = (modifier * denominator + 2048) / 4096;
        let scaled = numerator * 4096;
        if scaled / denominator == modifier || (scaled + denominator / 2) / denominator == modifier
        {
            return (numerator, denominator);
        }
    }
    (modifier, 4096)
}

/// A 4096-based modifier from a fraction, as Showdown's `chainModify(n, d)` builds it.
pub(crate) fn fraction_4096(numerator: u32, denominator: u32) -> u32 {
    numerator * 4096 / denominator.max(1)
}

/// The final damage: every modifier in order, then raised to at least `floor` (1, or 0 where
/// the generation raised it earlier in the order: Gen 5's `DamageModifierKind::Minimum`).
pub(crate) fn apply_final_modifiers(
    base: u16,
    modifiers: &mut [FinalDamageModifier],
    rounding: DamageRoundingPolicy,
    floor: u32,
) -> (DamageAmount, Vec<ModifierApplication>) {
    modifiers.sort_by_key(|modifier| modifier.descriptor.order);

    let mut value = u32::from(base);
    let mut applications = Vec::with_capacity(modifiers.len());
    for modifier in modifiers.iter() {
        let descriptor = modifier.descriptor;
        let from = value;
        value = match modifier.op {
            FinalModifierOp::Add(amount) => value.saturating_add(amount),
            FinalModifierOp::Multiply(fraction) => match rounding {
                DamageRoundingPolicy::FloorEachStep => fraction.apply_u32(value),
            },
            FinalModifierOp::Chain(modifier) => showdown_modify(value, modifier),
            FinalModifierOp::AtLeast(minimum) => value.max(minimum),
        };
        applications.push(ModifierApplication {
            descriptor,
            from,
            to: value,
        });
        if matches!(modifier.op, FinalModifierOp::Multiply(fraction) if fraction.is_zero()) {
            return (DamageAmount(0), applications);
        }
    }

    (
        DamageAmount(value.max(floor).min(u32::from(u16::MAX)) as u16),
        applications,
    )
}
