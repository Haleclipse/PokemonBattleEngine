//! Snapshots as JSON (DESIGN.md §48.2): every format's battle reads back from its JSON equal,
//! writes the same bytes again and plays on as the original does; a session reads back before
//! and during its battle; what is not a snapshot of this engine's layout and kind is refused;
//! and the layout fixtures under `tests/fixtures/snapshots/` hold the layout still.
//!
//! The fixtures: `v<N>/` holds snapshots of layout version N. The newest directory's must read
//! back and write the same bytes; an older one's must be refused as another version. A change
//! of layout fails the newest until the version moves (`src/snapshot.rs`, `VERSION`) and
//! `SNAPSHOT_FIXTURES=write cargo test --test snapshot_json` writes the new version's. They
//! catch a change to the types they contain: an enum variant none of them holds is not.

#[path = "support/policy.rs"]
mod policy;

use std::path::{Path, PathBuf};

use pokemon_battle_engine::engine::TraceMode;
use pokemon_battle_engine::ids::SideId;
use pokemon_battle_engine::session::{BattleSession, SessionFormat, SessionRequest, TeamChoice};
use pokemon_battle_engine::{
    BattleEngine, BattleInitError, BattleSnapshot, SessionSnapshot, SnapshotError,
};
use policy::{Policy, answer, built, library, play_out, start_battle, team_set};
use serde_json::Value;

const FORMAT: &str = "pokemon-battle-engine-snapshot";

fn format_ids() -> Vec<String> {
    built()["formats"]
        .as_object()
        .expect("built.json's formats")
        .keys()
        .cloned()
        .collect()
}

/// `format_id`'s battle after `turns` turns of the policy seeded `seed` (or its end).
fn battle(format_id: &str, seed: u64, turns: u32) -> (BattleEngine, Policy) {
    let (rules, dex, start) = start_battle(format_id);
    let mut engine = BattleEngine::restore_with_trace_mode(rules, dex, start, TraceMode::Off)
        .expect("the start restores");
    let mut policy = Policy(seed);
    for _ in 0..turns {
        if answer(&mut engine, &mut policy).is_none() {
            break;
        }
    }
    (engine, policy)
}

/// `format_id`'s session with both teams registered (the first two of `built.json`), its
/// battle started if `start`.
fn session(format_id: &str, start: bool) -> BattleSession<'static> {
    let format = SessionFormat::new(library(), format_id).expect("a target format");
    let teams = built()["formats"][format_id]["teams"]
        .as_array()
        .expect("the format's teams");
    let mut session = BattleSession::new(format, 11);
    for (side, team) in [SideId::P1, SideId::P2].into_iter().zip(teams) {
        let sets = team["sets"]
            .as_array()
            .expect("a team's sets")
            .iter()
            .map(team_set)
            .collect();
        session
            .register(side, sets)
            .expect("a team Showdown admits");
    }
    if start && let SessionRequest::Choose { sides, .. } = session.request() {
        for side in sides {
            session
                .choose(side, TeamChoice::default())
                .expect("the team's own order");
        }
    }
    session
}

