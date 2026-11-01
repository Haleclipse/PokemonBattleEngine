use crate::dex::PokemonType;
use crate::ids::{AbilityId, MoveId, SpeciesId};
use crate::spec::Generation;

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
pub enum ActionOrderStrategy {
    StaticAtTurnStart,
    RecalculateAfterAction,
}

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
    /// Gen 1-2: the damage before the `+ 2` is clamped to `legacy_clamp_min..=997`.
    LegacyClamp,
    /// Gen 3 (Showdown gen3 `modifyDamage`): a physical move's damage that burn, spread and
    /// weather brought down to 0 is raised to 1 before the `+ 2`.
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
    /// pokeplatinum's `BattleSystem_CalcMoveDamage` (DESIGN.md R125; issues.md I14-I20): the
    /// power, the attacking and the defending stat each multiplied by one modifier at a time and
    /// floored, in the profile's order, the stored stats before their stages; then
    /// `GEN4_DAMAGE_ORDER`, every step floored — Life Orb in the second phase, a dual type's
    /// multipliers one by one in the type chart's order, a resist berry last.
    Gen4(&'static crate::rules::CartridgeProfile),
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

/// Gen 3 (Bulbapedia "Damage", Generation III): like Gen 4, but the random factor comes after
/// STAB, type and the `ModifyDamage` event (Showdown gen3 `modifyDamage`: … types →
/// `ModifyDamage` → `randomizer`) — Gust / Twister against a Pokémon in the air double before
/// the roll. Verified against @smogon/calc.
const GEN3_DAMAGE_ORDER: &[DamageModifierKind] = &[
    DamageModifierKind::Burn,
    DamageModifierKind::ModifyDamagePhase1,
    DamageModifierKind::Spread,
    DamageModifierKind::Weather,
    DamageModifierKind::PhysicalMinimum,
    DamageModifierKind::BaseOffset,
    DamageModifierKind::Critical,
    DamageModifierKind::Stab,
    DamageModifierKind::TypeEffectiveness,
    DamageModifierKind::ModifyDamage,
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
    /// one that also hits the partner (Earthquake, Explosion) does not (Showdown gen3
    /// `modifyDamage`).
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

/// Stat stage multipliers: Gen 3+ `(2 + n) / 2` up and `2 / (2 + n)` down; Gen 1-2 Showdown's
/// tables, ×1, 1.5, 2 … 4 up and 100, 66, 50, 40, 33, 28, 25 % down (each floored).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageTable {
    Modern,
    Legacy,
}

/// How an in-battle stat comes from the stored one (DESIGN.md §31.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatRules {
    pub stage_table: StageTable,
    /// Gen 1-2 clamp every in-battle stat to `1..=999` after stages, paralysis and burn.
    pub cap: Option<u16>,
    /// Gen 1 (Showdown `modifiedStats`): each stat's in-battle value is kept on the Pokémon.
    /// A stage change recomputes it from the stored stat (dropping a paralysis / burn cut on
    /// it), paralysis and burn cut it when they are inflicted and again whenever the other
    /// side uses a move that changes stat stages, and a switch-out resets it.
    pub modified_stats: bool,
    /// Gen 6+ `boost`: no stage changes once the side facing the Pokémon has none left
    /// (`foePokemonLeft`) — after the hit that knocked out its last Pokémon, the move's
    /// `selfBoost` and Fell Stinger's boost change nothing.
    pub boosts_need_foe_pokemon_left: bool,
    /// Gen 2 (the gen2 `boostBy`): a raise changes nothing once the stat reads 999 — at its
    /// stage, with paralysis, clamped, the species item after (`stats::capped_stat_reading`).
    pub raise_stops_at_cap: bool,
    /// Gen 2 (the gen2 mod's `getStat`, after the clamp): a species' own item multiplies the
    /// stat itself — the items declare no callback there. Empty elsewhere, where the items'
    /// own `onModify*` handlers do it (`ItemBehavior::attack_boost` …).
    pub species_item_stats: &'static [SpeciesItemStat],
    /// Gen 2-4: a species' own item (Light Ball, Thick Club, Deep Sea Tooth / Scale, Stick,
    /// Lucky Punch) asks the species its holder battles as (Showdown `species`: a Ditto
    /// transformed into Pikachu gets the Light Ball's boost); Gen 5+ its own (`baseSpecies`).
    /// Gen 2's stat items (`species_item_stats`) read its own. Gen 3 on the cartridge
    /// (pokeemerald `CalculateBaseDamage`: `attacker->species`, which Transform copies) Soul Dew
    /// too, where Showdown asks `baseSpecies` (issues.md I113).
    pub species_items_read_battling: bool,
    /// Gen 3 on the cartridge (pokeemerald: `defender->species == SPECIES_DITTO`, nothing else):
    /// Metal Powder asks the species alone, a Ditto transformed into a Ditto included; Showdown
    /// and Gen 4+ also ask `!transformed` (issues.md I113).
    pub metal_powder_ignores_transform: bool,
    /// Gen 4 (pokeplatinum `BattleSystem_CompareBattlerSpeed`, issues.md I52): an item that
    /// halves its holder's Speed (Macho Brace, Iron Ball, the Power items: `modify_speed`
    /// below 1) is read from the held item itself — "not ignored by any negation effect": it
    /// slows its holder under Klutz and Embargo too.
    pub slowing_items_ignore_suppression: bool,
    /// Gen 5+ (Showdown: its `onAnyModifyBoost` is found through `foes()` /
    /// `alliesAndSelf()`, which skip a Pokémon at 0 HP): an Unaware holder knocked out by its
    /// own move first (`selfdestruct: 'always'`) meets its target's stages. Gen 4 on the
    /// cartridge (pokeplatinum `BattleSystem_CalcMoveDamage` and the accuracy check read
    /// `Battler_Ability`, whatever the HP) keeps its Unaware (issues.md I178).
    pub unaware_needs_hp: bool,
}

/// One species item of `StatRules::species_item_stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeciesItemStat {
    pub item: crate::ids::ItemId,
    /// The holder's species (by national dex number).
    pub species: &'static [crate::ids::SpeciesId],
    pub stats: &'static [crate::state::Stat],
    /// Floored.
    pub multiplier: Fraction,
}

/// Gen 2's `getStat`: Thick Club doubles Cubone's and Marowak's Attack, Light Ball Pikachu's
/// Special Attack, Metal Powder gives Ditto ×1.5 (floored) Defense and Special Defense.
const GEN2_SPECIES_ITEM_STATS: &[SpeciesItemStat] = &[
    SpeciesItemStat {
        item: crate::ids::ItemId::THICK_CLUB,
        species: &[
            crate::ids::SpeciesId::CUBONE,
            crate::ids::SpeciesId::MAROWAK,
        ],
        stats: &[crate::state::Stat::Attack],
        multiplier: Fraction::new(2, 1),
    },
    SpeciesItemStat {
        item: crate::ids::ItemId::LIGHT_BALL,
        species: &[crate::ids::SpeciesId::PIKACHU],
        stats: &[crate::state::Stat::SpecialAttack],
        multiplier: Fraction::new(2, 1),
    },
    SpeciesItemStat {
        item: crate::ids::ItemId::METAL_POWDER,
        species: &[crate::ids::SpeciesId::DITTO],
        stats: &[
            crate::state::Stat::Defense,
            crate::state::Stat::SpecialDefense,
        ],
        multiplier: Fraction::new(3, 2),
    },
];

/// The scale of accuracy and secondary chances: Gen 3+ percent (`randomChance(n, 100)`, a
/// declared 100 % never rolls); Gen 1-2 a byte, `⌊n × 255 / 100⌋` out of 256 — a 255
/// accuracy never rolls, but a declared 100 % secondary is a 255/256 roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChanceScale {
    Percent,
    Byte,
    /// Gen 1: every move with an accuracy rolls — even 255 (the 1/256 miss) — its stages
    /// multiply through Showdown's percent table one floor at a time, and a move aimed at
    /// the user adds 1 (never misses); a secondary is `⌈n × 256 / 100⌉` out of 256, one less
    /// for confusion.
    ByteAlwaysRolls,
}

/// The order of the percent-scale accuracy check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccuracyOrder {
    /// Gen 3-4, the cartridge (pokeemerald `Cmd_accuracycheck`, pokeplatinum
    /// `BattleControllerPlayer_CheckMoveHitAccuracy`; issues.md I5): the net stage
    /// (`6 + accuracy − evasion`, 0..=12) picks a fraction of the move's accuracy (33/100,
    /// 36/100, 43/100, 50/100, 60/100, 75/100, 1, 133/100, 166/100, 2, 233/100, 133/50, 3),
    /// then each modifier multiplies by its percentage in the cartridge's order
    /// (`AccuracyStep`), every step floored. (Showdown instead multiplies the two stages apart
    /// as floats and drops its `chainModify` modifiers when the result is not a whole number.)
    StagesThenPercentages,
    /// Gen 5+ (`hitStepAccuracy`): `ModifyAccuracy` on the move's accuracy, then the net stage,
    /// truncated.
    ModifiersThenStages,
}

/// A frozen user of a `defrost` move (Showdown `frz` `onBeforeMove` / `onModifyMove`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefrostUser {
    /// Gen 5+: no thaw roll; the move thaws the user as it starts.
    SkipsRoll,
    /// Gen 3-4: the thaw roll comes first; failing it, the move still thaws the user.
    AfterRoll,
    /// Gen 2: the move runs while frozen and thaws the user once it has landed.
    AfterMove,
    /// Gen 1: no such move.
    Never,
}

/// What thaws a frozen target that is hit (Showdown `frz` `onDamagingHit` /
/// `onAfterMoveSecondary`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThawOnHit {
    /// Gen 3+: a damaging Fire move that reaches the target's HP thaws it at once (before the
    /// move's secondaries), a `thawsTarget` move (Scald) after them.
    FireDamage,
    /// Gen 1-2: a move whose secondary is a burn thaws the target after the move.
    BurnSecondary,
}

/// When a frozen Pokémon may thaw by chance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThawTiming {
    BeforeMove,
    EndOfTurn,
    /// Gen 1: a frozen Pokémon never thaws by chance.
    Never,
}

/// Status lifecycle parameters (DESIGN.md §31, Class B), resolved per generation. Sleep and
/// confusion counters follow Showdown's `time` convention: rolled on application, decremented
/// at BeforeMove, cured when they reach 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusLifecycleRules {
    pub sleep_time_min: u8,
    pub sleep_time_max: u8,
    /// May the Pokémon act on the turn it wakes up? (false in Gen 1)
    pub sleep_wake_turn_can_act: bool,
    /// Gen 5: the sleep counter goes back to the rolled time whenever the sleeper switches in
    /// (Showdown gen5 `slp.onSwitchIn`: `time = startTime`).
    pub sleep_resets_on_switch_in: bool,
    /// Gen 3 (Showdown gen3 `slp.skippedTime`; pokeemerald's `CANCELER_ASLEEP` takes the turn
    /// off the battler's status without the asleep script that writes it to the party): the
    /// turns asleep spent on Snore / Sleep Talk since the last turn sleep stopped come back as
    /// the sleeper switches in (DESIGN.md R-M8-79).
    pub sleep_usable_turns_come_back: bool,
    /// Chance to thaw (Showdown `randomChance(n, d)`); zero = never thaws alone.
    pub thaw_chance: Fraction,
    /// When the thaw chance is rolled: at BeforeMove (Gen 3+), or in the end-of-turn residual
    /// (Gen 2, `ResidualKind::FreezeThaw`) while a frozen Pokémon never moves.
    pub thaw_timing: ThawTiming,
    /// A frozen user of a `defrost` move (Flame Wheel, Scald …).
    pub defrost_user: DefrostUser,
    /// What thaws a frozen target that is hit.
    pub thaw_on_hit: ThawOnHit,
    pub full_paralysis: Fraction,
    pub confusion_time_min: u8,
    pub confusion_time_max: u8,
    pub confusion_self_hit: Fraction,
    /// Whether confusion's self-hit rolls the damage's random factor (Gen 3+; Gen 2
    /// `noDamageVariance`, Gen 1's own formula has none).
    pub confusion_self_hit_varies: bool,
    /// Gen 3-4 hit themselves through the whole damage formula (Showdown gen4
    /// `getDamage(pokemon, pokemon, 40)`), so a burn halves it; from Gen 5 `getConfusionDamage`
    /// reads the stats alone and a burn does not.
    pub confusion_self_hit_burn_halves: bool,
    /// Gen 1-2 take the self-hit straight off the HP (Showdown gen1 / gen2 `directDamage`,
    /// pokecrystal `HitConfusion` → `DoPlayerDamage`): no `Damage` event, so Focus Band does not see it. From
    /// Gen 3 it is a move's damage (the `confused` pseudo-move, `effectType: 'Move'`).
    pub confusion_self_hit_direct: bool,
    /// Static / Flame Body / Poison Point on a contact hit (Gen 3: 1/3, Gen 4+: 3/10).
    pub contact_status_chance: Fraction,
    /// Shed Skin at the end of the turn.
    pub shed_skin_chance: Fraction,
    /// Synchronize passes bad poison as is from Gen 5; Gen 3-4 pass ordinary poison.
    pub synchronize_passes_toxic: bool,
    /// Synchronize announces itself as it passes the status on from Gen 5 (`-activate|holder|
    /// ability: Synchronize`); the gen4 mod's says nothing, the status it passes names it.
    pub synchronize_announced: bool,
    /// A status an ability inflicts (Static, Flame Body, Poison Point, Effect Spore, Poison Touch)
    /// is named on its line (`-status|…|[from] ability: …|[of] holder`: the base conditions'
    /// `onStart`, which the gen4 mod's burn, paralysis and poison inherit) — sleep only from Gen
    /// 5: the gen3 / gen4 `slp.onStart` names a move's alone (R-M10-40).
    pub ability_sleep_named: bool,
    /// Gen 3-4 (Showdown gen4 `par.onBeforeMove`: `!pokemon.hasAbility('magicguard') &&
    /// randomChance(1, 4)`): a Magic Guard holder is never fully paralysed, and draws nothing.
    pub magic_guard_skips_full_paralysis: bool,
}

/// When poison, burn, bad poison and Leech Seed take their HP (DESIGN.md §31.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusResidualTiming {
    /// In the end-of-turn residual (Gen 3+).
    EndOfTurn,
    /// Right after the Pokémon's own action — also one stopped at BeforeMove, but not a move
    /// after which the foe has fainted — and when it switches in, unless its side already
    /// lost a Pokémon this turn (Gen 2, Showdown `onAfterMoveSelf` / `onAfterSwitchInSelf`).
    AfterOwnAction,
}

/// How bad poison counts (DESIGN.md §31.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToxicModel {
    /// Gen 3+: a stage on the status, advanced by its own damage, reset on switch-out.
    Stage,
    /// Gen 2: Toxic starts the `residualdmg` volatile; each after-action tick of a burned,
    /// poisoned or badly poisoned holder advances it, and it scales burn and poison as well
    /// (`⌊max HP / 16⌋ × counter`). Switching in turns bad poison into poison.
    ResidualVolatile,
}

/// How Heal Bell or Aromatherapy cures its user's team (Showdown `onHit`, which the gen2 /
/// gen4 / gen5 / gen7 mods rewrite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamCure {
    /// Team members with one of these abilities (`hasAbility`, so a benched one's only where
    /// `ConditionRules::bench_abilities_ignored` is off) are left alone, with `-immune` when
    /// active.
    pub immune: &'static [crate::ids::AbilityId],
    /// The user's own ability keeps it off the user too (Gen 3-4, Gen 6-7 Heal Bell); later
    /// only the others' (`ally !== source`).
    pub user_immune_too: bool,
    /// An active partner behind a Substitute is left alone unless the user's move infiltrates
    /// (Gen 6+ Aromatherapy: `!move.infiltrates`, Infiltrator).
    pub substitute_blocks: bool,
    /// The move fails when it cured nobody (Showdown `return success`); before, `onHit`
    /// returns nothing and the move never fails.
    pub fails_when_nothing_cured: bool,
}

/// Heal Bell's and Aromatherapy's `TeamCure` per generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamCureRules {
    pub heal_bell: TeamCure,
    pub aromatherapy: TeamCure,
}

impl TeamCureRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        use crate::ids::AbilityId;
        const PLAIN: TeamCure = TeamCure {
            immune: &[],
            user_immune_too: false,
            substitute_blocks: false,
            fails_when_nothing_cured: false,
        };
        // The mods chain newer to older (gen2 → gen3 → … → gen8 → the base): Heal Bell is the
        // gen2 mod's in Gen 2, the gen4 mod's in Gen 3-4, the gen5 mod's in Gen 5, the gen7
        // mod's in Gen 6-7; Aromatherapy the gen4 mod's in Gen 3-4, the gen5 mod's in Gen 5,
        // the base's from Gen 6.
        let heal_bell = match generation {
            Generation::Gen3 | Generation::Gen4 => TeamCure {
                immune: &[AbilityId::SOUNDPROOF],
                user_immune_too: true,
                ..PLAIN
            },
            Generation::Gen6 | Generation::Gen7 => TeamCure {
                immune: &[AbilityId::SOUNDPROOF],
                user_immune_too: true,
                fails_when_nothing_cured: true,
                ..PLAIN
            },
            Generation::Gen8 | Generation::Gen9 => TeamCure {
                immune: &[AbilityId::SOUNDPROOF, AbilityId::GOOD_AS_GOLD],
                fails_when_nothing_cured: true,
                ..PLAIN
            },
            _ => PLAIN,
        };
        let aromatherapy = match generation {
            Generation::Gen6 | Generation::Gen7 | Generation::Gen8 | Generation::Gen9 => TeamCure {
                immune: &[AbilityId::SAP_SIPPER, AbilityId::GOOD_AS_GOLD],
                substitute_blocks: true,
                fails_when_nothing_cured: true,
                ..PLAIN
            },
            _ => PLAIN,
        };
        Self {
            heal_bell,
            aromatherapy,
        }
    }
}

/// What Haze does besides every active Pokémon's stages back to 0 (DESIGN.md §20.1, R171).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HazeRules {
    /// Volatiles it ends on every active Pokémon: Gen 1 (the gen1 mod's `onHit`) Disable,
    /// confusion, Mist, Focus Energy, Leech Seed, Light Screen, Reflect and the Toxic counter;
    /// Gen 4 (the gen4 mod's `onHitField`) Focus Energy.
    pub ends: &'static [crate::state::VolatileConditionKind],
    /// Gen 1: every other active Pokémon's status is cured (silently; a sleeping or frozen one
    /// that has not moved yet loses its action, `cannotmove`), and the user's bad poison turns
    /// into poison.
    pub cures_statuses: bool,
}

impl HazeRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        use crate::state::VolatileConditionKind as V;
        match generation {
            Generation::Gen1 => Self {
                ends: &[
                    V::Disable,
                    V::Confusion,
                    V::Mist,
                    V::FocusEnergy,
                    V::LeechSeed,
                    V::LightScreen,
                    V::Reflect,
                    V::ResidualDamage,
                ],
                cures_statuses: true,
            },
            Generation::Gen4 => Self {
                ends: &[V::FocusEnergy],
                cures_statuses: false,
            },
            _ => Self {
                ends: &[],
                cures_statuses: false,
            },
        }
    }
}

/// Where Flash Fire's boost multiplies a Fire move of its holder (DESIGN.md §20.2, R172).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashFireBoost {
    /// Gen 5+ (the base's condition): the attacking stat, `onModifyAtk` / `onModifySpA` at
    /// priority 5.
    OffensiveStat,
    /// Gen 3-4 (the gen4 mod's condition): `ModifyDamagePhase1` — Gen 4's damage as the
    /// cartridge (and the oracle's Gen 4 damage) has it: after the weather.
    ModifyDamagePhase1,
}

/// Flash Fire (Showdown `onTryHit`, which the gen3 and gen4 mods rewrite, and its `flashfire`
/// condition).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlashFireRules {
    pub boost: FlashFireBoost,
    /// Gen 3: Will-O-Wisp at a holder that is a Fire type, has a status or is behind a
    /// Substitute is not taken (it fails on its own).
    pub passes_failing_will_o_wisp: bool,
    /// Gen 3-4: a frozen holder takes no Fire move.
    pub frozen_holder_passes: bool,
}

impl FlashFireRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        match generation {
            Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => Self {
                boost: FlashFireBoost::ModifyDamagePhase1,
                passes_failing_will_o_wisp: true,
                frozen_holder_passes: true,
            },
            Generation::Gen4 => Self {
                boost: FlashFireBoost::ModifyDamagePhase1,
                passes_failing_will_o_wisp: false,
                frozen_holder_passes: true,
            },
            _ => Self {
                boost: FlashFireBoost::OffensiveStat,
                passes_failing_will_o_wisp: false,
                frozen_holder_passes: false,
            },
        }
    }
}

/// Liquid Ooze (Showdown `onSourceTryHeal`, which the gen4 and gen6 mods rewrite): which
/// drains it turns into damage, and how that damage knocks out. (The mods' `canOoze` lists
/// leave Strength Sap out only before the move exists.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiquidOozeRules {
    /// Dream Eater's drain is left alone (Gen 3-4: `activeMove.id !== 'dreameater'`).
    pub spares_dream_eater: bool,
    /// The damage knocks out at once (Gen 3-6: `damage(…, instafaint)`).
    pub instafaint: bool,
}

impl LiquidOozeRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        // The gen4 mod's in Gen 3-4, the gen6 mod's in Gen 5-6, the base's from Gen 7.
        Self {
            spares_dream_eater: matches!(
                generation,
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4
            ),
            instafaint: !matches!(
                generation,
                Generation::Gen7 | Generation::Gen8 | Generation::Gen9
            ),
        }
    }
}

/// How Pokémon coming in together run their switch-in (Showdown `runSwitch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchInOrder {
    /// Gen 1-4 (the gen4 mod's `runSwitch`): one Pokémon at a time — its entry hazards, the
    /// `SwitchIn` event (Healing Wish's slot), then its ability's and its item's `Start`.
    EachInTurn,
    /// Gen 5+: one `fieldEvent('SwitchIn')` over every Pokémon that came in, each ability's
    /// and item's `onStart` among the handlers, sorted by priority (`onSwitchInPriority`), then
    /// the holder's speed, then the effect type (status, slot, side condition, ability, item).
    ByPriority,
}

/// What Stockpile does to Defense and Special Defense (Showdown `stockpile.condition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StockpileRule {
    /// Gen 3: nothing.
    NoStages,
    /// Gen 4-6 (the gen6 mod): +1 each per layer, and as it ends -1 each per layer.
    StagesByLayer,
    /// Gen 7+: +1 each per layer, and as it ends -1 for each raise that moved the stage.
    StagesAsChanged,
}

/// What Belly Drum does (Showdown `bellydrum.onHit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BellyDrumRule {
    /// Gen 3+: it fails at or below half HP, at +6 Attack, or with 1 max HP; otherwise half the
    /// max HP goes and Attack rises by 12 (to +6).
    Maximize,
    /// Gen 2 (the cartridge): at +6 it fails; at or below half HP it still raises Attack by 2,
    /// then fails; otherwise half the max HP goes and Attack climbs two stages at a time (one to
    /// reach +6) for as long as the stage it leaves reads under 999, one short of the stage
    /// that reads 999 or more.
    ClimbUnderCap,
}

/// Where paralysis cuts Speed (Showdown `par`'s `onModifySpe`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParalysisSpeedStage {
    /// Gen 1-2: on the stat itself, floored, before the 1..=999 clamp.
    Stat,
    /// Gen 3-4: one more factor of the `ModifySpe` chain (`chainModify(0.25)`).
    Chained,
    /// Gen 5+: once the rest of the chain has been applied (`finalModify`), floored
    /// (`Math.floor(spe * 25 / 100)`; 50 from Gen 7).
    AfterChain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusRules {
    pub lifecycle: StatusLifecycleRules,
    /// When poison, burn, bad poison and Leech Seed take their HP.
    pub residual_timing: StatusResidualTiming,
    /// How bad poison counts.
    pub toxic_model: ToxicModel,
    pub paralysis_speed_modifier: Fraction,
    /// Where that cut applies in the Speed calculation.
    pub paralysis_speed_stage: ParalysisSpeedStage,
    pub burn_residual_divisor: u16,
    pub poison_residual_divisor: u16,
    pub toxic_residual_divisor: u16,
    pub fire_type_burn_immunity: bool,
    pub electric_type_paralysis_immunity: bool,
    pub poison_type_poison_immunity: bool,
    pub steel_type_poison_immunity: bool,
    pub ice_type_freeze_immunity: bool,
    pub electric_terrain_prevents_sleep: bool,
    pub misty_terrain_prevents_status: bool,
}

