//! The abilities that change another's ability (DESIGN.md §41.3, K.4): on a contact hit, Mummy
//! and Lingering Aroma give the attacker the holder's ability and Wandering Spirit trades with
//! it; when an ally faints, Receiver and Power of Alchemy take its ability
//! (`BattleContext::set_ability` / `skill_swap`).

use crate::engine::{BattleContext, DamagingHit};
use crate::ids::{AbilityId, PokemonId};
use crate::state::FaintCause;

/// Mummy (`onDamagingHit`): a contact move's user whose ability can be suppressed and is not
/// Mummy already gets Mummy.
pub fn mummy(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    hand_over(ctx, hit, AbilityId::MUMMY);
}

/// Lingering Aroma (`onDamagingHit`): Mummy's, with Lingering Aroma.
pub fn lingering_aroma(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    hand_over(ctx, hit, AbilityId::LINGERING_AROMA);
}

fn hand_over(ctx: &mut BattleContext<'_>, hit: DamagingHit, ability: AbilityId) {
    let Some(theirs) = ctx.raw_ability_of(hit.attacker) else {
        return;
    };
    if ctx.rules().conditions().cant_suppress.contains(&theirs) || theirs == ability {
        return;
    }
    // `checkMoveMakesContact(move, source, target, !source.isAlly(target))`: Protective Pads
    // keeping a foe's move from making contact name the holder's ability and themselves
    // (R-M10-51); a partner's go unnamed.
    if !hit.contact {
        if ctx.side_of(hit.attacker) != ctx.side_of(hit.holder) {
            ctx.padded_contact(hit, ability);
        }
        return;
    }
    let _ = ctx.set_ability(
        hit.attacker,
        ability,
        Some(hit.holder),
        Some((hit.holder, ability)),
    );
}

/// Wandering Spirit (`onDamagingHit`): a contact move's user trades abilities with the holder
/// (`skillSwap(source, target)`: the attacker is the swap's source).
pub fn wandering_spirit(ctx: &mut BattleContext<'_>, hit: DamagingHit) {
    if hit.contact {
        let _ = ctx.skill_swap(hit.attacker, hit.holder);
    }
}

/// Receiver, Power of Alchemy (`onAllyFaint`): a standing holder takes the fainted ally's
/// ability unless it is `noreceiver`.
pub fn receiver(ctx: &mut BattleContext<'_>, holder: PokemonId, cause: FaintCause) {
    if ctx.is_fainted(holder) {
        return;
    }
    let Some(ability) = ctx.raw_ability_of(cause.target) else {
        return;
    };
    if ctx.rules().conditions().no_receiver.contains(&ability) {
        return;
    }
    let _ = ctx.set_ability(holder, ability, Some(cause.target), None);
}
