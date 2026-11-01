//! A battle server's loop (DESIGN.md §47.0 items 8-9, 14): the pre-battle session — both teams
//! registered, the open team sheets accepted, each side choosing the four it brings after seeing
//! the other's preview — then turn after turn, each side's client given its own view and its
//! options, its choice checked before the turn runs. A client sees its own team whole and of the
//! other side only what was shown: the preview, the sheets, what the battle disclosed.
//!
//!     cargo run --manifest-path stable-api/Cargo.toml --example server_loop

mod shared;

use pokemon_battle_engine::view::{BattleView, Hp, Observer, Seen};
use pokemon_battle_engine::{
    BattleEngine, BattleRequest, BattleSession, EngineStep, SessionFormat, SessionRequest, SideId,
    TeamChoice, TeamSlot,
};
use shared::{Policy, answer_mid_turn, library, next_request, vgc_team};

/// What a client is shown of the other side: who stands there and what it knows of each.
fn report(engine: &BattleEngine, view: &BattleView<'_>, foe: SideId) -> Vec<String> {
    let names = engine.names();
    let mut lines = Vec::new();
    for (_, standing) in view.side(foe).active() {
        let Some(pokemon) = standing else { continue };
        let species = pokemon
            .species()
            .and_then(|s| names.species(s))
            .unwrap_or("?");
        let hp = match pokemon.hp() {
            Hp::Exact { hp, max } => format!("{hp}/{max}"),
            Hp::Percent(percent) => format!("{percent}%"),
            other => format!("{other:?}"),
        };
        let item = match pokemon.item() {
            Seen::Known(item) => names.item(item).unwrap_or("?").to_string(),
            Seen::KnownNone => "no item".into(),
            Seen::Unknown => "item unknown".into(),
        };
        let moves: Vec<_> = pokemon
            .moves()
            .revealed()
            .map(|known| names.move_(known.move_id()).unwrap_or("?"))
            .collect();
        lines.push(format!("{species} {hp}, {item}, moves {moves:?}"));
    }
    lines
}

fn main() {
    let library = library();
    let format = SessionFormat::new(&library, "gen9vgc2025regi").expect("a format");
    let mut session = BattleSession::new(format, 2026);
    for side in [SideId::P1, SideId::P2] {
        session.register(side, vgc_team()).expect("a legal team");
    }
    // Before the battle: each side sees the other's preview, both accept the open team sheets,
    // each brings four.
    for side in [SideId::P1, SideId::P2] {
        let shown = session.view(side);
        let preview: Vec<_> = shown
            .opponent_preview
            .unwrap_or_default()
            .iter()
            .map(|entry| entry.details.clone())
            .collect();
        println!("{side:?} sees the other side's preview: {preview:?}");
        session
            .accept_open_team_sheets(side)
            .expect("sheets offered");
    }
    let SessionRequest::Choose { .. } = session.request() else {
        panic!("Team Preview's choice");
    };
    session
        .choose(
            SideId::P1,
            TeamChoice {
                order: [0, 1, 2, 3].map(TeamSlot).to_vec(),
            },
        )
        .expect("four of six");
    session
        .choose(
            SideId::P2,
            TeamChoice {
                order: [2, 4, 3, 5].map(TeamSlot).to_vec(),
            },
        )
        .expect("four of six");
    let mut engine = session.engine().expect("the battle").clone();

    let mut clients = [Policy(1), Policy(2)];
    for _ in 0..30 {
        match next_request(&mut engine) {
            EngineStep::Finished(outcome) => {
                println!("over: {outcome:?}");
                return;
            }
            EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) => {
                let view = engine.view(Observer::Spectator);
                println!("— turn {} —", view.turn());
                let mut choices = Vec::new();
                for (side, foe) in [(SideId::P1, SideId::P2), (SideId::P2, SideId::P1)] {
                    // The client's own view: never the other side's hidden things.
                    let view = engine.view(Observer::Side(side));
                    for line in report(&engine, &view, foe) {
                        println!("  {side:?} sees {line}");
                    }
                    let choice = clients[side.index()].turn(&engine, side);
                    engine
                        .validate_choice(side, &choice)
                        .expect("a choice its options allow");
                    choices.push(choice);
                }
                let [p1, p2] = <[_; 2]>::try_from(choices).unwrap();
                engine.submit_turn_choices([p1, p2]).expect("valid choices");
            }
            EngineStep::Awaiting(request) => {
                assert!(answer_mid_turn(&mut engine, &request, &mut clients[0]));
            }
            EngineStep::Progressed { .. } => unreachable!(),
        }
    }
    println!("thirty turns in, still on");
}