/// Where BeforeMove checks the recharge turn after Hyper Beam and its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RechargeCheck {
    /// Gen 2, Gen 5+ (Showdown `mustrecharge`, `onBeforeMovePriority` 11): before anything.
    First,
    /// Gen 3-4 on the cartridge (pokeemerald `CANCELER_RECHARGE`, pokeplatinum
    /// `CHECK_STATUS_STATE_RECHARGING`; issues.md I66): after sleep and freeze — a sleeping
    /// recharger counts its sleep down, a frozen one rolls its thaw — before a flinch.
    AfterSleepAndFreeze,
    /// Gen 1 (priority 7): after freeze (12), sleep (10) and flinch (8); a frozen or sleeping
    /// Pokémon keeps owing it.
    AfterFlinch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionRules {
    /// Fraction-of-max-HP healing (Recover …): Gen 2 and Gen 5+ round half up, Gen 1 and
    /// Gen 3-4 floor (Showdown `battle-actions`, the gen1 / gen2 `moveHit`).
    pub heal_rounds_half_up: bool,
    /// Gen 1 Recover / Soft-Boiled / Rest (the gen1 `onHit`): they fail when the user is 255 or 511 HP
    /// short of its max, unless its HP is a multiple of 256.
    pub heal_fails_255_short: bool,
    /// Belly Drum (Gen 2: the gen2 `onHit`).
    pub belly_drum: BellyDrumRule,
    pub stockpile: StockpileRule,
    /// Focus Energy's critical-hit stages: Gen 2 +1, Gen 3+ +2; Gen 1 none — its (bugged)
    /// effect is the table's (`CriticalTable::gen1_chance`).
    pub focus_energy_stages: u8,
    /// Gen 6+ Psych Up (Showdown's base `onHit`; the gen5 mod copies the stages alone) also
    /// copies the critical-hit volatiles: Focus Energy, Dragon Cheer, Laser Focus.
    pub psych_up_copies_crit_volatiles: bool,
    pub haze: HazeRules,
    /// Gen 3-8 Charge (the gen8 mod's `onAfterMove` / `onMoveAborted`) ends after any move but
    /// Charge; from Gen 9 only after an Electric one.
    pub charge_ends_after_any_move: bool,
    pub switch_in_order: SwitchInOrder,
    pub team_cure: TeamCureRules,
    /// Gen 5+: the ability of a Pokémon off the field does nothing (Showdown
    /// `ignoringAbility`: `gen >= 5 && !isActive`); Gen 3-4 `hasAbility` answers for the
    /// bench too (`BattleContext::has_ability`).
    pub bench_abilities_ignored: bool,
    /// Pressure's announcement as its holder comes in shows to the foe from Gen 4; the gen3
    /// mod shows it to the holder's side alone (`addSplit`, `[silent]` to the other).
    pub pressure_shown_to_foes: bool,
    /// Gen 5+: nor does its item (Showdown `ignoringItem`: `gen >= 5 && !isActive`) — what a
    /// Future Sight from a user off the field meets (§15.22).
    pub bench_items_ignored: bool,
    /// Showdown's ability flag `cantsuppress` (DESIGN.md §41.4, F8): nothing sets these
    /// abilities aside (Gastro Acid, Neutralizing Gas, Core Enforcer). Checked against
    /// Showdown's flags by `tests/ability_flags.rs`.
    pub cant_suppress: AbilitySet,
    /// Showdown's ability flag `breakable` (DESIGN.md §41.2, F3): while a move that ignores
    /// abilities is used (Mold Breaker, Sunsteel Strike …), these abilities' handlers do not
    /// run. Checked against Showdown's flags by `tests/ability_flags.rs`.
    pub breakable: AbilitySet,
    /// Showdown's `failroleplay` (Role Play fails on these) and `failskillswap` (Skill Swap fails
    /// with these on either side), checked against Showdown's flags by `tests/ability_flags.rs`.
    pub fail_role_play: AbilitySet,
    pub fail_skill_swap: AbilitySet,
    /// Showdown's `noentrain`: Entrainment fails when its user has one of these, checked against
    /// Showdown's flags by `tests/ability_flags.rs`.
    pub no_entrain: AbilitySet,
    /// Showdown's `noreceiver`: Receiver and Power of Alchemy pass these over, checked against
    /// Showdown's flags by `tests/ability_flags.rs`.
    pub no_receiver: AbilitySet,
    /// Showdown's `notrace`: Trace copies none of these (Gen 3: none; Gen 4: Forecast, Multitype,
    /// Trace, as the cartridge's `ChooseTraceTarget` has them), checked against Showdown's flags
    /// by `tests/ability_flags.rs`.
    pub no_trace: AbilitySet,
    /// Showdown's `notransform` (DESIGN.md §43.3): a Pokémon that has transformed has none of
    /// these in effect, not even one that cannot be suppressed (`ignoringAbility`), checked
    /// against Showdown's flags by `tests/ability_flags.rs`.
    pub no_transform: AbilitySet,
    /// How an ability changes in battle (DESIGN.md §41.3).
    pub ability_change: AbilityChangeRules,
    /// Transform and Imposter (Showdown `transformInto`, DESIGN.md §41.7).
    pub transform: TransformRules,
    /// The forme-changing abilities' generation differences (DESIGN.md §43.4).
    pub forme: FormeRules,
    /// Moves that call or copy other moves (DESIGN.md §42.2).
    pub calls: CallRules,
    /// How a Pokémon's types change in battle (DESIGN.md §41.4).
    pub type_change: TypeRules,
    /// Arceus's Plates (DESIGN.md §43.4).
    pub plate: PlateRules,
    /// Gen 8+ (`suppressingAbility`: `activePokemon !== target || gen < 8`): a move that ignores
    /// abilities leaves its user's own breakable ability alone; before, the user's is ignored
    /// too.
    pub mold_breaker_spares_user: bool,
    pub liquid_ooze: LiquidOozeRules,
    pub flash_fire: FlashFireRules,
    /// Gen 6+ Minimize (the gen5 mod drops its `onAccuracy`): a `minimize` move is sure to hit
    /// the holder.
    pub minimize_sure_hit: bool,
    /// Where a `minimize` move doubles against a Minimized target: its condition's
    /// `onSourceModifyDamage` in the `ModifyDamage` event (Gen 1 has none in its order, so no
    /// doubling there), the final damage on the Gen 2 cartridge (issues.md I50), or the power
    /// on the Gen 4 cartridge (the move's own multiplier, as the oracle's Gen 4 damage has it).
    pub minimize_bonus: DamageBonusStage,
    /// How long Rest's sleep lasts (Showdown `statusState.time`, set over the duration the
    /// sleep rolled): Gen 1 2 (the gen1 mod), later 3.
    pub rest_sleep_turns: u8,
    /// Gen 2 Rest on a user already asleep (through Sleep Talk): the gen2 mod's `onTry` asks
    /// only for missing HP and its `onHit` keeps the sleep (`-status|…|slp|[from] move: Rest`,
    /// no sleep drawn), sets it to `rest_sleep_turns` and heals in full — pokecrystal
    /// `BattleCommand_Heal` writes `REST_SLEEP_TURNS + 1` over any status. Elsewhere a sleeping
    /// user's Rest fails (`onTry`).
    pub rest_renews_sleep: bool,
    /// Gen 4 Leaf Guard lets Rest's sleep through in sun (the gen4 mod's `onSetStatus`: "do
    /// nothing" for `effect.id === 'rest'`; Bulbapedia: Rest works normally, two turns asleep).
    /// From Gen 5 the sleep is refused, and Rest fails.
    pub leaf_guard_spares_rest: bool,
    /// Gen 4 Acupressure (the gen4 mod's `onHit`) fails against a target behind a Substitute
    /// — the user's own included.
    pub acupressure_fails_behind_substitute: bool,
    /// Gen 5-7 Moody (the gen7 mod) draws from accuracy and evasion too; Gen 8+ from the five
    /// stats alone.
    pub moody_draws_accuracy_evasion: bool,
    /// Gen 8+ Swallow that heals nothing (full HP, Heal Block) returns `NOT_FAIL`; the gen7
    /// mod (Gen 3-7) returns `null`, a failure.
    pub swallow_nothing_not_fail: bool,
    /// Gen 5+ Swallow's `onTry` lets a snatched Swallow through (`move.sourceEffect ===
    /// 'snatch'`): the Snatch user heals a quarter even with no Stockpile. The gen4 mod's
    /// (Gen 3-4) asks the Snatch user for its own Stockpile, as pokeemerald's
    /// `stockpiletohpheal` does (DESIGN.md R-M8-90).
    pub snatched_swallow_skips_stockpile: bool,
    /// Gen 6+ Substitute lets a move that `infiltrates` through (a heal in place of damage:
    /// Present's, Pollen Puff's). Gen 2-5's `onTryPrimaryHit` ignores it and computes the
    /// damage, which a heal has none of: the move fails (Gen 3-5 `-fail|user`, Gen 2 silently).
    pub substitute_honors_infiltrates: bool,
    pub substitute_hp_divisor: u16,
    /// Gen 1 (Showdown gen1 `substitute`): the doll holds one HP more than it cost, and the move
    /// fails only below a quarter of max HP (at exactly a quarter the user faints paying it); a
    /// confusion self-hit behind it strikes the foe's Substitute instead, or nothing.
    pub substitute_gen1: bool,
    /// Gen 1 (pokered's `ConfusionSideEffect` asks no Substitute; Showdown's gen1 `substitute`
    /// rolls it after the doll took the hit): a move's confusion secondary reaches a target
    /// behind a Substitute the hit did not break (DESIGN.md R-M8-93).
    pub confusion_secondary_through_substitute: bool,
    pub stealth_rock_hp_divisor: u16,
    /// Gen 5+ (`fieldEvent('SwitchIn')` checks faints after each handler): a hazard that knocks
    /// the entrant out stops the rest. Gen 1-4 (`runEvent('EntryHazard')`) run them all: a
    /// knocked-out Poison type still absorbs Toxic Spikes.
    pub hazards_stop_at_knock_out: bool,
    /// Spikes damage per layer in 24ths of max HP (Showdown `[3, 4, 6]`).
    pub spikes_damage_24ths: [u8; 3],
    /// Spikes layers a side can hold (Gen 2: one — a second use fails).
    pub spikes_max_layers: u8,
    pub toxic_spikes_max_layers: u8,
    /// Whom Sticky Web's Speed drop comes from (what Mirror Armor sends it back to): Gen 9 the
    /// entrant's foe in the first slot (`pokemon.side.foe.active[0]`), Gen 6-8 the Pokémon that
    /// set it (the gen8 mod's `effectState.source`).
    pub sticky_web_drop_from_foe_lead: bool,
    pub leftovers_hp_divisor: u16,
    /// Focus Band's chance against a move's damage that would knock its holder out (Gen 2
    /// 30 / 256, the gen2 mod and pokecrystal's item parameter; Gen 3+ 1 / 10).
    pub focus_band_chance: Fraction,
    /// Gen 3-4 (issues.md I38: pokeemerald `Cmd_adjustnormaldamage`'s `focusBanded`, cleared
    /// with the action; pokeplatinum's `focusItemActivated`, cleared at `MoveEnd`): Focus Band
    /// rolls on every hit of a move — one a Substitute takes too — and once it succeeds it holds
    /// every hit of the move that would knock its holder out. Otherwise (Gen 2, Gen 5+) each
    /// damage that would knock it out rolls on its own.
    pub focus_band_lasts_the_move: bool,
    pub leech_seed_hp_divisor: u16,
    /// Moonlight / Morning Sun / Synthesis / Shore Up (DESIGN.md §22.7).
    pub weather_heal: WeatherHealRules,
    /// Gen 1: Leech Seed takes `max HP / 16` times the `residualdmg` counter (advancing it)
    /// and heals the full amount, however little HP was left.
    pub leech_seed_gen1: bool,
    /// Gen 1 `useMove`: the user's after-action residual (`AfterMoveSelf`) runs while the
    /// move's target stands — the user itself for a move aimed at it (`move.target === 'self'`:
    /// a Substitute at exactly a quarter of its HP leaves it at 0, R-M8-127). Gen 2's `runMove`
    /// asks the foe.
    pub after_own_action_reads_the_target: bool,
    /// Showdown `mustrecharge`: its duration (Gen 1: none), and whether a knock-out skips the
    /// recharge (Gen 1).
    pub must_recharge_turns: Option<u8>,
    pub recharge_skipped_on_knock_out: bool,
    /// Taunt / Encore / Disable durations (Showdown `duration` / `durationCallback` and the
    /// `onStart` adjustment), per generation (DESIGN.md §31.1).
    pub taunt: TimedVolatileRules,
    pub encore: TimedVolatileRules,
    pub disable: TimedVolatileRules,
    /// Showdown `twoturnmove` duration: the charging turn and the attacking turn.
    pub two_turn_move_turns: u8,
    /// Bind / Wrap / Fire Spin …: turns rolled on application (Showdown `partiallytrapped`
    /// `durationCallback`: Gen 2 3-5, Gen 3-4 3-6, Gen 5+ 5-6) and the chip per residual
    /// (max HP / divisor: 16 to Gen 5, 8 from Gen 6).
    pub bind_turns_min: u8,
    pub bind_turns_max: u8,
    pub bind_damage_divisor: u16,
    /// A trapper holding Grip Claw binds for this many turns, no roll (Showdown
    /// `durationCallback`: the gen4 mod 6, pokeplatinum `HOLD_EFFECT_EXTEND_TRAPPING` 6; from Gen 5
    /// 8).
    pub grip_claw_bind_turns: u8,
    /// A trapper holding Binding Band as it binds (Gen 5+: Showdown `boundDivisor`) makes the
    /// chip this share: max HP / 8 in Gen 5 (the gen5 mod), / 6 from Gen 6.
    pub binding_band_damage_divisor: u16,
    /// Big Root's `onTryHeal` on a drain, Leech Seed, Ingrain, Aqua Ring or Strength Sap: Gen 4
    /// truncates `heal × 1.3` (the gen4 mod; pokeplatinum `HOLD_EFFECT_LEECH_BOOST` × 130 / 100),
    /// from Gen 5 a 4096-based modifier of 5324 (`chainModify([5324, 4096])`).
    pub big_root_truncates: bool,
    /// Shell Bell (issues.md I47).
    pub shell_bell: ShellBellCount,
    /// White Herb's reaction to a switch-in (DESIGN.md R233).
    pub any_switch_in_reach: AnySwitchInReach,
    /// From Gen 6 a Ghost type cannot be trapped (type chart `trapped` immunity).
    pub ghost_escapes_trapping: bool,
    /// From Gen 4 Ingrain roots its user: a Flying type or a Levitate holder is grounded
    /// (Showdown `isGrounded`).
    pub ingrain_grounds: bool,
    /// Gen 3-4 on the cartridge (pokeemerald `AtkCanceller_UnableToUseMove`, pokeplatinum
    /// `CheckStatusState`; issues.md I66): Taunt refuses a status move right after Disable —
    /// before Heal Block and Gravity (Gen 4) and confusion. From Gen 5 (Showdown priority 5) it
    /// comes after Heal Block, Throat Chop and Gravity (6). (Showdown's gen3 mod checks it last,
    /// after paralysis.)
    pub taunt_before_field_blocks: bool,
    /// Gen 5+ (Taunt's `onDisableMove` / `onBeforeMove`: `move.id !== 'mefirst'`): Taunt leaves
    /// Me First alone; Gen 4 refuses it as any status move (DESIGN.md R-M8-71).
    pub taunt_spares_me_first: bool,
    /// Paralysis before Attract: Gen 2 (its paralysis check has Attract's
    /// `onBeforeMovePriority` 2 and, as a status, comes first) and Gen 3-4 on the cartridge
    /// (issues.md I66: `CANCELER_PARALYZED` / `CHECK_STATUS_STATE_PARALYSIS` before love). From
    /// Gen 5 Attract (2) comes before paralysis (1).
    pub attract_after_paralysis: bool,
    /// Where the recharge turn is checked (`RechargeCheck`).
    pub recharge_check: RechargeCheck,
    /// Showdown `moveUsed`'s Gen 2 branch: Metronome, Mimic, Mirror Move, Sketch, Sleep Talk
    /// and Transform leave no last move behind (GSC's "last move used" is untouched by them;
    /// issues.md I115). Empty in every other generation.
    pub moves_leaving_no_last_move: &'static [MoveId],
    /// Gen 3-4 on the cartridge (pokeemerald `MOVEEND_UPDATE_LAST_MOVES`: `MOVE_UNAVAILABLE`
    /// without `HITMARKER_OBEYS`; pokeplatinum `movePrevByBattler` = `MOVE_NONE` without
    /// `SYSCTL_MOVE_SUCCEEDED`; issues.md I67): a move stopped by BeforeMove leaves its user no
    /// last move. Showdown keeps the last one used (`moveUsed` is never reached).
    pub blocked_move_clears_last_move: bool,
    /// Gen 1 (the gen1 `slp` / `frz` `onBeforeMove`: `pokemon.lastMove = null`; pokered clears
    /// `wPlayerUsedMove` on the sleeping and frozen branches of `CheckPlayerStatusConditions`): a
    /// turn lost to sleep or a freeze leaves the Pokémon no last move — Mirror Move then fails.
    /// Paralysis and a confusion self-hit keep it (DESIGN.md R-M8-61).
    pub sleep_or_freeze_clears_last_move: bool,
    /// Gen 1 bug (gen1 `tryMoveHit` / `moveHit`): a sleep move never misses a Pokémon still
    /// owing its Hyper Beam recharge, puts it to sleep over any other status, and clears the
    /// recharge.
    pub recharge_sleep_bug: bool,
    /// Gen 1 (the gen1 mod's `flinch.onStart`): flinching clears a Hyper Beam recharge — the
    /// flinch takes the turn the recharge would have.
    pub flinch_clears_recharge: bool,
    /// Gen 3 Follow Me is a slot condition (Showdown gen3 `slotCondition: 'followme'`), not a
    /// volatile of its user: whoever stands in the slot draws the moves for the rest of the turn
    /// (`SideState::follow_me_slots`), and a frame does not show it on the Pokémon.
    pub follow_me_on_slot: bool,
    /// Gen 2's Substitute takes a recoil move's recoil as exactly 1 HP (Showdown gen2
    /// `substitute`: `this.damage(1, source, target, 'recoil')`).
    pub substitute_recoil_one: bool,
    /// Gen 2's Substitute (Showdown gen2 `substitute` `onTryPrimaryHit`): a draining move
    /// misses the Pokémon behind it (`-miss`), and a stalling move (Protect, Detect, Endure)
    /// fails for a user behind its own, after the stall roll.
    pub substitute_gen2_quirks: bool,
    /// Gen 1 Disable (Showdown gen1 `disable`): it seals a random move that has PP (a
    /// `DisableSlot` decision) for 1-8 of the holder's own BeforeMove checks (`time`, counted
    /// there, not in a residual), and fails only when no move has PP.
    pub disable_gen1: bool,
    /// Gen 2-4 Disable fails when the target no longer holds its last move (a Sketch that
    /// became the move it copied: the gen4 `onStart` returns false past its slot loop; pokeemerald
    /// `Cmd_disablelastusedattack` wants the move in `moves[i]`). Gen 5+ only asks the PP of a slot
    /// that still holds it and disables a move it no longer has (issues.md I187, R-M8-115).
    pub disable_needs_the_move_held: bool,
    /// Gen 1-2 `runSwitch`: every Pokémon's last move is forgotten when anyone enters (after
    /// the entry hazards), so Disable and Encore then have nothing to seal.
    pub switch_in_clears_last_moves: bool,
    /// Gen 4 `switchIn` with a source effect: the Pokémon a U-turn / Baton Pass brings in takes
    /// over its user's last move (Disable and Encore then seal the user's move on it); the one
    /// a Healing Wish / Lunar Dance heals as it enters takes that move (the gen4 slot
    /// condition's `onSwitchIn`; pokeplatinum writes the slot's `movePrevByBattler` once the
    /// move is over, the replacement already in).
    pub self_switch_passes_last_move: bool,
    /// Gen 1-4 `switchIn`: the Pokémon that takes a position takes over the last item of the one
    /// that held it (`lastItem`, DESIGN.md §39.1), which forgets it.
    pub last_item_passes_to_entrant: bool,
    /// Embargo's `duration` (5 in every generation that has it).
    pub embargo_turns: u8,
    /// Gen 4 (pokeplatinum `subscript_embargo_start.s`, issues.md I52): Embargo fails on a
    /// target whose Quick Claw or Custap Berry went off as the turn's actions were ordered and
    /// that has not moved yet (`BattleState::priority_item_pending`).
    pub embargo_fails_after_priority_item: bool,
    /// Gen 4-5 Frisk (the gen4 / gen5 mods' `onStart`): one foe drawn at random
    /// (`side.randomFoe()`, a `Sample`), its item named without its holder; from Gen 6 every
    /// foe holding an item is named.
    pub frisk_one_random_foe: bool,
    /// Gen 5+ Forewarn names the foe whose move it names (`[of] foe`); the gen4 mod's line names
    /// the move alone.
    pub forewarn_names_the_foe: bool,
    /// Gen 8+ Moonlight / Morning Sun / Synthesis / Shore Up at full HP return `NOT_FAIL`: the
    /// move did not fail (Stomping Tantrum is not doubled). The gen7 mod returns `null`, a
    /// failure.
    pub weather_heal_full_hp_not_fail: bool,
    /// Gen 8+ Purify with no status to cure returns `NOT_FAIL`: the move did not fail. The
    /// gen7 mod returns `false`, a failure (Stomping Tantrum then doubles).
    pub cure_target_nothing_not_fail: bool,
    /// Gen 8+ Heal Pulse / Floral Healing that heal nothing (full HP, Heal Block) return
    /// `NOT_FAIL`; the gen7 mod returns `null` and the gen5 mod `false`, failures.
    pub target_heal_nothing_not_fail: bool,
    /// Heal Block's `onTryHeal` stops every heal (Gen 5+); the gen4 mod's only a drain's, Leech
    /// Seed's and Wish's.
    pub heal_block_stops_every_heal: bool,
    /// Mental Herb cures only an infatuation (the gen4 mod; pokeemerald / pokeplatinum
    /// `HOLD_EFFECT_CURE_ATTRACT` / `HOLD_EFFECT_HEAL_INFATUATION`); from Gen 5 also Taunt,
    /// Encore, Torment, Disable and Heal Block.
    pub mental_herb_cures_only_infatuation: bool,
    /// Download's `onStart` skips a foe behind a Substitute (the gen4 mod).
    pub download_skips_substitutes: bool,
    /// Intrepid Sword and Dauntless Shield boost only the first entry of the battle (Gen 9
    /// `swordBoost` / `shieldBoost`; the gen8 mod boosts every entry).
    pub entry_boost_once_per_battle: bool,
    /// Whose Life Orb recoil a Gen 5+ move spares (`onAfterMoveSecondarySelf`; Gen 4's
    /// volatile-based Life Orb needs a hit through the damage formula instead).
    pub life_orb_spares: LifeOrbSpares,
    /// Gen 5 Wide Guard / Quick Guard are stalling moves (`onTry`: `StallMove`): after a
    /// Protect or guard they roll the same 1 / counter as Protect. From Gen 6 they only check
    /// that somebody else still acts.
    pub guards_roll_stall: bool,
    /// Gen 3 Lightning Rod redirects only its foes' moves (`onFoeRedirectTarget`); from Gen 4
    /// it is `onAnyRedirectTarget`, the partner's moves too.
    pub redirect_ability_foes_only: bool,
    /// Showdown `getTarget`: a move aimed at "an ally or itself" (Acupressure) whose ally has
    /// fainted goes to its user instead — in every generation but Gen 5 (`this.gen !== 5`),
    /// where it fails like any move aimed at a fainted ally.
    pub fainted_ally_retargets_self: bool,
    /// Gen 3 Lightning Rod reads the move's type from the dex (`this.dex.moves.get(move.id)`):
    /// a Hidden Power is Normal there, whatever type it takes; from Gen 4 the move's own type.
    pub redirect_reads_dex_type: bool,
    /// Protect's consecutive-use counter (Showdown `stall`).
    pub protect_stall: ProtectStallRules,
    /// From Gen 6 powder moves do not affect Grass types, and Overcoat blocks them.
    pub powder_immunity: bool,
    /// Wide Guard blocks status spread moves too from Gen 7 (Gen 6: damaging ones only).
    pub wide_guard_blocks_status: bool,
    /// King's Shield's Attack drop on a contact move it stops: −2 in Gen 6-7 (the gen7 mod),
    /// −1 from Gen 8.
    pub kings_shield_attack_drop: i8,
    /// Showdown `hitStepBreakProtect`: from Gen 6 breaking a protection also ends the target's
    /// stall counter, and takes down the guards of the user's own side too.
    pub broken_protection_gen6: bool,
    /// Gen 3-4: a broken Substitute leaves a `substitutebroken` marker naming the move that
    /// broke it; Gen 4 clears it when a foe enters.
    pub substitute_broken_marker: bool,
    pub substitute_broken_cleared_by_foe_entry: bool,
    /// Gen 4 Intimidate (Showdown gen4 mod): a foe whose Substitute U-turn just broke is out of
    /// reach like one behind a Substitute.
    pub intimidate_blocked_by_u_turn_broken_substitute: bool,
    /// Which volatiles the observers see start and go (DESIGN.md §47.0 item 13, R-M10-26).
    pub volatile_visibility: crate::state::VolatileVisibility,
    /// Gen 3-4 Intimidate is not announced when no adjacent foe is in reach: the gen3 mod's
    /// `onStart` only hints that every target has a Substitute (pokeemerald's
    /// `BattleScript_IntimidateActivatesLoop` skips a target behind one, its message per target
    /// affected), the gen4 mod's stays silent; every other generation announces it, and a
    /// Substitute answers `-immune` (DESIGN.md R-M10-24).
    pub intimidate_silent_without_reachable_foe: bool,
    /// From Gen 6 a move's `sure_hit_user_type` holds (Toxic used by a Poison type never misses
    /// and reaches a semi-invulnerable target).
    pub typed_sure_hit: Option<TypedSureHit>,
    pub tailwind_turns: u8,
    /// Gen 4 Tailwind counts down at `onSideResidualOrder` 5, before the weather (8) and every
    /// other residual that can knock a Pokémon out; from Gen 5 it is 26, among the last.
    pub tailwind_ticks_first: bool,
    pub tailwind_speed_modifier: Fraction,
    pub trick_room_turns: u8,
    pub weather_turns: u8,
    pub sun_fire_modifier: Fraction,
    pub sun_water_modifier: Fraction,
    pub rain_fire_modifier: Fraction,
    /// Gen 2 (pokecrystal `WeatherMoveModifiers`: `EFFECT_SOLARBEAM` in the rain is
    /// `NOT_VERY_EFFECTIVE`; Showdown's gen2 damage formula, `move.id === 'solarbeam'`): Solar
    /// Beam's rain halving is a weather step of the damage, not its power's (Gen 3+'s
    /// `onBasePower`, `PowerCondition::WeakeningWeather`).
    pub rain_halves_sun_charge: Option<Fraction>,
    pub rain_water_modifier: Fraction,
    /// The terrains (Electric, Grassy, Misty, Psychic).
    pub terrain: super::TerrainRules,
    /// Defog (DESIGN.md §14.2).
    pub defog: super::DefogRules,
    /// The pseudo-weathers (DESIGN.md §23.4).
    pub field: super::FieldRules,
    /// Knock Off, Thief / Covet, Sticky Hold (DESIGN.md §39.3).
    pub items: super::ItemTransferRules,
    /// A user fainting by its own move (DESIGN.md §30.4).
    pub self_faint: super::SelfFaintRules,
    /// Moves that send their user out (DESIGN.md §30.6).
    pub switching: super::SwitchingRules,
    /// Berry numbers by generation (DESIGN.md §40.3).
    pub berries: super::BerryRules,
    pub retaliation: super::RetaliationRules,
    /// Thrash and its kind (DESIGN.md §15.12).
    pub rampage: super::RampageRules,
    /// Uproar (DESIGN.md §15.12).
    pub uproar: super::UproarRules,
    /// Lock-On, Mind Reader (DESIGN.md §15.14).
    pub lock_on: super::LockOnRules,
    /// Spite, Eerie Spell (DESIGN.md §15.15).
    pub pp: super::PpRules,
    /// Curse (DESIGN.md §15.16).
    pub curse: super::CurseRules,
    /// Fury Cutter (DESIGN.md §15.18).
    pub fury_cutter: super::FuryCutterRules,
    /// Rage (DESIGN.md §15.19).
    pub rage: super::RageRules,
    /// Truant (DESIGN.md §15.21).
    pub truant: super::TruantRules,
    /// Future Sight and Doom Desire (DESIGN.md §15.22).
    pub future_attack: super::FutureAttackRules,
    /// Gen 6+ Sky Drop (DESIGN.md §15.23): a target of 200 kg or more is too heavy to carry up
    /// (`-fail … [heavy]`); the gen5 `onTryHit` has no such check.
    pub sky_drop_heavy_fails: bool,
    /// Destiny Bond, Grudge (DESIGN.md §15.9).
    pub faint_bonds: super::FaintBondRules,
    /// Foresight, Odor Sleuth, Miracle Eye (DESIGN.md §41.5).
    pub identify: super::IdentifyRules,
    /// Gen 4 (pokeplatinum effect script 0252's `CheckAbility`; Showdown's gen4 `onStart` reads
    /// the ability itself, issues.md I95): Magnet Rise fails on a user with Levitate in effect.
    pub magnet_rise_fails_with_levitate: bool,
    /// Gen 6+ Oblivious also refuses Attract and Taunt at TryHit and ends a Taunt at `Update`;
    /// the gen5 mod's (Gen 3-5) refuses Captivate alone and ends an infatuation alone
    /// (DESIGN.md R270).
    pub oblivious_covers_taunt: bool,
    /// The gen7 mod's Soundproof (Gen 3-7) refuses its holder's own sound moves too (no
    /// `target !== source`): its own Perish Song spares it (DESIGN.md R275).
    pub soundproof_blocks_own_moves: bool,
    /// The gen4 mod's Wonder Guard (Gen 3-4) lets these through ("Fire Fang is always able to
    /// hit through Wonder Guard").
    pub wonder_guard_spared_moves: &'static [crate::ids::MoveId],
    /// Gen 6+ Anticipation reads Hidden Power as its user's type (`target.hpType`); the gen5
    /// mod's (Gen 4-5) reads the move's own.
    pub anticipation_reads_hidden_power_type: bool,
    /// Forewarn's power for a one-hit KO move: 160 in the gen4 mod, 150 from Gen 5.
    pub forewarn_ohko_power: u16,
    /// The `powder` condition's `onTryMove` comes before a strong weather's (the gen6 mod's
    /// priority 1, a Pokémon's handler ahead of the field's) in Gen 6, after everything at
    /// priority −1 from Gen 7.
    pub powder_before_strong_weather: bool,
    /// Reflect / Light Screen / Aurora Veil.
    pub screens: super::ScreenRules,
    /// The timed side conditions (the screens, Mist, Safeguard, Lucky Chant) and Mist.
    pub side_protections: super::SideProtectionRules,
}

