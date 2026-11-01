//! Moves that send their user out as a generation has them (DESIGN.md §30.6, §30.7), and
//! Pursuit, which strikes a Pokémon leaving (§15.11).

use crate::spec::Generation;
use crate::state::VolatileConditionKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchingRules {
    /// Gen 2 Roar and Whirlwind drag the replacement in within the move (the gen2 `moveHit`
    /// calls `dragIn` at once; pokecrystal's `BattleCommand_ForceSwitch` too), before the
    /// user's after-own-action residual: its Leech Seed heals the Pokémon dragged in
    /// (R-M8-115). Gen 3+ drag at the action's end (`forceSwitchFlag`).
    pub drag_within_the_move: bool,
    /// Gen 8+ Teleport switches its user out (`onTry`: only with someone to come in); Gen 1-7
    /// (the gen7 mod: `onTry: false`) it always fails.
    pub teleport_switches: bool,
    /// Gen 7+ Parting Shot sends its user out only when it lowered a stat (or met Mirror Armor);
    /// the gen6 mod always does.
    pub parting_shot_needs_a_drop: bool,
    /// The volatiles Baton Pass does not hand on (Showdown `noCopy`); it hands on every other.
    pub not_passed: &'static [VolatileConditionKind],
    /// Gen 4+ Shadow Tag does not trap a foe with Shadow Tag; the gen3 mod's
    /// `onFoeTrapPokemon` traps every foe.
    pub shadow_tag_spares_its_like: bool,
    /// Gen 2 (Showdown `draggedIn`, the Gen 1-2 `runSwitch`): a Pokémon dragged in by Roar or
    /// Whirlwind skips its switch-in status damage (`AfterSwitchInSelf`) that turn
    /// (DESIGN.md §15.17).
    pub dragged_skips_switch_in_status: bool,
    /// Gen 1 (the gen1 mod's `partialtrappinglock`, DESIGN.md §30.9): a binding move's hit
    /// locks its user into repeating it and keeps its target from moving — not from
    /// switching; later generations bind for turns of residual damage and trap.
    pub binding_locks_user: bool,
    /// Pursuit striking a Pokémon as it switches out (DESIGN.md §15.11).
    pub pursuit: PursuitRules,
}

/// Pursuit as a generation has it (its `beforeTurnCallback` and its volatile's
/// `onBeforeSwitchOut`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PursuitRules {
    /// Gen 4+ (`pokemon.foes()`): the pursuer marks every foe as the turn begins; Gen 2-3 only
    /// the one it chose.
    pub marks_every_foe: bool,
    /// Gen 3-4: a sleeping or frozen pursuer marks nobody, nor does a Truant one about to loaf
    /// (pokeemerald's `truantCounter`, pokeplatinum's `Battler_CheckTruant`; Showdown gen4 asks
    /// for the `truant` volatile, §15.21).
    pub stopped_pursuer_marks_nobody: bool,
    /// Gen 3-4 (`useMove`): the intercepting Pursuit skips BeforeMove; Gen 2 and 5+ run the whole
    /// move (`runMove`).
    pub skips_before_move: bool,
    /// Gen 5+: only a pursuer adjacent to the switching Pokémon strikes.
    pub adjacent_only: bool,
    /// What happens to the switching Pokémon's Destiny Bond.
    pub destiny_bond: PursuitDestinyBond,
    /// Gen 2-4 (`pursuitfaint`): a Pokémon Pursuit knocks out as it switches is still replaced by
    /// the Pokémon it chose, at once; Gen 5+ the switch is cancelled.
    pub switch_survives_faint: bool,
    /// Gen 5+ (`switchIn`): an `Update` follows a chosen switch's `BeforeSwitchOut`.
    pub update_after: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursuitDestinyBond {
    /// Gen 3-4: it stays, and a Pursuit knock-out takes the pursuer down with it.
    Stays,
    /// Gen 5+: it ends as the switch begins, pursued or not.
    Ends,
    /// Gen 2: it ends when the pursuer is slower (a tie: a coin).
    EndsWhenOutsped,
}

/// Gen 1's binding lock lasts 2, 2, 2, 3, 3, 3, 4 or 5 turns (`this.sample`).
pub const BINDING_LOCK_TURNS: [u16; 4] = [2, 3, 4, 5];
pub const BINDING_LOCK_WEIGHTS: [u16; 4] = [3, 3, 1, 1];
/// Gen 1's `partiallytrapped` (`duration: 2`, set again by every hit).
pub const GEN1_PARTIALLY_TRAPPED_TURNS: u8 = 2;

