//! A Pokémon battle engine, Generations 1-9, checked turn by turn against Pokémon Showdown and,
//! where they part, the cartridges (`DESIGN.md`, `TESTING.md`).
//!
//! # The stable API
//!
//! What this crate offers by default is its stable API; the modules behind it open with the
//! `internals` feature, with no compatibility promise (DESIGN.md §47.0 item 7). A battle goes:
//!
//! - **Before it:** a [`FormatLibrary`] of format data, a [`SessionFormat`], a [`BattleSession`]
//!   — each side registers a [`TeamSet`] list (validated as Showdown validates it), sees the
//!   other's Team Preview ([`BattleSession::view`]), may accept open team sheets, and chooses
//!   what it brings ([`TeamChoice`]); the session then starts the [`BattleEngine`].
//! - **During it:** [`BattleEngine::step`] runs the battle until it asks for something
//!   ([`EngineStep::Awaiting`] a [`BattleRequest`]) or ends ([`EngineStep::Finished`]). What a
//!   side may choose is [`BattleEngine::choice_options`]; its choices go to
//!   [`BattleEngine::submit_turn_choices`], [`BattleEngine::resolve_forced_switches`],
//!   [`BattleEngine::resolve_revive`], checked first if one likes
//!   ([`BattleEngine::validate_choice`]).
//! - **Seeing it:** [`BattleEngine::view`] is the battle as an [`Observer`](view::Observer)
//!   knows it — a player its own side whole and of the other side what was shown, a spectator
//!   what is public; unknown things are [`Seen::Unknown`](view::Seen). [`BattleEngine::
//!   omniscient_view`] shows everything, for trusted hosts, research and AI alone.
//!   [`BattleEngine::names`] names the ids the views give.
//! - **Forking it:** a [`BattleEngine`] is `Clone` (a rollout's fork), and
//!   [`BattleEngine::snapshot`] / [`BattleEngine::rewind`] go back to a moment, its random
//!   numbers included; [`TraceMode::Off`] keeps no trace, for speed.
//! - **Storing it:** a snapshot — a session's ([`BattleSession::snapshot`], for a server to keep
//!   a battle across restarts) or a battle's (for another process to rewind its copy of the
//!   battle to) — is written as JSON by `to_json` (one line, for storage) or `to_json_pretty`
//!   (for reading) and read back by `from_json`. See *Snapshots as JSON* below.
//!
//! ```
//! use pokemon_battle_engine::view::{Observer, Seen};
//! use pokemon_battle_engine::{
//!     BattleRequest, BattleSession, EngineStep, FormatLibrary, SessionFormat, SessionRequest,
//!     SideId, Stats, TeamChoice, TeamSet,
//! };
//!
//! let library = FormatLibrary::open(concat!(env!("CARGO_MANIFEST_DIR"), "/data")).unwrap();
//! let mut session = BattleSession::new(SessionFormat::new(&library, "gen9ou").unwrap(), 42);
//! let team = vec![TeamSet {
//!     species: "Garganacl".into(),
//!     item: "Leftovers".into(),
//!     ability: "Purifying Salt".into(),
//!     moves: ["Salt Cure", "Recover", "Protect", "Stealth Rock"].map(String::from).to_vec(),
//!     nature: "Careful".into(),
//!     evs: Some(Stats { hp: 252, def: 4, spd: 252, ..Stats::default() }),
//!     ..TeamSet::default()
//! }];
//! for side in [SideId::P1, SideId::P2] {
//!     session.register(side, team.clone()).unwrap();
//! }
//! if let SessionRequest::Choose { sides, .. } = session.request() {
//!     for side in sides {
//!         session.choose(side, TeamChoice::default()).unwrap();
//!     }
//! }
//! let mut battle = session.engine().unwrap().clone();
//!
//! // Run until the battle asks for the first turn's choices.
//! let request = loop {
//!     match battle.step().unwrap() {
//!         EngineStep::Progressed { .. } => continue,
//!         other => break other,
//!     }
//! };
//! assert!(matches!(request, EngineStep::Awaiting(BattleRequest::TurnChoices { .. })));
//!
//! // P1 knows its own Pokémon's item; of P2's, nothing was shown yet.
//! let view = battle.view(Observer::Side(SideId::P1));
//! let names = battle.names();
//! let (_, own) = view.side(SideId::P1).active().next().unwrap();
//! let Seen::Known(item) = own.unwrap().item() else { unreachable!() };
//! assert_eq!(names.item(item), Some("Leftovers"));
//! let (_, foe) = view.side(SideId::P2).active().next().unwrap();
//! assert_eq!(foe.unwrap().item(), Seen::Unknown);
//! ```
//!
//! The `stable-api` package's examples play whole battles this way: an AI's rollouts
//! (`rollout`), a server's loop (`server_loop`), a counterfactual (`research`).
//!
//! # Randomness and reproducibility
//!
//! A battle's randomness comes from one seed, the one [`BattleSession::new`] takes: it draws the
//! session's own random values (the genders a team leaves open) and is then handed on to the
//! [`BattleEngine`], which draws everything else from it.
//!
//! - **Guaranteed:** with the same build of this crate, the same format, the same seed and the
//!   same inputs in the same order (registrations, team choices, turn choices), a battle runs the
//!   same — the same requests, views and outcome. A [`BattleEngine::snapshot`] holds the random
//!   state too, so a [`BattleEngine::rewind`] or a clone goes on exactly as the original would;
//!   a snapshot taken under other rules or another dex is refused.
//! - **Not guaranteed:** the same battle across versions of this crate — a rule fixed or a draw
//!   reordered changes what a seed gives, and the random generator itself is an implementation
//!   detail; nor anything about Pokémon Showdown's seeds, whose generator this crate does not
//!   use. A Showdown battle is not replayed from its seed but from its log.
//!
//! # Snapshots as JSON
//!
//! [`SessionSnapshot::to_json`] and [`BattleSnapshot::to_json`] write a snapshot as one line of
//! JSON — compress it as you store it, if you like — and `from_json` reads it back; the same
//! state always writes the same bytes. The JSON is an envelope naming the format, the layout's
//! `version`, the engine that wrote it and the kind of snapshot (`session`, `battle`), around
//! the snapshot itself; the snapshot's types implement no serde trait of their own, so that
//! envelope is the only way in and out. Integers wider than 32 bits (the random state) are
//! decimal strings, for JavaScript's sake.
//!
//! - **Only the same layout version reads back** ([`SnapshotError::UnsupportedVersion`]): a
//!   snapshot is no archive across versions of this crate, as a battle itself is not (above).
//!   Keep the seed and the inputs to keep a battle for longer.
//! - **A snapshot is trusted data.** Malformed JSON, another format, version or kind is an
//!   error, and restoring it checks it belongs to the battle's rules and dex and that the battle
//!   can be in that state; JSON edited into something this engine never wrote is not guarded
//!   against beyond that. Where it is stored out of your hands, sign it.
//! - **A snapshot holds everything:** both sides' teams, the hidden information, the random
//!   state. Never send one to a player; send each player its [`view`](BattleEngine::view).

