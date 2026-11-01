use crate::dex::PokemonType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PokemonTypes {
    pub primary: PokemonType,
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
