//! A set's stats (M9.2c-2, DESIGN.md §45.3): Showdown's `validateStats` — EVs, IVs (Gen 1-2
//! DVs), Hidden Power, a Gen 6+ legendary's three perfect IVs, the gender and shininess Gen 2
//! DVs fix — with the sources (`PokemonSources`) as they stand without a learnset: the
//! generations a Pokémon can come from, the learnset seam's empty side (§45.3).

use super::data::to_id;
use super::legality::SpeciesFacts;
use super::set::{Stats, TeamSet};
use super::sources::Sources;
use super::validate::{Problem, ProblemKind, SetField, TeamValidator};

/// `Dex.stats.names`.
const STAT_NAMES: [&str; 6] = [
    "HP",
    "Attack",
    "Defense",
    "Special Attack",
    "Special Defense",
    "Speed",
];

/// The Hidden Power types in their order (`getHiddenPower`).
const HP_TYPES: [&str; 16] = [
    "Fighting", "Flying", "Poison", "Ground", "Rock", "Bug", "Ghost", "Steel", "Fire", "Water",
    "Grass", "Electric", "Psychic", "Ice", "Dragon", "Dark",
];

/// Species never legendary for the three perfect IVs, though they can neither breed nor evolve.
const NOT_LEGENDARY: [&str; 11] = [
    "Pikachu",
    "Unown",
    "Dracozolt",
    "Arctozolt",
    "Dracovish",
    "Arctovish",
    "Gouging Fire",
    "Raging Bolt",
    "Iron Boulder",
    "Iron Crown",
    "Terapagos",
];

/// Species legendary for the three perfect IVs, though they breed or evolve.
const LEGENDARY: [&str; 5] = ["Manaphy", "Cosmog", "Cosmoem", "Solgaleo", "Lunala"];

fn values(stats: &Stats) -> [i32; 6] {
    [
        stats.hp, stats.atk, stats.def, stats.spa, stats.spd, stats.spe,
    ]
}

/// `Math.floor(value / 2)`.
fn half(value: i32) -> i32 {
    value.div_euclid(2)
}

/// Showdown `trunc(num)`: `num >>> 0`, the number made an unsigned 32-bit integer.
fn trunc(value: f64) -> i64 {
    (value.trunc() as i64).rem_euclid(1 << 32)
}

