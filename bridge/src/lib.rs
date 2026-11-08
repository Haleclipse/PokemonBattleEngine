//! Local adapter over the stable battle API, plus the format catalog the UI needs.
//! Damage, legality and turn flow stay in `pokemon-battle-engine`.

mod ai;
mod battle;
mod catalog;
mod http;
mod log;
mod presets;
mod teams;

pub use http::serve;

pub use battle::{ActionIn, BattleRoom};
pub use catalog::Host;
pub use presets::preset_team;
pub use teams::{SetInput, TeamCheck, export_team, import_team, validate_sets};

use std::path::PathBuf;
use std::sync::OnceLock;

use pokemon_battle_engine::FormatLibrary;

pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../data")
}

pub fn library() -> &'static FormatLibrary {
    static LIB: OnceLock<FormatLibrary> = OnceLock::new();
    LIB.get_or_init(|| {
        FormatLibrary::open(data_dir()).unwrap_or_else(|error| panic!("format data: {error}"))
    })
}

pub fn host() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(|| Host::load(library()))
}
