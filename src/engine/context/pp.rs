//! PP taken by something else than a move's own use (DESIGN.md §15.15): Showdown `deductPP`,
//! Spite, Eerie Spell and Pressure.

use crate::dex::ResolvedMove;
use crate::engine::{BattleTrace, ConditionApplyError, EffectSource, TraceEvent};
use crate::ids::{MoveId, PokemonId, SideId};
use crate::rules::{PressureReach, ResolvedRules, ResolvedTargets, SpiteAmount, TargetPattern};
use crate::state::{BattleState, RandomRequestKind, VolatileConditionKind};

use super::BattleContext;

impl<'a> BattleContext<'a> {
    /// Showdown `pokemon.deductPP(move, amount)`: the first slot holding `move_id` counts as
    /// used (Last Resort) and loses `amount` PP, down to 0. Returns the PP it lost: 0 when the
    /// Pokémon has no such slot (Struggle) or none left.
    pub fn deduct_pp(
        &mut self,
        pokemon: PokemonId,
        move_id: MoveId,
        amount: u8,
        source: EffectSource,
    ) -> u8 {
        let Some(slot) = self.state.pokemon_mut(pokemon).and_then(|p| {
            p.move_slots_mut()
                .iter_mut()
                .find(|slot| slot.move_id == move_id)
        }) else {
            return 0;
        };
        slot.used = true;
        let taken = amount.min(slot.pp);
        if taken == 0 {
            return 0;
        }
        slot.pp -= taken;
        let remaining = slot.pp;
        self.trace.push(TraceEvent::PpReduced {
            pokemon,
            move_id,
            remaining,
            source,
        });
        taken
    }

    /// Spite on `target` (Showdown `spite.onHit`): its last move loses PP — Gen 4+ 4, Gen 2-3 a
    /// draw of 2-5, drawn once the checks passed (the cartridge, issues.md I71). It fails when
    /// the target has used no move since it came in, or none it knows (Struggle), or the move has
    /// too little PP left (`SpiteRules::min_pp`). Gen 3: a move it empties ends the target's
    /// multi-turn move.
    pub fn spite(&mut self, user: PokemonId, target: PokemonId) -> Result<(), ConditionApplyError> {
        let rules = self.rules.conditions.pp.spite;
        let pokemon = self
            .state
            .pokemon(target)
            .filter(|p| !p.is_fainted())
            .ok_or(ConditionApplyError::InvalidTarget)?;
        // `move.isZ` fails Spite, before a Max Move is read through its `baseMove` (DESIGN.md
        // §44.5): a status move used as a Z-Move, in its slot with PP (R-M9-36), and a Z-Move a
        // custom game put in a slot, whose one PP a Leppa Berry gives back (R-M10-20).
        if self.last_move_is_z(pokemon) {
            return Err(ConditionApplyError::Immune);
        }
        let last = pokemon
            .last_move_resolved()
            .ok_or(ConditionApplyError::Immune)?;
        let left = pokemon
            .move_slots()
            .iter()
            .find(|slot| slot.move_id == last)
            .map(|slot| slot.pp);
        let source = EffectSource::Move {
            pokemon: user,
            move_id: MoveId::SPITE,
        };
        let taken = match rules.amount {
            SpiteAmount::Fixed(amount) => self.deduct_pp(target, last, amount, source),
            SpiteAmount::Rolled { min, max } => {
                if left.is_none_or(|pp| pp < rules.min_pp) {
                    return Err(ConditionApplyError::Immune);
                }
                let amount = self.state.rng.range_inclusive_for(
                    RandomRequestKind::PpDrop,
                    u16::from(min),
                    u16::from(max),
                ) as u8;
                self.deduct_pp(target, last, amount, source)
            }
        };
        if taken == 0 {
            return Err(ConditionApplyError::Immune);
        }
        if rules.emptied_cancels_multi_turn && left == Some(taken) {
            self.cancel_multi_turn_moves(target);
        }
        Ok(())
    }

    /// Eerie Spell's secondary (Showdown `secondary.onHit`): the target's last move loses 3 PP,
    /// if it is still standing. True when it lost any.
    pub fn eerie_spell(&mut self, user: PokemonId, target: PokemonId, move_id: MoveId) -> bool {
        let Some(last) = self
            .state
            .pokemon(target)
            .filter(|p| !p.is_fainted() && !self.last_move_is_z(p))
            .and_then(|p| p.last_move_resolved())
        else {
            return false;
        };
        let source = EffectSource::Move {
            pokemon: user,
            move_id,
        };
        self.deduct_pp(target, last, 3, source) > 0
    }

    /// Showdown `lastMove.isZ` (Spite, Eerie Spell): the last move is a Z-Move — a status move
    /// used as one (`last_move_z`), or a move that is one (a custom game's, in a slot).
    fn last_move_is_z(&self, pokemon: &crate::state::PokemonState) -> bool {
        pokemon.last_move_z
            || pokemon
                .last_move
                .and_then(|last| self.dex?.move_by_id(last))
                .is_some_and(|last| last.z_crystal.is_some())
    }

    /// pokeemerald `CancelMultiTurnMoves`: the Pokémon's multi-turn move ends at once, silently —
    /// a two-turn move's charge (and its hiding place), Thrash's lock (no confusion), Uproar,
    /// Bide, Rollout's chain, Fury Cutter's count; the observers see none of it end.
    pub fn cancel_multi_turn_moves(&mut self, pokemon: PokemonId) {
        self.end_volatile(pokemon, VolatileConditionKind::TwoTurnMove);
        self.end_rampage(pokemon, false);
        self.cancel_uproar(pokemon);
        self.end_volatile_unseen(pokemon, VolatileConditionKind::Bide);
        self.end_volatile(pokemon, VolatileConditionKind::Rolling);
        self.end_fury_cutter(pokemon);
    }
}

