//! Mimic and Sketch (DESIGN.md §42.5): rewriting the move slots. Mimic writes the slots the
//! Pokémon battles with (`move_overlay`, forgotten on switch-out); Sketch writes the own ones
//! (learned for good).

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::{MoveId, PokemonId};
use crate::rules::{MovesSlot, ResolvedTarget};
use crate::state::{MoveSlotState, VolatileConditionKind};

/// The move Mimic or Sketch would copy, or why it cannot (`None` when the Pokémon has
/// transformed, the target has no last move — Gen 1 draws among the slots instead — or is behind
/// a Substitute, the move refuses `failmimic` / `nosketch`, or the user knows it already).
fn copyable(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    target: PokemonId,
    mimic: bool,
) -> Result<(usize, MoveSlotState), ConditionApplyError> {
    let dex = ctx.dex().ok_or(ConditionApplyError::NothingToDo)?;
    let rules = ctx.battle_rules();
    let transformed = ctx
        .battle_state()
        .pokemon(user)
        .ok_or(ConditionApplyError::InvalidTarget)?
        .transform
        .is_some();
    let target_state = ctx
        .battle_state()
        .pokemon(target)
        .ok_or(ConditionApplyError::InvalidTarget)?;
    // Gen 2+ (`source.transformed`) refuse a transformed user; Gen 1 Mimic writes the copies
    // Transform made (R-M8-75). Sketch (Gen 2+) always refuses.
    if transformed && (!mimic || rules.conditions.calls.mimic_fails_when_transformed) {
        return Err(ConditionApplyError::NothingToDo);
    }
    if rules.conditions.calls.mimic_fails_behind_substitute
        && target_state
            .volatiles
            .contains(VolatileConditionKind::Substitute)
    {
        return Err(ConditionApplyError::NothingToDo);
    }
    // Showdown copies `target.lastMove`, the move as it was used: the slot keeps the target its
    // `onModifyMove` gave it (`moveSlot.target`). Gen 1's draw from the slots has none.
    let mut copied_pattern = None;
    let copied = match rules.conditions.calls.mimic_draws_from_the_slots {
        // Gen 1 (`this.sample(target.moves)`).
        true => {
            let slots: Vec<MoveId> = target_state
                .move_slots()
                .iter()
                .map(|slot| slot.move_id)
                .collect();
            if slots.is_empty() {
                return Err(ConditionApplyError::NothingToDo);
            }
            let index = ctx.sample_index(slots.len());
            slots[index]
        }
        false => {
            // A status move used as a Z-Move keeps its own id; `move.isZ` refuses it (R-M9-36).
            if target_state.last_move_z {
                return Err(ConditionApplyError::NothingToDo);
            }
            let last = target_state
                .last_move
                .ok_or(ConditionApplyError::NothingToDo)?;
            copied_pattern = target_state
                .last_move_pattern
                .filter(|(move_id, _)| *move_id == last)
                .map(|(_, pattern)| pattern);
            last
        }
    };
    let Some(copied_def) = dex.move_by_id(copied) else {
        return Err(ConditionApplyError::NothingToDo);
    };
    // `move.isZ || move.isMax` (DESIGN.md §44.5): neither Mimic nor Sketch copies a Z-Move or
    // a Max Move, and neither reads one through its `baseMove`.
    let refused = (if mimic {
        copied_def.flags.fail_mimic
    } else {
        copied_def.flags.no_sketch
    }) || copied_def.z_crystal.is_some()
        || copied_def.is_max;
    let source = if mimic { MoveId::MIMIC } else { MoveId::SKETCH };
    let keep_pp = mimic && rules.conditions.calls.mimic_keeps_the_slot_pp;
    let cap = mimic && rules.conditions.calls.mimic_pp_capped;
    let user_state = ctx
        .battle_state()
        .pokemon(user)
        .ok_or(ConditionApplyError::InvalidTarget)?;
    if refused
        || (rules.conditions.calls.mimic_refuses_a_known_move
            && user_state
                .move_slots()
                .iter()
                .any(|slot| slot.move_id == copied))
    {
        return Err(ConditionApplyError::NothingToDo);
    }
    // The slot the copy lands in: the copying move's (`PpRules::moves_slot`, issues.md I170).
    // Gen 1 writes the slot the side last picked from the menu (Showdown
    // `source.side.lastSelectedMoveSlot`): using Mimic directly names its own slot; a Mimic that
    // was called (Metronome, Mirror Move …) overwrites the caller's. Gen 2-3 write the slot the
    // action chose when it holds the copying move — the second of two Mimics too (pokeemerald's
    // `gCurrMovePos`) —, Gen 4+ the first slot holding it (`source.moves.indexOf('mimic')`),
    // as Gen 2-3 do for a called one.
    let first = || {
        user_state
            .move_slots()
            .iter()
            .position(|slot| slot.move_id == source)
    };
    let slot = match rules.conditions.pp.moves_slot {
        MovesSlot::TheLastSelected => Some(
            ctx.battle_state()
                .side(user_state.side)
                .last_selected_move_slot,
        )
        .filter(|slot| *slot < user_state.move_slots().len()),
        MovesSlot::TheChosen => ctx
            .battle_state()
            .acting_slot
            .filter(|(actor, index)| {
                *actor == user
                    && user_state
                        .move_slots()
                        .get(*index)
                        .is_some_and(|slot| slot.move_id == source)
            })
            .map(|(_, index)| index)
            .or_else(first),
        MovesSlot::TheFirstHoldingIt => first(),
    }
    .ok_or(ConditionApplyError::NothingToDo)?;
    // Gen 1-4: the copy's max PP has the slot's three PP Ups (Showdown `calculatePP(move,
    // ppUps[slot])`; pokeplatinum `MoveTable_CalcMaxPP(move, ppUps)`, which a Leppa Berry
    // restores up to, R-M8-109); Gen 5+ the move's own PP.
    let (pp, max_pp) = if keep_pp {
        // Gen 1: the slot keeps its PP.
        (
            user_state.move_slots()[slot].pp,
            crate::dex::pp_with_ups(rules.generation, copied_def),
        )
    } else if cap {
        (
            copied_def.max_pp.min(5),
            crate::dex::pp_with_ups(rules.generation, copied_def),
        )
    } else {
        (copied_def.max_pp, copied_def.max_pp)
    };
    Ok((
        slot,
        MoveSlotState {
            move_id: copied,
            pp,
            max_pp,
            used: false,
            target: copied_pattern.filter(|pattern| *pattern != copied_def.target),
        },
    ))
}

/// Mimic (`onHit`, DESIGN.md §42.5): the copy is temporary.
pub fn mimic(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let (slot, entry) = copyable(ctx, user, target, true)?;
    let drawn_from = ctx
        .battle_rules()
        .conditions
        .calls
        .mimic_draws_from_the_slots
        .then_some(target);
    ctx.write_mimicked_slot(user, slot, entry, drawn_from);
    Ok(())
}

/// Sketch (`onHit`): permanent, and Gen 2 always fails in a link battle (Showdown's gen2
/// `-nothing`; the engine models link battles).
pub fn sketch(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    if ctx.battle_rules().conditions.calls.sketch_always_fails {
        return Err(ConditionApplyError::NothingToDo);
    }
    let (slot, entry) = copyable(ctx, user, target, false)?;
    ctx.write_learned_slot(user, slot, entry);
    Ok(())
}
