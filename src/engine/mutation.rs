use crate::ids::{AbilityId, ItemId, PokemonId};
use crate::state::Stat;

use super::{DamageAmount, EffectSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageKind {
    Move,
    Residual,
    /// Sandstorm / hail chip at the end of the turn (refused by Magic Guard like any non-move).
    Weather,
    EntryHazard,
    Recoil,
    /// Damage a hit's target deals back through an ability or item (Rough Skin, Rocky Helmet):
    /// straight to the attacker's HP.
    Reaction,
    /// Showdown `directDamage` that is no price: Struggle's recoil (Gen 4+), the Gen 1 crash.
    /// No `Damage` event (`DamageKind::is_direct`).
    Direct,
    /// The HP a move makes its own user pay (Showdown `directDamage` from the move's effect:
    /// Substitute, Shed Tail, Belly Drum, a Ghost's Curse, Fillet Away, Clangorous Soul). A direct
    /// damage, traced `HpCostPaid`; never Gen 1's `lastDamage` (DESIGN.md R-M8-91).
    Cost,
    /// Confusion self-hit and similar: goes straight to HP, never through a Substitute.
    SelfHit,
    Other,
}

impl DamageKind {
    /// Showdown `directDamage` (`Direct`, `Cost`): the HP goes without the `Damage` event — no
    /// ability or item refuses or notes it (Magic Guard, Berserk) — and sets no
    /// `hurtThisTurn`.
    pub fn is_direct(self) -> bool {
        matches!(self, Self::Direct | Self::Cost)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DamageFlags {
    pub bypass_substitute: bool,
    /// The move's own `onDamage` (False Swipe, Hold Back): it leaves the target at 1 HP.
    pub spare_one_hp: bool,
    /// Showdown `damage(…, instafaint)`: a target this knocks out faints at once, ahead of the
    /// Pokémon already waiting to (`faintMessages(lastFirst)`) — all of them announced then, as
    /// the hit loop's end announces them (R-M10-80).
    pub instafaint: bool,
    /// The damage's effect is a move that strikes once, its `AfterMoveSecondary` events still to
    /// come (Showdown `effect.effectType === 'Move' && !effect.multihit`, not a move Sheer Force
    /// boosted) — and confusion's self-hit, a pseudo-move of that type (issues.md I36). Berserk
    /// and Anger Shell hold the healing berries back after such a hit.
    pub single_strike: bool,
    /// The damage is a foreseen move's strike (`MoveFlags::future_move`): Destiny Bond and
    /// Grudge ignore what it knocks out.
    pub future_move: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRequest {
    pub target: PokemonId,
    pub amount: DamageAmount,
    pub kind: DamageKind,
    pub source: EffectSource,
    pub flags: DamageFlags,
}

impl DamageRequest {
    /// A move's HP price, `amount` (at least 1, as `directDamage` clamps it) off `target` — the
    /// move's own user (`DamageKind::Cost`).
    pub fn cost(target: PokemonId, amount: u16, source: EffectSource) -> Self {
        Self {
            target,
            amount: DamageAmount(amount.max(1)),
            kind: DamageKind::Cost,
            source,
            flags: DamageFlags {
                bypass_substitute: true,
                ..DamageFlags::default()
            },
        }
    }

    /// The damage's effect is a move (Showdown `effect.effectType === 'Move'`, what Focus Band,
    /// Focus Sash and Sturdy ask): a move's hit, a Gen 2-4 crash (the move itself), and the
    /// confusion self-hit's `confused` pseudo-move — but for Gen 1-2, where the self-hit is a
    /// `directDamage` (`StatusLifecycleRules::confusion_self_hit_direct`).
    pub fn from_a_move(&self, rules: &crate::rules::EffectRules<'_>) -> bool {
        match self.kind {
            DamageKind::Move => true,
            DamageKind::SelfHit => match self.source {
                EffectSource::Volatile(crate::state::VolatileConditionKind::Confusion) => {
                    !rules.status().lifecycle.confusion_self_hit_direct
                }
                _ => true,
            },
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatChangeCause {
    Move,
    /// A damaging move's secondary effect (Aurora Beam's Attack drop): Gen 1 Mist lets it
    /// through.
    Secondary,
    Ability,
    Item,
    Rule,
    /// A status Z-Move's Z power (`runZPower`'s `zpower` effect): Contrary and Simple leave it
    /// alone (Bulbapedia, R-M9-44).
    ZPower,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatChangeRequest {
    pub target: PokemonId,
    pub stat: Stat,
    pub stages: i8,
    pub source: EffectSource,
    pub cause: StatChangeCause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatChangeBlockReason {
    InvalidTarget,
    Fainted,
    /// Gen 6+: the side facing the Pokémon has no Pokémon left (`foePokemonLeft`).
    NoFoeLeft,
    NoChange,
    /// Refused by the target's ability at the TryBoost stage (Clear Body …).
    Ability(AbilityId),
    Item(ItemId),
    /// Mist on the target's side (Gen 1-2: on the target).
    Mist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryStatChangeResult {
    Allow,
    /// Refused; `shown`: the refusal's line names what refused it (Showdown's `onTryBoost`
    /// messages, most of them not for a damaging move's secondary effect).
    Blocked {
        reason: StatChangeBlockReason,
        shown: bool,
    },
}

/// What a damage-resolution handler (Showdown `onDamage`) decided about a DamageRequest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageDecision {
    Allow,
    /// Commit this amount instead (Sturdy / Focus Sash: leave 1 HP).
    Reduce(DamageAmount),
    /// Nothing is committed (Magic Guard).
    Prevent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatChangeResult {
    Applied { from: i8, to: i8 },
    Blocked(StatChangeBlockReason),
}

/// Healing goes through the same request → commit boundary as damage. `amount` is the HP to
/// restore before clamping to max HP; callers derive it (e.g. a fraction of max HP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealRequest {
    pub target: PokemonId,
    pub amount: u16,
    pub source: EffectSource,
    /// Showdown `pokemon.heal` rather than `battle.heal`: the HP itself, with no `TryHeal`
    /// event — Heal Block and Ripen never see it (Regenerator, Healing Wish, Lunar Dance; DESIGN.md
    /// R-M8-90, R-M8-91). The heal's counterpart of a direct damage (`DamageKind::is_direct`).
    pub direct: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealBlockReason {
    InvalidTarget,
    Fainted,
    /// Already at full HP (Showdown: the move fails).
    FullHp,
    /// The target's Heal Block (`onTryHeal`).
    HealBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealResult {
    Applied { from: u16, to: u16 },
    Blocked(HealBlockReason),
}
