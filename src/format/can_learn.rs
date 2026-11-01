//! Move legality (M9.3, DESIGN.md §45.3): Showdown's `checkCanLearn` — the union of every way a
//! species (and its prevolutions) learns a move, intersected with the set's sources — with
//! `validateMoves`, the events (`validateSource`, `validateEvent`, `getEventOnlyData`) and the
//! learnset's family (`getFullLearnset`, `learnsetParent`, `eggMovesOnly`).
//!
//! First phase (§45.3): an egg move combination always has a father (`findEggMoveFathers`
//! answers yes), and no Pokémon comes from Pokémon GO; both seams are the same on Showdown's
//! side of the differential.

use std::borrow::Cow;

use super::data::to_id;
use super::learnset::{EventInfo, EventShiny, Learnset, MoveSources};
use super::legality::{Named, SpeciesFacts, ThingFacts};
use super::set::TeamSet;
use super::sources::{EggLimit, Sources, Tri};
use super::validate::{Problem, ProblemKind, TeamValidator};

/// The HM moves a transfer out of Gen 3 loses, and out of Gen 4.
const GEN3_HMS: [&str; 8] = [
    "cut",
    "fly",
    "surf",
    "strength",
    "flash",
    "rocksmash",
    "waterfall",
    "dive",
];
const GEN4_HMS: [&str; 7] = [
    "cut",
    "fly",
    "surf",
    "strength",
    "rocksmash",
    "waterfall",
    "rockclimb",
];

/// `parseInt(text)` of a source's level or event index: the leading digits.
fn leading_number(text: &str) -> u32 {
    text.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn char_at(text: &str, index: usize) -> char {
    text.chars().nth(index).unwrap_or('\0')
}

/// `Dex.stats.names`, by Showdown's stat id.
fn stat_name(stat: &str) -> &'static str {
    match stat {
        "hp" => "HP",
        "atk" => "Attack",
        "def" => "Defense",
        "spa" => "Special Attack",
        "spd" => "Special Defense",
        _ => "Speed",
    }
}

