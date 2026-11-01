//! The item a Pokémon holds and whose effects apply (DESIGN.md §39.2, R180, R194): the single
//! entry every reading of a holder's item effect goes through, as Showdown's event dispatch
//! checks `ignoringItem()` before running an item's handler. Asking what an item is reads the
//! held item itself: a berry being eaten, the switch-in `Start` a set-aside item still gets,
//! Gen 4 Embargo's Griseous Orb, Frisk.

use crate::effects::items::ItemBehavior;
use crate::ids::{ItemId, PokemonId};
use crate::rules::ResolvedRules;
use crate::state::{BattleState, FieldConditionKind, PokemonState, VolatileConditionKind};

/// `pokemon`'s held item and its behavior, when its effects apply (not `ignoring_item`).
pub fn held_item(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: PokemonId,
) -> Option<(ItemId, &'static ItemBehavior)> {
    held_item_of(state, rules, state.pokemon(pokemon)?)
}

/// `held_item` for a Pokémon already at hand.
pub fn held_item_of(
    state: &BattleState,
    rules: &ResolvedRules,
    holder: &PokemonState,
) -> Option<(ItemId, &'static ItemBehavior)> {
    let item = holder.held_item?;
    let behavior = super::item_behavior(rules, item);
    (!ignoring_item(state, rules, holder, behavior)).then_some((item, behavior))
}

/// Showdown `hasItem(item)`: `pokemon` holds `item` and its effects apply. The held item is
/// compared first, so whether its effects apply — which can ask about the ability (Klutz) — is
/// only asked of that item, as in Showdown: Ability Shield, asked about from the ability's own
/// suppression, never asks back.
pub fn has_item(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    item: ItemId,
) -> bool {
    pokemon.held_item == Some(item) && held_item_of(state, rules, pokemon).is_some()
}

/// Showdown `getItem().isChoice`: the held item, set aside or not, is a Choice item — what a
/// `choicelock` needs to stay.
pub fn holds_choice_item(rules: &ResolvedRules, pokemon: &PokemonState) -> bool {
    pokemon
        .held_item
        .is_some_and(|item| super::item_behavior(rules, item).is_choice)
}

/// The id alone of `held_item`.
pub fn held_item_id(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: PokemonId,
) -> Option<ItemId> {
    held_item(state, rules, pokemon).map(|(item, _)| item)
}

