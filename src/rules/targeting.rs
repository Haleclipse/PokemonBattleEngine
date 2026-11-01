use crate::ids::{PokemonId, PositionId, SideId};
use crate::state::BattleState;

use super::BattleTopology;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPattern {
    SelfOnly,
    /// Showdown `any`: any other Pokémon, adjacent or not (Aura Sphere …).
    SelectedAny,
    /// Showdown `normal`: any adjacent Pokémon, an ally included (Tackle …).
    SelectedAdjacent,
    SelectedFoe,
    /// Showdown `adjacentFoe`.
    SelectedAdjacentFoe,
    /// Showdown `randomNormal` (Struggle): no target is chosen; a random adjacent foe
    /// (`getRandomTarget`), the foe in a single battle.
    RandomAdjacentFoe,
    Ally,
    AllyOrSelf,
    AllFoes,
    AllAdjacentFoes,
    AllAdjacent,
    UserSide,
    /// Showdown `allyTeam` (Heal Bell): the user's whole team, bench included; resolved as the
    /// user's side, the effect walks the team.
    UserTeam,
    /// Showdown `allies` (Howl, Life Dew): the user and its active allies.
    UserAndAllies,
    OpponentSide,
    Field,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredTarget(pub PositionId);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTarget {
    Pokemon(PokemonId),
    Side(SideId),
    Field,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTargets {
    pub targets: Vec<ResolvedTarget>,
}

impl ResolvedTargets {
    pub fn pokemon_ids(&self) -> impl Iterator<Item = PokemonId> + '_ {
        self.targets.iter().filter_map(|target| match target {
            ResolvedTarget::Pokemon(pokemon) => Some(*pokemon),
            _ => None,
        })
    }

    pub fn pokemon_count(&self) -> usize {
        self.pokemon_ids().count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum TargetError {
    ActorNotActive(PokemonId),
    TargetRequired,
    InvalidPosition(PositionId),
    TargetUnavailable(PositionId),
    WrongSide(PositionId),
    NotAdjacent(PositionId),
    SelfNotAllowed(PositionId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TargetingRules;

impl TargetingRules {
    /// Showdown `validTargetLoc`: is the declared position a legal choice for the pattern, by
    /// position alone (side, adjacency, not the user itself)? Whether a Pokémon stands there is
    /// an execution-time question (`resolve`, then the Shell's retarget / fail): a choice may
    /// name a slot emptied by a faint.
    pub fn validate_position(
        &self,
        topology: &BattleTopology,
        state: &BattleState,
        actor: PokemonId,
        pattern: TargetPattern,
        declared: Option<DeclaredTarget>,
    ) -> Result<(), TargetError> {
        let actor_position = state
            .active_position(actor, topology)
            .ok_or(TargetError::ActorNotActive(actor))?;
        let actor_side = topology
            .spec(actor_position)
            .ok_or(TargetError::InvalidPosition(actor_position))?
            .side;
        if !matches!(
            pattern,
            TargetPattern::SelectedAny
                | TargetPattern::SelectedAdjacent
                | TargetPattern::SelectedFoe
                | TargetPattern::SelectedAdjacentFoe
                | TargetPattern::Ally
                | TargetPattern::AllyOrSelf
        ) {
            return Ok(());
        }
        // A single battle takes a move without a target (Showdown asks for one only when a side
        // has two or more active Pokémon); the target is implicit (`resolve`).
        if declared.is_none() && topology.active_slots_per_side() == 1 {
            return Ok(());
        }
        let position = declared.ok_or(TargetError::TargetRequired)?.0;
        let spec = topology
            .spec(position)
            .ok_or(TargetError::InvalidPosition(position))?;
        let is_self = position == actor_position;
        match pattern {
            TargetPattern::SelectedAny if is_self => Err(TargetError::SelfNotAllowed(position)),
            TargetPattern::SelectedAdjacent if is_self => {
                Err(TargetError::SelfNotAllowed(position))
            }
            TargetPattern::SelectedAdjacent
                if !topology.all_adjacent(actor_position).contains(&position) =>
            {
                Err(TargetError::NotAdjacent(position))
            }
            TargetPattern::SelectedFoe | TargetPattern::SelectedAdjacentFoe
                if spec.side == actor_side =>
            {
                Err(TargetError::WrongSide(position))
            }
            TargetPattern::SelectedAdjacentFoe
                if !topology.adjacent_foes(actor_position).contains(&position) =>
            {
                Err(TargetError::NotAdjacent(position))
            }
            TargetPattern::Ally | TargetPattern::AllyOrSelf if spec.side != actor_side => {
                Err(TargetError::WrongSide(position))
            }
            TargetPattern::Ally if is_self => Err(TargetError::SelfNotAllowed(position)),
            _ => Ok(()),
        }
    }

    pub fn resolve(
        &self,
        topology: &BattleTopology,
        state: &BattleState,
        actor: PokemonId,
        pattern: TargetPattern,
        declared: Option<DeclaredTarget>,
    ) -> Result<ResolvedTargets, TargetError> {
        let actor_position = state
            .active_position(actor, topology)
            .ok_or(TargetError::ActorNotActive(actor))?;
        let actor_side = topology
            .spec(actor_position)
            .ok_or(TargetError::InvalidPosition(actor_position))?
            .side;

        match pattern {
            TargetPattern::SelfOnly => Ok(ResolvedTargets {
                targets: vec![ResolvedTarget::Pokemon(actor)],
            }),
            // Whatever was declared: the foe in a single battle; with several, the Shell picks
            // one at random (`TargetRequired` → `random_retarget_candidates`).
            TargetPattern::RandomAdjacentFoe => self.resolve(
                topology,
                state,
                actor,
                TargetPattern::SelectedAdjacentFoe,
                None,
            ),
            TargetPattern::AllFoes => Ok(ResolvedTargets {
                targets: self.resolve_positions(topology.all_foes(actor_position), topology, state),
            }),
            TargetPattern::AllAdjacentFoes => Ok(ResolvedTargets {
                targets: self.resolve_positions(
                    topology.adjacent_foes(actor_position),
                    topology,
                    state,
                ),
            }),
            TargetPattern::AllAdjacent => Ok(ResolvedTargets {
                targets: self.resolve_positions(
                    topology.all_adjacent(actor_position),
                    topology,
                    state,
                ),
            }),
            // Showdown `alliesAndSelf()`: the user's side in position order.
            TargetPattern::UserAndAllies => {
                let mut positions = vec![actor_position];
                positions.extend(topology.adjacent_allies(actor_position));
                positions.sort();
                Ok(ResolvedTargets {
                    targets: self.resolve_positions(positions, topology, state),
                })
            }
            TargetPattern::UserSide | TargetPattern::UserTeam => Ok(ResolvedTargets {
                targets: vec![ResolvedTarget::Side(actor_side)],
            }),
            TargetPattern::OpponentSide => Ok(ResolvedTargets {
                targets: vec![ResolvedTarget::Side(actor_side.other())],
            }),
            TargetPattern::Field => Ok(ResolvedTargets {
                targets: vec![ResolvedTarget::Field],
            }),
            TargetPattern::SelectedAny
            | TargetPattern::SelectedAdjacent
            | TargetPattern::SelectedFoe
            | TargetPattern::SelectedAdjacentFoe
            | TargetPattern::Ally
            | TargetPattern::AllyOrSelf => {
                // No target chosen, in a single battle (Showdown `getRandomTarget`): a move
                // that may target its user targets it, one that needs an ally has nothing to
                // target (the move fails), any other targets the foe.
                let declared = match declared {
                    Some(declared) => declared,
                    None if topology.active_slots_per_side() == 1 => match pattern {
                        TargetPattern::AllyOrSelf => {
                            return Ok(ResolvedTargets {
                                targets: vec![ResolvedTarget::Pokemon(actor)],
                            });
                        }
                        TargetPattern::Ally => {
                            return Ok(ResolvedTargets {
                                targets: Vec::new(),
                            });
                        }
                        _ => DeclaredTarget(
                            topology
                                .position(actor_side.other(), 0)
                                .ok_or(TargetError::TargetRequired)?,
                        ),
                    },
                    None => return Err(TargetError::TargetRequired),
                };
                let position = declared.0;
                let target_spec = topology
                    .spec(position)
                    .ok_or(TargetError::InvalidPosition(position))?;
                // One that fainted during the action keeps its slot until the faints are
                // resolved, and Showdown's `getTarget` passes it over (`target.fainted`): a foe
                // for a random one, an ally for no target — as the spread case filters it
                // (a dance that knocked out its target, R-M8-129).
                let target = state
                    .pokemon_at_position(position, topology)
                    .filter(|pokemon| state.pokemon(*pokemon).is_some_and(|p| !p.is_fainted()))
                    .ok_or(TargetError::TargetUnavailable(position))?;

                match pattern {
                    TargetPattern::SelectedAny => {
                        if target == actor {
                            return Err(TargetError::SelfNotAllowed(position));
                        }
                    }
                    TargetPattern::SelectedAdjacent => {
                        if target == actor {
                            return Err(TargetError::SelfNotAllowed(position));
                        }
                        if !topology.all_adjacent(actor_position).contains(&position) {
                            return Err(TargetError::NotAdjacent(position));
                        }
                    }
                    TargetPattern::SelectedFoe | TargetPattern::SelectedAdjacentFoe => {
                        if target_spec.side == actor_side {
                            return Err(TargetError::WrongSide(position));
                        }
                        if pattern == TargetPattern::SelectedAdjacentFoe
                            && !topology.adjacent_foes(actor_position).contains(&position)
                        {
                            return Err(TargetError::NotAdjacent(position));
                        }
                    }
                    TargetPattern::Ally => {
                        if target_spec.side != actor_side {
                            return Err(TargetError::WrongSide(position));
                        }
                        if target == actor {
                            return Err(TargetError::SelfNotAllowed(position));
                        }
                    }
                    TargetPattern::AllyOrSelf => {
                        if target_spec.side != actor_side {
                            return Err(TargetError::WrongSide(position));
                        }
                    }
                    _ => unreachable!(),
                }

                Ok(ResolvedTargets {
                    targets: vec![ResolvedTarget::Pokemon(target)],
                })
            }
        }
    }

    fn resolve_positions(
        &self,
        positions: Vec<PositionId>,
        topology: &BattleTopology,
        state: &BattleState,
    ) -> Vec<ResolvedTarget> {
        positions
            .into_iter()
            .filter_map(|position| state.pokemon_at_position(position, topology))
            .filter(|pokemon| state.pokemon(*pokemon).is_some_and(|p| !p.is_fainted()))
            .map(ResolvedTarget::Pokemon)
            .collect()
    }
}
