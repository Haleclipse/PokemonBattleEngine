//! A generation's major status conditions (`StatusRules`).

use super::*;

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

/// How the generation's sleep, freeze and paralysis come and go.
fn lifecycle(generation: Generation) -> StatusLifecycleRules {
    // Values follow Showdown's conditions data per generation (verified against the pinned
    // @pkmn/sim source when this table was written).
    match generation {
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
    }
}

/// The generation's major status conditions.
pub(super) fn resolve(generation: Generation) -> StatusRules {
    let lifecycle = lifecycle(generation);
    StatusRules {
        paralysis_speed_modifier: if generation <= Generation::Gen6 {
            Fraction::new(1, 4)
        } else {
            Fraction::new(1, 2)
        },
        paralysis_speed_stage: match generation {
            Generation::Gen1 | Generation::Gen2 => ParalysisSpeedStage::Stat,
            Generation::Gen3 | Generation::Gen4 => ParalysisSpeedStage::Chained,
            _ => ParalysisSpeedStage::AfterChain,
        },
        burn_residual_divisor: match generation {
            Generation::Gen1 | Generation::Gen7 | Generation::Gen8 | Generation::Gen9 => 16,
            _ => 8,
        },
        poison_residual_divisor: if generation == Generation::Gen1 {
            16
        } else {
            8
        },
        toxic_residual_divisor: 16,
        // The gen2 typechart gives Fire no `brn`, Ice no `frz` and Steel no `psn` / `tox` immunity: a
        // Fire / Ice move's own burn / freeze skips a target of its type
        // (`secondary_same_type_immunity`), but Tri Attack burns a Fire type and freezes an Ice type,
        // and Twineedle poisons a Steel type (Poison moves cannot touch it).
        fire_type_burn_immunity: generation != Generation::Gen2,
        electric_type_paralysis_immunity: generation >= Generation::Gen6,
        poison_type_poison_immunity: true,
        steel_type_poison_immunity: generation >= Generation::Gen3,
        ice_type_freeze_immunity: generation != Generation::Gen2,
        electric_terrain_prevents_sleep: generation >= Generation::Gen6,
        misty_terrain_prevents_status: generation >= Generation::Gen6,
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
    }
}
