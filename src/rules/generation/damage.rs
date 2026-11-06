//! A generation's damage formula and its modifiers (`DamageRules`).

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageModifierKind {
    /// The flat `+ 2` of the base formula. Its position is a generation fact: Gen 3–4 apply
    /// burn / spread / weather before it, Gen 5+ after it.
    BaseOffset,
    Spread,
    Weather,
    Critical,
    Random,
    Stab,
    TypeEffectiveness,
    Burn,
    /// Showdown's `ModifyDamage` event, the last multiplicative step: a semi-invulnerable
    /// target's weakness (Gust against a Pokémon in the air, Earthquake against one
    /// underground) from Gen 3 on.
    ModifyDamage,
    /// Gen 3-4's `ModifyDamagePhase1` (right after the burn, before the spread modifier):
    /// Reflect and Light Screen weaken the hit here (`ScreenModel::DamageModifier`).
    ModifyDamagePhase1,
    /// Gen 4's `ModifyDamagePhase2` (after the critical hit, before the random roll, floored):
    /// Life Orb's ×1.3 lives here in Diamond / Pearl / Platinum.
    ModifyDamagePhase2,
    /// Gen 3's `Cmd_damagecalc` after the critical hit: the move's own doubling
    /// (`gBattleScripting.dmgMultiplier`), Charge, Helping Hand — the cartridge profile's
    /// `after_critical` participants, in its order.
    DamageMultiplier,
    /// Gen 1-2: the damage before the `+ 2` is clamped to `legacy_clamp_min..=997`.
    LegacyClamp,
    /// Gen 3 (pokeemerald `CalculateBaseDamage`): a physical move's damage that burn, the screen
    /// and the spread brought down to 0 is raised to 1 before the `+ 2`.
    PhysicalMinimum,
    /// Gen 5 (Showdown `modifyDamage`: `gen === 5 && !baseDamage`): a damage of 0 is raised to
    /// 1 before `ModifyDamage`, which may bring it back to 0 (Multiscale's ×0.5 on 1) — and
    /// nothing raises it again. A generation without this step raises the final damage to 1.
    Minimum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageAlgorithm {
    Gen1Legacy,
    /// Showdown gen2 `getDamage` (DESIGN.md §31.2): stats from the Gen 1-2 stage table capped
    /// at 999 with burn inside the Attack stat, the high-stat rollover, a ×2 critical hit
    /// that ignores every stage unless the attacker is ahead, and `GEN2_DAMAGE_ORDER`.
    Gen2,
    /// The cartridge's formula — pokeemerald's `CalculateBaseDamage` and `Cmd_damagecalc` (Gen
    /// 3, DESIGN.md R-M11-84; issues.md I218), pokeplatinum's `BattleSystem_CalcMoveDamage` (Gen
    /// 4, DESIGN.md R125; issues.md I14-I20): the power, the attacking and the defending stat
    /// each multiplied by one modifier at a time and floored, in the profile's order, the stored
    /// stats before their stages; then the generation's order, every step floored — Gen 3's
    /// doubling, Charge and Helping Hand after the critical hit, Gen 4's Life Orb in the second
    /// phase, a dual type's multipliers one by one in the type chart's order, Gen 4's resist
    /// berry last.
    Cartridge(&'static crate::rules::CartridgeProfile),
    Modern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageRoundingPolicy {
    FloorEachStep,
}

/// Gen 5+ (Bulbapedia "Damage", Generation V onward). Verified against @smogon/calc for Gen 9.
const MODERN_DAMAGE_ORDER: &[DamageModifierKind] = &[
    DamageModifierKind::BaseOffset,
    DamageModifierKind::Spread,
    DamageModifierKind::Weather,
    DamageModifierKind::Critical,
    DamageModifierKind::Random,
    DamageModifierKind::Stab,
    DamageModifierKind::TypeEffectiveness,
    DamageModifierKind::Burn,
    DamageModifierKind::ModifyDamage,
];

/// Gen 5: the modern order, the damage raised to at least 1 before `ModifyDamage` rather than
/// after it.
const GEN5_DAMAGE_ORDER: &[DamageModifierKind] = &[
    DamageModifierKind::BaseOffset,
    DamageModifierKind::Spread,
    DamageModifierKind::Weather,
    DamageModifierKind::Critical,
    DamageModifierKind::Random,
    DamageModifierKind::Stab,
    DamageModifierKind::TypeEffectiveness,
    DamageModifierKind::Burn,
    DamageModifierKind::Minimum,
    DamageModifierKind::ModifyDamage,
];

/// Gen 4 (pokeplatinum `BattleSystem_CalcMoveDamage` and `BattleScript_CalcMoveDamage`): burn,
/// the screen, targets and weather multiply the raw base before `+ 2`; the critical hit, then
/// the second phase (Life Orb); random comes before STAB and type (`BattleSystem_ApplyTypeChart`,
/// whose tail — Filter / Solid Rock, Expert Belt, Tinted Lens — and the resist berry of the HP
/// update are the last step).
const GEN4_DAMAGE_ORDER: &[DamageModifierKind] = &[
    DamageModifierKind::Burn,
    DamageModifierKind::ModifyDamagePhase1,
    DamageModifierKind::Spread,
    DamageModifierKind::Weather,
    DamageModifierKind::BaseOffset,
    DamageModifierKind::Critical,
    DamageModifierKind::ModifyDamagePhase2,
    DamageModifierKind::Random,
    DamageModifierKind::Stab,
    DamageModifierKind::TypeEffectiveness,
    DamageModifierKind::ModifyDamage,
];

/// Gen 3 (pokeemerald `CalculateBaseDamage`, `Cmd_damagecalc`, `Cmd_typecalc`,
/// `Cmd_adjustnormaldamage`; Bulbapedia "Damage", Generation III; issues.md I218): burn, the
/// screen, the spread and the weather (with Flash Fire) on the raw base, a physical 0 raised to
/// 1, `+ 2`; the critical hit, then the move's doubling, Charge and Helping Hand; STAB and the
/// type chart; the random factor last.
const GEN3_DAMAGE_ORDER: &[DamageModifierKind] = &[
    DamageModifierKind::Burn,
    DamageModifierKind::ModifyDamagePhase1,
    DamageModifierKind::Spread,
    DamageModifierKind::Weather,
    DamageModifierKind::PhysicalMinimum,
    DamageModifierKind::BaseOffset,
    DamageModifierKind::Critical,
    DamageModifierKind::DamageMultiplier,
    DamageModifierKind::Stab,
    DamageModifierKind::TypeEffectiveness,
    DamageModifierKind::Random,
];

/// Gen 2 (Showdown gen2 `getDamage`, pokecrystal `BattleCommand_DamageCalc`): after the base
/// formula, the type-boosting item (×110/100, floor) and then ×2 for a critical hit — the
/// cartridge's order (issues.md I51; Showdown doubles first) — the 1..=997 clamp, `+ 2`,
/// weather, STAB (`+ ⌊d / 2⌋`), type effectiveness (×2 / ⌊÷2⌋ per type), and the random roll only
/// when the damage is above 1.
pub(crate) const GEN2_DAMAGE_ORDER: &[DamageModifierKind] = &[
    DamageModifierKind::ModifyDamage,
    DamageModifierKind::Critical,
    DamageModifierKind::LegacyClamp,
    DamageModifierKind::BaseOffset,
    DamageModifierKind::Weather,
    DamageModifierKind::Stab,
    DamageModifierKind::TypeEffectiveness,
    DamageModifierKind::Random,
];

/// Gen 1 (Showdown gen1 `getDamage`): a critical hit doubles the level and reads the
/// unmodified stats instead of multiplying; after the base formula the damage is clamped to
/// 0..=997, `+ 2`, STAB, type effectiveness per type (×20/10, ×5/10, floored each), and the
/// random roll only when the damage is above 1.
pub(crate) const GEN1_DAMAGE_ORDER: &[DamageModifierKind] = &[
    DamageModifierKind::LegacyClamp,
    DamageModifierKind::BaseOffset,
    DamageModifierKind::Stab,
    DamageModifierKind::TypeEffectiveness,
    DamageModifierKind::Random,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRules {
    pub algorithm: DamageAlgorithm,
    pub modifier_order: &'static [DamageModifierKind],
    pub rounding: DamageRoundingPolicy,
    pub critical_ignores_negative_offense: bool,
    pub critical_ignores_positive_defense: bool,
    pub critical_multiplier: Fraction,
    /// The critical-hit chance by crit ratio (Showdown `critMult`).
    pub critical: CriticalTable,
    /// Gen 1-2 `LegacyClamp` lower bound (Gen 1 0, Gen 2 1).
    pub legacy_clamp_min: u16,
    pub stab_modifier: Fraction,
    pub burn_modifier: Fraction,
    pub spread_modifier: Fraction,
    /// Parental Bond's second hit (Showdown `modifyDamage`, in the spread modifier's place):
    /// ½ in Gen 6, ¼ from Gen 7 (`gen > 6 ? 0.25 : 0.5`).
    pub parental_bond_second_hit: Fraction,
    /// Gen 3: only a move aimed at both foes (`allAdjacentFoes`) takes the spread modifier;
    /// one that also hits the partner (Earthquake, Explosion) does not — and only while both
    /// stand on the target's side, however many the move reaches (pokeemerald
    /// `CalculateBaseDamage`: `MOVE_TARGET_BOTH` and `CountAliveMonsInBattle(BATTLE_ALIVE_DEF_SIDE)
    /// == 2`).
    pub spread_only_foes: bool,
    pub random_min: u16,
    pub random_max: u16,
    pub random_denominator: u16,
    /// Where a type-boosting item (Charcoal …) multiplies.
    pub type_item_stage: TypeItemStage,
    /// The power boost of the move a gem was used up for (4096-based): Gen 5 ×1.5 (the gen5
    /// mod's `gem` condition, priority 0), from Gen 6 [5325, 4096] at priority 14.
    pub gem_boost: u32,
    pub gem_boost_priority: i16,
    /// The Metronome item's count and boost (Gen 4 the cartridge's, Gen 5+ Showdown's).
    pub metronome: crate::rules::MetronomeRules,
    /// Gen 2: a species' own critical-hit item (Stick, Lucky Punch) sets the stage to +2 whatever
    /// else there is (the gen2 mod's `onModifyCritRatio` returns 3 at priority −1); from Gen 3
    /// it adds 2.
    pub species_crit_item_sets_stage: bool,
    /// Flail / Reversal's HP bands (DESIGN.md §22.7).
    pub hp_bands: HpBands,
    /// Gen 2 (the gen2 mod's Return / Frustration: `|| null`): a power formula giving 0 makes
    /// the hit do nothing — no critical-hit check, no damage roll. Elsewhere it is 1.
    pub zero_formula_power_does_nothing: bool,
    /// Crush Grip / Wring Out / Hard Press.
    pub target_hp_power: TargetHpPower,
    /// From Gen 6 a move may exempt a burned user from the damage halving (Facade).
    pub move_can_ignore_burn: bool,
    /// Payback (DESIGN.md §22.8): Gen 4 counts a target that switched in this turn as having
    /// moved already (Showdown gen4 `basePowerCallback` only asks `queue.willMove`); from
    /// Gen 5 a newcomer does not double it.
    pub newcomer_counts_as_moved: bool,
    /// Psywave's damage (`damageCallback`, drawn in place of the damage roll).
    pub psywave: PsywaveDamage,
    /// How Beat Up hits (its `onModifyMove`, `basePowerCallback` and Gen 3-4 volatile).
    pub beat_up: BeatUpStyle,
    /// Transistor's boost to Electric moves (Gen 8 ×1.5, Gen 9 5325 / 4096).
    pub transistor_boost: u32,
    /// Plus / Minus (`onModifySpA`, ×1.5): whose ability turns the holder's boost on.
    pub plus_minus: PlusMinusPartners,
    /// Their handler's priority: 5 from Gen 5; the gen4 mod drops it (0).
    pub plus_minus_priority: i16,
    /// Technician's `onBasePowerPriority`: 19 through Gen 7 (the gen7 mod, which Gen 4-6
    /// inherit), 30 from Gen 8. Below Dark Aura / Fairy Aura's 20 it sees their boost in
    /// `event.modifier` and stays out once it lifts the power past 60 (R-M8-150).
    pub technician_priority: i16,
    /// Gen 4-7 Slow Start (the gen7 mod's `onModifyAtk` / `onModifySpA`) halves the Attack and
    /// the Special Attack of a move whose data says physical (`dex.moves.get(move.id)`, not the
    /// category the move took as it was used): an ordinary Z-Move, physical in its data, halves
    /// a special one's Special Attack; Photon Geyser made physical keeps its Attack. Gen 8+
    /// halves the Attack alone, whatever reads it (DESIGN.md R-M8-90).
    pub slow_start_reads_data_category: bool,
    /// The BasePower boost of an ability that changed the move's type (`typeChangerBoosted`:
    /// Aerilate … Gen 6 5325, Gen 7+ 4915; Normalize Gen 7+).
    pub type_change_boost: u32,
    /// Gen 5+ Normalize leaves the moves that set their own type (Weather Ball, Hidden Power,
    /// Revelation Dance …) alone: Gen 7+ list them (`noModifyType`), Gen 5-6 (the gen6 mod's
    /// `onModifyMove`) change only moves whose data type is not Normal, which theirs is. Gen 4
    /// changes everything but Struggle.
    pub all_moves_type_change_spares_own_type: bool,
    /// Gen 1-3: a damaging move's category follows its type, so a move whose type changes
    /// (Weather Ball, Hidden Power) changes category with it.
    pub category_follows_type: bool,
}

/// Flail / Reversal: `max(⌊HP × scale / max HP⌋, 1)`, then 200 below `thresholds[0]`, 150
/// below `[1]`, 100, 80, 40, and 20 from `[4]` up (Showdown: Gen 4 64 / 2, 6, 13, 22, 43;
/// every other generation 48 / 2, 5, 10, 17, 33).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpBands {
    pub scale: u32,
    pub thresholds: [u32; 5],
}

/// Crush Grip / Wring Out: Gen 4 `⌊HP × max / max HP⌋ + 1`; Gen 5+ through Showdown's
/// 4096-based percentage, `⌊⌊(max × 100 × ⌊HP × 4096 / max HP⌋ + 2047) / 4096⌋ / 100⌋`, at
/// least 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetHpPower {
    Linear,
    Scaled4096,
}

/// The critical-hit chance by crit ratio (Showdown `critRatio`: 1 for an ordinary move, +1
/// for Slash / Scope Lens / Super Luck …), clamped to the table (DESIGN.md §22.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriticalTable {
    /// Gen 3+: `1 / table[ratio]` (Gen 3-5 16 / 8 / 4 / 3 / 2, Gen 6 16 / 8 / 2 / 1, Gen 7+
    /// 24 / 8 / 2 / 1); index 0 never lands one.
    OneIn(&'static [u16]),
    /// Gen 2: `table[ratio] / 256` (17 / 32 / 64 / 85 / 128).
    OutOf256(&'static [u16]),
    /// Gen 1: from the attacker's species' base Speed (Showdown gen1 `getDamage`).
    Gen1BaseSpeed,
}

impl CriticalTable {
    /// The chance for a crit ratio, as a ratio; `None` for Gen 1 (species-dependent) or a
    /// ratio of 0 (never).
    pub fn chance(self, ratio: u8) -> Option<(u16, u16)> {
        match self {
            Self::OneIn(table) => {
                let index = usize::from(ratio).min(table.len() - 1);
                (table[index] > 0).then(|| (1, table[index]))
            }
            Self::OutOf256(table) => {
                let index = usize::from(ratio).min(table.len() - 1);
                (table[index] > 0).then(|| (table[index], 256))
            }
            Self::Gen1BaseSpeed => None,
        }
    }

    /// Gen 1: `⌊base Speed / 2⌋`, doubled (capped at 255) — halved instead under Focus Energy,
    /// the cartridge's bug —, then halved for an ordinary move or quadrupled (capped) for a
    /// high-ratio one, out of 256.
    pub fn gen1_chance(base_speed: u16, ratio: u8, focus_energy: bool) -> (u16, u16) {
        let doubled = if focus_energy {
            base_speed / 2 / 2
        } else {
            ((base_speed / 2) * 2).clamp(1, 255)
        };
        let numerator = if ratio >= 2 {
            (doubled * 4).clamp(1, 255)
        } else {
            doubled / 2
        };
        (numerator, 256)
    }
}

/// Where a type-boosting item multiplies: Gen 4+ the base power (the item's own 4096-based
/// numerator, ×1.2), Gen 3 the offensive stat (`onModifyAtk` / `onModifySpA`, ×1.1 with
/// `modify`), Gen 2 the damage right after the critical hit (`onModifyDamage`, ×1.1, floor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeItemStage {
    BasePower,
    OffensiveStat,
    ModifyDamage,
}

/// Psywave's damage by generation (Showdown `damageCallback`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsywaveDamage {
    /// Gen 1: `random(0, ⌊1.5 × level⌋)`; a 0 fails the move (the Desync Clause Mod).
    Gen1,
    /// Gen 2: `random(1, level + ⌊level / 2⌋)`.
    Gen2,
    /// Gen 3+: `random(50, 151) × level / 100`, floored, at least 1.
    Percent,
}

/// Plus / Minus by generation: which Pokémon's ability turns the holder's ×1.5 on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlusMinusPartners {
    /// Gen 3 (pokeemerald `ABILITY_ON_FIELD2`): any Pokémon in a slot, a foe too, a fainted one
    /// a replacement has not taken the place of yet as well, with the other ability.
    AnyActiveOpposite,
    /// Gen 4: a partner with the other ability.
    AlliesOpposite,
    /// Gen 5+: a partner with either.
    AlliesEither,
}

