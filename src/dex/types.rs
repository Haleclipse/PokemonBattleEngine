#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PokemonType {
    Normal,
    Fire,
    Water,
    Electric,
    Grass,
    Ice,
    Fighting,
    Poison,
    Ground,
    Flying,
    Psychic,
    Bug,
    Rock,
    Ghost,
    Dragon,
    Dark,
    Steel,
    Fairy,
    /// Showdown `Stellar`: the Terastallization-only type (Gen 9, DESIGN.md §44.1). Never a
    /// Pokémon's own type, so not in `ALL` and refused by `setType`; a move can have it (Tera
    /// Blast, Tera Starstorm) and then has no effectiveness of its own — against a
    /// Terastallized target it is always super effective (`Target::runEffectiveness`).
    Stellar,
    /// Showdown `???`: no type at all (Struggle from Gen 2). Neutral against every type,
    /// never a Pokémon's type, so never STAB; not in `ALL`.
    Typeless,
}

impl PokemonType {
    pub const ALL: [Self; 18] = [
        Self::Normal,
        Self::Fire,
        Self::Water,
        Self::Electric,
        Self::Grass,
        Self::Ice,
        Self::Fighting,
        Self::Poison,
        Self::Ground,
        Self::Flying,
        Self::Psychic,
        Self::Bug,
        Self::Rock,
        Self::Ghost,
        Self::Dragon,
        Self::Dark,
        Self::Steel,
        Self::Fairy,
    ];

    /// The types whose moves were physical before the physical / special split (the gen3 mod's
    /// `physicalTypes`): Normal, Fighting, Flying, Poison, Ground, Rock, Bug, Ghost, Steel.
    pub const fn physical_before_split(self) -> bool {
        matches!(
            self,
            Self::Normal
                | Self::Fighting
                | Self::Flying
                | Self::Poison
                | Self::Ground
                | Self::Rock
                | Self::Bug
                | Self::Ghost
                | Self::Steel
        )
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Normal => 0,
            Self::Fire => 1,
            Self::Water => 2,
            Self::Electric => 3,
            Self::Grass => 4,
            Self::Ice => 5,
            Self::Fighting => 6,
            Self::Poison => 7,
            Self::Ground => 8,
            Self::Flying => 9,
            Self::Psychic => 10,
            Self::Bug => 11,
            Self::Rock => 12,
            Self::Ghost => 13,
            Self::Dragon => 14,
            Self::Dark => 15,
            Self::Steel => 16,
            Self::Fairy => 17,
            Self::Stellar => 18,
            Self::Typeless => 19,
        }
    }

    /// Showdown / Bulbapedia display name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Fire => "Fire",
            Self::Water => "Water",
            Self::Electric => "Electric",
            Self::Grass => "Grass",
            Self::Ice => "Ice",
            Self::Fighting => "Fighting",
            Self::Poison => "Poison",
            Self::Ground => "Ground",
            Self::Flying => "Flying",
            Self::Psychic => "Psychic",
            Self::Bug => "Bug",
            Self::Rock => "Rock",
            Self::Ghost => "Ghost",
            Self::Dragon => "Dragon",
            Self::Dark => "Dark",
            Self::Steel => "Steel",
            Self::Fairy => "Fairy",
            Self::Stellar => "Stellar",
            Self::Typeless => "???",
        }
    }

    /// Parses a Showdown / Bulbapedia display name (case-insensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Stellar, Self::Typeless])
            .find(|candidate| candidate.name().eq_ignore_ascii_case(name.trim()))
    }
}