impl<'d> TeamValidator<'d> {
    fn learnset_data(&self, id: &str) -> &'d Learnset {
        self.learnset_data_or_none(id)
    }

    /// Showdown `getLearnsetData(id)`: no data at all on the learnset seam's empty side.
    pub(super) fn learnset_data_or_none(&self, id: &str) -> &'d Learnset {
        match &self.learnsets {
            Some(learnsets) => learnsets.data(id),
            None => super::learnset::Learnset::none(),
        }
    }

    /// Showdown `learnsetParent(species, checkingMoves)`.
    pub fn learnset_parent(
        &self,
        species: &SpeciesFacts,
        checking_moves: bool,
    ) -> Option<Cow<'d, SpeciesFacts>> {
        if ["Gastrodon", "Pumpkaboo", "Sinistea", "Tatsugiri"]
            .contains(&species.base_species.as_str())
            && species.forme.is_some()
        {
            return Some(self.species(&species.base_species));
        }
        if let Some(prevo) = &species.prevo {
            let prevo = self.species(prevo);
            if u32::from(prevo.generation) > self.generation.max(2) {
                return None;
            }
            return Some(prevo);
        }
        if let Some(changes_from) = &species.changes_from
            && species.base_species != "Kyurem"
        {
            return Some(self.species(changes_from));
        }
        if checking_moves && !species.base_species.is_empty() {
            let mut base = self.species(&species.base_species);
            if base.prevo.is_some() {
                while let Some(prevo) = base.prevo.clone() {
                    base = self.species(&prevo);
                }
                return Some(base);
            }
        }
        None
    }

    /// Showdown `getFullLearnset(id)`: the species' learnset and its family's, nearest first.
    pub fn full_learnset(&self, id: &str) -> Vec<(Cow<'d, SpeciesFacts>, &'d MoveSources)> {
        let mut out = Vec::new();
        let mut species = Some(self.species(id));
        let mut checked: Vec<String> = Vec::new();
        while let Some(current) = species.take() {
            if current.name.is_empty() || checked.contains(&current.id) {
                break;
            }
            checked.push(current.id.clone());
            let data = self.learnset_data(&current.id);
            if let Some(learnset) = &data.learnset {
                species = self.learnset_parent(&current, true);
                out.push((current, learnset));
                continue;
            }
            let own = current
                .changes_from
                .clone()
                .unwrap_or_else(|| current.base_species.clone());
            if own != current.name {
                // A forme without its own learnset.
                species = Some(self.species(&own));
                continue;
            }
            if current.nonstandard.is_some() {
                return out;
            }
            if let Some(prevo) = &current.prevo
                && self.learnset_data(&to_id(prevo)).learnset.is_some()
            {
                species = Some(self.species(&to_id(prevo)));
                continue;
            }
            // Showdown throws here: a species with no learnset data at all.
            break;
        }
        out
    }

    /// Showdown `eggMovesOnly(child, father)`: a forme that inherits only egg moves.
    pub fn egg_moves_only(&self, child: &SpeciesFacts, father: &SpeciesFacts) -> bool {
        if child.base_species == father.base_species {
            return false;
        }
        let mut father = Some(Cow::Borrowed(father));
        while let Some(current) = father {
            if current.name == child.name {
                return false;
            }
            father = self.learnset_parent(&current, false);
        }
        true
    }

    /// Showdown `motherCanLearn(species, move)`.
    fn mother_can_learn(&self, species: &str, move_id: &str) -> bool {
        !species.is_empty()
            && self
                .full_learnset(species)
                .iter()
                .any(|(_, learnset)| learnset.contains_key(move_id))
    }

    /// Showdown `checkCanLearn(move, species, setSources, set)`: `None` if the set can have the
    /// move (its sources narrowed), else the problem's rest (`'s move Tackle can't be …`).
    pub fn check_can_learn(
        &self,
        the_move: &ThingFacts,
        original: &SpeciesFacts,
        set_sources: &mut Sources,
        set: &TeamSet,
    ) -> Option<String> {
        let table = &self.table;
        let generation = self.generation;
        let move_id = the_move.id.as_str();
        let base_species = original;
        let level = if set.level == 0 { 100 } else { set.level };
        let mut can_learn_species: Vec<String> = Vec::new();
        let mut cant_learn_reason: Option<String> = None;
        let mut limit1 = true;
        let mut sketch = false;
        let mut blocked_hm = false;
        let mut baby_only = String::new();
        let mut min_learn_gen = generation;
        let mut move_sources = Sources::default();
        let no_future_gen = !table.has("allowtradeback");
        let can_sketch_post_gen7 = table.has("sketchpostgen7moves") || self.mod_name == "gen8bdsp";
        let mut tradeback_eligible = false;
        let full = self.full_learnset(&original.id);
        if full.is_empty() {
            return Some(" can't learn any moves at all.".into());
        }
        let only_legal_abilities = table.has("obtainableabilities");
        let can_use_ability_patch = generation >= 8 && self.mod_name != "gen8dlc1";
        let ability = self.ability(&set.ability);
        for (species, learnset) in &full {
            if generation <= 2 && species.generation == 1 {
                tradeback_eligible = true;
            }
            let checking_prevo = species.base_species != original.base_species;
            if checking_prevo
                && move_sources.size() == 0
                && (set_sources.baby_only.is_none() || species.prevo.is_none())
            {
                baby_only = species.id.clone();
            }
            let forme_cant_inherit = self.egg_moves_only(species, base_species);
            if forme_cant_inherit && generation < 9 {
                break;
            }
            let mut sources: Vec<String> = learnset.get(move_id).cloned().unwrap_or_default();
            if move_id == "sketch" {
                sketch = true;
            } else if let Some(sketch_sources) = learnset.get("sketch") {
                if the_move.nosketch || the_move.z || the_move.max.is_some() {
                    cant_learn_reason = Some("can't be Sketched.".into());
                } else if the_move.generation > 7
                    && !can_sketch_post_gen7
                    && (generation == 8
                        || (generation == 9 && ["gen9dlc1", "gen9predlc"].contains(&self.mod_name)))
                {
                    cant_learn_reason = Some(format!(
                        "can't be Sketched because it's a Gen {0} move and Sketch isn't \
                         available in Gen {0}.",
                        the_move.generation
                    ));
                } else {
                    if sources.is_empty() || move_sources.size() == 0 {
                        sketch = true;
                    }
                    let mut all = sketch_sources.clone();
                    all.extend(sources);
                    sources = all;
                }
            }
            let mut can_use_home_relearner = false;
            for learned_source in sources {
                let mut learned = learned_source;
                let learned_gen = char_at(&learned, 0).to_digit(10).unwrap_or(0);
                let kind = char_at(&learned, 1);
                if forme_cant_inherit && (kind != 'E' || learned_gen < 9) {
                    continue;
                }
                if let Some(domain) = &set_sources.learnset_domain
                    && !domain.contains(&format!("{learned_gen}{}", to_id(&species.base_species)))
                    && (kind != 'E' || learned_gen < 8)
                {
                    if cant_learn_reason.is_none() {
                        let restrictive = set_sources.restrictive_moves.clone().unwrap_or_default();
                        cant_learn_reason =
                            Some(format!("is incompatible with {}.", restrictive.join(", ")));
                    }
                    continue;
                }
                let min_source_gen = table.numbers.min_source_gen;
                if learned_gen < min_source_gen && !can_use_home_relearner {
                    if cant_learn_reason.is_none() {
                        cant_learn_reason = Some(format!(
                            "can't be transferred from Gen {learned_gen} to {min_source_gen}."
                        ));
                    }
                    continue;
                }
                if no_future_gen && learned_gen > generation {
                    if cant_learn_reason.is_none() {
                        cant_learn_reason = Some(format!(
                            "can't be transferred from Gen {learned_gen} to {generation}."
                        ));
                    }
                    continue;
                }
                if learned_gen == 9 && kind != 'S' {
                    can_use_home_relearner = true;
                }
                if base_species.evo_region.as_deref() == Some("Alola")
                    && checking_prevo
                    && learned_gen >= 8
                    && (generation < 9 || kind != 'E')
                {
                    cant_learn_reason = Some(format!(
                        "is from a {} that can't be transferred to USUM to evolve into {}.",
                        species.name, base_species.name
                    ));
                    continue;
                }
                if learned_gen < 7
                    && set_sources.is_hidden == Some(true)
                    && !can_use_ability_patch
                    && only_legal_abilities
                    && !self.hidden_ability_in(learned_gen, &base_species.name)
                {
                    cant_learn_reason =
                        Some("can only be learned in gens without Hidden Abilities.".into());
                    continue;
                }
                if generation < 6
                    && u32::from(ability.generation) > learned_gen
                    && !checking_prevo
                    && only_legal_abilities
                {
                    cant_learn_reason = Some(format!(
                        "is learned in gen {learned_gen}, but the Ability {} did not exist then.",
                        ability.name
                    ));
                    continue;
                }
                if species.nonstandard.as_deref() != Some("CAP") {
                    if generation >= 4 && learned_gen <= 3 && GEN3_HMS.contains(&move_id) {
                        cant_learn_reason = Some(
                            "can't be transferred from Gen 3 to 4 because it's an HM move.".into(),
                        );
                        continue;
                    }
                    if generation >= 5 && learned_gen <= 4 && GEN4_HMS.contains(&move_id) {
                        cant_learn_reason = Some(
                            "can't be transferred from Gen 4 to 5 because it's an HM move.".into(),
                        );
                        continue;
                    }
                    if generation >= 5
                        && ["defog", "whirlpool"].contains(&move_id)
                        && learned_gen <= 4
                    {
                        blocked_hm = true;
                    }
                }
                if kind == 'L' {
                    let learned_level = leading_number(&learned[2..]);
                    if level >= learned_level || learned_gen == 7 {
                        // Past the level (Gen 7 relearns any level-up move).
                    } else if table.has("pomegglitchclause")
                        && level >= 5
                        && learned_gen == 3
                        && species.can_hatch
                    {
                        learned = format!("{learned_gen}Epomeg");
                    } else if species.gender.as_deref() != Some("N")
                        && learned_gen >= 2
                        && species.can_hatch
                    {
                        // An egg move.
                        if species.gender.as_deref() == Some("M")
                            && !self.mother_can_learn(
                                &to_id(species.mother.as_deref().unwrap_or_default()),
                                move_id,
                            )
                        {
                            cant_learn_reason =
                                Some(format!("is learned at level {learned_level}."));
                            continue;
                        }
                        learned = format!("{learned_gen}Eany");
                    } else {
                        cant_learn_reason = Some(format!("is learned at level {learned_level}."));
                        continue;
                    }
                }
                let kind = char_at(&learned, 1);
                let tail = &learned[1..];
                if (learned_gen >= 8 && kind == 'E' && tail != "Eany" && tail != "Epomeg")
                    || "LMTR".contains(kind)
                {
                    if learned_gen == generation && kind != 'R' {
                        // A level-up, TM or tutor move of this generation: always available.
                        if !(learned_gen >= 8 && kind == 'E')
                            && !baby_only.is_empty()
                            && species.evo_level.is_some()
                        {
                            set_sources.add_go_incompatible_move(&the_move.name);
                        }
                        if move_sources.move_evo_carry_count == 0 && baby_only.is_empty() {
                            return None;
                        }
                    }
                    if kind == 'R' {
                        move_sources.restricted_move = Some(move_id.to_string());
                    }
                    limit1 = false;
                    move_sources.add_gen(learned_gen);
                } else if kind == 'E' {
                    let mut limit = EggLimit::None;
                    if tail == "Eany" {
                        if species.gender.as_deref() == Some("F") {
                            limit = EggLimit::Limited(the_move.id.clone());
                            move_sources.level_up_egg_moves = Some(vec![the_move.id.clone()]);
                        } else {
                            limit = EggLimit::Unlimited;
                        }
                    } else if tail == "Epomeg" {
                        move_sources.pomeg_egg_moves = Some(vec![the_move.id.clone()]);
                    } else if learned_gen < 6
                        || species
                            .mother
                            .as_deref()
                            .is_some_and(|mother| !self.mother_can_learn(&to_id(mother), move_id))
                    {
                        limit = EggLimit::Limited(the_move.id.clone());
                    }
                    learned = format!(
                        "{learned_gen}E{}",
                        if species.prevo.is_some() {
                            species.id.as_str()
                        } else {
                            ""
                        }
                    );
                    if tradeback_eligible && learned_gen == 2 && the_move.generation <= 1 {
                        move_sources.add(&format!("1ET{}", &learned[2..]), limit.clone());
                    }
                    move_sources.add(&learned, limit);
                } else if kind == 'S' {
                    if tradeback_eligible && learned_gen == 2 && the_move.generation <= 1 {
                        move_sources.add(
                            &format!("1ST{} {}", &learned[2..], species.id),
                            EggLimit::None,
                        );
                    }
                    move_sources.add(&format!("{learned} {}", species.id), EggLimit::None);
                    // `eventData[parseInt(learned.charAt(2))]`: the index's first digit.
                    let index = char_at(&learned, 2).to_digit(10).unwrap_or(0) as usize;
                    let emerald = self
                        .learnset_data(&species.id)
                        .event_data
                        .as_ref()
                        .and_then(|events| events.get(index))
                        .is_some_and(|event| event.emerald_event_egg);
                    if emerald && learned_gen == 3 {
                        move_sources.pomeg_event_egg =
                            Tri::Some(format!("{learned} {}", species.id));
                    }
                } else if kind == 'D' {
                    move_sources.add(&format!("{learned}{}", species.id), EggLimit::None);
                    if move_sources.sources_before == 0 {
                        move_sources.dream_world_move_count += 1;
                    }
                } else if kind == 'V' && table.numbers.min_source_gen < learned_gen {
                    if learned == "8V" && !baby_only.is_empty() && species.evo_level.is_some() {
                        set_sources.add_go_incompatible_move(&the_move.name);
                    }
                    move_sources.add(&learned, EggLimit::None);
                }
                let base_id = to_id(&base_species.base_species);
                if kind == 'E' && learned_gen >= 8 && !can_learn_species.contains(&base_id) {
                    can_learn_species.push(base_id);
                }
                let own_id = to_id(&species.base_species);
                if !can_learn_species.contains(&own_id) {
                    can_learn_species.push(own_id);
                }
                min_learn_gen = min_learn_gen.min(learned_gen);
            }
            if can_use_home_relearner {
                let learnsets = self.learnsets.as_ref().unwrap();
                let mut full_sources: Vec<String> = Vec::new();
                if !["nincada", "spinda"].contains(&species.id.as_str())
                    && let Some(bdsp) = learnsets.external(&species.id, false, &the_move.id)
                {
                    full_sources.extend(bdsp.iter().cloned());
                }
                if let Some(legends) = learnsets.external(&species.id, true, &the_move.id) {
                    full_sources.extend(legends.iter().cloned());
                }
                for source in full_sources {
                    // Brilliant Diamond / Shining Pearl's and Legends' non-event sources reach
                    // Gen 9 through HOME's relearner, the level permitting.
                    if char_at(&source, 1) == 'S' {
                        continue;
                    }
                    if char_at(&source, 1) == 'L' && level < leading_number(&source[2..]) {
                        continue;
                    }
                    return None;
                }
            }
            if table.has("mimicglitchclause") && species.generation < 5 {
                let glitch = ["metronome", "copycat", "transform", "mimic", "assist"]
                    .iter()
                    .any(|glitch| {
                        learnset.contains_key(*glitch)
                            && !(*glitch == "mimic"
                                && ability.generation == 4
                                && species.prevo.is_none())
                    });
                if glitch {
                    move_sources.add_gen(4);
                    if the_move.generation < 5 {
                        limit1 = false;
                    }
                }
            }
            if move_sources.size() == 0
                && ((species.evo_type.as_deref() == Some("levelMove")
                    && species.evo_move.as_deref() != Some(the_move.name.as_str()))
                    || (species.id == "sylveon" && the_move.move_type.as_deref() != Some("Fairy")))
            {
                move_sources.move_evo_carry_count = 1;
            }
        }
        if limit1 && sketch {
            if let Some(sketched) = &set_sources.sketch_move {
                return Some(format!(
                    " can't Sketch {} and {sketched} because it can only Sketch 1 move.",
                    the_move.name
                ));
            }
            set_sources.sketch_move = Some(the_move.name.clone());
        }
        if blocked_hm {
            if set_sources.hm.is_some() {
                return Some(
                    " can't simultaneously transfer Defog and Whirlpool from Gen 4 to 5.".into(),
                );
            }
            set_sources.hm = Some(move_id.to_string());
        }
        let restrictive = set_sources.restrictive_moves.get_or_insert_with(Vec::new);
        if !restrictive.contains(&the_move.name) {
            restrictive.push(the_move.name.clone());
        }
        // A Pokémon that cannot be sent from GO to Let's Go (a purified one, Mew) reaches Let's
        // Go's moves through HOME alone: a chain of four level-up moves, no TM.
        let checked = if baby_only.is_empty() {
            Some(Cow::Borrowed(base_species))
        } else {
            full.last().map(|(species, _)| species.clone())
        };
        if let Some(checked) = checked
            && (set_sources.pokemon_go_source.as_deref() == Some("purified") || checked.id == "mew")
            && let Some(restrictive) = self
                .pokemon_go
                .and_then(|go| go.get(&checked.id))
                .and_then(|go| go.lgpe_restrictive_moves.as_ref())
        {
            let mut level_up_count = 0;
            let known: Vec<String> = set_sources
                .restrictive_moves
                .iter()
                .flatten()
                .map(|m| to_id(m))
                .collect();
            for (restrictive_move, move_level) in &restrictive.0 {
                if *restrictive_move == the_move.id
                    && (*move_level == 0 || (set.level != 0 && (set.level as i32) < *move_level))
                {
                    set_sources.add_go_incompatible_move(&the_move.name);
                }
                if level_up_count != 0 {
                    level_up_count += 1;
                }
                if known.contains(restrictive_move) {
                    if level_up_count == 0 {
                        level_up_count += 1;
                    } else if level_up_count > 4 {
                        set_sources.add_go_incompatible_move("");
                    }
                }
            }
        }
        if !tradeback_eligible {
            let mut next = Cow::Borrowed(base_species);
            if self.learnset_data(&next.id).learnset.is_none() {
                let own = next
                    .changes_from
                    .clone()
                    .unwrap_or_else(|| next.base_species.clone());
                next = self.species(&own);
            }
            let mut species_count = 0;
            while !next.name.is_empty() {
                let base_id = to_id(&next.base_species);
                for g in u32::from(next.generation)..=generation {
                    if can_learn_species.contains(&base_id)
                        || (0 < species_count && species_count < can_learn_species.len())
                        || (species_count == 0 && g >= min_learn_gen)
                        || (species_count == can_learn_species.len()
                            && g <= move_sources.sources_before)
                    {
                        move_sources
                            .learnset_domain
                            .get_or_insert_with(Vec::new)
                            .push(format!("{g}{base_id}"));
                    }
                }
                // (Showdown compares the species' own id with the base species' ids here.)
                if can_learn_species.contains(&next.id) {
                    species_count += 1;
                }
                next = self.species(next.prevo.as_deref().unwrap_or_default());
            }
        }
        if move_sources.size() == 0 {
            return Some(match cant_learn_reason {
                Some(reason) => format!("'s move {} {reason}", the_move.name),
                None => format!(" can't learn {}.", the_move.name),
            });
        }
        let has_egg = move_sources.sources.iter().any(|s| char_at(s, 1) == 'E');
        if generation >= 3
            && has_egg
            && move_sources.limited_egg_moves.is_null()
            && move_sources.sources_before != 0
        {
            move_sources.possibly_limited_egg_moves = Some(vec![to_id(&format!(
                "{}{}",
                move_sources.sources_before, the_move.id
            ))]);
        }
        let backup = (set_sources.sources.clone(), set_sources.sources_before);
        set_sources.intersect_with(&mut move_sources);
        if set_sources.size() == 0 {
            set_sources.sources = backup.0;
            set_sources.sources_before = backup.1;
            let restrictive = set_sources.restrictive_moves.clone().unwrap_or_default();
            return Some(format!(
                "'s moves {} are incompatible.",
                restrictive.join(", ")
            ));
        }
        if !baby_only.is_empty() {
            set_sources.baby_only = Some(baby_only);
        }
        None
    }

    /// Whether the species had its Hidden Ability in that generation
    /// (`dex.forGen(gen).species.get(name).abilities['H']`).
    fn hidden_ability_in(&self, generation: u32, species: &str) -> bool {
        self.legality_of(generation)
            .and_then(|legality| {
                legality
                    .species_named(self.aliases, species)
                    .found()
                    .cloned()
            })
            .is_some_and(|species| species.abilities.contains_key("H"))
    }

    /// Showdown `validateMoves(species, moves, setSources, set, name)`.
    pub fn validate_moves(
        &self,
        species: &SpeciesFacts,
        moves: &[String],
        set_sources: &mut Sources,
        set: &TeamSet,
        name: &str,
    ) -> Vec<Problem> {
        let mut problems = Vec::new();
        let problem = |message: String| Problem::new(ProblemKind::Learnset, message);
        for (index, move_name) in moves.iter().enumerate() {
            let the_move = match self.legality.move_named(self.aliases, move_name) {
                Named::Found(the_move) => the_move,
                // (`validateSet` has refused a move that does not exist already.)
                _ => continue,
            };
            if let Some(found) = self.check_can_learn(the_move, species, set_sources, set) {
                problems.push(
                    problem(format!("{name}{found}")).at(super::validate::SetField::Move(index)),
                );
                break;
            }
        }
        if set_sources.size() != 0 && set_sources.move_evo_carry_count > 3 {
            if set_sources.sources_before < 6 {
                set_sources.sources_before = 0;
            }
            set_sources
                .sources
                .retain(|s| char_at(s, 1) == 'E' && char_at(s, 0).to_digit(10).unwrap_or(0) >= 6);
            if set_sources.size() == 0 {
                problems.push(problem(format!(
                    "{name} needs to know {} to evolve, so it can only know 3 other moves from {}.",
                    species.evo_move.as_deref().unwrap_or("a Fairy-type move"),
                    species.prevo.as_deref().unwrap_or("undefined")
                )));
            }
        }
        if !problems.is_empty() {
            return problems;
        }
        if set_sources.is_hidden == Some(true) {
            set_sources
                .sources
                .retain(|s| char_at(s, 0).to_digit(10).unwrap_or(0) >= 5);
            if set_sources.sources_before < 5 {
                set_sources.sources_before = 0;
            }
            let can_use_ability_patch = self.generation >= 8 && self.mod_name != "gen8dlc1";
            if set_sources.size() == 0
                && !can_use_ability_patch
                && self.table.has("obtainableabilities")
            {
                problems.push(problem(format!(
                    "{name} has a hidden ability - it can't have moves only learned before gen 5."
                )));
                return problems;
            }
        }
        if let Some(baby_id) = set_sources.baby_only.clone()
            && !set_sources.sources.is_empty()
        {
            let baby = self.species(&baby_id);
            let baby_evo = baby.evos.first().map(|e| to_id(e)).unwrap_or_default();
            set_sources
                .sources
                .retain(|source| match char_at(source, 1) {
                    'S' => {
                        let id = source.split(' ').nth(1).unwrap_or_default();
                        id == baby.id
                    }
                    'E' | 'D' => baby_evo.is_empty() || source[2..] != baby_evo,
                    _ => true,
                });
            if set_sources.size() == 0 {
                problems.push(problem(format!(
                    "{name}'s event/egg moves are from an evolution, and are incompatible with its \
                     moves from {}.",
                    baby.name
                )));
            }
        }
        if let Some(baby_id) = set_sources.baby_only.clone()
            && set_sources.size() != 0
            && self.generation > 2
        {
            let baby = self.species(&baby_id);
            let baby_gen = u32::from(baby.generation);
            set_sources.sources.retain(|source| {
                // `baby.gen > parseInt(…)`: false for NaN, the source kept.
                if char_at(source, 0)
                    .to_digit(10)
                    .is_some_and(|g| baby_gen > g)
                    && !source.starts_with("1ST")
                {
                    return false;
                }
                !(baby_gen > 2 && source == "7V")
            });
            if set_sources.sources_before < baby_gen {
                set_sources.sources_before = 0;
            }
            if set_sources.size() == 0 {
                problems.push(problem(format!(
                    "{name} has moves from before Gen {baby_gen}, which are incompatible with its \
                     moves from {}.",
                    baby.name
                )));
            }
        }
        problems
    }

    /// Showdown `getEventOnlyData(species)`: the events of an event-only species (its own, its
    /// prevolution's, its base forme's).
    pub fn event_only_data(
        &self,
        species: &SpeciesFacts,
        no_recurse: bool,
    ) -> Option<(Cow<'d, SpeciesFacts>, &'d Vec<EventInfo>)> {
        let data = self.learnset_data(&species.id);
        if !data.event_only {
            if no_recurse {
                return None;
            }
            let prevo = self.species(species.prevo.as_deref().unwrap_or_default());
            return self.event_only_data(&prevo, true);
        }
        if data.event_data.is_none() && species.forme.is_some() {
            let base = self.species(&species.base_species);
            return self.event_only_data(&base, true);
        }
        data.event_data
            .as_ref()
            .map(|events| (self.species(&species.id), events))
    }

    /// Showdown `validateSource(set, source, setSources, species, because)`: `None` when the
    /// set can come from the source, else its problems (none listed without `because`).
    pub fn validate_source(
        &self,
        set: &mut TeamSet,
        source: &str,
        set_sources: &Sources,
        species: &SpeciesFacts,
        because: Option<&str>,
    ) -> Option<Vec<String>> {
        let kind = char_at(source, 1);
        let (event, event_species): (Cow<'_, EventInfo>, Cow<'_, SpeciesFacts>) = if kind == 'S' {
            let rest = &source[if char_at(source, 2) == 'T' { 3 } else { 2 }..];
            let mut parts = rest.split(' ');
            let index = leading_number(parts.next().unwrap_or_default()) as usize;
            let id = parts.next().unwrap_or_default();
            // Gen 1's events are Gen 2's.
            let event_species = if self.generation == 1 {
                self.legality_of(2)
                    .and_then(|gen2| gen2.species_named(self.aliases, id).found().cloned())
                    .map(Cow::Owned)
                    .unwrap_or_else(|| self.species(id))
            } else {
                self.species(id)
            };
            // Showdown throws on a source with no event behind it: not a source then.
            let Some(event) = self
                .learnset_data(&event_species.id)
                .event_data
                .as_ref()
                .and_then(|events| events.get(index))
            else {
                return Some(Vec::new());
            };
            (Cow::Borrowed(event), event_species)
        } else if source == "7V" {
            let is_mew = species.id == "mew";
            let is_celebi = species.id == "celebi";
            let g7_name = if species.generation > 2 && species.prevo.is_some() {
                species.prevo.clone().unwrap()
            } else {
                species.id.clone()
            };
            let event = EventInfo {
                generation: 2,
                level: Some(if is_mew {
                    5
                } else if is_celebi {
                    30
                } else {
                    3
                }),
                perfect_ivs: Some(if is_mew || is_celebi { 5 } else { 3 }),
                is_hidden: self.hidden_ability_in(7, &g7_name),
                shiny: if is_mew {
                    EventShiny::Never
                } else {
                    EventShiny::May
                },
                from: Some("Gen 1-2 Virtual Console transfer".into()),
                ..EventInfo::default()
            };
            (Cow::Owned(event), Cow::Borrowed(species))
        } else if source == "8V" {
            let is_mew = species.id == "mew";
            let event = EventInfo {
                generation: 8,
                perfect_ivs: is_mew.then_some(3),
                shiny: if is_mew {
                    EventShiny::Never
                } else {
                    EventShiny::May
                },
                from: Some("Gen 7 Let's Go! HOME transfer".into()),
                ..EventInfo::default()
            };
            (Cow::Owned(event), Cow::Borrowed(species))
        } else if kind == 'D' {
            let event = EventInfo {
                generation: 5,
                level: Some(10),
                from: Some("Gen 5 Dream World".into()),
                is_hidden: self.hidden_ability_in(5, &species.id),
                ..EventInfo::default()
            };
            (Cow::Owned(event), Cow::Borrowed(species))
        } else if kind == 'E' {
            // An egg: some father must pass its egg moves (`findEggMoveFathers`); without the
            // learnsets, the seam's empty side, one always does.
            if self.learnsets.is_none()
                || self.find_egg_move_fathers(source, species, set_sources, &mut Vec::new(), false)
            {
                return None;
            }
            return Some(Vec::new());
        } else {
            // Showdown throws on a source it cannot identify.
            return Some(Vec::new());
        };
        self.validate_event(
            set,
            set_sources,
            &event,
            &event_species,
            because,
            "from an event",
        )
    }

    /// Showdown `validateEvent(set, setSources, eventData, eventSpecies, because, from)`: `None`
    /// when the set matches the event (its gender then the event's), else its problems (none
    /// listed in the fast form, without `because`).
    pub fn validate_event(
        &self,
        set: &mut TeamSet,
        set_sources: &Sources,
        event: &EventInfo,
        event_species: &SpeciesFacts,
        because: Option<&str>,
        from: &str,
    ) -> Option<Vec<String>> {
        let table = &self.table;
        let generation = self.generation;
        let species = self.species(&set.species);
        let max_source_gen = if table.has("allowtradeback") {
            (generation + 1).clamp(1, 8)
        } else {
            generation
        };
        let mut name = set.species.clone();
        if !set.name.is_empty() && set.species != set.name && species.base_species != set.name {
            name = format!("{} ({})", set.name, set.species);
        }
        let fast = because.is_none();
        let because = because.unwrap_or_default();
        let from = match &event.from {
            Some(origin) => format!("from {origin}"),
            None => from.to_string(),
        };
        let etc = format!("{because} {from}");
        let mut problems: Vec<String> = Vec::new();
        macro_rules! fail {
            ($message:expr) => {{
                if fast {
                    return Some(Vec::new());
                }
                problems.push($message);
            }};
        }
        let min_source_gen = table.numbers.min_source_gen;
        if generation < 8 && min_source_gen > event.generation {
            fail!(format!(
                "This format requires Pokemon from gen {min_source_gen} or later and {name} is \
                 from gen {}{etc}.",
                event.generation
            ));
        }
        if max_source_gen < event.generation {
            fail!(format!(
                "This format is in gen {generation} and {name} is from gen {}{etc}.",
                event.generation
            ));
        }
        if event.japan && self.mod_name != "gen1jpn" {
            fail!(format!(
                "{name} has moves from Japan-only events, but this format simulates \
                 International Yellow/Crystal which can't trade with Japanese games."
            ));
        }
        if let Some(level) = event.level.filter(|l| *l != 0)
            && set.level < level
        {
            fail!(format!("{name} must be at least level {level}{etc}."));
        }
        if (generation == 3 || generation == 4)
            && event.level == Some(100)
            && let Some(evs) = &set.evs
        {
            for ev in [evs.hp, evs.atk, evs.def, evs.spa, evs.spd, evs.spe] {
                if ev > 100 {
                    problems.push(format!(
                        "{name} can't have more than 100 EVs in any stat, because it is only \
                         obtainable from level 100 events. Level 100 Pokemon can only gain EVs \
                         from vitamins (Carbos etc), which are capped at 100 EVs."
                    ));
                }
                if !(ev % 10 == 0 || (ev % 10 == 8 && ev % 4 == 0)) {
                    problems.push(format!(
                        "{name} can only have EVs that are multiples of 10, because it is only \
                         obtainable from level 100 events. Level 100 Pokemon can only gain EVs \
                         from vitamins (Carbos etc), which boost in multiples of 10."
                    ));
                }
            }
        }
        let event_shiny = event.shiny != EventShiny::Never;
        if (event.shiny == EventShiny::Must && !set.shiny) || (!event_shiny && set.shiny) {
            let required = if event_shiny {
                " be shiny"
            } else {
                " not be shiny"
            };
            fail!(format!("{name} must{required}{etc}."));
        }
        if let Some(gender) = &event.gender
            && !set.gender.is_empty()
            && *gender != set.gender
        {
            fail!(format!("{name}'s gender must be {gender}{etc}."));
        }
        let can_mint = generation > 7;
        if let Some(nature) = &event.nature
            && *nature != set.nature
            && !can_mint
        {
            fail!(format!(
                "{name} must have a {nature} nature{etc} - Mints are only available starting gen 8."
            ));
        }
        let mut required_ivs = 0;
        if let Some(ivs) = &event.ivs {
            let can_bottle_cap =
                generation >= 7 && set.level >= if generation < 9 { 100 } else { 50 };
            let set_ivs = *set.ivs.get_or_insert(super::set::Stats::filled(31));
            for (stat, required) in &ivs.0 {
                let have = match stat.as_str() {
                    "hp" => set_ivs.hp,
                    "atk" => set_ivs.atk,
                    "def" => set_ivs.def,
                    "spa" => set_ivs.spa,
                    "spd" => set_ivs.spd,
                    _ => set_ivs.spe,
                };
                if can_bottle_cap && have == 31 {
                    continue;
                }
                if have != *required {
                    fail!(format!(
                        "{name} must have {required} {} IVs{etc}.",
                        stat_name(stat)
                    ));
                }
            }
            if can_bottle_cap && ivs.0.len() >= 6 {
                let stat = |name: &str| ivs.get(name).unwrap_or(0);
                let stats = super::set::Stats {
                    hp: stat("hp"),
                    atk: stat("atk"),
                    def: stat("def"),
                    spa: stat("spa"),
                    spd: stat("spd"),
                    spe: stat("spe"),
                };
                let required_type = self.hidden_power_type(&stats);
                if !set.hp_type.is_empty() && set.hp_type != required_type {
                    fail!(format!(
                        "{name} can only have Hidden Power {required_type}{etc}."
                    ));
                }
                set.hp_type = required_type.into();
            }
        } else {
            required_ivs = event.perfect_ivs.unwrap_or(0);
        }
        if required_ivs != 0
            && let Some(ivs) = &set.ivs
        {
            let perfect = [ivs.hp, ivs.atk, ivs.def, ivs.spa, ivs.spd, ivs.spe]
                .iter()
                .filter(|iv| **iv >= 31)
                .count() as u32;
            if perfect < required_ivs {
                if fast {
                    return Some(Vec::new());
                }
                if event.perfect_ivs.is_some_and(|p| p != 0) {
                    problems.push(format!(
                        "{name} must have at least {required_ivs} perfect IVs{etc}."
                    ));
                }
            }
            if generation >= 3 && required_ivs >= 3 && set.hp_type == "Fighting" {
                fail!(format!(
                    "{name} can't use Hidden Power Fighting because it must have at least three \
                     perfect IVs{etc}."
                ));
            } else if generation >= 3
                && required_ivs >= 5
                && !set.hp_type.is_empty()
                && !["Dark", "Dragon", "Electric", "Steel", "Ice"].contains(&set.hp_type.as_str())
            {
                fail!(format!(
                    "{name} can only use Hidden Power Dark/Dragon/Electric/Steel/Ice because it \
                     must have at least 5 perfect IVs{etc}."
                ));
            }
        }
        if table.has("obtainablemoves") {
            let ss_max = set_sources.max_source_gen();
            let tradeback_eligible =
                generation == 2 && (species.generation == 1 || event_species.generation == 1);
            if ss_max != 0 && event.generation > ss_max && !tradeback_eligible {
                fail!(format!(
                    "{name} must not have moves only learnable in gen {ss_max}{etc}."
                ));
            }
            if event.from.as_deref() == Some("Gen 5 Dream World")
                && set_sources.dream_world_move_count > 1
            {
                problems.push(format!("{name} can only have one Dream World move."));
            }
        }
        if table.has("obtainableabilities") {
            if generation <= 5
                && let Some(abilities) = &event.abilities
                && abilities.len() == 1
                && !event.is_hidden
            {
                if species.name == event_species.name {
                    // Not evolved: the ability must match.
                    let required = self.ability(&abilities[0]).name.clone();
                    if set.ability != required {
                        fail!(format!("{name} must have {required}{etc}."));
                    }
                } else {
                    // Evolved: the event's ability slot.
                    let ability1 = self.ability(
                        event_species
                            .abilities
                            .get("1")
                            .map(String::as_str)
                            .unwrap_or_default(),
                    );
                    if ability1.generation != 0
                        && event.generation >= u32::from(ability1.generation)
                    {
                        let slot = if to_id(&abilities[0]) == ability1.id {
                            "1"
                        } else {
                            "0"
                        };
                        let required_name = species
                            .abilities
                            .get(slot)
                            .or_else(|| species.abilities.get("0"))
                            .cloned()
                            .unwrap_or_default();
                        let required = self.ability(&required_name).name.clone();
                        if set.ability != required {
                            let original = self.ability(&abilities[0]).name.clone();
                            fail!(format!(
                                "{name} must have {required}{because} from a {original} {} event.",
                                event_species.name
                            ));
                        }
                    }
                }
            }
            if let Some(hidden) = species.abilities.get("H") {
                let is_hidden = set.ability == *hidden;
                if !is_hidden && event.is_hidden && generation <= 8 {
                    fail!(format!("{name} must have its Hidden Ability{etc}."));
                }
                let can_use_ability_patch = generation >= 8 && self.mod_name != "gen8dlc1";
                if is_hidden && !event.is_hidden && !can_use_ability_patch {
                    fail!(format!("{name} must not have its Hidden Ability{etc}."));
                }
            }
        }
        if !problems.is_empty() {
            return Some(problems);
        }
        if let Some(gender) = &event.gender {
            set.gender = gender.clone();
        }
        None
    }
}