/// Where two snapshots' debug forms first differ.
fn first_difference(a: &BattleSnapshot, b: &BattleSnapshot) -> String {
    let (a, b) = (format!("{a:#?}"), format!("{b:#?}"));
    let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let at = a
        .iter()
        .zip(&b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let context = at.saturating_sub(12);
    format!(
        "{}\n  - {}\n  + {}",
        a[context..at].join("\n"),
        a.get(at).unwrap_or(&"(end)"),
        b.get(at).unwrap_or(&"(end)")
    )
}

fn round_trips(snapshot: &BattleSnapshot) {
    let json = snapshot.to_json();
    let back = BattleSnapshot::from_json(&json).expect("its own JSON reads back");
    assert!(
        back == *snapshot,
        "the snapshot read back differs:\n{}",
        first_difference(snapshot, &back)
    );
    assert_eq!(
        back.to_json(),
        json,
        "the snapshot read back writes other bytes"
    );
    let pretty = snapshot.to_json_pretty();
    let back = BattleSnapshot::from_json(&pretty).expect("its own pretty JSON reads back");
    assert!(
        back == *snapshot,
        "the snapshot read back from pretty JSON differs:\n{}",
        first_difference(snapshot, &back)
    );
    assert_eq!(back.to_json_pretty(), pretty);
    assert!(
        !json.contains('\n') && pretty.contains('\n'),
        "storage on one line, debugging indented"
    );
}

#[test]
fn every_format_round_trips_and_plays_on_from_its_json() {
    for format_id in format_ids() {
        let (rules, dex, start) = start_battle(&format_id);
        round_trips(&start);
        let (mut engine, mut policy) = battle(&format_id, 2026, 10);
        let snapshot = engine.snapshot();
        round_trips(&snapshot);

        // The battle read back plays on as the original does.
        let back = BattleSnapshot::from_json(&snapshot.to_json()).expect("reads back");
        let mut restored = BattleEngine::restore_with_trace_mode(rules, dex, back, TraceMode::Off)
            .expect("the battle's own snapshot restores");
        let mut twin = Policy(policy.0);
        let turns = play_out(&mut engine, &mut policy);
        assert_eq!(play_out(&mut restored, &mut twin), turns, "{format_id}");
        assert!(
            engine.state() == restored.state(),
            "{format_id}: the battle restored from JSON ends elsewhere"
        );
        round_trips(&engine.snapshot());
    }
}

#[test]
fn one_state_writes_one_sequence_of_bytes() {
    let a = battle("gen9ou", 5, 8).0.snapshot().to_json();
    let b = battle("gen9ou", 5, 8).0.snapshot().to_json();
    assert_eq!(a, b);
    assert_ne!(a, battle("gen9ou", 6, 8).0.snapshot().to_json());
}

#[test]
fn a_session_round_trips_before_and_during_its_battle() {
    for start in [false, true] {
        let session = session("gen9vgc2025regi", start);
        let snapshot = session.snapshot();
        let json = snapshot.to_json();
        let back = SessionSnapshot::from_json(&json).expect("its own JSON reads back");
        assert!(back == snapshot);
        assert_eq!(back.to_json(), json);
        let format = SessionFormat::new(library(), "gen9vgc2025regi").expect("the format");
        let restored = BattleSession::restore(format, back).expect("the same registration");
        assert_eq!(restored.snapshot().to_json(), json);
        assert_eq!(restored.engine().is_some(), start);
    }
}

#[test]
fn the_envelope_names_the_format_version_engine_and_kind() {
    let json = battle("gen4ou", 3, 4).0.snapshot().to_json();
    let prefix = format!(
        "{{\"format\":\"{FORMAT}\",\"version\":{},\"engine\":\"{}\",\"kind\":\"battle\",\"body\":{{",
        version(&json),
        env!("CARGO_PKG_VERSION")
    );
    assert!(json.starts_with(&prefix), "{}", &json[..120]);
    let value: Value = serde_json::from_str(&json).expect("JSON");
    // u64s are decimal strings: a JavaScript number holds 53 bits exactly.
    let rng = &value["body"]["state"]["rng"];
    assert!(rng["state"].is_string(), "{rng}");
    let session = session("gen9ou", false).snapshot().to_json();
    let value: Value = serde_json::from_str(&session).expect("JSON");
    assert_eq!(value["kind"], "session");
    assert!(value["body"]["rng"]["state"].is_string());
}

fn version(json: &str) -> u64 {
    let value: Value = serde_json::from_str(json).expect("JSON");
    value["version"].as_u64().expect("a version")
}

#[test]
fn what_is_not_a_snapshot_of_this_layout_and_kind_is_refused() {
    let (mut engine, _) = battle("gen9ou", 9, 3);
    let json = engine.snapshot().to_json();
    let current = version(&json) as u32;

    assert!(matches!(
        BattleSnapshot::from_json("not json"),
        Err(SnapshotError::Json(_))
    ));
    assert_eq!(
        BattleSnapshot::from_json(r#"{"name":"a team"}"#),
        Err(SnapshotError::NotASnapshot)
    );
    let other = json.replacen(
        &format!("\"version\":{current}"),
        &format!("\"version\":{}", current + 1),
        1,
    );
    assert_eq!(
        BattleSnapshot::from_json(&other),
        Err(SnapshotError::UnsupportedVersion {
            found: current + 1,
            supported: current
        })
    );
    let session = session("gen9ou", true).snapshot().to_json();
    assert_eq!(
        BattleSnapshot::from_json(&session),
        Err(SnapshotError::WrongKind {
            expected: "battle",
            found: "session".to_owned()
        })
    );
    assert!(matches!(
        SessionSnapshot::from_json(&json),
        Err(SnapshotError::WrongKind {
            expected: "session",
            ..
        })
    ));
    // A body without one of its fields.
    let truncated = json.replacen("\"turn\":", "\"turns\":", 1);
    assert!(matches!(
        BattleSnapshot::from_json(&truncated),
        Err(SnapshotError::Json(_))
    ));

    // Read back, a snapshot still belongs to its battle: another format's, or one whose
    // fingerprint was edited, is refused as restoring refuses it.
    let gen4 = battle("gen4ou", 9, 3).0.snapshot().to_json();
    let gen4 = BattleSnapshot::from_json(&gen4).expect("reads back");
    assert!(engine.rewind(&gen4).is_err());
    let mut value: Value = serde_json::from_str(&json).expect("JSON");
    value["body"]["rules_fingerprint"] = Value::from("rules:edited");
    let edited = BattleSnapshot::from_json(&value.to_string()).expect("reads back");
    assert_eq!(
        engine.rewind(&edited),
        Err(BattleInitError::SnapshotRulesMismatch)
    );
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/snapshots")
}

/// The layout fixtures: battles mid-way across the generations and their gimmicks, singles and
/// doubles, and sessions before and during their battle.
fn fixtures() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (format_id, turns) in [
        ("gen1ou", 12),
        ("gen4ou", 12),
        ("gen7ou", 8),
        ("gen7vgc2019", 6),
        ("gen8vgc2022", 4),
        ("gen9ou", 10),
        ("gen9vgc2025regi", 5),
    ] {
        let json = battle(format_id, 48, turns).0.snapshot().to_json_pretty();
        out.push((format!("{format_id}.battle.json"), json));
    }
    out.push((
        "gen9vgc2025regi-preview.session.json".to_owned(),
        session("gen9vgc2025regi", false)
            .snapshot()
            .to_json_pretty(),
    ));
    out.push((
        "gen6ou-started.session.json".to_owned(),
        session("gen6ou", true).snapshot().to_json_pretty(),
    ));
    out
}

fn read_back(name: &str, text: &str) -> Result<String, SnapshotError> {
    if name.ends_with(".session.json") {
        SessionSnapshot::from_json(text).map(|snapshot| snapshot.to_json_pretty())
    } else {
        BattleSnapshot::from_json(text).map(|snapshot| snapshot.to_json_pretty())
    }
}

#[test]
fn the_layout_fixtures_hold_the_layout() {
    let fixtures = fixtures();
    let current = version(&fixtures[0].1);
    let dir = fixtures_dir().join(format!("v{current}"));
    if std::env::var("SNAPSHOT_FIXTURES").as_deref() == Ok("write") {
        std::fs::create_dir_all(&dir).expect("the fixtures' directory");
        for (name, json) in &fixtures {
            std::fs::write(dir.join(name), json).expect("a fixture");
        }
    }
    let mut versions: Vec<u64> = std::fs::read_dir(fixtures_dir())
        .expect("tests/fixtures/snapshots")
        .map(|entry| entry.expect("an entry").file_name())
        .filter_map(|name| name.to_str()?.strip_prefix('v')?.parse().ok())
        .collect();
    versions.sort_unstable();
    assert_eq!(
        versions.last(),
        Some(&current),
        "the newest fixtures are not of the layout version the engine writes: write them \
         (SNAPSHOT_FIXTURES=write)"
    );
    for version in versions {
        let dir = fixtures_dir().join(format!("v{version}"));
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .expect("a version's fixtures")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .into_string()
                    .expect("UTF-8")
            })
            .collect();
        names.sort();
        assert!(!names.is_empty(), "{}: no fixtures", dir.display());
        for name in names {
            let text = std::fs::read_to_string(dir.join(&name)).expect("a fixture");
            let read = read_back(&name, &text);
            if version == current {
                let written = read.unwrap_or_else(|error| {
                    panic!(
                        "v{version}/{name}: {error} — the layout changed: move the version \
                         (src/snapshot.rs) and write the new fixtures (SNAPSHOT_FIXTURES=write)"
                    )
                });
                assert!(
                    written == text,
                    "v{version}/{name} reads back into other bytes — the layout changed: move \
                     the version (src/snapshot.rs) and write the new fixtures \
                     (SNAPSHOT_FIXTURES=write)"
                );
            } else {
                assert_eq!(
                    read,
                    Err(SnapshotError::UnsupportedVersion {
                        found: version as u32,
                        supported: current as u32
                    }),
                    "v{version}/{name}"
                );
            }
        }
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a source directory") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

/// serde writes `Some(None)` as `null`, like `None`: an `Option<Option<_>>` field of a type a
/// snapshot serializes goes through `crate::snapshot::wire::option_option`, or reads back as
/// `None`.
#[test]
fn a_serialized_option_of_an_option_keeps_its_some_none() {
    const ATTRIBUTE: &str = "#[serde(with = \"crate::snapshot::wire::option_option\")]";
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut missing = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("a source file");
        let lines: Vec<&str> = text.lines().collect();
        // Whether the type being read derives `Serialize`: its last `#[derive(…)]` before it.
        let (mut derive_serialize, mut in_serialized) = (false, false);
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("#[derive(") {
                derive_serialize = trimmed.contains("Serialize");
            } else if trimmed.contains("struct ") || trimmed.contains("enum ") {
                in_serialized = derive_serialize && !trimmed.starts_with("//");
                derive_serialize = false;
            } else if in_serialized
                && trimmed.contains(": Option<Option<")
                && !trimmed.starts_with("//")
                && !lines[index - 1].trim().eq(ATTRIBUTE)
            {
                missing.push(format!("{}:{}", file.display(), index + 1));
            }
        }
    }
    assert!(missing.is_empty(), "without {ATTRIBUTE}: {missing:#?}");
}

