//! The pre-battle session (M9.4, DESIGN.md §45.5): from the teams the players register under a
//! real format to the battle the engine runs. It sits above both the format layer (the rule
//! table, the team validator) and the engine; neither reads it.
//!
//! - **The format as a session reads it** (`SessionFormat`): the validator, what the format asks
//!   of the teams before the battle (`TeamPolicy`: their size, the picks, Team Preview), and
//!   the switches it hands the engine (`FormatRules`: the generation, the battle mode, the
//!   special mechanics its clauses leave).
//! - **Registration** (`SessionFormat::register`): Showdown's validation of the whole team as
//!   registered — Species Clause, Item Clause and every other registration rule hold of all of
//!   it, picked or not — which normalises its sets; a gender the set leaves open is drawn here,
//!   since the preview shows it.
//! - **The team choice** (`BattleSession::choose`): with Team Preview or picks, each side orders
//!   its team by registered slot, leads first, and brings the first `picked`; neither sees the
//!   other's choice until both have chosen. What a side sees of the other's team is projected
//!   from the registered sets (`SessionView`: Team Preview's entries, the open team sheets).
//! - **The start** (`SessionFormat::start`): the chosen Pokémon only, in their order, built as
//!   Showdown's `Pokemon` constructor reads a set, the leads in the active slots;
//!   `BattleEngine::create_new` once, then the leads come in.
//!
//! A session's snapshot carries the registration's fingerprint (the rule table, the data, the
//! dex); the battle's own carries the battle rules'.

mod bootstrap;
mod format;
mod protocol;
mod view;

pub use bootstrap::SetBuildError;
pub use format::{
    OpenTeamSheets, RegisteredTeam, SessionError, SessionFormat, TeamPolicy, VisibilityPolicy,
    format_switches,
};
pub use protocol::{
    BattleSession, SessionPhase, SessionRequest, SessionSnapshot, SessionView, TeamChoice,
    TeamChoiceError, TeamSlot,
};
pub use view::PreviewEntry;
