//! A generation's end-of-turn order (`ResidualRules`).

use super::*;

/// Which residual (end-of-turn) effects the engine schedules, in Showdown's `onResidualOrder`
/// / `onResidualSubOrder` terms (ROADMAP §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResidualKind {
    /// The weather's upkeep (Showdown `onFieldResidualOrder`): duration tick, then sandstorm /
    /// hail damage on every Pokémon that is not immune.
    Weather,
    /// Gen 2: a frozen Pokémon's 25/256 chance to thaw (`frz` `onResidualOrder` 7).
    FreezeThaw,
    /// Shed Skin's chance to cure the holder.
    ShedSkin,
    /// Gen 3 Rain Dish's heal in rain (its gen3 `onResidual`; later an `onWeather`).
    RainDish,
    /// Gen 5+ Hydration's cure in rain (Gen 4: an `onWeather`).
    Hydration,
    /// Wish's slot condition: the turn it was made it counts down, the next it heals.
    Wish,
    /// Future Sight / Doom Desire's slot condition (`futuremove`): two counts, then the strike.
    FutureAttack,
    /// Perish Song's count (the holder faints at 0).
    PerishSong,
    /// Grassy Terrain's heal (its `onResidual`, run for every active Pokémon: a grounded one
    /// that is not semi-invulnerable gets 1/16 of its max HP).
    GrassyTerrain,
    /// A combined Fire Pledge's side (`onResidualOrder` 5.1): every non-Fire type on that side
    /// loses 1/8 of its max HP.
    FirePledge,
    Leftovers,
    /// Gen 2-3's berries at the end of the turn (`onResidual`: Gen 3's HP and confusion berries,
    /// Gen 2's Berry, Gold Berry and Berry Juice — and, as the cartridge has them, Gen 2's status
    /// berries: issues.md I33).
    Berry,
    /// Ingrain's heal.
    Ingrain,
    /// Bind / Wrap / Fire Spin …: the trapped Pokémon's chip (ended when the trapper left).
    PartiallyTrapped,
    /// Octolock's Defense and Special Defense drops (ended when the octolocker left).
    Octolock,
    BlackSludge,
    /// Toxic Orb / Flame Orb: the holder's own status, late in the schedule.
    StatusOrb,
    /// Speed Boost: +1 Speed, late in the schedule.
    SpeedBoost,
    /// Slow Start's countdown (`onResidualOrder` 28.2 in every generation that has it).
    SlowStart,
    /// Gen 3 Truant's flag flipping (the gen3 mod's `onResidual`, order 27; pokeemerald
    /// `ABILITYEFFECT_ENDTURN`).
    Truant,
    LeechSeed,
    Poison,
    Toxic,
    Burn,
    Taunt,
    Encore,
    Disable,
    /// Nightmare's chip (Gen 2: after the holder's own action instead).
    Nightmare,
    /// Curse's chip (Gen 2: after the holder's own action instead).
    Curse,
    /// Yawn's countdown; the holder falls asleep when it runs out.
    Yawn,
    /// Heal Block's countdown.
    HealBlock,
    /// Embargo's countdown (Gen 5+ 21, Gen 4 10.18).
    Embargo,
    /// Magnet Rise's countdown (Gen 5+ 18, Gen 4 10.16).
    MagnetRise,
    /// Telekinesis's countdown (19).
    Telekinesis,
    /// Healer's chance to cure an adjacent ally (Gen 5+).
    Healer,
    /// Salt Cure's chip.
    SaltCure,
    /// Syrup Bomb's countdown and Speed drop.
    SyrupBomb,
    /// Throat Chop's countdown.
    ThroatChop,
    /// Laser Focus's countdown (its `duration`, no residual order of its own: last).
    LaserFocus,
    /// Roost's end (`duration: 1`, `onResidualOrder` 25): the holder is Flying again.
    Roost,
    /// Electrify's end (its `duration`, no residual order of its own: last).
    Electrify,
    /// Powder's end (its `duration`, no residual order of its own: last).
    Powder,
    /// The self-changing abilities' own form (Zen Mode, Schooling, Shields Down, Hunger Switch,
    /// Power Construct: `onResidualOrder` 29, DESIGN.md §43.4).
    FormeChange,
    /// Gen 5+ Micle Berry (its `onResidual`, no order of its own: last): eaten at a quarter of
    /// the HP.
    MicleBerry,
    /// Gen 5+ Micle Berry's boost wearing off (its `duration`, no order of its own).
    MicleBoost,
    /// Bad Dreams: 1/8 of their max HP off the holder's sleeping foes.
    BadDreams,
    /// Aqua Ring's heal: a sixteenth of the holder's max HP.
    AquaRing,
    /// Moody: +2 to a random stat, -1 to another.
    Moody,
    /// White Herb's check at the end of the turn (its `onResidual`, order 29: last).
    WhiteHerb,
    /// Mirror Herb's use of the raises it kept (order 29).
    MirrorHerb,
    /// Eject Pack's use after a drop (order 29).
    EjectPack,
    /// Opportunist's copy of the raises it kept (order 29).
    Opportunist,
    /// Dynamax's turn counter (its own `onResidualPriority: -100`: after every other residual).
    DynamaxCounter,
    /// Sticky Barb's eighth (Gen 4: 10.20, with the orbs; Gen 5+ 28.3).
    StickyBarb,
    /// Harvest (Gen 5+): the berry the holder last ate back — in the sun, or on a 1-in-2
    /// chance.
    Harvest,
    /// Cud Chew (Gen 9): the berry the holder ate eaten again, the second turn's end after.
    CudChew,
    /// Gen 3-4 Thrash and its kind on the cartridge (`ENDTURN_THRASH`,
    /// `MON_COND_CHECK_STATE_THRASH`, issues.md I61): the lock's count, and its end. From Gen 5
    /// it is a duration of the residual (`DurationEffect`'s volatiles).
    Rampage,
    /// Uproar's turn-end step: Gen 3-4 its wake-ups and count (10.11), Gen 5+ its count and
    /// Throat Chop's end (28.1).
    Uproar,
    /// Pickup (Gen 5+): an adjacent Pokémon's item used up this turn, picked up.
    Pickup,
}

