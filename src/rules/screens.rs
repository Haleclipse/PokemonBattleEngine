//! What a side puts up against its foes, as a generation has it (DESIGN.md §10.7, §10.8): the
//! screens — Reflect, Light Screen, Aurora Veil: where one lives and how it weakens a hit — and,
//! with them, Mist, Safeguard and Lucky Chant: how long they last and when they count down.

use crate::spec::Generation;

/// Where a screen lives and how it weakens a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenModel {
    /// Gen 1 (`volatileStatus`, `target: "self"`): a volatile on its user, until it leaves the
    /// field. `getDamage` doubles the holder's Defense / Special before the high-stat rollover;
    /// a critical hit, which reads the stored stats, ignores it.
    DoubledStatVolatile,
    /// Gen 2: a side condition. `getStat` doubles the Defense / Special Defense of the side's
    /// Pokémon after the 999 cap — a confusion self-hit on itself included — unless a critical
    /// hit ignores the stages (the attacker's stage not ahead of the defender's).
    DoubledStatSide,
    /// Gen 3+: a side condition weakening a move's hit on the side's Pokémon by a 4096-based
    /// modifier — not the user's hit on itself, not a critical hit. Gen 3-4 apply it in
    /// `ModifyDamagePhase1` (`DamageModifierKind::ModifyDamagePhase1`), later generations in the
    /// `ModifyDamage` chain, after every Pokémon's handler (a side's handlers have no speed).
    DamageModifier,
}

/// When a screen's side counts as more than one (`ScreenRules::crowded`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrowdedSide {
    /// Gen 3 on the cartridge (pokeemerald `CountAliveMonsInBattle(BATTLE_ALIVE_DEF_SIDE) == 2`,
    /// which reads the absent flags, not the HP; issues.md I218): a partner of the target in its
    /// slot that has not fainted yet — one a spread move just knocked out still counts at the
    /// next target's hit (Showdown's `!fainted`, the oracle's Gen 3 damage). The Gen 3 spread
    /// modifier reads the same count (`DamageRules::spread_only_foes`).
    PartnerPresent,
    /// Gen 4 (`target.alliesAndSelf().length > 1`): a partner of the target still standing.
    PartnerStanding,
    /// Gen 5+ (`activePerHalf > 1`): whenever the format has two active slots a side.
    TwoSlots,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRules {
    pub model: ScreenModel,
    /// Showdown `durationCallback`: 8 turns when the user holds Light Clay
    /// (`SideProtectionRules::turns` otherwise).
    pub light_clay_turns: u8,
    /// `chainModify(0.5)`.
    pub modifier: u32,
    /// The modifier when the side counts as more than one: Gen 3-4 `chainModify(2, 3)` (2730),
    /// Gen 5 [2703, 4096], Gen 6+ [2732, 4096].
    pub crowded_modifier: u32,
    /// When the target's side counts as more than one.
    pub crowded: CrowdedSide,
    /// Gen 3 Brick Break (`source.side.foe.removeSideCondition`) breaks the screens of its
    /// user's foes; from Gen 4 those of its target's side (`pokemon.side`), a partner's too.
    pub broken_on_user_foes: bool,
}

impl ScreenRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        let model = match generation {
            Generation::Gen1 => ScreenModel::DoubledStatVolatile,
            Generation::Gen2 => ScreenModel::DoubledStatSide,
            _ => ScreenModel::DamageModifier,
        };
        Self {
            model,
            light_clay_turns: 8,
            modifier: 2048,
            crowded_modifier: match generation {
                Generation::Gen5 => 2703,
                Generation::Gen3 | Generation::Gen4 => 2730,
                _ => 2732,
            },
            crowded: match generation {
                Generation::Gen3 => CrowdedSide::PartnerPresent,
                Generation::Gen4 => CrowdedSide::PartnerStanding,
                _ => CrowdedSide::TwoSlots,
            },
            broken_on_user_foes: matches!(generation, Generation::Gen3),
        }
    }
}

/// The side conditions that only last a number of turns (`SideCondition::Timed`: the screens,
/// Mist, Safeguard, Lucky Chant) and Mist's generation difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideProtectionRules {
    /// Showdown `duration`: 5.
    pub turns: u8,
    /// Gen 3-4 count them down at `onSideResidualOrder` 1-6 (Reflect 1, Light Screen 2, Mist 3,
    /// Safeguard 4, Lucky Chant 6), before the weather (8) and anything else that can decide
    /// the battle; Gen 2 (Safeguard 8, the screens 9) and Gen 5+ (26) after it.
    pub tick_first: bool,
    /// Gen 1 Mist (its gen1 `onTryBoost`: `effect.category !== 'Status'`) stops the drops of a
    /// status move only; a damaging move's go through. From Gen 2 it stops every drop another
    /// Pokémon causes.
    pub mist_spares_damaging_moves: bool,
}

impl SideProtectionRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        Self {
            turns: 5,
            tick_first: matches!(generation, Generation::Gen3 | Generation::Gen4),
            mist_spares_damaging_moves: matches!(generation, Generation::Gen1),
        }
    }
}
