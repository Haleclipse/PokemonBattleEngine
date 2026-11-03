//! Snapshots as JSON (DESIGN.md §48.2): an envelope naming the format, the layout's version, the
//! engine that wrote it and the kind of snapshot, around the snapshot's body.
//!
//! ```json
//! {"format":"pokemon-battle-engine-snapshot","version":2,"engine":"0.1.0","kind":"battle","body":{…}}
//! ```
//!
//! Only a snapshot of the same layout version reads back (§48.2.3 question 1): the engine
//! migrates none, as it promises no battle across versions either (crate docs, "Randomness and
//! reproducibility"). The body is the internal types' own serde layout; `u64` values are decimal
//! strings, beyond what a JavaScript number holds exactly. The state has no hashed container, so
//! one state writes one sequence of bytes.
//!
//! A snapshot is trusted data (§48.2.3 question 2): malformed JSON, another format, version or
//! kind is an error, and restoring checks the fingerprints and the battle's invariants; a body
//! edited into something the engine never wrote is not guarded against. It holds both sides'
//! hidden information.

use std::fmt;
use std::sync::Arc;

use serde::de::{DeserializeOwned, Error as _, IgnoredAny};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::engine::{BattleSnapshot, PendingExecution};
use crate::session::{RegisteredTeam, SessionPhase, SessionSnapshot, TeamSlot};
use crate::state::{BattleRng, BattleState};

pub(crate) mod wire;

const FORMAT: &str = "pokemon-battle-engine-snapshot";

/// The layout's version: one more at every change of what a snapshot serializes to. The
/// fixtures under `tests/fixtures/snapshots/` read back byte for byte only under their version,
/// so a change of layout fails them until this moves.
pub(crate) const VERSION: u32 = 2;

const BATTLE: &str = "battle";
const SESSION: &str = "session";

/// Why a JSON text is not a snapshot to read back (`BattleSnapshot::from_json`,
/// `SessionSnapshot::from_json`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotError {
    /// Not JSON, or not a snapshot body of this engine's layout: the parser's message, with
    /// its line and column.
    Json(String),
    /// JSON, but not a snapshot: no `"format": "pokemon-battle-engine-snapshot"`.
    NotASnapshot,
    /// A snapshot of another layout version: only the same version reads back.
    UnsupportedVersion { found: u32, supported: u32 },
    /// A snapshot of the other kind: a session's read as a battle's, or a battle's as a
    /// session's.
    WrongKind {
        expected: &'static str,
        found: String,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(message) => write!(f, "not a snapshot's JSON: {message}"),
            Self::NotASnapshot => write!(f, "not a {FORMAT}"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "a snapshot of layout version {found}; this engine reads version {supported}"
            ),
            Self::WrongKind { expected, found } => {
                write!(
                    f,
                    "a {found} snapshot where a {expected} snapshot was expected"
                )
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

#[derive(Serialize)]
struct Envelope<B> {
    format: &'static str,
    version: u32,
    engine: &'static str,
    kind: &'static str,
    body: B,
}

/// The envelope without its body, read first: its fields decide the error before the body is.
#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
    kind: Option<String>,
    #[allow(dead_code)]
    body: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct Body<B> {
    body: B,
}

fn write<B: Serialize>(kind: &'static str, body: B, pretty: bool) -> String {
    let envelope = Envelope {
        format: FORMAT,
        version: VERSION,
        engine: env!("CARGO_PKG_VERSION"),
        kind,
        body,
    };
    let json = if pretty {
        serde_json::to_string_pretty(&envelope)
    } else {
        serde_json::to_string(&envelope)
    };
    // Only maps with non-string keys or a failing `Serialize` impl can fail, and a snapshot
    // has neither.
    json.expect("a snapshot serializes")
}

fn read<B: DeserializeOwned>(kind: &'static str, json: &str) -> Result<B, SnapshotError> {
    let json_error = |error: serde_json::Error| SnapshotError::Json(error.to_string());
    let header: Header = serde_json::from_str(json).map_err(json_error)?;
    if header.format.as_deref() != Some(FORMAT) {
        return Err(SnapshotError::NotASnapshot);
    }
    let found = header.version.ok_or(SnapshotError::NotASnapshot)?;
    if found != VERSION {
        return Err(SnapshotError::UnsupportedVersion {
            found,
            supported: VERSION,
        });
    }
    let found = header.kind.ok_or(SnapshotError::NotASnapshot)?;
    if found != kind {
        return Err(SnapshotError::WrongKind {
            expected: kind,
            found,
        });
    }
    serde_json::from_str::<Body<B>>(json)
        .map(|body| body.body)
        .map_err(json_error)
}

#[derive(Serialize)]
struct BattleBodyRef<'a> {
    rules_fingerprint: &'a str,
    dex_fingerprint: &'a str,
    state: &'a BattleState,
    pending: &'a PendingExecution,
}

#[derive(Deserialize)]
struct BattleBody {
    rules_fingerprint: String,
    dex_fingerprint: String,
    state: BattleState,
    pending: PendingExecution,
}

impl<'a> From<&'a BattleSnapshot> for BattleBodyRef<'a> {
    fn from(snapshot: &'a BattleSnapshot) -> Self {
        Self {
            rules_fingerprint: &snapshot.rules_fingerprint,
            dex_fingerprint: &snapshot.dex_fingerprint,
            state: &snapshot.state,
            pending: &snapshot.pending,
        }
    }
}

