//! The moves that make their user use another move (DESIGN.md §42.1): Metronome, Assist,
//! Sleep Talk, Copycat, Nature Power and Mirror Move. Each draws or looks up one move and asks
//! the Shell for it through `BattleContext::call_move`; what the called move then does — the
//! hit gate, the secondaries, the faints — is the Shell's usual work.

use crate::engine::{BattleContext, ConditionApplyError};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::{CallSource, CallTarget};

/// Metronome (`onHit`): one of the generation's `metronome` moves, drawn (`sample`) from
/// `engine::metronome_candidates` — the user's own moves left out in Gen 2 and Gen 4. It fails
/// (`-fail`) with nothing to draw, which no generation reaches.
pub fn metronome(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let dex = ctx.dex().ok_or(ConditionApplyError::NothingToDo)?;
    let candidates =
        crate::engine::metronome_candidates(ctx.battle_state(), ctx.battle_rules(), dex, user);
    if candidates.is_empty() {
        return Err(ConditionApplyError::NothingToDo);
    }
    let index = rand_index(ctx, candidates.len());
    ctx.select_the_called_move(user, candidates[index]);
    ctx.call_move(
        user,
        candidates[index],
        CallTarget::Random,
        CallSource::Move(MoveId::METRONOME),
    );
    Ok(())
}

/// Assist (`onHit`): a move slot of a party member other than the user (Showdown's list, which
/// keeps its order and repeats a move two of them know), drawn (`sample`).
pub fn assist(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let dex = ctx.dex().ok_or(ConditionApplyError::NothingToDo)?;
    let candidates =
        crate::engine::assist_candidates(ctx.battle_state(), ctx.battle_rules(), dex, user);
    if candidates.is_empty() {
        return Err(ConditionApplyError::NothingToDo);
    }
    let index = rand_index(ctx, candidates.len());
    ctx.call_move(
        user,
        candidates[index],
        CallTarget::Random,
        CallSource::Move(MoveId::ASSIST),
    );
    Ok(())
}

/// Sleep Talk (`onHit`): one of the user's own moves without `nosleeptalk` or `charge`, drawn
/// (`sample`). Gen 3 (`CallRules::sleep_talk_names_depleted`): a draw that lands on a move with
/// no PP left is `cant|p|nopp|<move>` — the move is not used.
pub fn sleep_talk(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let dex = ctx.dex().ok_or(ConditionApplyError::NothingToDo)?;
    let Some(user_pokemon) = ctx.battle_state().pokemon(user) else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    let candidates = crate::engine::sleep_talk_candidates(dex, user_pokemon);
    if candidates.is_empty() {
        return Err(ConditionApplyError::NothingToDo);
    }
    let index = rand_index(ctx, candidates.len());
    let move_id = candidates[index];
    if ctx
        .battle_rules()
        .conditions
        .calls
        .sleep_talk_names_depleted
        && ctx
            .battle_state()
            .pokemon(user)
            .and_then(|pokemon| pokemon.move_slot_of(move_id))
            .is_some_and(|slot| slot.pp == 0)
    {
        ctx.moves_without_pp(user, move_id);
        return Err(ConditionApplyError::NothingToDo);
    }
    ctx.call_move(
        user,
        move_id,
        CallTarget::Random,
        CallSource::Move(MoveId::SLEEP_TALK),
    );
    Ok(())
}

/// Sleep Talk's Gen 3-4 `onTryHit` (`CallRules::sleep_talk_fails_locked`): under a Choice lock or
/// an Encore the move fails — Gen 5+ draw and use the move all the same.
pub fn sleep_talk_tries(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
) -> Result<(), ConditionApplyError> {
    if !ctx.battle_rules().conditions.calls.sleep_talk_fails_locked {
        return Ok(());
    }
    let locked = ctx.battle_state().pokemon(user).is_some_and(|pokemon| {
        pokemon.volatiles.choice_locked_move().is_some()
            || pokemon.volatiles.encored_move().is_some()
    });
    if locked {
        return Err(ConditionApplyError::AlreadyActive);
    }
    Ok(())
}