/// A side or field condition that only counts down in the residual (Showdown `fieldEvent`
/// with `getKey = 'duration'`: a handler for its `duration` alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationEffect {
    Side(crate::state::SideConditionKind),
    Field(crate::state::FieldConditionKind),
    Terrain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResidualRules {
    /// `(kind, order, sub_order)`; lower runs first, ties broken by speed (faster first).
    pub order: &'static [(ResidualKind, u8, u8)],
    /// Gen 2+ (DESIGN.md §31.1, R190): the duration-only handlers take their place among
    /// the others — the side's (`onSideResidualOrder`, speed 0) and the field's
    /// (`onFieldResidualOrder`, speed 0) by these keys; one not listed, and every Pokémon
    /// volatile that only counts down, has no order and runs after every numbered handler
    /// (Showdown `comparePriority`: a volatile by its holder's speed at sub-order 2, then the
    /// side's at 4 and the field's at 5). `None`: Gen 1, whose durations are ticked at the end
    /// of the turn (`BattleEngine::expire_conditions_end_turn`).
    pub durations: Option<&'static [(DurationEffect, u8, u8)]>,
}

impl ResidualRules {
    /// The kind's `(order, sub-order)`, or `None` when the generation has no such end-of-turn
    /// residual (Gen 2's poison, burn and Leech Seed act after the Pokémon's own action).
    pub fn key(&self, kind: ResidualKind) -> Option<(u8, u8)> {
        self.order
            .iter()
            .find(|(k, _, _)| *k == kind)
            .map(|(_, order, sub)| (*order, *sub))
    }

    /// A duration-only side or field handler's `(order, sub-order)` (`durations`).
    pub fn duration_key(&self, effect: DurationEffect) -> (u8, u8) {
        self.durations
            .unwrap_or(&[])
            .iter()
            .find(|(e, _, _)| *e == effect)
            .map_or_else(
                || match effect {
                    DurationEffect::Side(_) => (u8::MAX, UNORDERED_SIDE_SUB_ORDER),
                    DurationEffect::Field(_) | DurationEffect::Terrain => {
                        (u8::MAX, UNORDERED_FIELD_SUB_ORDER)
                    }
                },
                |(_, order, sub)| (*order, *sub),
            )
    }
}

/// Showdown `resolvePriority`'s sub-orders for a condition with none of its own: on a Pokémon
/// 2, on a slot 3, on a side 4, on the field 5.
pub const UNORDERED_VOLATILE_SUB_ORDER: u8 = 2;