/// `parseInt(source.charAt(0))` as a number that can be NaN (a source that begins with no digit),
/// so comparisons with it are false as in JavaScript.
fn source_gen_f64(source: &str) -> f64 {
    char_at(source, 0).to_digit(10).map_or(f64::NAN, f64::from)
}

impl<'d> TeamValidator<'d> {
    /// The dex Gen 1's eggs come from (`dex.mod('gen2')`), else the format's: its species and its
    /// learnsets.
    fn egg_dex(
        &self,
    ) -> (
        &'d super::legality::LegalityData,
        super::learnset::Learnsets<'d>,
    ) {
        let learnsets = self
            .learnsets
            .expect("the father search reads the learnsets");
        if self.generation == 1
            && let (Some(gen2), Ok(gen2_learnsets)) =
                (self.legality_of(2), self.library.learnsets("gen2"))
        {
            return (gen2, gen2_learnsets);
        }
        (self.legality, learnsets)
    }

    /// Showdown `findEggMoveFathers(source, species, setSources, false, blacklist, noRecurse)`:
    /// whether a father can pass the egg moves the source needs.
    pub fn find_egg_move_fathers(
        &self,
        source: &str,
        species: &SpeciesFacts,
        set_sources: &Sources,
        blacklist: &mut Vec<String>,
        no_recurse: bool,
    ) -> bool {
        if !blacklist.contains(&species.id) {
            blacklist.push(species.id.clone());
        }
        // A tradeback's egg is a Gen 2 egg. `Math.max(NaN, 2)` is NaN (Rust's `max` would drop
        // the NaN).
        let parsed = source_gen_f64(source);
        let egg_gen = if parsed.is_nan() {
            f64::NAN
        } else {
            parsed.max(2.0)
        };
        if egg_gen >= 6.0 && set_sources.level_up_egg_moves.is_none() && species.mother.is_none() {
            return true;
        }
        // `limitedEggMoves`, without Pomeg's in Gen 3 (`null?.filter` is undefined).
        let mut egg_moves: Option<Vec<String>> = match &set_sources.limited_egg_moves {
            Tri::Some(moves) if egg_gen == 3.0 => Some(
                moves
                    .iter()
                    .filter(|m| {
                        !set_sources
                            .pomeg_egg_moves
                            .as_ref()
                            .is_some_and(|pomeg| pomeg.contains(m))
                    })
                    .cloned()
                    .collect(),
            ),
            Tri::Some(moves) => Some(moves.clone()),
            _ => None,
        };
        let Some(moves) = &egg_moves else {
            return true;
        };
        if moves.len() <= 1 && set_sources.level_up_egg_moves.is_none() {
            return true;
        }
        if let Some(level_up) = &set_sources.level_up_egg_moves
            && egg_gen >= 6.0
        {
            egg_moves = Some(level_up.clone());
        }
        let egg_moves = egg_moves.unwrap();
        let (dex, dex_learnsets) = self.egg_dex();
        let species_in = |name: &str| -> Option<SpeciesFacts> {
            dex.species_named(self.aliases, name).found().cloned()
        };
        let mut egg_groups = species.egg_groups.clone();
        if species.id == "nidoqueen" || species.id == "nidorina" {
            egg_groups = species_in("nidoranf")
                .map(|s| s.egg_groups)
                .unwrap_or_default();
        } else if species.id == "shedinja" {
            egg_groups = species_in("nincada")
                .map(|s| s.egg_groups)
                .unwrap_or_default();
        } else if self.generation == 1 {
            egg_groups = species_in(&species.id)
                .map(|s| s.egg_groups)
                .unwrap_or_default();
        }
        if egg_groups.first().map(String::as_str) == Some("Undiscovered") {
            egg_groups = species
                .evos
                .first()
                .and_then(|evo| species_in(evo))
                .map(|s| s.egg_groups)
                .unwrap_or_default();
        }
        if egg_groups.is_empty() || egg_groups[0] == "Undiscovered" {
            // Showdown throws: a species with no egg groups for the source.
            return false;
        }
        let smeargle_standard = self.species("Smeargle").nonstandard.is_none();
        if egg_groups.iter().any(|g| g == "Field") && smeargle_standard {
            return true;
        }
        for father_id in &dex.species_order {
            let Some(father) = dex.species(father_id) else {
                continue;
            };
            if father.nonstandard.is_some() || f64::from(father.generation) > egg_gen {
                continue;
            }
            if matches!(father.gender.as_deref(), Some("N" | "F")) {
                continue;
            }
            if dex_learnsets.data(&father.id).learnset.is_none() {
                continue;
            }
            if blacklist.contains(&father.id)
                && !["dragonite", "snorlax"].contains(&father.id.as_str())
            {
                continue;
            }
            // Not an NFE father: its evolution knows its moves and more (Combee, Salandit
            // excepted, their evolutions being female).
            if let Some(evolved) = father.evos.first().and_then(|evo| species_in(evo))
                && f64::from(evolved.generation) <= egg_gen
                && evolved.gender.as_deref() != Some("F")
            {
                continue;
            }
            if !father.egg_groups.iter().any(|g| egg_groups.contains(g)) {
                continue;
            }
            if !self.father_can_learn(species, father, &egg_moves, egg_gen, blacklist, no_recurse) {
                continue;
            }
            return true;
        }
        false
    }

