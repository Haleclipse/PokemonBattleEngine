//! Moves that strike back at the damage their user took (DESIGN.md §15.8, §15.10): Counter,
//! Mirror Coat, Metal Burst, Comeuppance, Bide, as a generation has them.

use crate::ids::MoveId;
use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetaliationRules {
    /// Gen 2 Counter / Mirror Coat (`getLastAttackedBy`): only the very last attack the user
    /// took this turn counts, of the right category, and it must be its attacker's last move;
    /// from Gen 3 the last hit of the right category from a foe (the `counter` volatile's
    /// `onDamage` / `onDamagingHit`).
    pub last_attack_only: bool,
    /// Gen 2 (pokecrystal `BattleCommand_Counter` / `BattleCommand_MirrorCoat`, issues.md I40):
    /// the move fails when the attack it would answer was itself a Counter or Mirror Coat
    /// (`EFFECT_COUNTER` / `EFFECT_MIRROR_COAT`), or dealt no damage (`wCurDamage` 0).
    /// Showdown's gen2 mod counts a failed move as a 0-damage attack and returns 0 — a hit for
    /// nothing; the oracle is patched (TESTING.md §7).
    pub fails_on_retaliation_or_no_damage: bool,
    /// Gen 1 Counter (the gen1 mod's `damageCallback`): twice the last damage dealt in the
    /// battle (`battle.lastDamage`, `BattleState::last_damage`), if the foe's side last used
    /// and last selected a counterable move (a damaging Normal or Fighting move, not Counter).
    pub gen1_last_damage: bool,
    /// Gen 1 (`SKIP_LASTDAMAGE`): the moves whose use does not reset the last damage.
    pub keeps_last_damage: &'static [MoveId],
    /// Bide (DESIGN.md §15.10).
    pub bide: BideRules,
}

/// Bide as a generation has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BideRules {
    /// Gen 1 (the gen1 mod's condition): at each of the holder's Bide turns the battle's last
    /// damage is added and `time` (2-3, drawn as it starts) counts down; at 0 twice the total
    /// strikes a random foe, whatever its type. No duration, no `onMoveAborted`, and a faint
    /// anywhere wipes what was stored. Gen 2+ store the damage moves deal the holder
    /// (`onDamage`) and release it on the turn their `duration` reads 1, at the Pokémon that
    /// dealt the last of it.
    pub gen1: bool,
    /// The `duration` (Gen 2's `durationCallback`: 3 or 4), or Gen 1's `time`: the least and
    /// the most.
    pub turns: (u8, u8),
    /// Gen 3+ (`onLockMove`): the holder cannot switch; Gen 1-2's `onSemiLockMove` only keeps
    /// its other moves off the request.
    pub traps: bool,
    /// Gen 2-3 (their `tryMoveHit`): the release goes through the hit steps — a semi-invulnerable
    /// target, the type chart, Protect, accuracy (100). Gen 4+ (`tryMoveHit`: `TryHitSide`, then
    /// `moveHit`) and Gen 1 (`moveHit`) meet none of them.
    pub release_hit_steps: bool,
    /// The release is an attack its target records (`attackedBy`: Counter, Mirror Coat, Metal
    /// Burst answer it). Gen 2-3: their `tryMoveHit`'s `gotAttacked`. Gen 4 on the cartridge:
    /// pokeplatinum's release goes on to `BattleControllerPlayer_UpdateHP`, which records Bide's
    /// physical damage, where Showdown's gen4 release (`tryMoveHit` → `moveHit`) records none
    /// (issues.md I179). Gen 1 (`moveHit`) and Gen 5+ (Showdown, `moveHit`) record none (DESIGN.md
    /// R-M8-90).
    pub release_records_attack: bool,
    /// The release checks the type chart (Gen 2-3 in the hit steps, Gen 5+ in `getDamage`); Gen
    /// 4's release data ignores it (`ignoreImmunity`), and Gen 1's has no type.
    pub release_type_immunity: bool,
    /// Gen 4 (`onAfterSetStatus`): falling asleep or frozen ends it.
    pub ends_on_sleep_or_freeze: bool,
    /// Gen 3 on the cartridge (pokeemerald `CANCELER_BIDE`, in `AtkCanceler_UnableToUseMove`:
    /// `HITMARKER_OBEYS` is never set, so `MOVEEND_UPDATE_LAST_MOVES` writes
    /// `MOVE_UNAVAILABLE`): a storing turn leaves no last move, as the release does in Gen 3-4
    /// (`ConditionRules::blocked_move_clears_last_move`; Showdown's storing turn is the move's
    /// `beforeMoveCallback`, which keeps it; issues.md I114).
    pub storing_clears_last_move: bool,
}

/// The moves Gen 2 leaves out of its last move (Showdown `moveUsed`: Metronome, Mimic, Mirror
/// Move, Sketch, Sleep Talk, Transform; issues.md I115).
pub const GEN2_NO_LAST_MOVE: &[crate::ids::MoveId] = &[
    crate::ids::MoveId::METRONOME,
    crate::ids::MoveId::MIMIC,
    crate::ids::MoveId::MIRROR_MOVE,
    crate::ids::MoveId::SKETCH,
    crate::ids::MoveId::SLEEP_TALK,
    crate::ids::MoveId::TRANSFORM,
];

/// The gen1 scripts' `SKIP_LASTDAMAGE`.
const GEN1_KEEPS_LAST_DAMAGE: &[MoveId] = &[
    MoveId::CONFUSE_RAY,
    MoveId::CONVERSION,
    MoveId::COUNTER,
    MoveId::FOCUS_ENERGY,
    MoveId::GLARE,
    MoveId::HAZE,
    MoveId::LEECH_SEED,
    MoveId::LIGHT_SCREEN,
    MoveId::MIMIC,
    MoveId::MIST,
    MoveId::POISON_GAS,
    MoveId::POISON_POWDER,
    MoveId::RECOVER,
    MoveId::REFLECT,
    MoveId::REST,
    MoveId::SOFT_BOILED,
    MoveId::SPLASH,
    MoveId::STUN_SPORE,
    MoveId::SUBSTITUTE,
    MoveId::SUPERSONIC,
    MoveId::TELEPORT,
    MoveId::THUNDER_WAVE,
    MoveId::TOXIC,
    MoveId::TRANSFORM,
];

impl RetaliationRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        let gen1 = matches!(generation, Generation::Gen1);
        Self {
            last_attack_only: matches!(generation, Generation::Gen2),
            fails_on_retaliation_or_no_damage: matches!(generation, Generation::Gen2),
            gen1_last_damage: gen1,
            keeps_last_damage: if gen1 { GEN1_KEEPS_LAST_DAMAGE } else { &[] },
            bide: BideRules {
                gen1,
                turns: match generation {
                    Generation::Gen1 => (2, 3),
                    Generation::Gen2 => (3, 4),
                    _ => (3, 3),
                },
                traps: !matches!(generation, Generation::Gen1 | Generation::Gen2),
                release_hit_steps: matches!(generation, Generation::Gen2 | Generation::Gen3),
                release_records_attack: matches!(
                    generation,
                    Generation::Gen2 | Generation::Gen3 | Generation::Gen4
                ),
                release_type_immunity: !matches!(generation, Generation::Gen1 | Generation::Gen4),
                ends_on_sleep_or_freeze: matches!(generation, Generation::Gen4),
                storing_clears_last_move: matches!(generation, Generation::Gen3),
            },
        }
    }
}