/// A Pokémon taking `from`'s move over (Magic Coat, Snatch: `taking`) pays one PP for its
/// Pressure, whatever their sides, where the generation has it (`CallRules::taker_pays_pressure`,
/// issues.md I169): Gen 3 from the taking move's slot, Gen 4 from the slot it chose this turn
/// (its last move — a called move is nobody's).
pub(crate) fn takes_over_under_pressure(
    state: &mut BattleState,
    rules: &ResolvedRules,
    trace: &mut BattleTrace,
    taker: PokemonId,
    from: PokemonId,
    taking: MoveId,
) {
    let Some(paid_from) = rules.conditions.calls.taker_pays_pressure else {
        return;
    };
    if !state.pokemon(from).is_some_and(|pokemon| {
        crate::engine::ability_handler(state, rules, pokemon, |b| b.pressure)
    }) {
        return;
    }
    let charged = match paid_from {
        crate::rules::PressurePaidFrom::TheTakingMove => Some(taking),
        crate::rules::PressurePaidFrom::TheChosenMove => {
            state.pokemon(taker).and_then(|pokemon| pokemon.last_move)
        }
    };
    if let Some(charged) = charged {
        BattleContext::new(state, rules, trace).deduct_pp(
            taker,
            charged,
            1,
            EffectSource::Ability {
                pokemon: from,
                ability: crate::ids::AbilityId::PRESSURE,
            },
        );
    }
}

/// The Pokémon whose Pressure `user`'s move meets as it starts (Showdown `useMoveInner`'s
/// `DeductPP` over `pressureTargets`; the generation's reach, `PpRules::pressure`): standing,
/// on the field, not the user, its Pressure in effect. `targets` are the move's after
/// redirection.
pub(crate) fn pressure_holders(
    state: &BattleState,
    rules: &ResolvedRules,
    user: PokemonId,
    move_def: &ResolvedMove,
    targets: &ResolvedTargets,
) -> Vec<PokemonId> {
    let Some(side) = state.pokemon(user).map(|p| p.side) else {
        return Vec::new();
    };
    let standing = |side: SideId| -> Vec<PokemonId> {
        state
            .side(side)
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|id| state.pokemon(*id).is_some_and(|p| !p.is_fainted()))
            .collect()
    };
    let foes = standing(side.other());
    let others: Vec<PokemonId> = standing(side)
        .into_iter()
        .filter(|id| *id != user)
        .chain(foes.iter().copied())
        .collect();
    let aimed: Vec<PokemonId> = targets.pokemon_ids().collect();
    let reach: Vec<PokemonId> = match rules.conditions.pp.pressure {
        PressureReach::Targets => {
            if move_def.flags.must_pressure {
                foes
            } else {
                match move_def.target {
                    TargetPattern::Field => foes,
                    TargetPattern::OpponentSide => Vec::new(),
                    _ => aimed,
                }
            }
        }
        PressureReach::Gen4 => {
            if move_def.id == MoveId::IMPRISON {
                foes
            } else if matches!(
                move_def.behavior.damage_override,
                Some(crate::dex::FixedDamage::Retaliation(_))
            ) {
                Vec::new()
            } else {
                match move_def.target {
                    TargetPattern::Field | TargetPattern::AllAdjacent => others,
                    TargetPattern::AllAdjacentFoes
                    | TargetPattern::AllFoes
                    | TargetPattern::OpponentSide => foes,
                    TargetPattern::SelfOnly
                    | TargetPattern::Ally
                    | TargetPattern::AllyOrSelf
                    | TargetPattern::UserSide
                    | TargetPattern::UserTeam
                    | TargetPattern::UserAndAllies => Vec::new(),
                    _ => aimed,
                }
            }
        }
        PressureReach::Gen3 => {
            if move_def.id == MoveId::IMPRISON {
                foes
            } else if move_def.id == MoveId::PERISH_SONG {
                others
            } else {
                match move_def.target {
                    TargetPattern::AllAdjacent => others,
                    TargetPattern::AllAdjacentFoes
                    | TargetPattern::AllFoes
                    | TargetPattern::OpponentSide => foes,
                    TargetPattern::Field
                    | TargetPattern::SelfOnly
                    | TargetPattern::Ally
                    | TargetPattern::AllyOrSelf
                    | TargetPattern::UserSide
                    | TargetPattern::UserTeam
                    | TargetPattern::UserAndAllies => Vec::new(),
                    _ => aimed,
                }
            }
        }
    };
    let spares_side = rules.conditions.pp.pressure == PressureReach::Targets;
    // Gen 3-4 check the target's ability whatever its HP (pokeemerald `Cmd_ppreduce`,
    // pokeplatinum `BattleControllerPlayer_DecrementPP`): a foreseen attack aimed at a foe that
    // fell still meets its Pressure (R-M11-155). From Gen 5 a Pokémon off the field has none
    // (Showdown `ignoringAbility`).
    let fallen_count = rules.conditions.pp.pressure != PressureReach::Targets;
    reach
        .into_iter()
        .filter(|id| *id != user)
        .filter(|id| !(spares_side && state.pokemon(*id).is_some_and(|p| p.side == side)))
        .filter(|id| {
            state.pokemon(*id).is_some_and(|p| {
                (fallen_count || !p.is_fainted())
                    && crate::engine::ability_handler(state, rules, p, |b| b.pressure)
            })
        })
        .collect()
}