pub const UNORDERED_SLOT_SUB_ORDER: u8 = 3;

const UNORDERED_SIDE_SUB_ORDER: u8 = 4;

const UNORDERED_FIELD_SUB_ORDER: u8 = 5;

/// Gen 5+ duration-only handlers (Showdown `data/moves` conditions): the screens and the side's
/// other protections at 26 (Reflect 1, Light Screen 2, Safeguard 3, Mist 4, Tailwind 5, Lucky
/// Chant 6, Aurora Veil 10), the field's at 27 (Trick Room 1, Gravity 2, Water Sport 3, Mud
/// Sport 4, Wonder Room 5, the terrain 7). The guards of a turn (Wide / Quick Guard, Crafty
/// Shield, Mat Block), Echoed Voice, Ion Deluge and Fairy Lock have no order.
const MODERN_DURATION_ORDER: &[(DurationEffect, u8, u8)] = {
    use crate::state::{FieldConditionKind as F, SideConditionKind as S};
    &[
        (DurationEffect::Side(S::Reflect), 26, 1),
        (DurationEffect::Side(S::LightScreen), 26, 2),
        (DurationEffect::Side(S::Safeguard), 26, 3),
        (DurationEffect::Side(S::Mist), 26, 4),
        (DurationEffect::Side(S::Tailwind), 26, 5),
        (DurationEffect::Side(S::LuckyChant), 26, 6),
        (DurationEffect::Side(S::AuroraVeil), 26, 10),
        (DurationEffect::Field(F::TrickRoom), 27, 1),
        (DurationEffect::Field(F::Gravity), 27, 2),
        (DurationEffect::Field(F::WaterSport), 27, 3),
        (DurationEffect::Field(F::MudSport), 27, 4),
        (DurationEffect::Field(F::WonderRoom), 27, 5),
        (DurationEffect::Field(F::MagicRoom), 27, 6),
        (DurationEffect::Terrain, 27, 7),
    ]
};

/// Gen 4 (Showdown gen4 mod): Gravity 9, Trick Room 13, neither with a sub-order. The side's
/// protections and Tailwind count down before the residual (`side_protections.tick_first`,
/// `tailwind_ticks_first`).
const GEN4_DURATION_ORDER: &[(DurationEffect, u8, u8)] = &[
    (
        DurationEffect::Field(crate::state::FieldConditionKind::Gravity),
        9,
        UNORDERED_FIELD_SUB_ORDER,
    ),
    (
        DurationEffect::Field(crate::state::FieldConditionKind::TrickRoom),
        13,
        UNORDERED_FIELD_SUB_ORDER,
    ),
];

/// Gen 2 (Showdown gen2 mod): Safeguard 8, Reflect and Light Screen 9, none with a sub-order.
const GEN2_DURATION_ORDER: &[(DurationEffect, u8, u8)] = &[
    (
        DurationEffect::Side(crate::state::SideConditionKind::Safeguard),
        8,
        UNORDERED_SIDE_SUB_ORDER,
    ),
    (
        DurationEffect::Side(crate::state::SideConditionKind::Reflect),
        9,
        UNORDERED_SIDE_SUB_ORDER,
    ),
    (
        DurationEffect::Side(crate::state::SideConditionKind::LightScreen),
        9,
        UNORDERED_SIDE_SUB_ORDER,
    ),
];

