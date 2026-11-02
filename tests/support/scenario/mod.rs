//! Neutral scenario compiler and the oracle-guided replay driver (TESTING.md §4–6, §10).
//!
//! A scenario names teams and choices by name; the compiler resolves them through the dex and
//! the stat formula. Replay drives the Rust engine turn by turn against the Showdown adapter's
//! output: Showdown's per-event log fixes the semantic random decisions (miss / crit /
//! secondary), and the damage roll is recovered by trying all sixteen rolls on a snapshot and
//! keeping the one whose committed HP equals Showdown's. Frames are compared at every request
//! boundary; the first divergence is reported with the surrounding facts.

mod bundle;
mod log;
mod model;
mod plan;
mod replay;
mod search;
mod tape_template;
mod tapes;

pub use bundle::*;
pub use log::*;
pub use model::*;
pub use plan::*;
pub use replay::*;
pub use search::*;
pub use tape_template::*;
pub use tapes::*;
