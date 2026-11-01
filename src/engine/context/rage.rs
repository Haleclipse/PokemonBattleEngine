//! Rage (DESIGN.md §15.19): the user's flag and what the hits it takes build, as `RageRules`
//! has them.

use crate::engine::{EffectSource, StatChangeCause, StatChangeRequest, TraceEvent};
use crate::ids::PokemonId;
use crate::rules::RageEnd;
use crate::state::{Stat, VolatileConditionKind};

use super::BattleContext;

impl<'a> BattleContext<'a> {
    /// The Pokémon is raging (Showdown `addVolatile('rage')`, the cartridges' flag), if it was
    /// not already — Gen 2's count at 0, Gen 1 locked into Rage (`RageEnd::Locked`).
    pub fn start_rage(&mut self, pokemon: PokemonId) {
        let locks = self.rules.conditions.rage.end == RageEnd::Locked;
        if self
            .state
            .pokemon_mut(pokemon)
            .is_some_and(|p| !p.is_fainted() && p.volatiles.start_rage(locks))
        {
            self.trace.push(TraceEvent::VolatileStarted {
                target: pokemon,
                kind: VolatileConditionKind::Rage,
            });
        }
    }

    /// The Pokémon stops raging, silently (Gen 2: its count with it).
    pub fn end_rage(&mut self, pokemon: PokemonId) {
        self.end_volatile(pokemon, VolatileConditionKind::Rage);
    }

    /// Gen 2 (pokecrystal `BattleCommand_BuildOpponentRage`): a hit builds the raging Pokémon's
    /// count, up to 255.
    pub fn count_rage(&mut self, holder: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.volatiles.count_rage();
        }
    }

    /// The raging Pokémon's Attack goes up one stage — Gen 5+ through Showdown's `boost` (its
    /// `ChangeBoost`: Contrary, Simple; from Gen 6 nothing once the holder's foes have no
    /// Pokémon left), Gen 3-4 the cartridges' own increment, Gen 1 its stat-up (the modified
    /// stat recomputed, nothing at 999). True when the stage moved.
    pub fn raise_rage(&mut self, holder: PokemonId) -> bool {
        if self.rules.stats.boosts_need_foe_pokemon_left && !self.foe_pokemon_left(holder) {
            return false;
        }
        matches!(
            self.try_change_stat_stage(StatChangeRequest {
                target: holder,
                stat: Stat::Attack,
                stages: 1,
                source: EffectSource::Volatile(VolatileConditionKind::Rage),
                cause: StatChangeCause::Rule,
            }),
            crate::engine::StatChangeResult::Applied { .. }
        )
    }

    /// Gen 4 (pokeplatinum `PRE_MOVE_ACTION_STATE_CHECK_RAGE_FLAG`, issues.md I78): as the turn
    /// starts, a raging Pokémon that did not choose Rage loses every volatile the cartridge's
    /// bug clears (`RageEnd::Gen4`), silently; it stays raging.
    pub fn rage_wipe(&mut self, pokemon: PokemonId) {
        let RageEnd::Gen4 { wiped } = self.rules.conditions.rage.end else {
            return;
        };
        for &kind in wiped {
            // Mean Look's hold goes with its link: the trapper that holds nobody else is no
            // trapper any more (the cartridge has no such state).
            if kind == VolatileConditionKind::Trapped {
                self.free_trapped(pokemon);
            } else {
                // Silently, with no word of its own (I78): the observers still see it on.
                self.end_volatile_unseen(pokemon, kind);
            }
        }
    }
}
