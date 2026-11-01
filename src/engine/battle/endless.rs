//! The rules that end a battle at a turn's end (Showdown `maybeTriggerEndlessBattleClause`,
//! DESIGN.md §45.7): Gen 1's no-progress tie, the turn limit, and Endless Battle Clause.

use crate::dex::PokemonType;
use crate::engine::context::RESTORATIVE_BERRIES;
use crate::engine::{BattleEndRule, TraceEvent, trapped_at_turn_end};
use crate::ids::{AbilityId, MoveId, SideId};
use crate::state::{BattleOutcome, NonVolatileStatus, PokemonState, Staleness};

use super::BattleEngine;

/// Endless Battle Clause and the turn limit wait until this turn is over.
const QUIET_TURNS: u32 = 100;
/// Once this turn is over, the battle is a tie.
const TURN_LIMIT: u32 = 1000;

const SIDES: [SideId; 2] = [SideId::P1, SideId::P2];

impl BattleEngine {
    /// The turn boundary's rules (Showdown `endTurn` → `maybeTriggerEndlessBattleClause`), read
    /// once `turn` names the coming turn. The first rule that applies ends the battle.
    pub(super) fn end_turn_rules(&mut self) {
        let Some((rule, outcome)) = self.ending_rule() else {
            return;
        };
        self.trace.push(TraceEvent::BattleEndedByRule { rule });
        self.finish(outcome);
    }

    fn ending_rule(&self) -> Option<(BattleEndRule, BattleOutcome)> {
        // Gen 1 checks before the wait: such a battle cannot go on at all.
        if self.rules.faint.no_progress_ties && self.no_progress_possible() {
            return Some((BattleEndRule::NoProgress, BattleOutcome::Draw));
        }
        if self.state.turn <= QUIET_TURNS {
            return None;
        }
        // "It is turn 1000": the limit is not part of the clause.
        if self.state.turn > TURN_LIMIT {
            return Some((BattleEndRule::TurnLimit, BattleOutcome::Draw));
        }
        if !self.rules.clauses().endless_battle_clause {
            return None;
        }
        self.endless_battle()
    }

    /// Showdown's Gen 1 check, which holds when each side's Pokémon have all fainted or are
    /// stuck:
    /// - frozen (nothing thaws it in Gen 1 without help);
    /// - knowing only Transform, against a foe team that knows only Transform as well;
    /// - with no PP left, against a foe team of Ghosts (Struggle does not touch them).
    fn no_progress_possible(&self) -> bool {
        let only_transform = |pokemon: &PokemonState| {
            pokemon
                .move_slots()
                .iter()
                .all(|slot| slot.move_id == MoveId::TRANSFORM)
        };
        SIDES.into_iter().all(|side| {
            let foes = self.party(side.other());
            let foes_all_ghosts = foes.iter().all(|foe| {
                foe.is_fainted()
                    || crate::engine::types_of(&self.state, &self.rules, foe)
                        .contains(PokemonType::Ghost)
            });
            let foes_all_transform = foes
                .iter()
                .all(|foe| foe.is_fainted() || only_transform(foe));
            self.party(side).iter().all(|pokemon| {
                pokemon.is_fainted()
                    || pokemon.has_status(NonVolatileStatus::Freeze)
                    || (only_transform(pokemon) && foes_all_transform)
                    || (pokemon.move_slots().iter().all(|slot| slot.pp == 0) && foes_all_ghosts)
            })
        })
    }

    /// Endless Battle Clause.
    /// 1. Every side has a stale Pokémon on the field, and on some side that staleness is a
    ///    foe's doing. One stale Pokémon is enough for a doubles side: Showdown keeps the last
    ///    staleness it reads, and an external one sticks.
    /// 2. Some side cannot switch to a Pokémon that is not stale: it is trapped, or it has no
    ///    such Pokémon standing.
    /// 3. The sides that started with the means to cycle restorative berries. Exactly one such
    ///    side loses; otherwise it is a tie.
    fn endless_battle(&self) -> Option<(BattleEndRule, BattleOutcome)> {
        let mut trapped = [true; 2];
        let mut staleness: [Option<Staleness>; 2] = [None; 2];
        for (index, side) in SIDES.into_iter().enumerate() {
            for &id in self.state.side(side).active.iter().flatten() {
                let Some(pokemon) = self.state.pokemon(id).filter(|p| !p.is_fainted()) else {
                    continue;
                };
                trapped[index] &= trapped_at_turn_end(&self.state, &self.rules, id);
                if let Some(stale) = stale(pokemon)
                    && staleness[index] != Some(Staleness::External)
                {
                    staleness[index] = Some(stale);
                }
            }
        }
        if staleness.iter().any(Option::is_none) || !staleness.contains(&Some(Staleness::External))
        {
            return None;
        }
        let can_switch = |index: usize| {
            !trapped[index]
                && self
                    .party(SIDES[index])
                    .iter()
                    .any(|pokemon| !pokemon.is_fainted() && stale(pokemon).is_none())
        };
        if can_switch(0) && can_switch(1) {
            return None;
        }
        let berry_cycling = SIDES.map(|side| self.berry_cycling(side));
        let outcome = match berry_cycling {
            [true, false] => BattleOutcome::Winner(SideId::P2),
            [false, true] => BattleOutcome::Winner(SideId::P1),
            _ => BattleOutcome::Draw,
        };
        Some((BattleEndRule::EndlessBattle { berry_cycling }, outcome))
    }

    /// Whether the side's team started with the means to cycle restorative berries. Showdown
    /// walks the party in its in-battle order with two flags:
    /// - `berry`: whether this Pokémon's set holds a restorative berry. It is set afresh for each
    ///   Pokémon, not accumulated.
    /// - `cycle`: whether this Pokémon, or one before it, has Harvest, Pickup or Recycle in its
    ///   set.
    ///
    /// The side cycles berries when both flags are true at the same Pokémon. A berry holder
    /// placed before the only Recycle user does not count.
    fn berry_cycling(&self, side: SideId) -> bool {
        let mut cycle = false;
        self.party(side).into_iter().any(|pokemon| {
            let berry = pokemon
                .set_item
                .is_some_and(|item| RESTORATIVE_BERRIES.contains(&item));
            cycle |= matches!(pokemon.set_ability, AbilityId::HARVEST | AbilityId::PICKUP)
                || pokemon.set_moves.contains(&MoveId::RECYCLE);
            berry && cycle
        })
    }

    /// The side's Pokémon in their in-battle order (Showdown `side.pokemon`).
    fn party(&self, side: SideId) -> Vec<&PokemonState> {
        self.state
            .side(side)
            .battle_order
            .iter()
            .filter_map(|&id| self.state.pokemon(id))
            .collect()
    }
}

/// Showdown `pokemon.volatileStaleness || pokemon.staleness`.
fn stale(pokemon: &PokemonState) -> Option<Staleness> {
    pokemon.volatile_staleness.or(pokemon.staleness)
}
