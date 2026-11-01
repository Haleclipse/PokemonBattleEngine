//! Synchronize (Showdown `onAfterSetStatus`): a burn, paralysis or poison inflicted by another
//! Pokémon is passed back to it (sleep and freeze are not; Toxic Spikes and self-inflicted
//! statuses have no Pokémon to pass to). Gen 3-4 pass ordinary poison for bad poison
//! (`StatusLifecycleRules::synchronize_passes_toxic`). The pass is a status request: the other
//! side's immunities refuse it like any status.

use crate::engine::{BattleContext, EffectSource};
use crate::ids::{AbilityId, PokemonId};
use crate::state::NonVolatileStatus;

pub fn after_set_status(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
) {
    let Some(culprit) = source.pokemon() else {
        return;
    };
    if culprit == holder {
        return;
    }
    let passed = match status {
        NonVolatileStatus::Sleep | NonVolatileStatus::Freeze => return,
        NonVolatileStatus::Toxic if !ctx.rules().status().lifecycle.synchronize_passes_toxic => {
            NonVolatileStatus::Poison
        }
        other => other,
    };
    // From Gen 5 it announces itself, the status going with an effect carrying it (`{status:
    // status.id, id: 'synchronize'}`), which an ability keeping it off names itself for. The
    // gen4 mod's passes the ability, which the status's line names where it lands
    // (`-status|culprit|par|[from] ability: Synchronize|[of] holder`, R-M10-40).
    if ctx.rules().status().lifecycle.synchronize_announced {
        ctx.ability_activated(holder, AbilityId::SYNCHRONIZE);
        let source = EffectSource::Ability {
            pokemon: holder,
            ability: AbilityId::SYNCHRONIZE,
        };
        ctx.try_set_carried_status(culprit, passed, source);
    } else {
        ctx.inflict_status_by_ability(holder, culprit, passed, AbilityId::SYNCHRONIZE);
    }
}
