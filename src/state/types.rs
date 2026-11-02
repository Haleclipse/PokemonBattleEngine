use crate::dex::PokemonType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PokemonTypes {
    #[serde(with = "crate::snapshot::wire")]
    pub primary: PokemonType,
    #[serde(with = "crate::snapshot::wire")]
    pub secondary: Option<PokemonType>,
}

impl PokemonTypes {
    pub const fn mono(primary: PokemonType) -> Self {
        Self {
            primary,
            secondary: None,
        }
    }

    pub const fn dual(primary: PokemonType, secondary: PokemonType) -> Self {
        Self {
            primary,
            secondary: Some(secondary),
        }
    }

    pub fn contains(self, type_: PokemonType) -> bool {
        self.primary == type_ || self.secondary == Some(type_)
    }

    pub fn iter(self) -> impl Iterator<Item = PokemonType> {
        [Some(self.primary), self.secondary].into_iter().flatten()
    }
}
