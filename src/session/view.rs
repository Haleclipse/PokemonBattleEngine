//! What a side sees of the other's registered team before the battle (DESIGN.md §45.5): Team
//! Preview's entries and the open team sheets, each projected from the registered sets as
//! Showdown shows them — never the sets themselves.

use crate::dex::{PokemonType, build_pokemon};
use crate::format::{TeamSet, to_id};
use crate::ids::{PokemonId, SideId};
use crate::state::{Gender, MemberSheet, PokemonState, PreviewedMember, TeamPreview};

use super::bootstrap::SetBuildError;
use super::format::{SessionError, SessionFormat};

/// A Pokémon as Team Preview shows it (Showdown's `|poke|` line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewEntry {
    /// Showdown's details, as the preview writes them: the species (a few formes hidden,
    /// `Urshifu-*`), the level unless 100, the gender; never the shininess.
    pub details: String,
    /// Whether it holds something, where the generation's preview tells (Gen 4-7: `item`; Gen
    /// 5: `mail` for a Mail); empty otherwise.
    pub item: &'static str,
    /// Tera Type Preview: the Tera type.
    pub tera_type: Option<String>,
}

/// The species whose forme Team Preview hides (`$1-*`): each generation's mod has its own
/// `teampreview` rule. Gen 4-7 (`gen4`, `gen5`, `gen7`'s rule, Gen 6 inheriting Gen 7's) hide
/// Arceus' and Genesect's too and Greninja's short of Greninja-Ash; Gen 8 Arceus' and
/// Greninja's; Gen 9 Greninja's, unless Species Reveal Clause.
const HIDDEN_GEN4_7: [&str; 8] = [
    "Arceus",
    "Genesect",
    "Gourgeist",
    "Pumpkaboo",
    "Xerneas",
    "Silvally",
    "Urshifu",
    "Dudunsparce",
];
const HIDDEN_GEN8: [&str; 8] = [
    "Arceus",
    "Greninja",
    "Gourgeist",
    "Pumpkaboo",
    "Xerneas",
    "Silvally",
    "Urshifu",
    "Dudunsparce",
];
const HIDDEN_GEN9: [&str; 7] = [
    "Greninja",
    "Gourgeist",
    "Pumpkaboo",
    "Xerneas",
    "Silvally",
    "Urshifu",
    "Dudunsparce",
];

/// `name` as `$1-*` when it is `base` or `base-<forme>` of one of `bases`.
fn hide_forme(name: &str, bases: &[&str]) -> Option<String> {
    bases
        .iter()
        .find(|base| {
            name.strip_prefix(**base)
                .is_some_and(|forme| forme.is_empty() || forme.starts_with('-'))
        })
        .map(|base| format!("{base}-*"))
}

