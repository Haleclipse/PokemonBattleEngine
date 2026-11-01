//! Stockpile's volatile (DESIGN.md §20.1, R159): its layers, the stages it raises and takes
//! back (`ConditionRules::stockpile`), and its end by Spit Up or Swallow.

use crate::engine::{
    ConditionApplyError, EffectSource, StatChangeCause, StatChangeRequest, StatChangeResult,
    TraceEvent,
};
use crate::ids::{MoveId, PokemonId};
use crate::rules::StockpileRule;
use crate::state::{Stat, VolatileConditionKind};

use super::BattleContext;

impl<'a> BattleContext<'a> {
    /// `pokemon`'s Stockpile layers (0 without one).
    pub fn stockpile_layers(&self, pokemon: PokemonId) -> u8 {
        self.state
            .pokemon(pokemon)
            .map_or(0, |p| p.volatiles.stockpile_layers())
    }

    fn stockpile_stage(&mut self, pokemon: PokemonId, stat: Stat, stages: i8) -> bool {
        matches!(
            self.try_change_stat_stage(StatChangeRequest {
                target: pokemon,
                stat,
                stages,
                // Showdown `this.boost(…, pokemon, pokemon)`: the holder's own doing, so Clear
                // Body, Flower Veil and Mist let the end's drops through (R-M8-107).
                source: EffectSource::Move {
                    pokemon,
                    move_id: MoveId::STOCKPILE,
                },
                cause: StatChangeCause::Move,
            }),
            StatChangeResult::Applied { .. }
        )
    }

    /// Showdown `addVolatile('stockpile')`: one more layer (refused at three), then +1 Defense
    /// and Special Defense as the generation says, counting what its end takes back.
    pub fn stockpile(&mut self, pokemon: PokemonId) -> Result<(), ConditionApplyError> {
        if self.stockpile_layers(pokemon) >= 3 {
            return Err(ConditionApplyError::AlreadyActive);
        }
        let Some(target) = self.state.pokemon_mut(pokemon) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        target.volatiles.add_stockpile_layer();
        self.trace.push(TraceEvent::VolatileStarted {
            target: pokemon,
            kind: VolatileConditionKind::Stockpile,
        });
        let rule = self.rules.conditions.stockpile;
        if rule == StockpileRule::NoStages {
            return Ok(());
        }
        let defense = self.stockpile_stage(pokemon, Stat::Defense, 1);
        let special_defense = self.stockpile_stage(pokemon, Stat::SpecialDefense, 1);
        let every_layer = rule == StockpileRule::StagesByLayer;
        if let Some(target) = self.state.pokemon_mut(pokemon) {
            target
                .volatiles
                .count_stockpile_stages(every_layer || defense, every_layer || special_defense);
        }
        Ok(())
    }

    /// Showdown `removeVolatile('stockpile')`: the stages it counted come back down, then it
    /// ends.
    pub fn end_stockpile(&mut self, pokemon: PokemonId) {
        let Some((defense, special_defense)) = self
            .state
            .pokemon_mut(pokemon)
            .and_then(|p| p.volatiles.take_stockpile())
        else {
            return;
        };
        if defense != 0 {
            self.stockpile_stage(pokemon, Stat::Defense, defense);
        }
        if special_defense != 0 {
            self.stockpile_stage(pokemon, Stat::SpecialDefense, special_defense);
        }
        self.trace.push(TraceEvent::VolatileEnded {
            target: pokemon,
            kind: VolatileConditionKind::Stockpile,
            seen: true,
        });
    }
}
