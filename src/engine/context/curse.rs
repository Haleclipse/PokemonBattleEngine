//! Curse on its target (Showdown `curse`'s `onTryHit`, `onHit` and condition; DESIGN.md §15.16).

use crate::engine::{ConditionApplyError, EffectSource};
use crate::ids::{MoveId, PokemonId};
use crate::state::VolatileConditionKind;

use super::BattleContext;

impl<'a> BattleContext<'a> {
    /// A Ghost type's Curse: refused when the target is cursed already (`-fail`); otherwise the
    /// target is cursed (`-start|target|Curse|[of] user`) and the user pays half its max HP at
    /// once (`directDamage`: at least 1, and it may faint from it).
    pub fn curse(&mut self, user: PokemonId, target: PokemonId) -> Result<(), ConditionApplyError> {
        if self
            .state
            .pokemon(target)
            .is_none_or(|pokemon| pokemon.is_fainted())
        {
            return Err(ConditionApplyError::InvalidTarget);
        }
        self.set_marker_volatile(target, VolatileConditionKind::Curse)?;
        let max_hp = self.state.pokemon(user).map_or(0, |p| p.max_hp);
        self.apply_damage(crate::engine::DamageRequest::cost(
            user,
            max_hp / 2,
            EffectSource::Move {
                pokemon: user,
                move_id: MoveId::CURSE,
            },
        ));
        Ok(())
    }
}