/// Whether a type implements serde's traits, without naming them in a bound that fails: the
/// inherent method applies where the bound holds, the trait's everywhere else.
struct Probe<T>(std::marker::PhantomData<T>);

trait NoSerde {
    fn has_serde(&self) -> bool {
        false
    }
}

impl<T> NoSerde for Probe<T> {}

impl<T: serde::Serialize + for<'de> serde::Deserialize<'de>> Probe<T> {
    fn has_serde(&self) -> bool {
        true
    }
}

macro_rules! serde_of {
    ($($ty:ty),* $(,)?) => {
        vec![$((stringify!($ty), Probe::<$ty>(std::marker::PhantomData).has_serde())),*]
    };
}

/// The stable API's value types and its snapshots implement no serde trait (DESIGN.md §48.2.3a):
/// serde stays out of the stable API's promise, and a snapshot is written and read through its
/// envelope alone. A snapshot reaches them through `crate::snapshot::wire`.
#[test]
fn the_stable_api_implements_no_serde_trait() {
    use pokemon_battle_engine::*;
    // The probe tells one that has them.
    assert_eq!(
        serde_of!(serde_json::Value),
        vec![("serde_json::Value", true)]
    );
    let with: Vec<&str> = serde_of!(
        AbilityId,
        ActionChoice,
        BattleOutcome,
        BattlePhase,
        DeclaredTarget,
        FieldConditionKind,
        Gender,
        ItemId,
        MechanicChoice,
        MoveId,
        MoveSlot,
        NonVolatileStatus,
        PokemonId,
        PokemonType,
        PositionId,
        SessionPhase,
        SideConditionKind,
        SideId,
        SpeciesId,
        Stat,
        Stats,
        TargetPattern,
        TeamSet,
        TeamSlot,
        TerrainKind,
        VolatileConditionKind,
        WeatherKind,
        BattleSnapshot,
        SessionSnapshot,
    )
    .into_iter()
    .filter(|(_, has)| *has)
    .map(|(name, _)| name)
    .collect();
    assert!(with.is_empty(), "serde on the stable API: {with:?}");
}