/// The volatiles Showdown marks `noCopy`, generation by generation (`tests/baton_pass_tables.rs`
/// checks every table against Showdown's own flags): in every generation that has them Encore,
/// Disable, a Choice lock, infatuation, Quark Drive / Protosynthesis, Yawn, Nightmare, Salt
/// Cure, Syrup Bomb, Destiny Bond, Spotlight, Defense Curl's mark, Stockpile (its stages go with
/// the others), Minimize's mark, Autotomize (Gen 6+ lowers `weighthg`, never handed on), Flash
/// Fire's boost; Gen 3-4's `substitutebroken` marker and Gen 6-7's `toxic` sure-hit (the gen4
/// conditions, the gen7 mod). Gen 2-4 hand Mean Look's links on (the gen4 conditions' `trapped`
/// / `trapper`: `noCopy: false`).
const NOT_PASSED_GEN2: &[VolatileConditionKind] = &[
    // Glaive Rush's drawback (`noCopy`, §15.20).
    VolatileConditionKind::GlaiveRush,
    // The cartridge's Fury Cutter count is the switching Pokémon's (§15.18), and so is its
    // rage (§15.19).
    VolatileConditionKind::FuryCutter,
    VolatileConditionKind::Rage,
    VolatileConditionKind::Encore,
    VolatileConditionKind::Disable,
    VolatileConditionKind::ChoiceLock,
    VolatileConditionKind::Attract,
    VolatileConditionKind::QuarkDrive,
    VolatileConditionKind::Protosynthesis,
    VolatileConditionKind::SubstituteBroken,
    VolatileConditionKind::SureHit,
    VolatileConditionKind::MagicCoat,
    VolatileConditionKind::Snatch,
    VolatileConditionKind::MeFirst,
    VolatileConditionKind::AllySwitch,
    VolatileConditionKind::Yawn,
    VolatileConditionKind::Nightmare,
    VolatileConditionKind::SaltCure,
    VolatileConditionKind::SyrupBomb,
    VolatileConditionKind::DestinyBond,
    VolatileConditionKind::Spotlight,
    VolatileConditionKind::DefenseCurl,
    VolatileConditionKind::Stockpile,
    VolatileConditionKind::Minimize,
    VolatileConditionKind::Autotomize,
    VolatileConditionKind::FlashFire,
    VolatileConditionKind::Torment,
    VolatileConditionKind::Imprison,
];
/// Gen 3+ Counter / Mirror Coat turn-start volatiles are `noCopy` (the gen2 mod's are not). Gen 4's
/// Micle Berry boost is not handed on either, as the cartridge has it (issues.md I42:
/// pokeplatinum `BattleSystem_UpdateAfterSwitch` clears `moveEffectsData` and keeps none of it
/// for Baton Pass; Showdown's `micleberry` is not `noCopy`, the oracle patch makes the gen4 one).
const NOT_PASSED_GEN3_4: &[VolatileConditionKind] = &[
    // Foresight / Miracle Eye (`noCopy`; Gen 2's Foresight is handed on, §41.5).
    VolatileConditionKind::Foresight,
    VolatileConditionKind::MiracleEye,
    // Smack Down's grounding (`noCopy`; Magnet Rise and Telekinesis are handed on).
    VolatileConditionKind::SmackDown,
    // Glaive Rush's drawback (`noCopy`, §15.20).
    VolatileConditionKind::GlaiveRush,
    // The cartridges' Fury Cutter count is the switching Pokémon's (§15.18), and so is its
    // rage (pokeplatinum's `VOLATILE_CONDITION_BATON_PASSED` leaves it out, §15.19).
    VolatileConditionKind::FuryCutter,
    VolatileConditionKind::Rage,
    VolatileConditionKind::MicleBerry,
    VolatileConditionKind::Encore,
    VolatileConditionKind::Disable,
    VolatileConditionKind::ChoiceLock,
    VolatileConditionKind::Attract,
    VolatileConditionKind::QuarkDrive,
    VolatileConditionKind::Protosynthesis,
    VolatileConditionKind::SubstituteBroken,
    VolatileConditionKind::SureHit,
    VolatileConditionKind::MagicCoat,
    VolatileConditionKind::Snatch,
    VolatileConditionKind::MeFirst,
    VolatileConditionKind::AllySwitch,
    VolatileConditionKind::Yawn,
    VolatileConditionKind::Nightmare,
    VolatileConditionKind::SaltCure,
    VolatileConditionKind::SyrupBomb,
    VolatileConditionKind::Counter,
    VolatileConditionKind::MirrorCoat,
    VolatileConditionKind::DestinyBond,
    VolatileConditionKind::Spotlight,
    VolatileConditionKind::DefenseCurl,
    VolatileConditionKind::Stockpile,
    VolatileConditionKind::Minimize,
    VolatileConditionKind::Autotomize,
    VolatileConditionKind::FlashFire,
    VolatileConditionKind::Torment,
    VolatileConditionKind::Imprison,
];

