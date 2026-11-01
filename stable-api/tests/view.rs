//! A battle as an observer sees it, through the stable API alone (DESIGN.md §47.0 items 8, 9
//! and 11-13, M10.4b-c): a player sees their own team in full, and of the other side's hidden
//! things only what was shown — an item, ability or move it names is one the Pokémon has; the
//! omniscient view, everything.

mod common;

use common::{Policy, started};
use pokemon_battle_engine::SideId;
use pokemon_battle_engine::view::{Hp, Observer, Seen};

#[test]
fn a_player_sees_their_own_team_and_none_of_the_others_hidden_things() {
    let mut engine = started("gen9ou");
    let (outcome, _) = Policy(4).play_out(&mut engine, 3);
    assert!(outcome.is_none(), "over before the third turn");
    let view = engine.view(Observer::Side(SideId::P1));
    let own: Vec<_> = view.side(SideId::P1).team().collect();
    assert_eq!(own.len(), 6, "the whole team");
    for pokemon in own {
        assert!(matches!(pokemon.hp(), Hp::Exact { .. }));
        assert_ne!(pokemon.item(), Seen::Unknown);
        let moves = pokemon.moves();
        assert!(moves.complete());
        assert!(moves.revealed().count() > 0);
        assert!(
            moves
                .revealed()
                .all(|m| matches!(m.pp(), Seen::Known((pp, max)) if pp <= max))
        );
    }
    let omniscient = engine.omniscient_view();
    for (_, standing) in view.side(SideId::P2).active() {
        let foe = standing.expect("a foe on the field");
        assert!(matches!(foe.hp(), Hp::Percent(_)));
        let truth = omniscient
            .side(SideId::P2)
            .team()
            .find(|member| member.id() == foe.id())
            .expect("the foe in its team");
        if let Seen::Known(item) = foe.item() {
            assert_eq!(
                truth.item(),
                Seen::Known(item),
                "an item shown is the one held"
            );
        }
        if let Seen::Known(ability) = foe.ability() {
            assert_eq!(
                truth.ability(),
                Seen::Known(ability),
                "an ability shown is its own"
            );
        }
        // Moves seen, never its slots' PP.
        let held: Vec<_> = truth.moves().revealed().map(|m| m.move_id()).collect();
        for seen in foe.moves().revealed() {
            assert!(held.contains(&seen.move_id()), "a move shown is one it has");
            assert_eq!(seen.pp(), Seen::Unknown);
        }
    }
    assert_eq!(omniscient.observer(), None);
    for foe in omniscient.side(SideId::P2).team() {
        assert!(foe.moves().complete());
        assert!(matches!(foe.hp(), Hp::Exact { .. }));
    }
}