/// How long a timed volatile lasts: a fixed number of turns or a `Duration` decision (inclusive
/// range; Showdown `random(a, b)` is `a..b-1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimedDuration {
    Fixed(u8),
    Rolled { min: u8, max: u8 },
}

/// The `onStart` adjustment to a timed volatile's duration, from the target's place in the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationAdjust {
    None,
    /// +1 when the target has been on the field for a turn and already acted (Taunt, Gen 5+).
    PlusIfActedAndSettled,
    /// +1 when the target does not act again this turn (Encore; Disable before Gen 5).
    PlusIfWontMove,
    /// −1 when the target still acts this turn (Disable, Gen 5+).
    MinusIfWillMove,
}

/// When a Gen 5+ Life Orb holds back its recoil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeOrbSpares {
    /// Gen 4 (pokeplatinum `AFTER_MOVE_HIT_STATE_LIFE_ORB`): nothing — a one-hit KO pays too
    /// (issues.md I193).
    Nothing,
    /// Gen 5-6 (gen6 mod): a one-hit KO move.
    OneHitKo,
    /// Gen 7+: a user being forced out (`forceSwitchFlag`: the target's Red Card went off).
    ForcedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedVolatileRules {
    pub duration: TimedDuration,
    pub adjust: DurationAdjust,
}

/// How much a weather-dependent healing move restores (Showdown `onHit`): `boosted` in its
/// weather (sun; sandstorm for Shore Up), `weakened` in any other weather (Moonlight …),
/// `normal` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeatherHealRules {
    pub boosted: HealAmount,
    pub weakened: HealAmount,
    pub normal: HealAmount,
}

/// Gen 2 heals in full in sun; Gen 3-4 take a truncated fraction of max HP; Gen 5+ Showdown's
/// `modify(max HP, factor)` (0.667 → 2732 / 4096, rounded half down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealAmount {
    Full,
    Fraction(Fraction),
    Modify(u32),
}

impl HealAmount {
    pub fn of(self, max_hp: u16) -> u16 {
        let value = match self {
            Self::Full => u32::from(max_hp),
            Self::Fraction(fraction) => fraction.apply_u32(u32::from(max_hp)),
            Self::Modify(modifier) => (u32::from(max_hp) * modifier + 2047) / 4096,
        };
        value.clamp(1, u32::from(max_hp)) as u16
    }
}

/// Protect / Detect's consecutive-use counter (Showdown `stall`, DESIGN.md §10.6, §31.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectStallRules {
    /// Gen 3+: success `1 / counter`; the counter starts at `initial` and is multiplied on
    /// every success up to `max` (Gen 6+: 3 × 3 up to 729; Gen 5: 2 × 2 up to 256, then 1 /
    /// 2^32, which the engine treats as 1 / 256; Gen 3-4: 2 × 2 up to 8). From Gen 6 a failure
    /// ends it (`delete pokemon.volatiles['stall']`); the gen5 mod's `onStallMove` (Gen 3-5)
    /// leaves it to run out.
    Growing {
        initial: u16,
        multiplier: u16,
        max: u16,
        failure_ends: bool,
    },
    /// Gen 2: success `counter / 255`; the counter starts at `initial` (127) and halves on
    /// every success, a counter of 0 counts as `initial` again, and a failure keeps it (the
    /// `stall` volatile only runs out).
    Halving { initial: u16 },
}

impl ProtectStallRules {
    /// The counter a fresh `stall` starts with.
    pub const fn start(self) -> u16 {
        match self {
            Self::Growing { initial, .. } | Self::Halving { initial } => initial,
        }
    }

    /// The counter after another success.
    pub fn next(self, counter: u16) -> u16 {
        match self {
            Self::Growing {
                multiplier, max, ..
            } => {
                if counter < max {
                    counter.saturating_mul(multiplier).min(max)
                } else {
                    counter
                }
            }
            Self::Halving { .. } => counter / 2,
        }
    }

    /// The success chance with a live counter, as a ratio.
    pub const fn chance(self, counter: u16) -> (u16, u16) {
        match self {
            Self::Growing { .. } => (1, counter),
            Self::Halving { initial } => (if counter == 0 { initial } else { counter }, 255),
        }
    }

    /// Whether a failed Protect ends the `stall` volatile.
    pub const fn failure_ends_stall(self) -> bool {
        matches!(
            self,
            Self::Growing {
                failure_ends: true,
                ..
            }
        )
    }
}

/// How a fraction of a damage amount is turned into an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    Floor,
    HalfUp,
}

impl Rounding {
    pub fn apply(self, value: u32, fraction: Fraction) -> u32 {
        let numerator = value.saturating_mul(u32::from(fraction.numerator));
        let denominator = u32::from(fraction.denominator.max(1));
        match self {
            Self::Floor => numerator / denominator,
            Self::HalfUp => (numerator + denominator / 2) / denominator,
        }
    }
}

/// Which held items react to a Pokémon's switch-in (Showdown `onAnySwitchIn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnySwitchInReach {
    /// Gen 4+ (Showdown; pokeplatinum `SWITCH_IN_CHECK_STATE_HELD_ITEM_STATUS` for every battler
    /// once the switch-in abilities are done): every Pokémon on the field.
    Everyone,
    /// Gen 3 (pokeemerald): the newcomer's own item (`ITEMEFFECT_ON_SWITCH_IN`), and after the
    /// action only the first battler's (`HandleFaintedMonActions` state 6:
    /// `ItemBattleEffects(ITEMEFFECT_NORMAL, 0, …)`, P1's first slot); the others wait for the
    /// next move's end (`MOVEEND_ITEM_EFFECTS_ALL`) or the turn's.
    EntrantAndFirstSlot,
}

/// What Shell Bell heals an eighth of (issues.md I47).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellBellCount {
    /// Gen 3 (pokeemerald `ITEMEFFECT_KINGSROCK_SHELLBELL` at every move end, `shellBellDmg`):
    /// each hit's damage on its own, a Substitute's included — a multi-hit move's after each hit
    /// (`moveendto MOVEEND_NEXT_TARGET`), any other's (a spread move's target by target) once
    /// the move is over.
    EachHit,
    /// Gen 4 (pokeplatinum `shellBellDamageDealt`): the action's damage to HP and to Substitutes
    /// together, once after the move.
    MoveWithSubstitutes,
    /// Gen 5+ (Showdown `move.totalDamage`): the move's damage to HP, once after the move.
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoilTiming {
    /// Gen 1: recoil is taken after every hit from that hit's damage.
    PerHit,
    /// Gen 2+: one recoil from the total damage once every hit has landed.
    AfterAllHits,
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
    /// Gen 3 (its mod: `getAllActive`): any active Pokémon, a foe too, with the other ability.
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

/// How a typed sure hit (`ConditionRules::typed_sure_hit`) works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedSureHit {
    /// Gen 6-7 (the gen7 mod's Toxic `onPrepareHit`): the user takes the `toxic` volatile for
    /// the turn.
    Volatile,
    /// Gen 8+: the hit steps check the move and the user's type; nothing is left on the user.
    HitSteps,
}

/// Effect Spore's draw on a contact hit (Showdown `onDamagingHit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectSporeRoll {
    /// Gen 3-4 (their mods): a chance on a hit that dealt damage, then a `sample` of sleep /
    /// paralysis / poison.
    ChanceThenSample(Fraction),
    /// Gen 5+: one `random(100)` — below 11 sleep, below 21 paralysis, below 30 poison
    /// (`engine::effect_spore_status`) — against an attacker not immune to powder.
    Percent,
}

/// How a move with crash damage (`hasCrashDamage`: High Jump Kick, Jump Kick, Axe Kick,
/// Supercell Slam) hurts its user when it hit no target (Showdown `onMoveFail`: a miss, an
/// immunity, Protect).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashDamage {
    /// Gen 1: 1 HP (`directDamage(1)`).
    OneHp,
    /// Gen 2-4: a share of the damage the move would have dealt the target (`getDamage`: a
    /// fresh critical-hit check and damage roll), at least 1, as the move's own damage (Magic
    /// Guard lets it through). Gen 2-3 spare a user whose target is immune to the move's type;
    /// Gen 4 counts such a target's max HP instead, and Gen 3-4 cap it at half the target's
    /// max HP.
    ShareOfDamage {
        divisor: u16,
        immune_spared: bool,
        capped_at_half_target_max_hp: bool,
    },
    /// Gen 5+: half the user's max HP, as the move's condition (Magic Guard refuses it).
    HalfUserMaxHp,
}

/// Hit-sequence rules (ROADMAP §6.4 / §6.5): multi-hit count distribution, recoil and drain.
/// Values follow Showdown's `hitStepMoveHitLoop`, `calcRecoilDamage` and `spreadDamage` per
/// generation (verified against the pinned @pkmn/sim source when this table was written).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitRules {
    /// Relative weights of 2, 3, 4 and 5 hits for the `[2, 5]` multi-hit range.
    pub multi_hit_weights: [u16; 4],
    /// Gen 1: every hit of a multi-hit move deals the first hit's damage.
    pub same_damage_each_hit: bool,
    /// Gen 1-2 (the gen1 / gen2 `moveHit`: `!move.multihit || move.lastHit`) and Gen 3
    /// (pokeemerald `BattleScript_MultiHitEnd`'s one `seteffectwithchance`, issues.md I35): a
    /// multi-hit move's secondaries roll on its last hit only (Twineedle's poison), none when the
    /// sequence ends before it.
    pub secondaries_on_last_hit: bool,
    /// Pollen Puff on a full-HP ally: Gen 7 (its gen7 mod `onHit`) reports `-immune` and the
    /// move fails; Gen 8+ returns `NOT_FAIL` — nothing happens and the move counts as used.
    pub ally_heal_fails_at_full_hp: bool,
    pub recoil_timing: RecoilTiming,
    pub recoil_rounding: Rounding,
    /// Recoil never rounds below 1 HP.
    pub recoil_minimum: u16,
    pub drain_rounding: Rounding,
    /// Gen 1-4 drain at least 1 HP; Gen 5+ may drain 0 (no heal event).
    pub drain_minimum: u16,
    /// Rough Skin: fraction of the attacker's max HP (Gen 3: 1/16, Gen 4+: 1/8).
    pub rough_skin_divisor: u16,
    /// Weak Armor's Speed rise after a physical hit: +1 before Gen 7 (the gen6 mod), +2 after.
    pub weak_armor_speed: i8,
    pub crash: CrashDamage,
    /// Fell Stinger's boost for knocking its target out (the gen6 mod: +2; Gen 7+: +3).
    pub knock_out_boost_stages: i8,
    /// Gen 5-6: a move's own immunity check (`hitStepTryImmunity`) comes after TryHit; Gen 1-2
    /// and 7+ right after the type immunity (Gen 3-4 place it themselves, `step_order`).
    pub move_immunity_after_try_hit: bool,
    /// Gen 1-4 (the gen4 mod's Dream Eater `onTryImmunity`): a Substitute makes the target
    /// immune too.
    pub dream_eater_substitute_immune: bool,
    /// Gen 5-6 (the gen6 mod's `damage(…, true)`): an attacker Rough Skin, Iron Barbs or Rocky
    /// Helmet knocks out faints at once, before the Pokémon its hit knocked out — the holder's
    /// side then fainted last.
    pub contact_reaction_instafaint: bool,
    /// Gen 4+ (Showdown `hitStepMoveHitLoop` ends with `faintMessages`, before the recoil and
    /// `AfterMoveSecondarySelf`; the gen1-3 mods' `tryMoveHit` do not): the `Faint` event of a
    /// Pokémon the move knocked out runs as the hit loop ends — its Destiny Bond takes the user
    /// down before the user's recoil (`BattleContext::knocked_out_bonds`).
    pub bonds_at_hit_loop_end: bool,
    /// Gen 1-4 contact reactions need the hit to have dealt damage (Showdown gen4 mod
    /// `damage &&`); from Gen 5 a 0-damage contact hit still triggers them.
    pub contact_reaction_needs_damage: bool,
    /// Gen 3-4: Static / Flame Body / Poison Point roll only against an attacker still standing
    /// (pokeemerald `AbilityBattleEffects` and pokeplatinum `BattleSystem_TriggerAbilityOnHit`
    /// check `hp != 0` before the `Random()`). From Gen 5 Showdown's `onDamagingHit` rolls
    /// whatever the attacker's HP — a Final Gambit user is down before its hit's reactions
    /// (DESIGN.md R-M8-152).
    pub contact_status_needs_living_attacker: bool,
    /// Gen 1-4 (Showdown `spreadMoveHit`: `gen < 5`): the move's `onAfterHit` runs before the
    /// struck Pokémon's `DamagingHit` reactions; from Gen 5 after them.
    pub after_hit_before_reactions: bool,
    pub effect_spore: EffectSporeRoll,
    /// Sturdy leaves 1 HP from full HP from Gen 5; before that it only blocks OHKO moves.
    pub sturdy_survives_from_full_hp: bool,
    /// Gen 7+: Magic Guard keeping an ability's damage off names that ability on the damage's
    /// source (`-activate|source|ability: …`); the gen6 mod's `onDamage` (Gen 4-6) says nothing
    /// (R-M10-42).
    pub magic_guard_names_ability: bool,
    /// Gen 3 Volt Absorb lets Thunder Wave through (the gen3 mod's `onTryHit` excludes it);
    /// from Gen 4 it absorbs every Electric move.
    pub volt_absorb_spares_thunder_wave: bool,
    /// Gen 4+: a Ground move's immunity through Levitate names the ability (`runImmunity(move,
    /// true)`: `-immune|p|[from] ability: Levitate`); Gen 3's hit step (the gen3 mod's
    /// `tryMoveHit`) writes `-immune|p` alone (R-M10-39).
    pub levitate_immunity_named: bool,
    /// Accuracy and secondary chances (Gen 1-2 byte-sized).
    pub chance_scale: ChanceScale,
    /// Gen 3+ (the percent scale): how the accuracy / evasion stages and the modifiers
    /// (Compound Eyes, Sand Veil, Bright Powder …) combine.
    pub accuracy_order: AccuracyOrder,
    /// Lax Incense's Gen 3-4 percentage: 100 minus its hold-effect parameter (Emerald 5,
    /// Platinum 10).
    pub lax_incense_percent: u32,
    /// Gen 3 (the gen3 mod's Hustle): a "physical" move for an accuracy modifier is one of a
    /// physical type (`PokemonType::physical_before_split`) — a status move of such a type too;
    /// from Gen 4 it is a physical move.
    pub physical_accuracy_by_type: bool,
    /// Gen 1-2: a secondary inflicting one of these never rolls against a target sharing the
    /// move's type (Gen 2 burn / freeze, Gen 1 paralysis too; Showdown gen1 / gen2 `moveHit`).
    pub secondary_same_type_immunity: &'static [crate::state::NonVolatileStatus],
    /// Gen 2: a flinch never rolls against a sleeping or frozen target.
    pub flinch_needs_awake_target: bool,
    /// King's Rock / Razor Fang (DESIGN.md R119).
    pub item_flinch: super::ItemFlinchRules,
    /// One-hit KO moves (DESIGN.md §22.6).
    pub ohko: OhkoRules,
    /// The order of the hit gate's steps (DESIGN.md §15.1).
    pub step_order: HitStepOrder,
    /// Gen 4+: a spread move passes every target's gate, then hits them stage by stage
    /// (Showdown `spreadMoveHit`). Gen 1-3 run the whole hit target by target (the mods'
    /// `tryMoveHit` per target: gate, damage, secondaries, recoil, then the next target).
    pub staged_spread_hits: bool,
    /// Gen 3: a damaging move whose last hit a Substitute took records the attack with no HP
    /// taken (the gen3 `tryMoveHit` calls `gotAttacked` after `moveHit`; pokeemerald's
    /// `MOVEEND_MIRROR_MOVE` writes the taken move, the hit having an effect, R-M8-102). Gen 4+
    /// drop such a target before `gotAttacked` (`spreadMoveHit`). A move that failed records
    /// nothing in any generation — Gen 3 too since R-M8-143: `MOVEEND_MIRROR_MOVE` skips a move
    /// with `MOVE_RESULT_NO_EFFECT`, a failure included (issues.md I173; Showdown's gen3 records
    /// it, the oracle is patched). Read by Gen 3-4 Mirror Move and Gen 1-4 Counter / Mirror Coat
    /// (DESIGN.md R-M8-80).
    pub records_substitute_hits: bool,
    /// A multi-hit move a sleep-usable move called (Sleep Talk's Twineedle) keeps hitting while
    /// its user sleeps (Showdown `hitStepMoveHitLoop`'s `isSleepUsable`; Emerald's
    /// `BattleScript_MultiHitLoop` skips the sleep check for `MOVE_SLEEP_TALK`); Gen 4 stops it
    /// after the first hit (`this.battle.gen === 4`) as it stops any sleeper's (DESIGN.md
    /// R-M8-81).
    pub sleep_callers_hit_on: bool,
    /// Gen 2-3: the caller that lets a sleeping user's multi-hit move go on is the move the
    /// action used — Sleep Talk calling Metronome calling Bone Rush hits every time (pokeemerald's
    /// `BattleScript_MultiHitLoop` reads `gChosenMove`; pokecrystal's `BattleCommand_EndLoop`
    /// checks no sleep at all). Gen 4+ read the move's direct caller (Showdown `sourceEffect`;
    /// issues.md I203, R-M8-186).
    pub sleep_check_reads_the_chosen_move: bool,
    /// Struggle's quarter-max-HP recoil (Gen 4+): Gen 4 floors it, later generations round it
    /// (Showdown `clampIntRange(trunc(maxhp / 4), 1)` / `Math.round(baseMaxhp / 4)`).
    pub struggle_recoil_rounding: Rounding,
}

/// The order in which a foreign target passes the hit gate's steps (Showdown
/// `trySpreadMoveHit`'s `moveSteps`, and the Gen 1-3 mods' `tryMoveHit`). Semi-invulnerability
/// always comes first; a status move meets a Substitute last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitStepOrder {
    /// Gen 1-2, 5-6: type immunity → TryHit (Protect, absorbing abilities) → accuracy.
    ImmunityFirst,
    /// Gen 7+: TryHit → type immunity → accuracy.
    TryHitFirst,
    /// Gen 4: type immunity → accuracy → TryHit.
    AccuracyBeforeTryHit,
    /// Gen 3: accuracy is rolled whether or not the target is immune by type; a hit then meets
    /// TryHit, and only then the immunity (a miss against an immune target is `-immune`).
    AccuracyFirst,
}

/// One-hit KO moves: who they fail against, how their accuracy works, what they deal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OhkoRules {
    /// Gen 1: fail against a faster target; ordinary accuracy; the target's max HP.
    SpeedGate,
    /// Gen 2: fail against a higher-level target; the byte accuracy gains twice the level
    /// difference (capped at 255), then the stages apply; the target's max HP.
    LevelGateByteBonus,
    /// Gen 3+: fail against a higher-level target (or, Sheer Cold from Gen 7, an Ice type);
    /// accuracy is the move's (20 for a typed one whose user lacks the type) plus the level
    /// difference, out of 100, no stages. Gen 3 deals the target's current HP, later its max.
    /// Gen 3 hits when `Random() % 100 + 1 < chance` (pokeemerald `Cmd_tryKO`: one point less,
    /// `one_point_less`, issues.md I11); Gen 4 (pokeplatinum `BtlCmd_TryOHKOMove`: `% 100 <`)
    /// on.
    LevelGatePercent {
        current_hp_damage: bool,
        one_point_less: bool,
    },
}

/// Which residual (end-of-turn) effects the engine schedules, in Showdown's `onResidualOrder`
/// / `onResidualSubOrder` terms (ROADMAP §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResidualKind {
    /// The weather's upkeep (Showdown `onFieldResidualOrder`): duration tick, then sandstorm /
    /// hail damage on every Pokémon that is not immune.
    Weather,
    /// Gen 2: a frozen Pokémon's 25/256 chance to thaw (`frz` `onResidualOrder` 7).
    FreezeThaw,
    /// Shed Skin's chance to cure the holder.
    ShedSkin,
    /// Gen 3 Rain Dish's heal in rain (its gen3 `onResidual`; later an `onWeather`).
    RainDish,
    /// Gen 5+ Hydration's cure in rain (Gen 4: an `onWeather`).
    Hydration,
    /// Wish's slot condition: the turn it was made it counts down, the next it heals.
    Wish,
    /// Future Sight / Doom Desire's slot condition (`futuremove`): two counts, then the strike.
    FutureAttack,
    /// Perish Song's count (the holder faints at 0).
    PerishSong,
    /// Grassy Terrain's heal (its `onResidual`, run for every active Pokémon: a grounded one
    /// that is not semi-invulnerable gets 1/16 of its max HP).
    GrassyTerrain,
    /// A combined Fire Pledge's side (`onResidualOrder` 5.1): every non-Fire type on that side
    /// loses 1/8 of its max HP.
    FirePledge,
    Leftovers,
    /// Gen 2-3's berries at the end of the turn (`onResidual`: Gen 3's HP and confusion berries,
    /// Gen 2's Berry, Gold Berry and Berry Juice — and, as the cartridge has them, Gen 2's status
    /// berries: issues.md I33).
    Berry,
    /// Ingrain's heal.
    Ingrain,
    /// Bind / Wrap / Fire Spin …: the trapped Pokémon's chip (ended when the trapper left).
    PartiallyTrapped,
    /// Octolock's Defense and Special Defense drops (ended when the octolocker left).
    Octolock,
    BlackSludge,
    /// Toxic Orb / Flame Orb: the holder's own status, late in the schedule.
    StatusOrb,
    /// Speed Boost: +1 Speed, late in the schedule.
    SpeedBoost,
    /// Slow Start's countdown (`onResidualOrder` 28.2 in every generation that has it).
    SlowStart,
    /// Gen 3 Truant's flag flipping (the gen3 mod's `onResidual`, order 27; pokeemerald
    /// `ABILITYEFFECT_ENDTURN`).
    Truant,
    LeechSeed,
    Poison,
    Toxic,
    Burn,
    Taunt,
    Encore,
    Disable,
    /// Nightmare's chip (Gen 2: after the holder's own action instead).
    Nightmare,
    /// Curse's chip (Gen 2: after the holder's own action instead).
    Curse,
    /// Yawn's countdown; the holder falls asleep when it runs out.
    Yawn,
    /// Heal Block's countdown.
    HealBlock,
    /// Embargo's countdown (Gen 5+ 21, Gen 4 10.18).
    Embargo,
    /// Magnet Rise's countdown (Gen 5+ 18, Gen 4 10.16).
    MagnetRise,
    /// Telekinesis's countdown (19).
    Telekinesis,
    /// Healer's chance to cure an adjacent ally (Gen 5+).
    Healer,
    /// Salt Cure's chip.
    SaltCure,
    /// Syrup Bomb's countdown and Speed drop.
    SyrupBomb,
    /// Throat Chop's countdown.
    ThroatChop,
    /// Laser Focus's countdown (its `duration`, no residual order of its own: last).
    LaserFocus,
    /// Roost's end (`duration: 1`, `onResidualOrder` 25): the holder is Flying again.
    Roost,
    /// Electrify's end (its `duration`, no residual order of its own: last).
    Electrify,
    /// Powder's end (its `duration`, no residual order of its own: last).
    Powder,
    /// The self-changing abilities' own form (Zen Mode, Schooling, Shields Down, Hunger Switch,
    /// Power Construct: `onResidualOrder` 29, DESIGN.md §43.4).
    FormeChange,
    /// Gen 5+ Micle Berry (its `onResidual`, no order of its own: last): eaten at a quarter of
    /// the HP.
    MicleBerry,
    /// Gen 5+ Micle Berry's boost wearing off (its `duration`, no order of its own).
    MicleBoost,
    /// Bad Dreams: 1/8 of their max HP off the holder's sleeping foes.
    BadDreams,
    /// Aqua Ring's heal: a sixteenth of the holder's max HP.
    AquaRing,
    /// Moody: +2 to a random stat, -1 to another.
    Moody,
    /// White Herb's check at the end of the turn (its `onResidual`, order 29: last).
    WhiteHerb,
    /// Mirror Herb's use of the raises it kept (order 29).
    MirrorHerb,
    /// Eject Pack's use after a drop (order 29).
    EjectPack,
    /// Opportunist's copy of the raises it kept (order 29).
    Opportunist,
    /// Dynamax's turn counter (its own `onResidualPriority: -100`: after every other residual).
    DynamaxCounter,
    /// Sticky Barb's eighth (Gen 4: 10.20, with the orbs; Gen 5+ 28.3).
    StickyBarb,
    /// Harvest (Gen 5+): the berry the holder last ate back — in the sun, or on a 1-in-2
    /// chance.
    Harvest,
    /// Cud Chew (Gen 9): the berry the holder ate eaten again, the second turn's end after.
    CudChew,
    /// Gen 3-4 Thrash and its kind on the cartridge (`ENDTURN_THRASH`,
    /// `MON_COND_CHECK_STATE_THRASH`, issues.md I61): the lock's count, and its end. From Gen 5
    /// it is a duration of the residual (`DurationEffect`'s volatiles).
    Rampage,
    /// Uproar's turn-end step: Gen 3-4 its wake-ups and count (10.11), Gen 5+ its count and
    /// Throat Chop's end (28.1).
    Uproar,
    /// Pickup (Gen 5+): an adjacent Pokémon's item used up this turn, picked up.
    Pickup,
}

