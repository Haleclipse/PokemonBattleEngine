//! The format layer (M9, DESIGN.md §45): formats are data, resolved into a rule table the way
//! Showdown composes them, and read by the team validator and the pre-battle session — never by a
//! move's or an ability's code (`Generation ≠ BattleMode ≠ Format`). It reads no battle: the
//! pre-battle session (`crate::session`) builds the battle from what it validates.

mod can_learn;
mod data;
mod learnset;
mod legality;
mod library;
mod pokemon_go;
mod resolve;
mod set;
mod sources;
mod stats;
mod table;
mod validate;
mod validate_set;
mod validate_team;

pub use data::{FORMATS_SCHEMA, FormatData, FormatEntry, HasValue, to_id};
pub use learnset::{
    Encounter, EventInfo, EventShiny, LEARNSET_SCHEMA, Learnset, LearnsetTable, Learnsets,
    MoveSources,
};
pub use legality::{
    Aliases, BanError, BanKind, BattleOnly, GenderRatio, LEGALITY_SCHEMA, LegalityData, MaxMove,
    Named, SpeciesFacts, TAGS, Tag, ThingFacts, TypeFacts, Unreleased,
};
pub use library::{DataError, FormatLibrary};
pub use resolve::{FormatError, FormatResolver, RuleSpec};
pub use set::{Stats, TeamSet};
pub use sources::{EggLimit, Sources, Tri};
pub use table::{ComplexBan, OrderedMap, RuleNumbers, RuleTable};
pub use validate::{
    Problem, ProblemKind, ProblemLocation, SetField, SetHas, SpeciesSet, TeamValidator, Verdict,
};
pub use validate_set::{TeamHas, clean_name};
