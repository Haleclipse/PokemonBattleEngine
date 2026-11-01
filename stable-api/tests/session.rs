//! A battle through the stable API alone (DESIGN.md §47.5): the pre-battle session, the first
//! request, a fork by clone, a snapshot rewound, and a team the format refuses.

mod common;

use common::{library, next_request, started, teams};
use pokemon_battle_engine::{
    BattleRequest, BattleSession, EngineStep, ProblemKind, SessionError, SessionFormat, SetField,
    SideId, TraceMode,
};

#[test]
fn a_session_starts_a_battle_that_asks_for_turn_choices() {
    let mut engine = started("gen9ou");
    assert!(matches!(
        next_request(&mut engine),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
}

#[test]
fn a_clone_is_a_battle_of_its_own() {
    let mut engine = started("gen9ou");
    let mut fork = engine.clone();
    fork.set_trace_mode(TraceMode::Off);
    assert_eq!(fork.trace_mode(), TraceMode::Off);
    assert_eq!(engine.trace_mode(), TraceMode::Full);
    assert_eq!(next_request(&mut fork), next_request(&mut engine));
}

#[test]
fn a_snapshot_rewinds_the_battle_in_place() {
    let mut engine = started("gen9ou");
    let start = engine.snapshot();
    let first = next_request(&mut engine);
    engine.rewind(&start).expect("a snapshot of this battle");
    assert_eq!(next_request(&mut engine), first);
}

#[test]
fn a_team_the_format_refuses_is_reported() {
    let format = SessionFormat::new(library(), "gen9ou").expect("gen9ou");
    let mut session = BattleSession::new(format, 7);
    let mut sets = teams("gen9ou").remove(0);
    sets[0].moves = vec!["Not A Move".to_string()];
    match session.register(SideId::P1, sets) {
        Err(SessionError::Team { side, problems }) => {
            assert_eq!(side, SideId::P1);
            // The invalid move, where it stands: the first set's first move.
            assert!(
                problems
                    .iter()
                    .any(|problem| problem.kind == ProblemKind::Invalid
                        && problem.location.set == Some(0)
                        && problem.location.field == Some(SetField::Move(0)))
            );
        }
        other => panic!("not refused: {other:?}"),
    }
}
