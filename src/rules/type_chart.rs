use crate::dex::PokemonType;
use crate::spec::Generation;

use super::Fraction;

/// The 18 types, Showdown's `Stellar` (Terastallization only, `PokemonType::Stellar`) and its
/// `???` (`PokemonType::Typeless`), all three neutral both ways in the table itself.
const TYPE_COUNT: usize = 20;

/// Fully resolved per-generation type chart. Battle hot paths never branch on Generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeChart {
    table: [[Fraction; TYPE_COUNT]; TYPE_COUNT],
    available: [bool; TYPE_COUNT],
}

impl TypeChart {
    /// Builds the generation's chart from the committed `@pkmn/data` snapshot. Type
    /// availability (no Dark / Steel in Gen 1, no Fairy before Gen 6) and every historical
    /// quirk (Gen 1 Ghost vs Psychic) come from the data, not from code.
    pub fn for_generation(generation: Generation) -> Self {
        let snapshot = crate::data::snapshot(generation);
        let mut chart = Self {
            table: [[Fraction::ONE; TYPE_COUNT]; TYPE_COUNT],
            available: [false; TYPE_COUNT],
        };
        for name in &snapshot.types.names {
            if let Some(pokemon_type) = PokemonType::from_name(name) {
                chart.available[pokemon_type.index()] = true;
            }
        }
        for (attack_name, row) in &snapshot.types.chart {
            let Some(attack) = PokemonType::from_name(attack_name) else {
                continue;
            };
            for (defense_name, multiplier) in row {
                let Some(defense) = PokemonType::from_name(defense_name) else {
                    continue;
                };
                chart.set(attack, defense, fraction_from_multiplier(*multiplier));
            }
        }
        chart
    }

    fn set(&mut self, attack: PokemonType, defense: PokemonType, modifier: Fraction) {
        self.table[attack.index()][defense.index()] = modifier;
    }

    pub fn is_available(&self, pokemon_type: PokemonType) -> bool {
        self.available[pokemon_type.index()]
    }

    pub fn effectiveness(&self, attack: PokemonType, defense: PokemonType) -> Fraction {
        self.table[attack.index()][defense.index()]
    }

    pub fn combined_effectiveness(
        &self,
        attack: PokemonType,
        defenders: impl IntoIterator<Item = PokemonType>,
    ) -> Fraction {
        defenders
            .into_iter()
            .fold(Fraction::ONE, |current, defender| {
                current.multiply(self.effectiveness(attack, defender))
            })
    }
}

fn fraction_from_multiplier(multiplier: f64) -> Fraction {
    // Floating-point literals are not patterns in edition 2024; compare explicitly.
    if multiplier == 0.0 {
        Fraction::ZERO
    } else if multiplier == 0.25 {
        Fraction::new(1, 4)
    } else if multiplier == 0.5 {
        Fraction::new(1, 2)
    } else if multiplier == 2.0 {
        Fraction::new(2, 1)
    } else if multiplier == 4.0 {
        Fraction::new(4, 1)
    } else {
        Fraction::ONE
    }
}