impl TeamValidator<'_> {
    /// Showdown `allSources(species)`: from the format's minimum source generation (Gen 3 at
    /// the least from Gen 3 on) or the species' own, whichever is later, to this generation
    /// (one more with Allow Tradeback).
    pub fn all_sources(&self, species: Option<&SpeciesFacts>) -> Sources {
        let mut min = self.table.numbers.min_source_gen;
        if self.generation >= 3 && min < 3 {
            min = 3;
        }
        if let Some(species) = species {
            min = min.max(u32::from(species.generation));
        }
        let max = if self.table.has("allowtradeback") {
            (self.generation + 1).clamp(1, 8)
        } else {
            self.generation
        };
        Sources::new(max, min)
    }

    /// Showdown `dex.getHiddenPower(ivs).type` (`undefined` where its index runs off the
    /// table, as an IV out of range makes it).
    pub fn hidden_power_type(&self, ivs: &Stats) -> &'static str {
        let index = if self.generation <= 2 {
            let atk = trunc(f64::from(ivs.atk) / 2.0);
            let def = trunc(f64::from(ivs.def) / 2.0);
            4 * (atk % 4) + (def % 4)
        } else {
            // Showdown's order: HP, Atk, Def, Spe, SpA, SpD.
            let order = [ivs.hp, ivs.atk, ivs.def, ivs.spe, ivs.spa, ivs.spd];
            let x: i64 = order
                .iter()
                .enumerate()
                .map(|(i, iv)| (1_i64 << i) * i64::from(iv % 2))
                .sum();
            trunc(x as f64 * 15.0 / 63.0)
        };
        usize::try_from(index)
            .ok()
            .and_then(|index| HP_TYPES.get(index))
            .copied()
            .unwrap_or("undefined")
    }

    /// Showdown `validateStats(set, species, setSources, pokemonGoProblems)`, with no Pokémon
    /// GO origin to fall back on (the learnset seam's empty side). The set comes back with its
    /// EVs and IVs filled, its Hidden Power type, Gen 2's DV-fixed gender, Gen 1-2's nature.
    /// `go_problems`: `validatePokemonGo`'s answer — `None` when no GO origin explains the set.
    pub fn validate_stats(
        &self,
        set: &mut TeamSet,
        species: &SpeciesFacts,
        sources: &mut Sources,
        go_problems: Option<&[String]>,
    ) -> Vec<Problem> {
        let table = &self.table;
        let generation = self.generation;
        let obtainable_misc = table.has("obtainablemisc");
        let allow_avs = !table.has("lgpenormalrules");
        let ev_limit = table.numbers.ev_limit;
        let can_bottle_cap = generation >= 7
            && (set.level >= if generation < 9 { 100 } else { 50 } || !obtainable_misc);
        let mut problems = Vec::new();
        let mut problem = |field: SetField, message: String| {
            problems.push(Problem::new(ProblemKind::Stats, message).at(field));
        };
        if set.evs.is_none() {
            set.evs = Some(Stats::filled(if ev_limit.is_none() { 252 } else { 0 }));
        }
        if set.ivs.is_none() {
            set.ivs = Some(Stats::filled(31));
        }
        let name = if set.name.is_empty() {
            set.species.clone()
        } else {
            set.name.clone()
        };
        let maxed = values(set.ivs.as_ref().unwrap()).iter().all(|iv| *iv == 31);
        for move_name in &set.moves {
            let the_move = self.legality.move_named(self.aliases, move_name);
            let Some(the_move) = the_move.found() else {
                continue;
            };
            let move_type = the_move.move_type.as_deref().unwrap_or_default();
            if the_move.id == "hiddenpower" && move_type != "Normal" {
                if set.hp_type.is_empty() {
                    set.hp_type = move_type.to_string();
                } else if set.hp_type != move_type && obtainable_misc {
                    problem(
                        SetField::HpType,
                        format!(
                            "{name}'s Hidden Power type {} is incompatible with Hidden Power {move_type}",
                            set.hp_type
                        ),
                    );
                }
            }
        }
        if !set.hp_type.is_empty() && maxed && obtainable_misc {
            let hp_type = self.legality.type_named(&set.hp_type);
            let hp_type = hp_type.found();
            if generation <= 2 {
                let mut ivs = Stats::filled(30);
                if let Some(dvs) = hp_type.and_then(|t| t.hp_dvs.as_ref()) {
                    for (stat, dv) in dvs {
                        set_stat(&mut ivs, stat, i32::from(*dv) * 2);
                    }
                }
                ivs.hp = -1;
                set.ivs = Some(ivs);
            } else if !can_bottle_cap {
                let mut ivs = Stats::filled(31);
                if let Some(hp_ivs) = hp_type.and_then(|t| t.hp_ivs.as_ref()) {
                    for (stat, iv) in hp_ivs {
                        set_stat(&mut ivs, stat, i32::from(*iv));
                    }
                }
                set.ivs = Some(ivs);
            }
        }
        if set.hp_type.is_empty()
            && set.moves.iter().any(|m| {
                self.legality
                    .move_named(self.aliases, m)
                    .found()
                    .is_some_and(|m| m.id == "hiddenpower")
            })
        {
            set.hp_type = self.hidden_power_type(set.ivs.as_ref().unwrap()).into();
        }
        let cant_breed_nor_evolve = species.egg_groups.first().map(String::as_str)
            == Some("Undiscovered")
            && species.prevo.is_none()
            && !species.nfe;
        let is_legendary = (cant_breed_nor_evolve
            && !species.tags.iter().any(|tag| tag == "Paradox")
            && !NOT_LEGENDARY.contains(&species.base_species.as_str()))
            || LEGENDARY.contains(&species.base_species.as_str());
        let diancie_exception = species.name == "Diancie" && !set.shiny;
        let three_perfect = sources.min_source_gen() >= 6 && is_legendary && !diancie_exception;
        if set.hp_type == "Fighting" && obtainable_misc && three_perfect {
            problem(
                SetField::HpType,
                format!(
                    "{name} must not have Hidden Power Fighting because it starts with 3 perfect IVs \
                 because it's a Gen 6+ legendary."
                ),
            );
        }
        if three_perfect && obtainable_misc {
            let perfect = values(set.ivs.as_ref().unwrap())
                .iter()
                .filter(|iv| **iv >= 31)
                .count();
            if perfect < 3 {
                if go_problems.is_none_or(|go| !go.is_empty()) {
                    let reason = if table.numbers.min_source_gen == 6 {
                        format!(" and this format requires Gen {generation} Pokémon")
                    } else {
                        " in Gen 6 or later".into()
                    };
                    problem(
                        SetField::Ivs,
                        format!(
                            "{name} must have at least three perfect IVs because it's a legendary{reason}."
                        ),
                    );
                    if let Some(go) = go_problems.filter(|go| !go.is_empty()) {
                        problem(
                            SetField::Origin,
                            "Additionally, it failed to validate as a Pokemon from Pokemon GO because:"
                                .into(),
                        );
                        for message in go {
                            problem(SetField::Origin, message.clone());
                        }
                    }
                } else {
                    // A Pokémon GO origin explains it.
                    sources.is_from_pokemon_go = Some(true);
                }
            }
        }
        if !set.hp_type.is_empty() && !can_bottle_cap {
            let iv_type = self.hidden_power_type(set.ivs.as_ref().unwrap());
            if set.hp_type != iv_type {
                problem(
                    SetField::HpType,
                    format!(
                        "{name} has Hidden Power {}, but its IVs are for Hidden Power {iv_type}.",
                        set.hp_type
                    ),
                );
            }
        } else if !set.hp_type.is_empty()
            && !possible_bottle_cap_hp_type(&set.hp_type, set.ivs.as_ref().unwrap())
        {
            problem(
                SetField::HpType,
                format!(
                    "{name} has Hidden Power {}, but its IVs don't allow this even with (Bottle Cap) \
                 Hyper Training.",
                    set.hp_type
                ),
            );
        }
        if generation <= 2 {
            let ivs = set.ivs.as_mut().unwrap();
            let atk_dv = half(ivs.atk);
            let def_dv = half(ivs.def);
            let spe_dv = half(ivs.spe);
            let spc_dv = half(ivs.spa);
            let expected_hp_dv =
                (atk_dv % 2) * 8 + (def_dv % 2) * 4 + (spe_dv % 2) * 2 + (spc_dv % 2);
            if ivs.hp == -1 {
                ivs.hp = expected_hp_dv * 2;
            }
            let hp_dv = half(ivs.hp);
            if expected_hp_dv != hp_dv {
                problem(
                    SetField::Ivs,
                    format!(
                        "{name} has an HP DV of {hp_dv}, but its Atk, Def, Spe, and Spc DVs give it an \
                     HP DV of {expected_hp_dv}."
                    ),
                );
            }
            if ivs.spa != ivs.spd {
                if generation == 2 {
                    problem(
                        SetField::Ivs,
                        format!(
                            "{name} has different SpA and SpD DVs, which is not possible in Gen 2."
                        ),
                    );
                } else {
                    ivs.spd = ivs.spa;
                }
            }
            if generation > 1 && species.gender.is_none() {
                // Gen 2's gender comes from the Atk DV, high for male by the gender ratio.
                let threshold = species.gender_ratio.female * 16.0;
                let expected = if f64::from(atk_dv) >= threshold {
                    "M"
                } else {
                    "F"
                };
                if !set.gender.is_empty() && set.gender != expected {
                    problem(
                        SetField::Gender,
                        format!(
                            "{name} is {}, but it has an Atk DV of {atk_dv}, which makes its gender \
                         {expected}.",
                            set.gender
                        ),
                    );
                } else {
                    set.gender = expected.into();
                }
            }
            if set.species == "Marowak"
                && to_id(&set.item) == "thickclub"
                && set.moves.iter().any(|m| to_id(m) == "swordsdance")
                && set.level == 100
            {
                // The Marowak hack: its Attack never overflows 255.
                let ivs = set.ivs.as_mut().unwrap();
                ivs.atk = half(ivs.atk) * 2;
                let atk_iv = ivs.atk;
                let evs = set.evs.as_mut().unwrap();
                while evs.atk > 0 && 2 * 80 + atk_iv + evs.atk.div_euclid(4) + 5 > 255 {
                    evs.atk -= 4;
                }
            }
            if generation > 1 {
                let expected_shiny =
                    def_dv == 10 && spe_dv == 10 && spc_dv == 10 && atk_dv % 4 >= 2;
                if expected_shiny && !set.shiny {
                    problem(
                        SetField::Ivs,
                        format!("{name} is not shiny, which does not match its DVs."),
                    );
                } else if !expected_shiny && set.shiny {
                    problem(
                        SetField::Ivs,
                        format!(
                            "{name} is shiny, which does not match its DVs (its DVs must all be 10, \
                         except Atk which must be 2, 3, 6, 7, 10, 11, 14, or 15)."
                        ),
                    );
                }
            }
            set.nature = "Serious".into();
        }
        let evs = set.evs.as_mut().unwrap();
        // Showdown names a negative EV after Let's Go's Awakening Values wherever Let's Go's
        // normal rules are not in force: every target format.
        let value_name = if allow_avs { "Awakening Values" } else { "EVs" };
        for (stat, ev) in values(evs).iter().enumerate() {
            if *ev < 0 {
                problem(
                    SetField::Evs,
                    format!(
                        "{name} has less than 0 {value_name} in {}.",
                        STAT_NAMES[stat]
                    ),
                );
            }
        }
        for (stat, ev) in values(evs).iter().enumerate() {
            if *ev > 255 {
                problem(
                    SetField::Evs,
                    format!("{name} has more than 255 EVs in {}.", STAT_NAMES[stat]),
                );
            }
        }
        if generation <= 2 && evs.spa != evs.spd {
            if generation == 2 {
                problem(
                    SetField::Evs,
                    format!(
                        "{name} has different SpA and SpD EVs, which is not possible in Gen 2."
                    ),
                );
            } else {
                evs.spd = evs.spa;
            }
        }
        let total: i64 = values(evs).iter().map(|ev| i64::from(*ev)).sum();
        let limit = ev_limit.map(i64::from);
        if set.level > 1 && limit != Some(0) && total == 0 {
            problem(
                SetField::Evs,
                format!(
                    "{name} has exactly 0 EVs - did you forget to EV it? (If this was intentional, add \
                 exactly 1 to one of your EVs, which won't change its stats but will tell us that \
                 it wasn't a mistake)."
                ),
            );
        } else if !matches!(limit, Some(508 | 510)) && matches!(total, 508 | 510) {
            problem(
                SetField::Evs,
                format!(
                    "{name} has exactly {total} EVs, but this format does not restrict you to 510 EVs \
                 (If this was intentional, add exactly 1 to one of your EVs, which won't change its \
                 stats but will tell us that it wasn't a mistake)."
                ),
            );
        }
        // A VGC set brought into a format of level 100.
        let numbers = &table.numbers;
        if set.level == 50
            && numbers.max_level != 50
            && numbers.max_total_level.is_none()
            && limit != Some(0)
            && total % 4 == 0
        {
            problem(
                SetField::Level,
                format!(
                    "{name} is level 50, but this format allows level {} Pokémon. (If this was \
                 intentional, add exactly 1 to one of your EVs, which won't change its stats but \
                 will tell us that it wasn't a mistake).",
                    numbers.max_level
                ),
            );
        }
        if let Some(limit) = limit
            && total > limit
        {
            if limit == 0 {
                problem(
                    SetField::Evs,
                    format!("{name} has EVs, which is not allowed by this format."),
                );
            } else {
                problem(
                    SetField::Evs,
                    format!(
                        "{name} has {total} total EVs, which is more than this format's limit of \
                     {limit}."
                    ),
                );
            }
        }
        problems
    }
}

fn set_stat(stats: &mut Stats, stat: &str, value: i32) {
    match stat {
        "hp" => stats.hp = value,
        "atk" => stats.atk = value,
        "def" => stats.def = value,
        "spa" => stats.spa = value,
        "spd" => stats.spd = value,
        "spe" => stats.spe = value,
        _ => {}
    }
}

/// Showdown `possibleBottleCapHpType(type, ivs)`: the Atk and Spe parities a Hidden Power type
/// needs even with Hyper Training.
fn possible_bottle_cap_hp_type(hp_type: &str, ivs: &Stats) -> bool {
    if ["Dark", "Dragon", "Grass", "Ghost", "Poison"].contains(&hp_type) && ivs.spe % 2 == 0 {
        return false;
    }
    if ["Psychic", "Fire", "Rock", "Fighting"].contains(&hp_type)
        && ivs.spe != 31
        && ivs.spe % 2 == 1
    {
        return false;
    }
    if hp_type == "Dark" && ivs.atk % 2 == 0 {
        return false;
    }
    if ["Ice", "Water"].contains(&hp_type) && ivs.spe % 2 == 0 && ivs.atk % 2 == 0 {
        return false;
    }
    true
}