/// Beat Up by generation: one hit per party member taking part (`engine::beat_up_members`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatUpStyle {
    /// Gen 2 (the gen2 mod): typeless and special; each hit's damage reads the member's base
    /// Attack and level and the target's base Defense (`-activate|user|move: Beat Up|[of] …`),
    /// of the species they battle as (`species.baseStats`; Gen 3+ `set.species`).
    /// Members: neither fainted nor statused, the user included.
    Gen2,
    /// Gen 3: typeless, special, power 10; Special Attack reads the member's base Attack and
    /// Special Defense the target's base Defense (the `beatup` volatile, priority -101).
    Gen3,
    /// Gen 4: as Gen 3, but physical: Attack and Defense. On the cartridge (pokeplatinum
    /// `BtlCmd_BeatUp`, issues.md I20) the user always takes part, whatever its status, each hit
    /// is its own formula (`cartridge_beat_up`), and no type immunity stops it
    /// (`SYSCTL_IGNORE_IMMUNITIES`).
    Gen4,
    /// Gen 5+: Dark and physical; each hit's power is 5 + ⌊base Attack / 10⌋ of its member;
    /// the user always takes part.
    Modern,
}

/// The generation's damage formula and its modifiers.
pub(super) fn resolve(generation: Generation) -> DamageRules {
    DamageRules {
        algorithm: match generation {
            Generation::Gen1 => DamageAlgorithm::Gen1Legacy,
            Generation::Gen2 => DamageAlgorithm::Gen2,
            Generation::Gen3 => {
                DamageAlgorithm::Cartridge(&crate::rules::cartridge_damage::GEN3_CARTRIDGE)
            }
            Generation::Gen4 => {
                DamageAlgorithm::Cartridge(&crate::rules::cartridge_damage::GEN4_CARTRIDGE)
            }
            _ => DamageAlgorithm::Modern,
        },
        modifier_order: match generation {
            Generation::Gen1 => GEN1_DAMAGE_ORDER,
            Generation::Gen2 => GEN2_DAMAGE_ORDER,
            Generation::Gen3 => GEN3_DAMAGE_ORDER,
            Generation::Gen4 => GEN4_DAMAGE_ORDER,
            Generation::Gen5 => GEN5_DAMAGE_ORDER,
            _ => MODERN_DAMAGE_ORDER,
        },
        rounding: DamageRoundingPolicy::FloorEachStep,
        critical_ignores_negative_offense: generation >= Generation::Gen3,
        critical_ignores_positive_defense: generation >= Generation::Gen3,
        critical_multiplier: if generation <= Generation::Gen5 {
            Fraction::new(2, 1)
        } else {
            Fraction::new(3, 2)
        },
        critical: match generation {
            Generation::Gen1 => CriticalTable::Gen1BaseSpeed,
            Generation::Gen2 => CriticalTable::OutOf256(&[0, 17, 32, 64, 85, 128]),
            Generation::Gen3 | Generation::Gen4 | Generation::Gen5 => {
                CriticalTable::OneIn(&[0, 16, 8, 4, 3, 2])
            }
            Generation::Gen6 => CriticalTable::OneIn(&[0, 16, 8, 2, 1]),
            _ => CriticalTable::OneIn(&[0, 24, 8, 2, 1]),
        },
        legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
        stab_modifier: Fraction::new(3, 2),
        burn_modifier: Fraction::new(1, 2),
        spread_modifier: match generation {
            Generation::Gen1 | Generation::Gen2 => Fraction::ONE,
            Generation::Gen3 => Fraction::new(1, 2),
            _ => Fraction::new(3, 4),
        },
        parental_bond_second_hit: if generation <= Generation::Gen6 {
            Fraction::new(1, 2)
        } else {
            Fraction::new(1, 4)
        },
        spread_only_foes: generation == Generation::Gen3,
        random_min: if generation <= Generation::Gen2 {
            217
        } else {
            85
        },
        random_max: if generation <= Generation::Gen2 {
            255
        } else {
            100
        },
        random_denominator: if generation <= Generation::Gen2 {
            255
        } else {
            100
        },
        type_item_stage: match generation {
            Generation::Gen1 | Generation::Gen2 => TypeItemStage::ModifyDamage,
            Generation::Gen3 => TypeItemStage::OffensiveStat,
            _ => TypeItemStage::BasePower,
        },
        species_crit_item_sets_stage: generation == Generation::Gen2,
        gem_boost: if generation <= Generation::Gen5 {
            6144
        } else {
            5325
        },
        gem_boost_priority: if generation <= Generation::Gen5 {
            0
        } else {
            14
        },
        metronome: crate::rules::MetronomeRules::for_generation(generation),
        zero_formula_power_does_nothing: generation == Generation::Gen2,
        psywave: match generation {
            Generation::Gen1 => PsywaveDamage::Gen1,
            Generation::Gen2 => PsywaveDamage::Gen2,
            _ => PsywaveDamage::Percent,
        },
        beat_up: match generation {
            Generation::Gen1 | Generation::Gen2 => BeatUpStyle::Gen2,
            Generation::Gen3 => BeatUpStyle::Gen3,
            Generation::Gen4 => BeatUpStyle::Gen4,
            _ => BeatUpStyle::Modern,
        },
        transistor_boost: if generation == Generation::Gen9 {
            5325
        } else {
            6144
        },
        plus_minus: match generation {
            Generation::Gen3 => PlusMinusPartners::AnyActiveOpposite,
            Generation::Gen4 => PlusMinusPartners::AlliesOpposite,
            _ => PlusMinusPartners::AlliesEither,
        },
        plus_minus_priority: if generation >= Generation::Gen5 { 5 } else { 0 },
        technician_priority: if generation >= Generation::Gen8 {
            30
        } else {
            19
        },
        slow_start_reads_data_category: generation <= Generation::Gen7,
        type_change_boost: if generation == Generation::Gen6 {
            5325
        } else {
            4915
        },
        all_moves_type_change_spares_own_type: generation >= Generation::Gen5,
        category_follows_type: generation <= Generation::Gen3,
        hp_bands: if generation == Generation::Gen4 {
            HpBands {
                scale: 64,
                thresholds: [2, 6, 13, 22, 43],
            }
        } else {
            HpBands {
                scale: 48,
                thresholds: [2, 5, 10, 17, 33],
            }
        },
        target_hp_power: if generation <= Generation::Gen4 {
            TargetHpPower::Linear
        } else {
            TargetHpPower::Scaled4096
        },
        move_can_ignore_burn: generation >= Generation::Gen6,
        newcomer_counts_as_moved: generation == Generation::Gen4,
    }
}
