//! Showdown `setStatus` and a sleep of a set length, for Rest (DESIGN.md §20.2, R170).

use crate::engine::StatusApplyResult;
use crate::ids::PokemonId;
use crate::state::NonVolatileStatus;

use super::{BattleContext, EffectSource};

impl<'a> BattleContext<'a> {
    /// Showdown `setStatus`: `status` replaces whatever status `target` has, through every
    /// check `try_set_status` makes.
    pub fn replace_status(
        &mut self,
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
    ) -> StatusApplyResult {
        crate::engine::status::replace_status(
            self.state, self.rules, self.trace, target, status, source,
        )
    }

    /// `target`'s sleep lasts `turns` from now (Showdown `statusState.time` and `startTime`, which
    /// Rest sets over the duration its sleep rolled).
    pub fn set_sleep_turns(&mut self, target: PokemonId, turns: u8) {
        if let Some(status) = self
            .state
            .pokemon_mut(target)
            .and_then(|pokemon| pokemon.status.as_mut())
            .filter(|status| status.kind == NonVolatileStatus::Sleep)
        {
            status.sleep_turns_remaining = Some(turns);
            status.sleep_turns_rolled = Some(turns);
        }
    }
}
