//! A generation's conditions, abilities and items in battle (`ConditionRules`).

use super::*;

mod abilities;
mod calls;
mod typing;
mod visibility;

pub use abilities::*;
pub use calls::*;
pub use typing::*;
use visibility::volatile_visibility;

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
    pub terrain: crate::rules::TerrainRules,
    /// Defog (DESIGN.md §14.2).
    pub defog: crate::rules::DefogRules,
    /// The pseudo-weathers (DESIGN.md §23.4).
    pub field: crate::rules::FieldRules,
    /// Knock Off, Thief / Covet, Sticky Hold (DESIGN.md §39.3).
    pub items: crate::rules::ItemTransferRules,
    /// A user fainting by its own move (DESIGN.md §30.4).
    pub self_faint: crate::rules::SelfFaintRules,
    /// Moves that send their user out (DESIGN.md §30.6).
    pub switching: crate::rules::SwitchingRules,
    /// Berry numbers by generation (DESIGN.md §40.3).
    pub berries: crate::rules::BerryRules,
    pub retaliation: crate::rules::RetaliationRules,
    /// Thrash and its kind (DESIGN.md §15.12).
    pub rampage: crate::rules::RampageRules,
    /// Uproar (DESIGN.md §15.12).
    pub uproar: crate::rules::UproarRules,
    /// Lock-On, Mind Reader (DESIGN.md §15.14).
    pub lock_on: crate::rules::LockOnRules,
    /// Spite, Eerie Spell (DESIGN.md §15.15).
    pub pp: crate::rules::PpRules,
    /// Curse (DESIGN.md §15.16).
    pub curse: crate::rules::CurseRules,
    /// Fury Cutter (DESIGN.md §15.18).
    pub fury_cutter: crate::rules::FuryCutterRules,
    /// Rage (DESIGN.md §15.19).
    pub rage: crate::rules::RageRules,
    /// Truant (DESIGN.md §15.21).
    pub truant: crate::rules::TruantRules,
    /// Future Sight and Doom Desire (DESIGN.md §15.22).
    pub future_attack: crate::rules::FutureAttackRules,
    /// Gen 6+ Sky Drop (DESIGN.md §15.23): a target of 200 kg or more is too heavy to carry up
    /// (`-fail … [heavy]`); the gen5 `onTryHit` has no such check.
    pub sky_drop_heavy_fails: bool,
    /// Destiny Bond, Grudge (DESIGN.md §15.9).
    pub faint_bonds: crate::rules::FaintBondRules,
    /// Foresight, Odor Sleuth, Miracle Eye (DESIGN.md §41.5).
    pub identify: crate::rules::IdentifyRules,
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
    pub screens: crate::rules::ScreenRules,
    /// The timed side conditions (the screens, Mist, Safeguard, Lucky Chant) and Mist.
    pub side_protections: crate::rules::SideProtectionRules,
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

/// How a typed sure hit (`ConditionRules::typed_sure_hit`) works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedSureHit {
    /// Gen 6-7 (the gen7 mod's Toxic `onPrepareHit`): the user takes the `toxic` volatile for
    /// the turn.
    Volatile,
    /// Gen 8+: the hit steps check the move and the user's type; nothing is left on the user.
    HitSteps,
}

/// The generation's conditions, abilities and items in battle.
pub(super) fn resolve(generation: Generation) -> ConditionRules {
    ConditionRules {
        // Gen 2 overrides `moveHit` with `Math.round`; Gen 1 and 3-4 floor.
        heal_rounds_half_up: generation == Generation::Gen2 || generation >= Generation::Gen5,
        heal_fails_255_short: generation == Generation::Gen1,
        stockpile: match generation {
            Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => StockpileRule::NoStages,
            Generation::Gen4 | Generation::Gen5 | Generation::Gen6 => StockpileRule::StagesByLayer,
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
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => StartNewAbility::Never,
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
            snatch_takes_a_snatched_move: matches!(generation, Generation::Gen3 | Generation::Gen4),
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
        blocked_move_clears_last_move: matches!(generation, Generation::Gen3 | Generation::Gen4),
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
        terrain: crate::rules::TerrainRules::for_generation(generation),
        defog: crate::rules::DefogRules::for_generation(generation),
        field: crate::rules::FieldRules::for_generation(generation),
        items: crate::rules::ItemTransferRules::for_generation(generation),
        self_faint: crate::rules::SelfFaintRules::for_generation(generation),
        switching: crate::rules::SwitchingRules::for_generation(generation),
        berries: crate::rules::BerryRules::for_generation(generation),
        retaliation: crate::rules::RetaliationRules::for_generation(generation),
        rampage: crate::rules::RampageRules::for_generation(generation),
        uproar: crate::rules::UproarRules::for_generation(generation),
        lock_on: crate::rules::LockOnRules::for_generation(generation),
        pp: crate::rules::PpRules::for_generation(generation),
        curse: crate::rules::CurseRules::for_generation(generation),
        fury_cutter: crate::rules::FuryCutterRules::for_generation(generation),
        rage: crate::rules::RageRules::for_generation(generation),
        truant: crate::rules::TruantRules::for_generation(generation),
        future_attack: crate::rules::FutureAttackRules::for_generation(generation),
        sky_drop_heavy_fails: generation >= Generation::Gen6,
        faint_bonds: crate::rules::FaintBondRules::for_generation(generation),
        identify: crate::rules::IdentifyRules::for_generation(generation),
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
        screens: crate::rules::ScreenRules::for_generation(generation),
        side_protections: crate::rules::SideProtectionRules::for_generation(generation),
    }
}
