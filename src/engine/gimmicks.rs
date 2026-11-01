//! The generation gimmicks at the move level (DESIGN.md §44): what a Z-Crystal makes of a move,
//! and (O.5a) what Dynamax makes of one. The choice layer asks whether a mechanic is available,
//! the queue gives it its own step where Showdown does, and this is what the move action runs.

use crate::dex::{ResolvedMove, ZConversion, ZStatusEffect};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedRules;
use crate::state::BattleState;

use super::move_pipeline::ZMoveExecution;
use super::{BattleError, BattleTrace, MechanicChoice, TraceEvent};

/// A mechanic spent (DESIGN.md §44): the side's use of it is gone and `MechanicActivated` says
/// so. Mega Evolution, Dynamax and Terastallization spend it as their step runs; a Z-Move as its
/// move is used (`MovePipeline::spend_z_power`).
pub(crate) fn spend_mechanic(
    state: &mut BattleState,
    trace: &mut BattleTrace,
    pokemon: PokemonId,
    mechanic: MechanicChoice,
) -> Result<(), BattleError> {
    let side = state
        .pokemon(pokemon)
        .map(|pokemon| pokemon.side)
        .ok_or(BattleError::UnknownPokemon(pokemon))?;
    let side_state = state.side_mut(side);
    let usage = match mechanic {
        MechanicChoice::MegaEvolve => &mut side_state.mechanics.mega,
        MechanicChoice::UltraBurst => &mut side_state.mechanics.ultra_burst,
        MechanicChoice::ZMove => &mut side_state.mechanics.z_move,
        MechanicChoice::Dynamax => &mut side_state.mechanics.dynamax,
        MechanicChoice::Terastallize => &mut side_state.mechanics.tera,
    };
    *usage = crate::state::MechanicUsage::Consumed;
    trace.push(TraceEvent::MechanicActivated {
        side,
        pokemon,
        mechanic,
    });
    Ok(())
}

/// The Z-Move the holder's crystal makes of the move in `slot` (Showdown `getZMove` +
/// `getActiveZMove`, DESIGN.md §44.4): `None` when the holder has no Z-Crystal, the crystal does
/// not apply to this move, the slot has no PP left, or the crystal is another species'.
pub fn z_move_execution(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &crate::dex::ResolvedDex,
    holder: PokemonId,
    base: &ResolvedMove,
) -> Option<ZMoveExecution> {
    let _ = rules;
    let holder_state = state.pokemon(holder)?;
    let item_id = holder_state.held_item?;
    let item = dex.item_by_id(item_id)?;
    item.z_crystal?;
    // Showdown `item.itemUser` (Pikashunium Z lists the cosplay formes alone): the crystal
    // belongs to these species.
    if !item.item_user.is_empty()
        && !holder_state
            .species
            .is_some_and(|species| item.item_user.contains(&species))
    {
        return None;
    }
    match crate::dex::z_conversion(item, base, |move_type| dex.z_move_of_type(move_type))? {
        ZConversion::Status => Some(ZMoveExecution {
            move_def: base.clone(),
            powered: true,
            base: base.id,
            status_effect: base.z_status.clone(),
        }),
        ZConversion::Generic(id) => {
            let z_def = dex.move_by_id(id)?.clone();
            let mut def = z_def;
            def.base_power = base.z_power?;
            def.category = base.category;
            def.priority = base.priority;
            Some(ZMoveExecution {
                move_def: def,
                powered: true,
                base: base.id,
                status_effect: None,
            })
        }
        ZConversion::Signature(id) => {
            let def = dex.move_by_id(id)?.clone();
            Some(ZMoveExecution {
                move_def: def,
                powered: true,
                base: base.id,
                status_effect: None,
            })
        }
    }
}

/// Showdown `getActiveZMove` for a damaging move a Z status move called (`useMoveInner`:
/// `move.category !== 'Status' && sourceEffect.isZ` — Z-Nature Power's Tri Attack, a
/// Z-Metronome's draw): the holder's signature Z-Move when its crystal is made from that move
/// (`item.zMoveFrom`), otherwise the Z-Move of the move's type with the move's Z power, category
/// and priority — no crystal of that type needed (R-M9-41). `None` for a status move or one
/// without a Z power.
pub fn called_z_move(
    state: &BattleState,
    dex: &crate::dex::ResolvedDex,
    holder: PokemonId,
    base: &ResolvedMove,
) -> Option<ResolvedMove> {
    if base.category == crate::dex::MoveCategory::Status {
        return None;
    }
    if let Some(item) = state
        .pokemon(holder)
        .and_then(|pokemon| pokemon.held_item)
        .and_then(|item| dex.item_by_id(item))
        && let Some(Some(signature)) = item.z_crystal
        && item.z_move_from == Some(base.id)
    {
        return dex.move_by_id(signature).cloned();
    }
    let mut def = dex.move_by_id(dex.z_move_of_type(base.move_type)?)?.clone();
    def.base_power = base.z_power?;
    def.category = base.category;
    def.priority = base.priority;
    Some(def)
}

