use crate::dex::PokemonType;
use crate::ids::{MoveId, MoveSlot, PokemonId, PositionId, SideId};
use crate::spec::Generation;
use crate::state::BattlePhase;

use super::ChoiceError;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum BattleInitError {
    DexGenerationMismatch {
        rules: Generation,
        dex: Generation,
    },
    ActiveSlotCountMismatch {
        side: SideId,
        expected: usize,
        actual: usize,
    },
    DuplicatePokemonId(PokemonId),
    DuplicateTeamPokemon {
        side: SideId,
        pokemon: PokemonId,
    },
    DuplicateActivePokemon {
        side: SideId,
        pokemon: PokemonId,
    },
    UnassignedPokemon(PokemonId),
    UnknownTeamPokemon {
        side: SideId,
        pokemon: PokemonId,
    },
    PokemonSideMismatch {
        pokemon: PokemonId,
        expected: SideId,
        actual: SideId,
    },
    ActivePokemonNotOnTeam {
        side: SideId,
        pokemon: PokemonId,
    },
    ActivePokemonFainted(PokemonId),
    SnapshotRulesMismatch,
    SnapshotDexMismatch,
    /// The snapshot's phase is not one a battle hands control back in with its pending
    /// execution (a state edited out of a snapshot).
    SnapshotPhaseMismatch {
        phase: BattlePhase,
    },
    UnavailablePokemonType {
        pokemon: PokemonId,
        pokemon_type: PokemonType,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum BattleError {
    WrongPhase {
        expected: BattlePhase,
        actual: BattlePhase,
    },
    Choice(ChoiceError),
    UnknownPokemon(PokemonId),
    UnknownMove(MoveId),
    InvalidMoveSlot {
        actor: PokemonId,
        slot: MoveSlot,
    },
    NoPp {
        actor: PokemonId,
        move_id: MoveId,
    },
    ActorNotActive(PokemonId),
    InvalidReplacementPosition(PositionId),
}

impl From<ChoiceError> for BattleError {
    fn from(value: ChoiceError) -> Self {
        Self::Choice(value)
    }
}
