//! Gen 3's and Gen 4's damage formula as the cartridge computes it (DESIGN.md R125, R-M11-84;
//! issues.md I14-I20, I218): the order in which pokeemerald's `CalculateBaseDamage` and
//! `Cmd_damagecalc`, and pokeplatinum's `BattleSystem_CalcMoveDamage`, meet the modifiers the
//! engine collects, and the type table's order for a dual type. The values and the conditions
//! stay with the abilities, items and moves (their data components); what this profile adds is
//! where each stands, because the cartridge multiplies one modifier at a time and floors each,
//! so the order decides the result.

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

/// The cartridge's order (`DamageAlgorithm::Cartridge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CartridgeProfile {
    /// The power. Gen 4: `BTLVAR_POWER_MULTI` (Reckless's ×12/10 is one), Charge, Helping Hand,
    /// Technician, the attacker's item, Thick Fat, Mud Sport / Water Sport, the pinch abilities,
    /// Heatproof, Dry Skin, Rivalry, Iron Fist. Gen 3: Mud Sport / Water Sport, the pinch
    /// abilities.
    pub power: &'static [CartridgeStep],
    /// The power's participants the cartridge multiplies into the damage after the critical
    /// hit instead (`DamageModifierKind::DamageMultiplier`), in this order — Gen 3's
    /// `Cmd_damagecalc`: the move's own doubling (`gBattleScripting.dmgMultiplier`: Facade, a
    /// power formula that only multiplies the data's power, Weather Ball in weather, a hiding or
    /// Minimized target; Spit Up's layers), Charge on a move whose data type is Electric
    /// (`gBattleMoves[gCurrentMove].type`: not a Hidden Power), Helping Hand.
    pub after_critical: &'static [CartridgeStep],
    /// The power's participants the cartridge applies to the attacking stat instead, in the
    /// `offense` order: Gen 3's Thick Fat halves the attacker's Special Attack.
    pub power_on_offense: &'static [CartridgeStep],
    /// The attacking stat before its stage: Huge Power / Pure Power, Slow Start, the item, then
    /// Hustle, Guts, Plus / Minus, Solar Power, Flower Gift (Gen 3: the item, the target's Thick
    /// Fat, Hustle, Plus / Minus, Guts).
    pub offense: &'static [CartridgeStep],
    /// The defending stat before its stage: the item, Marvel Scale, the sandstorm, Flower Gift.
    pub defense: &'static [CartridgeStep],
    /// What Struggle, and the confusion self-hit that borrows its data, leaves out
    /// (`move != MOVE_STRUGGLE`: Technician).
    pub struggle_skips: &'static [CartridgeStep],
    /// For each attacking type, its defending types that are not neutral, in the table's order.
    pub type_chart_order: &'static [(PokemonType, &'static [PokemonType])],
    /// Reflect / Light Screen with the target's partner standing: Gen 3 `2 * (damage / 3)`,
    /// Gen 4 `damage * 2 / 3`.
    pub crowded_screen_divides_first: bool,
    /// Beat Up: Helping Hand before the critical hit (Gen 3 `Cmd_trydobeatup`, the critical hit
    /// a `manipulatedamage` after it) or after it (Gen 4 `BtlCmd_BeatUp`).
    pub beat_up_helping_hand_first: bool,
    /// Spit Up as Gen 3's `Cmd_stockpiletobasedamage` computes it: the formula at the move's
    /// own power (`STOCKPILE_LAYER_POWER`) with no critical hit, times the layers after it (a
    /// `Move` step of `after_critical`), then Helping Hand and the type chart (`typecalc`), and
    /// no random factor (`adjustsetdamage`). Gen 4: the layers in the power, as any move.
    pub stockpile_after_formula: bool,
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

/// pokeemerald, `src/pokemon.c` `CalculateBaseDamage` and `src/battle_script_commands.c`
/// `Cmd_damagecalc` (issues.md I218): the stats each modifier in turn — Huge Power / Pure Power,
/// the item (its type's, Choice Band, a species' own), Thick Fat, Hustle, Plus / Minus, Guts;
/// Marvel Scale after the item — then the power's Mud Sport / Water Sport and pinch abilities;
/// the move's doubling, Charge and Helping Hand after the critical hit. The type table
/// (`gTypeEffectiveness`) lists the pairs in Gen 4's order.
pub(crate) const GEN3_CARTRIDGE: CartridgeProfile = CartridgeProfile {
    power: &[
        Volatile(VolatileConditionKind::MudSport),
        Volatile(VolatileConditionKind::WaterSport),
        Ability(AbilityId::OVERGROW),
        Ability(AbilityId::BLAZE),
        Ability(AbilityId::TORRENT),
        Ability(AbilityId::SWARM),
    ],
    after_critical: &[
        Move,
        Volatile(VolatileConditionKind::Charge),
        Volatile(VolatileConditionKind::HelpingHand),
    ],
    power_on_offense: &[Ability(AbilityId::THICK_FAT)],
    offense: &[
        Ability(AbilityId::HUGE_POWER),
        Ability(AbilityId::PURE_POWER),
        Item,
        Ability(AbilityId::THICK_FAT),
        Ability(AbilityId::HUSTLE),
        Ability(AbilityId::PLUS),
        Ability(AbilityId::MINUS),
        Ability(AbilityId::GUTS),
    ],
    defense: &[Item, Ability(AbilityId::MARVEL_SCALE)],
    struggle_skips: &[],
    type_chart_order: GEN4_CARTRIDGE.type_chart_order,
    crowded_screen_divides_first: true,
    beat_up_helping_hand_first: true,
    stockpile_after_formula: true,
};

/// pokeplatinum c248fb3, `src/battle/battle_lib.c` `BattleSystem_CalcMoveDamage` and the move
/// scripts under `res/battle/scripts/effects` (`UpdateVar OPCODE_SET, BTLVAR_POWER_MULTI`).
pub(crate) const GEN4_CARTRIDGE: CartridgeProfile = CartridgeProfile {
    power: &[
        Move,
        Ability(AbilityId::RECKLESS),
        // `MOVE_EFFECT_CHARGE` on an Electric move: ×2 right after `powerMul` (R-M11-161).
        Volatile(VolatileConditionKind::Charge),
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
    after_critical: &[],
    power_on_offense: &[],
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
    crowded_screen_divides_first: false,
    beat_up_helping_hand_first: false,
    stockpile_after_formula: false,
};
