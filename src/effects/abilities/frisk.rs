//! Frisk (`onStart`, DESIGN.md R194): as the holder enters it names what its foes hold.

use crate::engine::BattleContext;
use crate::ids::PokemonId;

/// Every foe still up that holds an item has it named; Gen 4-5
/// (`ConditionRules::frisk_one_random_foe`) draw one foe (`Sample`, among two or more) and name
/// its item, if it has one — not the foe (`-item||item|[from] ability: Frisk`), which only a
/// single foe standing gives away. The item is named whatever sets it aside.
pub fn switch_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let foes = ctx.active_foes(holder);
    if ctx.rules().conditions().frisk_one_random_foe {
        if !foes.is_empty() {
            let foe = foes[ctx.sample_index(foes.len())];
            ctx.reveal_item(foe, holder, foes.len() == 1);
        }
        return;
    }
    for foe in foes {
        ctx.reveal_item(foe, holder, true);
    }
}