/// A side or field condition that only counts down in the residual (Showdown `fieldEvent`
/// with `getKey = 'duration'`: a handler for its `duration` alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationEffect {
    Side(crate::state::SideConditionKind),
    Field(crate::state::FieldConditionKind),
    Terrain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResidualRules {
    /// `(kind, order, sub_order)`; lower runs first, ties broken by speed (faster first).
    pub order: &'static [(ResidualKind, u8, u8)],
    /// Gen 2+ (DESIGN.md §31.1, R190): the duration-only handlers take their place among
    /// the others — the side's (`onSideResidualOrder`, speed 0) and the field's
    /// (`onFieldResidualOrder`, speed 0) by these keys; one not listed, and every Pokémon
    /// volatile that only counts down, has no order and runs after every numbered handler
    /// (Showdown `comparePriority`: a volatile by its holder's speed at sub-order 2, then the
    /// side's at 4 and the field's at 5). `None`: Gen 1, whose durations are ticked at the end
    /// of the turn (`BattleEngine::expire_conditions_end_turn`).
    pub durations: Option<&'static [(DurationEffect, u8, u8)]>,
}

impl ResidualRules {
    /// The kind's `(order, sub-order)`, or `None` when the generation has no such end-of-turn
    /// residual (Gen 2's poison, burn and Leech Seed act after the Pokémon's own action).
    pub fn key(&self, kind: ResidualKind) -> Option<(u8, u8)> {
        self.order
            .iter()
            .find(|(k, _, _)| *k == kind)
            .map(|(_, order, sub)| (*order, *sub))
    }

    /// A duration-only side or field handler's `(order, sub-order)` (`durations`).
    pub fn duration_key(&self, effect: DurationEffect) -> (u8, u8) {
        self.durations
            .unwrap_or(&[])
            .iter()
            .find(|(e, _, _)| *e == effect)
            .map_or_else(
                || match effect {
                    DurationEffect::Side(_) => (u8::MAX, UNORDERED_SIDE_SUB_ORDER),
                    DurationEffect::Field(_) | DurationEffect::Terrain => {
                        (u8::MAX, UNORDERED_FIELD_SUB_ORDER)
                    }
                },
                |(_, order, sub)| (*order, *sub),
            )
    }
}

/// Showdown `resolvePriority`'s sub-orders for a condition with none of its own: on a Pokémon
/// 2, on a slot 3, on a side 4, on the field 5.
pub const UNORDERED_VOLATILE_SUB_ORDER: u8 = 2;
pub const UNORDERED_SLOT_SUB_ORDER: u8 = 3;
const UNORDERED_SIDE_SUB_ORDER: u8 = 4;
const UNORDERED_FIELD_SUB_ORDER: u8 = 5;

/// Gen 5+ duration-only handlers (Showdown `data/moves` conditions): the screens and the side's
/// other protections at 26 (Reflect 1, Light Screen 2, Safeguard 3, Mist 4, Tailwind 5, Lucky
/// Chant 6, Aurora Veil 10), the field's at 27 (Trick Room 1, Gravity 2, Water Sport 3, Mud
/// Sport 4, Wonder Room 5, the terrain 7). The guards of a turn (Wide / Quick Guard, Crafty
/// Shield, Mat Block), Echoed Voice, Ion Deluge and Fairy Lock have no order.
const MODERN_DURATION_ORDER: &[(DurationEffect, u8, u8)] = {
    use crate::state::{FieldConditionKind as F, SideConditionKind as S};
    &[
        (DurationEffect::Side(S::Reflect), 26, 1),
        (DurationEffect::Side(S::LightScreen), 26, 2),
        (DurationEffect::Side(S::Safeguard), 26, 3),
        (DurationEffect::Side(S::Mist), 26, 4),
        (DurationEffect::Side(S::Tailwind), 26, 5),
        (DurationEffect::Side(S::LuckyChant), 26, 6),
        (DurationEffect::Side(S::AuroraVeil), 26, 10),
        (DurationEffect::Field(F::TrickRoom), 27, 1),
        (DurationEffect::Field(F::Gravity), 27, 2),
        (DurationEffect::Field(F::WaterSport), 27, 3),
        (DurationEffect::Field(F::MudSport), 27, 4),
        (DurationEffect::Field(F::WonderRoom), 27, 5),
        (DurationEffect::Field(F::MagicRoom), 27, 6),
        (DurationEffect::Terrain, 27, 7),
    ]
};

/// Showdown's `breakable` flag by the generations it holds in (Dark Aura and Fairy Aura lose it
/// in Gen 8), as exported into `tests/fixtures/oracle/ability-flags.json`.
const BREAKABLE: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::BATTLE_ARMOR, Generation::Gen3, Generation::Gen9),
    (AbilityId::CLEAR_BODY, Generation::Gen3, Generation::Gen9),
    (AbilityId::DAMP, Generation::Gen3, Generation::Gen9),
    (AbilityId::FLASH_FIRE, Generation::Gen3, Generation::Gen9),
    (AbilityId::HYPER_CUTTER, Generation::Gen3, Generation::Gen9),
    (AbilityId::IMMUNITY, Generation::Gen3, Generation::Gen9),
    (AbilityId::INNER_FOCUS, Generation::Gen3, Generation::Gen9),
    (AbilityId::INSOMNIA, Generation::Gen3, Generation::Gen9),
    (AbilityId::KEEN_EYE, Generation::Gen3, Generation::Gen9),
    (AbilityId::LEVITATE, Generation::Gen3, Generation::Gen9),
    (AbilityId::LIGHTNING_ROD, Generation::Gen3, Generation::Gen9),
    (AbilityId::LIMBER, Generation::Gen3, Generation::Gen9),
    (AbilityId::MAGMA_ARMOR, Generation::Gen3, Generation::Gen9),
    (AbilityId::MARVEL_SCALE, Generation::Gen3, Generation::Gen9),
    (AbilityId::OBLIVIOUS, Generation::Gen3, Generation::Gen9),
    (AbilityId::OWN_TEMPO, Generation::Gen3, Generation::Gen9),
    (AbilityId::SAND_VEIL, Generation::Gen3, Generation::Gen9),
    (AbilityId::SHELL_ARMOR, Generation::Gen3, Generation::Gen9),
    (AbilityId::SHIELD_DUST, Generation::Gen3, Generation::Gen9),
    (AbilityId::SOUNDPROOF, Generation::Gen3, Generation::Gen9),
    (AbilityId::STICKY_HOLD, Generation::Gen3, Generation::Gen9),
    (AbilityId::STURDY, Generation::Gen3, Generation::Gen9),
    (AbilityId::SUCTION_CUPS, Generation::Gen3, Generation::Gen9),
    (AbilityId::THICK_FAT, Generation::Gen3, Generation::Gen9),
    (AbilityId::VITAL_SPIRIT, Generation::Gen3, Generation::Gen9),
    (AbilityId::VOLT_ABSORB, Generation::Gen3, Generation::Gen9),
    (AbilityId::WATER_ABSORB, Generation::Gen3, Generation::Gen9),
    (AbilityId::WATER_VEIL, Generation::Gen3, Generation::Gen9),
    (AbilityId::WHITE_SMOKE, Generation::Gen3, Generation::Gen9),
    (AbilityId::WONDER_GUARD, Generation::Gen3, Generation::Gen9),
    (AbilityId::DRY_SKIN, Generation::Gen4, Generation::Gen9),
    (AbilityId::FILTER, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen4, Generation::Gen9),
    (AbilityId::HEATPROOF, Generation::Gen4, Generation::Gen9),
    (AbilityId::LEAF_GUARD, Generation::Gen4, Generation::Gen9),
    (AbilityId::MOTOR_DRIVE, Generation::Gen4, Generation::Gen9),
    (AbilityId::SIMPLE, Generation::Gen4, Generation::Gen9),
    (AbilityId::SNOW_CLOAK, Generation::Gen4, Generation::Gen9),
    (AbilityId::SOLID_ROCK, Generation::Gen4, Generation::Gen9),
    (AbilityId::STORM_DRAIN, Generation::Gen4, Generation::Gen9),
    (AbilityId::TANGLED_FEET, Generation::Gen4, Generation::Gen9),
    (AbilityId::UNAWARE, Generation::Gen4, Generation::Gen9),
    (AbilityId::BIG_PECKS, Generation::Gen5, Generation::Gen9),
    (AbilityId::CONTRARY, Generation::Gen5, Generation::Gen9),
    (AbilityId::FRIEND_GUARD, Generation::Gen5, Generation::Gen9),
    (AbilityId::HEAVY_METAL, Generation::Gen5, Generation::Gen9),
    (AbilityId::LIGHT_METAL, Generation::Gen5, Generation::Gen9),
    (AbilityId::MAGIC_BOUNCE, Generation::Gen5, Generation::Gen9),
    (AbilityId::MULTISCALE, Generation::Gen5, Generation::Gen9),
    (AbilityId::SAP_SIPPER, Generation::Gen5, Generation::Gen9),
    (AbilityId::TELEPATHY, Generation::Gen5, Generation::Gen9),
    (AbilityId::WONDER_SKIN, Generation::Gen5, Generation::Gen9),
    (AbilityId::AROMA_VEIL, Generation::Gen6, Generation::Gen9),
    (AbilityId::AURA_BREAK, Generation::Gen6, Generation::Gen9),
    (AbilityId::BULLETPROOF, Generation::Gen6, Generation::Gen9),
    (AbilityId::DARK_AURA, Generation::Gen6, Generation::Gen7),
    (AbilityId::FAIRY_AURA, Generation::Gen6, Generation::Gen7),
    (AbilityId::FLOWER_VEIL, Generation::Gen6, Generation::Gen9),
    (AbilityId::FUR_COAT, Generation::Gen6, Generation::Gen9),
    (AbilityId::GRASS_PELT, Generation::Gen6, Generation::Gen9),
    (AbilityId::OVERCOAT, Generation::Gen6, Generation::Gen9),
    (AbilityId::SWEET_VEIL, Generation::Gen6, Generation::Gen9),
    (AbilityId::DAZZLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (AbilityId::FLUFFY, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::QUEENLY_MAJESTY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::WATER_BUBBLE, Generation::Gen7, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_SCALES, Generation::Gen8, Generation::Gen9),
    (AbilityId::MIRROR_ARMOR, Generation::Gen8, Generation::Gen9),
    (AbilityId::PASTEL_VEIL, Generation::Gen8, Generation::Gen9),
    (AbilityId::PUNK_ROCK, Generation::Gen8, Generation::Gen9),
    (AbilityId::ARMOR_TAIL, Generation::Gen9, Generation::Gen9),
    (AbilityId::EARTH_EATER, Generation::Gen9, Generation::Gen9),
    (AbilityId::GOOD_AS_GOLD, Generation::Gen9, Generation::Gen9),
    (AbilityId::GUARD_DOG, Generation::Gen9, Generation::Gen9),
    (AbilityId::ILLUMINATE, Generation::Gen9, Generation::Gen9),
    (AbilityId::MINDS_EYE, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::PURIFYING_SALT,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::THERMAL_EXCHANGE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::WELL_BAKED_BODY,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::WIND_RIDER, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `failroleplay` flag by the generations it holds in: Role Play (and Doodle) fails on
/// these abilities.
const FAIL_ROLE_PLAY: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::WONDER_GUARD, Generation::Gen3, Generation::Gen9),
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen5, Generation::Gen9),
    (AbilityId::FORECAST, Generation::Gen5, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::IMPOSTER, Generation::Gen5, Generation::Gen9),
    (AbilityId::TRACE, Generation::Gen5, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (
        AbilityId::POWER_OF_ALCHEMY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RECEIVER, Generation::Gen7, Generation::Gen9),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `failskillswap` flag by the generations it holds in: Skill Swap fails when either
/// Pokémon has one of these abilities.
const FAIL_SKILL_SWAP: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::WONDER_GUARD, Generation::Gen3, Generation::Gen9),
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `noentrain` flag by the generations it holds in: Entrainment fails when its user has
/// one of these abilities.
const NO_ENTRAIN: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen5, Generation::Gen9),
    (AbilityId::FORECAST, Generation::Gen5, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::IMPOSTER, Generation::Gen5, Generation::Gen9),
    (AbilityId::TRACE, Generation::Gen5, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (
        AbilityId::POWER_OF_ALCHEMY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RECEIVER, Generation::Gen7, Generation::Gen9),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::WONDER_GUARD, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `noreceiver` flag by the generations it holds in: Receiver and Power of Alchemy do
/// not take these abilities from a fainted ally.
const NO_RECEIVER: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::WONDER_GUARD, Generation::Gen3, Generation::Gen9),
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen5, Generation::Gen9),
    (AbilityId::FORECAST, Generation::Gen5, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::IMPOSTER, Generation::Gen5, Generation::Gen9),
    (AbilityId::TRACE, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (
        AbilityId::POWER_OF_ALCHEMY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RECEIVER, Generation::Gen7, Generation::Gen9),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `notrace` flag by the generations it holds in: Trace copies none of these.
const NO_TRACE: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::FORECAST, Generation::Gen4, Generation::Gen9),
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::TRACE, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen5, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::IMPOSTER, Generation::Gen5, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (
        AbilityId::POWER_OF_ALCHEMY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RECEIVER, Generation::Gen7, Generation::Gen9),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// How the forme-changing abilities differ by generation (DESIGN.md §43.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormeRules {
    /// Gen 7-8: Battle Bond's Greninja-Bond becomes Ash-Greninja. Gen 9 raises its Attack,
    /// Special Attack and Speed instead (the Ash-Greninja forme went with the generation).
    pub battle_bond_becomes_ash_greninja: bool,
    /// Gen 8+: breaking Disguise costs an eighth of the holder's max HP (Gen 7 costs nothing).
    pub disguise_costs_hp: bool,
    /// Gen 7 (the gen7 mod's `onDamage`; Sword and Shield fixed it in 1.3.1): a Rollout / Ice
    /// Ball hit Disguise takes counts as a hit and not as a doubling, and a chain that ends its
    /// five hits short of doublings stores the rest (the Rollout storage glitch, issues.md I212).
    pub disguise_delays_rolling: bool,
    /// The abilities whose permanent forme goes back to the set's as the holder faints (Showdown
    /// `formeRegression`, which these set themselves): Power Construct (Gen 7+), Battle Bond
    /// (the gen8 mod, Gen 7-8), Disguise (the gen7 and gen8 mods; the base's, Gen 9, does not) —
    /// R-M8-111, R-M8-116.
    pub regressing_abilities: &'static [AbilityId],
    /// Gen 6+: Autotomize lightens `weighthg` itself, which every forme change sets anew to the
    /// forme's (`setSpecies`): the weight it took off is gone (Bulbapedia: "A Pokémon's weight
    /// is reset if it changes form (from Generation VI onward)", R-M9-43). Gen 5's lightening is
    /// the user's volatile (`onModifyWeight`), which stays.
    pub resets_autotomize: bool,
}

/// Showdown's `notransform` flag by the generations it holds in (DESIGN.md §43.3).
const NO_TRANSFORM: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen9),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// A set of abilities: one of Showdown's ability flags as a generation has it (DESIGN.md §41).
/// Its members in id order (`Deref` to the slice), and a bitset over the ids, so `contains` —
/// asked whenever a handler or a suppression is looked up — is one bit test (R-M10-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilitySet {
    members: &'static [AbilityId],
    bits: &'static [u64],
}

impl AbilitySet {
    pub fn contains(&self, ability: &AbilityId) -> bool {
        let id = usize::from(ability.0);
        self.bits
            .get(id / 64)
            .is_some_and(|word| (word >> (id % 64)) & 1 == 1)
    }
}

impl std::ops::Deref for AbilitySet {
    type Target = [AbilityId];

    fn deref(&self) -> &[AbilityId] {
        self.members
    }
}

impl IntoIterator for AbilitySet {
    type Item = &'static AbilityId;
    type IntoIter = std::slice::Iter<'static, AbilityId>;

    fn into_iter(self) -> Self::IntoIter {
        self.members.iter()
    }
}

/// Where one generation's set lives for the program's life: its members and its bitset.
type SetCell = std::sync::OnceLock<(Vec<AbilityId>, Vec<u64>)>;

/// The set `members` builds, built once into `cell`.
fn ability_set(cell: &'static SetCell, members: impl FnOnce() -> Vec<AbilityId>) -> AbilitySet {
    let (members, bits) = cell.get_or_init(|| {
        let members = members();
        let words = members
            .iter()
            .map(|ability| usize::from(ability.0) / 64 + 1)
            .max()
            .unwrap_or(0);
        let mut bits = vec![0u64; words];
        for ability in &members {
            bits[usize::from(ability.0) / 64] |= 1 << (ability.0 % 64);
        }
        (members, bits)
    });
    AbilitySet { members, bits }
}

/// The abilities a flag table holds in `generation`, resolved once per generation into `sets`.
fn flagged(
    sets: &'static [SetCell; 9],
    table: &[(AbilityId, Generation, Generation)],
    generation: Generation,
) -> AbilitySet {
    ability_set(&sets[usize::from(generation.number() - 1)], || {
        table
            .iter()
            .filter(|(_, from, until)| (*from..=*until).contains(&generation))
            .map(|(ability, ..)| *ability)
            .collect()
    })
}

/// `breakable` for `generation`, from `BREAKABLE`.
fn breakable(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, BREAKABLE, generation)
}

/// `failroleplay` for `generation`, from `FAIL_ROLE_PLAY`.
fn fail_role_play(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, FAIL_ROLE_PLAY, generation)
}

/// `noentrain` for `generation`, from `NO_ENTRAIN`.
fn no_entrain(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, NO_ENTRAIN, generation)
}

/// `noreceiver` for `generation`, from `NO_RECEIVER`.
fn no_receiver(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, NO_RECEIVER, generation)
}

/// `notrace` for `generation`, from `NO_TRACE`.
fn no_trace(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, NO_TRACE, generation)
}

/// `notransform` for `generation`, from `NO_TRANSFORM`.
fn no_transform(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, NO_TRANSFORM, generation)
}

/// `failskillswap` for `generation`, from `FAIL_SKILL_SWAP`.
fn fail_skill_swap(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, FAIL_SKILL_SWAP, generation)
}

/// `cantsuppress` for `generation`.
fn cant_suppress(generation: Generation) -> AbilitySet {
    static SETS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    ability_set(&SETS[usize::from(generation.number() - 1)], || {
        let members: &[AbilityId] = match generation {
            Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => &[],
            Generation::Gen4 | Generation::Gen5 => &[AbilityId::MULTITYPE],
            Generation::Gen6 => &[AbilityId::MULTITYPE, AbilityId::STANCE_CHANGE],
            Generation::Gen7 => GEN7_CANT_SUPPRESS,
            Generation::Gen8 => GEN8_CANT_SUPPRESS,
            Generation::Gen9 => GEN9_CANT_SUPPRESS,
        };
        members.to_vec()
    })
}

/// Gen 7's `cantsuppress` abilities: the battle-form ones join Multitype and Stance Change.
const GEN7_CANT_SUPPRESS: &[AbilityId] = &[
    AbilityId::BATTLE_BOND,
    AbilityId::COMATOSE,
    AbilityId::DISGUISE,
    AbilityId::MULTITYPE,
    AbilityId::POWER_CONSTRUCT,
    AbilityId::RKS_SYSTEM,
    AbilityId::SCHOOLING,
    AbilityId::SHIELDS_DOWN,
    AbilityId::STANCE_CHANGE,
    AbilityId::ZEN_MODE,
];

/// Gen 8's: Gen 7's, As One, Gulp Missile and Ice Face.
const GEN8_CANT_SUPPRESS: &[AbilityId] = &[
    AbilityId::AS_ONE_GLASTRIER,
    AbilityId::AS_ONE_SPECTRIER,
    AbilityId::BATTLE_BOND,
    AbilityId::COMATOSE,
    AbilityId::DISGUISE,
    AbilityId::GULP_MISSILE,
    AbilityId::ICE_FACE,
    AbilityId::MULTITYPE,
    AbilityId::POWER_CONSTRUCT,
    AbilityId::RKS_SYSTEM,
    AbilityId::SCHOOLING,
    AbilityId::SHIELDS_DOWN,
    AbilityId::STANCE_CHANGE,
    AbilityId::ZEN_MODE,
];

/// Gen 9's: Gen 8's, Tera Shift and Zero to Hero.
const GEN9_CANT_SUPPRESS: &[AbilityId] = &[
    AbilityId::AS_ONE_GLASTRIER,
    AbilityId::AS_ONE_SPECTRIER,
    AbilityId::BATTLE_BOND,
    AbilityId::COMATOSE,
    AbilityId::DISGUISE,
    AbilityId::GULP_MISSILE,
    AbilityId::ICE_FACE,
    AbilityId::MULTITYPE,
    AbilityId::POWER_CONSTRUCT,
    AbilityId::RKS_SYSTEM,
    AbilityId::SCHOOLING,
    AbilityId::SHIELDS_DOWN,
    AbilityId::STANCE_CHANGE,
    AbilityId::TERA_SHIFT,
    AbilityId::ZEN_MODE,
    AbilityId::ZERO_TO_HERO,
];

/// Gen 4 (Showdown gen4 mod): Gravity 9, Trick Room 13, neither with a sub-order. The side's
/// protections and Tailwind count down before the residual (`side_protections.tick_first`,
/// `tailwind_ticks_first`).
const GEN4_DURATION_ORDER: &[(DurationEffect, u8, u8)] = &[
    (
        DurationEffect::Field(crate::state::FieldConditionKind::Gravity),
        9,
        UNORDERED_FIELD_SUB_ORDER,
    ),
    (
        DurationEffect::Field(crate::state::FieldConditionKind::TrickRoom),
        13,
        UNORDERED_FIELD_SUB_ORDER,
    ),
];

/// Gen 2 (Showdown gen2 mod): Safeguard 8, Reflect and Light Screen 9, none with a sub-order.
const GEN2_DURATION_ORDER: &[(DurationEffect, u8, u8)] = &[
    (
        DurationEffect::Side(crate::state::SideConditionKind::Safeguard),
        8,
        UNORDERED_SIDE_SUB_ORDER,
    ),
    (
        DurationEffect::Side(crate::state::SideConditionKind::Reflect),
        9,
        UNORDERED_SIDE_SUB_ORDER,
    ),
    (
        DurationEffect::Side(crate::state::SideConditionKind::LightScreen),
        9,
        UNORDERED_SIDE_SUB_ORDER,
    ),
];

/// Gen 5+ (Showdown `data/conditions`, `items`, `moves`): weather 1, Leftovers 5.4, Leech
/// Seed 8, poison / toxic 9, burn 10, Nightmare 11, Taunt 15, Encore 16, Disable 17, Heal Block
/// 20, Yawn 23.
const MODERN_RESIDUAL_ORDER: &[(ResidualKind, u8, u8)] = &[
    (ResidualKind::Weather, 1, 0),
    (ResidualKind::FutureAttack, 3, 3),
    (ResidualKind::Wish, 4, 0),
    (ResidualKind::FirePledge, 5, 1),
    (ResidualKind::GrassyTerrain, 5, 2),
    (ResidualKind::ShedSkin, 5, 3),
    (ResidualKind::Hydration, 5, 3),
    (ResidualKind::Healer, 5, 3),
    (ResidualKind::Leftovers, 5, 4),
    (ResidualKind::BlackSludge, 5, 4),
    (ResidualKind::AquaRing, 6, 0),
    (ResidualKind::Ingrain, 7, 0),
    (ResidualKind::LeechSeed, 8, 0),
    (ResidualKind::Poison, 9, 0),
    (ResidualKind::Toxic, 9, 0),
    (ResidualKind::Burn, 10, 0),
    (ResidualKind::Nightmare, 11, 0),
    (ResidualKind::Curse, 12, 0),
    (ResidualKind::PartiallyTrapped, 13, 0),
    (ResidualKind::SaltCure, 13, 0),
    (ResidualKind::SyrupBomb, 14, 0),
    (ResidualKind::Octolock, 14, 0),
    (ResidualKind::PerishSong, 24, 0),
    (ResidualKind::Taunt, 15, 0),
    (ResidualKind::Encore, 16, 0),
    (ResidualKind::Disable, 17, 0),
    (ResidualKind::MagnetRise, 18, 0),
    (ResidualKind::Telekinesis, 19, 0),
    (ResidualKind::HealBlock, 20, 0),
    (ResidualKind::Embargo, 21, 0),
    (ResidualKind::ThroatChop, 22, 0),
    (ResidualKind::Roost, 25, 0),
    (ResidualKind::LaserFocus, u8::MAX, 0),
    (ResidualKind::Electrify, u8::MAX, 0),
    (ResidualKind::Powder, u8::MAX, 0),
    (ResidualKind::MicleBoost, u8::MAX, 2),
    (ResidualKind::MicleBerry, u8::MAX, 8),
    (ResidualKind::Yawn, 23, 0),
    (ResidualKind::SpeedBoost, 28, 2),
    (ResidualKind::SlowStart, 28, 2),
    (ResidualKind::BadDreams, 28, 2),
    (ResidualKind::Uproar, 28, 1),
    (ResidualKind::Moody, 28, 2),
    (ResidualKind::Harvest, 28, 2),
    (ResidualKind::CudChew, 28, 2),
    (ResidualKind::Pickup, 28, 2),
    (ResidualKind::StatusOrb, 28, 3),
    (ResidualKind::StickyBarb, 28, 3),
    (ResidualKind::FormeChange, 29, 0),
    (ResidualKind::WhiteHerb, 29, 0),
    (ResidualKind::MirrorHerb, 29, 0),
    (ResidualKind::EjectPack, 29, 0),
    (ResidualKind::Opportunist, 29, 0),
    (ResidualKind::DynamaxCounter, u8::MAX, 1),
];

