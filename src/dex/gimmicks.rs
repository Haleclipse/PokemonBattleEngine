//! What the generation gimmicks make of a move (DESIGN.md §44.1): the power of a damaging move's
//! Z-Move (Gen 7) and Max Move (Gen 8) when its data declares none, and what a status move's Z
//! version adds. Showdown derives the powers in `dex-moves.mjs`; the move data overrides them
//! (Mega Drain's Z-Move 120, the multi-hit moves' Max Moves …).

use super::{MoveId, MultiHit, PokemonType};
use crate::state::Stat;

/// The power of a damaging move's Z-Move from its base power (Showdown `dex-moves.mjs`): a move
/// that hits a random number of times (`multihit: [2, 5]`) counts three hits; no base power
/// (a fixed-damage or variable-power move) gives 100.
pub fn z_move_power(base_power: u16, multi_hit: MultiHit) -> u16 {
    let power = match multi_hit {
        MultiHit::Range { .. } => base_power.saturating_mul(3),
        MultiHit::Single | MultiHit::Fixed(_) => base_power,
    };
    match power {
        0 => 100,
        140.. => 200,
        130.. => 195,
        120.. => 190,
        110.. => 185,
        100.. => 180,
        90.. => 175,
        80.. => 160,
        70.. => 140,
        60.. => 120,
        _ => 100,
    }
}

/// The power of a damaging move's Max Move from its base power and type (Showdown
/// `dex-moves.mjs`): Fighting and Poison moves (Max Knuckle, Max Ooze) have their own, lower
/// table; no base power gives 100.
pub fn max_move_power(move_type: PokemonType, base_power: u16) -> u16 {
    if base_power == 0 {
        return 100;
    }
    let low = matches!(move_type, PokemonType::Fighting | PokemonType::Poison);
    let (top, steps) = if low {
        (100, [95, 90, 85, 80, 75, 70])
    } else {
        (150, [140, 130, 120, 110, 100, 90])
    };
    match base_power {
        150.. => top,
        110.. => steps[0],
        75.. => steps[1],
        65.. => steps[2],
        55.. => steps[3],
        45.. => steps[4],
        _ => steps[5],
    }
}

/// What a status move's Z version adds before the move (Showdown `runZPower`, the move data's
/// `zMove.boost` / `zMove.effect`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZStatusEffect {
    /// The user's stages rise (Z-Swords Dance's Defense …).
    Boosts(Vec<(Stat, i8)>),
    /// `heal`: the user recovers all its HP.
    Heal,
    /// `healreplacement`: the Pokémon that next comes in at the user's position is healed.
    HealReplacement,
    /// `clearnegativeboost`: the user's lowered stages go back to 0.
    ClearNegativeBoost,
    /// `redirect`: Follow Me on the user.
    Redirect,
    /// `crit2`: Focus Energy on the user.
    Crit2,
    /// `curse`: a Ghost-type user recovers all its HP, any other gains one Attack stage.
    Curse,
}

impl ZStatusEffect {
    pub(crate) fn from_effect(name: &str) -> Option<Self> {
        Some(match name {
            "heal" => Self::Heal,
            "healreplacement" => Self::HealReplacement,
            "clearnegativeboost" => Self::ClearNegativeBoost,
            "redirect" => Self::Redirect,
            "crit2" => Self::Crit2,
            "curse" => Self::Curse,
            _ => return None,
        })
    }
}

/// What a Z-Crystal makes of a move (Showdown `getZMove`, DESIGN.md §44.4). `None` when the
/// crystal does not apply to this move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZConversion {
    /// The base move itself, powered (`isZ`): a status move of the crystal's type. The move data
    /// carries the Z effect (`ResolvedMove::z_status`); `runZPower` applies it.
    Status,
    /// The generic Z-Move of the crystal's type (究极无敌大冲撞 …), with the power the base
    /// move's data gives (`ResolvedMove::z_power`).
    Generic(MoveId),
    /// The crystal's own signature Z-Move (皮卡皮卡必杀击 …).
    Signature(MoveId),
}

/// The Z-Crystal's conversion of `base` for a holder of `item`, per Showdown `getZMove`.
pub fn z_conversion(
    item: &super::ResolvedItem,
    base: &super::ResolvedMove,
    generic: impl FnOnce(PokemonType) -> Option<MoveId>,
) -> Option<ZConversion> {
    let crystal = item.z_crystal?;
    match crystal {
        Some(signature) => item
            .z_move_from
            .is_some_and(|from| from == base.id)
            .then_some(ZConversion::Signature(signature)),
        None => {
            // A type crystal: the move must have its type.
            if item.z_move_type != Some(base.move_type) {
                return None;
            }
            if base.category == super::MoveCategory::Status {
                return Some(ZConversion::Status);
            }
            base.z_power?;
            generic(base.move_type).map(ZConversion::Generic)
        }
    }
}

/// What a Max Move is made of, resolved for a Dynamaxed user (Showdown `getActiveMaxMove`,
/// DESIGN.md §44.5): the Max Move of the base move's type — the type *after* its own and its
/// ability's type changes — with the base move's category and priority, and the power the
/// engine's table (or the move's data) gives. A status move is Max Guard; Struggle is itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaxMove {
    pub move_def: super::ResolvedMove,
    /// The base move the Dynamax converted (`move.baseMove`).
    pub base: MoveId,
}

/// The Max Move a type's damaging moves become (Showdown `MAX_MOVES`): 极巨攻击 for Normal,
/// 极巨拳斗 for Fighting … Each is the move whose own data names that Max Move and whose type
/// is that type; the engine reads them off the snapshot.
pub fn max_move_for(dex: &super::ResolvedDex, base: &super::ResolvedMove) -> Option<MoveId> {
    if base.id == MoveId::STRUGGLE {
        return Some(base.id);
    }
    if base.category == super::MoveCategory::Status {
        return dex.max_guard();
    }
    let max_type = max_move_type(base.move_type)?;
    dex.max_move_of_type(max_type)
}

/// The type a Max Move is chosen by (Showdown `MAX_MOVES[move.type]`): the base move's own type
/// as it will be used. `None` for the types whose Max Move the snapshots do not carry.
fn max_move_type(move_type: PokemonType) -> Option<PokemonType> {
    Some(move_type)
}
