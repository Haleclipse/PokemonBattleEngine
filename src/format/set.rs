//! A Pokémon of a team as it is registered (Showdown `PokemonSet`, DESIGN.md §45.3): the names
//! as the player wrote them. The validator normalises a set as Showdown's does — the species'
//! name, the out-of-battle forme, the moves a forme changes — so the set it hands back is the
//! one a battle is built from.

/// Six stats (EVs, IVs): HP, Attack, Defense, Special Attack, Special Defense, Speed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub hp: i32,
    pub atk: i32,
    pub def: i32,
    pub spa: i32,
    pub spd: i32,
    pub spe: i32,
}

impl Stats {
    /// Showdown `TeamValidator.fillStats(null, value)`.
    pub const fn filled(value: i32) -> Self {
        Self {
            hp: value,
            atk: value,
            def: value,
            spa: value,
            spd: value,
            spe: value,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamSet {
    /// The nickname (empty: none).
    pub name: String,
    pub species: String,
    pub item: String,
    pub ability: String,
    pub moves: Vec<String>,
    pub nature: String,
    /// `M`, `F`, or empty.
    pub gender: String,
    pub evs: Option<Stats>,
    pub ivs: Option<Stats>,
    /// 0: not given (the format's default level).
    pub level: u32,
    pub shiny: bool,
    pub happiness: Option<f64>,
    pub hp_type: String,
    pub gigantamax: bool,
    pub tera_type: String,
}
