//! Berry numbers and timings that change between generations (DESIGN.md §40.3):
//! - the five confusion berries (Figy, Wiki, Mago, Aguav, Iapapa) are eaten at half HP and restore
//!   an eighth through Gen 6 (the gen6 mod), at a quarter from Gen 7 (Gluttony: half) and restore
//!   a half in Gen 7 (the gen7 mod) and a third from Gen 8;
//! - Gen 2's berries wait for less than half (pokecrystal `HandleHPHealingItem`: `2 × HP < max
//!   HP`), later ones take half too (issues.md I32);
//! - Sitrus Berry restores 30 HP in Gen 3 (the gen3 mod), a quarter from Gen 4;
//! - where the status berries are eaten (issues.md I33, I34);
//! - whether Jaboca and Rowap Berry need the attacker standing (issues.md I45).

use crate::spec::Generation;

use super::Fraction;

/// How low a holder's HP must be for a berry: half (Gen 2: below half), or a quarter (half with
/// Gluttony — Showdown's `hp <= maxhp / 4 || (hp <= maxhp / 2 && hasAbility('gluttony'))`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpShare {
    Half,
    Pinch,
}

/// How much a berry restores (Showdown `heal(n)`: truncated, at least 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BerryHeal {
    Fixed(u16),
    /// A share of max HP (Showdown's `baseMaxhp`: the same without Dynamax).
    MaxHp(Fraction),
}

impl BerryHeal {
    pub fn of(self, max_hp: u16) -> u16 {
        match self {
            Self::Fixed(amount) => amount,
            Self::MaxHp(share) => share.apply_u16(max_hp).max(1),
        }
    }
}

/// Where the status berries (and the confusion cures: Persim, Lum, Gen 2's Bitter and Miracle
/// Berry) are eaten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBerryTiming {
    /// Gen 2 (pokecrystal, I33): as another Pokémon's move inflicts the status or the confusion
    /// (`UseHeldStatusHealingItem` / `UseConfusionHealingItem` in the effect commands), otherwise
    /// at the end of the turn (`HandleHealingItems`, the residual's `ResidualKind::Berry`) —
    /// never at an `Update`.
    InflictionOrTurnEnd,
    /// Gen 3 (pokeemerald, I34): at every `Update`, and also where a replacement request cuts
    /// one short (`MOVEEND_ITEM_EFFECTS_ALL`, `ENDTURN_ITEMS2`, after the replacements), before
    /// the request.
    UpdateAndBeforeReplacement,
    /// Gen 4+ (Showdown): at every `Update`.
    Update,
}

/// Micle Berry as the generation has it (issues.md I42). Gen 4 follows pokeplatinum
/// (`HOLD_EFFECT_PINCH_ACC_UP` in `BattleSystem_TriggerHeldItem`, the `moveEffectsData.micleBerry`
/// flag, `BattleControllerPlayer_CheckMoveHitAccuracy`); Gen 5+ Showdown (`onResidual`, a
/// `micleberry` condition with `duration: 2`, `onSourceAccuracy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicleRules {
    /// Eaten like the other pinch berries, at an `Update` (Gen 4); otherwise in the residual.
    pub eaten_at_update: bool,
    /// How long the boost waits for a move (Gen 5+: 2 turns); none: until used, or the holder
    /// leaves the field (Gen 4).
    pub boost_turns: Option<u8>,
    /// A move sure to hit uses the boost up too (Gen 5+: every move that runs the `Accuracy`
    /// event); Gen 4 only a move with an accuracy to check.
    pub sure_hit_moves_use_it: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BerryRules {
    pub confusion_berry_at: HpShare,
    /// The share of max HP a confusion berry restores (truncated, at least 1).
    pub confusion_berry_heal: Fraction,
    /// `HpShare::Half` means below half, not at half (Gen 2).
    pub half_means_below: bool,
    pub sitrus_heal: BerryHeal,
    pub status_berries: StatusBerryTiming,
    /// Micle Berry (issues.md I42).
    pub micle: MicleRules,
    /// A stat berry is only eaten while its effect has room (Gen 3-4, issues.md I37:
    /// pokeemerald `ItemBattleEffects`, pokeplatinum `BattleSystem_TriggerHeldItem`): Liechi …
    /// Apicot Berry with the stat below +6, Lansat Berry without Focus Energy, Starf Berry with
    /// one of its five stats below +6.
    pub stat_berries_need_room: bool,
    /// Jaboca Berry, Rowap Berry (issues.md I45): the attacker must still stand on the field
    /// for the berry to go off — Gen 4 (pokeplatinum `BattleSystem_TriggerHeldItemOnHit` and
    /// `BattleSystem_TriggerHeldItemOnPivotMove` ask `ATTACKING_MON.curHP`), Gen 7+ (Showdown
    /// `source.hp && source.isActive`). Gen 5-6 (the gen6 mod) eat it whatever: an attacker Rough
    /// Skin has just knocked out still costs the holder its berry.
    pub recoil_berries_need_the_attacker: bool,
    /// Gen 4 (pokeplatinum `BattleSystem_PluckBerry`, issues.md I57): a confusion berry eaten
    /// from another's hand (Bug Bite, Pluck) does nothing — no heal, no confusion — to an eater
    /// at full HP. Showdown's `onEat` heals nothing then but still confuses an eater that
    /// dislikes the flavor.
    pub taken_confusion_berry_needs_damage: bool,
    /// Gen 4 (pokeplatinum `BattleSystem_PluckBerry`, issues.md I59): an eater with Klutz gets
    /// nothing from a berry it takes (Bug Bite, Pluck), whatever it holds — the cartridge asks
    /// the ability itself. Showdown's `ignoringItem()` spares a Klutz holder of an item marked
    /// `ignoreKlutz` (Macho Brace, the Power items).
    pub klutz_spoils_taken_berries: bool,
    /// The PP berries (Gen 2 Mystery Berry, Gen 3 Leppa Berry; issues.md I49) are eaten only at
    /// the end of the turn — pokecrystal `HandleMysteryberry`, pokeemerald `ENDTURN_ITEMS1`
    /// (`HOLD_EFFECT_RESTORE_PP` only `!moveTurn`) — for the first move with no PP left; from
    /// Gen 4 at an `Update` (pokeplatinum `BattleSystem_TriggerHeldItem`, Showdown `onUpdate`).
    pub pp_berries_at_turn_end: bool,
    /// Gen 3 (pokeemerald `HOLD_EFFECT_RESTORE_PP`, issues.md I185): the Leppa Berry reads the
    /// party's moves and PP (`GetMonData(mon, MON_DATA_MOVE1 + i)`), which a mimicked or
    /// transformed slot never writes back to — the own slots (Showdown `baseMoveSlots`) — and
    /// restores the first own move with no PP left; the battle's slot only when it is that own
    /// move (`MOVE_IS_PERMANENT`). Showdown and Gen 4 (pokeplatinum: the battler's `ppCur`) read
    /// the slots the Pokémon battles with.
    pub pp_berry_reads_own_slots: bool,
}

