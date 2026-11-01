//! The reactions to a hit that roll a chance after the ordered ones (Showdown `DamagingHit`
//! handlers without an order, DESIGN.md §21): the holder's Effect Spore, Cursed Body or Cute
//! Charm, then the attacker's Poison Touch or Toxic Chain (`engine::reactions` says when each
//! rolls). They roll even when an ordered reaction (Rough Skin, Rocky Helmet) just knocked the
//! attacker out — Showdown marks it fainted only at `faintMessages` — and the effect then finds
//! nobody to take it; on a holder the hit knocked out they still roll and act. Which of them
//! roll is settled before the ordered ones run (`engine::ChanceReactors`).

use crate::dex::ResolvedMove;
use crate::ids::{AbilityId, PokemonId};
use crate::rules::EffectSporeRoll;
use crate::state::RandomRequestKind;

use super::MovePipeline;
use crate::engine::BattleContext;

impl<'a> MovePipeline<'a> {
    pub(super) fn run_chance_reactions(
        &mut self,
        attacker: PokemonId,
        holder: PokemonId,
        move_def: &ResolvedMove,
        damage: u16,
        reactors: crate::engine::ChanceReactors,
    ) {
        let chance = crate::engine::HIT_REACTION_CHANCE;
        if let Some(roll) = crate::engine::effect_spore_roll(
            self.state, self.rules, reactors, attacker, move_def, damage,
        ) {
            let status = match roll {
                EffectSporeRoll::ChanceThenSample(chance) => self
                    .state
                    .rng
                    .roll_ratio_for(
                        RandomRequestKind::HitReaction,
                        chance.numerator,
                        chance.denominator,
                    )
                    .then(|| {
                        let index =
                            self.state
                                .rng
                                .range_inclusive_for(RandomRequestKind::Sample, 0, 2);
                        crate::engine::EFFECT_SPORE_SAMPLE[usize::from(index)]
                    }),
                EffectSporeRoll::Percent => crate::engine::effect_spore_status(
                    self.state
                        .rng
                        .range_inclusive_for(RandomRequestKind::EffectSpore, 0, 99),
                ),
            };
            if let Some(status) = status {
                BattleContext::new(self.state, self.rules, self.trace).inflict_status_by_ability(
                    holder,
                    attacker,
                    status,
                    AbilityId::EFFECT_SPORE,
                );
            }
        }
        if crate::engine::cursed_body_rolls(self.state, reactors, attacker, move_def)
            && self.state.rng.roll_ratio_for(
                RandomRequestKind::HitReaction,
                chance.numerator,
                chance.denominator,
            )
        {
            // With the dex: Disable reads it for a last move the attacker does not hold (a
            // Copycat's Bounce, Gen 5+) and for a Z- or Max Move (R-M8-158).
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace).with_dex(self.dex);
            if ctx.set_disable_on_mover(attacker, !self.external).is_ok() {
                ctx.ability_activated(holder, AbilityId::CURSED_BODY);
            }
        }
        let contact_chance = self.rules.status.lifecycle.contact_status_chance;
        if crate::engine::cute_charm_rolls(
            self.state, self.rules, reactors, attacker, move_def, damage,
        ) && self.state.rng.roll_ratio_for(
            RandomRequestKind::HitReaction,
            contact_chance.numerator,
            contact_chance.denominator,
        ) {
            let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
            if ctx.set_attract(attacker, holder).is_ok() {
                ctx.ability_activated(holder, AbilityId::CUTE_CHARM);
            }
        }
        if let Some((ability, status)) =
            crate::engine::dealt_hit_status(self.state, self.rules, reactors, holder, move_def)
            && self.state.rng.roll_ratio_for(
                RandomRequestKind::HitReaction,
                chance.numerator,
                chance.denominator,
            )
        {
            BattleContext::new(self.state, self.rules, self.trace)
                .inflict_status_by_ability(attacker, holder, status, ability);
        }
    }
}