impl SessionFormat<'_> {
    /// Showdown `pokemon.details` of a registered set, before the battle: the species' name
    /// (`dex::HIDDEN_FORMES` as their base species), `, L<level>` unless 100, the
    /// gender (`M` / `F`; a fixed one the set leaves out included).
    pub(super) fn details(&self, set: &TeamSet) -> String {
        let species = self.validator.species(&set.species);
        let name = if crate::dex::HIDDEN_FORMES.contains(&species.name.as_str()) {
            species.base_species.clone()
        } else {
            species.name.clone()
        };
        let mut details = name;
        if set.level != 100 {
            details.push_str(&format!(", L{}", set.level));
        }
        if let Some(gender) = self.gender(set) {
            details.push_str(&format!(", {gender}"));
        }
        details
    }

    /// The Pokémon's gender (Showdown `pokemon.gender`): the set's `M` / `F`, else the species'
    /// fixed one; none for a genderless Pokémon (`N`).
    fn gender(&self, set: &TeamSet) -> Option<&'static str> {
        match set.gender.as_str() {
            "M" => Some("M"),
            "F" => Some("F"),
            _ => self
                .dex
                .species_by_name(&self.battle_species(set))
                .and_then(|species| species.gender)
                .and_then(|gender| match gender {
                    Gender::Male => Some("M"),
                    Gender::Female => Some("F"),
                    Gender::Genderless => None,
                }),
        }
    }

    internal!(
        /// Team Preview's entry for a registered set (the `Team Preview` rule's `onTeamPreview`):
        /// the details without the shininess, Zacian and Zamazenta short of their Crowned forme as
        /// `Zacian-*`, and unless Species Reveal Clause the species in `HIDDEN_FORMES` with any forme
        /// as `Urshifu-*`.
        pub fn preview_entry(&self, set: &TeamSet) -> PreviewEntry {
            let details = self.details(set);
            let (species, rest) = match details.find(',') {
                Some(at) => details.split_at(at),
                None => (details.as_str(), ""),
            };
            let mut shown = species.to_string();
            let generation = self.validator.generation;
            // (Zacian and Zamazenta short of their Crowned forme: `$1-*`, before any forme.)
            let heroes = |shown: &mut String| {
                for hero in ["Zacian", "Zamazenta"] {
                    if shown.starts_with(hero) && !shown[hero.len()..].starts_with("-Crowned") {
                        *shown = format!("{hero}-*{}", &shown[hero.len()..]);
                    }
                }
            };
            match generation {
                ..=7 => {
                    if let Some(hidden) = hide_forme(&shown, &HIDDEN_GEN4_7) {
                        shown = hidden;
                    }
                    heroes(&mut shown);
                    if shown.starts_with("Greninja")
                        && !shown["Greninja".len()..].starts_with("-Ash")
                    {
                        shown = format!("Greninja-*{}", &shown["Greninja".len()..]);
                    }
                }
                8 => {
                    if let Some(hidden) = hide_forme(&shown, &HIDDEN_GEN8) {
                        shown = hidden;
                    }
                    heroes(&mut shown);
                }
                _ => {
                    heroes(&mut shown);
                    if !self.visibility.species_reveal
                        && let Some(hidden) = hide_forme(&shown, &HIDDEN_GEN9)
                    {
                        shown = hidden;
                    }
                }
            }
            let item_id = to_id(&set.item);
            let item = match generation {
                5 if item_id.contains("mail") => "mail",
                4..=7 if !item_id.is_empty() => "item",
                _ => "",
            };
            PreviewEntry {
                details: format!("{shown}{rest}"),
                item,
                tera_type: self.visibility.tera_types.then(|| {
                    if set.tera_type.is_empty() {
                        // `set.teraType || types[0]`.
                        self.validator
                            .species(&set.species)
                            .types
                            .first()
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        set.tera_type.clone()
                    }
                }),
            }
        }
    );

    internal!(
        /// A registered set's open team sheet (Showdown `showOpenTeamSheets`): the species, item,
        /// ability, moves, gender and level — no nickname, nature, EVs or IVs; the Gigantamax flag
        /// in Gen 8 without Dynamax Clause, the Tera type in Gen 9 without Terastal Clause, the
        /// Hidden Power type with Hidden Power; Zacian and Zamazenta with their crowning item as
        /// their Crowned forme, Iron Head as their signature move.
        pub fn open_sheet(&self, set: &TeamSet) -> TeamSet {
            let generation = self.validator.generation;
            let table = &self.validator.table;
            let mut sheet = TeamSet {
                name: String::new(),
                species: set.species.clone(),
                item: set.item.clone(),
                ability: set.ability.clone(),
                moves: set.moves.clone(),
                nature: String::new(),
                gender: self.gender(set).unwrap_or_default().to_string(),
                evs: None,
                ivs: None,
                level: set.level,
                shiny: false,
                happiness: None,
                hp_type: String::new(),
                gigantamax: generation == 8 && !table.has("dynamaxclause") && set.gigantamax,
                tera_type: if generation == 9 && !table.has("terastalclause") {
                    set.tera_type.clone()
                } else {
                    String::new()
                },
            };
            let knows_hidden_power = set.moves.iter().any(|name| {
                matches!(
                    self.validator.legality.move_named(self.validator.aliases, name),
                    crate::format::Named::Found(found) if found.id == "hiddenpower"
                )
            });
            if knows_hidden_power {
                sheet.hp_type = set.hp_type.clone();
            }
            for (hero, sword, signature) in [
                ("zacian", "rustedsword", "Behemoth Blade"),
                ("zamazenta", "rustedshield", "Behemoth Bash"),
            ] {
                if to_id(&set.species) == hero && to_id(&set.item) == sword {
                    sheet.species = self
                        .validator
                        .species(&format!("{hero}crowned"))
                        .name
                        .clone();
                    if let Some(slot) = sheet.moves.iter_mut().find(|m| to_id(m) == "ironhead") {
                        *slot = signature.to_string();
                    }
                }
            }
            sheet
        }
    );

    internal!(
        /// What the team preview shows of both registered teams, for the battle's views
        /// (DESIGN.md §47.0 item 14): each member as everyone sees it — its Team Preview entry
        /// (`preview_entry`), and its open team sheet (`open_sheet`) when `sheets_shown` — and as
        /// its side knows it. `picked` is each side's registered indices in the order brought:
        /// the battle numbers those Pokémon from 1, P1's first (`state`). `None` without a Team
        /// Preview.
        pub fn team_preview(
            &self,
            registered: [&[TeamSet]; 2],
            picked: [&[usize]; 2],
            sheets_shown: bool,
        ) -> Result<Option<TeamPreview>, SessionError> {
            if !self.policy.preview {
                return Ok(None);
            }
            let generation = self.validator.generation;
            let mut sides: [Vec<PreviewedMember>; 2] = [Vec::new(), Vec::new()];
            let mut first = 1usize;
            for (side_index, side) in [SideId::P1, SideId::P2].into_iter().enumerate() {
                for (index, set) in registered[side_index].iter().enumerate() {
                    let built = self.built(side, index, set)?;
                    let known = self.known_sheet(&built);
                    let entry = self.preview_entry(set);
                    let name = entry.details.split(',').next().unwrap_or_default();
                    // `Urshifu-*`: the base species, its forme hidden.
                    let (named, forme_hidden) = match name.strip_suffix("-*") {
                        Some(base) => (base, true),
                        None => (name, false),
                    };
                    let mut shown = MemberSheet {
                        species: self
                            .dex
                            .species_by_name(named)
                            .map_or(known.species, |species| species.id),
                        forme_hidden,
                        level: known.level,
                        gender: known.gender,
                        // Gen 4-7 mark the members that hold one (`item`, Gen 5's Mail `mail`).
                        item: (matches!(generation, 4..=7) && entry.item.is_empty())
                            .then_some(None),
                        ability: None,
                        moves: None,
                        tera_type: if generation < 9 {
                            Some(None)
                        } else {
                            entry
                                .tera_type
                                .as_deref()
                                .and_then(PokemonType::from_name)
                                .map(Some)
                        },
                    };
                    if sheets_shown {
                        let sheet = self.open_sheet(set);
                        let shown_set = self.known_sheet(&self.built(side, index, &sheet)?);
                        shown = MemberSheet {
                            forme_hidden: false,
                            // The sheet names a Tera type in Gen 9 short of Terastal Clause.
                            tera_type: if sheet.tera_type.is_empty() {
                                shown.tera_type
                            } else {
                                shown_set.tera_type
                            },
                            ..shown_set
                        };
                    }
                    let pokemon = picked[side_index]
                        .iter()
                        .position(|picked| *picked == index)
                        .map(|at| PokemonId((first + at) as u8));
                    sides[side_index].push(PreviewedMember {
                        shown,
                        known,
                        pokemon,
                    });
                }
                first += picked[side_index].len();
            }
            Ok(Some(TeamPreview { sides }))
        }
    );

    /// A registered set built as the battle builds it (`state`).
    fn built(
        &self,
        side: SideId,
        index: usize,
        set: &TeamSet,
    ) -> Result<PokemonState, SessionError> {
        let fail = |error| SessionError::Set { side, index, error };
        let engine_set = self.engine_set(set).map_err(fail)?;
        build_pokemon(&self.dex, PokemonId(1), side, &engine_set)
            .map_err(|error| fail(SetBuildError::Dex(error)))
    }

    /// Everything of a built member.
    fn known_sheet(&self, built: &PokemonState) -> MemberSheet {
        MemberSheet {
            species: built.species.expect("a built Pokémon has its species"),
            forme_hidden: false,
            level: built.level,
            gender: built.gender,
            item: Some(built.held_item),
            ability: Some(built.ability),
            moves: Some(built.move_slots().iter().map(|slot| slot.move_id).collect()),
            tera_type: Some((self.validator.generation >= 9).then_some(built.tera_type)),
        }
    }
}