    /// Showdown `fatherCanLearn(baseSpecies, species, moves, eggGen, blacklist, noRecurse)`: the
    /// father can know every egg move, with at most one from a restrictive source — a chain
    /// bred at most once more.
    fn father_can_learn(
        &self,
        base_species: &SpeciesFacts,
        father: &SpeciesFacts,
        moves: &[String],
        egg_gen: f64,
        blacklist: &mut Vec<String>,
        no_recurse: bool,
    ) -> bool {
        if self.learnset_data(&father.id).learnset.is_none() {
            return false;
        }
        if father.id == "smeargle" {
            return true;
        }
        let can_breed_with_smeargle = father.egg_groups.iter().any(|g| g == "Field")
            && self.species("Smeargle").nonstandard.is_none();
        let mut all_egg_sources = Sources {
            sources_before: if egg_gen.is_nan() { 0 } else { egg_gen as u32 },
            ..Sources::default()
        };
        let egg_gen_u = all_egg_sources.sources_before;
        for egg_move in moves {
            let mut egg_sources = Sources::default();
            for (current, learnset) in self.full_learnset(&father.id) {
                let egg_pokemon = if current.prevo.is_some() {
                    current.id.clone()
                } else {
                    String::new()
                };
                if let Some(sources) = learnset.get(egg_move) {
                    for move_source in sources {
                        let source_gen = source_gen_f64(move_source);
                        if egg_gen > 8.0 && source_gen <= 8.0 {
                            continue;
                        }
                        if source_gen > egg_gen {
                            continue;
                        }
                        let kind = char_at(move_source, 1);
                        let from_smeargle = kind == 'E' && can_breed_with_smeargle;
                        if !"ESDV".contains(kind) || from_smeargle {
                            egg_sources.add_gen(source_gen as u32);
                            break;
                        }
                        if kind == 'E' {
                            egg_sources.add(
                                &format!("{move_source}{egg_pokemon}"),
                                EggLimit::Limited(egg_move.clone()),
                            );
                            let move_gen = self
                                .legality
                                .move_named(self.aliases, egg_move)
                                .found()
                                .map_or(0, |m| m.generation);
                            if egg_gen == 2.0 && move_gen == 1 {
                                egg_sources.add(
                                    &format!("1ET{egg_pokemon}"),
                                    EggLimit::Limited(egg_move.clone()),
                                );
                            }
                        } else {
                            egg_sources.add(&format!("{move_source}{egg_pokemon}"), EggLimit::None);
                        }
                    }
                }
                if egg_sources.sources_before == egg_gen_u && egg_gen_u != 0 {
                    break;
                }
            }
            if egg_sources.sources_before == egg_gen_u && egg_gen_u != 0 {
                continue;
            }
            if egg_sources.sources_before == 0 && egg_sources.sources.is_empty() {
                return false;
            }
            let has_egg = egg_sources.sources.iter().any(|s| char_at(s, 1) == 'E');
            if egg_gen >= 3.0
                && has_egg
                && egg_sources.limited_egg_moves.is_null()
                && egg_sources.sources_before != 0
            {
                egg_sources.possibly_limited_egg_moves = Some(vec![to_id(&format!(
                    "{}{egg_move}",
                    egg_sources.sources_before
                ))]);
            }
            all_egg_sources.intersect_with(&mut egg_sources);
            if all_egg_sources.size() == 0 {
                return false;
            }
        }
        blacklist.push(father.id.clone());
        if let Tri::Some(limited) = &all_egg_sources.limited_egg_moves
            && limited.len() > 1
        {
            if no_recurse {
                return false;
            }
            let can_chainbreed = father
                .egg_groups
                .iter()
                .any(|g| !base_species.egg_groups.contains(g));
            if !can_chainbreed && limited.len() == moves.len() {
                return false;
            }
            let set_sources = Sources {
                limited_egg_moves: Tri::Some(limited.clone()),
                ..Sources::default()
            };
            let first = all_egg_sources.sources.first().cloned().unwrap_or_default();
            return self.find_egg_move_fathers(&first, father, &set_sources, blacklist, true);
        }
        true
    }
}