/// Gen 5's Mud Sport / Water Sport volatiles are `noCopy` too (Gen 3-4's, the gen4 mod's
/// `noCopy: false`, are handed on), and from Gen 5 Mean Look's links.
const NOT_PASSED_GEN5: &[VolatileConditionKind] = &[
    // Foresight / Miracle Eye (`noCopy`; Gen 2's Foresight is handed on, §41.5).
    VolatileConditionKind::Foresight,
    VolatileConditionKind::MiracleEye,
    // Smack Down's grounding (`noCopy`; Magnet Rise and Telekinesis are handed on).
    VolatileConditionKind::SmackDown,
    // Glaive Rush's drawback (`noCopy`, §15.20).
    VolatileConditionKind::GlaiveRush,
    VolatileConditionKind::Encore,
    VolatileConditionKind::Disable,
    VolatileConditionKind::ChoiceLock,
    VolatileConditionKind::Attract,
    VolatileConditionKind::QuarkDrive,
    VolatileConditionKind::Protosynthesis,
    VolatileConditionKind::SubstituteBroken,
    VolatileConditionKind::SureHit,
    VolatileConditionKind::MagicCoat,
    VolatileConditionKind::Snatch,
    VolatileConditionKind::MeFirst,
    VolatileConditionKind::AllySwitch,
    VolatileConditionKind::Yawn,
    VolatileConditionKind::Nightmare,
    VolatileConditionKind::SaltCure,
    VolatileConditionKind::SyrupBomb,
    VolatileConditionKind::MudSport,
    VolatileConditionKind::WaterSport,
    VolatileConditionKind::Trapped,
    VolatileConditionKind::Trapper,
    VolatileConditionKind::Counter,
    VolatileConditionKind::MirrorCoat,
    VolatileConditionKind::DestinyBond,
    VolatileConditionKind::Spotlight,
    VolatileConditionKind::DefenseCurl,
    VolatileConditionKind::Stockpile,
    VolatileConditionKind::Minimize,
    VolatileConditionKind::Autotomize,
    VolatileConditionKind::FlashFire,
    VolatileConditionKind::Torment,
    VolatileConditionKind::Imprison,
    VolatileConditionKind::LockOn,
];

const NOT_PASSED: &[VolatileConditionKind] = &[
    // Foresight / Miracle Eye (`noCopy`; Gen 2's Foresight is handed on, §41.5).
    VolatileConditionKind::Foresight,
    VolatileConditionKind::MiracleEye,
    // Smack Down's grounding (`noCopy`; Magnet Rise and Telekinesis are handed on).
    VolatileConditionKind::SmackDown,
    // Glaive Rush's drawback (`noCopy`, §15.20).
    VolatileConditionKind::GlaiveRush,
    VolatileConditionKind::Encore,
    VolatileConditionKind::Disable,
    VolatileConditionKind::ChoiceLock,
    VolatileConditionKind::Attract,
    VolatileConditionKind::QuarkDrive,
    VolatileConditionKind::Protosynthesis,
    VolatileConditionKind::SubstituteBroken,
    VolatileConditionKind::SureHit,
    VolatileConditionKind::MagicCoat,
    VolatileConditionKind::Snatch,
    VolatileConditionKind::MeFirst,
    VolatileConditionKind::AllySwitch,
    VolatileConditionKind::Yawn,
    VolatileConditionKind::Nightmare,
    VolatileConditionKind::SaltCure,
    VolatileConditionKind::SyrupBomb,
    VolatileConditionKind::Trapped,
    VolatileConditionKind::Trapper,
    VolatileConditionKind::Counter,
    VolatileConditionKind::MirrorCoat,
    VolatileConditionKind::DestinyBond,
    VolatileConditionKind::Spotlight,
    VolatileConditionKind::DefenseCurl,
    VolatileConditionKind::Stockpile,
    VolatileConditionKind::Minimize,
    VolatileConditionKind::Autotomize,
    VolatileConditionKind::FlashFire,
    VolatileConditionKind::Torment,
    VolatileConditionKind::Imprison,
    VolatileConditionKind::LockOn,
];

impl SwitchingRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        Self {
            drag_within_the_move: matches!(generation, Generation::Gen2),
            teleport_switches: matches!(generation, Generation::Gen8 | Generation::Gen9),
            parting_shot_needs_a_drop: !matches!(
                generation,
                Generation::Gen1
                    | Generation::Gen2
                    | Generation::Gen3
                    | Generation::Gen4
                    | Generation::Gen5
                    | Generation::Gen6
            ),
            shadow_tag_spares_its_like: !matches!(generation, Generation::Gen3),
            dragged_skips_switch_in_status: matches!(generation, Generation::Gen2),
            binding_locks_user: matches!(generation, Generation::Gen1),
            pursuit: {
                let modern = !matches!(
                    generation,
                    Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4
                );
                let gen3_4 = matches!(generation, Generation::Gen3 | Generation::Gen4);
                PursuitRules {
                    marks_every_foe: !matches!(
                        generation,
                        Generation::Gen1 | Generation::Gen2 | Generation::Gen3
                    ),
                    stopped_pursuer_marks_nobody: gen3_4,
                    skips_before_move: gen3_4,
                    adjacent_only: modern,
                    destiny_bond: if modern {
                        PursuitDestinyBond::Ends
                    } else if gen3_4 {
                        PursuitDestinyBond::Stays
                    } else {
                        PursuitDestinyBond::EndsWhenOutsped
                    },
                    switch_survives_faint: !modern,
                    update_after: modern,
                }
            },
            not_passed: match generation {
                Generation::Gen2 => NOT_PASSED_GEN2,
                Generation::Gen3 | Generation::Gen4 => NOT_PASSED_GEN3_4,
                Generation::Gen5 => NOT_PASSED_GEN5,
                _ => NOT_PASSED,
            },
        }
    }
}