/// Showdown `pokemon.ignoringItem()` asked of the Pokémon itself rather than of one item: a
/// berry it eats goes without its effect (`singleEvent('Eat')`) — its own, forced (Stuff Cheeks,
/// Teatime), or another's (Bug Bite, Pluck). What it holds, if anything, decides Klutz's
/// exception, as Showdown's `getItem().ignoreKlutz` does.
pub(crate) fn ignores_items(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> bool {
    match pokemon.held_item {
        Some(item) => ignoring_item(state, rules, pokemon, super::item_behavior(rules, item)),
        None => ignoring_item(state, rules, pokemon, &ItemBehavior::default()),
    }
}

/// The item `user`'s Fling throws, with its behavior and Fling's power — none when Fling fails
/// (Showdown Fling's `onPrepareHit`): the user sets items aside for Fling (`ignoringItem(true)`:
/// Embargo, Magic Room, and Klutz where `ItemTransferRules::fling_klutz_blocks` says, whatever
/// the item), or the item has no `fling`. Gen 4 also fails for a user with Multitype or holding
/// a Griseous Orb (`ItemTransferRules::fling_refuses_multitype`, issues.md I90). Showdown's
/// check of the item's own `TakeItem` comes with the first item that has both a `fling` and an
/// `onTakeItem` (the Plates …): the ones implemented that refuse (Mail, the Rusted Sword and
/// Shield) cannot be flung anyway.
pub(crate) fn fling_item(
    state: &BattleState,
    rules: &ResolvedRules,
    user: &PokemonState,
) -> Option<(ItemId, &'static ItemBehavior, u16)> {
    let item = user.held_item?;
    let behavior = super::item_behavior(rules, item);
    let transfer = &rules.conditions.items;
    let klutz = || super::active_ability_handler(state, rules, user, |b| b.suppresses_own_item);
    if user.volatiles.contains(VolatileConditionKind::Embargo)
        || state.field.contains(FieldConditionKind::MagicRoom)
        || (transfer.fling_klutz_blocks && klutz())
        || (transfer.fling_refuses_multitype
            && (super::raw_ability(user) == crate::ids::AbilityId::MULTITYPE
                || item == ItemId::GRISEOUS_ORB))
        // Fling's `onPrepareHit` asks the item's `TakeItem` with its own holder as the taker:
        // a Mega Stone its holder Mega Evolves with, a Z-Crystal, Mail stay (DESIGN.md R-M8-89).
        || item_refuses_take(
            state,
            rules,
            item,
            user.id(),
            user.id(),
            Some(crate::ids::MoveId::FLING),
        )
    {
        return None;
    }
    Some((item, behavior, behavior.fling_power?))
}

/// The berry `user`'s Natural Gift uses up, and what it makes of the move (Showdown Natural
/// Gift's `onModifyType` / `onPrepareHit`): the held item, its effects applying
/// (`ignoringItem()`), with a `naturalGift`.
pub(crate) fn natural_gift_berry(
    state: &BattleState,
    rules: &ResolvedRules,
    user: &PokemonState,
) -> Option<(ItemId, crate::effects::items::NaturalGift)> {
    let (item, behavior) = held_item_of(state, rules, user)?;
    Some((item, behavior.natural_gift?))
}

/// Embargo, Magic Room, or Klutz on `pokemon`, whatever it holds: Gen 4 Bug Bite and Pluck's
/// eater (pokeplatinum `BattleSystem_PluckBerry` asks the ability itself, issues.md I59).
pub(crate) fn sets_any_item_aside(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> bool {
    ignoring_item(state, rules, pokemon, &ItemBehavior::default())
}

/// Showdown `ignoringItem()`: the holder is under Embargo, Magic Room is up, or its Klutz sets
/// the item aside (unless the item is marked `ignoreKlutz`). Klutz is Showdown's
/// `hasAbility('klutz')`: the ability in effect (`engine::ability_active`, DESIGN.md §41.1).
/// Showdown's other steps — a Primal Orb never ignored, and Gen 5+ a Pokémon off the field
/// always ignoring its item — change nothing any implemented item or reading shows yet.
fn ignoring_item(
    state: &BattleState,
    rules: &ResolvedRules,
    holder: &PokemonState,
    item: &ItemBehavior,
) -> bool {
    holder.volatiles.contains(VolatileConditionKind::Embargo)
        || state.field.contains(FieldConditionKind::MagicRoom)
        || (rules.conditions.bench_items_ignored
            && state.active_position(holder.id(), &rules.topology).is_none())
        // An item Klutz leaves alone is not asked about it first: Ability Shield is one, and
        // whether an ability is in effect asks about Ability Shield (Showdown's order too).
        || (!item.ignores_klutz
            && super::active_ability_handler(state, rules, holder, |b| b.suppresses_own_item))
}

/// The item's own `onTakeItem` refusing to leave `holder` for `taker` while `move_id` is the
/// active move, if any (DESIGN.md §39.3): Mail goes only to Knock Off, Thief and Covet (the last two
/// where `ItemTransferRules::mail_stolen` says); the Rusted Sword / Shield (and Gen 5+ Plates)
/// stay when either is the species they belong to. It answers whatever sets the item aside.
pub fn item_refuses_take(
    state: &BattleState,
    rules: &ResolvedRules,
    item: ItemId,
    holder: PokemonId,
    taker: PokemonId,
    move_id: Option<crate::ids::MoveId>,
) -> bool {
    use crate::effects::items::TakeRefusal;
    use crate::ids::MoveId;
    let behavior = super::item_behavior(rules, item);
    // Gen 4's Griseous Orb is never taken, whoever holds it (the gen4 mod's `onTakeItem:
    // false`; DESIGN.md §43.4).
    if behavior.refuses_take_always && rules.conditions.items.griseous_orb_never_taken {
        return true;
    }
    match behavior.take_refusal {
        None => false,
        Some(TakeRefusal::Mail) => {
            !(move_id == Some(MoveId::KNOCK_OFF)
                || (rules.conditions.items.mail_stolen
                    && matches!(move_id, Some(MoveId::THIEF | MoveId::COVET))))
        }
        // Gen 4's Plates have no refusal of their own (`ItemTransferRules::plates_stay_with_arceus`).
        Some(TakeRefusal::Species(_))
            if behavior.plate.is_some() && !rules.conditions.items.plates_stay_with_arceus =>
        {
            false
        }
        Some(TakeRefusal::Species(dex)) => [holder, taker].into_iter().any(|id| {
            state
                .pokemon(id)
                .and_then(|p| p.species)
                .is_some_and(|species| species.dex == dex)
        }),
        Some(TakeRefusal::Paradox) => state
            .pokemon(holder)
            .and_then(|p| p.species)
            .is_some_and(|species| PARADOX_SPECIES.iter().any(|p| p.dex == species.dex)),
        Some(TakeRefusal::ZCrystal) => true,
        // A Mega Stone stays with the species it belongs to (any forme of it).
        Some(TakeRefusal::MegaStone(dex)) => state
            .pokemon(holder)
            .and_then(|p| p.species)
            .is_some_and(|species| species.dex == dex),
    }
}

/// The species Showdown tags `Paradox` (`species.tags`, @pkmn/sim 0.10.11: sixteen of them),
/// whose Booster Energy cannot be taken. The dex snapshot carries no tags.
pub const PARADOX_SPECIES: [crate::ids::SpeciesId; 16] = {
    use crate::ids::SpeciesId as S;
    [
        S::GREAT_TUSK,
        S::SCREAM_TAIL,
        S::BRUTE_BONNET,
        S::FLUTTER_MANE,
        S::SLITHER_WING,
        S::SANDY_SHOCKS,
        S::IRON_TREADS,
        S::IRON_BUNDLE,
        S::IRON_HANDS,
        S::IRON_JUGULIS,
        S::IRON_MOTH,
        S::IRON_THORNS,
        S::ROARING_MOON,
        S::IRON_VALIANT,
        S::WALKING_WAKE,
        S::IRON_LEAVES,
    ]
};
