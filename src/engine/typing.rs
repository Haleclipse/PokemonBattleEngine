//! The types a Pokémon has right now (DESIGN.md §41.1, §41.4): Showdown `getTypes`. Every engine
//! reading of a Pokémon's types goes through `types_of`, so Roost and an added type (Forest's
//! Curse, Trick-or-Treat) join in one place, Terastallization later (O). The writes are
//! `BattleContext::set_type` / `add_type` and, for a forme's types (Forecast, Mimicry),
//! `PokemonState::change_types`.

use crate::dex::{PokemonType, ResolvedDex};
use crate::ids::{MoveId, PokemonId};
use crate::rules::{Conversion, ResolvedRules};
use crate::state::{BattleState, PokemonState, VolatileConditionKind};

/// The species `pokemon` battles as (Showdown `pokemon.species`, DESIGN.md §41.7): Dark Void's
/// and Hyperspace Fury's user, Aura Wheel's, Ivy Cudgel's and Raging Bull's type, Gen 4-5
/// Chatter's confusion, Water Shuriken's Ash-Greninja, Arceus and Silvally's type lock, the
/// weight. Its own species (`baseSpecies`) is `PokemonState::species`; a forme it changed into
/// for the time it stays on the field is `PokemonState::forme` (DESIGN.md §43.1).
pub fn species_of(pokemon: &PokemonState) -> Option<crate::ids::SpeciesId> {
    match &pokemon.transform {
        Some(transform) => transform.species,
        None => pokemon.forme.or(pokemon.species),
    }
}

/// The species a species' own item asks of its holder (Light Ball, Stick …): the one it battles
/// as in Gen 2-4, its own from Gen 5 (`StatRules::species_items_read_battling`).
pub fn item_species(
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> Option<crate::ids::SpeciesId> {
    if rules.stats.species_items_read_battling {
        species_of(pokemon)
    } else {
        pokemon.species
    }
}

/// Showdown `getTypes()`: the types as the battle reads them — up to two, then the added one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeSet {
    pub primary: PokemonType,
    pub secondary: Option<PokemonType>,
    /// Showdown `addedType`, last.
    pub added: Option<PokemonType>,
}

impl TypeSet {
    /// One type, nothing added: what a Terastallized Pokémon reads (`getTypes`).
    pub const fn mono(type_: PokemonType) -> Self {
        Self {
            primary: type_,
            secondary: None,
            added: None,
        }
    }

    pub fn contains(self, type_: PokemonType) -> bool {
        self.iter().any(|t| t == type_)
    }

    pub fn iter(self) -> impl Iterator<Item = PokemonType> {
        [Some(self.primary), self.secondary, self.added]
            .into_iter()
            .flatten()
    }

    /// Showdown `getTypes(true)`: without the added type.
    pub fn without_added(self) -> impl Iterator<Item = PokemonType> {
        [Some(self.primary), self.secondary].into_iter().flatten()
    }
}