/// Gen 4 (Showdown gen4 mod): weather 8, then everything at 10 with a sub-order — Leftovers
/// 10.4, Leech Seed 10.5, burn / poison / toxic 10.6, Nightmare 10.7, Curse 10.8, Disable 10.13, Encore
/// 10.14, Taunt 10.15, Heal Block 10.17, Yawn 10.19.
const GEN4_RESIDUAL_ORDER: &[(ResidualKind, u8, u8)] = &[
    (ResidualKind::Wish, 7, 0),
    (ResidualKind::Weather, 8, 0),
    (ResidualKind::Ingrain, 10, 1),
    (ResidualKind::AquaRing, 10, 2),
    (ResidualKind::ShedSkin, 10, 3),
    (ResidualKind::SpeedBoost, 10, 3),
    (ResidualKind::Leftovers, 10, 4),
    (ResidualKind::BlackSludge, 10, 4),
    (ResidualKind::LeechSeed, 10, 5),
    (ResidualKind::Poison, 10, 6),
    (ResidualKind::Toxic, 10, 6),
    (ResidualKind::Burn, 10, 6),
    (ResidualKind::Nightmare, 10, 7),
    (ResidualKind::Curse, 10, 8),
    (ResidualKind::PartiallyTrapped, 10, 9),
    (ResidualKind::BadDreams, 10, 10),
    (ResidualKind::Uproar, 10, 11),
    (ResidualKind::Rampage, 10, 12),
    (ResidualKind::Disable, 10, 13),
    (ResidualKind::Encore, 10, 14),
    (ResidualKind::Taunt, 10, 15),
    (ResidualKind::MagnetRise, 10, 16),
    (ResidualKind::HealBlock, 10, 17),
    (ResidualKind::Embargo, 10, 18),
    (ResidualKind::Roost, 25, 0),
    (ResidualKind::Yawn, 10, 19),
    (ResidualKind::StatusOrb, 10, 20),
    (ResidualKind::StickyBarb, 10, 20),
    (ResidualKind::FutureAttack, 11, 3),
    (ResidualKind::PerishSong, 12, 0),
    (ResidualKind::SlowStart, 28, 2),
    (ResidualKind::WhiteHerb, 29, 0),
];

/// Gen 3: Gen 4's table and the berries' 10.4 (the gen3 mod's `onResidual`), but the gen3 mod
/// drops Disable's `onResidualOrder`, and a handler
/// without an order runs after every numbered one (`comparePriority`): a Disable ticks last, so
/// a battle decided earlier in the residual leaves it unticked.
const GEN3_RESIDUAL_ORDER: &[(ResidualKind, u8, u8)] = &[
    (ResidualKind::Wish, 7, 0),
    (ResidualKind::Weather, 8, 0),
    (ResidualKind::Ingrain, 10, 1),
    (ResidualKind::ShedSkin, 10, 3),
    (ResidualKind::RainDish, 10, 3),
    (ResidualKind::SpeedBoost, 10, 3),
    (ResidualKind::Leftovers, 10, 4),
    (ResidualKind::BlackSludge, 10, 4),
    (ResidualKind::Berry, 10, 4),
    (ResidualKind::LeechSeed, 10, 5),
    (ResidualKind::Poison, 10, 6),
    (ResidualKind::Toxic, 10, 6),
    (ResidualKind::Burn, 10, 6),
    (ResidualKind::Nightmare, 10, 7),
    (ResidualKind::Curse, 10, 8),
    (ResidualKind::PartiallyTrapped, 10, 9),
    (ResidualKind::Uproar, 10, 11),
    (ResidualKind::Rampage, 10, 12),
    (ResidualKind::Encore, 10, 14),
    (ResidualKind::Taunt, 10, 15),
    (ResidualKind::Yawn, 10, 19),
    (ResidualKind::StatusOrb, 10, 20),
    (ResidualKind::FutureAttack, 11, 3),
    (ResidualKind::PerishSong, 12, 0),
    (ResidualKind::Truant, 27, 0),
    (ResidualKind::Disable, u8::MAX, 0),
    (ResidualKind::WhiteHerb, 29, 0),
];

/// Gen 2 (Showdown gen2 mod over the gen3 / gen4 conditions): weather 2, Bind / Wrap … 3.1,
/// Leftovers 5.1, the freeze's thaw 7, the berries 10, Encore 13, and Disable last (gen3 drops
/// its order).
/// Poison, burn, bad poison, Leech Seed, Nightmare and Curse are not here: they act after the
/// Pokémon's own action (`StatusResidualTiming::AfterOwnAction`).
const GEN2_RESIDUAL_ORDER: &[(ResidualKind, u8, u8)] = &[
    (ResidualKind::FutureAttack, 1, 3),
    (ResidualKind::Weather, 2, 0),
    (ResidualKind::PartiallyTrapped, 3, 1),
    (ResidualKind::PerishSong, 4, 0),
    (ResidualKind::Leftovers, 5, 1),
    (ResidualKind::FreezeThaw, 7, 0),
    (ResidualKind::Berry, 10, 0),
    (ResidualKind::Encore, 13, 0),
    (ResidualKind::Disable, u8::MAX, 0),
];

/// Weather (DESIGN.md §23.1): duration, who sets it for how long, what it damages and what it
/// shields. `WeatherKind::Snow` is hail before Gen 9 and snow from Gen 9.
/// A weather move used under the weather it sets: it fails (Gen 3+), or — Gen 2 — only
/// Sandstorm fails and Rain Dance / Sunny Day start the weather over for five turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameWeatherMove {
    Fails,
    OnlySandstormFails,
}

impl SameWeatherMove {
    pub fn fails(self, kind: crate::state::WeatherKind) -> bool {
        match self {
            Self::Fails => true,
            Self::OnlySandstormFails => kind == crate::state::WeatherKind::Sandstorm,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeatherRules {
    /// A weather-setting ability lasts forever before Gen 6 (Showdown `duration = 0`).
    pub ability_weather_permanent: bool,
    /// A weather move used under the same weather (Showdown `setWeather`).
    pub same_weather_move: SameWeatherMove,
    /// Sandstorm / hail chip: max HP divided by this, at least 1.
    pub damage_divisor: u16,
    pub sandstorm_immune_types: &'static [PokemonType],
    /// Hail damages (Gen 3-8); Gen 9 snow does not.
    pub hail_damages: bool,
    pub hail_immune_types: &'static [PokemonType],
    /// Gen 4+: a Rock type's Special Defense under sandstorm.
    pub sandstorm_rock_special_defense: Fraction,
    /// Gen 9: an Ice type's Defense under snow.
    pub snow_ice_defense: Fraction,
    /// Heat / Damp / Smooth / Icy Rock on the setter (Showdown `durationCallback`).
    pub rock_turns: u8,
    /// Flower Gift in the sun (DESIGN.md §23.6): Gen 5+ only a Cherrim's; Gen 4 anyone's, and
    /// once however many hold it (pokeplatinum `BattleSystem_CountAbility`).
    pub flower_gift: FlowerGiftRules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowerGiftRules {
    /// Gen 5+ (`effectState.target.baseSpecies.baseSpecies !== 'Cherrim'`): only a Cherrim.
    pub holder_must_be_cherrim: bool,
    /// Gen 4 (the cartridge counts holders): one boost for any number of holders; Gen 5+
    /// Showdown's handlers, one per holder.
    pub once_per_side: bool,
    /// Gen 4 (issues.md I70, pokeplatinum `BattleSystem_CalcMoveDamage`): the attacker's side
    /// is counted by the ability alone, so the attacker's Mold Breaker does not set an ally's
    /// Flower Gift aside (only the defender's side checks it); Showdown's handlers are breakable
    /// on both sides.
    pub attack_ignores_mold_breaker: bool,
}

/// How an ability changes in battle (DESIGN.md §41.3): Skill Swap, Role Play, Trace …
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AbilityChangeRules {
    /// Whether and when the new ability starts (its switch-in effect).
    pub start_new: StartNewAbility,
    /// Gen 5 (Showdown `skillSwap`: `gen <= 5` and the same ability fails; Gen 3-4 follow the
    /// cartridge, which has no such check: issues.md A6 of §41.10).
    pub skill_swap_same_fails: bool,
    /// Gen 5+ (Showdown Role Play's `onTryHit`; Gen 3-4 follow the cartridge, A6).
    pub role_play_same_fails: bool,
}

/// A move that calls or copies other moves (DESIGN.md §42.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallRules {
    /// Gen 1 (`useMove`): the called move becomes the user's last move (Encore, Disable, Mimic
    /// read it). Gen 2 clears it as the chosen move is used; Gen 3+ write the chosen move.
    pub called_sets_last_move: bool,
    /// Gen 2, Gen 4: Metronome cannot draw a move the user's own move slots hold.
    pub metronome_excludes_known: bool,
    /// Where Mirror Move takes its move from.
    pub mirror_move: MirrorMoveSource,
    /// The moves each generation's Mirror Move refuses beside its source rule (Showdown's
    /// `noMirror` tables).
    pub mirror_tables: MirrorTables,
    /// Gen 4: Metronome, Assist and Copycat cannot draw a move Gravity grounds or a Heal Block
    /// on the user blocks.
    pub filters_gravity_and_heal_block: bool,
    /// Nature Power's fixed move before Gen 6 (Swift, Tri Attack, Earthquake); `None` from
    /// Gen 6, where it follows the terrain.
    pub nature_power: Option<crate::ids::MoveId>,
    /// Gen 3: a Sleep Talk draw that lands on a move with no PP left is `cant|p|nopp|move`.
    pub sleep_talk_names_depleted: bool,
    /// Gen 3-4 (`onTryHit`, which the gen3 mod inherits from gen4's): Sleep Talk fails while the
    /// user is choice-locked or Encored — pokeemerald's `Cmd_trychoosesleeptalkmove` finds every
    /// other move barred by `CheckMoveLimitations` (Choice Band, Encore) (DESIGN.md R-M8-158).
    pub sleep_talk_fails_locked: bool,
    /// Gen 4 (Showdown restores `activeMove` at the end of the outermost `runMove`): Copycat
    /// reads the outermost move of the action; Gen 5+ the innermost.
    pub copycat_reads_outermost: bool,
    /// Gen 3-4 (the gen4 mod's Magic Coat `onTryHit` removes the volatile, and Gen 3 inherits
    /// it; pokeemerald's `Cmd_attackcanceler` clears `bounceMove`): reflecting ends the coat at
    /// once; Gen 5+ leave it for the turn (DESIGN.md §42.8, R-M8-68).
    pub magic_coat_ends_on_reflection: bool,
    /// Gen 6+ (`onAllyTryHitSide` returns for a semi-invulnerable `target`; the gen5 mod's has
    /// no such check): a reflectable move aimed at the foes' side — reflected by the first of
    /// them, fastest first, wearing Magic Coat or holding Magic Bounce — is not when the
    /// Pokémon in the side's first position, the event's target (`foes(true)[0]`), is hiding
    /// (Fly, Dig … Sky Drop). Gen 3-4's side moves are not `reflectable` (the gen4 data;
    /// pokeplatinum's hazards lack `MOVE_FLAG_CAN_MAGIC_COAT`). Side moves are the only ones a
    /// partner reflects: a move aimed at a Pokémon is the target's own (`onTryHit`, DESIGN.md
    /// R-M8-77).
    pub hiding_first_stops_side_reflection: bool,
    /// Gen 3-4: a Pokémon that takes another's move over (Magic Coat, Snatch) pays one PP when
    /// that Pokémon has Pressure, whatever its side (pokeemerald `PressurePPLose`, pokeplatinum
    /// `BattleSystem_DecPPForPressure`; Showdown's gen4 Snatch, which Gen 3 inherits, but not its
    /// Magic Coat — issues.md I169). Gen 5+: nobody pays.
    pub taker_pays_pressure: Option<PressurePaidFrom>,
    /// Gen 1 (pokered `MetronomePickMove` / `MirrorMoveCopyMove` write `wPlayerSelectedMove`;
    /// Showdown's gen1 `onHit`): Metronome and Mirror Move make the move they call the side's
    /// last selected move — what a frozen or sleeping Pokémon's next action and Counter read
    /// (DESIGN.md R-M8-72).
    pub caller_selects_the_called_move: bool,
    /// Snatch is Gen 3-7 (`isNonstandard: Past` from Gen 8).
    pub snatch_exists: bool,
    /// Gen 3-4: a move Snatch took is taken again by another holder (the gen4 handler has no
    /// `sourceEffect === 'snatch'` check, and Gen 3 inherits it; pokeemerald's
    /// `BattleScript_SnatchedMove` returns into `attackcanceler`, which the snatcher runs
    /// again). Gen 5+ leave a snatched move alone (DESIGN.md R-M8-68).
    pub snatch_takes_a_snatched_move: bool,
    /// Gen 3 (the gen3 `tryMoveHit` runs the `PrepareHit` event before the move's `Try`;
    /// pokeemerald's `attackcanceler` takes a snatched move before Swallow's
    /// `stockpiletohpheal` or Rest's `trysetrest` runs): Snatch takes a move its own `onTry`
    /// would fail (a Swallow with nothing stockpiled). Gen 4+ (`trySpreadMoveHit`) run `Try`
    /// first (DESIGN.md R-M8-102).
    pub snatch_before_try: bool,
    /// Gen 5+: two allies' different Pledge moves used in the same turn combine (the second
    /// one runs first, at 150 power, with the pair's type and side condition; DESIGN.md §42.9).
    pub pledge_combines: bool,
    /// A combined Water Pledge doubles the side's secondaries (its `onModifyMove`).
    pub water_pledge_doubles_secondaries: bool,
    /// How long a Pledge combination's side condition lasts (Showdown's `duration: 4`).
    pub pledge_turns: u8,
    /// Gen 9 (`onPrepareHit`): Ally Switch rolls its consecutive-use chance (1 / the
    /// `allyswitch` counter) and fails on a miss; Gen 5-8 always succeed.
    pub ally_switch_consecutive_chance: bool,
    /// Gen 4+ (pokeplatinum `BattleControllerPlayer_FightCommand` asks Encore before the lock;
    /// Showdown's `onOverrideAction` from Gen 5): Encore turns a locked action — a charge or
    /// rampage a caller brought in, which the user's slots do not hold — into the encored move.
    /// Gen 3 (pokeemerald `HandleAction_UseMove`, issues.md I182) keeps the lock; Gen 2 too, as
    /// before (unchecked) (R-M8-137).
    pub encore_overrides_a_called_lock: bool,
    /// Gen 2+ (`source.moves.includes(move.id)`): Mimic refuses a move its user already knows.
    /// Gen 1's `onHit` writes the draw straight in, however bad a match it is.
    pub mimic_refuses_a_known_move: bool,
    /// Gen 1 Mimic (`this.sample(target.moves)`): the copied move is drawn from the target's
    /// slots rather than being its last move (DESIGN.md §42.5).
    pub mimic_draws_from_the_slots: bool,
    /// Gen 2+ (`if (source.transformed …) return false`): a transformed Pokémon's Mimic fails.
    /// Gen 1's goes ahead and writes the copies Transform made (DESIGN.md R-M8-75).
    pub mimic_fails_when_transformed: bool,
    /// Gen 1's Transform + Mirror Move / Metronome PP error (the gen1 `useMove`): a transformed
    /// Pokémon's move called by Metronome or Mirror Move gives the own slot the side last picked
    /// one PP back (`baseMoveSlots[lastSelectedMoveSlot].pp += 1`, mod 64; DESIGN.md R-M8-64).
    pub transformed_caller_restores_own_pp: bool,
    /// Gen 1 Mimic keeps the slot's PP (`source.moveSlots[moveslot].pp`): the copy and the slot
    /// share one PP byte, so what is spent from the copy is the slot's too (the gen1
    /// `deductPP` syncs a `virtual` slot into `baseMoveSlots`, DESIGN.md R-M8-54).
    pub mimic_keeps_the_slot_pp: bool,
    /// Gen 2-4 Mimic caps the copied move's PP at 5 (the gen4 mod's `onHit`, which Gen 2-3
    /// inherit; Gen 5+ use the base's `pp: move.pp` again).
    pub mimic_pp_capped: bool,
    /// Gen 2-4 (the gen4 mod's `onHit` checks `target.volatiles['substitute']`; the base's
    /// checks `source.transformed` and the target's `lastMove`).
    pub mimic_fails_behind_substitute: bool,
    /// Gen 2 Sketch always fails in a link battle (the gen2 `onHit`'s `-nothing`).
    pub sketch_always_fails: bool,
    /// Where Me First's ×1.5 applies (Gen 4's `onModifyDamagePhase2`, Gen 5+'s `onBasePower`
    /// priority 12); `None` before the move exists (DESIGN.md §42.8).
    pub me_first_boost: Option<MeFirstBoost>,
    /// Gen 5+ (Sucker Punch's `onTry`: `move.category === 'Status' && move.id !== 'mefirst'`):
    /// a target about to use Me First counts as attacking. Gen 4 (the gen4 `onTry`; pokeplatinum
    /// `BtlCmd_TrySuckerPunch` fails on a move of power 0) does not (DESIGN.md R-M8-70).
    pub me_first_counts_as_an_attack: bool,
}

/// Where Me First's ×1.5 applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeFirstBoost {
    /// Gen 5+ (`onBasePowerPriority: 12`): the BasePower chain.
    BasePower,
    /// Gen 4 (`onModifyDamagePhase2`): after the critical hit, before the random roll.
    DamagePhase2,
}

/// Which of its moves a Pokémon taking another's move over pays Pressure's PP from
/// (`CallRules::taker_pays_pressure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PressurePaidFrom {
    /// Gen 3 (pokeemerald `PressurePPLose(…, MOVE_MAGIC_COAT / MOVE_SNATCH)`): the slot holding
    /// the taking move — none when a caller (Assist, Metronome) used it.
    TheTakingMove,
    /// Gen 4 (pokeplatinum `moveSlot[battler]`): the slot the taker chose this turn, a caller's
    /// when one called the taking move.
    TheChosenMove,
}

/// Where a generation's Mirror Move takes the move it copies from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirrorMoveSource {
    /// Gen 1 (`onHit`): the opposite active Pokémon's last move, unless it is Mirror Move
    /// itself.
    Gen1FoeLastMove,
    /// Gen 2 (`onHit`): the same, without the calling moves (`MirrorTables::gen2`) and never one
    /// the user already knows.
    Gen2FoeLastMove,
    /// Gen 3-4: the last move that attacked the user (`getLastAttackedBy`), its user still
    /// knowing it.
    LastAttacker,
    /// Gen 5+: the target's last move, which must carry `mirror`.
    TargetLastMove,
}

/// The moves a generation's Mirror Move refuses whatever its source rule says (Showdown's
/// `noMirror` lists).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirrorTables {
    pub gen2: &'static [crate::ids::MoveId],
    pub gen3: &'static [crate::ids::MoveId],
    pub gen4: &'static [crate::ids::MoveId],
}

impl Default for MirrorTables {
    fn default() -> Self {
        Self {
            gen2: &[
                crate::ids::MoveId::METRONOME,
                crate::ids::MoveId::MIMIC,
                crate::ids::MoveId::MIRROR_MOVE,
                crate::ids::MoveId::SKETCH,
                crate::ids::MoveId::SLEEP_TALK,
                crate::ids::MoveId::TRANSFORM,
            ],
            gen3: &[
                crate::ids::MoveId::ASSIST,
                crate::ids::MoveId::CURSE,
                crate::ids::MoveId::DOOM_DESIRE,
                crate::ids::MoveId::FOCUS_PUNCH,
                crate::ids::MoveId::FUTURE_SIGHT,
                crate::ids::MoveId::MAGIC_COAT,
                crate::ids::MoveId::METRONOME,
                crate::ids::MoveId::MIMIC,
                crate::ids::MoveId::MIRROR_MOVE,
                crate::ids::MoveId::NATURE_POWER,
                crate::ids::MoveId::PSYCH_UP,
                crate::ids::MoveId::ROLE_PLAY,
                crate::ids::MoveId::SKETCH,
                crate::ids::MoveId::SLEEP_TALK,
                crate::ids::MoveId::SPIT_UP,
                crate::ids::MoveId::TAUNT,
                crate::ids::MoveId::TEETER_DANCE,
                crate::ids::MoveId::TRANSFORM,
            ],
            gen4: &[
                crate::ids::MoveId::ACUPRESSURE,
                crate::ids::MoveId::AROMATHERAPY,
                crate::ids::MoveId::ASSIST,
                crate::ids::MoveId::CHATTER,
                crate::ids::MoveId::COPYCAT,
                crate::ids::MoveId::COUNTER,
                crate::ids::MoveId::CURSE,
                crate::ids::MoveId::DOOM_DESIRE,
                crate::ids::MoveId::FEINT,
                crate::ids::MoveId::FOCUS_PUNCH,
                crate::ids::MoveId::FUTURE_SIGHT,
                crate::ids::MoveId::GRAVITY,
                crate::ids::MoveId::HAIL,
                crate::ids::MoveId::HAZE,
                crate::ids::MoveId::HEAL_BELL,
                crate::ids::MoveId::HELPING_HAND,
                crate::ids::MoveId::LIGHT_SCREEN,
                crate::ids::MoveId::LUCKY_CHANT,
                crate::ids::MoveId::MAGIC_COAT,
                crate::ids::MoveId::ME_FIRST,
                crate::ids::MoveId::METRONOME,
                crate::ids::MoveId::MIMIC,
                crate::ids::MoveId::MIRROR_COAT,
                crate::ids::MoveId::MIRROR_MOVE,
                crate::ids::MoveId::MIST,
                crate::ids::MoveId::MUD_SPORT,
                crate::ids::MoveId::NATURE_POWER,
                crate::ids::MoveId::PERISH_SONG,
                crate::ids::MoveId::PSYCH_UP,
                crate::ids::MoveId::RAIN_DANCE,
                crate::ids::MoveId::REFLECT,
                crate::ids::MoveId::ROLE_PLAY,
                crate::ids::MoveId::SAFEGUARD,
                crate::ids::MoveId::SANDSTORM,
                crate::ids::MoveId::SKETCH,
                crate::ids::MoveId::SLEEP_TALK,
                crate::ids::MoveId::SNATCH,
                crate::ids::MoveId::SPIT_UP,
                crate::ids::MoveId::SPIKES,
                crate::ids::MoveId::STEALTH_ROCK,
                crate::ids::MoveId::STRUGGLE,
                crate::ids::MoveId::SUNNY_DAY,
                crate::ids::MoveId::TAILWIND,
                crate::ids::MoveId::TOXIC_SPIKES,
                crate::ids::MoveId::TRANSFORM,
                crate::ids::MoveId::WATER_SPORT,
            ],
        }
    }
}

impl MirrorTables {
    /// The table the generation's Mirror Move refuses (Gen 3's stands for Gen 4 too where the
    /// gen4 mod only changed the source).
    pub fn for_generation(
        &self,
        generation: crate::spec::Generation,
    ) -> &'static [crate::ids::MoveId] {
        match generation {
            crate::spec::Generation::Gen2 => self.gen2,
            crate::spec::Generation::Gen3 => self.gen3,
            _ => self.gen4,
        }
    }
}

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

/// When a Pokémon's new ability starts (`AbilityChangeRules::start_new`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartNewAbility {
    /// Gen 3 (pokeemerald, Showdown's `gen > 3`): it does not.
    Never,
    /// Gen 4 (pokeplatinum `BattleSystem_TriggerEffectOnSwitch` at the move's end): the
    /// switch-in effect it has, unless this stint on the field already announced that kind of
    /// effect (`PokemonHistory::announced`).
    Unannounced,
    /// Gen 5+ (Showdown `setAbility`'s `Start`): at once.
    AtOnce,
}

/// Where a charging move's user is while it is out of reach (Showdown `onInvulnerability`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemiInvulnerableKind {
    /// Fly, Bounce: in the air.
    Flying,
    /// Dig: underground.
    Underground,
    /// Dive: under water.
    Underwater,
    /// Phantom Force, Shadow Force: gone (their condition's `onInvulnerability` is `false`
    /// outright — no move reaches them through it).
    Vanished,
}

impl SemiInvulnerableKind {
    /// Showdown `dig` / `dive` `onImmunity('sandstorm' | 'hail')`: no weather chip there.
    pub const fn sheltered_from_weather(self) -> bool {
        matches!(self, Self::Underground | Self::Underwater)
    }
}

/// One hiding place's exceptions (the condition's `onInvulnerability` list and its doubling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HidingRules {
    /// The moves that still reach a Pokémon hiding there.
    pub reached_by: &'static [MoveId],
    /// Among them, the ones whose damage doubles there.
    pub doubled: &'static [MoveId],
}

/// The stage at which a semi-invulnerable target's weakness doubles the damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageBonusStage {
    /// Gen 2 (`onSourceBasePower`), Gen 4 on the cartridge (the move's power multiplier).
    BasePower,
    /// Gen 3+ (`onSourceModifyDamage`, the last step of the damage formula).
    FinalDamage,
    /// Gen 2 Minimize on the cartridge (pokecrystal `doubleminimizedamage`, after
    /// `damagevariation`): the final damage, the random roll included, doubled, at most 65535
    /// (issues.md I50).
    AfterRandom,
}

