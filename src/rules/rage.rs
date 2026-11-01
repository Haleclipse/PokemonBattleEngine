//! Rage (DESIGN.md §15.19): how long a generation keeps its user raging and what a hit on a
//! raging Pokémon builds.

use crate::spec::Generation;
use crate::state::VolatileConditionKind;

/// What a hit on a raging Pokémon builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RageBuild {
    /// Gen 5+ (Showdown `rage.onHit`, in the `Hit` event): a damaging move that reaches the
    /// holder's body — an ally's too, a heal in place of the damage (Present, Pollen Puff) too —
    /// raises its Attack through Showdown's `boost`.
    Boost,
    /// Gen 3-4 on the cartridges (pokeemerald `MOVEEND_RAGE`, pokeplatinum
    /// `BattleControllerPlayer_RageBuilding`; issues.md I77): a hit that took the standing
    /// holder's HP — not its Substitute's — raises its Attack by one, after the move's
    /// secondaries and before the `DamagingHit` reactions; nothing happens at +6. `foes_only`:
    /// Gen 3 counts a foe's move only (`GetBattlerSide(attacker) != GetBattlerSide(target)`),
    /// Gen 4 any other Pokémon's.
    Stage { foes_only: bool },
    /// Gen 2 on the cartridge (pokecrystal `BattleCommand_BuildOpponentRage`,
    /// `BattleCommand_RageDamage`; I77): every hit of a damaging move that did not miss — a
    /// Substitute's too — counts, up to 255, and raises nothing; the holder's Rage deals its
    /// damage × (count + 1), after the type matchups and before the random factor, at most
    /// 65535.
    Counter,
    /// Gen 1 (Showdown gen1 `rage.onHit` and `useMove`; pokered `HandleBuildingRage`): a hit by
    /// a damaging move raises the holder's Attack through the Gen 1 stat-up (the modified
    /// stat recomputed; at 999 nothing), and so does Disable or a self-destructing move, hit or
    /// miss, once — the cartridge's move flow reaches `HandleBuildingRage` past them without a
    /// hit test; never against a holder the move knocked out.
    Gen1 {
        /// pokered: a hit on the holder's Substitute builds too (`HandleBuildingRage` follows
        /// `AttackSubstitute`; issues.md I79). Showdown's gen1 Substitute runs no `Hit` event.
        substitute_builds: bool,
        /// pokered: the build is the holder's own Attack-up effect (`StatModifierUpEffect`),
        /// which ends by cutting its foe's paralysed Speed / burned Attack again (I79).
        cuts_foe_status_stats: bool,
    },
}

/// When a Pokémon stops raging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RageEnd {
    /// Gen 5+ (Showdown `rage.onBeforeMove`, priority 100): at its next move attempt, Rage's
    /// too — a Rage that hits starts it again.
    BeforeMove,
    /// Gen 2-3 on the cartridges (pokecrystal `ParsePlayerAction` / `ParseEnemyAction`,
    /// pokeemerald `TryClearRageStatuses`; I77): as the turn starts, unless the Pokémon chose
    /// Rage — a switch ends it too. Gen 2 resets the count with it.
    TurnStart,
    /// Gen 4 on the cartridge (pokeplatinum `PRE_MOVE_ACTION_STATE_CHECK_RAGE_FLAG`,
    /// `AFTER_MOVE_HIT_STATE_RAGE`; issues.md I78): as the turn starts, unless the Pokémon chose
    /// Rage, the cartridge's bug (`statusVolatile &= VOLATILE_CONDITION_RAGE`) wipes the volatiles
    /// of `wiped` silently and leaves the flag; the flag ends once the Pokémon's own move other
    /// than Rage is over, if BeforeMove let it start.
    Gen4 {
        wiped: &'static [VolatileConditionKind],
    },
    /// Gen 1 (Showdown gen1 `rage`: `onLockMove`, no `onBeforeMove`; pokered `USING_RAGE`):
    /// never while the Pokémon stays in, and it locks the Pokémon into Rage — no other move, no
    /// switch, no PP spent — with an accuracy byte of its own, 255 as the rage starts, that each
    /// use's check rewrites with the stages applied again (the cartridge's
    /// `wPlayerMoveAccuracy` left as `CalcHitChance` wrote it).
    Locked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RageRules {
    pub build: RageBuild,
    pub end: RageEnd,
    /// Gen 3 on the cartridge (pokeemerald `BattleScript_EffectRage`: `accuracycheck
    /// BattleScript_RageMiss` then `seteffectprimary`; I77): the flag is the accuracy check's.
    /// A Rage that misses — or meets Protect, or a target out of reach — ends it; one that
    /// passes the check starts it, a type immunity (Ghost) or Wonder Guard after it
    /// notwithstanding, but not against a Substitute (`SetMoveEffect` skips a target behind
    /// one). Elsewhere a Rage that lands (Showdown `self`, a Substitute's hit too) starts it and
    /// one that does not leaves it be.
    pub accuracy_sets: bool,
}

/// pokeplatinum's `statusVolatile` bits a raging Pokémon can hold at a turn's start that the
/// engine models (issues.md I78): confusion, a binding move's hold, love, Focus Energy, a
/// Substitute, Mean Look's hold, Nightmare, a Ghost's Curse, Defense Curl, Torment. Not listed:
/// the flinch (none at a turn's start) and the holder's own multi-turn states — Uproar, Bide,
/// Thrash's lock and count, the move lock of a charge or Rollout, recharging — and Destiny Bond,
/// which only a move other than Rage starts, and that move ends the flag; Foresight and
/// Transform, not modelled yet (issues.md I78: to be added with them, K.10 for Transform).
const GEN4_WIPED: &[VolatileConditionKind] = &[
    VolatileConditionKind::Confusion,
    VolatileConditionKind::PartiallyTrapped,
    VolatileConditionKind::Attract,
    VolatileConditionKind::FocusEnergy,
    VolatileConditionKind::Substitute,
    VolatileConditionKind::Trapped,
    VolatileConditionKind::Nightmare,
    VolatileConditionKind::Curse,
    VolatileConditionKind::DefenseCurl,
    VolatileConditionKind::Torment,
];

impl RageRules {
    pub const fn for_generation(generation: Generation) -> Self {
        match generation {
            Generation::Gen1 => Self {
                build: RageBuild::Gen1 {
                    substitute_builds: true,
                    cuts_foe_status_stats: true,
                },
                end: RageEnd::Locked,
                accuracy_sets: false,
            },
            Generation::Gen2 => Self {
                build: RageBuild::Counter,
                end: RageEnd::TurnStart,
                accuracy_sets: false,
            },
            Generation::Gen3 => Self {
                build: RageBuild::Stage { foes_only: true },
                end: RageEnd::TurnStart,
                accuracy_sets: true,
            },
            Generation::Gen4 => Self {
                build: RageBuild::Stage { foes_only: false },
                end: RageEnd::Gen4 { wiped: GEN4_WIPED },
                accuracy_sets: false,
            },
            _ => Self {
                build: RageBuild::Boost,
                end: RageEnd::BeforeMove,
                accuracy_sets: false,
            },
        }
    }
}