/// Showdown `getTypes`: `pokemon`'s types as the battle reads them — a Terastallized Pokémon
/// (except a Stellar one) has its Tera type alone; otherwise its own, without Flying while it
/// roosts (`onType`), the generation's filler when nothing is left
/// (`TypeRules::empty_becomes`), then the added type.
pub fn types_of(
    state: &crate::state::BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> TypeSet {
    match pokemon.terastallized {
        Some(tera) if tera != PokemonType::Stellar => TypeSet::mono(tera),
        _ => pre_tera_types_of(state, rules, pokemon),
    }
}

/// Showdown `getTypes(false, true)`: the types before Terastallization. STAB and the 60-power
/// floor read them (`getDamage`).
pub fn pre_tera_types_of(
    state: &crate::state::BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> TypeSet {
    // Arceus's and Silvally's own conditions (`onType`, priority 1, DESIGN.md §43.4): a holder
    // of the species with its own ability (Multitype / RKS System), untransformed, is the type
    // of the Plate / Memory it holds ('Normal' without one — not the generation's `???`
    // filler). The conditions are the species' effects, not the item's: they run only while
    // the Pokémon is on the field (Showdown's `findEventHandlers` collects nothing for an
    // inactive Pokémon: a benched one keeps its species' types) and read the item as it is
    // held, so Embargo, Magic Room and Klutz do not take the type away (probed:
    // `gen7-arceus-type-under-magic-room`, `gen7-silvally-type-under-magic-room`). A holder
    // without the ability instead gets Normal in Gen 5-7 (`this.gen >= 8` in the conditions;
    // the cartridge's `Battler_MonType` agrees for Gen 4's Multitype holder, R-N.6, and Gen 4
    // reads the types as they stand — its Arceus entered Normal by `onBeforeSwitchIn`,
    // `PlateRules::before_switch_in_normal`). A transformed one keeps its own types.
    if state
        .active_position(pokemon.id(), &rules.topology)
        .is_some()
    {
        let own_gift = pokemon
            .species
            .filter(|_| pokemon.transform.is_none())
            .and_then(|own| {
                if own.same_species(crate::ids::SpeciesId::ARCEUS) {
                    Some((
                        crate::ids::AbilityId::MULTITYPE,
                        raw_plate_type(rules, pokemon),
                    ))
                } else if own.same_species(crate::ids::SpeciesId::SILVALLY) {
                    Some((
                        crate::ids::AbilityId::RKS_SYSTEM,
                        raw_memory_type(rules, pokemon),
                    ))
                } else {
                    None
                }
            });
        // The condition replaces the types only: an added type (Forest's Curse,
        // Trick-or-Treat) still comes last, as `getTypes` concatenates it outside the event.
        let replace = |type_: PokemonType| TypeSet {
            primary: type_,
            secondary: None,
            added: pokemon.added_type,
        };
        if let Some((ability, gift)) = own_gift {
            if crate::engine::raw_ability(pokemon) == ability {
                return replace(gift.unwrap_or(PokemonType::Normal));
            }
            if rules.conditions.type_change.without_ability_becomes_normal {
                return replace(PokemonType::Normal);
            }
        }
    }
    let roosting = pokemon.volatiles.contains(VolatileConditionKind::Roost);
    let mut own = pokemon
        .battle_types()
        .iter()
        .filter(|t| !(roosting && *t == PokemonType::Flying));
    let primary = own
        .next()
        .unwrap_or(rules.conditions.type_change.empty_becomes);
    TypeSet {
        primary,
        secondary: own.next(),
        added: pokemon.added_type,
    }
}

/// The Plate type `pokemon`'s held item carries, read as it is held (`pokemon.getItem()
/// .onPlate`): the species conditions ask the item itself, so Embargo, Magic Room, Klutz and
/// the bench do not take it away (DESIGN.md §43.4). The move callbacks use `plate_type`.
pub fn raw_plate_type(rules: &ResolvedRules, pokemon: &PokemonState) -> Option<PokemonType> {
    let item = pokemon.held_item?;
    crate::engine::item_behavior(rules, item).plate
}

/// `raw_plate_type` for a Memory (`pokemon.getItem().onMemory`): Silvally's RKS System type.
pub fn raw_memory_type(rules: &ResolvedRules, pokemon: &PokemonState) -> Option<PokemonType> {
    let item = pokemon.held_item?;
    crate::engine::item_behavior(rules, item).memory
}

/// The type the Plate `pokemon` holds gives it (Showdown `pokemon.getItem().onPlate`: an item
/// set aside gives nothing, `engine::held_item`). DESIGN.md §43.4.
pub fn plate_type(
    state: &crate::state::BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> Option<PokemonType> {
    crate::engine::held_item(state, rules, pokemon.id())
        .and_then(|(item, _)| crate::engine::item_behavior(rules, item).plate)
}

/// The type the Memory `pokemon` holds gives its RKS System (Silvally) and its Multi-Attack
/// (Showdown `pokemon.getItem().onMemory` / the Memory event; an item set aside gives nothing).
/// DESIGN.md §43.4.
pub fn memory_type(
    state: &crate::state::BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> Option<PokemonType> {
    crate::engine::held_item(state, rules, pokemon.id())
        .and_then(|(item, _)| crate::engine::item_behavior(rules, item).memory)
}

/// The type the Drive `pokemon` holds gives its Techno Blast (Showdown `pokemon.getItem()
/// .onDrive` / the Drive event; an item set aside gives nothing). DESIGN.md §43.4.
pub fn drive_type(
    state: &crate::state::BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
) -> Option<PokemonType> {
    crate::engine::held_item(state, rules, pokemon.id())
        .and_then(|(item, _)| crate::engine::item_behavior(rules, item).drive)
}

/// Showdown `getDamage`'s STAB step (DESIGN.md §44.1): the modifier the move's type gets, and
/// whether it used up Stellar's one-time boost for that type.
///
/// `isSTAB` is Showdown's `move.forceSTAB || hasType(type) || getTypes(false, true).includes(type)`:
/// a Terastallized Pokémon keeps STAB on the types it had before, so its Tera type adds to them
/// rather than replacing them.
///
/// - Stellar: `2` for such a type, `4915/4096` otherwise, once per type and move (the caller
///   records it in `PokemonState::stellar_boosted_types`);
/// - a Terastallized Pokémon whose Tera type is the move's type: `2` when that type is one of
///   its own, the generation's STAB otherwise;
/// - its own type: the generation's STAB (`DamageRules::stab_modifier`);
/// - then `onModifySTAB` (Adaptability: `2` → `2.25`, else `2`), for a move the Pokémon has the
///   type of now (`hasType`, not the pre-Tera one), which Stellar skips.
pub fn stab_modifier(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    move_type: PokemonType,
    force_stab: bool,
) -> StabModifier {
    // "The '???' type never gets STAB" (`if (type !== '???')` around the whole step, Stellar and
    // Adaptability included): a typeless Struggle from a Pokémon Burn Up left typeless gets none
    // (R-M8-107).
    if move_type == PokemonType::Typeless {
        return StabModifier {
            modifier: crate::rules::Fraction::ONE,
            stellar_boosted: false,
        };
    }
    let current = types_of(state, rules, pokemon);
    let base_types = pre_tera_types_of(state, rules, pokemon).contains(move_type);
    let is_stab = force_stab || current.contains(move_type) || base_types;
    let mut modifier = if is_stab {
        rules.damage.stab_modifier
    } else {
        crate::rules::Fraction::ONE
    };
    let mut stellar_boosted = false;
    // `move.stellarBoosted`: the move under way already applied the boost, so every other target
    // of the same spread hit gets it too (the move object is shared).
    let move_used_it = state
        .active_move
        .is_some_and(|active| active.stellar_boosted);
    match pokemon.terastallized {
        Some(PokemonType::Stellar)
            if move_used_it || !pokemon.stellar_boosted_types.contains(&move_type) =>
        {
            modifier = if is_stab {
                crate::rules::Fraction::new(2, 1)
            } else {
                crate::rules::Fraction::new(4915, 4096)
            };
            stellar_boosted = true;
        }
        Some(tera) if tera == move_type && base_types => {
            modifier = crate::rules::Fraction::new(2, 1);
        }
        _ => {}
    }
    // Adaptability (`onModifySTAB`): `move.forceSTAB || source.hasType(move.type)` — the types
    // it has now, so a Terastallized Pokémon's original types get only the plain STAB above.
    if pokemon.terastallized != Some(PokemonType::Stellar)
        && (force_stab || current.contains(move_type))
        && let Some(boost) =
            crate::engine::active_ability_handler(state, rules, pokemon, |b| b.stab_modifier)
    {
        // `if (stab === 2) return 2.25;` — the doubled STAB becomes 2.25, not 3.
        modifier = if modifier == crate::rules::Fraction::new(2, 1) {
            crate::rules::Fraction::new(9, 4)
        } else {
            boost
        };
    }
    StabModifier {
        modifier,
        stellar_boosted,
    }
}

/// What `stab_modifier` resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StabModifier {
    pub modifier: crate::rules::Fraction,
    /// Stellar's boost was applied: the move records the type (`getDamage`'s
    /// `stellarBoostedTypes`), which the caller does.
    pub stellar_boosted: bool,
}

/// Showdown `hasType`.
pub fn has_type(
    state: &crate::state::BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    pokemon_type: PokemonType,
) -> bool {
    types_of(state, rules, pokemon).contains(pokemon_type)
}

/// Gen 2-5 Conversion's candidates (`Conversion::RandomMoveType`), one per move slot in order
/// (Showdown samples over the slots, so a type two slots share counts twice): the slot's type,
/// unless the user has it already. Gen 4-5 leave Conversion out; a `???` move (Curse before
/// Gen 5) is Ghost for a Ghost user and Normal for another where the generation counts it
/// (Gen 3-4 on the cartridge), left out where not. The move and the replay share it.
pub fn conversion_candidates(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    user: PokemonId,
) -> Vec<PokemonType> {
    let Conversion::RandomMoveType {
        skips_conversion,
        typeless_counts,
    } = rules.conditions.type_change.conversion
    else {
        return Vec::new();
    };
    let Some(pokemon) = state.pokemon(user) else {
        return Vec::new();
    };
    let types = types_of(state, rules, pokemon);
    pokemon
        .move_slots()
        .iter()
        .filter(|slot| !(skips_conversion && slot.move_id == MoveId::CONVERSION))
        .filter_map(|slot| {
            let move_type = dex.move_by_id(slot.move_id)?.move_type;
            let move_type = match move_type {
                PokemonType::Typeless if !typeless_counts => return None,
                PokemonType::Typeless if types.contains(PokemonType::Ghost) => PokemonType::Ghost,
                PokemonType::Typeless => PokemonType::Normal,
                other => other,
            };
            (!types.contains(move_type)).then_some(move_type)
        })
        .collect()
}

/// Conversion 2's candidates: the types that resist or are immune to the type of the move
/// `target` last used (Gen 2 its `lastMove` with the move's own type; later its `lastMoveUsed`
/// with the type it was used as; Gen 2-4 Struggle counts as Normal, `TypeRules::
/// conversion2_struggle_as_normal`), leaving out the user's own from Gen 4. `None` when the target has used no move. In the engine's type order (the replay
/// reads the type drawn, not its place).
pub fn conversion2_candidates(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    user: PokemonId,
    target: PokemonId,
) -> Option<Vec<PokemonType>> {
    let foe = state.pokemon(target)?;
    let rule = rules.conditions.type_change;
    let (move_id, move_type) = if rule.conversion2_reads_last_move {
        let move_id = foe.last_move?;
        (move_id, dex.move_by_id(move_id)?.move_type)
    } else {
        foe.last_move_used?
    };
    let attack = if move_id == MoveId::STRUGGLE && rule.conversion2_struggle_as_normal {
        PokemonType::Normal
    } else {
        move_type
    };
    let own = state
        .pokemon(user)
        .map(|pokemon| types_of(state, rules, pokemon));
    Some(
        PokemonType::ALL
            .into_iter()
            .filter(|t| rules.type_chart.is_available(*t))
            .filter(|t| {
                !(rule.conversion2_skips_own_types && own.is_some_and(|own| own.contains(*t)))
            })
            .filter(|t| {
                let resisted = rules.type_chart.effectiveness(attack, *t);
                u32::from(resisted.numerator) < u32::from(resisted.denominator)
            })
            .collect(),
    )
}

/// Whether the generation keeps `pokemon`'s types as they are (`TypeRules::lock`): Gen 4 a
/// Multitype holder, Gen 5+ Arceus and Silvally.
pub fn types_locked(state: &BattleState, rules: &ResolvedRules, pokemon: &PokemonState) -> bool {
    match rules.conditions.type_change.lock {
        crate::rules::TypeLock::None => false,
        crate::rules::TypeLock::MultitypeHolder => {
            super::has_ability(state, rules, pokemon, crate::ids::AbilityId::MULTITYPE)
        }
        crate::rules::TypeLock::Species(dex) => {
            species_of(pokemon).is_some_and(|s| dex.contains(&s.dex))
        }
    }
}

/// The type Protean / Libero (`onPrepareHit`, after the move's own) gives `user` as it uses a
/// move of `move_type`, if any: not for `???` or a type already the user's alone
/// (`getTypes().join()`), not (Gen 9) once the ability has changed it this stint, not where the
/// types are locked (`setType` refusing). Shared by `BattleContext::protean` and the replay,
/// which plans what the move draws after it (Conversion's candidates) on the types it leaves.
pub fn protean_type(
    state: &BattleState,
    rules: &ResolvedRules,
    user: PokemonId,
    move_type: PokemonType,
) -> Option<PokemonType> {
    let pokemon = state.pokemon(user)?;
    if !super::ability_handler(state, rules, pokemon, |b| b.retypes_before_hit)
        || (rules.conditions.type_change.protean_once_per_entry
            && pokemon.ability_state.protean_used)
        || move_type == PokemonType::Typeless
        || types_of(state, rules, pokemon).iter().eq([move_type])
        || types_locked(state, rules, pokemon)
    {
        return None;
    }
    Some(move_type)
}
