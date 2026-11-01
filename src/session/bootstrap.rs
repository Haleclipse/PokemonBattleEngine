//! The start: the chosen Pokémon of two registered teams built into the battle's state, the
//! way Showdown's `Side` builds them, and the engine created from it.

use std::sync::Arc;

use crate::BattleEngine;
use crate::dex::{Nature, PokemonSet, PokemonType, SetError, StatSpread, build_pokemon};
use crate::format::{Named, Stats, TeamSet};
use crate::ids::{PokemonId, SideId};
use crate::spec::BattleMode;
use crate::state::{BattleRng, BattleState, Gender, SideState};

use super::format::{SessionError, SessionFormat};

/// Why one normalised set could not become a Pokémon of the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum SetBuildError {
    /// The dex has no such species, move, ability or item, or the level is out of its range.
    Dex(SetError),
    /// A type the engine does not know (a Tera or Hidden Power type).
    UnknownType(String),
    /// Gigantamax: the engine's Dynamax has no G-Max formes or moves (DESIGN.md §44.5).
    Gigantamax,
}

impl SessionFormat<'_> {
    internal!(
        /// The battle the chosen Pokémon start: `chosen` is each side's Pokémon in the order the
        /// side put them, leads first, from its registered team. `rng` is the battle's generator
        /// as registration left it (one stream, as Showdown's).
        pub fn start(
            &self,
            chosen: [&[TeamSet]; 2],
            rng: BattleRng,
        ) -> Result<BattleEngine, SessionError> {
            let state = self.state(chosen, rng)?;
            let rules = Arc::new(self.rules()?);
            BattleEngine::create_new(rules, self.dex.clone(), state).map_err(SessionError::Init)
        }
    );

    internal!(
        /// `start`'s state, before the engine starts the battle: the chosen sets built side by side
        /// in their order, the first of each side in its active slots.
        pub fn state(
            &self,
            chosen: [&[TeamSet]; 2],
            rng: BattleRng,
        ) -> Result<BattleState, SessionError> {
            let slots = match self.spec.mode {
                BattleMode::Singles => 1,
                BattleMode::Doubles => 2,
            };
            let mut pokemon = Vec::new();
            let mut ids: [Vec<PokemonId>; 2] = [Vec::new(), Vec::new()];
            let mut next = 1u8;
            for (side_index, (side, sets)) in
                [SideId::P1, SideId::P2].into_iter().zip(chosen).enumerate()
            {
                for (index, set) in sets.iter().enumerate() {
                    let fail = |error| SessionError::Set { side, index, error };
                    let engine_set = self.engine_set(set).map_err(fail)?;
                    let id = PokemonId(next);
                    next += 1;
                    pokemon.push(
                        build_pokemon(&self.dex, id, side, &engine_set)
                            .map_err(|error| fail(SetBuildError::Dex(error)))?,
                    );
                    ids[side_index].push(id);
                }
            }
            let side_state = |team: &Vec<PokemonId>| {
                let mut side = SideState::new(team.clone(), slots);
                for (slot, id) in team.iter().take(slots).enumerate() {
                    side.active[slot] = Some(*id);
                }
                side
            };
            let mut state =
                BattleState::new(pokemon, [side_state(&ids[0]), side_state(&ids[1])], 0);
            state.rng = rng;
            Ok(state)
        }
    );

    internal!(
        /// A normalised set as the engine's dex builds it, read the way Showdown's `Pokemon`
        /// constructor reads it: EVs 0 and IVs 31 where the set names none, clamped to 0-255 and
        /// 0-31, happiness 255, a `Hidden Power Fire` move as Hidden Power of the type, `N` as no
        /// gender, a cosmetic forme as its base species. (Below Gen 3 Showdown also evens the IVs,
        /// its DVs: the dex halves them anyway.)
        pub fn engine_set(&self, set: &TeamSet) -> Result<PokemonSet, SetBuildError> {
            if set.gigantamax {
                return Err(SetBuildError::Gigantamax);
            }
            let generation = self.validator.generation;
            let named_type = |name: &str| {
                if name.is_empty() {
                    Ok(None)
                } else {
                    PokemonType::from_name(name)
                        .map(Some)
                        .ok_or_else(|| SetBuildError::UnknownType(name.to_string()))
                }
            };
            let mut hp_type = named_type(&set.hp_type)?;
            let mut moves = Vec::new();
            for name in &set.moves {
                match self
                    .validator
                    .legality
                    .move_named(self.validator.aliases, name)
                {
                    Named::Found(found) if found.id == "hiddenpower" => {
                        // `move.id === 'hiddenpower' && move.type !== 'Normal'`: the set's type
                        // (the validator has written it in already).
                        if let Some(kind) = found.move_type.as_deref().filter(|t| *t != "Normal")
                            && hp_type.is_none()
                        {
                            hp_type = named_type(kind)?;
                        }
                        moves.push("Hidden Power".to_string());
                    }
                    Named::Found(found) => moves.push(found.name.clone()),
                    _ => moves.push(name.clone()),
                }
            }
            let spread = |stats: Option<Stats>, default: i32, max: i32| {
                let stats = stats.unwrap_or(Stats::filled(default));
                let clamp = |value: i32| value.clamp(0, max) as u8;
                StatSpread {
                    hp: clamp(stats.hp),
                    atk: clamp(stats.atk),
                    def: clamp(stats.def),
                    spa: clamp(stats.spa),
                    spd: clamp(stats.spd),
                    spe: clamp(stats.spe),
                }
            };
            let ability = match set.ability.as_str() {
                // Before Gen 3 every set is `No Ability`: the engine's Pokémon then has none.
                "" | "No Ability" if generation <= 2 => None,
                other => Some(other.to_string()),
            };
            Ok(PokemonSet {
                species: self.battle_species(set),
                level: u8::try_from(set.level).unwrap_or(u8::MAX),
                nature: Nature::from_name(&set.nature).unwrap_or(Nature::Hardy),
                evs: spread(set.evs, 0, 255),
                ivs: spread(set.ivs, 31, 31),
                ability,
                item: (!set.item.is_empty()).then(|| set.item.clone()),
                moves,
                happiness: set
                    .happiness
                    .map_or(255, |value| value.floor().clamp(0.0, 255.0) as u8),
                gender: match set.gender.as_str() {
                    "M" => Some(Gender::Male),
                    "F" => Some(Gender::Female),
                    "N" => Some(Gender::Genderless),
                    _ => None,
                },
                tera_type: named_type(&set.tera_type)?,
                dynamax_level: 10,
                hp_type,
            })
        }
    );
}
