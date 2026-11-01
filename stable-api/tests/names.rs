//! The names of a battle's ids, through the stable API alone (DESIGN.md §47.0 item 15): each
//! member of a registered team is named as its set names it — species, item, ability, moves — and
//! each name reads back to its id; a name the generation does not have is none.

mod common;

use common::{started, teams};
use pokemon_battle_engine::SideId;
use pokemon_battle_engine::view::{Observer, Seen};

/// Showdown `toID`.
fn to_id(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[test]
fn a_team_is_named_as_its_sets_name_it() {
    for format_id in ["gen9ou", "gen4ou"] {
        let engine = started(format_id);
        let names = engine.names();
        let view = engine.view(Observer::Side(SideId::P1));
        let sets = &teams(format_id)[0];
        for (member, set) in view.side(SideId::P1).team().zip(sets) {
            let species = member.species().expect("a species");
            let name = names.species(species).expect("a species name");
            assert_eq!(to_id(name), to_id(&set.species), "{format_id}");
            assert_eq!(names.species_id(name), Some(species));
            if let Seen::Known(item) = member.item() {
                let name = names.item(item).expect("an item name");
                assert_eq!(to_id(name), to_id(&set.item), "{format_id}");
                assert_eq!(names.item_id(name), Some(item));
            }
            let Seen::Known(ability) = member.ability() else {
                panic!("its own side knows its ability");
            };
            let name = names.ability(ability).expect("an ability name");
            assert_eq!(names.ability_id(name), Some(ability));
            let moves: Vec<String> = member
                .moves()
                .revealed()
                .map(|known| to_id(names.move_(known.move_id()).expect("a move name")))
                .collect();
            let expected: Vec<String> = set.moves.iter().map(|m| to_id(m)).collect();
            // Hidden Power's type is the slot's own (`hiddenpowerfire`), not in its name.
            for (named, expected) in moves.iter().zip(&expected) {
                assert!(expected.starts_with(named.as_str()), "{named} {expected}");
            }
            for known in member.moves().revealed() {
                let name = names.move_(known.move_id()).unwrap();
                assert_eq!(names.move_id(name), Some(known.move_id()));
            }
        }
        // A name read as Showdown reads one; none the generation does not have.
        assert_eq!(names.move_id("thunder bolt"), names.move_id("Thunderbolt"));
        assert!(names.move_id("Thunderbolt").is_some());
        assert_eq!(names.species_id("Not A Pokémon"), None);
    }
    // Gen 4 has no Great Tusk.
    assert_eq!(started("gen4ou").names().species_id("Great Tusk"), None);
    assert!(started("gen9ou").names().species_id("Great Tusk").is_some());
}
