//! The focus items outside a move's hits (DESIGN.md R196): the move's own hits are the
//! `MovePipeline`'s (`HoldsForTheMove`), Focus Band and Gen 5+ Focus Sash also answer a
//! `DamageRequest` (`damage_resolution`).

use crate::ids::PokemonId;

use super::BattleContext;

impl<'a> BattleContext<'a> {
    /// Gen 4 Focus Sash and the confusion self-hit (issues.md I39: pokeplatinum
    /// `subscript_hurt_self_in_confusion` → `CheckHoldOnWith1HP`, then the follow-up message's
    /// `RemoveItem`): a holder at full HP whose sash is Gen 4's (`HoldsForTheMove::FromFullHp`) is
    /// left at 1 and the sash is used up. `amount` as it now lands.
    pub(crate) fn focus_sash_holds_self_hit(&mut self, holder: PokemonId, amount: u16) -> u16 {
        let full_and_lethal = self
            .state
            .pokemon(holder)
            .is_some_and(|pokemon| pokemon.hp == pokemon.max_hp && amount >= pokemon.hp);
        if full_and_lethal
            && crate::engine::held_item(self.state, self.rules, holder).is_some_and(
                |(_, behavior)| {
                    behavior.holds_for_the_move
                        == Some(crate::effects::items::HoldsForTheMove::FromFullHp)
                },
            )
            && let Some(item) = self.consume_item(holder)
        {
            let held = self
                .state
                .pokemon(holder)
                .map_or(amount, |pokemon| pokemon.hp - 1);
            // Traced as every other hold is (the replay reads a sash's hold off it).
            self.trace.push(crate::engine::TraceEvent::DamageReduced {
                target: holder,
                requested: amount,
                applied: held,
                by: crate::engine::EffectSource::Item {
                    pokemon: holder,
                    item,
                },
            });
            held
        } else {
            amount
        }
    }
}