impl BerryRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        let (confusion_berry_at, confusion_berry_heal) = match generation {
            Generation::Gen7 => (HpShare::Pinch, Fraction::new(1, 2)),
            Generation::Gen8 | Generation::Gen9 => (HpShare::Pinch, Fraction::new(1, 3)),
            _ => (HpShare::Half, Fraction::new(1, 8)),
        };
        Self {
            confusion_berry_at,
            confusion_berry_heal,
            half_means_below: matches!(generation, Generation::Gen1 | Generation::Gen2),
            sitrus_heal: match generation {
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => BerryHeal::Fixed(30),
                _ => BerryHeal::MaxHp(Fraction::new(1, 4)),
            },
            status_berries: match generation {
                Generation::Gen1 | Generation::Gen2 => StatusBerryTiming::InflictionOrTurnEnd,
                Generation::Gen3 => StatusBerryTiming::UpdateAndBeforeReplacement,
                _ => StatusBerryTiming::Update,
            },
            stat_berries_need_room: matches!(generation, Generation::Gen3 | Generation::Gen4),
            // Jaboca / Rowap Berry: Gen 5-6 eat for an attacker that has fainted or left the
            // field (the gen6 mod drops the check); Gen 7+ ask `source.hp && source.isActive`.
            // Gen 4's own handler has no check either, yet Showdown keeps the berry when the
            // attacker is knocked out the same hit — open (issues.md I116).
            recoil_berries_need_the_attacker: !matches!(
                generation,
                Generation::Gen5 | Generation::Gen6
            ),
            taken_confusion_berry_needs_damage: matches!(generation, Generation::Gen4),
            klutz_spoils_taken_berries: matches!(generation, Generation::Gen4),
            pp_berries_at_turn_end: matches!(
                generation,
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3
            ),
            pp_berry_reads_own_slots: matches!(generation, Generation::Gen3),
            micle: if matches!(generation, Generation::Gen4) {
                MicleRules {
                    eaten_at_update: true,
                    boost_turns: None,
                    sure_hit_moves_use_it: false,
                }
            } else {
                MicleRules {
                    eaten_at_update: false,
                    boost_turns: Some(2),
                    sure_hit_moves_use_it: true,
                }
            },
        }
    }

    /// Whether `hp` out of `max_hp` is low enough for `share`; `gluttony`: the holder's Gluttony.
    pub const fn reached(&self, share: HpShare, hp: u16, max_hp: u16, gluttony: bool) -> bool {
        let (hp, max_hp) = (hp as u32, max_hp as u32);
        let half = if self.half_means_below {
            hp * 2 < max_hp
        } else {
            hp * 2 <= max_hp
        };
        match share {
            HpShare::Half => half,
            HpShare::Pinch => hp * 4 <= max_hp || (gluttony && half),
        }
    }
}