/// The Max Move the holder's Dynamax makes of the move in a slot (Showdown `getMaxMove` +
/// `getActiveMaxMove`, DESIGN.md §44.5): the Max Move of the move's type as it will be used
/// (its own and the ability's type changes have run), the base move's category and priority,
/// the power the engine's table gives (`ResolvedMove::max_power`, the move's data first). `None`
/// when the holder is not Dynamaxed or the move has no Max version.
pub fn max_move_execution(
    state: &BattleState,
    dex: &crate::dex::ResolvedDex,
    holder: PokemonId,
    base: &ResolvedMove,
) -> Option<crate::dex::MaxMove> {
    let dynamaxed = state.pokemon(holder).is_some_and(|pokemon| {
        pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::Dynamax)
    });
    if !dynamaxed {
        return None;
    }
    let move_id = crate::dex::max_move_for(dex, base)?;
    let mut def = dex.move_by_id(move_id)?.clone();
    if base.id != MoveId::STRUGGLE && base.category != crate::dex::MoveCategory::Status {
        // The Max Move takes the base move's power and category (`getActiveMaxMove`; the three
        // G-Max moves with a power of their own are not in the snapshots — no Gigantamax). A
        // status move is Max Guard, which keeps its own.
        def.category = base.category;
        // A Max Move converting a Max Move gives 0 power (Showdown's `getDamage`: "Hacked Max
        // Moves have 0 base power, even if you Dynamax"), so the hit is the minimum.
        def.base_power = if base.is_max { 0 } else { base.max_power? };
    }
    def.priority = base.priority;
    Some(crate::dex::MaxMove {
        move_def: def,
        base: base.id,
    })
}

/// The Z power of a status move (`runZPower`): what the crystal's `zMove.boost` / `zMove.effect`
/// does to its user before the move itself runs.
pub fn run_z_power(
    ctx: &mut crate::engine::BattleContext<'_>,
    holder: PokemonId,
    move_id: MoveId,
    effect: &ZStatusEffect,
) {
    use crate::engine::{EffectSource, HealRequest, StatChangeCause, StatChangeRequest};
    let source = EffectSource::Move {
        pokemon: holder,
        move_id,
    };
    let boost = |ctx: &mut crate::engine::BattleContext<'_>, stat, stages| {
        ctx.try_change_stat_stage(StatChangeRequest {
            target: holder,
            stat,
            stages,
            source,
            cause: StatChangeCause::ZPower,
        });
    };
    let full_heal = |ctx: &mut crate::engine::BattleContext<'_>| {
        let max = ctx.max_hp(holder).unwrap_or(0);
        ctx.apply_heal(HealRequest {
            target: holder,
            amount: max,
            source,
            direct: false,
        });
    };
    match effect {
        ZStatusEffect::Boosts(stages) => {
            for (stat, delta) in stages {
                boost(ctx, *stat, *delta);
            }
        }
        ZStatusEffect::Heal => full_heal(ctx),
        ZStatusEffect::HealReplacement => {
            let _ = ctx.wish_for_the_next(holder, crate::state::SlotHealKind::HealingWish);
        }
        ZStatusEffect::ClearNegativeBoost => {
            let lowered: Vec<crate::state::Stat> = crate::engine::STAGED_STATS
                .into_iter()
                .filter(|stat| ctx.stage(holder, *stat) < 0)
                .collect();
            for stat in lowered {
                ctx.set_stage(holder, stat, 0, source);
            }
        }
        ZStatusEffect::Redirect => {
            let _ = ctx.set_follow_me(holder);
        }
        ZStatusEffect::Crit2 => {
            let _ = ctx.set_focus_energy(holder);
        }
        ZStatusEffect::Curse => {
            let ghost = ctx.has_type(holder, crate::dex::PokemonType::Ghost);
            if ghost {
                full_heal(ctx);
            } else {
                boost(ctx, crate::state::Stat::Attack, 1);
            }
        }
    }
}

/// Whether `holder` may Dynamax right now (Showdown `getDynamaxRequest`, DESIGN.md §44.5): the
/// species is not one the dex forbids (`cannot_dynamax`: Zacian, Zamazenta, Eternatus), it holds
/// no Z-Crystal, and it cannot Mega Evolve (Gen 8 has neither, so the last is always false
/// there); at least one Max Move must be available.
pub fn can_dynamax(state: &BattleState, dex: &crate::dex::ResolvedDex, holder: PokemonId) -> bool {
    let Some(pokemon) = state.pokemon(holder) else {
        return false;
    };
    if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::Dynamax)
    {
        return false;
    }
    // `species.cannotDynamax`: Zacian, Zamazenta, Eternatus. Showdown's other two clauses — a
    // Z-Crystal held and an available Mega Evolution — cannot fire: Gen 8, the one generation
    // with Dynamax, has neither Z-Crystals nor Mega Stones in its dex (DESIGN.md §44.5).
    if dex
        .species()
        .iter()
        .any(|entry| entry.cannot_dynamax && Some(entry.id) == pokemon.species)
    {
        return false;
    }
    pokemon.move_slots().iter().any(|slot| {
        slot.pp > 0
            && dex
                .move_by_id(slot.move_id)
                .is_some_and(|base| max_move_for(dex, base).is_some())
    })
}

/// The Max Move a base move becomes (the dex's table).
pub fn max_move_for(dex: &crate::dex::ResolvedDex, base: &ResolvedMove) -> Option<MoveId> {
    crate::dex::max_move_for(dex, base)
}