/// Copycat (`onHit`): the last move used in the battle (`BattleState::last_move`), unless it
/// fails its `failcopycat` flag or — Gen 4 — Gravity or a Heal Block stands in its way.
pub fn copycat(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    _targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let dex = ctx.dex().ok_or(ConditionApplyError::NothingToDo)?;
    let Some(move_id) = crate::engine::copycat_move(ctx.battle_state(), dex) else {
        return Err(ConditionApplyError::NothingToDo);
    };
    if ctx
        .battle_rules()
        .conditions
        .calls
        .filters_gravity_and_heal_block
        && crate::engine::blocked_by_the_user(
            ctx.battle_state(),
            ctx.battle_rules(),
            user,
            dex.move_by_id(move_id)
                .ok_or(ConditionApplyError::NothingToDo)?,
        )
    {
        return Err(ConditionApplyError::NothingToDo);
    }
    ctx.call_move(
        user,
        move_id,
        CallTarget::Random,
        CallSource::Move(MoveId::COPYCAT),
    );
    Ok(())
}

/// Nature Power (`onTryHit`): the terrain's move where the generation follows the terrain
/// (Gen 6+), by the generation's table before (Swift Gen 3, Tri Attack Gen 4, Earthquake Gen 5),
/// aimed where the move was aimed.
pub fn nature_power(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Pokemon(target)) = targets.first().copied() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    // Gen 3-5 a fixed move (the generation's table: Swift, Tri Attack, Earthquake), Gen 6+ the
    // terrain's. The fixed moves aim at random (`useMove` without a target); the terrain's at
    // what Nature Power aimed at.
    let move_id = ctx
        .battle_rules()
        .conditions
        .calls
        .nature_power
        .unwrap_or_else(|| crate::engine::nature_power_move(ctx.battle_state()));
    let target =
        if crate::engine::called_move_aims_at_random(ctx.battle_rules(), MoveId::NATURE_POWER) {
            CallTarget::Random
        } else {
            CallTarget::Pokemon(target)
        };
    ctx.call_move(
        user,
        move_id,
        target,
        CallSource::Move(MoveId::NATURE_POWER),
    );
    Ok(())
}

/// Mirror Move (`onHit` Gen 1-4, `onTryHit` Gen 5+): the move the generation's source rule
/// names — the opposite Pokémon's last move, the last move that attacked the user, or the
/// target's, which must carry `mirror`.
pub fn mirror_move(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let dex = ctx.dex().ok_or(ConditionApplyError::NothingToDo)?;
    let target = targets.first().and_then(|target| match target {
        ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
        _ => None,
    });
    let Some((move_id, _)) =
        crate::engine::mirror_move_from(ctx.battle_state(), ctx.battle_rules(), dex, user, target)
    else {
        return Err(ConditionApplyError::NothingToDo);
    };
    // Gen 1-4 aim the copied move at random (`useMove` without a target: in a double battle any
    // foe, not the one that used it — pokeemerald's `GetMoveTarget`, pokeplatinum's
    // `BattleSystem_Defender(…, randomize)` draw one too, DESIGN.md R-M8-80); Gen 5+ take it at
    // the Pokémon the move was aimed at.
    let call_target =
        if crate::engine::called_move_aims_at_random(ctx.battle_rules(), MoveId::MIRROR_MOVE) {
            CallTarget::Random
        } else {
            CallTarget::Pokemon(target.ok_or(ConditionApplyError::InvalidTarget)?)
        };
    ctx.select_the_called_move(user, move_id);
    ctx.call_move(
        user,
        move_id,
        call_target,
        CallSource::Move(MoveId::MIRROR_MOVE),
    );
    Ok(())
}

/// Showdown `this.sample(items)`: the index drawn among `len` candidates.
fn rand_index(ctx: &mut BattleContext<'_>, len: usize) -> usize {
    ctx.sample_index(len)
}
