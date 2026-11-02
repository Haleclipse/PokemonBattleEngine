//! What Anticipation and Forewarn read of their holder's foes' moves as it comes in (DESIGN.md
//! R275). They read the dex's own facts of a move — its category, type, base power and `ohko`
//! — which come from the generation's snapshot: a switch-in runs without a resolved dex at hand.
//! Shared by the engine and the replay driver.

use crate::dex::PokemonType;
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedRules;
use crate::state::BattleState;

fn move_facts(
    rules: &ResolvedRules,
    move_id: MoveId,
) -> Option<&'static crate::data::MoveSnapshot> {
    crate::data::snapshot(rules.generation)
        .moves
        .iter()
        .find(|row| row.eid == move_id.0)
}

fn ohko(row: &crate::data::MoveSnapshot) -> bool {
    row.other
        .get("ohko")
        .is_some_and(|value| !value.is_null() && value != &serde_json::Value::Bool(false))
}

/// The foes on the field (`pokemon.foes()`: active, not fainted), in position order.
fn foes(state: &BattleState, holder: PokemonId) -> Vec<PokemonId> {
    let Some(side) = state.pokemon(holder).map(|p| p.side) else {
        return Vec::new();
    };
    state
        .side(side.other())
        .active
        .iter()
        .flatten()
        .copied()
        .filter(|foe| state.pokemon(*foe).is_some_and(|p| !p.is_fainted()))
        .collect()
}

/// Anticipation (`onStart`): a foe knows a damaging move whose type the holder's types do not
/// shrug off and are weak to on the chart (`getImmunity && getEffectiveness > 0`), or a one-hit
/// KO move. Hidden Power counts by its user's type from Gen 6
/// (`ConditionRules::anticipation_reads_hidden_power_type`).
pub fn anticipates(state: &BattleState, rules: &ResolvedRules, holder: PokemonId) -> bool {
    let Some(pokemon) = state.pokemon(holder) else {
        return false;
    };
    let types = crate::engine::types_of(state, rules, pokemon);
    foes(state, holder).into_iter().any(|foe| {
        let Some(user) = state.pokemon(foe) else {
            return false;
        };
        user.move_slots().iter().any(|slot| {
            let Some(row) = move_facts(rules, slot.move_id) else {
                return false;
            };
            if row.category == "Status" {
                return false;
            }
            let move_type = if slot.move_id == MoveId::HIDDEN_POWER
                && rules.conditions.anticipation_reads_hidden_power_type
            {
                user.hidden_power_of().0
            } else {
                PokemonType::from_name(&row.type_name).unwrap_or(PokemonType::Typeless)
            };
            let effectiveness = rules
                .type_chart
                .combined_effectiveness(move_type, types.iter());
            (!effectiveness.is_zero() && effectiveness.numerator > effectiveness.denominator)
                || ohko(row)
        })
    })
}

/// Forewarn (`onStart`): every foe's move at the highest warning power — its base power, a
/// one-hit KO move's `ConditionRules::forewarn_ohko_power`, Counter / Metal Burst / Mirror Coat
/// 120, a move of base power 1 or a damaging move of none 80 — with its foe, in position then
/// move-slot order;
/// one is drawn (`sample`). Nothing above a power of 1: no warning.
pub fn forewarn_candidates(
    state: &BattleState,
    rules: &ResolvedRules,
    holder: PokemonId,
) -> Vec<(MoveId, PokemonId)> {
    let mut best = 1u16;
    let mut warned = Vec::new();
    for foe in foes(state, holder) {
        let Some(user) = state.pokemon(foe) else {
            continue;
        };
        for slot in user.move_slots() {
            let Some(row) = move_facts(rules, slot.move_id) else {
                continue;
            };
            let mut power = row.bp;
            if ohko(row) {
                power = rules.conditions.forewarn_ohko_power;
            }
            if matches!(
                slot.move_id,
                MoveId::COUNTER | MoveId::METAL_BURST | MoveId::MIRROR_COAT
            ) {
                power = 120;
            }
            // A base power of 1 (Gen 7's Z-Moves in a move slot) warns as 80 too (R-M11-12).
            if power == 1 {
                power = 80;
            }
            if power == 0 && row.category != "Status" {
                power = 80;
            }
            if power > best {
                best = power;
                warned = vec![(slot.move_id, foe)];
            } else if power == best {
                warned.push((slot.move_id, foe));
            }
        }
    }
    warned
}