/// Gen 5+ (Showdown `data/conditions`, `items`, `moves`): weather 1, Leftovers 5.4, Leech
/// Seed 8, poison / toxic 9, burn 10, Nightmare 11, Taunt 15, Encore 16, Disable 17, Heal Block
/// 20, Yawn 23.
const MODERN_RESIDUAL_ORDER: &[(ResidualKind, u8, u8)] = &[
    (ResidualKind::Weather, 1, 0),
    (ResidualKind::FutureAttack, 3, 3),
    (ResidualKind::Wish, 4, 0),
    (ResidualKind::FirePledge, 5, 1),
    (ResidualKind::GrassyTerrain, 5, 2),
    (ResidualKind::ShedSkin, 5, 3),
    (ResidualKind::Hydration, 5, 3),
    (ResidualKind::Healer, 5, 3),
    (ResidualKind::Leftovers, 5, 4),
    (ResidualKind::BlackSludge, 5, 4),
    (ResidualKind::AquaRing, 6, 0),
    (ResidualKind::Ingrain, 7, 0),
    (ResidualKind::LeechSeed, 8, 0),
    (ResidualKind::Poison, 9, 0),
    (ResidualKind::Toxic, 9, 0),
    (ResidualKind::Burn, 10, 0),
    (ResidualKind::Nightmare, 11, 0),
    (ResidualKind::Curse, 12, 0),
    (ResidualKind::PartiallyTrapped, 13, 0),
    (ResidualKind::SaltCure, 13, 0),
    (ResidualKind::SyrupBomb, 14, 0),
    (ResidualKind::Octolock, 14, 0),
    (ResidualKind::PerishSong, 24, 0),
    (ResidualKind::Taunt, 15, 0),
    (ResidualKind::Encore, 16, 0),
    (ResidualKind::Disable, 17, 0),
    (ResidualKind::MagnetRise, 18, 0),
    (ResidualKind::Telekinesis, 19, 0),
    (ResidualKind::HealBlock, 20, 0),
    (ResidualKind::Embargo, 21, 0),
    (ResidualKind::ThroatChop, 22, 0),
    (ResidualKind::Roost, 25, 0),
    (ResidualKind::LaserFocus, u8::MAX, 0),
    (ResidualKind::Electrify, u8::MAX, 0),
    (ResidualKind::Powder, u8::MAX, 0),
    (ResidualKind::MicleBoost, u8::MAX, 2),
    (ResidualKind::MicleBerry, u8::MAX, 8),
    (ResidualKind::Yawn, 23, 0),
    (ResidualKind::SpeedBoost, 28, 2),
    (ResidualKind::SlowStart, 28, 2),
    (ResidualKind::BadDreams, 28, 2),
    (ResidualKind::Uproar, 28, 1),
    (ResidualKind::Moody, 28, 2),
    (ResidualKind::Harvest, 28, 2),
    (ResidualKind::CudChew, 28, 2),
    (ResidualKind::Pickup, 28, 2),
    (ResidualKind::StatusOrb, 28, 3),
    (ResidualKind::StickyBarb, 28, 3),
    (ResidualKind::FormeChange, 29, 0),
    (ResidualKind::WhiteHerb, 29, 0),
    (ResidualKind::MirrorHerb, 29, 0),
    (ResidualKind::EjectPack, 29, 0),
    (ResidualKind::Opportunist, 29, 0),
    (ResidualKind::DynamaxCounter, u8::MAX, 1),
];

/// Gen 4 (Showdown gen4 mod): weather 8, then everything at 10 with a sub-order — Leftovers
/// 10.4, Leech Seed 10.5, burn / poison / toxic 10.6, Nightmare 10.7, Curse 10.8, Disable 10.13, Encore
/// 10.14, Taunt 10.15, Heal Block 10.17, Yawn 10.19.
const GEN4_RESIDUAL_ORDER: &[(ResidualKind, u8, u8)] = &[
    (ResidualKind::Wish, 7, 0),
    (ResidualKind::Weather, 8, 0),
    (ResidualKind::Ingrain, 10, 1),
    (ResidualKind::AquaRing, 10, 2),
    (ResidualKind::ShedSkin, 10, 3),
    (ResidualKind::SpeedBoost, 10, 3),
    (ResidualKind::Leftovers, 10, 4),
    (ResidualKind::BlackSludge, 10, 4),
    (ResidualKind::LeechSeed, 10, 5),
    (ResidualKind::Poison, 10, 6),
    (ResidualKind::Toxic, 10, 6),
    (ResidualKind::Burn, 10, 6),
    (ResidualKind::Nightmare, 10, 7),
    (ResidualKind::Curse, 10, 8),
    (ResidualKind::PartiallyTrapped, 10, 9),
    (ResidualKind::BadDreams, 10, 10),
    (ResidualKind::Uproar, 10, 11),
    (ResidualKind::Rampage, 10, 12),
    (ResidualKind::Disable, 10, 13),
    (ResidualKind::Encore, 10, 14),
    (ResidualKind::Taunt, 10, 15),
    (ResidualKind::MagnetRise, 10, 16),
    (ResidualKind::HealBlock, 10, 17),
    (ResidualKind::Embargo, 10, 18),
    (ResidualKind::Roost, 25, 0),
    (ResidualKind::Yawn, 10, 19),
    (ResidualKind::StatusOrb, 10, 20),
    (ResidualKind::StickyBarb, 10, 20),
    (ResidualKind::FutureAttack, 11, 3),
    (ResidualKind::PerishSong, 12, 0),
    (ResidualKind::SlowStart, 28, 2),
    (ResidualKind::WhiteHerb, 29, 0),
];

