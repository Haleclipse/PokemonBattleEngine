//! What Mist, Safeguard and Lucky Chant stop (DESIGN.md §10.8), and Infiltrator's moves going
//! through them (Showdown `move.infiltrates`).

use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedRules;
use crate::state::{BattleState, SideConditionKind, VolatileConditionKind};

use super::{EffectSource, StatChangeCause, StatChangeRequest};

/// Infiltrator (`onModifyMove`: `move.infiltrates`): the Pokémon's moves go through the screens,
/// a foe's Mist and Safeguard, and — as `ConditionRules::substitute_honors_infiltrates` says —
/// a Substitute.
pub fn infiltrates(state: &BattleState, rules: &ResolvedRules, pokemon: PokemonId) -> bool {
    state
        .pokemon(pokemon)
        .is_some_and(|p| super::ability_handler(state, rules, p, |b| b.infiltrates))
}

/// Who inflicts something on a Pokémon, as Mist and Safeguard see it (Showdown's `source` and
/// `effect`): someone else — a hazard counts as laid by the other side (Showdown hands
/// `pokemon.side.foe.active[0]` as its source) — and whether it is a foe's infiltrating move
/// (`effect.effectType === 'Move' && effect.infiltrates && !target.isAlly(source)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Culprit {
    /// The target itself, or nobody (Showdown `!source || target === source`).
    None,
    Another {
        infiltrating: bool,
    },
}

impl Culprit {
    pub(crate) fn of(
        state: &BattleState,
        rules: &ResolvedRules,
        source: EffectSource,
        target: PokemonId,
    ) -> Self {
        match source {
            // Sticky Web's drop names a Pokémon as its source (Showdown hands
            // `getActiveMove('stickyweb')`), but nobody uses the move: Infiltrator's
            // `onModifyMove` never marks it, and a foe's Mist stops it (R-M11-147).
            EffectSource::Move { pokemon, move_id }
                if move_id == MoveId::STICKY_WEB && pokemon != target =>
            {
                Self::Another {
                    infiltrating: false,
                }
            }
            EffectSource::Move { pokemon, .. } => Self::by_move(state, rules, pokemon, target),
            EffectSource::Ability { pokemon, .. } | EffectSource::Item { pokemon, .. }
                if pokemon != target =>
            {
                Self::Another {
                    infiltrating: false,
                }
            }
            EffectSource::SideCondition(_) => Self::Another {
                infiltrating: false,
            },
            _ => Self::None,
        }
    }

    /// `user`'s move on `target`.
    pub(crate) fn by_move(
        state: &BattleState,
        rules: &ResolvedRules,
        user: PokemonId,
        target: PokemonId,
    ) -> Self {
        if user == target {
            return Self::None;
        }
        let foe = match (state.pokemon(user), state.pokemon(target)) {
            (Some(user), Some(target)) => user.side != target.side,
            _ => false,
        };
        Self::Another {
            infiltrating: foe && infiltrates(state, rules, user),
        }
    }

    /// Someone else, not by a foe's infiltrating move: what Mist and Safeguard stop.
    const fn stopped(self) -> bool {
        matches!(
            self,
            Self::Another {
                infiltrating: false
            }
        )
    }
}

/// Mist (`onTryBoost`): a drop someone else causes on a Pokémon of the side (Gen 3+ side
/// condition) or on the holder (Gen 1-2 volatile) is refused — in Gen 1 only a status move's
/// (`SideProtectionRules::mist_spares_damaging_moves`).
pub(crate) fn mist_refuses(
    state: &BattleState,
    rules: &ResolvedRules,
    request: &StatChangeRequest,
) -> bool {
    if request.stages >= 0 {
        return false;
    }
    let Some(target) = state.pokemon(request.target) else {
        return false;
    };
    let misted = target.volatiles.contains(VolatileConditionKind::Mist)
        || state
            .side(target.side)
            .conditions
            .contains(SideConditionKind::Mist);
    if !misted
        || (rules.conditions.side_protections.mist_spares_damaging_moves
            && request.cause == StatChangeCause::Secondary)
    {
        return false;
    }
    Culprit::of(state, rules, request.source, request.target).stopped()
}

/// Safeguard (`onSetStatus`, `onTryAddVolatile` for confusion): what `culprit` inflicts on a
/// Pokémon of the side is refused.
pub(crate) fn safeguard_refuses(state: &BattleState, target: PokemonId, culprit: Culprit) -> bool {
    culprit.stopped()
        && state.pokemon(target).is_some_and(|pokemon| {
            state
                .side(pokemon.side)
                .conditions
                .contains(SideConditionKind::Safeguard)
        })
}

/// Lucky Chant (`onCriticalHit: false`): a critical hit on the side's Pokémon is turned back
/// after the roll, like Battle Armor.
pub(crate) fn lucky_chant(state: &BattleState, target: PokemonId) -> bool {
    state.pokemon(target).is_some_and(|pokemon| {
        state
            .side(pokemon.side)
            .conditions
            .contains(SideConditionKind::LuckyChant)
    })
}

/// Dazzling, Queenly Majesty, Armor Tail (`onFoeTryMove`, DESIGN.md R275): the first of
/// `user`'s foes on the field, fastest first, whose ability stops a move of raised priority
/// (`priority`, as the move's effects see it) aimed at its side — `target` there, the user
/// itself for a move with no target of its own (Showdown `getTarget`) — or a Perish Song,
/// Flower Shield or Rototiller; a move at the foe's side or the rest of the field is left
/// alone. The holder and its ability.
pub fn priority_guard(
    state: &BattleState,
    rules: &ResolvedRules,
    user: PokemonId,
    target: Option<PokemonId>,
    move_def: &crate::dex::ResolvedMove,
    priority: i8,
) -> Option<(PokemonId, crate::ids::AbilityId)> {
    use crate::ids::MoveId;
    use crate::rules::TargetPattern;
    let whole_field = move_def.target == TargetPattern::Field;
    if priority <= 0
        || move_def.target == TargetPattern::OpponentSide
        || (whole_field
            && !matches!(
                move_def.id,
                MoveId::PERISH_SONG | MoveId::FLOWER_SHIELD | MoveId::ROTOTILLER
            ))
    {
        return None;
    }
    let side = state.pokemon(user)?.side;
    // The move's target is on the holders' side (`source.isAlly(holder)`): the user's foes'.
    let aimed = state
        .pokemon(target.unwrap_or(user))
        .is_some_and(|aimed| aimed.side != side);
    let mut holders: Vec<(PokemonId, u32)> = state
        .side(side.other())
        .active
        .iter()
        .flatten()
        .copied()
        .filter_map(|holder| {
            let pokemon = state.pokemon(holder)?;
            (!pokemon.is_fainted()
                && super::ability_handler(state, rules, pokemon, |b| b.blocks_priority_moves)
                && (whole_field || aimed))
                .then(|| (holder, super::stats::handler_speed(state, holder)))
        })
        .collect();
    holders.sort_by_key(|(_, speed)| std::cmp::Reverse(*speed));
    let (holder, _) = holders.first().copied()?;
    Some((holder, super::raw_ability(state.pokemon(holder)?)))
}