/// Two-turn moves (DESIGN.md §15.5): which moves still reach a semi-invulnerable Pokémon and
/// which of those deal double damage, per hiding place and generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwoTurnRules {
    /// In the air (Fly, Bounce): Gust, Twister, Thunder, Hurricane, Sky Uppercut, Smack Down,
    /// Thousand Arrows from Gen 3 (Gust / Twister doubled); Gen 2 adds Whirlwind; Gen 1 only
    /// Swift.
    pub flying: HidingRules,
    /// Underground (Dig): Earthquake, Magnitude (doubled); Gen 2 adds Fissure.
    pub underground: HidingRules,
    /// Under water (Dive, Gen 3+): Surf, Whirlpool (doubled).
    pub underwater: HidingRules,
    /// Moves that reach a semi-invulnerable Pokémon wherever it is: Gen 5+ Helping Hand
    /// (Showdown's `move.id === 'helpinghand'` in `hitStepInvulnerabilityEvent`, which the Gen 4
    /// mod's override lacks). The only way into a vanished Pokémon.
    pub reach_everywhere: &'static [MoveId],
    pub bonus: Fraction,
    /// Where every hiding place's doubling applies, over the move's own
    /// (`TwoTurnSpec::bonus_stage`): Gen 2 (the gen2 mod's conditions all use
    /// `onSourceBasePower`), Gen 4 on the cartridge (issues.md I17).
    pub bonus_stage_override: Option<DamageBonusStage>,
    pub charge: ChargeModel,
    /// Gen 2+: a hiding Pokémon's move at itself misses it as well (Showdown's `Invulnerability`
    /// runs on every target, the user too: a Metronome Encore keeps on the turn the user is in the
    /// air from the Bounce it called, R-M10-50). The cartridge agrees in Gen 4: pokeplatinum's
    /// `BattleSystem_Defender` makes the user a `RANGE_USER` move's defender, and
    /// `CheckMoveHitOverrides` marks a defender in the air `MOVE_STATUS_SEMI_INVULNERABLE`, a
    /// miss before the move's script. Gen 1's `invulnerability` lets its holder through
    /// (`target === source`): one left hiding by a full paralysis uses its moves at itself.
    pub hides_from_itself: bool,
}

/// How a charge is kept (DESIGN.md §15.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeModel {
    /// Gen 2+ (Showdown `twoturnmove`, `duration: 2`, with the move's own condition): the charge
    /// pays the PP and is the user's last move; the hiding place is the charging move's; the
    /// attack leaves `twoturnmove` to the turn's end; any move stopped before it drops it.
    Timed,
    /// Gen 1 (pokered `CHARGING_UP` / `INVULNERABLE`; Showdown gen1 `twoturnmove` /
    /// `invulnerability`, neither with a duration): the charging turn costs no PP and is not the
    /// user's last move (`lastMove`; the side's is), the attack pays (`DecrementPP` after
    /// `PlayerCanExecuteChargingMove`) and clears both flags. Fly and Dig hide the user with a
    /// flag of its own: full paralysis and Disable (`MonHurtItselfOrFullyParalysed`,
    /// `PrintMoveIsDisabledText`) clear the charge but leave it — the user stays out of reach
    /// until it switches out or attacks from the air or ground —, a confusion self-hit clears
    /// both, any other stop (sleep, freeze, a flinch, a binding move) keeps the charge for the
    /// next turn. Skull Bash raises nothing.
    Gen1,
}

impl TwoTurnRules {
    fn place(&self, kind: SemiInvulnerableKind) -> Option<&HidingRules> {
        match kind {
            SemiInvulnerableKind::Flying => Some(&self.flying),
            SemiInvulnerableKind::Underground => Some(&self.underground),
            SemiInvulnerableKind::Underwater => Some(&self.underwater),
            SemiInvulnerableKind::Vanished => None,
        }
    }

    pub fn bypasses(&self, kind: SemiInvulnerableKind, move_id: MoveId) -> bool {
        self.reach_everywhere.contains(&move_id)
            || self
                .place(kind)
                .is_some_and(|place| place.reached_by.contains(&move_id))
    }

    pub fn doubles(&self, kind: SemiInvulnerableKind, move_id: MoveId) -> bool {
        self.place(kind)
            .is_some_and(|place| place.doubled.contains(&move_id))
    }

    /// Gen 1's charge (`ChargeModel::Gen1`).
    pub fn gen1(&self) -> bool {
        self.charge == ChargeModel::Gen1
    }

    /// The stage of the doubling against a Pokémon hiding by a move whose own stage is
    /// `spec_stage`.
    pub fn bonus_stage(&self, spec_stage: DamageBonusStage) -> DamageBonusStage {
        self.bonus_stage_override.unwrap_or(spec_stage)
    }
}

/// Showdown `dive`'s `onInvulnerability` / `onSourceModifyDamage` (Gen 3+).
const UNDERWATER: HidingRules = HidingRules {
    reached_by: &[MoveId::SURF, MoveId::WHIRLPOOL],
    doubled: &[MoveId::SURF, MoveId::WHIRLPOOL],
};

/// Gen 1: nothing but Swift (and Transform) reaches a Pokémon in the air or underground, and
/// nothing deals extra damage (Showdown gen1 `invulnerability`).
const GEN1_TWO_TURN: TwoTurnRules = TwoTurnRules {
    flying: HidingRules {
        reached_by: &[MoveId::SWIFT, MoveId::TRANSFORM],
        doubled: &[],
    },
    underground: HidingRules {
        reached_by: &[MoveId::SWIFT, MoveId::TRANSFORM],
        doubled: &[],
    },
    underwater: HidingRules {
        reached_by: &[],
        doubled: &[],
    },
    reach_everywhere: &[],
    bonus: Fraction::ONE,
    bonus_stage_override: None,
    charge: ChargeModel::Gen1,
    hides_from_itself: false,
};

/// Gen 2 (Showdown gen2 mod): Whirlwind reaches the air and Fissure the ground; the doubling
/// happens at the base-power stage.
const GEN2_TWO_TURN: TwoTurnRules = TwoTurnRules {
    flying: HidingRules {
        reached_by: &[
            MoveId::GUST,
            MoveId::TWISTER,
            MoveId::THUNDER,
            MoveId::WHIRLWIND,
        ],
        doubled: &[MoveId::GUST, MoveId::TWISTER],
    },
    underground: HidingRules {
        reached_by: &[MoveId::EARTHQUAKE, MoveId::MAGNITUDE, MoveId::FISSURE],
        doubled: &[MoveId::EARTHQUAKE, MoveId::MAGNITUDE],
    },
    underwater: HidingRules {
        reached_by: &[],
        doubled: &[],
    },
    reach_everywhere: &[],
    bonus: Fraction::new(2, 1),
    bonus_stage_override: Some(DamageBonusStage::BasePower),
    charge: ChargeModel::Timed,
    hides_from_itself: true,
};

/// Gen 3-4 (Showdown `fly` / `bounce` / `dig` / `dive` conditions; the gen4 mod's
/// `hitStepInvulnerabilityEvent`).
const GEN3_TWO_TURN: TwoTurnRules = TwoTurnRules {
    reach_everywhere: &[],
    ..MODERN_TWO_TURN
};

/// Gen 4 on the cartridge (issues.md I17): the doubling is the move's own power multiplier
/// (`BTLVAR_POWER_MULTI` 20 in pokeplatinum's effect scripts 0146 / 0149 for Gust / Twister, 0147
/// / 0126 for Earthquake / Magnitude, and for Surf / Whirlpool against a diver), the first step of
/// the power.
const GEN4_TWO_TURN: TwoTurnRules = TwoTurnRules {
    bonus_stage_override: Some(DamageBonusStage::BasePower),
    ..GEN3_TWO_TURN
};

/// Gen 5+ (Showdown `fly` / `bounce` / `dig` / `dive` conditions).
const MODERN_TWO_TURN: TwoTurnRules = TwoTurnRules {
    flying: HidingRules {
        reached_by: &[
            MoveId::GUST,
            MoveId::TWISTER,
            MoveId::SKY_UPPERCUT,
            MoveId::THUNDER,
            MoveId::HURRICANE,
            MoveId::SMACK_DOWN,
            MoveId::THOUSAND_ARROWS,
        ],
        doubled: &[MoveId::GUST, MoveId::TWISTER],
    },
    underground: HidingRules {
        reached_by: &[MoveId::EARTHQUAKE, MoveId::MAGNITUDE],
        doubled: &[MoveId::EARTHQUAKE, MoveId::MAGNITUDE],
    },
    underwater: UNDERWATER,
    reach_everywhere: &[MoveId::HELPING_HAND],
    bonus: Fraction::new(2, 1),
    bonus_stage_override: None,
    charge: ChargeModel::Timed,
    hides_from_itself: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaintRules {
    /// Gen 5+: when both sides' last Pokémon faint in the same step, the side whose Pokémon
    /// fainted last wins (Showdown `checkWin`); before that it is a draw.
    pub last_to_faint_wins_double_ko: bool,
    /// Gen 1-3: a Pokémon knocked out by a move is replaced right after that move, before the
    /// rest of the turn and its residual (Showdown `checkFainted` when the next queued action
    /// is a move or the residual); from Gen 4 at the end of the turn.
    pub replace_after_each_move: bool,
    /// What a faint does to the rest of a singles turn (Showdown `faintMessages`).
    pub singles_faint_effect: FaintTurnEffect,
    /// Gen 1: at a turn's end, a battle that cannot progress is a tie (DESIGN.md §45.7). This is
    /// the Gen 1 check of Showdown `maybeTriggerEndlessBattleClause`, and no format turns it off.
    /// It holds when every Pokémon on each side has fainted, is frozen, knows only Transform
    /// against a foe team that does too, or has no PP left against a team of Ghosts.
    pub no_progress_ties: bool,
}

/// What a faint does to the rest of a singles turn (Showdown `faintMessages`, DESIGN.md
/// §31.2). Doubles are never affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaintTurnEffect {
    /// Gen 4+: the remaining actions run.
    None,
    /// Gen 3: every remaining action — moves and switches — is cancelled; the residual runs.
    CancelActions,
    /// Gen 2: every remaining move is cancelled; switches still happen, then the residual.
    CancelMoves,
    /// Gen 1: the rest of the turn is skipped.
    SkipTurn,
}

/// Terastallization's step (Showdown `resolveAction`'s order 106). The engine's `action_priority`
/// counts *down* (a higher value acts earlier): Showdown's order 106 sits between the switch
/// (103) and the moves (200, the engine's 0), so the step reads 5 — a switch still goes first and
/// a move that was put at the front of the queue (`prioritizeAction`, the engine's 100) still
/// beats it, as in Showdown (3 < 106).
pub const TERA_ACTION_PRIORITY: i16 = 5;

/// Dynamax's step (Showdown `resolveAction`'s order 105): before Terastallization's, after the
/// switches.
pub const DYNAMAX_ACTION_PRIORITY: i16 = 6;

/// Mega Evolution's and Ultra Burst's step (Showdown `resolveAction`'s order 104): after the
/// switches (103, the engine's 8) and before Dynamax (105, 6).
pub const MEGA_ACTION_PRIORITY: i16 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionOrderRules {
    pub strategy: ActionOrderStrategy,
    /// Gen 7 (Showdown's `runAction`): a Mega Evolution takes the Pokémon's move action out of
    /// the queue and puts it back at its new speed (DESIGN.md §44.3).
    pub mega_reorders_move: bool,
    pub switch_action_priority: i16,
    pub move_action_priority: i16,
    pub pass_action_priority: i16,
    /// Terastallization (`resolveAction`'s order 106): a step of its own, after switches and
    /// before every move (DESIGN.md §44.2).
    pub tera_action_priority: i16,
    /// Dynamax (`resolveAction`'s order 105): the step before Terastallization's.
    pub dynamax_action_priority: i16,
    /// Mega Evolution and Ultra Burst (`resolveAction`'s order 104): after the switches,
    /// before Dynamax's step.
    pub mega_action_priority: i16,
    /// An action moved to the front of the queue (Showdown `prioritizeAction`: order 3, ahead of
    /// everything a turn queues).
    pub prioritized_action_priority: i16,
    /// Gen 1 (the cartridge's move menu, Showdown `side.lastSelectedMove`): a frozen or
    /// sleeping Pokémon acts with the move its side last selected from the menu, so the turn
    /// order takes that move's priority (Quick Attack's +1, Counter's −1).
    pub sleeping_frozen_act_with_last_selected: bool,
    /// Gen 1 (Showdown gen1 `side.commitChoices`: `choice.moveid = this.lastSelectedMove`; the
    /// cartridge skips the menu and keeps `wPlayerSelectedMove`): a recharging Pokémon's action
    /// is its side's last selected move — Hyper Beam again when a partial-trapping move clears
    /// the recharge before it acts (R-M8-107).
    pub recharge_acts_with_last_selected: bool,
    /// Gen 2-3 Quick Claw (Showdown `quickClawRoll`, drawn as the turn starts: 60/256 in Gen 2,
    /// 1/5 in Gen 3): when it succeeds every holder acts at Speed 65535 for the rest of the
    /// turn. `None` from Gen 4, where Quick Claw is a fractional priority rolled per move.
    pub quick_claw_turn_roll: Option<Fraction>,
    /// Lagging Tail and Full Incense's `onFractionalPriority`, in tenths of a priority step:
    /// Gen 4 −2 (run before Stall, which keeps a negative value), Gen 5+ −1.
    pub delay_item_tenths: i8,
    /// Gen 4 `getActionSpeed`: a move with a negative fractional priority orders by reversed
    /// Speed (slowest first), Trick Room or not.
    pub delayed_slowest_first: bool,
    /// Gen 4 (issues.md I44: pokeplatinum `BattleSystem_CompareBattlerSpeed` compares two
    /// Pokémon whose Quick Claw or Custap Berry went off by their Speed before its Trick Room
    /// branch): a move with a positive fractional priority orders fastest first, Trick Room or
    /// not.
    pub boosted_fastest_first: bool,
    /// Gen 4 Custap Berry (Showdown gen4 `onBeforeTurn` / `onCustap`, pokeplatinum's
    /// `subscript_check_quick_claw` as the move begins): its holder is put first in its bracket
    /// as the turn starts and eats the berry as its move begins. Gen 5+
    /// (`onFractionalPriority`): it eats it as the turn's actions are ordered.
    pub custap_eaten_as_the_move_begins: bool,
    /// Gale Wings needs its holder at full HP from Gen 7 (the gen6 mod drops the check).
    pub gale_wings_full_hp: bool,
    /// Gen 7+ `hitStepTryImmunity`: a Dark-type foe is immune to a Prankster-boosted move.
    pub prankster_dark_immunity: bool,
    /// Gen 6+ `getActionSpeed` writes the modified priority back to the move
    /// (`action.move.priority`): Quick Guard then sees Prankster's +1. Gen 5 keeps the base.
    pub boosted_priority_at_hit: bool,
    /// Where Quash sends the target's action (DESIGN.md §42.4).
    pub quash: QuashOrder,
}