// The stable API alone, unless the `internals` feature opens the modules (DESIGN.md §47.0 item
// 7, §47.5): a type the stable API reaches but does not name is an error. Code and re-exports
// only the internals reach are neither dead nor unused there (the `internals` build checks both),
// and the names clippy leaves alone on an exported item it leaves alone there too.
#![cfg_attr(
    not(feature = "internals"),
    deny(unnameable_types, private_interfaces, private_bounds),
    allow(
        dead_code,
        unused_imports,
        clippy::enum_variant_names,
        clippy::wrong_self_convention
    )
)]

/// An item of the internals (DESIGN.md §47.0 item 7): `pub` with the `internals` feature (no
/// compatibility promise), the crate's own without it. Called with parentheses, `internal!(…);`:
/// rustfmt formats the items of a parenthesized macro call, not a braced one's, and follows no
/// module declared inside a macro (R-M10-17, `tests/architecture.rs`).
macro_rules! internal {
    ($(#[$meta:meta])* pub $($item:tt)*) => {
        $(#[$meta])*
        #[cfg(feature = "internals")]
        pub $($item)*
        $(#[$meta])*
        #[cfg(not(feature = "internals"))]
        pub(crate) $($item)*
    };
}

// The crate's modules: public with the `internals` feature (no compatibility promise), else the
// crate's own. Written out rather than through `internal!`, so rustfmt follows them.
#[cfg(feature = "internals")]
pub mod data;
#[cfg(not(feature = "internals"))]
pub(crate) mod data;
#[cfg(feature = "internals")]
pub mod dex;
#[cfg(not(feature = "internals"))]
pub(crate) mod dex;
#[cfg(feature = "internals")]
pub mod digest;
#[cfg(not(feature = "internals"))]
pub(crate) mod digest;
#[cfg(feature = "internals")]
pub mod effects;
#[cfg(not(feature = "internals"))]
pub(crate) mod effects;
#[cfg(feature = "internals")]
pub mod engine;
#[cfg(not(feature = "internals"))]
pub(crate) mod engine;
#[cfg(feature = "internals")]
pub mod format;
#[cfg(not(feature = "internals"))]
pub(crate) mod format;
#[cfg(feature = "internals")]
pub mod ids;
#[cfg(not(feature = "internals"))]
pub(crate) mod ids;
#[cfg(feature = "internals")]
pub mod rules;
#[cfg(not(feature = "internals"))]
pub(crate) mod rules;
#[cfg(feature = "internals")]
pub mod session;
#[cfg(not(feature = "internals"))]
pub(crate) mod session;
#[cfg(feature = "internals")]
pub mod spec;
#[cfg(not(feature = "internals"))]
pub(crate) mod spec;
#[cfg(feature = "internals")]
pub mod state;
#[cfg(not(feature = "internals"))]
pub(crate) mod state;
// The stable API's view of a battle (DESIGN.md §47.0 items 8-9, 11).
pub mod view;
// Snapshots as JSON (DESIGN.md §48.2).
mod snapshot;

// The internals' own shortcuts at the crate's root.
#[cfg(feature = "internals")]
pub use rules::{ResolvedRules, RulesResolver};
#[cfg(feature = "internals")]
pub use spec::BattleSpec;

// The stable API (DESIGN.md §47.0 items 7-10, §47.5).
pub use dex::PokemonType;
pub use dex::SetError;
pub use engine::{
    ActionChoice, ActorOptions, BattleEngine, BattleError, BattleInitError, BattleRequest,
    BattleSnapshot, ChoiceError, ChoiceOptions, EngineStep, ForcedReplacement, ForcedSwitchChoice,
    MaxMoveOption, MechanicChoice, MechanicOptions, MoveOption, ReplacementRequirement,
    ReplacementSlot, ReviveChoice, SideChoice, SlotChoice, TraceCategory, TraceFilter, TraceMode,
    Trapped, TurnResult, ZMoveOption,
};
pub use format::{
    DataError, FormatError, FormatLibrary, Problem, ProblemKind, ProblemLocation, SetField, Stats,
    TeamSet,
};
pub use ids::{AbilityId, ItemId, MoveId, MoveSlot, PokemonId, PositionId, SideId, SpeciesId};
pub use rules::RuleError;
pub use rules::{DeclaredTarget, TargetError, TargetPattern};
pub use session::{
    BattleSession, PreviewEntry, SessionError, SessionFormat, SessionPhase, SessionRequest,
    SessionSnapshot, SessionView, SetBuildError, TeamChoice, TeamChoiceError, TeamSlot,
};
pub use snapshot::SnapshotError;
pub use spec::{BattleMode, FormatId, Generation};
pub use state::{
    BattleOutcome, BattlePhase, FieldConditionKind, Gender, NonVolatileStatus, SideConditionKind,
    Stat, TerrainKind, VolatileConditionKind, WeatherKind,
};