/// Gen 3: Gen 4's table and the berries' 10.4 (the gen3 mod's `onResidual`), but the gen3 mod
/// drops Disable's `onResidualOrder`, and a handler
/// without an order runs after every numbered one (`comparePriority`): a Disable ticks last, so
/// a battle decided earlier in the residual leaves it unticked.
const GEN3_RESIDUAL_ORDER: &[(ResidualKind, u8, u8)] = &[
    (ResidualKind::Wish, 7, 0),
    (ResidualKind::Weather, 8, 0),
    (ResidualKind::Ingrain, 10, 1),
    (ResidualKind::ShedSkin, 10, 3),
    (ResidualKind::RainDish, 10, 3),
    (ResidualKind::SpeedBoost, 10, 3),
    (ResidualKind::Leftovers, 10, 4),
    (ResidualKind::BlackSludge, 10, 4),
    (ResidualKind::Berry, 10, 4),
    (ResidualKind::LeechSeed, 10, 5),
    (ResidualKind::Poison, 10, 6),
    (ResidualKind::Toxic, 10, 6),
    (ResidualKind::Burn, 10, 6),
    (ResidualKind::Nightmare, 10, 7),
    (ResidualKind::Curse, 10, 8),
    (ResidualKind::PartiallyTrapped, 10, 9),
    (ResidualKind::Uproar, 10, 11),
    (ResidualKind::Rampage, 10, 12),
    (ResidualKind::Encore, 10, 14),
    (ResidualKind::Taunt, 10, 15),
    (ResidualKind::Yawn, 10, 19),
    (ResidualKind::StatusOrb, 10, 20),
    (ResidualKind::FutureAttack, 11, 3),
    (ResidualKind::PerishSong, 12, 0),
    (ResidualKind::Truant, 27, 0),
    (ResidualKind::Disable, u8::MAX, 0),
    (ResidualKind::WhiteHerb, 29, 0),
];

/// Gen 2 (Showdown gen2 mod over the gen3 / gen4 conditions): weather 2, Bind / Wrap … 3.1,
/// Leftovers 5.1, the freeze's thaw 7, the berries 10, Encore 13, and Disable last (gen3 drops
/// its order).
/// Poison, burn, bad poison, Leech Seed, Nightmare and Curse are not here: they act after the
/// Pokémon's own action (`StatusResidualTiming::AfterOwnAction`).
const GEN2_RESIDUAL_ORDER: &[(ResidualKind, u8, u8)] = &[
    (ResidualKind::FutureAttack, 1, 3),
    (ResidualKind::Weather, 2, 0),
    (ResidualKind::PartiallyTrapped, 3, 1),
    (ResidualKind::PerishSong, 4, 0),
    (ResidualKind::Leftovers, 5, 1),
    (ResidualKind::FreezeThaw, 7, 0),
    (ResidualKind::Berry, 10, 0),
    (ResidualKind::Encore, 13, 0),
    (ResidualKind::Disable, u8::MAX, 0),
];

/// The generation's end-of-turn order.
pub(super) fn resolve(generation: Generation) -> ResidualRules {
    ResidualRules {
        order: match generation {
            // Gen 1 has no end-of-turn residual at all.
            Generation::Gen1 => &[],
            Generation::Gen2 => GEN2_RESIDUAL_ORDER,
            Generation::Gen3 => GEN3_RESIDUAL_ORDER,
            Generation::Gen4 => GEN4_RESIDUAL_ORDER,
            _ => MODERN_RESIDUAL_ORDER,
        },
        durations: match generation {
            Generation::Gen1 => None,
            Generation::Gen2 => Some(GEN2_DURATION_ORDER),
            // Gen 3's side protections count down first; it has no field condition.
            Generation::Gen3 => Some(&[]),
            Generation::Gen4 => Some(GEN4_DURATION_ORDER),
            _ => Some(MODERN_DURATION_ORDER),
        },
    }
}