/// Where Quash sends the target's pending action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuashOrder {
    /// Gen 5-7 (the gen7 mod: `priority = -7.1`, `cancelMove`, back in before the residual):
    /// to the end of the queue, after anything already there.
    End,
    /// Gen 8+ (`action.order = 201`): into a bracket of its own after every move, which the
    /// re-sort before each move orders like any other — this action priority.
    LastBracket(i16),
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
        let action_order = ActionOrderRules {
            tera_action_priority: TERA_ACTION_PRIORITY,
            dynamax_action_priority: DYNAMAX_ACTION_PRIORITY,
            mega_action_priority: MEGA_ACTION_PRIORITY,
            strategy: if matches!(generation, Generation::Gen8 | Generation::Gen9) {
                ActionOrderStrategy::RecalculateAfterAction
            } else {
                ActionOrderStrategy::StaticAtTurnStart
            },
            // A switch runs before a Mega Evolution's step of the same turn (Showdown's 103 <
            // 104): the engine's scale counts down, so the switch's value is the higher one.
            switch_action_priority: 8,
            mega_reorders_move: generation == Generation::Gen7,
            move_action_priority: 0,
            pass_action_priority: -100,
            prioritized_action_priority: 100,
            sleeping_frozen_act_with_last_selected: generation == Generation::Gen1,
            recharge_acts_with_last_selected: generation == Generation::Gen1,
            quick_claw_turn_roll: match generation {
                Generation::Gen2 => Some(Fraction::new(60, 256)),
                Generation::Gen3 => Some(Fraction::new(1, 5)),
                _ => None,
            },
            delay_item_tenths: if generation == Generation::Gen4 {
                -2
            } else {
                -1
            },
            delayed_slowest_first: generation == Generation::Gen4,
            boosted_fastest_first: generation == Generation::Gen4,
            custap_eaten_as_the_move_begins: generation == Generation::Gen4,
            gale_wings_full_hp: generation >= Generation::Gen7,
            prankster_dark_immunity: generation >= Generation::Gen7,
            boosted_priority_at_hit: generation >= Generation::Gen6,
            quash: if generation >= Generation::Gen8 {
                QuashOrder::LastBracket(-1)
            } else {
                QuashOrder::End
            },
        };
        // Values follow Showdown's conditions data per generation (verified against the pinned
        // @pkmn/sim source when this table was written).
        let lifecycle = match generation {
            // random(1, 8) sleep, no natural thaw, 63/256 paralysis, 128/256 self-hit, and the
            // waking turn is lost.
            Generation::Gen1 => StatusLifecycleRules {
                sleep_time_min: 1,
                sleep_time_max: 7,
                sleep_wake_turn_can_act: false,
                sleep_resets_on_switch_in: generation == Generation::Gen5,
                sleep_usable_turns_come_back: generation == Generation::Gen3,
                thaw_chance: Fraction::ZERO,
                thaw_timing: match generation {
                    Generation::Gen1 => ThawTiming::Never,
                    Generation::Gen2 => ThawTiming::EndOfTurn,
                    _ => ThawTiming::BeforeMove,
                },
                defrost_user: match generation {
                    Generation::Gen1 => DefrostUser::Never,
                    Generation::Gen2 => DefrostUser::AfterMove,
                    Generation::Gen3 | Generation::Gen4 => DefrostUser::AfterRoll,
                    _ => DefrostUser::SkipsRoll,
                },
                thaw_on_hit: if generation <= Generation::Gen2 {
                    ThawOnHit::BurnSecondary
                } else {
                    ThawOnHit::FireDamage
                },
                full_paralysis: Fraction::new(63, 256),
                confusion_time_min: 2,
                confusion_time_max: 5,
                confusion_self_hit: Fraction::new(1, 2),
                confusion_self_hit_varies: generation >= Generation::Gen3,
                confusion_self_hit_burn_halves: matches!(
                    generation,
                    Generation::Gen3 | Generation::Gen4
                ),
                confusion_self_hit_direct: generation <= Generation::Gen2,
                contact_status_chance: Fraction::new(1, 3),
                shed_skin_chance: Fraction::new(33, 100),
                synchronize_passes_toxic: false,
                synchronize_announced: false,
                ability_sleep_named: false,
                magic_guard_skips_full_paralysis: false,
            },
            Generation::Gen2 => StatusLifecycleRules {
                sleep_time_min: 2,
                sleep_time_max: 7,
                sleep_wake_turn_can_act: true,
                sleep_resets_on_switch_in: generation == Generation::Gen5,
                sleep_usable_turns_come_back: generation == Generation::Gen3,
                thaw_chance: Fraction::new(25, 256),
                thaw_timing: match generation {
                    Generation::Gen1 => ThawTiming::Never,
                    Generation::Gen2 => ThawTiming::EndOfTurn,
                    _ => ThawTiming::BeforeMove,
                },
                defrost_user: match generation {
                    Generation::Gen1 => DefrostUser::Never,
                    Generation::Gen2 => DefrostUser::AfterMove,
                    Generation::Gen3 | Generation::Gen4 => DefrostUser::AfterRoll,
                    _ => DefrostUser::SkipsRoll,
                },
                thaw_on_hit: if generation <= Generation::Gen2 {
                    ThawOnHit::BurnSecondary
                } else {
                    ThawOnHit::FireDamage
                },
                full_paralysis: Fraction::new(63, 256),
                confusion_time_min: 2,
                confusion_time_max: 5,
                confusion_self_hit: Fraction::new(1, 2),
                confusion_self_hit_varies: generation >= Generation::Gen3,
                confusion_self_hit_burn_halves: matches!(
                    generation,
                    Generation::Gen3 | Generation::Gen4
                ),
                confusion_self_hit_direct: generation <= Generation::Gen2,
                contact_status_chance: Fraction::new(1, 3),
                shed_skin_chance: Fraction::new(33, 100),
                synchronize_passes_toxic: false,
                synchronize_announced: false,
                ability_sleep_named: false,
                magic_guard_skips_full_paralysis: false,
            },
            Generation::Gen3 | Generation::Gen4 => StatusLifecycleRules {
                sleep_time_min: 2,
                sleep_time_max: 5,
                sleep_wake_turn_can_act: true,
                sleep_resets_on_switch_in: generation == Generation::Gen5,
                sleep_usable_turns_come_back: generation == Generation::Gen3,
                thaw_chance: Fraction::new(1, 5),
                thaw_timing: match generation {
                    Generation::Gen1 => ThawTiming::Never,
                    Generation::Gen2 => ThawTiming::EndOfTurn,
                    _ => ThawTiming::BeforeMove,
                },
                defrost_user: match generation {
                    Generation::Gen1 => DefrostUser::Never,
                    Generation::Gen2 => DefrostUser::AfterMove,
                    Generation::Gen3 | Generation::Gen4 => DefrostUser::AfterRoll,
                    _ => DefrostUser::SkipsRoll,
                },
                thaw_on_hit: if generation <= Generation::Gen2 {
                    ThawOnHit::BurnSecondary
                } else {
                    ThawOnHit::FireDamage
                },
                full_paralysis: Fraction::new(1, 4),
                confusion_time_min: 2,
                confusion_time_max: 5,
                confusion_self_hit: Fraction::new(1, 2),
                confusion_self_hit_varies: generation >= Generation::Gen3,
                confusion_self_hit_burn_halves: matches!(
                    generation,
                    Generation::Gen3 | Generation::Gen4
                ),
                confusion_self_hit_direct: generation <= Generation::Gen2,
                contact_status_chance: if generation == Generation::Gen3 {
                    Fraction::new(1, 3)
                } else {
                    Fraction::new(3, 10)
                },
                shed_skin_chance: Fraction::new(33, 100),
                synchronize_passes_toxic: false,
                synchronize_announced: false,
                ability_sleep_named: false,
                magic_guard_skips_full_paralysis: true,
            },
            Generation::Gen5 | Generation::Gen6 => StatusLifecycleRules {
                sleep_time_min: 2,
                sleep_time_max: 4,
                sleep_wake_turn_can_act: true,
                sleep_resets_on_switch_in: generation == Generation::Gen5,
                sleep_usable_turns_come_back: generation == Generation::Gen3,
                thaw_chance: Fraction::new(1, 5),
                thaw_timing: match generation {
                    Generation::Gen1 => ThawTiming::Never,
                    Generation::Gen2 => ThawTiming::EndOfTurn,
                    _ => ThawTiming::BeforeMove,
                },
                defrost_user: match generation {
                    Generation::Gen1 => DefrostUser::Never,
                    Generation::Gen2 => DefrostUser::AfterMove,
                    Generation::Gen3 | Generation::Gen4 => DefrostUser::AfterRoll,
                    _ => DefrostUser::SkipsRoll,
                },
                thaw_on_hit: if generation <= Generation::Gen2 {
                    ThawOnHit::BurnSecondary
                } else {
                    ThawOnHit::FireDamage
                },
                full_paralysis: Fraction::new(1, 4),
                confusion_time_min: 2,
                confusion_time_max: 5,
                confusion_self_hit: Fraction::new(1, 2),
                confusion_self_hit_varies: generation >= Generation::Gen3,
                confusion_self_hit_burn_halves: matches!(
                    generation,
                    Generation::Gen3 | Generation::Gen4
                ),
                confusion_self_hit_direct: generation <= Generation::Gen2,
                contact_status_chance: Fraction::new(3, 10),
                shed_skin_chance: Fraction::new(33, 100),
                synchronize_passes_toxic: true,
                synchronize_announced: true,
                ability_sleep_named: true,
                magic_guard_skips_full_paralysis: false,
            },
            Generation::Gen7 | Generation::Gen8 | Generation::Gen9 => StatusLifecycleRules {
                sleep_time_min: 2,
                sleep_time_max: 4,
                sleep_wake_turn_can_act: true,
                sleep_resets_on_switch_in: generation == Generation::Gen5,
                sleep_usable_turns_come_back: generation == Generation::Gen3,
                thaw_chance: Fraction::new(1, 5),
                thaw_timing: match generation {
                    Generation::Gen1 => ThawTiming::Never,
                    Generation::Gen2 => ThawTiming::EndOfTurn,
                    _ => ThawTiming::BeforeMove,
                },
                defrost_user: match generation {
                    Generation::Gen1 => DefrostUser::Never,
                    Generation::Gen2 => DefrostUser::AfterMove,
                    Generation::Gen3 | Generation::Gen4 => DefrostUser::AfterRoll,
                    _ => DefrostUser::SkipsRoll,
                },
                thaw_on_hit: if generation <= Generation::Gen2 {
                    ThawOnHit::BurnSecondary
                } else {
                    ThawOnHit::FireDamage
                },
                full_paralysis: Fraction::new(1, 4),
                confusion_time_min: 2,
                confusion_time_max: 5,
                confusion_self_hit: Fraction::new(33, 100),
                confusion_self_hit_varies: generation >= Generation::Gen3,
                confusion_self_hit_burn_halves: matches!(
                    generation,
                    Generation::Gen3 | Generation::Gen4
                ),
                confusion_self_hit_direct: generation <= Generation::Gen2,
                contact_status_chance: Fraction::new(3, 10),
                shed_skin_chance: Fraction::new(33, 100),
                synchronize_passes_toxic: true,
                synchronize_announced: true,
                ability_sleep_named: true,
                magic_guard_skips_full_paralysis: false,
            },
        };
        let conditions = ConditionRules {
            // Gen 2 overrides `moveHit` with `Math.round`; Gen 1 and 3-4 floor.
            heal_rounds_half_up: generation == Generation::Gen2 || generation >= Generation::Gen5,
            heal_fails_255_short: generation == Generation::Gen1,
            stockpile: match generation {
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => StockpileRule::NoStages,
                Generation::Gen4 | Generation::Gen5 | Generation::Gen6 => {
                    StockpileRule::StagesByLayer
                }
                _ => StockpileRule::StagesAsChanged,
            },
            swallow_nothing_not_fail: generation >= Generation::Gen8,
            snatched_swallow_skips_stockpile: generation >= Generation::Gen5,
            focus_energy_stages: match generation {
                Generation::Gen1 => 0,
                Generation::Gen2 => 1,
                _ => 2,
            },
            psych_up_copies_crit_volatiles: generation >= Generation::Gen6,
            haze: HazeRules::for_generation(generation),
            charge_ends_after_any_move: generation <= Generation::Gen8,
            acupressure_fails_behind_substitute: generation == Generation::Gen4,
            moody_draws_accuracy_evasion: matches!(
                generation,
                Generation::Gen5 | Generation::Gen6 | Generation::Gen7
            ),
            team_cure: TeamCureRules::for_generation(generation),
            bench_abilities_ignored: generation >= Generation::Gen5,
            pressure_shown_to_foes: generation >= Generation::Gen4,
            bench_items_ignored: generation >= Generation::Gen5,
            breakable: breakable(generation),
            fail_role_play: fail_role_play(generation),
            fail_skill_swap: fail_skill_swap(generation),
            no_transform: no_transform(generation),
            forme: FormeRules {
                battle_bond_becomes_ash_greninja: generation <= Generation::Gen8,
                disguise_costs_hp: generation >= Generation::Gen8,
                disguise_delays_rolling: generation == Generation::Gen7,
                regressing_abilities: match generation {
                    Generation::Gen7 => &[
                        AbilityId::POWER_CONSTRUCT,
                        AbilityId::BATTLE_BOND,
                        AbilityId::DISGUISE,
                    ],
                    Generation::Gen8 => &[
                        AbilityId::POWER_CONSTRUCT,
                        AbilityId::BATTLE_BOND,
                        AbilityId::DISGUISE,
                    ],
                    Generation::Gen9 => &[AbilityId::POWER_CONSTRUCT],
                    _ => &[],
                },
                resets_autotomize: generation >= Generation::Gen6,
            },
            no_entrain: no_entrain(generation),
            no_receiver: no_receiver(generation),
            no_trace: no_trace(generation),
            ability_change: AbilityChangeRules {
                start_new: match generation {
                    Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => {
                        StartNewAbility::Never
                    }
                    Generation::Gen4 => StartNewAbility::Unannounced,
                    _ => StartNewAbility::AtOnce,
                },
                skill_swap_same_fails: generation == Generation::Gen5,
                role_play_same_fails: generation >= Generation::Gen5,
            },
            calls: CallRules {
                // Gen 1 (`useMove`): the called move becomes the user's last move.
                called_sets_last_move: generation == Generation::Gen1,
                // Gen 2's clearing list is `ConditionRules::moves_leaving_no_last_move`
                // (main's R-M8-45); a called move never writes the last move at all.
                // Gen 2 and Gen 4: Metronome cannot draw a move the user knows.
                metronome_excludes_known: matches!(generation, Generation::Gen2 | Generation::Gen4),
                // Gen 2: Mirror Move takes the foe's last move; Gen 3-4 the last move that
                // attacked the user (whose user still knows it); Gen 5+ the target's.
                mirror_move: match generation {
                    Generation::Gen1 => MirrorMoveSource::Gen1FoeLastMove,
                    Generation::Gen2 => MirrorMoveSource::Gen2FoeLastMove,
                    Generation::Gen3 | Generation::Gen4 => MirrorMoveSource::LastAttacker,
                    _ => MirrorMoveSource::TargetLastMove,
                },
                mirror_tables: MirrorTables::default(),
                filters_gravity_and_heal_block: generation == Generation::Gen4,
                magic_coat_ends_on_reflection: matches!(
                    generation,
                    Generation::Gen3 | Generation::Gen4
                ),
                hiding_first_stops_side_reflection: generation >= Generation::Gen6,
                taker_pays_pressure: match generation {
                    Generation::Gen3 => Some(PressurePaidFrom::TheTakingMove),
                    Generation::Gen4 => Some(PressurePaidFrom::TheChosenMove),
                    _ => None,
                },
                caller_selects_the_called_move: generation == Generation::Gen1,
                snatch_exists: (Generation::Gen3..=Generation::Gen7).contains(&generation),
                snatch_takes_a_snatched_move: matches!(
                    generation,
                    Generation::Gen3 | Generation::Gen4
                ),
                snatch_before_try: generation == Generation::Gen3,
                ally_switch_consecutive_chance: generation == Generation::Gen9,
                encore_overrides_a_called_lock: generation >= Generation::Gen4,
                pledge_combines: generation >= Generation::Gen5,
                pledge_turns: 4,
                water_pledge_doubles_secondaries: generation >= Generation::Gen5,
                mimic_refuses_a_known_move: generation >= Generation::Gen2,
                mimic_draws_from_the_slots: generation == Generation::Gen1,
                mimic_fails_when_transformed: generation >= Generation::Gen2,
                transformed_caller_restores_own_pp: generation == Generation::Gen1,
                mimic_keeps_the_slot_pp: generation == Generation::Gen1,
                mimic_pp_capped: (Generation::Gen2..=Generation::Gen4).contains(&generation),
                mimic_fails_behind_substitute: (Generation::Gen2..=Generation::Gen4)
                    .contains(&generation),
                sketch_always_fails: generation == Generation::Gen2,
                me_first_counts_as_an_attack: generation >= Generation::Gen5,
                me_first_boost: match generation {
                    Generation::Gen4 => Some(MeFirstBoost::DamagePhase2),
                    Generation::Gen5 | Generation::Gen6 | Generation::Gen7 => {
                        Some(MeFirstBoost::BasePower)
                    }
                    _ => None,
                },
                nature_power: match generation {
                    Generation::Gen3 => Some(crate::ids::MoveId::SWIFT),
                    Generation::Gen4 => Some(crate::ids::MoveId::TRI_ATTACK),
                    Generation::Gen5 => Some(crate::ids::MoveId::EARTHQUAKE),
                    _ => None,
                },
                // Gen 3: a Sleep Talk draw that lands on a move with no PP left is `cant nopp`.
                sleep_talk_names_depleted: generation == Generation::Gen3,
                // Gen 4: Sleep Talk fails under a Choice lock or an Encore.
                sleep_talk_fails_locked: matches!(generation, Generation::Gen3 | Generation::Gen4),
                // Gen 4: the last move Copycat reads is the outermost move of the action
                // (`activeMove = oldActiveMove`); later the innermost.
                copycat_reads_outermost: generation <= Generation::Gen4,
            },
            transform: TransformRules {
                fails_into_transformed: generation >= Generation::Gen2,
                fails_when_transformed: generation >= Generation::Gen5,
                fails_behind_substitute: generation >= Generation::Gen5,
                max_pp_capped: generation >= Generation::Gen5,
                copies_hidden_power: matches!(
                    generation,
                    Generation::Gen2 | Generation::Gen3 | Generation::Gen4
                ),
                copies_ability: generation >= Generation::Gen3,
                starts_different_ability: generation >= Generation::Gen5,
                clears_announcements: generation == Generation::Gen4,
                copies_crit_volatiles: generation >= Generation::Gen6,
                copies_autotomize: generation >= Generation::Gen6,
                ends_disable: matches!(generation, Generation::Gen3 | Generation::Gen4),
            },
            plate: PlateRules {
                switch_in_forme: matches!(generation, Generation::Gen4),
                before_switch_in_normal: matches!(generation, Generation::Gen4),
            },
            type_change: TypeRules {
                lock: match generation {
                    Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => TypeLock::None,
                    Generation::Gen4 => TypeLock::MultitypeHolder,
                    _ => TypeLock::Species(&[SpeciesId::ARCEUS.dex, SpeciesId::SILVALLY.dex]),
                },
                // The `arceus` / `silvally` conditions' `this.gen >= 8` guard, negated: before
                // Gen 8 a holder without its ability is Normal regardless of its species.
                without_ability_becomes_normal: matches!(
                    generation,
                    Generation::Gen5 | Generation::Gen6 | Generation::Gen7
                ),
                empty_becomes: if generation >= Generation::Gen5 {
                    PokemonType::Normal
                } else {
                    PokemonType::Typeless
                },
                soak_fails_if_same: generation >= Generation::Gen6,
                camouflage: match generation {
                    Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4 => {
                        Camouflage::Normal
                    }
                    Generation::Gen5 => Camouflage::Ground,
                    _ => Camouflage::ByTerrain,
                },
                conversion: match generation {
                    Generation::Gen1 => Conversion::CopyTarget,
                    Generation::Gen2 => Conversion::RandomMoveType {
                        skips_conversion: false,
                        typeless_counts: false,
                    },
                    Generation::Gen3 => Conversion::RandomMoveType {
                        skips_conversion: false,
                        typeless_counts: true,
                    },
                    Generation::Gen4 => Conversion::RandomMoveType {
                        skips_conversion: true,
                        typeless_counts: true,
                    },
                    Generation::Gen5 => Conversion::RandomMoveType {
                        skips_conversion: true,
                        typeless_counts: false,
                    },
                    _ => Conversion::FirstSlot,
                },
                conversion2_reads_last_move: generation == Generation::Gen2,
                conversion2_skips_own_types: generation >= Generation::Gen4,
                conversion2_struggle_as_normal: generation <= Generation::Gen4,
                protean_once_per_entry: generation >= Generation::Gen9,
            },
            mold_breaker_spares_user: generation >= Generation::Gen8,
            cant_suppress: cant_suppress(generation),
            liquid_ooze: LiquidOozeRules::for_generation(generation),
            flash_fire: FlashFireRules::for_generation(generation),
            minimize_sure_hit: generation >= Generation::Gen6,
            rest_sleep_turns: if generation == Generation::Gen1 { 2 } else { 3 },
            rest_renews_sleep: generation == Generation::Gen2,
            leaf_guard_spares_rest: generation == Generation::Gen4,
            minimize_bonus: match generation {
                Generation::Gen2 => DamageBonusStage::AfterRandom,
                Generation::Gen4 => DamageBonusStage::BasePower,
                _ => DamageBonusStage::FinalDamage,
            },
            switch_in_order: if generation <= Generation::Gen4 {
                SwitchInOrder::EachInTurn
            } else {
                SwitchInOrder::ByPriority
            },
            belly_drum: if generation == Generation::Gen2 {
                BellyDrumRule::ClimbUnderCap
            } else {
                BellyDrumRule::Maximize
            },
            substitute_honors_infiltrates: generation >= Generation::Gen6,
            substitute_hp_divisor: 4,
            substitute_gen1: generation == Generation::Gen1,
            confusion_secondary_through_substitute: generation == Generation::Gen1,
            stealth_rock_hp_divisor: 8,
            hazards_stop_at_knock_out: generation >= Generation::Gen5,
            spikes_damage_24ths: [3, 4, 6],
            spikes_max_layers: if generation <= Generation::Gen2 { 1 } else { 3 },
            toxic_spikes_max_layers: 2,
            sticky_web_drop_from_foe_lead: generation >= Generation::Gen9,
            leftovers_hp_divisor: 16,
            focus_band_chance: if generation <= Generation::Gen2 {
                Fraction::new(30, 256)
            } else {
                Fraction::new(1, 10)
            },
            focus_band_lasts_the_move: matches!(generation, Generation::Gen3 | Generation::Gen4),
            leech_seed_hp_divisor: if generation == Generation::Gen1 {
                16
            } else {
                8
            },
            leech_seed_gen1: generation == Generation::Gen1,
            after_own_action_reads_the_target: generation == Generation::Gen1,
            weather_heal: match generation {
                Generation::Gen1 | Generation::Gen2 => WeatherHealRules {
                    boosted: HealAmount::Full,
                    weakened: HealAmount::Fraction(Fraction::new(1, 4)),
                    normal: HealAmount::Fraction(Fraction::new(1, 2)),
                },
                Generation::Gen3 | Generation::Gen4 => WeatherHealRules {
                    boosted: HealAmount::Fraction(Fraction::new(2, 3)),
                    weakened: HealAmount::Fraction(Fraction::new(1, 4)),
                    normal: HealAmount::Fraction(Fraction::new(1, 2)),
                },
                _ => WeatherHealRules {
                    boosted: HealAmount::Modify(2732),
                    weakened: HealAmount::Modify(1024),
                    normal: HealAmount::Modify(2048),
                },
            },
            must_recharge_turns: if generation == Generation::Gen1 {
                None
            } else {
                Some(2)
            },
            recharge_skipped_on_knock_out: generation == Generation::Gen1,
            // Gen 5+: fixed with an adjustment; Gen 4 rolls (Taunt 3-5 without adjustment,
            // Encore 4-8, Disable 4-7 then +1 when the target does not move again); Gen 3: Taunt
            // 2, Encore 3-6, Disable 2-5. Gen 1-2 share Gen 3's until a scenario covers them.
            taunt: match generation {
                g if g >= Generation::Gen5 => TimedVolatileRules {
                    duration: TimedDuration::Fixed(3),
                    adjust: DurationAdjust::PlusIfActedAndSettled,
                },
                Generation::Gen4 => TimedVolatileRules {
                    duration: TimedDuration::Rolled { min: 3, max: 5 },
                    adjust: DurationAdjust::None,
                },
                _ => TimedVolatileRules {
                    duration: TimedDuration::Fixed(2),
                    adjust: DurationAdjust::None,
                },
            },
            encore: TimedVolatileRules {
                duration: match generation {
                    g if g >= Generation::Gen5 => TimedDuration::Fixed(3),
                    Generation::Gen4 => TimedDuration::Rolled { min: 4, max: 8 },
                    _ => TimedDuration::Rolled { min: 3, max: 6 },
                },
                // Gen 2's `onStart` override drops the +1 (Showdown gen2 `encore`).
                adjust: if generation == Generation::Gen2 {
                    DurationAdjust::None
                } else {
                    DurationAdjust::PlusIfWontMove
                },
            },
            disable: match generation {
                g if g >= Generation::Gen5 => TimedVolatileRules {
                    duration: TimedDuration::Fixed(5),
                    adjust: DurationAdjust::MinusIfWillMove,
                },
                Generation::Gen4 => TimedVolatileRules {
                    duration: TimedDuration::Rolled { min: 4, max: 7 },
                    adjust: DurationAdjust::PlusIfWontMove,
                },
                _ => TimedVolatileRules {
                    duration: TimedDuration::Rolled { min: 2, max: 5 },
                    adjust: DurationAdjust::PlusIfWontMove,
                },
            },
            two_turn_move_turns: 2,
            bind_turns_min: if generation >= Generation::Gen5 { 5 } else { 3 },
            bind_turns_max: match generation {
                Generation::Gen1 | Generation::Gen2 => 5,
                Generation::Gen3 | Generation::Gen4 => 6,
                _ => 6,
            },
            grip_claw_bind_turns: if generation >= Generation::Gen5 { 8 } else { 6 },
            binding_band_damage_divisor: if generation >= Generation::Gen6 { 6 } else { 8 },
            big_root_truncates: generation <= Generation::Gen4,
            any_switch_in_reach: if generation == Generation::Gen3 {
                AnySwitchInReach::EntrantAndFirstSlot
            } else {
                AnySwitchInReach::Everyone
            },
            shell_bell: match generation {
                Generation::Gen3 => ShellBellCount::EachHit,
                Generation::Gen4 => ShellBellCount::MoveWithSubstitutes,
                _ => ShellBellCount::Move,
            },
            bind_damage_divisor: if generation >= Generation::Gen6 {
                8
            } else {
                16
            },
            ghost_escapes_trapping: generation >= Generation::Gen6,
            ingrain_grounds: generation >= Generation::Gen4,
            taunt_before_field_blocks: matches!(generation, Generation::Gen3 | Generation::Gen4),
            taunt_spares_me_first: generation >= Generation::Gen5,
            attract_after_paralysis: matches!(
                generation,
                Generation::Gen2 | Generation::Gen3 | Generation::Gen4
            ),
            recharge_check: match generation {
                Generation::Gen1 => RechargeCheck::AfterFlinch,
                Generation::Gen3 | Generation::Gen4 => RechargeCheck::AfterSleepAndFreeze,
                _ => RechargeCheck::First,
            },
            moves_leaving_no_last_move: if generation == Generation::Gen2 {
                crate::rules::retaliation::GEN2_NO_LAST_MOVE
            } else {
                &[]
            },
            blocked_move_clears_last_move: matches!(
                generation,
                Generation::Gen3 | Generation::Gen4
            ),
            sleep_or_freeze_clears_last_move: generation == Generation::Gen1,
            recharge_sleep_bug: generation == Generation::Gen1,
            flinch_clears_recharge: generation == Generation::Gen1,
            follow_me_on_slot: generation == Generation::Gen3,
            substitute_recoil_one: generation == Generation::Gen2,
            substitute_gen2_quirks: generation == Generation::Gen2,
            disable_gen1: generation == Generation::Gen1,
            disable_needs_the_move_held: generation <= Generation::Gen4,
            switch_in_clears_last_moves: generation <= Generation::Gen2,
            last_item_passes_to_entrant: generation <= Generation::Gen4,
            embargo_turns: 5,
            embargo_fails_after_priority_item: generation == Generation::Gen4,
            frisk_one_random_foe: matches!(generation, Generation::Gen4 | Generation::Gen5),
            forewarn_names_the_foe: generation >= Generation::Gen5,
            self_switch_passes_last_move: generation == Generation::Gen4,
            weather_heal_full_hp_not_fail: generation >= Generation::Gen8,
            cure_target_nothing_not_fail: generation >= Generation::Gen8,
            target_heal_nothing_not_fail: generation >= Generation::Gen8,
            heal_block_stops_every_heal: generation >= Generation::Gen5,
            mental_herb_cures_only_infatuation: generation <= Generation::Gen4,
            download_skips_substitutes: generation <= Generation::Gen4,
            entry_boost_once_per_battle: generation >= Generation::Gen9,
            life_orb_spares: if generation >= Generation::Gen7 {
                LifeOrbSpares::ForcedOut
            } else if generation >= Generation::Gen5 {
                LifeOrbSpares::OneHitKo
            } else {
                LifeOrbSpares::Nothing
            },
            guards_roll_stall: generation == Generation::Gen5,
            redirect_ability_foes_only: generation == Generation::Gen3,
            fainted_ally_retargets_self: generation != Generation::Gen5,
            redirect_reads_dex_type: generation == Generation::Gen3,
            protect_stall: match generation {
                g if g >= Generation::Gen6 => ProtectStallRules::Growing {
                    initial: 3,
                    multiplier: 3,
                    max: 729,
                    failure_ends: true,
                },
                Generation::Gen5 => ProtectStallRules::Growing {
                    initial: 2,
                    multiplier: 2,
                    max: 256,
                    failure_ends: false,
                },
                Generation::Gen3 | Generation::Gen4 => ProtectStallRules::Growing {
                    initial: 2,
                    multiplier: 2,
                    max: 8,
                    failure_ends: false,
                },
                _ => ProtectStallRules::Halving { initial: 127 },
            },
            powder_immunity: generation >= Generation::Gen6,
            wide_guard_blocks_status: generation >= Generation::Gen7,
            kings_shield_attack_drop: if generation >= Generation::Gen8 {
                -1
            } else {
                -2
            },
            broken_protection_gen6: generation >= Generation::Gen6,
            typed_sure_hit: match generation {
                Generation::Gen6 | Generation::Gen7 => Some(TypedSureHit::Volatile),
                _ if generation >= Generation::Gen8 => Some(TypedSureHit::HitSteps),
                _ => None,
            },
            substitute_broken_marker: matches!(generation, Generation::Gen3 | Generation::Gen4),
            substitute_broken_cleared_by_foe_entry: generation == Generation::Gen4,
            intimidate_blocked_by_u_turn_broken_substitute: generation == Generation::Gen4,
            intimidate_silent_without_reachable_foe: matches!(
                generation,
                Generation::Gen3 | Generation::Gen4
            ),
            volatile_visibility: volatile_visibility(generation),
            tailwind_turns: if generation == Generation::Gen4 { 3 } else { 4 },
            tailwind_ticks_first: generation == Generation::Gen4,
            tailwind_speed_modifier: Fraction::new(2, 1),
            trick_room_turns: 5,
            weather_turns: 5,
            sun_fire_modifier: Fraction::new(3, 2),
            sun_water_modifier: Fraction::new(1, 2),
            rain_fire_modifier: Fraction::new(1, 2),
            rain_halves_sun_charge: (generation == Generation::Gen2).then(|| Fraction::new(1, 2)),
            rain_water_modifier: Fraction::new(3, 2),
            terrain: super::TerrainRules::for_generation(generation),
            defog: super::DefogRules::for_generation(generation),
            field: super::FieldRules::for_generation(generation),
            items: super::ItemTransferRules::for_generation(generation),
            self_faint: super::SelfFaintRules::for_generation(generation),
            switching: super::SwitchingRules::for_generation(generation),
            berries: super::BerryRules::for_generation(generation),
            retaliation: super::RetaliationRules::for_generation(generation),
            rampage: super::RampageRules::for_generation(generation),
            uproar: super::UproarRules::for_generation(generation),
            lock_on: super::LockOnRules::for_generation(generation),
            pp: super::PpRules::for_generation(generation),
            curse: super::CurseRules::for_generation(generation),
            fury_cutter: super::FuryCutterRules::for_generation(generation),
            rage: super::RageRules::for_generation(generation),
            truant: super::TruantRules::for_generation(generation),
            future_attack: super::FutureAttackRules::for_generation(generation),
            sky_drop_heavy_fails: generation >= Generation::Gen6,
            faint_bonds: super::FaintBondRules::for_generation(generation),
            identify: super::IdentifyRules::for_generation(generation),
            magnet_rise_fails_with_levitate: generation == Generation::Gen4,
            oblivious_covers_taunt: generation >= Generation::Gen6,
            soundproof_blocks_own_moves: generation <= Generation::Gen7,
            wonder_guard_spared_moves: if generation <= Generation::Gen4 {
                &[crate::ids::MoveId::FIRE_FANG]
            } else {
                &[]
            },
            anticipation_reads_hidden_power_type: generation >= Generation::Gen6,
            forewarn_ohko_power: if generation == Generation::Gen4 {
                160
            } else {
                150
            },
            powder_before_strong_weather: generation == Generation::Gen6,
            screens: super::ScreenRules::for_generation(generation),
            side_protections: super::SideProtectionRules::for_generation(generation),
        };

        let hits = HitRules {
            multi_hit_weights: if generation >= Generation::Gen5 {
                [35, 35, 15, 15]
            } else {
                [3, 3, 1, 1]
            },
            same_damage_each_hit: generation == Generation::Gen1,
            secondaries_on_last_hit: matches!(
                generation,
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3
            ),
            ally_heal_fails_at_full_hp: generation <= Generation::Gen7,
            recoil_timing: if generation == Generation::Gen1 {
                RecoilTiming::PerHit
            } else {
                RecoilTiming::AfterAllHits
            },
            recoil_rounding: if generation >= Generation::Gen5 {
                Rounding::HalfUp
            } else {
                Rounding::Floor
            },
            recoil_minimum: 1,
            drain_rounding: if generation >= Generation::Gen5 {
                Rounding::HalfUp
            } else {
                Rounding::Floor
            },
            drain_minimum: if generation >= Generation::Gen5 { 0 } else { 1 },
            rough_skin_divisor: if generation <= Generation::Gen3 {
                16
            } else {
                8
            },
            weak_armor_speed: if generation >= Generation::Gen7 { 2 } else { 1 },
            crash: match generation {
                Generation::Gen1 => CrashDamage::OneHp,
                Generation::Gen2 => CrashDamage::ShareOfDamage {
                    divisor: 8,
                    immune_spared: true,
                    capped_at_half_target_max_hp: false,
                },
                Generation::Gen3 => CrashDamage::ShareOfDamage {
                    divisor: 2,
                    immune_spared: true,
                    capped_at_half_target_max_hp: true,
                },
                Generation::Gen4 => CrashDamage::ShareOfDamage {
                    divisor: 2,
                    immune_spared: false,
                    capped_at_half_target_max_hp: true,
                },
                _ => CrashDamage::HalfUserMaxHp,
            },
            contact_reaction_instafaint: matches!(generation, Generation::Gen5 | Generation::Gen6),
            move_immunity_after_try_hit: matches!(generation, Generation::Gen5 | Generation::Gen6),
            knock_out_boost_stages: if generation == Generation::Gen6 { 2 } else { 3 },
            dream_eater_substitute_immune: generation <= Generation::Gen4,
            bonds_at_hit_loop_end: generation >= Generation::Gen4,
            contact_reaction_needs_damage: generation <= Generation::Gen4,
            contact_status_needs_living_attacker: generation <= Generation::Gen4,
            after_hit_before_reactions: generation <= Generation::Gen4,
            effect_spore: match generation {
                Generation::Gen3 => EffectSporeRoll::ChanceThenSample(Fraction::new(1, 10)),
                Generation::Gen4 => EffectSporeRoll::ChanceThenSample(Fraction::new(3, 10)),
                _ => EffectSporeRoll::Percent,
            },
            sturdy_survives_from_full_hp: generation >= Generation::Gen5,
            magic_guard_names_ability: generation >= Generation::Gen7,
            volt_absorb_spares_thunder_wave: generation == Generation::Gen3,
            levitate_immunity_named: generation >= Generation::Gen4,
            chance_scale: match generation {
                Generation::Gen1 => ChanceScale::ByteAlwaysRolls,
                Generation::Gen2 => ChanceScale::Byte,
                _ => ChanceScale::Percent,
            },
            accuracy_order: if generation >= Generation::Gen5 {
                AccuracyOrder::ModifiersThenStages
            } else {
                AccuracyOrder::StagesThenPercentages
            },
            lax_incense_percent: if generation == Generation::Gen3 {
                95
            } else {
                90
            },
            physical_accuracy_by_type: generation == Generation::Gen3,
            secondary_same_type_immunity: match generation {
                Generation::Gen1 => &[
                    crate::state::NonVolatileStatus::Paralysis,
                    crate::state::NonVolatileStatus::Burn,
                    crate::state::NonVolatileStatus::Freeze,
                ],
                Generation::Gen2 => &[
                    crate::state::NonVolatileStatus::Burn,
                    crate::state::NonVolatileStatus::Freeze,
                ],
                _ => &[],
            },
            flinch_needs_awake_target: generation == Generation::Gen2,
            item_flinch: super::ItemFlinchRules::for_generation(generation),
            struggle_recoil_rounding: if generation <= Generation::Gen4 {
                Rounding::Floor
            } else {
                Rounding::HalfUp
            },
            staged_spread_hits: generation >= Generation::Gen4,
            records_substitute_hits: generation == Generation::Gen3,
            sleep_callers_hit_on: generation != Generation::Gen4,
            sleep_check_reads_the_chosen_move: matches!(
                generation,
                Generation::Gen2 | Generation::Gen3
            ),
            step_order: match generation {
                Generation::Gen3 => HitStepOrder::AccuracyFirst,
                Generation::Gen4 => HitStepOrder::AccuracyBeforeTryHit,
                Generation::Gen1 | Generation::Gen2 | Generation::Gen5 | Generation::Gen6 => {
                    HitStepOrder::ImmunityFirst
                }
                _ => HitStepOrder::TryHitFirst,
            },
            ohko: match generation {
                Generation::Gen1 => OhkoRules::SpeedGate,
                Generation::Gen2 => OhkoRules::LevelGateByteBonus,
                _ => OhkoRules::LevelGatePercent {
                    current_hp_damage: generation == Generation::Gen3,
                    one_point_less: generation == Generation::Gen3,
                },
            },
        };
        let stats = if generation <= Generation::Gen2 {
            StatRules {
                stage_table: StageTable::Legacy,
                cap: Some(999),
                modified_stats: generation == Generation::Gen1,
                boosts_need_foe_pokemon_left: false,
                raise_stops_at_cap: generation == Generation::Gen2,
                species_item_stats: if generation == Generation::Gen2 {
                    GEN2_SPECIES_ITEM_STATS
                } else {
                    &[]
                },
                species_items_read_battling: generation <= Generation::Gen4,
                metal_powder_ignores_transform: generation == Generation::Gen3,
                slowing_items_ignore_suppression: false,
                unaware_needs_hp: false,
            }
        } else {
            StatRules {
                stage_table: StageTable::Modern,
                cap: None,
                modified_stats: false,
                species_item_stats: &[],
                species_items_read_battling: generation <= Generation::Gen4,
                metal_powder_ignores_transform: generation == Generation::Gen3,
                boosts_need_foe_pokemon_left: generation >= Generation::Gen6,
                raise_stops_at_cap: false,
                slowing_items_ignore_suppression: generation == Generation::Gen4,
                unaware_needs_hp: generation >= Generation::Gen5,
            }
        };

        let residual = ResidualRules {
            order: match generation {
                // Gen 1 has no end-of-turn residual at all.
                Generation::Gen1 => &[],
                Generation::Gen2 => GEN2_RESIDUAL_ORDER,
                Generation::Gen3 => GEN3_RESIDUAL_ORDER,
                Generation::Gen4 => GEN4_RESIDUAL_ORDER,
                _ => MODERN_RESIDUAL_ORDER,
            },
            durations: match generation {
                Generation::Gen1 => None,
                Generation::Gen2 => Some(GEN2_DURATION_ORDER),
                // Gen 3's side protections count down first; it has no field condition.
                Generation::Gen3 => Some(&[]),
                Generation::Gen4 => Some(GEN4_DURATION_ORDER),
                _ => Some(MODERN_DURATION_ORDER),
            },
        };
        let weather = WeatherRules {
            ability_weather_permanent: generation <= Generation::Gen5,
            same_weather_move: if generation >= Generation::Gen3 {
                SameWeatherMove::Fails
            } else {
                SameWeatherMove::OnlySandstormFails
            },
            // Gen 2's sandstorm takes 1/8 (Showdown gen2 `onWeather`).
            damage_divisor: if generation <= Generation::Gen2 {
                8
            } else {
                16
            },
            sandstorm_immune_types: &[PokemonType::Rock, PokemonType::Ground, PokemonType::Steel],
            hail_damages: generation <= Generation::Gen8,
            hail_immune_types: &[PokemonType::Ice],
            sandstorm_rock_special_defense: if generation >= Generation::Gen4 {
                Fraction::new(3, 2)
            } else {
                Fraction::ONE
            },
            snow_ice_defense: if generation >= Generation::Gen9 {
                Fraction::new(3, 2)
            } else {
                Fraction::ONE
            },
            rock_turns: 8,
            flower_gift: FlowerGiftRules {
                holder_must_be_cherrim: generation >= Generation::Gen5,
                once_per_side: generation <= Generation::Gen4,
                attack_ignores_mold_breaker: generation <= Generation::Gen4,
            },
        };
        let faint = FaintRules {
            last_to_faint_wins_double_ko: generation >= Generation::Gen5,
            replace_after_each_move: generation <= Generation::Gen3,
            singles_faint_effect: match generation {
                Generation::Gen1 => FaintTurnEffect::SkipTurn,
                Generation::Gen2 => FaintTurnEffect::CancelMoves,
                Generation::Gen3 => FaintTurnEffect::CancelActions,
                _ => FaintTurnEffect::None,
            },
            no_progress_ties: generation == Generation::Gen1,
        };
        let update = crate::rules::UpdateRules::for_generation(generation);
        // Gen 1-2 values come from Showdown's mods; no oracle scenario covers them yet.
        let two_turn = match generation {
            Generation::Gen1 => GEN1_TWO_TURN,
            Generation::Gen2 => GEN2_TWO_TURN,
            Generation::Gen3 => GEN3_TWO_TURN,
            Generation::Gen4 => GEN4_TWO_TURN,
            _ => MODERN_TWO_TURN,
        };

        match generation {
            Generation::Gen1 => Self {
                damage: DamageRules {
                    algorithm: DamageAlgorithm::Gen1Legacy,
                    modifier_order: GEN1_DAMAGE_ORDER,
                    rounding: DamageRoundingPolicy::FloorEachStep,
                    critical_ignores_negative_offense: false,
                    critical_ignores_positive_defense: false,
                    critical_multiplier: Fraction::new(2, 1),
                    critical: CriticalTable::Gen1BaseSpeed,
                    legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
                    stab_modifier: Fraction::new(3, 2),
                    burn_modifier: Fraction::new(1, 2),
                    spread_modifier: Fraction::ONE,
                    parental_bond_second_hit: Fraction::new(1, 2),
                    spread_only_foes: generation == Generation::Gen3,
                    random_min: 217,
                    random_max: 255,
                    random_denominator: 255,
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
                },
                stats,
                status: StatusRules {
                    paralysis_speed_modifier: Fraction::new(1, 4),
                    paralysis_speed_stage: ParalysisSpeedStage::Stat,
                    burn_residual_divisor: 16,
                    poison_residual_divisor: 16,
                    toxic_residual_divisor: 16,
                    fire_type_burn_immunity: true,
                    electric_type_paralysis_immunity: false,
                    poison_type_poison_immunity: true,
                    steel_type_poison_immunity: false,
                    ice_type_freeze_immunity: true,
                    electric_terrain_prevents_sleep: false,
                    misty_terrain_prevents_status: false,
                    residual_timing: if generation <= Generation::Gen2 {
                        StatusResidualTiming::AfterOwnAction
                    } else {
                        StatusResidualTiming::EndOfTurn
                    },
                    toxic_model: if generation <= Generation::Gen2 {
                        ToxicModel::ResidualVolatile
                    } else {
                        ToxicModel::Stage
                    },
                    lifecycle,
                },
                conditions,
                hits,
                residual,
                faint,
                update,
                two_turn,
                weather,
                action_order,
                capabilities: GenerationCapabilities {
                    mega: false,
                    z_move: false,
                    dynamax: false,
                    tera: false,
                },
            },
            Generation::Gen2 => Self {
                damage: DamageRules {
                    algorithm: DamageAlgorithm::Gen2,
                    modifier_order: GEN2_DAMAGE_ORDER,
                    rounding: DamageRoundingPolicy::FloorEachStep,
                    critical_ignores_negative_offense: false,
                    critical_ignores_positive_defense: false,
                    critical_multiplier: Fraction::new(2, 1),
                    critical: CriticalTable::OutOf256(&[0, 17, 32, 64, 85, 128]),
                    legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
                    stab_modifier: Fraction::new(3, 2),
                    burn_modifier: Fraction::new(1, 2),
                    spread_modifier: Fraction::ONE,
                    parental_bond_second_hit: Fraction::new(1, 2),
                    spread_only_foes: generation == Generation::Gen3,
                    random_min: 217,
                    random_max: 255,
                    random_denominator: 255,
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
                },
                stats,
                status: StatusRules {
                    paralysis_speed_modifier: Fraction::new(1, 4),
                    paralysis_speed_stage: ParalysisSpeedStage::Stat,
                    burn_residual_divisor: 8,
                    poison_residual_divisor: 8,
                    toxic_residual_divisor: 16,
                    // The gen2 typechart gives Fire no `brn`, Ice no `frz` and Steel no `psn` /
                    // `tox` immunity: a Fire / Ice move's own burn / freeze skips a target of its
                    // type (`secondary_same_type_immunity`), but Tri Attack burns a Fire type and
                    // freezes an Ice type, and Twineedle poisons a Steel type (Poison moves cannot
                    // touch it).
                    fire_type_burn_immunity: false,
                    electric_type_paralysis_immunity: false,
                    poison_type_poison_immunity: true,
                    steel_type_poison_immunity: false,
                    ice_type_freeze_immunity: false,
                    electric_terrain_prevents_sleep: false,
                    misty_terrain_prevents_status: false,
                    residual_timing: if generation <= Generation::Gen2 {
                        StatusResidualTiming::AfterOwnAction
                    } else {
                        StatusResidualTiming::EndOfTurn
                    },
                    toxic_model: if generation <= Generation::Gen2 {
                        ToxicModel::ResidualVolatile
                    } else {
                        ToxicModel::Stage
                    },
                    lifecycle,
                },
                conditions,
                hits,
                residual,
                faint,
                update,
                two_turn,
                weather,
                action_order,
                capabilities: GenerationCapabilities {
                    mega: false,
                    z_move: false,
                    dynamax: false,
                    tera: false,
                },
            },
            Generation::Gen3 => Self {
                damage: DamageRules {
                    algorithm: DamageAlgorithm::Modern,
                    modifier_order: GEN3_DAMAGE_ORDER,
                    rounding: DamageRoundingPolicy::FloorEachStep,
                    critical_ignores_negative_offense: true,
                    critical_ignores_positive_defense: true,
                    critical_multiplier: Fraction::new(2, 1),
                    critical: CriticalTable::OneIn(&[0, 16, 8, 4, 3, 2]),
                    legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
                    stab_modifier: Fraction::new(3, 2),
                    burn_modifier: Fraction::new(1, 2),
                    spread_modifier: Fraction::new(1, 2),
                    parental_bond_second_hit: Fraction::new(1, 2),
                    spread_only_foes: generation == Generation::Gen3,
                    random_min: 85,
                    random_max: 100,
                    random_denominator: 100,
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
                },
                stats,
                status: StatusRules {
                    paralysis_speed_modifier: Fraction::new(1, 4),
                    paralysis_speed_stage: ParalysisSpeedStage::Chained,
                    burn_residual_divisor: 8,
                    poison_residual_divisor: 8,
                    toxic_residual_divisor: 16,
                    fire_type_burn_immunity: true,
                    electric_type_paralysis_immunity: false,
                    poison_type_poison_immunity: true,
                    steel_type_poison_immunity: true,
                    ice_type_freeze_immunity: true,
                    electric_terrain_prevents_sleep: false,
                    misty_terrain_prevents_status: false,
                    residual_timing: if generation <= Generation::Gen2 {
                        StatusResidualTiming::AfterOwnAction
                    } else {
                        StatusResidualTiming::EndOfTurn
                    },
                    toxic_model: if generation <= Generation::Gen2 {
                        ToxicModel::ResidualVolatile
                    } else {
                        ToxicModel::Stage
                    },
                    lifecycle,
                },
                conditions,
                hits,
                residual,
                faint,
                update,
                two_turn,
                weather,
                action_order,
                capabilities: GenerationCapabilities {
                    mega: false,
                    z_move: false,
                    dynamax: false,
                    tera: false,
                },
            },
            Generation::Gen4 | Generation::Gen5 => Self {
                damage: DamageRules {
                    algorithm: if generation == Generation::Gen4 {
                        DamageAlgorithm::Gen4(&crate::rules::cartridge_damage::GEN4_CARTRIDGE)
                    } else {
                        DamageAlgorithm::Modern
                    },
                    modifier_order: if generation == Generation::Gen4 {
                        GEN4_DAMAGE_ORDER
                    } else {
                        GEN5_DAMAGE_ORDER
                    },
                    rounding: DamageRoundingPolicy::FloorEachStep,
                    critical_ignores_negative_offense: true,
                    critical_ignores_positive_defense: true,
                    critical_multiplier: Fraction::new(2, 1),
                    critical: CriticalTable::OneIn(&[0, 16, 8, 4, 3, 2]),
                    legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
                    stab_modifier: Fraction::new(3, 2),
                    burn_modifier: Fraction::new(1, 2),
                    spread_modifier: Fraction::new(3, 4),
                    parental_bond_second_hit: Fraction::new(1, 2),
                    spread_only_foes: generation == Generation::Gen3,
                    random_min: 85,
                    random_max: 100,
                    random_denominator: 100,
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
                },
                stats,
                status: StatusRules {
                    paralysis_speed_modifier: Fraction::new(1, 4),
                    paralysis_speed_stage: if generation == Generation::Gen4 {
                        ParalysisSpeedStage::Chained
                    } else {
                        ParalysisSpeedStage::AfterChain
                    },
                    burn_residual_divisor: 8,
                    poison_residual_divisor: 8,
                    toxic_residual_divisor: 16,
                    fire_type_burn_immunity: true,
                    electric_type_paralysis_immunity: false,
                    poison_type_poison_immunity: true,
                    steel_type_poison_immunity: true,
                    ice_type_freeze_immunity: true,
                    electric_terrain_prevents_sleep: false,
                    misty_terrain_prevents_status: false,
                    residual_timing: if generation <= Generation::Gen2 {
                        StatusResidualTiming::AfterOwnAction
                    } else {
                        StatusResidualTiming::EndOfTurn
                    },
                    toxic_model: if generation <= Generation::Gen2 {
                        ToxicModel::ResidualVolatile
                    } else {
                        ToxicModel::Stage
                    },
                    lifecycle,
                },
                conditions,
                hits,
                residual,
                faint,
                update,
                two_turn,
                weather,
                action_order,
                capabilities: GenerationCapabilities {
                    mega: false,
                    z_move: false,
                    dynamax: false,
                    tera: false,
                },
            },
            Generation::Gen6 => Self {
                damage: DamageRules {
                    algorithm: DamageAlgorithm::Modern,
                    modifier_order: MODERN_DAMAGE_ORDER,
                    rounding: DamageRoundingPolicy::FloorEachStep,
                    critical_ignores_negative_offense: true,
                    critical_ignores_positive_defense: true,
                    critical_multiplier: Fraction::new(3, 2),
                    critical: CriticalTable::OneIn(&[0, 16, 8, 2, 1]),
                    legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
                    stab_modifier: Fraction::new(3, 2),
                    burn_modifier: Fraction::new(1, 2),
                    spread_modifier: Fraction::new(3, 4),
                    parental_bond_second_hit: Fraction::new(1, 2),
                    spread_only_foes: generation == Generation::Gen3,
                    random_min: 85,
                    random_max: 100,
                    random_denominator: 100,
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
                },
                stats,
                status: StatusRules {
                    paralysis_speed_modifier: Fraction::new(1, 4),
                    paralysis_speed_stage: ParalysisSpeedStage::AfterChain,
                    burn_residual_divisor: 8,
                    poison_residual_divisor: 8,
                    toxic_residual_divisor: 16,
                    fire_type_burn_immunity: true,
                    electric_type_paralysis_immunity: true,
                    poison_type_poison_immunity: true,
                    steel_type_poison_immunity: true,
                    ice_type_freeze_immunity: true,
                    electric_terrain_prevents_sleep: true,
                    misty_terrain_prevents_status: true,
                    residual_timing: if generation <= Generation::Gen2 {
                        StatusResidualTiming::AfterOwnAction
                    } else {
                        StatusResidualTiming::EndOfTurn
                    },
                    toxic_model: if generation <= Generation::Gen2 {
                        ToxicModel::ResidualVolatile
                    } else {
                        ToxicModel::Stage
                    },
                    lifecycle,
                },
                conditions,
                hits,
                residual,
                faint,
                update,
                two_turn,
                weather,
                action_order,
                capabilities: GenerationCapabilities {
                    mega: true,
                    z_move: false,
                    dynamax: false,
                    tera: false,
                },
            },
            Generation::Gen7 => Self {
                damage: DamageRules {
                    algorithm: DamageAlgorithm::Modern,
                    modifier_order: MODERN_DAMAGE_ORDER,
                    rounding: DamageRoundingPolicy::FloorEachStep,
                    critical_ignores_negative_offense: true,
                    critical_ignores_positive_defense: true,
                    critical_multiplier: Fraction::new(3, 2),
                    critical: CriticalTable::OneIn(&[0, 24, 8, 2, 1]),
                    legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
                    stab_modifier: Fraction::new(3, 2),
                    burn_modifier: Fraction::new(1, 2),
                    spread_modifier: Fraction::new(3, 4),
                    parental_bond_second_hit: Fraction::new(1, 4),
                    spread_only_foes: generation == Generation::Gen3,
                    random_min: 85,
                    random_max: 100,
                    random_denominator: 100,
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
                },
                stats,
                status: StatusRules {
                    paralysis_speed_modifier: Fraction::new(1, 2),
                    paralysis_speed_stage: ParalysisSpeedStage::AfterChain,
                    burn_residual_divisor: 16,
                    poison_residual_divisor: 8,
                    toxic_residual_divisor: 16,
                    fire_type_burn_immunity: true,
                    electric_type_paralysis_immunity: true,
                    poison_type_poison_immunity: true,
                    steel_type_poison_immunity: true,
                    ice_type_freeze_immunity: true,
                    electric_terrain_prevents_sleep: true,
                    misty_terrain_prevents_status: true,
                    residual_timing: if generation <= Generation::Gen2 {
                        StatusResidualTiming::AfterOwnAction
                    } else {
                        StatusResidualTiming::EndOfTurn
                    },
                    toxic_model: if generation <= Generation::Gen2 {
                        ToxicModel::ResidualVolatile
                    } else {
                        ToxicModel::Stage
                    },
                    lifecycle,
                },
                conditions,
                hits,
                residual,
                faint,
                update,
                two_turn,
                weather,
                action_order,
                capabilities: GenerationCapabilities {
                    mega: true,
                    z_move: true,
                    dynamax: false,
                    tera: false,
                },
            },
            Generation::Gen8 => Self {
                damage: DamageRules {
                    algorithm: DamageAlgorithm::Modern,
                    modifier_order: MODERN_DAMAGE_ORDER,
                    rounding: DamageRoundingPolicy::FloorEachStep,
                    critical_ignores_negative_offense: true,
                    critical_ignores_positive_defense: true,
                    critical_multiplier: Fraction::new(3, 2),
                    critical: CriticalTable::OneIn(&[0, 24, 8, 2, 1]),
                    legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
                    stab_modifier: Fraction::new(3, 2),
                    burn_modifier: Fraction::new(1, 2),
                    spread_modifier: Fraction::new(3, 4),
                    parental_bond_second_hit: Fraction::new(1, 4),
                    spread_only_foes: generation == Generation::Gen3,
                    random_min: 85,
                    random_max: 100,
                    random_denominator: 100,
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
                },
                stats,
                status: StatusRules {
                    paralysis_speed_modifier: Fraction::new(1, 2),
                    paralysis_speed_stage: ParalysisSpeedStage::AfterChain,
                    burn_residual_divisor: 16,
                    poison_residual_divisor: 8,
                    toxic_residual_divisor: 16,
                    fire_type_burn_immunity: true,
                    electric_type_paralysis_immunity: true,
                    poison_type_poison_immunity: true,
                    steel_type_poison_immunity: true,
                    ice_type_freeze_immunity: true,
                    electric_terrain_prevents_sleep: true,
                    misty_terrain_prevents_status: true,
                    residual_timing: if generation <= Generation::Gen2 {
                        StatusResidualTiming::AfterOwnAction
                    } else {
                        StatusResidualTiming::EndOfTurn
                    },
                    toxic_model: if generation <= Generation::Gen2 {
                        ToxicModel::ResidualVolatile
                    } else {
                        ToxicModel::Stage
                    },
                    lifecycle,
                },
                conditions,
                hits,
                residual,
                faint,
                update,
                two_turn,
                weather,
                action_order,
                capabilities: GenerationCapabilities {
                    mega: false,
                    z_move: false,
                    dynamax: true,
                    tera: false,
                },
            },
            Generation::Gen9 => Self {
                damage: DamageRules {
                    algorithm: DamageAlgorithm::Modern,
                    modifier_order: MODERN_DAMAGE_ORDER,
                    rounding: DamageRoundingPolicy::FloorEachStep,
                    critical_ignores_negative_offense: true,
                    critical_ignores_positive_defense: true,
                    critical_multiplier: Fraction::new(3, 2),
                    critical: CriticalTable::OneIn(&[0, 24, 8, 2, 1]),
                    legacy_clamp_min: if generation == Generation::Gen1 { 0 } else { 1 },
                    stab_modifier: Fraction::new(3, 2),
                    burn_modifier: Fraction::new(1, 2),
                    spread_modifier: Fraction::new(3, 4),
                    parental_bond_second_hit: Fraction::new(1, 4),
                    spread_only_foes: generation == Generation::Gen3,
                    random_min: 85,
                    random_max: 100,
                    random_denominator: 100,
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
                },
                stats,
                status: StatusRules {
                    paralysis_speed_modifier: Fraction::new(1, 2),
                    paralysis_speed_stage: ParalysisSpeedStage::AfterChain,
                    burn_residual_divisor: 16,
                    poison_residual_divisor: 8,
                    toxic_residual_divisor: 16,
                    fire_type_burn_immunity: true,
                    electric_type_paralysis_immunity: true,
                    poison_type_poison_immunity: true,
                    steel_type_poison_immunity: true,
                    ice_type_freeze_immunity: true,
                    electric_terrain_prevents_sleep: true,
                    misty_terrain_prevents_status: true,
                    residual_timing: if generation <= Generation::Gen2 {
                        StatusResidualTiming::AfterOwnAction
                    } else {
                        StatusResidualTiming::EndOfTurn
                    },
                    toxic_model: if generation <= Generation::Gen2 {
                        ToxicModel::ResidualVolatile
                    } else {
                        ToxicModel::Stage
                    },
                    lifecycle,
                },
                conditions,
                hits,
                residual,
                faint,
                update,
                two_turn,
                weather,
                action_order,
                capabilities: GenerationCapabilities {
                    mega: false,
                    z_move: false,
                    dynamax: false,
                    tera: true,
                },
            },
        }
    }
}

