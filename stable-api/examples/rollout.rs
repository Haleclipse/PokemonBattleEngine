//! An AI's rollouts (DESIGN.md §47.0 item 10): at the battle's first turn, every action P1 may
//! take is tried in random playouts to the end — each a clone of the battle, its trace off — and
//! scored by the playouts P1 wins. A flat Monte Carlo search, to show the parts: `choice_options`
//! for what may be chosen, `Clone` to fork the battle, `TraceMode::Off` for speed, `names` to
//! show the result.
//!
//!     cargo run --release --manifest-path stable-api/Cargo.toml --example rollout [playouts]

mod shared;

use pokemon_battle_engine::{
    ActionChoice, BattleOutcome, BattleRequest, EngineStep, SideChoice, SideId, SlotChoice,
    TraceMode,
};
use shared::{Policy, library, next_request, ou_team, play_out, started};

fn main() {
    let playouts: u32 = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(40);
    let library = library();
    let mut root = started(&library, "gen9ou", ou_team(), 1);
    let EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) = next_request(&mut root) else {
        panic!("the first turn's choices");
    };
    // P1's actions for its one Pokémon on the field: each move (at the first target it accepts:
    // a singles battle has one foe), each switch.
    let options = root.choice_options(SideId::P1).expect("P1's options");
    let actor = &options.actors[0];
    let mut actions: Vec<ActionChoice> = actor
        .moves
        .iter()
        .filter_map(|option| {
            option.targets.first().map(|target| ActionChoice::Move {
                slot: option.slot,
                target: *target,
                mechanic: None,
            })
        })
        .collect();
    actions.extend(
        actor
            .switches
            .iter()
            .map(|pokemon| ActionChoice::Switch { pokemon: *pokemon }),
    );

    let names = root.names();
    let view = root.omniscient_view();
    let p1 = view.side(SideId::P1);
    let label = |action: &ActionChoice| -> String {
        match action {
            ActionChoice::Move { slot, .. } => {
                let lead = p1.active().next().and_then(|(_, p)| p).expect("P1's lead");
                let move_id = lead
                    .moves()
                    .revealed()
                    .nth(slot.index())
                    .expect("the slot's move")
                    .move_id();
                format!("move {}", names.move_(move_id).unwrap_or("?"))
            }
            ActionChoice::Switch { pokemon } => {
                let member = p1.team().find(|p| p.id() == *pokemon).expect("a member");
                format!(
                    "switch to {}",
                    names.species(member.species().unwrap()).unwrap_or("?")
                )
            }
            other => format!("{other:?}"),
        }
    };

    println!("{playouts} playouts per action, P1's first turn of a Gen 9 OU battle:");
    for (index, action) in actions.iter().enumerate() {
        let mut wins = 0;
        for playout in 0..playouts {
            // A fork of the battle: the root stays as it is.
            let mut battle = root.clone();
            battle.set_trace_mode(TraceMode::Off);
            let mut policy = Policy(1000 * index as u64 + u64::from(playout));
            let p2 = policy.turn(&battle, SideId::P2);
            let p1 = SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: actor.actor,
                    action: action.clone(),
                }],
            };
            battle.submit_turn_choices([p1, p2]).expect("valid choices");
            if play_out(&mut battle, &mut policy, 500) == Some(BattleOutcome::Winner(SideId::P1)) {
                wins += 1;
            }
        }
        println!("  {:<32} {:>3} / {playouts} won", label(action), wins);
    }
}
