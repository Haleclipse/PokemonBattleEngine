//! Where a Pokémon can come from (Showdown `PokemonSources`, DESIGN.md §45.3): every source at
//! or before `sources_before`, none before `sources_after`, plus the sources named one by one
//! (`5E`, `4S11 pikachu`, `8V` …), and what narrows them — limited egg moves, a restricted move,
//! the learnset domain, the Sketch and HM limits. Each move's possible sources are intersected
//! with the set's; an empty intersection is an illegal moveset.

use super::data::to_id;

/// A JavaScript field that is `undefined`, `null` or a value, the three told apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Tri<T> {
    #[default]
    Undefined,
    Null,
    Some(T),
}

impl<T> Tri<T> {
    pub fn is_null(&self) -> bool {
        matches!(self, Tri::Null)
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            Tri::Some(value) => Some(value),
            _ => None,
        }
    }
}

/// A limited egg move passed to `add`: none (`undefined`), unlimited (`null`), or limited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EggLimit {
    None,
    Unlimited,
    Limited(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources {
    pub sources: Vec<String>,
    pub sources_before: u32,
    pub sources_after: u32,
    pub is_hidden: Option<bool>,
    pub limited_egg_moves: Tri<Vec<String>>,
    pub possibly_limited_egg_moves: Option<Vec<String>>,
    pub tradeback_limited_egg_moves: Option<Vec<String>>,
    pub level_up_egg_moves: Option<Vec<String>>,
    pub pomeg_egg_moves: Option<Vec<String>>,
    pub pomeg_event_egg: Tri<String>,
    pub event_only_min_source_gen: Option<u32>,
    pub learnset_domain: Option<Vec<String>>,
    pub restrictive_moves: Option<Vec<String>>,
    pub restricted_move: Option<String>,
    pub move_evo_carry_count: u32,
    pub dream_world_move_count: u32,
    pub baby_only: Option<String>,
    pub sketch_move: Option<String>,
    pub hm: Option<String>,
    pub is_from_pokemon_go: Option<bool>,
    pub go_incompatible_moves: Option<Vec<String>>,
    /// `purified`: a Giovanni Shadow Pokémon purified and levelled down.
    pub pokemon_go_source: Option<String>,
}

/// `parseInt(source.charAt(0))`: the source's generation, or NaN.
fn source_gen(source: &str) -> Option<u32> {
    source.chars().next()?.to_digit(10)
}

impl Sources {
    pub fn new(sources_before: u32, sources_after: u32) -> Self {
        Self {
            sources_before,
            sources_after,
            ..Self::default()
        }
    }

    /// Showdown `size()`: every source before a generation counts as infinitely many.
    pub fn size(&self) -> usize {
        if self.sources_before != 0 {
            usize::MAX
        } else {
            self.sources.len()
        }
    }

    /// Showdown `add(source, limitedEggMove)`. A limited egg move replaces the list, an
    /// unlimited one (`null`) clears it for good.
    pub fn add(&mut self, source: &str, limit: EggLimit) {
        if self.sources.last().map(String::as_str) != Some(source) {
            self.sources.push(source.to_string());
        }
        if let EggLimit::Limited(egg_move) = &limit {
            if source.starts_with("1ET") {
                self.tradeback_limited_egg_moves = Some(vec![egg_move.clone()]);
            }
            if !self.limited_egg_moves.is_null() {
                self.limited_egg_moves = Tri::Some(vec![egg_move.clone()]);
            }
        } else if limit == EggLimit::Unlimited {
            self.limited_egg_moves = Tri::Null;
        }
    }

    /// Showdown `addGen(sourceGen)`.
    pub fn add_gen(&mut self, generation: u32) {
        self.sources_before = self.sources_before.max(generation);
        self.limited_egg_moves = Tri::Null;
    }

    /// Showdown `minSourceGen()`.
    pub fn min_source_gen(&self) -> u32 {
        if let Some(generation) = self.event_only_min_source_gen.filter(|g| *g != 0) {
            return generation;
        }
        if self.sources_before != 0 {
            return if self.sources_after == 0 {
                1
            } else {
                self.sources_after
            };
        }
        self.sources
            .iter()
            .filter_map(|source| source_gen(source))
            .min()
            .unwrap_or(0)
    }

    /// Showdown `maxSourceGen()`.
    pub fn max_source_gen(&self) -> u32 {
        self.sources
            .iter()
            .filter_map(|source| source_gen(source))
            .fold(self.sources_before, u32::max)
    }

    /// Showdown `addGoIncompatibleMove(move)`.
    pub fn add_go_incompatible_move(&mut self, name: &str) {
        let moves = self.go_incompatible_moves.get_or_insert_with(Vec::new);
        if !name.is_empty() {
            moves.push(name.to_string());
        }
        self.is_from_pokemon_go = Some(false);
    }

    /// Showdown `intersectWith(other)`: the sources both allow, and what limits either.
    pub fn intersect_with(&mut self, other: &mut Sources) {
        if self.pomeg_event_egg.value().is_some() && other.pomeg_egg_moves.is_some() {
            let event = self.pomeg_event_egg.value().unwrap().clone();
            other.sources = other
                .sources
                .iter()
                .map(|s| {
                    if s.starts_with("3E") {
                        event.clone()
                    } else {
                        s.clone()
                    }
                })
                .collect();
        } else if other.pomeg_event_egg.value().is_some() && !self.pomeg_event_egg.is_null() {
            let event = other.pomeg_event_egg.value().unwrap().clone();
            self.sources = self
                .sources
                .iter()
                .map(|s| {
                    if s.starts_with("3E") {
                        event.clone()
                    } else {
                        s.clone()
                    }
                })
                .collect();
            self.pomeg_event_egg = Tri::Some(event);
        } else if other.pomeg_egg_moves.is_none() && other.sources_before == 0 {
            self.pomeg_event_egg = Tri::Null;
        }
        if other.sources_before != 0 || self.sources_before != 0 {
            // Every source before a generation, written out, before the intersection.
            if other.sources_before > self.sources_before {
                for source in &self.sources {
                    if source_gen(source).is_some_and(|g| g <= other.sources_before) {
                        other.sources.push(source.clone());
                    }
                }
            } else if self.sources_before > other.sources_before {
                let mut added = Vec::new();
                for source in &other.sources {
                    if source_gen(source).is_some_and(|g| g <= self.sources_before) {
                        added.push(source.clone());
                    }
                }
                self.sources.extend(added);
            }
            self.sources_before = self.sources_before.min(other.sources_before);
        }
        if !self.sources.is_empty() {
            if other.sources.is_empty() {
                self.sources.clear();
            } else {
                self.sources.retain(|source| other.sources.contains(source));
            }
        }
        if let Some(restricted) = &other.restricted_move
            && self.restricted_move.as_ref() != Some(restricted)
        {
            if self.restricted_move.is_some() {
                // Two restricted moves: incompatible.
                self.sources.clear();
                self.sources_before = 0;
            } else {
                self.restricted_move = Some(restricted.clone());
            }
        }
        if let Tri::Some(moves) = &other.limited_egg_moves {
            match &mut self.limited_egg_moves {
                Tri::Some(mine) => mine.extend(moves.iter().cloned()),
                mine => *mine = Tri::Some(moves.clone()),
            }
        }
        extend(
            &mut self.possibly_limited_egg_moves,
            &other.possibly_limited_egg_moves,
        );
        extend(
            &mut self.tradeback_limited_egg_moves,
            &other.tradeback_limited_egg_moves,
        );
        extend(&mut self.level_up_egg_moves, &other.level_up_egg_moves);
        extend(&mut self.pomeg_egg_moves, &other.pomeg_egg_moves);
        if let Some(domain) = &other.learnset_domain
            && self.learnset_domain.is_none()
        {
            // (Showdown filters an existing domain by the other's and drops the result: an
            // existing one is kept as it is.)
            self.learnset_domain = Some(domain.clone());
        }
        if let Some(possibly) = &self.possibly_limited_egg_moves
            && self.sources_before == 0
        {
            // `parseInt(eggSources[0])`, then the least egg source's generation: NaN without one.
            let eggs: Vec<&String> = self
                .sources
                .iter()
                .filter(|s| s.chars().nth(1) == Some('E'))
                .collect();
            let min_egg_gen = eggs
                .first()
                .and_then(|first| source_gen(first))
                .map(|first| {
                    eggs.iter()
                        .filter_map(|s| source_gen(s))
                        .fold(first, u32::min)
                });
            if let Some(min_egg_gen) = min_egg_gen.filter(|g| *g != 0) {
                let possibly = possibly.clone();
                for egg_move_and_gen in &possibly {
                    let limited = match &mut self.limited_egg_moves {
                        Tri::Some(list) => list,
                        other => {
                            *other = Tri::Some(Vec::new());
                            match other {
                                Tri::Some(list) => list,
                                _ => unreachable!(),
                            }
                        }
                    };
                    if source_gen(egg_move_and_gen).is_some_and(|g| g < min_egg_gen) {
                        let egg_move = to_id(&egg_move_and_gen[1..]);
                        if !limited.contains(&egg_move) {
                            limited.push(egg_move);
                        }
                    }
                }
            }
        }
        let egg_tradeback_legal = self.sources.iter().any(|s| s.starts_with("1ET"));
        if !egg_tradeback_legal && let Some(tradeback) = self.tradeback_limited_egg_moves.clone() {
            for egg_move in tradeback {
                let limited = match &mut self.limited_egg_moves {
                    Tri::Some(list) => list,
                    other => {
                        *other = Tri::Some(Vec::new());
                        match other {
                            Tri::Some(list) => list,
                            _ => unreachable!(),
                        }
                    }
                };
                if !limited.contains(&egg_move) {
                    limited.push(egg_move);
                }
            }
        }
        self.move_evo_carry_count += other.move_evo_carry_count;
        self.dream_world_move_count += other.dream_world_move_count;
        if other.sources_after > self.sources_after {
            self.sources_after = other.sources_after;
        }
        if other.is_hidden == Some(true) {
            self.is_hidden = Some(true);
        }
        // `this.restrictiveMoves` is truthy when empty too: no move then, but no Pokémon GO.
        if other.sources_before < 8
            && !other.sources.iter().any(|s| s == "8V")
            && let Some(moves) = &self.restrictive_moves
        {
            let last = moves.last().cloned().unwrap_or_default();
            self.add_go_incompatible_move(&last);
        }
    }
}

fn extend(mine: &mut Option<Vec<String>>, other: &Option<Vec<String>>) {
    if let Some(other) = other {
        match mine {
            Some(list) => list.extend(other.iter().cloned()),
            None => *mine = Some(other.clone()),
        }
    }
}
