//! Gen 4's damage formula as the cartridge computes it (DESIGN.md R125; issues.md I14-I20):
//! the order in which pokeplatinum's `BattleSystem_CalcMoveDamage` meets the modifiers the
//! engine collects, and `sTypeMatchupMultipliers`' order for a dual type. The values and the
//! conditions stay with the abilities, items and moves (their data components); what this
//! profile adds is where each stands, because the cartridge multiplies one modifier at a time
//! and floors each, so the order decides the result.

use crate::dex::PokemonType;
use crate::ids::AbilityId;
use crate::state::VolatileConditionKind;

/// A participant of one of the cartridge's sequences, as the engine collects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartridgeStep {
    /// The move's own multiplier (`BTLVAR_POWER_MULTI`, set by the move's script before the
    /// calculation: Facade, Brine, a doubling against a Pokémon in the air or underground).
    Move,
    Volatile(VolatileConditionKind),
    /// The attacker's or the target's ability.
    Ability(AbilityId),
    /// The holder's item, whatever it boosts: a Pokémon holds one, so the items never meet.
    Item,
    /// The weather's own stat boost (a Rock type's Special Defense in a sandstorm).
    Weather,
}

/// The cartridge's order for Gen 4 (`DamageAlgorithm::Gen4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CartridgeProfile {
    /// The power: `BTLVAR_POWER_MULTI` (Reckless's ×12/10 is one), Helping Hand, Technician,
    /// the attacker's item, Thick Fat, Mud Sport / Water Sport, the pinch abilities, Heatproof,
    /// Dry Skin, Rivalry, Iron Fist.
    pub power: &'static [CartridgeStep],
    /// The attacking stat before its stage: Huge Power / Pure Power, Slow Start, the item, then
    /// Hustle, Guts, Plus / Minus, Solar Power, Flower Gift.
    pub offense: &'static [CartridgeStep],
    /// The defending stat before its stage: the item, Marvel Scale, the sandstorm, Flower Gift.
    pub defense: &'static [CartridgeStep],
    /// What Struggle, and the confusion self-hit that borrows its data, leaves out
    /// (`move != MOVE_STRUGGLE`: Technician).
    pub struggle_skips: &'static [CartridgeStep],
    /// For each attacking type, its defending types that are not neutral, in the table's order.
    pub type_chart_order: &'static [(PokemonType, &'static [PokemonType])],
}

impl CartridgeProfile {
    /// Where `step` stands in `sequence` (`None`: nowhere — it comes after every listed one).
    pub fn position(sequence: &[CartridgeStep], step: CartridgeStep) -> Option<usize> {
        sequence.iter().position(|candidate| *candidate == step)
    }

    /// Where `defending` stands among the entries of `attacking` (neutral pairs, which change
    /// nothing, come last).
    pub fn chart_position(&self, attacking: PokemonType, defending: PokemonType) -> usize {
        self.type_chart_order
            .iter()
            .find(|(candidate, _)| *candidate == attacking)
            .and_then(|(_, order)| order.iter().position(|t| *t == defending))
            .unwrap_or(usize::MAX)
    }
}

use CartridgeStep::{Ability, Item, Move, Volatile, Weather};
use PokemonType as T;

