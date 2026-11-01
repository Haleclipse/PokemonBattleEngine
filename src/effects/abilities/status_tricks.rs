//! Abilities around statuses and stages that act on others (DESIGN.md §20.2, R122): Good as
//! Gold refuses other Pokémon's status moves, Curious Medicine resets its partners' stages as
//! it enters, and Poison Puppeteer confuses the Pokémon its moves poison.

use crate::dex::MoveCategory;
use crate::engine::{BattleContext, EffectSource, HitBlockReason, TryHitContext, TryHitResult};
use crate::ids::{AbilityId, PokemonId, SpeciesId};
use crate::state::NonVolatileStatus;

/// Good as Gold (`onTryHit`): another Pokémon's status move does not affect the holder
/// (`-immune`).
pub fn good_as_gold(ctx: &mut TryHitContext<'_>) -> TryHitResult {
    if ctx.move_category() == MoveCategory::Status && ctx.user() != ctx.target() {
        TryHitResult::Blocked(HitBlockReason::Ability(AbilityId::GOOD_AS_GOLD))
    } else {
        TryHitResult::Allow
    }
}

/// Curious Medicine (`onStart`): each adjacent partner's stat stages go back to 0, its line
/// naming the ability whether there were any (`-clearboost|ally|[from] ability: Curious
/// Medicine|[of] holder`, R-M10-40).
pub fn curious_medicine(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let allies = ctx.adjacent_allies(holder);
    if !allies.is_empty() {
        ctx.ability_activated(holder, AbilityId::CURIOUS_MEDICINE);
    }
    for ally in allies {
        ctx.clear_stages(
            ally,
            EffectSource::Ability {
                pokemon: holder,
                ability: AbilityId::CURIOUS_MEDICINE,
            },
        );
    }
}

/// Poison Puppeteer (`onAnyAfterSetStatus`): when the holder — a Pecharunt — poisons another
/// Pokémon with a move, that Pokémon is confused too.
pub fn poison_puppeteer(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
) {
    let by_holders_move = matches!(source, EffectSource::Move { pokemon, .. } if pokemon == holder);
    if by_holders_move
        && target != holder
        && ctx.own_species(holder) == Some(SpeciesId::PECHARUNT)
        && matches!(status, NonVolatileStatus::Poison | NonVolatileStatus::Toxic)
        // The confusion's line names it (`-start|target|confusion|[from] ability: Poison
        // Puppeteer|[of] holder`, R-M10-40).
        && ctx.try_set_confusion(target, holder).is_ok()
    {
        ctx.ability_activated(holder, AbilityId::POISON_PUPPETEER);
    }
}