/// Which volatiles the observers see start and go (DESIGN.md §47.0 item 13, R-M10-26): Showdown's
/// lines (`-start`, `-singleturn`, `-singlemove`, `-mustrecharge` … and their ends), as
/// @pkmn/client reads them.
fn volatile_visibility(generation: Generation) -> crate::state::VolatileVisibility {
    use crate::state::{VolatileConditionKind as V, VolatileSet};
    // The battle's own counters, marks and locks, and what shows itself otherwise (a forme, an
    // ability's line, a `-prepare`, Lock-On's and Sky Drop's `-activate`).
    let mut unseen: VolatileSet = [
        V::Flinch,
        V::ChoiceLock,
        V::TwoTurnMove,
        V::Stall,
        V::AllySwitch,
        V::SubstituteBroken,
        V::ResidualDamage,
        V::SureHit,
        V::MeFirst,
        V::Unburden,
        V::DefenseCurl,
        V::GastroAcid,
        V::MicleBerry,
        V::Fling,
        V::Minimize,
        V::ZenMode,
        V::Commanding,
        V::Commanded,
        V::SparklingAria,
        V::Metronome,
        V::ChillyReception,
        V::Trapper,
        V::PartialTrappingLock,
        V::FakePartiallyTrapped,
        V::Counter,
        V::MirrorCoat,
        V::Pursuit,
        V::Rolling,
        V::RolloutStorage,
        V::Rampage,
        V::FuryCutter,
        V::Truant,
        V::Invulnerability,
        V::SkyDrop,
        V::SkyDropped,
        V::LockOn,
        V::LockedOn,
    ]
    .into_iter()
    .collect();
    // Gen 1 starts Rage and a partial trap with no line of their own.
    if generation == Generation::Gen1 {
        unseen.insert(V::Rage);
        unseen.insert(V::PartiallyTrapped);
    }
    // `-singlemove`'s and the recharge's: no `-end`, gone as the Pokémon next acts. (Charge's
    // `-start` lasts until an Electric move, its `-end … [silent]` shows.)
    let single_move = [
        V::MustRecharge,
        V::DestinyBond,
        V::Grudge,
        V::Rage,
        V::GlaiveRush,
    ];
    // (Focus Punch's focus, `-singleturn`, goes with the turn, as its user uses the move, or as
    // it loses its focus — not as it acts otherwise: a sleeping user keeps it, R-M10-43.)
    let until_it_acts: VolatileSet = single_move.into_iter().collect();
    // `trapped` has no `onEnd`: its trapper leaving frees it without a word.
    let mut ending_unseen: VolatileSet = single_move.into_iter().collect();
    ending_unseen.insert(V::Trapped);
    crate::state::VolatileVisibility {
        unseen,
        ending_unseen,
        until_it_acts,
        // `-singleturn`'s: some Showdown removes before the turn ends with no word (Beak Blast's
        // after the move, Snatch's as it steals).
        for_the_turn: [
            V::Protect,
            V::Endure,
            V::FollowMe,
            V::RagePowder,
            V::HelpingHand,
            V::MagicCoat,
            V::Snatch,
            V::Roost,
            V::Powder,
            V::Electrify,
            V::Spotlight,
            V::FocusPunch,
            V::BeakBlast,
            V::ShellTrap,
        ]
        .into_iter()
        .collect(),
    }
}
