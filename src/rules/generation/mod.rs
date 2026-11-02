//! Generation profiles (`GenerationRules::for_generation`), one file per rule family: each
//! family's types and tables beside the `resolve(generation)` that builds its rules, and
//! `for_generation` putting them together (DESIGN.md §48.3).

use crate::dex::PokemonType;
use crate::ids::{AbilityId, MoveId, SpeciesId};
use crate::spec::Generation;

mod action_order;
mod conditions;
mod damage;
mod faint;
mod hits;
mod residual;
mod stats;
mod status;
mod two_turn;
mod weather;

pub use action_order::*;
pub use conditions::*;
pub use damage::*;
pub use faint::*;
pub use hits::*;
pub use residual::*;
pub use stats::*;
pub use status::*;
pub use two_turn::*;
pub use weather::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fraction {
    pub numerator: u16,
    pub denominator: u16,
}

impl Fraction {
    pub const ZERO: Self = Self::new(0, 1);
    pub const ONE: Self = Self::new(1, 1);

    pub const fn new(numerator: u16, denominator: u16) -> Self {
        if denominator == 0 {
            panic!("fraction denominator must be non-zero");
        }
        Self {
            numerator,
            denominator,
        }
    }

    pub fn apply_u32(self, value: u32) -> u32 {
        value.saturating_mul(self.numerator as u32) / self.denominator as u32
    }

    pub fn apply_u16(self, value: u16) -> u16 {
        self.apply_u32(u32::from(value)).min(u32::from(u16::MAX)) as u16
    }

    pub fn multiply(self, other: Self) -> Self {
        Self::new(
            self.numerator.saturating_mul(other.numerator),
            self.denominator.saturating_mul(other.denominator),
        )
    }

    pub const fn is_zero(self) -> bool {
        self.numerator == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationCapabilities {
    pub mega: bool,
    pub z_move: bool,
    pub dynamax: bool,
    pub tera: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationRules {
    pub damage: DamageRules,
    pub stats: StatRules,
    pub status: StatusRules,
    pub conditions: ConditionRules,
    pub hits: HitRules,
    pub residual: ResidualRules,
    pub faint: FaintRules,
    pub update: crate::rules::UpdateRules,
    pub two_turn: TwoTurnRules,
    pub weather: WeatherRules,
    pub action_order: ActionOrderRules,
    pub capabilities: GenerationCapabilities,
}

impl GenerationRules {
    /// Generation profiles are resolved before battle execution. The values here are intentionally
    /// explicit so later corrections stay localized instead of becoming scattered `if gen >= ...`
    /// branches throughout move and ability implementations.
    pub fn for_generation(generation: Generation) -> Self {
        Self {
            damage: damage::resolve(generation),
            stats: stats::resolve(generation),
            status: status::resolve(generation),
            conditions: conditions::resolve(generation),
            hits: hits::resolve(generation),
            residual: residual::resolve(generation),
            faint: faint::resolve(generation),
            update: crate::rules::UpdateRules::for_generation(generation),
            two_turn: two_turn::resolve(generation),
            weather: weather::resolve(generation),
            action_order: action_order::resolve(generation),
            capabilities: capabilities(generation),
        }
    }
}

/// The gimmicks the generation's battles have at all.
fn capabilities(generation: Generation) -> GenerationCapabilities {
    GenerationCapabilities {
        mega: matches!(generation, Generation::Gen6 | Generation::Gen7),
        z_move: generation == Generation::Gen7,
        dynamax: generation == Generation::Gen8,
        tera: generation == Generation::Gen9,
    }
}
