//! The residual of Future Sight and Doom Desire (DESIGN.md §15.22): the slot condition counts
//! down at its place in the residual (`ResidualKind::FutureAttack`) and, at its last count,
//! strikes whoever stands in the slot.

use crate::ids::{ItemId, PokemonId, SideId};
use crate::state::{FutureAttack, VolatileConditionKind};

use super::BattleEngine;
use super::residual::ResidualRun;
use crate::engine::{BattleContext, MovePipeline, TraceEvent};

impl BattleEngine {
    /// The side and index of the attack foreseen on `holder`'s slot, if any.
    pub(super) fn future_attack_slot(&self, holder: PokemonId) -> Option<(SideId, usize)> {
        let position = self.state.active_position(holder, &self.rules.topology)?;
        let spec = self.rules.topology.spec(position)?;
        let slot = usize::from(spec.slot);
        let index = self
            .state
            .side(spec.side)
            .future_attacks
            .iter()
            .position(|attack| attack.slot == slot)?;
        Some((spec.side, index))
    }

    /// Gen 3 on the cartridge (`FutureAttackRules::battler_order`, issues.md I111): `holder`'s
    /// battler number — the player's left 0, the foe's left 1, the player's right 2, the foe's
    /// right 3 — by which the strikes go.
    pub(super) fn future_attack_battler(&self, holder: PokemonId) -> Option<u32> {
        if !self.rules.conditions.future_attack.battler_order {
            return None;
        }
        let position = self.state.active_position(holder, &self.rules.topology)?;
        let spec = self.rules.topology.spec(position)?;
        Some(u32::from(spec.slot) * 2 + u32::from(spec.side == SideId::P2))
    }

    /// Every attack foreseen on a slot with no standing Pokémon: one count off (Showdown's slot
    /// conditions run for a fainted occupant too); the last strikes nobody.
    pub(super) fn tick_unattended_future_attacks(&mut self) {
        for side in [SideId::P1, SideId::P2] {
            let unattended: Vec<usize> = self
                .state
                .side(side)
                .future_attacks
                .iter()
                .enumerate()
                .filter(|(_, attack)| {
                    self.state
                        .side(side)
                        .active
                        .get(attack.slot)
                        .copied()
                        .flatten()
                        .and_then(|id| self.state.pokemon(id))
                        .is_none_or(|pokemon| pokemon.is_fainted())
                })
                .map(|(index, _)| index)
                .collect();
            let attacks = &mut self.state.side_mut(side).future_attacks;
            for index in unattended.into_iter().rev() {
                attacks[index].turns_remaining = attacks[index].turns_remaining.saturating_sub(1);
                if attacks[index].turns_remaining == 0 {
                    attacks.remove(index);
                }
            }
        }
    }

    /// The residual of the attack foreseen on `holder`'s slot: one count off; at the last the
    /// condition ends (`removeSlotCondition` → `onEnd`) and the attack strikes `holder`, then
    /// (Gen 5+) the user's Life Orb takes its toll if the user is on the field. The strike's hit
    /// loop settles its faints (`hitStepMoveHitLoop`'s `faintMessages`, in every generation:
    /// `trySpreadMoveHit`), so the handler is followed by them whatever ends it.
    pub(super) fn run_future_attack(&mut self, holder: PokemonId) -> ResidualRun {
        let Some((side, index)) = self.future_attack_slot(holder) else {
            return ResidualRun::Ran;
        };
        let attacks = &mut self.state.side_mut(side).future_attacks;
        attacks[index].turns_remaining = attacks[index].turns_remaining.saturating_sub(1);
        if attacks[index].turns_remaining > 0 {
            return ResidualRun::Ran;
        }
        let attack = attacks.remove(index);
        // Showdown's `onEnd` also spares the attack's own user standing in the slot, which only
        // Ally Switch could bring about (not implemented).
        self.strike(attack, holder);
        ResidualRun::Ran
    }

    /// The condition's `onEnd` on a standing `target`: `-end`, its Protect and Endure go
    /// (`removeVolatile`), the strike, then the user's Life Orb (Gen 5+, on the field).
    fn strike(&mut self, attack: FutureAttack, target: PokemonId) {
        self.trace.push(TraceEvent::FutureAttackStruck {
            source: attack.source,
            target,
            move_id: attack.move_id,
        });
        {
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            ctx.end_plain_protect(target);
            ctx.end_volatile(target, VolatileConditionKind::Endure);
        }
        MovePipeline::new(&mut self.state, &self.rules, &self.dex, &mut self.trace)
            .strike_future_attack(attack, target);
        if !self.rules.conditions.future_attack.life_orb_after
            || self
                .state
                .active_position(attack.source, &self.rules.topology)
                .is_none()
        {
            return;
        }
        let Some((ItemId::LIFE_ORB, behavior)) =
            crate::engine::held_item(&self.state, &self.rules, attack.source)
        else {
            return;
        };
        let (Some(after), Some(strike)) = (
            behavior.after_move_self,
            self.dex.move_by_id(attack.move_id).cloned(),
        ) else {
            return;
        };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        after(&mut ctx, attack.source, &strike);
    }
}
