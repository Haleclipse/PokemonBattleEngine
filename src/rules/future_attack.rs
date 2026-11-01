//! Future Sight and Doom Desire (DESIGN.md §15.22): when a generation computes the damage of a
//! foreseen attack, in what order the strikes go, and what the use returns.

use crate::spec::Generation;

/// Where the random factor of a damage computed at use is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeseenRoll {
    /// Gen 4 (pokeplatinum `BattleSystem_CalcDamageVariance` in `BtlCmd_TryFutureSight`;
    /// Showdown's `getDamage`): with the damage, at use.
    AtUse,
    /// Gen 3 on the cartridge (issues.md I110: pokeemerald's `adjustnormaldamage2` after the
    /// strike's `accuracycheck`): at the strike, once it hit.
    AfterAccuracy,
    /// Gen 2 on the cartridge (I110: pokecrystal's `damagevariation`, the command after
    /// `futuresight` the strike skips to, before `checkhit`): at the strike, before its
    /// accuracy check — a miss has drawn it too.
    BeforeAccuracy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FutureAttackRules {
    /// Gen 2-4 (Showdown's gen4 `onTry`; pokecrystal `damagecalc`, pokeemerald
    /// `CalculateBaseDamage`, pokeplatinum `BattleSystem_CalcMoveDamage`): the damage is computed
    /// as the move is used — typeless, never a critical hit — and the strike deals that amount,
    /// its random factor drawn where `ForeseenRoll` says. Gen 5+ (`None`): the strike is the
    /// move itself, computed as it strikes.
    pub damage_at_use: Option<ForeseenRoll>,
    /// Gen 6+: the use returns `NOT_FAIL` (the move succeeded); Gen 2-5 `null` (as failed).
    pub use_succeeds: bool,
    /// Gen 3 on the cartridge (issues.md I111: pokeemerald's `HandleWishPerishSongOnTurnEnd`
    /// goes through the battlers by number): the strikes of one end of turn go by battler, not
    /// by speed (Showdown's `speedSort`; pokeplatinum's `monSpeedOrder`).
    pub battler_order: bool,
    /// Gen 5+: the user's Life Orb takes its toll after the strike, if the user is on the field
    /// (the condition's `onEnd`, whatever the strike did).
    pub life_orb_after: bool,
    /// Gen 6+ (the condition's `onEnd`: `data.source.hasAbility('normalize')`): the strike is
    /// Normal when its user is on the field with Normalize as it strikes (R-M8-151).
    pub source_normalize_at_strike: bool,
}

impl FutureAttackRules {
    pub const fn for_generation(generation: Generation) -> Self {
        let damage_at_use = match generation {
            Generation::Gen1 | Generation::Gen2 => Some(ForeseenRoll::BeforeAccuracy),
            Generation::Gen3 => Some(ForeseenRoll::AfterAccuracy),
            Generation::Gen4 => Some(ForeseenRoll::AtUse),
            _ => None,
        };
        Self {
            damage_at_use,
            use_succeeds: damage_at_use.is_none() && !matches!(generation, Generation::Gen5),
            battler_order: matches!(generation, Generation::Gen3),
            life_orb_after: damage_at_use.is_none(),
            source_normalize_at_strike: damage_at_use.is_none()
                && !matches!(generation, Generation::Gen5),
        }
    }
}
