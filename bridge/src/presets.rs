//! Teams the engine's own examples already accept. They are registration data,
//! not a second damage model.

use pokemon_battle_engine::{Stats, TeamSet};

fn set(
    species: &str,
    item: &str,
    ability: &str,
    moves: [&str; 4],
    nature: &str,
    evs: [i32; 6],
    tera_type: &str,
) -> TeamSet {
    let [hp, atk, def, spa, spd, spe] = evs;
    TeamSet {
        species: species.into(),
        item: item.into(),
        ability: ability.into(),
        moves: moves.into_iter().map(str::to_string).collect(),
        nature: nature.into(),
        evs: Some(Stats {
            hp,
            atk,
            def,
            spa,
            spd,
            spe,
        }),
        tera_type: tera_type.into(),
        ..TeamSet::default()
    }
}

/// Gen 9 OU sample from the stable-api examples.
pub fn ou_team() -> Vec<TeamSet> {
    vec![
        set(
            "Great Tusk",
            "Booster Energy",
            "Protosynthesis",
            ["Headlong Rush", "Ice Spinner", "Knock Off", "Rapid Spin"],
            "Jolly",
            [0, 252, 4, 0, 0, 252],
            "Ground",
        ),
        set(
            "Gholdengo",
            "Choice Scarf",
            "Good as Gold",
            ["Make It Rain", "Shadow Ball", "Trick", "Focus Blast"],
            "Timid",
            [0, 0, 0, 252, 4, 252],
            "Steel",
        ),
        set(
            "Dragapult",
            "Choice Specs",
            "Infiltrator",
            ["Draco Meteor", "Shadow Ball", "Flamethrower", "U-turn"],
            "Timid",
            [0, 0, 0, 252, 4, 252],
            "Dragon",
        ),
        set(
            "Garganacl",
            "Leftovers",
            "Purifying Salt",
            ["Salt Cure", "Recover", "Protect", "Stealth Rock"],
            "Careful",
            [252, 0, 4, 0, 252, 0],
            "Water",
        ),
        set(
            "Corviknight",
            "Leftovers",
            "Pressure",
            ["Brave Bird", "Roost", "U-turn", "Defog"],
            "Impish",
            [252, 0, 168, 0, 88, 0],
            "Dragon",
        ),
        set(
            "Clefable",
            "Leftovers",
            "Magic Guard",
            ["Moonblast", "Calm Mind", "Moonlight", "Flamethrower"],
            "Bold",
            [252, 0, 252, 0, 4, 0],
            "Steel",
        ),
    ]
}

/// Same legal sets, lead reversed, so a mirror is not move-for-move identical.
pub fn ou_team_mirror() -> Vec<TeamSet> {
    let mut team = ou_team();
    team.reverse();
    team
}

/// Gen 9 VGC Regulation I sample from the stable-api examples.
pub fn vgc_team() -> Vec<TeamSet> {
    vec![
        set(
            "Incineroar",
            "Safety Goggles",
            "Intimidate",
            ["Fake Out", "Flare Blitz", "Knock Off", "Parting Shot"],
            "Careful",
            [252, 4, 0, 0, 252, 0],
            "Ghost",
        ),
        set(
            "Rillaboom",
            "Assault Vest",
            "Grassy Surge",
            ["Fake Out", "Grassy Glide", "Wood Hammer", "U-turn"],
            "Adamant",
            [252, 252, 0, 0, 4, 0],
            "Fire",
        ),
        set(
            "Urshifu-Rapid-Strike",
            "Choice Scarf",
            "Unseen Fist",
            ["Surging Strikes", "Close Combat", "Aqua Jet", "U-turn"],
            "Jolly",
            [0, 252, 4, 0, 0, 252],
            "Water",
        ),
        set(
            "Amoonguss",
            "Rocky Helmet",
            "Regenerator",
            ["Spore", "Rage Powder", "Pollen Puff", "Protect"],
            "Relaxed",
            [252, 0, 252, 0, 4, 0],
            "Water",
        ),
        set(
            "Flutter Mane",
            "Booster Energy",
            "Protosynthesis",
            ["Moonblast", "Shadow Ball", "Icy Wind", "Protect"],
            "Timid",
            [4, 0, 0, 252, 0, 252],
            "Fairy",
        ),
        set(
            "Farigiraf",
            "Electric Seed",
            "Armor Tail",
            ["Trick Room", "Psychic", "Hyper Voice", "Protect"],
            "Quiet",
            [252, 0, 4, 252, 0, 0],
            "Water",
        ),
    ]
}

/// Leads are the two supplied battle models. The other five are the OU sample, which Ubers allows.
pub fn showcase_player() -> Vec<TeamSet> {
    let mut team = vec![set(
        "Pikachu",
        "Light Ball",
        "Static",
        ["Thunderbolt", "Nuzzle", "Play Rough", "Protect"],
        "Timid",
        [0, 0, 0, 252, 4, 252],
        "Electric",
    )];
    team.extend(ou_team().into_iter().take(5));
    team
}

pub fn showcase_opponent() -> Vec<TeamSet> {
    let mut team = vec![set(
        "Charizard",
        "Heavy-Duty Boots",
        "Blaze",
        ["Flamethrower", "Air Slash", "Dragon Dance", "Earthquake"],
        "Jolly",
        [0, 252, 0, 0, 4, 252],
        "Fire",
    )];
    team.extend(ou_team().into_iter().skip(1));
    team
}

pub fn preset_team(format: &str, which: &str) -> Option<Vec<TeamSet>> {
    match (format, which) {
        ("gen9ou", "sample" | "a") => Some(ou_team()),
        ("gen9ou", "mirror" | "b") => Some(ou_team_mirror()),
        ("gen9vgc2025regi", "sample" | "a" | "b") => Some(vgc_team()),
        ("gen9ubers", "showcase" | "pikachu") => Some(showcase_player()),
        ("gen9ubers", "showcase-foe" | "charizard") => Some(showcase_opponent()),
        _ => None,
    }
}