impl From<BattleBody> for BattleSnapshot {
    fn from(body: BattleBody) -> Self {
        Self {
            state: body.state,
            pending: body.pending,
            rules_fingerprint: body.rules_fingerprint,
            dex_fingerprint: body.dex_fingerprint,
        }
    }
}

#[derive(Serialize)]
struct SessionBodyRef<'a> {
    fingerprint: &'a str,
    #[serde(with = "wire")]
    phase: SessionPhase,
    registered: &'a [Option<RegisteredTeam>; 2],
    #[serde(serialize_with = "wire::serialize_ref")]
    choices: &'a [Option<Vec<TeamSlot>>; 2],
    sheets_accepted: [bool; 2],
    rng: &'a BattleRng,
    battle: Option<BattleBodyRef<'a>>,
}

#[derive(Deserialize)]
struct SessionBody {
    fingerprint: String,
    #[serde(with = "wire")]
    phase: SessionPhase,
    registered: [Option<RegisteredTeam>; 2],
    #[serde(with = "wire")]
    choices: [Option<Vec<TeamSlot>>; 2],
    sheets_accepted: [bool; 2],
    rng: BattleRng,
    battle: Option<BattleBody>,
}

/// Snapshots as JSON (DESIGN.md §48.2). The JSON holds both sides' hidden information: a
/// server keeps it to itself and sends players their views.
impl BattleSnapshot {
    /// The snapshot as one line of JSON, for storage.
    pub fn to_json(&self) -> String {
        write(BATTLE, BattleBodyRef::from(self), false)
    }

    /// The snapshot as indented JSON, for reading and diffing.
    pub fn to_json_pretty(&self) -> String {
        write(BATTLE, BattleBodyRef::from(self), true)
    }

    /// A snapshot back from `to_json` / `to_json_pretty` of an engine of the same layout
    /// version. Restoring it (`rewind`, or a session's `restore`) checks it belongs to the
    /// battle's rules and dex. Trusted data: a text the engine never wrote is not guarded
    /// against beyond being read as JSON of the snapshot's shape.
    pub fn from_json(json: &str) -> Result<Self, SnapshotError> {
        read::<BattleBody>(BATTLE, json).map(Self::from)
    }
}

/// Snapshots as JSON (DESIGN.md §48.2). The JSON holds both sides' teams and the battle's
/// hidden information: a server keeps it to itself and sends players their views.
impl SessionSnapshot {
    /// The snapshot as one line of JSON, for storage.
    pub fn to_json(&self) -> String {
        write(SESSION, self.body(), false)
    }

    /// The snapshot as indented JSON, for reading and diffing.
    pub fn to_json_pretty(&self) -> String {
        write(SESSION, self.body(), true)
    }

    /// A snapshot back from `to_json` / `to_json_pretty` of an engine of the same layout
    /// version. `BattleSession::restore` checks it belongs to the format's registration.
    /// Trusted data: a text the engine never wrote is not guarded against beyond being read
    /// as JSON of the snapshot's shape.
    pub fn from_json(json: &str) -> Result<Self, SnapshotError> {
        let body: SessionBody = read(SESSION, json)?;
        Ok(Self {
            fingerprint: body.fingerprint,
            phase: body.phase,
            registered: body.registered,
            choices: body.choices,
            sheets_accepted: body.sheets_accepted,
            rng: body.rng,
            battle: body.battle.map(BattleSnapshot::from),
        })
    }

    fn body(&self) -> SessionBodyRef<'_> {
        SessionBodyRef {
            fingerprint: &self.fingerprint,
            phase: self.phase,
            registered: &self.registered,
            choices: &self.choices,
            sheets_accepted: self.sheets_accepted,
            rng: &self.rng,
            battle: self.battle.as_ref().map(BattleBodyRef::from),
        }
    }
}

/// A `u64` as a decimal string (`#[serde(with = "crate::snapshot::u64_string")]`).
pub(crate) mod u64_string {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(value)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

/// `u64`s as decimal strings.
pub(crate) mod u64_strings {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(
        values: &[u64],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(values.iter().map(u64::to_string))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u64>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|value| value.parse().map_err(D::Error::custom))
            .collect()
    }
}

/// An `Arc<[T]>` as its elements (serde's own needs its `rc` feature).
pub(crate) mod arc_slice {
    use super::*;

    pub(crate) fn serialize<S: Serializer, T: Serialize>(
        values: &Arc<[T]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(values.iter())
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Arc<[T]>, D::Error> {
        Vec::<T>::deserialize(deserializer).map(Arc::from)
    }
}

/// An `Option<Arc<T>>` as an `Option<T>`.
pub(crate) mod option_arc {
    use super::*;

    pub(crate) fn serialize<S: Serializer, T: Serialize>(
        value: &Option<Arc<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_deref().serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Option<Arc<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(|value| value.map(Arc::new))
    }
}
