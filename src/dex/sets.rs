//! Sets → battle-ready Pokémon.
//!
//! A `PokemonSet` is what a team file, a scenario fixture or a front end describes: species,
//! level, nature, EVs / IVs, ability, item, moves, all by name. `build_pokemon` resolves the
//! names through the generation's dex and applies the stat formula. Battles never see a set;
//! they hold the resulting `PokemonState`.

use crate::dex::{BaseStats, ResolvedDex};
use crate::ids::{AbilityId, PokemonId, SideId};
use crate::spec::Generation;
use crate::state::{BattleStats, Gender, MoveSlotState, PokemonState, Stat};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Nature {
    Hardy,
    Lonely,
    Brave,
    Adamant,
    Naughty,
    Bold,
    Docile,
    Relaxed,
    Impish,
    Lax,
    Timid,
    Hasty,
    Serious,
    Jolly,
    Naive,
    Modest,
    Mild,
    Quiet,
    Bashful,
    Rash,
    Calm,
    Gentle,
    Sassy,
    Careful,
    Quirky,
}

impl Nature {
    pub const ALL: [Self; 25] = [
        Self::Hardy,
        Self::Lonely,
        Self::Brave,
        Self::Adamant,
        Self::Naughty,
        Self::Bold,
        Self::Docile,
        Self::Relaxed,
        Self::Impish,
        Self::Lax,
        Self::Timid,
        Self::Hasty,
        Self::Serious,
        Self::Jolly,
        Self::Naive,
        Self::Modest,
        Self::Mild,
        Self::Quiet,
        Self::Bashful,
        Self::Rash,
        Self::Calm,
        Self::Gentle,
        Self::Sassy,
        Self::Careful,
        Self::Quirky,
    ];

    /// The stat raised by 10 %, if any.
    pub const fn plus(self) -> Option<Stat> {
        use Stat::*;
        match self {
            Self::Lonely | Self::Brave | Self::Adamant | Self::Naughty => Some(Attack),
            Self::Bold | Self::Relaxed | Self::Impish | Self::Lax => Some(Defense),
            Self::Timid | Self::Hasty | Self::Jolly | Self::Naive => Some(Speed),
            Self::Modest | Self::Mild | Self::Quiet | Self::Rash => Some(SpecialAttack),
            Self::Calm | Self::Gentle | Self::Sassy | Self::Careful => Some(SpecialDefense),
            Self::Hardy | Self::Docile | Self::Serious | Self::Bashful | Self::Quirky => None,
        }
    }

