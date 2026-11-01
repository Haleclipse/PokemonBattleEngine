//! Rules research (DESIGN.md §47.0 items 9 and 10): a counterfactual. At a turn of a battle, each
//! move P1's lead may use is tried against the same choice of P2's, the battle rewound to the
//! turn's start each time (`snapshot` / `rewind`: the random numbers too, so every try meets the
//! same rolls), and what each did is read from the omniscient view — every hidden thing included,
//! for trusted hosts and research alone.
//!
//!     cargo run --manifest-path stable-api/Cargo.toml --example research

mod shared;

use pokemon_battle_engine::view::{Hp, Seen};
use pokemon_battle_engine::{
    ActionChoice, BattleRequest, EngineStep, SideChoice, SideId, SlotChoice,
};
use shared::{Policy, answer_mid_turn, library, next_request, ou_team, play_out, started};

fn main() {
    let library = library();
    let mut engine = started(&library, "gen9ou", ou_team(), 3);
    // Three turns in, the position to study.
    let mut policy = Policy(7);
    assert!(play_out(&mut engine, &mut policy, 3).is_none(), "still on");
    let EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) = next_request(&mut engine) else {
        panic!("a turn's choices");
    };
    let names = engine.names();
    let view = engine.omniscient_view();
    let name_of = |side: SideId| {
        view.side(side)
            .active()
            .find_map(|(_, standing)| standing)
            .and_then(|pokemon| pokemon.species())
            .and_then(|species| names.species(species))
            .unwrap_or("?")
            .to_string()
    };
    println!(
        "turn {}: P1's {} against P2's {}",
        view.turn(),
        name_of(SideId::P1),
        name_of(SideId::P2)
    );

    let p2 = policy.turn(&engine, SideId::P2);
    let options = engine.choice_options(SideId::P1).expect("P1's options");
    let actor = &options.actors[0];
    let start = engine.snapshot();
    for option in &actor.moves {
        let Some(target) = option.targets.first() else {
            continue;
        };
        let p1 = SideChoice {
            side: SideId::P1,
            actions: vec![SlotChoice {
                actor: actor.actor,
                action: ActionChoice::Move {
                    slot: option.slot,
                    target: *target,
                    mechanic: None,
                },
            }],
        };
        engine
            .submit_turn_choices([p1, p2.clone()])
            .expect("valid choices");
        // The turn to its end (and a replacement, should one be asked for).
        loop {
            match next_request(&mut engine) {
                EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
                | EngineStep::Finished(_) => {
                    break;
                }
                EngineStep::Awaiting(request) => {
                    answer_mid_turn(&mut engine, &request, &mut Policy(0));
                }
                EngineStep::Progressed { .. } => unreachable!(),
            }
        }
        // Each lead after the turn: its HP and item, as they are.
        let after = engine.omniscient_view();
        let lead = |side: SideId| -> String {
            let Some(pokemon) = after.side(side).active().find_map(|(_, standing)| standing) else {
                return "none standing".into();
            };
            let hp = match pokemon.hp() {
                Hp::Exact { hp, max } => format!("{hp}/{max}"),
                other => format!("{other:?}"),
            };
            let item = match pokemon.item() {
                Seen::Known(item) => names.item(item).unwrap_or("?"),
                _ => "no item",
            };
            let species = pokemon
                .species()
                .and_then(|s| names.species(s))
                .unwrap_or("?");
            format!("{species} {hp} {item}")
        };
        let used = after
            .side(SideId::P1)
            .team()
            .find(|pokemon| pokemon.id() == actor.actor)
            .and_then(|pokemon| pokemon.moves().revealed().nth(option.slot.index()))
            .map(|known| known.move_id());
        println!(
            "  {:<16} P1: {:<34} P2: {}",
            used.and_then(|id| names.move_(id)).unwrap_or("?"),
            lead(SideId::P1),
            lead(SideId::P2)
        );
        // Back to the turn's start, its random numbers included.
        engine.rewind(&start).expect("a snapshot of this battle");
    }
}