/// pokeplatinum c248fb3, `src/battle/battle_lib.c` `BattleSystem_CalcMoveDamage` and the move
/// scripts under `res/battle/scripts/effects` (`UpdateVar OPCODE_SET, BTLVAR_POWER_MULTI`).
pub(crate) const GEN4_CARTRIDGE: CartridgeProfile = CartridgeProfile {
    power: &[
        Move,
        Ability(AbilityId::RECKLESS),
        Volatile(VolatileConditionKind::HelpingHand),
        Ability(AbilityId::TECHNICIAN),
        Item,
        Ability(AbilityId::THICK_FAT),
        Volatile(VolatileConditionKind::MudSport),
        Volatile(VolatileConditionKind::WaterSport),
        Ability(AbilityId::OVERGROW),
        Ability(AbilityId::BLAZE),
        Ability(AbilityId::TORRENT),
        Ability(AbilityId::SWARM),
        Ability(AbilityId::HEATPROOF),
        Ability(AbilityId::DRY_SKIN),
        Ability(AbilityId::RIVALRY),
        Ability(AbilityId::IRON_FIST),
    ],
    offense: &[
        Ability(AbilityId::HUGE_POWER),
        Ability(AbilityId::PURE_POWER),
        Ability(AbilityId::SLOW_START),
        Item,
        Ability(AbilityId::HUSTLE),
        Ability(AbilityId::GUTS),
        Ability(AbilityId::PLUS),
        Ability(AbilityId::MINUS),
        Ability(AbilityId::SOLAR_POWER),
        Ability(AbilityId::FLOWER_GIFT),
    ],
    defense: &[
        Item,
        Ability(AbilityId::MARVEL_SCALE),
        Weather,
        Ability(AbilityId::FLOWER_GIFT),
    ],
    struggle_skips: &[Ability(AbilityId::TECHNICIAN)],
    type_chart_order: &[
        (T::Normal, &[T::Rock, T::Steel, T::Ghost]),
        (
            T::Fire,
            &[
                T::Fire,
                T::Water,
                T::Grass,
                T::Ice,
                T::Bug,
                T::Rock,
                T::Dragon,
                T::Steel,
            ],
        ),
        (
            T::Water,
            &[T::Fire, T::Water, T::Grass, T::Ground, T::Rock, T::Dragon],
        ),
        (
            T::Electric,
            &[
                T::Water,
                T::Electric,
                T::Grass,
                T::Ground,
                T::Flying,
                T::Dragon,
            ],
        ),
        (
            T::Grass,
            &[
                T::Fire,
                T::Water,
                T::Grass,
                T::Poison,
                T::Ground,
                T::Flying,
                T::Bug,
                T::Rock,
                T::Dragon,
                T::Steel,
            ],
        ),
        (
            T::Ice,
            &[
                T::Water,
                T::Grass,
                T::Ice,
                T::Ground,
                T::Flying,
                T::Dragon,
                T::Steel,
                T::Fire,
            ],
        ),
        (
            T::Fighting,
            &[
                T::Normal,
                T::Ice,
                T::Poison,
                T::Flying,
                T::Psychic,
                T::Bug,
                T::Rock,
                T::Dark,
                T::Steel,
                T::Ghost,
            ],
        ),
        (
            T::Poison,
            &[T::Grass, T::Poison, T::Ground, T::Rock, T::Ghost, T::Steel],
        ),
        (
            T::Ground,
            &[
                T::Fire,
                T::Electric,
                T::Grass,
                T::Poison,
                T::Flying,
                T::Bug,
                T::Rock,
                T::Steel,
            ],
        ),
        (
            T::Flying,
            &[
                T::Electric,
                T::Grass,
                T::Fighting,
                T::Bug,
                T::Rock,
                T::Steel,
            ],
        ),
        (
            T::Psychic,
            &[T::Fighting, T::Poison, T::Psychic, T::Dark, T::Steel],
        ),
        (
            T::Bug,
            &[
                T::Fire,
                T::Grass,
                T::Fighting,
                T::Poison,
                T::Flying,
                T::Psychic,
                T::Ghost,
                T::Dark,
                T::Steel,
            ],
        ),
        (
            T::Rock,
            &[
                T::Fire,
                T::Ice,
                T::Fighting,
                T::Ground,
                T::Flying,
                T::Bug,
                T::Steel,
            ],
        ),
        (
            T::Ghost,
            &[T::Normal, T::Psychic, T::Dark, T::Steel, T::Ghost],
        ),
        (T::Dragon, &[T::Dragon, T::Steel]),
        (
            T::Dark,
            &[T::Fighting, T::Psychic, T::Ghost, T::Dark, T::Steel],
        ),
        (
            T::Steel,
            &[T::Fire, T::Water, T::Electric, T::Ice, T::Rock, T::Steel],
        ),
    ],
};