    /// The stat lowered by 10 %, if any.
    pub const fn minus(self) -> Option<Stat> {
        use Stat::*;
        match self {
            Self::Bold | Self::Timid | Self::Modest | Self::Calm => Some(Attack),
            Self::Lonely | Self::Hasty | Self::Mild | Self::Gentle => Some(Defense),
            Self::Brave | Self::Relaxed | Self::Quiet | Self::Sassy => Some(Speed),
            Self::Adamant | Self::Impish | Self::Jolly | Self::Careful => Some(SpecialAttack),
            Self::Naughty | Self::Lax | Self::Naive | Self::Rash => Some(SpecialDefense),
            Self::Hardy | Self::Docile | Self::Serious | Self::Bashful | Self::Quirky => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Hardy => "Hardy",
            Self::Lonely => "Lonely",
            Self::Brave => "Brave",
            Self::Adamant => "Adamant",
            Self::Naughty => "Naughty",
            Self::Bold => "Bold",
            Self::Docile => "Docile",
            Self::Relaxed => "Relaxed",
            Self::Impish => "Impish",
            Self::Lax => "Lax",
            Self::Timid => "Timid",
            Self::Hasty => "Hasty",
            Self::Serious => "Serious",
            Self::Jolly => "Jolly",
            Self::Naive => "Naive",
            Self::Modest => "Modest",
            Self::Mild => "Mild",
            Self::Quiet => "Quiet",
            Self::Bashful => "Bashful",
            Self::Rash => "Rash",
            Self::Calm => "Calm",
            Self::Gentle => "Gentle",
            Self::Sassy => "Sassy",
            Self::Careful => "Careful",
            Self::Quirky => "Quirky",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|nature| nature.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// EVs (0..=252 each) or IVs (0..=31 each) per stat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatSpread {
    pub hp: u8,
    pub atk: u8,
    pub def: u8,
    pub spa: u8,
    pub spd: u8,
    pub spe: u8,
}

impl StatSpread {
    pub const ZERO: Self = Self::uniform(0);
    pub const MAX_IV: Self = Self::uniform(31);

    pub const fn uniform(value: u8) -> Self {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PokemonSet {
    pub species: String,
    pub level: u8,
    pub nature: Nature,
    pub evs: StatSpread,
    pub ivs: StatSpread,
    /// `None` = the species' first ability slot (or no ability before Gen 3).
    pub ability: Option<String>,
    pub item: Option<String>,
    pub moves: Vec<String>,
    /// Showdown `happiness` (Return, Frustration): 255 unless the set says otherwise.
    pub happiness: u8,
    /// `None` = the species' fixed gender (`ResolvedSpecies::gender`), or genderless when it
    /// has a gender ratio: Showdown draws M / F for such a set, and a scenario replay takes
    /// that draw from Showdown's log.
    pub gender: Option<Gender>,
    /// Showdown `set.teraType`: the type Terastallization gives. `None` = the species' first
    /// type (`terastallize`: `set.teraType || types[0]`). Gen 9 only.
    pub tera_type: Option<crate::dex::PokemonType>,
    /// Showdown `set.dynamaxLevel` (Q5, DESIGN.md §44.5): 0-10, 10 when the set names none.
    pub dynamax_level: u8,
    /// Showdown `set.hpType`: Hidden Power's type when the set names one (Gen 7+ Hyper
    /// Training keeps it whatever the IVs; a `Hidden Power Fire` move names it too); `None` =
    /// the type the IVs give.
    pub hp_type: Option<crate::dex::PokemonType>,
}

impl PokemonSet {
    /// Level 100, neutral nature, 0 EVs, 31 IVs, default ability, no item.
    pub fn new(
        species: impl Into<String>,
        moves: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            species: species.into(),
            level: 100,
            nature: Nature::Hardy,
            evs: StatSpread::ZERO,
            ivs: StatSpread::MAX_IV,
            ability: None,
            item: None,
            happiness: 255,
            gender: None,
            tera_type: None,
            dynamax_level: 10,
            hp_type: None,
            moves: moves.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum SetError {
    UnknownSpecies(String),
    UnknownMove(String),
    UnknownAbility(String),
    UnknownItem(String),
    NoMoves,
    TooManyMoves(usize),
    InvalidLevel(u8),
}

/// The stat formula (Bulbapedia "Stat", Generation III onward).
///
/// Generations I–II are stated in the same form, as Showdown and `@smogon/calc` do: IV = 2·DV,
/// EV = the 0..=252 figure that stands for stat experience, no nature, HP DV derived from the
/// low bits of the other four DVs, and one shared Special DV / EV.
pub fn calculate_stats(
    generation: Generation,
    base: BaseStats,
    level: u8,
    nature: Nature,
    evs: StatSpread,
    ivs: StatSpread,
) -> (u16, BattleStats) {
    let legacy = generation <= Generation::Gen2;
    let (evs, ivs, nature) = if legacy {
        // IV = 2·DV can only be even; 31 means DV 15, i.e. 30.
        let even = |iv: u8| (iv >> 1) * 2;
        let ivs = StatSpread {
            hp: legacy_hp_iv(ivs),
            atk: even(ivs.atk),
            def: even(ivs.def),
            spa: even(ivs.spa),
            spd: even(ivs.spa),
            spe: even(ivs.spe),
        };
        let evs = StatSpread {
            spd: evs.spa,
            ..evs
        };
        (evs, ivs, Nature::Hardy)
    } else {
        (evs, ivs, nature)
    };

    let level = u32::from(level);
    let core = |base: u16, iv: u8, ev: u8| -> u32 {
        (2 * u32::from(base) + u32::from(iv) + u32::from(ev) / 4) * level / 100
    };
    let stat = |which: Stat, base: u16, iv: u8, ev: u8| -> u16 {
        let value = core(base, iv, ev) + 5;
        let value = if nature.plus() == Some(which) {
            value * 110 / 100
        } else if nature.minus() == Some(which) {
            value * 90 / 100
        } else {
            value
        };
        value.min(u32::from(u16::MAX)) as u16
    };

    // Shedinja: the only species with base HP 1 always has exactly 1 HP.
    let hp = if base.hp == 1 {
        1
    } else {
        (core(base.hp, ivs.hp, evs.hp) + level + 10).min(u32::from(u16::MAX)) as u16
    };
    (
        hp,
        BattleStats {
            attack: stat(Stat::Attack, base.attack, ivs.atk, evs.atk),
            defense: stat(Stat::Defense, base.defense, ivs.def, evs.def),
            special_attack: stat(Stat::SpecialAttack, base.special_attack, ivs.spa, evs.spa),
            special_defense: stat(Stat::SpecialDefense, base.special_defense, ivs.spd, evs.spd),
            speed: stat(Stat::Speed, base.speed, ivs.spe, evs.spe),
        },
    )
}

/// Gen I–II: the HP DV is the low bit of the Attack, Defense, Speed and Special DVs (8/4/2/1);
/// IV = 2·DV.
fn legacy_hp_iv(ivs: StatSpread) -> u8 {
    let bit = |iv: u8| (iv >> 1) & 1;
    (bit(ivs.atk) * 8 + bit(ivs.def) * 4 + bit(ivs.spe) * 2 + bit(ivs.spa)) * 2
}

/// A move's PP in a set: Showdown gives every move three PP Ups (`calculatePP(move, 3)`),
/// `pp × 8 / 5`; before Gen 3 a 40-PP move loses one PP per PP Up (61, the old games' cap).
/// Trump Card takes none (`MoveBehavior::no_pp_boosts`). The `noPPBoosts` data key (Sketch,
/// Struggle, Revival Blessing) is not exported yet: those moves are not implemented.
/// A move's max PP with the set's three PP Ups (Showdown `calculatePP`; none for a move
/// that takes none).
pub fn pp_with_ups(generation: Generation, move_def: &crate::dex::ResolvedMove) -> u8 {
    let pp = move_def.max_pp;
    if move_def.behavior.no_pp_boosts {
        return pp;
    }
    let boosted = u16::from(pp) * 8 / 5;
    let boosted = if generation <= Generation::Gen2 && pp == 40 {
        boosted - 3
    } else {
        boosted
    };
    boosted.min(u16::from(u8::MAX)) as u8
}

/// Resolves a set against the dex and produces a battle-ready Pokémon.
pub fn build_pokemon(
    dex: &ResolvedDex,
    id: PokemonId,
    side: SideId,
    set: &PokemonSet,
) -> Result<PokemonState, SetError> {
    if !(1..=100).contains(&set.level) {
        return Err(SetError::InvalidLevel(set.level));
    }
    if set.moves.is_empty() {
        return Err(SetError::NoMoves);
    }
    if set.moves.len() > 4 {
        return Err(SetError::TooManyMoves(set.moves.len()));
    }
    let species = dex
        .species_by_name(&set.species)
        .ok_or_else(|| SetError::UnknownSpecies(set.species.clone()))?;

    let ability = match &set.ability {
        Some(name) => dex
            .ability_by_name(name)
            .map(|ability| ability.id)
            .ok_or_else(|| SetError::UnknownAbility(name.clone()))?,
        None => species
            .abilities
            .first()
            .copied()
            .unwrap_or(AbilityId::NONE),
    };
    let held_item = match &set.item {
        Some(name) => Some(
            dex.item_by_name(name)
                .map(|item| item.id)
                .ok_or_else(|| SetError::UnknownItem(name.clone()))?,
        ),
        None => None,
    };
    let moves = set
        .moves
        .iter()
        .map(|name| {
            dex.move_by_name(name)
                .map(|m| MoveSlotState::new(m.id, pp_with_ups(dex.generation, m)))
                .ok_or_else(|| SetError::UnknownMove(name.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let (max_hp, stats) = calculate_stats(
        dex.generation,
        species.base_stats,
        set.level,
        set.nature,
        set.evs,
        set.ivs,
    );

    let mut pokemon = PokemonState::new(id, side);
    pokemon.level = set.level;
    pokemon.hp = max_hp;
    pokemon.max_hp = max_hp;
    pokemon.stats = stats;
    pokemon.types = species.types;
    pokemon.ability = ability;
    pokemon.held_item = held_item;
    pokemon.moves = moves;
    pokemon.species = Some(species.id);
    pokemon.happiness = set.happiness;
    pokemon.nature = set.nature;
    // Showdown `genders[set.gender] || species.gender || sample(['M', 'F'])`: the set first.
    pokemon.gender = set.gender.or(species.gender).unwrap_or(Gender::Genderless);
    // Showdown `this.teraType = this.set.teraType || this.types[0]`.
    pokemon.tera_type = set.tera_type.unwrap_or(pokemon.types.primary);
    pokemon.dynamax_level = set.dynamax_level.min(10);
    // Showdown `this.hpType = set.hpType || hpData.type`: the power comes from the IVs.
    let (hp_type, hp_power) = hidden_power(dex.generation, set.ivs);
    pokemon.hidden_power = (set.hp_type.unwrap_or(hp_type), hp_power);
    pokemon.formes = formes(dex, species, set);
    // Showdown's constructor (`Side`): `canMegaEvo` / `canUltraBurst` are fixed here, from the
    // species, the moves and the item the set gives — a stone picked up later never counts
    // (DESIGN.md §44.3).
    pokemon.set_species = Some(species.id);
    pokemon.set_ability = pokemon.ability;
    pokemon.set_item = pokemon.held_item;
    pokemon.set_moves = pokemon.moves.iter().map(|slot| slot.move_id).collect();
    pokemon.mega_evolves_into = can_mega_evolve(dex, &pokemon, set);
    pokemon.ultra_bursts_into = can_ultra_burst(dex, &pokemon, set);
    Ok(pokemon)
}

/// Every forme of `species` in the dex (the same national dex number) with the values `set`
/// gives it (DESIGN.md §43.1): what a forme change switches to.
fn formes(
    dex: &ResolvedDex,
    species: &crate::dex::ResolvedSpecies,
    set: &PokemonSet,
) -> std::sync::Arc<[crate::state::Forme]> {
    dex.species()
        .iter()
        .filter(|forme| forme.id.same_species(species.id))
        .map(|forme| {
            let (max_hp, stats) = calculate_stats(
                dex.generation,
                forme.base_stats,
                set.level,
                set.nature,
                set.evs,
                set.ivs,
            );
            crate::state::Forme {
                species: forme.id,
                types: forme.types,
                stats,
                max_hp,
                first_ability: forme.abilities.first().copied().unwrap_or(AbilityId::NONE),
                changes_from: forme
                    .changes_from
                    .as_deref()
                    .and_then(|name| dex.species_by_name(name))
                    .map(|from| from.id),
                mega: forme.forme_kind == crate::dex::FormeKind::Mega,
                required_move: forme.required_move,
            }
        })
        .collect()
}

/// Showdown `canMegaEvo`: Mega Rayquaza (a mega forme whose `requiredMove` is in the moves,
/// with no Z-Crystal held), or the stone's forme — the species' own name first, then the family
/// base's, never a forme the Pokémon is already in.
fn can_mega_evolve(
    dex: &ResolvedDex,
    pokemon: &crate::state::PokemonState,
    set: &PokemonSet,
) -> Option<crate::ids::SpeciesId> {
    let held = set.item.as_deref().and_then(|name| dex.item_by_name(name));
    let z_crystal = held.is_some_and(|item| item.z_crystal.is_some());
    let knows = |move_id: crate::ids::MoveId| {
        set.moves.iter().any(|name| {
            dex.move_by_name(name)
                .is_some_and(|move_def| move_def.id == move_id)
        })
    };
    if !z_crystal {
        let required = pokemon
            .formes
            .iter()
            .find(|forme| forme.mega && forme.required_move.is_some())
            .filter(|forme| forme.required_move.is_some_and(knows));
        if let Some(forme) = required {
            return Some(forme.species);
        }
    }
    let (holder, forme) = held.and_then(|item| item.mega_stone)?;
    let current = dex.species_by_name(&set.species).map(|species| species.id);
    (holder.same_species(current?) && current != Some(forme)).then_some(forme)
}

/// Showdown `canUltraBurst`: Necrozma's two winged formes holding the Ultranecrozium Z.
fn can_ultra_burst(
    dex: &ResolvedDex,
    pokemon: &crate::state::PokemonState,
    set: &PokemonSet,
) -> Option<crate::ids::SpeciesId> {
    let held = set.item.as_deref().and_then(|name| dex.item_by_name(name));
    if !held.is_some_and(|item| item.id == crate::ids::ItemId::ULTRANECROZIUM_Z) {
        return None;
    }
    let winged = pokemon.set_species.is_some_and(|species| {
        species == crate::ids::SpeciesId::NECROZMA_DUSK_MANE
            || species == crate::ids::SpeciesId::NECROZMA_DAWN_WINGS
    });
    if !winged {
        return None;
    }
    pokemon
        .formes
        .iter()
        .find(|forme| forme.species == crate::ids::SpeciesId::NECROZMA_ULTRA)
        .map(|forme| forme.species)
}

/// Hidden Power's type and power from IVs (Showdown `getHiddenPower`): Gen 2 from the DVs
/// (IV / 2), Gen 3+ from each IV's low bits (power 30-70; the move itself fixes 60 from Gen 6).
pub fn hidden_power(generation: Generation, ivs: StatSpread) -> (crate::dex::PokemonType, u8) {
    use crate::dex::PokemonType as T;
    const TYPES: [T; 16] = [
        T::Fighting,
        T::Flying,
        T::Poison,
        T::Ground,
        T::Rock,
        T::Bug,
        T::Ghost,
        T::Steel,
        T::Fire,
        T::Water,
        T::Grass,
        T::Electric,
        T::Psychic,
        T::Ice,
        T::Dragon,
        T::Dark,
    ];
    if generation <= Generation::Gen2 {
        let (atk, def, spe, spc) = (ivs.atk / 2, ivs.def / 2, ivs.spe / 2, ivs.spa / 2);
        let kind = TYPES[usize::from(4 * (atk % 4) + def % 4)];
        let bits = u16::from(spc >> 3)
            + 2 * u16::from(spe >> 3)
            + 4 * u16::from(def >> 3)
            + 8 * u16::from(atk >> 3);
        let power = (5 * bits + u16::from(spc % 4)) / 2 + 31;
        return (kind, power as u8);
    }
    // Showdown's stat order: HP, Attack, Defense, Speed, Special Attack, Special Defense.
    let order = [ivs.hp, ivs.atk, ivs.def, ivs.spe, ivs.spa, ivs.spd];
    let (mut kind_bits, mut power_bits) = (0u16, 0u16);
    for (index, iv) in order.iter().enumerate() {
        kind_bits += (1 << index) * u16::from(iv % 2);
        power_bits += (1 << index) * u16::from((iv / 2) % 2);
    }
    let kind = TYPES[usize::from(kind_bits * 15 / 63)];
    (kind, (power_bits * 40 / 63 + 30) as u8)
}
