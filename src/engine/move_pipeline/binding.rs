//! Gen 1 Bind / Wrap / Clamp / Fire Spin (DESIGN.md §30.9, the gen1 mod's
//! `partialtrappinglock`): the first hit locks the user into the move for 2-5 turns; each
//! later turn it hits the caught Pokémon again, without PP, accuracy or a new damage roll.

use crate::dex::ResolvedMove;
use crate::ids::{MoveId, MoveSlot, PokemonId};
use crate::state::VolatileConditionKind;

use super::{HitOutcome, MovePipeline};
use crate::engine::{BattleContext, TraceEvent};

impl<'a> MovePipeline<'a> {
    /// The move a locked user uses, whatever its action named (Showdown `chooseMove`: the
    /// `lockedMove`), and its slot.
    pub(super) fn binding_locked_move(
        &self,
        actor: PokemonId,
        slot: MoveSlot,
        move_id: MoveId,
    ) -> (MoveSlot, MoveId) {
        match crate::engine::binding_locked_move(self.state, actor) {
            Some(locked) if locked != move_id => {
                let locked_slot = self
                    .state
                    .pokemon(actor)
                    .and_then(|pokemon| {
                        pokemon
                            .move_slots()
                            .iter()
                            .position(|m| m.move_id == locked)
                    })
                    .and_then(|index| MoveSlot::new(index as u8));
                (locked_slot.unwrap_or(slot), locked)
            }
            _ => (slot, move_id),
        }
    }

    /// The lock's `onBeforeMove`: aimed at anyone but the Pokémon it caught (that one switched
    /// out), the lock ends and the move is a fresh one. Returns whether the lock goes on.
    pub(super) fn binding_continues(
        &mut self,
        actor: PokemonId,
        target: Option<PokemonId>,
    ) -> bool {
        let Some((locked, ..)) = self
            .state
            .pokemon(actor)
            .and_then(|pokemon| pokemon.volatiles.partial_trapping_lock())
        else {
            return false;
        };
        if target == Some(locked) {
            return true;
        }
        BattleContext::new(self.state, self.rules, self.trace)
            .end_volatile(actor, VolatileConditionKind::PartialTrappingLock);
        false
    }

    /// The gen1 mod's `onTryMove`: a binding move ends its target's recharge turn, hit or miss.
    pub(super) fn binding_ends_recharge(
        &mut self,
        move_def: &ResolvedMove,
        target: Option<PokemonId>,
    ) {
        if !move_def.behavior.binding_lock {
            return;
        }
        if let Some(target) = target
            && self
                .state
                .pokemon_mut(target)
                .is_some_and(|p| p.volatiles.remove(VolatileConditionKind::MustRecharge))
        {
            self.trace.push(TraceEvent::VolatileEnded {
                target,
                kind: VolatileConditionKind::MustRecharge,
                seen: true,
            });
        }
    }

    /// After the hit: a user not locked yet locks onto the Pokémon it reached (body or
    /// Substitute: `doSelf || partialtrappinglock`) with the damage dealt; then the lock's
    /// `onAfterMove`.
    pub(super) fn binding_after_move(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        target: Option<PokemonId>,
        hits: &[HitOutcome],
    ) {
        if !move_def.behavior.binding_lock || !self.rules.conditions.switching.binding_locks_user {
            return;
        }
        // The lock repeats the battle's last damage (gen1 `partialtrappinglock`: `this.lastDamage`;
        // pokered's `wDamage`): a Substitute's hit leaves it uncapped by the doll's HP, even the
        // hit that broke it (R-M8-143).
        let dealt = hits
            .iter()
            .find_map(|hit| match hit {
                HitOutcome::Damaged { amount, .. } => Some(amount.0),
                HitOutcome::SubstituteDamaged { amount, .. } => Some(amount.0),
                _ => None,
            })
            .map(|dealt| {
                if self.rules.conditions.retaliation.gen1_last_damage {
                    self.state.last_damage
                } else {
                    dealt
                }
            });
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        if let (Some(target), Some(dealt)) = (target, dealt) {
            ctx.start_binding_lock(actor, target, move_def.id, dealt);
        }
        if let Some(target) = target {
            ctx.binding_lock_after_move(actor, target);
        }
    }
}
