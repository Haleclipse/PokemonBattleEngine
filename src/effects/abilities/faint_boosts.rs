//! Abilities that raise the holder's stats as Pokémon faint (DESIGN.md §20.2, R128): Moxie,
//! Chilling Neigh, Grim Neigh and Beast Boost once the holder's move has knocked Pokémon out
//! (`onSourceAfterFaint`), Soul-Heart as any Pokémon faints (`onAnyFaint`).

use crate::engine::BattleContext;
use crate::ids::PokemonId;
use crate::state::{FaintCause, Stat};

use super::hit_boosts::boost;

/// Moxie, Chilling Neigh: Attack rises by the number of Pokémon down, when a move did it.
pub fn moxie(ctx: &mut BattleContext<'_>, holder: PokemonId, cause: FaintCause, length: i8) {
    if cause.by_move {
        boost(ctx, holder, &[(Stat::Attack, length)]);
    }
}

/// Grim Neigh: Special Attack rises by the number of Pokémon down, when a move did it.
pub fn grim_neigh(ctx: &mut BattleContext<'_>, holder: PokemonId, cause: FaintCause, length: i8) {
    if cause.by_move {
        boost(ctx, holder, &[(Stat::SpecialAttack, length)]);
    }
}

/// Beast Boost: the holder's best stored stat (`BattleContext::best_stored_stat`) rises by the
/// number of Pokémon down, when a move did it.
pub fn beast_boost(ctx: &mut BattleContext<'_>, holder: PokemonId, cause: FaintCause, length: i8) {
    if cause.by_move {
        let stat = ctx.best_stored_stat(holder);
        boost(ctx, holder, &[(stat, length)]);
    }
}

/// Soul-Heart: +1 Special Attack whenever a Pokémon faints, whoever it is and however it went.
pub fn soul_heart(ctx: &mut BattleContext<'_>, holder: PokemonId, _cause: FaintCause) {
    boost(ctx, holder, &[(Stat::SpecialAttack, 1)]);
}
