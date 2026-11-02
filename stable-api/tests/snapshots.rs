//! Snapshots as JSON through the stable API alone (DESIGN.md §48.2): a session written, read back
//! and restored goes on as it was; a battle's snapshot through its JSON rewinds the battle; JSON
//! that is no snapshot of this engine is refused.

mod common;

use common::{library, next_request, started, teams};
use pokemon_battle_engine::{
    BattleSession, BattleSnapshot, SessionFormat, SessionRequest, SessionSnapshot, SideId,
    SnapshotError, TeamChoice,
};

#[test]
fn a_session_written_as_json_restores_and_goes_on() {
    let format = SessionFormat::new(library(), "gen9ou").expect("gen9ou");
    let mut session = BattleSession::new(format, 31);
    for (side, sets) in [SideId::P1, SideId::P2].into_iter().zip(teams("gen9ou")) {
        session
            .register(side, sets)
            .expect("a team Showdown admits");
    }
    if let SessionRequest::Choose { sides, .. } = session.request() {
        for side in sides {
            session
                .choose(side, TeamChoice::default())
                .expect("the team's own order");
        }
    }
    let json = session.snapshot().to_json();
    let snapshot = SessionSnapshot::from_json(&json).expect("its own JSON");
    let format = SessionFormat::new(library(), "gen9ou").expect("gen9ou");
    let restored = BattleSession::restore(format, snapshot).expect("the same registration");
    assert_eq!(restored.snapshot().to_json(), json);
    let mut original = session.engine().expect("the battle").clone();
    let mut again = restored.engine().expect("the battle").clone();
    assert_eq!(next_request(&mut again), next_request(&mut original));
}

#[test]
fn a_battle_snapshot_through_json_rewinds_the_battle() {
    let mut engine = started("gen9ou");
    let start = engine.snapshot().to_json_pretty();
    let first = next_request(&mut engine);
    let snapshot = BattleSnapshot::from_json(&start).expect("its own JSON");
    engine.rewind(&snapshot).expect("a snapshot of this battle");
    assert_eq!(next_request(&mut engine), first);
}

#[test]
fn json_that_is_no_snapshot_is_refused() {
    assert_eq!(
        BattleSnapshot::from_json(r#"{"species": "Pikachu"}"#),
        Err(SnapshotError::NotASnapshot)
    );
    assert!(matches!(
        SessionSnapshot::from_json("not json"),
        Err(SnapshotError::Json(_))
    ));
}
