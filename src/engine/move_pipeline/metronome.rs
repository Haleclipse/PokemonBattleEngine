//! The Metronome item's count (DESIGN.md R126; issues.md I21): the holder's `metronome`
//! condition (`VolatileCondition::Metronome`, started with the item) counts the moves it uses
//! in a row, as the generation's `DamageRules::metronome` says. Gen 5+ `onTryMove` ends the
//! condition on a Pokémon that does not hold the item (Baton Pass hands it on, R142); Gen 4's
//! count moved to `onModifyMove` (I21) and keeps it.

use crate::engine::BattleContext;
use crate::ids::{MoveId, PokemonId};
use crate::rules::MetronomeRules;
use crate::state::{MoveResult, VolatileConditionKind};

use super::{HitOutcome, MoveExecution, MovePipeline};

impl<'a> MovePipeline<'a> {
    /// Gen 4 (`BattleSystem_UpdateMetronomeCount`, as the move starts, its PP spent): the chosen
    /// move is one more in a row, up to the cap, when it is the one counted before; any other
    /// move starts over at 0. The attacking turn of a two-turn move leaves the count alone
    /// (`SYSCTL_LAST_OF_MULTI_TURN`): its charging turn counted. While the item is set aside
    /// (Klutz, Embargo: `Battler_HeldItemEffect` names none) the count drops to 0, the counted
    /// move kept.
    pub(super) fn metronome_starts(&mut self, actor: PokemonId, move_id: MoveId, releasing: bool) {
        let MetronomeRules::Cartridge { cap } = self.rules.damage.metronome else {
            return;
        };
        let holds = crate::engine::held_item(self.state, self.rules, actor)
            .is_some_and(|(_, item)| item.condition == Some(VolatileConditionKind::Metronome));
        let Some(pokemon) = self.state.pokemon_mut(actor) else {
            return;
        };
        let Some((last, count)) = pokemon.volatiles.metronome() else {
            return;
        };
        if !holds {
            pokemon.volatiles.set_metronome(last, 0);
            return;
        }
        if releasing {
            return;
        }
        let count = if last == Some(move_id) {
            count.saturating_add(1).min(cap)
        } else {
            0
        };
        pokemon.volatiles.set_metronome(Some(move_id), count);
    }

    /// Gen 5+ (Showdown's `onTryMove`, once a two-turn move's own `onTryMove` let it through):
    /// a Pokémon without the item loses the condition; otherwise one more when the move is the
    /// one counted before and the holder's move last turn succeeded; the attacking turn of a
    /// two-turn move (`twoturnmove`) counts from 1, or on from the count when it is the counted
    /// move; anything else starts over at 0.
    pub(super) fn metronome_tries(&mut self, actor: PokemonId, move_id: MoveId, releasing: bool) {
        let MetronomeRules::Showdown { .. } = self.rules.damage.metronome else {
            return;
        };
        // `if (move.callsMove) return;` (after the item check): a caller leaves the count alone,
        // and the move it calls is counted against the one before.
        let calls_move = self
            .dex
            .move_by_id(move_id)
            .is_some_and(|move_def| move_def.calls_move);
        let Some(pokemon) = self.state.pokemon(actor) else {
            return;
        };
        let holds = crate::engine::held_item(self.state, self.rules, actor)
            .is_some_and(|(_, item)| item.condition == Some(VolatileConditionKind::Metronome));
        if pokemon.volatiles.metronome().is_some() && !holds {
            BattleContext::new(self.state, self.rules, self.trace)
                .end_volatile(actor, VolatileConditionKind::Metronome);
            return;
        }
        if calls_move {
            return;
        }
        let Some(pokemon) = self.state.pokemon_mut(actor) else {
            return;
        };
        let Some((last, count)) = pokemon.volatiles.metronome() else {
            return;
        };
        let same = last == Some(move_id);
        let count = if same && pokemon.history.move_last_turn == Some(MoveResult::Succeeded) {
            count.saturating_add(1)
        } else if releasing {
            if same { count.saturating_add(1) } else { 1 }
        } else {
            0
        };
        pokemon.volatiles.set_metronome(Some(move_id), count);
    }

    /// Gen 4 (`BattleSystem_VerifyMetronomeCount`, as the action ends): when the chosen move is
    /// the counted one and did no work — it missed, failed, met an immunity or a Protect
    /// (`MOVE_STATUS_NO_EFFECTS`), or the turn was lost before it (`MOVE_STATUS_NO_MORE_WORK`) —
    /// the count takes one step back. The attacking turn of a two-turn move is left alone.
    pub(super) fn metronome_takes_back(&mut self, actor: PokemonId, move_id: MoveId) {
        let MetronomeRules::Cartridge { .. } = self.rules.damage.metronome else {
            return;
        };
        let Some(pokemon) = self.state.pokemon_mut(actor) else {
            return;
        };
        if let Some((Some(last), count)) = pokemon.volatiles.metronome()
            && last == move_id
            && count > 0
        {
            pokemon.volatiles.set_metronome(Some(last), count - 1);
        }
    }
}

/// Whether a move that ran did no work, as the cartridge flags it (`MOVE_STATUS_NO_EFFECTS`): it
/// failed, or every target it reached protected itself.
pub(super) fn did_nothing(execution: &MoveExecution) -> bool {
    execution.result == MoveResult::Failed
        || (!execution.hits.is_empty()
            && execution.hits.iter().all(|hit| {
                matches!(
                    hit,
                    HitOutcome::Blocked {
                        reason: crate::engine::HitBlockReason::Protect,
                        ..
                    }
                )
            }))
}
