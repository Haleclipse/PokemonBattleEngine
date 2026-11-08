//! A generation's hit checks: accuracy, critical hits, multi-hits, recoil (`HitRules`).

use super::*;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoilTiming {
    /// Gen 1: recoil is taken after every hit from that hit's damage.
    PerHit,
    /// Gen 2+: one recoil from the total damage once every hit has landed.
    AfterAllHits,
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
    /// Gen 1 (pokered `AdjustDamageForMoveType`: "if damage is 0, make the move miss"): a damage
    /// the type chart takes to 0 — 2 or 3 at 0.25× — misses: no damage, drain, recoil or
    /// secondary, and a crash move crashes. Showdown's gen1 `getDamage` returns the 0 as a hit
    /// (issues.md I221, R-M11-107).
    pub zero_damage_misses: bool,
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
    pub item_flinch: crate::rules::ItemFlinchRules,
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

/// The generation's hit checks.
pub(super) fn resolve(generation: Generation) -> HitRules {
    HitRules {
        multi_hit_weights: if generation >= Generation::Gen5 {
            [35, 35, 15, 15]
        } else {
            [3, 3, 1, 1]
        },
        same_damage_each_hit: generation == Generation::Gen1,
        zero_damage_misses: generation == Generation::Gen1,
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
        item_flinch: crate::rules::ItemFlinchRules::for_generation(generation),
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
    }
}
