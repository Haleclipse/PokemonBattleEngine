//! Heals taken off another Pokémon (DESIGN.md §20.2, R168): a drain, Leech Seed, Strength Sap
//! — Showdown `heal(amount, receiver, drained, effect)`, whose `TryHeal` event also asks the
//! drained Pokémon's `onSourceTryHeal` (Liquid Ooze).

use crate::engine::{HealRequest, HealResult};
use crate::ids::{MoveId, PokemonId};

use super::BattleContext;

/// What a heal off another Pokémon comes from (Showdown's effect ids `drain`, `leechseed`,
/// `strengthsap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealDrain {
    /// A draining move's hit (Giga Drain, Dream Eater …).
    Move(MoveId),
    LeechSeed,
    StrengthSap,
}

/// A heal about to be taken off `drained` for `receiver` (Showdown `onSourceTryHeal`'s
/// arguments).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainedHeal {
    pub receiver: PokemonId,
    pub drained: PokemonId,
    pub amount: u16,
    pub drain: HealDrain,
}

impl<'a> BattleContext<'a> {
    /// Heals `request.target` with HP taken off `drained`. Showdown's `TryHeal` runs before
    /// anything else `heal` checks (the receiver's HP, full or none): the drained Pokémon's
    /// `AbilityBehavior::source_try_heal` (Liquid Ooze) may take the heal over — unless the
    /// receiver's Heal Block goes first and stops it (handlers by speed; at a tie the volatile's
    /// sub-order 2 before the ability's 7). Returns whether the receiver was healed.
    pub fn drain_heal(
        &mut self,
        request: HealRequest,
        drained: PokemonId,
        drain: HealDrain,
    ) -> bool {
        // Big Root first: Liquid Ooze then hurts by the larger heal (pokeplatinum boosts before
        // the ooze check; Bulbapedia's Big Root, every generation — Showdown's Gen 5+ `chainModify`
        // comes too late for it, patched in the oracle: issues.md I199, R-M8-160).
        let request = HealRequest {
            amount: self.big_root(request.target, request.amount),
            ..request
        };
        let heal_block_first = self.heal_refused(request.target, request.source)
            && crate::engine::stats::handler_speed(self.state, request.target)
                >= crate::engine::stats::handler_speed(self.state, drained);
        let handler = self.running_ability(drained).and_then(|ability| {
            crate::engine::ability_behavior(self.rules, ability).source_try_heal
        });
        if !heal_block_first
            && let Some(handler) = handler
            && handler(
                self,
                DrainedHeal {
                    receiver: request.target,
                    drained,
                    amount: request.amount,
                    drain,
                },
            )
        {
            return false;
        }
        matches!(self.apply_heal(request), HealResult::Applied { .. })
    }

    /// Big Root's `onTryHeal` on a heal it boosts (a drain, Leech Seed, Ingrain, Aqua Ring,
    /// Strength Sap — the callers): Gen 4 truncates `amount × 1.3`, later generations apply
    /// the 4096-based 5324.
    pub fn big_root(&self, receiver: PokemonId, amount: u16) -> u16 {
        if !crate::engine::held_item(self.state, self.rules, receiver)
            .is_some_and(|(_, item)| item.boosts_absorbed_heals)
        {
            return amount;
        }
        let boosted = if self.rules.conditions.big_root_truncates {
            u32::from(amount) * 13 / 10
        } else {
            crate::engine::damage::showdown_modify(u32::from(amount), 5324)
        };
        boosted.min(u32::from(u16::MAX)) as u16
    }
}
