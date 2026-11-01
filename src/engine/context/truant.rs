//! Truant (DESIGN.md §15.21): the turns its holder loafs on, as the generation keeps them
//! (`TruantRules`).

use crate::ids::{AbilityId, PokemonId};
use crate::rules::TruantModel;
use crate::state::{BattleState, PokemonState, VolatileConditionKind};

use super::BattleContext;

/// Whether `pokemon`'s next move attempt this turn would loaf, without counting it: what the
/// cartridges ask before the turn's moves (Gen 3-4 Pursuit's mark). Gen 3 reads the flag alone
/// (pokeemerald `!gDisableStructs[…].truantCounter`), Gen 4 the ability and the turn's parity
/// (`Battler_CheckTruant`), Gen 5+ the ability and the volatile.
pub(crate) fn truant_loafs_next(
    state: &BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: &PokemonState,
) -> bool {
    let truant = || crate::engine::ability_handler(state, rules, pokemon, |b| b.truant);
    match rules.conditions.truant.model {
        TruantModel::TurnEndFlip => pokemon.truant_turn,
        TruantModel::TurnParity => truant() && pokemon.truant_turn == (state.turn % 2 == 1),
        TruantModel::MoveFlip => {
            truant() && pokemon.volatiles.contains(VolatileConditionKind::Truant)
        }
    }
}

impl BattleContext<'_> {
    /// Truant's check in BeforeMove (Showdown `onBeforeMove`, priority 9; the cartridges' between
    /// freeze and the recharge, issues.md I105): whether the attempt loafs. Gen 5+ flips the
    /// volatile — it goes as the holder loafs, comes as it acts.
    pub(crate) fn truant_loafs(&mut self, actor: PokemonId) -> bool {
        let Some(pokemon) = self.state.pokemon(actor) else {
            return false;
        };
        if !crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.truant) {
            return false;
        }
        match self.rules.conditions.truant.model {
            TruantModel::TurnEndFlip => pokemon.truant_turn,
            TruantModel::TurnParity => pokemon.truant_turn == (self.state.turn % 2 == 1),
            TruantModel::MoveFlip => {
                if self.end_volatile(actor, VolatileConditionKind::Truant) {
                    return true;
                }
                let _ = self.set_marker_volatile(actor, VolatileConditionKind::Truant);
                false
            }
        }
    }

    /// Gen 5+ Truant's `onStart` (`TruantModel::MoveFlip`): the volatile goes, and comes back on a
    /// holder past its first turn that has moved this turn or will not move any more — its next
    /// attempt loafs. Gen 3-4 start Truant only as the holder comes in (`StartNewAbility::Never`,
    /// `Unannounced` — Truant announces nothing), on its first turn: nothing is set there; their
    /// counts are `enter_truant`'s, Role Play's and Skill Swap's.
    pub fn start_truant(&mut self, holder: PokemonId) {
        self.end_volatile(holder, VolatileConditionKind::Truant);
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        if pokemon.active_turns > 0
            && (pokemon.history.move_this_turn.is_some() || !self.state.will_move(holder))
        {
            let _ = self.set_marker_volatile(holder, VolatileConditionKind::Truant);
        }
    }

    /// Gen 3 Truant's `onResidual` (`TruantModel::TurnEndFlip`; only the gen3 data declares it):
    /// every turn end the holder has Truant flips the flag.
    pub fn flip_truant_turn(&mut self, holder: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.truant_turn = !pokemon.truant_turn;
        }
    }

    /// `pokemon` comes in after the battle's start (the leads never pass through here): Gen 3 sets
    /// the flag for a Truant holder (pokeemerald `Cmd_switchineffects`; Showdown gen3 `onSwitchIn`:
    /// `turn !== 0`) and clears it for any other (the cartridge's `DisableStruct` starts over,
    /// issues.md I107); Gen 4 sets every Pokémon's parity to the turn's (pokeplatinum
    /// `BattleSystem_UpdateAfterSwitch`: `(totalTurns + 1) & 1`, issues.md I106).
    pub(crate) fn enter_truant(&mut self, pokemon: PokemonId) {
        let model = self.rules.conditions.truant.model;
        let turn = self.state.turn;
        let Some(entrant) = self.state.pokemon_mut(pokemon) else {
            return;
        };
        match model {
            TruantModel::TurnEndFlip => {
                entrant.truant_turn = crate::engine::raw_ability(entrant) == AbilityId::TRUANT;
            }
            TruantModel::TurnParity => entrant.truant_turn = turn % 2 == 1,
            TruantModel::MoveFlip => {}
        }
    }

    /// Gen 4 Role Play (its user) and Skill Swap (both) as they succeed (pokeplatinum
    /// `subscript_copy_ability` / `subscript_exchange_abilities`: `BATTLEMON_TRUANT =
    /// totalTurns & 1`, issues.md I106): the parity of the turn after — a Pokémon that gained
    /// Truant loafs on it.
    pub fn restart_truant_parity(&mut self, pokemon: PokemonId) {
        if self.rules.conditions.truant.model != TruantModel::TurnParity {
            return;
        }
        let turn = self.state.turn;
        if let Some(pokemon) = self.state.pokemon_mut(pokemon) {
            pokemon.truant_turn = turn.is_multiple_of(2);
        }
    }
}
