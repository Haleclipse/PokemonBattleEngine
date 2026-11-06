//! A generation's two-turn moves and semi-invulnerability (`TwoTurnRules`).

use super::*;
use serde::{Deserialize, Serialize};

/// Where a charging move's user is while it is out of reach (Showdown `onInvulnerability`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Gen 2 (`onSourceBasePower`); Gen 3-4 on the cartridge, the move's own multiplier — Gen 4
    /// the power's (`BTLVAR_POWER_MULTI`), Gen 3 `gBattleScripting.dmgMultiplier`, which the
    /// profile moves after the critical hit (`CartridgeProfile::after_critical`).
    BasePower,
    /// Gen 5+ (`onSourceModifyDamage`, the last step of the damage formula).
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
    /// `onSourceBasePower`), Gen 3-4 on the cartridge (issues.md I17, I218).
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
/// `hitStepInvulnerabilityEvent`). On the cartridge the doubling is the move's own multiplier:
/// Gen 4's power multiplier (issues.md I17: `BTLVAR_POWER_MULTI` 20 in pokeplatinum's effect
/// scripts 0146 / 0149 for Gust / Twister, 0147 / 0126 for Earthquake / Magnitude, and for Surf /
/// Whirlpool against a diver), the first step of the power; Gen 3's `dmgMultiplier` (I218:
/// pokeemerald's `Cmd_jumpifsemiinvulnerable` scripts set `sDMG_MULTIPLIER` 2), after the
/// critical hit.
const GEN3_TWO_TURN: TwoTurnRules = TwoTurnRules {
    reach_everywhere: &[],
    bonus_stage_override: Some(DamageBonusStage::BasePower),
    ..MODERN_TWO_TURN
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

/// The generation's two-turn moves.
pub(super) fn resolve(generation: Generation) -> TwoTurnRules {
    // Gen 1-2 values come from Showdown's mods; no oracle scenario covers them yet.
    match generation {
        Generation::Gen1 => GEN1_TWO_TURN,
        Generation::Gen2 => GEN2_TWO_TURN,
        Generation::Gen3 | Generation::Gen4 => GEN3_TWO_TURN,
        _ => MODERN_TWO_TURN,
    }
}
