//! The moves that call or copy other moves (DESIGN.md §42): their candidate tables, shared by
//! the effects and the replay, which reads back where a drawn move sits in them.

use crate::dex::{ResolvedDex, ResolvedMove};
use crate::ids::MoveId;
use crate::ids::PokemonId;
use crate::rules::{MirrorMoveSource, ResolvedRules};
use crate::state::{BattleState, FieldConditionKind, VolatileConditionKind};

/// Metronome's candidates (Showdown `onHit`): the generation's moves with the `metronome` flag,
/// in the dex's order (Showdown sorts by num), without the moves the user's own slots hold where
/// the generation says so (Gen 2, Gen 4: `CallRules::metronome_excludes_known`), and — Gen 4 —
/// without a move Gravity grounds or a Heal Block the user is under would block.
pub fn metronome_candidates(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    user: PokemonId,
) -> Vec<MoveId> {
    let known: Vec<MoveId> = if rules.conditions.calls.metronome_excludes_known {
        state
            .pokemon(user)
            .map(|pokemon| {
                pokemon
                    .move_slots()
                    .iter()
                    .map(|slot| slot.move_id)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    dex.moves()
        .iter()
        .filter(|move_def| move_def.flags.callable)
        .filter(|move_def| !known.contains(&move_def.id))
        .filter(|move_def| !blocked_by_the_user(state, rules, user, move_def))
        .map(|move_def| move_def.id)
        .collect()
}

/// Assist's candidates (Showdown `onHit`): every move slot of the user's *other* party members,
/// the fainted included, without `noassist` or a Z-Move in a slot (`move.isZ`, DESIGN.md
/// R-M8-71) — a move two of them know counts twice, as Showdown's list does.
pub fn assist_candidates(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    user: PokemonId,
) -> Vec<MoveId> {
    let Some(pokemon) = state.pokemon(user) else {
        return Vec::new();
    };
    let team = state.side(pokemon.side).team.clone();
    let mut moves = Vec::new();
    for member in team {
        if member == user {
            continue;
        }
        let Some(member) = state.pokemon(member) else {
            continue;
        };
        for slot in member.move_slots() {
            let Some(move_def) = dex.move_by_id(slot.move_id) else {
                continue;
            };
            if move_def.flags.no_assist
                || move_def.z_crystal.is_some()
                || blocked_by_the_user(state, rules, user, move_def)
            {
                continue;
            }
            moves.push(slot.move_id);
        }
    }
    moves
}

/// Sleep Talk's candidates (Showdown `onHit`): the user's own moves, without `nosleeptalk`,
/// without a move that charges (`charge`, Gen 2-4's `charge` flag on two-turn moves), and
/// without a Z-Move or Max Move in a slot (`(move.isZ && move.basePower !== 1) || move.isMax`,
/// DESIGN.md §44.5).
pub fn sleep_talk_candidates(dex: &ResolvedDex, user: &crate::state::PokemonState) -> Vec<MoveId> {
    user.move_slots()
        .iter()
        .filter(|slot| slot.move_id.0 != 0)
        .filter(|slot| {
            dex.move_by_id(slot.move_id).is_some_and(|move_def| {
                !move_def.flags.no_sleep_talk
                    && !move_def.flags.charge
                    && !(move_def.z_crystal.is_some() && move_def.base_power != 1)
                    && !move_def.is_max
            })
        })
        .map(|slot| slot.move_id)
        .collect()
}

/// Copycat's move (Showdown `this.lastMove`): the last move used in the battle, which fails its
/// `failcopycat` flag (Copycat, Metronome, Mimic, Sketch, Me First, Struggle …) and the Z / Max
/// marks (`move.isZ || move.isMax`). A Max Move is copied as the move it was made from
/// (`move.isMax && move.baseMove`, DESIGN.md §44.5). `None` when nothing has been used yet.
pub fn copycat_move(state: &BattleState, dex: &ResolvedDex) -> Option<MoveId> {
    // A status move used as a Z-Move keeps its own id (`battle.lastMove.isZ`, R-M9-36).
    if state.last_move_z {
        return None;
    }
    let move_id = state.last_move_base.or(state.last_move)?;
    let move_def = dex.move_by_id(move_id)?;
    (!move_def.flags.fail_copycat && move_def.z_crystal.is_none() && !move_def.is_max)
        .then_some(move_id)
}

/// Mirror Move's source (Showdown's per-generation `onHit` / `onTryHit`), with the generation
/// tables of moves it refuses.
pub fn mirror_move_from(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    user: PokemonId,
    target: Option<PokemonId>,
) -> Option<(MoveId, Option<PokemonId>)> {
    let user_pokemon = state.pokemon(user)?;
    let active_foe = |state: &BattleState| {
        state
            .all_pokemon()
            .iter()
            .find(|pokemon| {
                pokemon.side != user_pokemon.side
                    && state
                        .active_position(pokemon.id(), &rules.topology)
                        .is_some()
            })
            .map(|pokemon| pokemon.id())
    };
    match rules.conditions.calls.mirror_move {
        // Gen 1: the opposite active Pokémon's last move, unless it is Mirror Move itself.
        MirrorMoveSource::Gen1FoeLastMove => {
            let foe = active_foe(state)?;
            let move_id = state.pokemon(foe)?.last_move?;
            (move_id != MoveId::MIRROR_MOVE).then_some((move_id, Some(foe)))
        }
        // Gen 2: the same, without the calling moves and never one the user already knows.
        MirrorMoveSource::Gen2FoeLastMove => {
            let foe = active_foe(state)?;
            let move_id = state.pokemon(foe)?.last_move?;
            let refused = rules.conditions.calls.mirror_tables.gen2.contains(&move_id)
                || user_pokemon
                    .move_slots()
                    .iter()
                    .any(|slot| slot.move_id == move_id);
            (!refused).then_some((move_id, Some(foe)))
        }
        // Gen 3-4: the last move that attacked the user, its user still knowing it — and still
        // there since: one that left the field (switched out, knocked out) has no last move
        // any more (Showdown's `lastAttackedBy.source.lastMove`; the cartridges clear what the
        // user took from it as it goes: pokeemerald `SwitchInClearSetData`, pokeplatinum
        // `BattleSystem_UpdateAfterSwitch` / `CleanupFaintedMon`, DESIGN.md R-M8-80).
        MirrorMoveSource::LastAttacker => {
            let record = *user_pokemon.history.attacked_by.last()?;
            state.pokemon(record.source)?.last_move?;
            let refuses = rules
                .conditions
                .calls
                .mirror_tables
                .for_generation(rules.generation());
            if refuses.contains(&record.move_id) {
                return None;
            }
            let knows = state
                .pokemon(record.source)?
                .move_slots()
                .iter()
                .any(|slot| slot.move_id == record.move_id);
            knows.then_some((record.move_id, Some(record.source)))
        }
        // Gen 5+: the target's last move, which must carry `mirror`. Showdown's
        // `move.isZ || move.isMax` needs no line: no Z-Move or Max Move in the snapshots carries
        // the `mirror` flag, so the flag alone refuses them (DESIGN.md §44.5).
        MirrorMoveSource::TargetLastMove => {
            let target = target?;
            // A status move used as a Z-Move keeps the `mirror` flag; `isZ` refuses it
            // (R-M9-36).
            if state.pokemon(target)?.last_move_z {
                return None;
            }
            let move_id = state.pokemon(target)?.last_move?;
            let move_def = dex.move_by_id(move_id)?;
            move_def.flags.mirror.then_some((move_id, Some(target)))
        }
    }
}

/// Nature Power's move for the terrain (Showdown's Gen 6+ `onTryHit`: Tri Attack when none is
/// up).
pub fn nature_power_move(state: &BattleState) -> MoveId {
    use crate::state::TerrainKind;
    match state.field.terrain.map(|terrain| terrain.kind) {
        Some(TerrainKind::Electric) => MoveId::THUNDERBOLT,
        Some(TerrainKind::Grassy) => MoveId::ENERGY_BALL,
        Some(TerrainKind::Misty) => MoveId::MOONBLAST,
        Some(TerrainKind::Psychic) => MoveId::PSYCHIC,
        _ => MoveId::TRI_ATTACK,
    }
}

/// Whether the move `caller` calls goes at random (Showdown `useMove` without a target:
/// Metronome, Assist, Copycat, Sleep Talk, Gen 1-4 Mirror Move, Gen 3-5 Nature Power's fixed
/// move) rather than at a target the caller names (Me First, Magic Coat's reflection, Gen 5+
/// Mirror Move, Gen 6+ Nature Power). Shared with the replay, which plans the random target only
/// where one is drawn (R-M8-144).
pub fn called_move_aims_at_random(rules: &ResolvedRules, caller: MoveId) -> bool {
    match caller {
        MoveId::ME_FIRST | MoveId::MAGIC_COAT => false,
        MoveId::MIRROR_MOVE => {
            rules.conditions.calls.mirror_move != MirrorMoveSource::TargetLastMove
        }
        MoveId::NATURE_POWER => rules.conditions.calls.nature_power.is_some(),
        _ => true,
    }
}

/// Whether Gravity grounds this move or a Heal Block on the user blocks it: Showdown filters
/// both out of Metronome, Assist and Copycat's candidates in Gen 4.
pub fn blocked_by_the_user(
    state: &BattleState,
    rules: &ResolvedRules,
    user: PokemonId,
    move_def: &ResolvedMove,
) -> bool {
    if !rules.conditions.calls.filters_gravity_and_heal_block {
        return false;
    }
    if move_def.flags.gravity && state.field.contains(FieldConditionKind::Gravity) {
        return true;
    }
    move_def.flags.heal
        && state
            .pokemon(user)
            .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::HealBlock))
}
