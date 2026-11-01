//! The pre-battle session (M9.4, DESIGN.md §45.5) against Showdown (`tests/fixtures/formats/
//! built.json`, export-builder.mjs): every target format's team policy and engine switches as
//! its battle reads them, and the Pokémon built from teams Showdown admits — registered
//! (validated, normalised), then constructed.

use std::sync::OnceLock;

use pokemon_battle_engine::BattleMode;
use pokemon_battle_engine::format::{FormatLibrary, Stats, TeamSet, to_id};
use pokemon_battle_engine::ids::{AbilityId, PokemonId, SideId};
use pokemon_battle_engine::session::{
    BattleSession, SessionError, SessionFormat, SessionPhase, SessionRequest, SetBuildError,
    TeamChoice, TeamChoiceError, TeamSlot,
};
use pokemon_battle_engine::state::{BattleRng, Gender};
use serde_json::{Value, json};

fn library() -> &'static FormatLibrary {
    static LIBRARY: OnceLock<FormatLibrary> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        FormatLibrary::open(concat!(env!("CARGO_MANIFEST_DIR"), "/data"))
            .unwrap_or_else(|error| panic!("{error}"))
    })
}

fn fixture() -> &'static Value {
    static FIXTURE: OnceLock<Value> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/formats/built.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    })
}

/// A set as the export writes it (Showdown's `PokemonSet` JSON).
fn team_set(input: &Value) -> TeamSet {
    let text = |field: &str| input[field].as_str().unwrap_or_default().to_string();
    let stats = |value: &Value| {
        value.as_object().map(|stats| {
            let stat = |name: &str| stats[name].as_i64().unwrap() as i32;
            Stats {
                hp: stat("hp"),
                atk: stat("atk"),
                def: stat("def"),
                spa: stat("spa"),
                spd: stat("spd"),
                spe: stat("spe"),
            }
        })
    };
    TeamSet {
        name: text("name"),
        species: text("species"),
        item: text("item"),
        ability: text("ability"),
        moves: input["moves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap().to_string())
            .collect(),
        nature: text("nature"),
        gender: text("gender"),
        evs: stats(&input["evs"]),
        ivs: stats(&input["ivs"]),
        level: input["level"].as_u64().unwrap_or(0) as u32,
        shiny: input["shiny"].as_bool().unwrap_or(false),
        happiness: input["happiness"].as_f64(),
        hp_type: text("hpType"),
        gigantamax: input["gigantamax"].as_bool().unwrap_or(false),
        tera_type: text("teraType"),
    }
}

/// Each format's team policy and engine switches: its game type, the team's size, picks and
/// preview, the mechanics its clauses take away.
#[test]
fn every_format_s_switches_are_the_ones_its_battle_reads() {
    let mut wrong = Vec::new();
    let formats = fixture()["formats"].as_object().unwrap();
    for (format, expected) in formats {
        let session = SessionFormat::new(library(), format).unwrap();
        let policy = session.policy();
        let rules = session.rules().unwrap();
        let expected = &expected["rules"];
        let mode = match session.spec().mode {
            BattleMode::Singles => "singles",
            BattleMode::Doubles => "doubles",
        };
        let actual = json!({
            "game_type": mode,
            "max_team_size": policy.max_team_size,
            "picked_team_size": policy.picked_team_size,
            // Showdown asks for the order when a rule previews the teams, or privately when the
            // format has picks and no preview (`runPickTeam`).
            "team_preview": policy.preview || policy.picked_team_size.is_some(),
            "max_chosen_team_size": if policy.preview { policy.picked_team_size } else { None },
            "dynamax": (rules.generation().number() == 8).then(|| rules.special().dynamax),
            "tera": (rules.generation().number() == 9).then(|| rules.special().tera),
        });
        let mut expected = expected.clone();
        // Registration clauses are the validator's (tests/team_validator.rs); battle-time ones
        // come with M9.5.
        for clause in ["species_clause", "item_clause", "sleep_clause"] {
            expected.as_object_mut().unwrap().remove(clause);
        }
        if actual != expected {
            wrong.push(format!(
                "{format}:\n  engine   {actual}\n  showdown {expected}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {}:\n{}",
        wrong.len(),
        formats.len(),
        wrong.join("\n")
    );
}

fn gender_name(gender: Gender) -> &'static str {
    match gender {
        Gender::Male => "M",
        Gender::Female => "F",
        Gender::Genderless => "N",
    }
}

fn fixture_team(format: &str, index: usize) -> Vec<TeamSet> {
    fixture()["formats"][format]["teams"][index]["sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(team_set)
        .collect()
}

/// Teams Showdown admits register under the engine's validation too, and each Pokémon the
/// session makes of a normalised set is the one Showdown's `Side` makes: species, level, gender
/// (unless drawn), HP and stats, ability, item, moves with their PP, types, Tera type, Hidden
/// Power's type, happiness.
#[test]
fn every_team_a_format_admits_is_built_as_showdown_builds_it() {
    let mut wrong = Vec::new();
    let mut count = 0;
    for (format, entry) in fixture()["formats"].as_object().unwrap() {
        let session = SessionFormat::new(library(), format).unwrap();
        let dex = session.dex().clone();
        for team in entry["teams"].as_array().unwrap() {
            let sets: Vec<TeamSet> = team["sets"]
                .as_array()
                .unwrap()
                .iter()
                .map(team_set)
                .collect();
            let mut rng = BattleRng::new(1);
            let registered = match session.register(SideId::P1, sets, &mut rng) {
                Ok(registered) => registered,
                Err(error) => {
                    wrong.push(format!("{format}: {error:?}"));
                    continue;
                }
            };
            let state = match session.state([&registered.sets, &registered.sets], rng) {
                Ok(state) => state,
                Err(error) => {
                    wrong.push(format!("{format}: {error:?}"));
                    continue;
                }
            };
            for (index, expected) in team["built"].as_array().unwrap().iter().enumerate() {
                count += 1;
                let pokemon = state.pokemon(PokemonId(index as u8 + 1)).unwrap();
                assert_eq!(pokemon.side, SideId::P1);
                let name_of = |name: Option<&str>| to_id(name.unwrap_or_default());
                let ability = if pokemon.ability == AbilityId::NONE {
                    // Showdown's `No Ability` before Gen 3.
                    "noability".to_string()
                } else {
                    name_of(dex.ability_by_id(pokemon.ability).map(|a| a.name.as_str()))
                };
                let stats = pokemon.stats;
                let types: Vec<&str> = [Some(pokemon.types.primary), pokemon.types.secondary]
                    .into_iter()
                    .flatten()
                    .map(|t| t.name())
                    .collect();
                let moves: Vec<Value> = pokemon
                    .moves
                    .iter()
                    .map(|slot| {
                        json!([
                            name_of(dex.move_by_id(slot.move_id).map(|m| m.name.as_str())),
                            slot.max_pp
                        ])
                    })
                    .collect();
                let species = pokemon.species.and_then(|s| dex.species_by_id(s));
                let item = pokemon.held_item.and_then(|i| dex.item_by_id(i));
                let tera = session.spec().generation.number() >= 9;
                let mut actual = json!({
                    "species": name_of(species.map(|s| s.name.as_str())),
                    "level": pokemon.level,
                    "gender": gender_name(pokemon.gender),
                    "max_hp": pokemon.max_hp,
                    "stats": {"atk": stats.attack, "def": stats.defense, "spa": stats.special_attack,
                              "spd": stats.special_defense, "spe": stats.speed},
                    "ability": ability,
                    "item": name_of(item.map(|i| i.name.as_str())),
                    "moves": moves,
                    "types": types,
                    "tera_type": tera.then(|| pokemon.tera_type.name()),
                    "hp_type": pokemon.hidden_power.0.name(),
                    "happiness": pokemon.happiness,
                });
                if expected["gender"].is_null() {
                    actual["gender"] = Value::Null;
                }
                if &actual != expected {
                    wrong.push(format!(
                        "{format} #{index}:\n  engine   {actual}\n  showdown {expected}"
                    ));
                }
            }
        }
    }
    let shown: Vec<_> = wrong.iter().take(12).cloned().collect();
    assert!(
        wrong.is_empty(),
        "{} wrong of {count}:\n{}",
        wrong.len(),
        shown.join("\n")
    );
    assert!(count > 700, "{count} Pokémon");
}

/// Registration is of the whole team: a team the format does not admit is refused with
/// Showdown's problems — Item Clause too when the duplicate would not be picked (VGC brings 4 of
/// 6). A Gigantamax set, which the engine cannot play, is a set error.
#[test]
fn a_team_the_format_refuses_is_not_registered() {
    let mut rng = BattleRng::new(1);
    let session = SessionFormat::new(library(), "gen9ou").unwrap();
    let pikachu = || TeamSet {
        species: "Pikachu".into(),
        ability: "Static".into(),
        moves: vec!["Thunderbolt".into()],
        ..TeamSet::default()
    };
    let Err(SessionError::Team { side, problems }) =
        session.register(SideId::P2, vec![pikachu(), pikachu()], &mut rng)
    else {
        panic!("registered");
    };
    assert_eq!(side, SideId::P2);
    assert!(
        problems
            .iter()
            .any(|p| p.message == "You are limited to one of each Pokémon by Species Clause."),
        "{problems:?}"
    );

    let session = SessionFormat::new(library(), "gen9vgc2025regi").unwrap();
    let mut team = fixture_team("gen9vgc2025regi", 0);
    team[5].item = team[0].item.clone();
    let Err(SessionError::Team { problems, .. }) = session.register(SideId::P1, team, &mut rng)
    else {
        panic!("registered");
    };
    assert!(
        problems
            .iter()
            .any(|p| p.message == "You are limited to 1 of each item by Item Clause."),
        "{problems:?}"
    );

    let session = SessionFormat::new(library(), "gen8vgc2022").unwrap();
    let gmax = TeamSet {
        gigantamax: true,
        ..pikachu()
    };
    assert_eq!(session.engine_set(&gmax), Err(SetBuildError::Gigantamax));
}

/// `Item Clause = N` keeps its number (Showdown's `onValidateTeam` limit, 1 by default).
#[test]
fn item_clause_keeps_its_limit() {
    let mut rng = BattleRng::new(1);
    let session =
        SessionFormat::with_custom(library(), "gen9ou", &["Item Clause = 2".to_string()]).unwrap();
    let with_leftovers = |count: usize| {
        let mut team = fixture_team("gen9ou", 0);
        for set in team.iter_mut().take(count) {
            set.item = "Leftovers".into();
        }
        for (index, set) in team.iter_mut().enumerate().skip(count) {
            if set.item == "Leftovers" {
                set.item = ["Life Orb", "Choice Scarf", "Focus Sash", "Heavy-Duty Boots"]
                    [index % 4]
                    .into();
            }
        }
        team
    };
    assert!(
        session
            .register(SideId::P1, with_leftovers(2), &mut rng)
            .is_ok()
    );
    let Err(SessionError::Team { problems, .. }) =
        session.register(SideId::P1, with_leftovers(3), &mut rng)
    else {
        panic!("registered");
    };
    assert_eq!(
        problems[0].message,
        "You are limited to 2 of each item by Item Clause."
    );
}

/// The battle starts from the chosen Pokémon only, in their order, the leads in the active
/// slots; a gender a set leaves open is drawn at registration (the preview shows it), from the
/// battle's generator (Showdown `sample(['M', 'F'])`), the same for the same seed.
#[test]
fn a_battle_starts_from_the_chosen_pokemon_only() {
    let session = SessionFormat::new(library(), "gen9vgc2025regi").unwrap();
    let mut open = fixture_team("gen9vgc2025regi", 1);
    for set in &mut open {
        set.gender.clear();
    }
    let register = |seed| {
        let mut rng = BattleRng::new(seed);
        let p1 = session
            .register(SideId::P1, fixture_team("gen9vgc2025regi", 0), &mut rng)
            .unwrap();
        let p2 = session
            .register(SideId::P2, open.clone(), &mut rng)
            .unwrap();
        (p1, p2, rng)
    };
    let (p1, p2, rng) = register(3);
    assert_eq!(register(3).1, p2);
    let dex = session.dex().clone();
    let mut drawn = Vec::new();
    for set in &p2.sets {
        let fixed = dex.species_by_name(&set.species).and_then(|s| s.gender);
        // (The validator writes a fixed gender in itself.)
        if fixed.is_none() {
            drawn.push(set.gender.clone());
        }
    }
    assert!(drawn.iter().all(|g| g == "M" || g == "F"), "{drawn:?}");
    assert!(
        drawn.contains(&"M".into()) && drawn.contains(&"F".into()),
        "{drawn:?}"
    );

    // VGC: four of six, in the order chosen.
    let chosen_p1 = [
        p1.sets[3].clone(),
        p1.sets[0].clone(),
        p1.sets[5].clone(),
        p1.sets[1].clone(),
    ];
    // One generator, as Showdown's: the battle goes on from where registration left it.
    let state = session
        .state([&chosen_p1, &p2.sets[..4]], rng.clone())
        .unwrap();
    assert_eq!(state.rng, rng);
    let engine = session.start([&chosen_p1, &p2.sets[..4]], rng).unwrap();
    let state = engine.state();
    assert_eq!(state.all_pokemon().len(), 8);
    assert_eq!(state.side(SideId::P1).team.len(), 4);
    assert_eq!(
        state.side(SideId::P1).active,
        vec![Some(PokemonId(1)), Some(PokemonId(2))]
    );
    let lead = state.pokemon(PokemonId(1)).unwrap().species.unwrap();
    assert_eq!(
        dex.species_by_id(lead).unwrap().name,
        session.engine_set(&p1.sets[3]).unwrap().species
    );
    // Adjust Level = 50.
    assert!(
        state
            .all_pokemon()
            .iter()
            .all(|pokemon| pokemon.level == 50)
    );
    assert_eq!(engine.rules().format().as_str(), "gen9vgc2025regi");
}

/// A team choice as Showdown's protocol writes it (`team 3142`: 1-based slots).
fn choice(text: &str) -> TeamChoice {
    TeamChoice {
        order: text
            .chars()
            .map(|c| TeamSlot(c.to_digit(10).unwrap() as u8 - 1))
            .collect(),
    }
}

/// A session of `format` with both sides registered: P1 the fixture's first team, P2 its
/// second (the export's preview pair).
fn registered(format: &str, seed: u64) -> BattleSession<'static> {
    let mut session = BattleSession::new(SessionFormat::new(library(), format).unwrap(), seed);
    session
        .register(SideId::P1, fixture_team(format, 0))
        .unwrap();
    session
        .register(SideId::P2, fixture_team(format, 1))
        .unwrap();
    session
}

/// What each side sees of the other at Team Preview is Showdown's `|poke|` line of each of its
/// Pokémon — the formes it hides hidden (a gender drawn on either side is left out) — and the
/// choice asks for Showdown's picks; a format without preview or picks goes straight to the
/// battle.
#[test]
fn team_preview_shows_what_showdown_shows() {
    let mut previews = 0;
    for (format, entry) in fixture()["formats"].as_object().unwrap() {
        let session = registered(format, 1);
        let preview = &entry["preview"];
        if preview.is_null() {
            assert_eq!(session.phase(), SessionPhase::InBattle, "{format}");
            assert!(session.view(SideId::P1).opponent_preview.is_none());
            let engine = session.engine().unwrap();
            assert_eq!(engine.state().side(SideId::P1).team.len(), 6, "{format}");
            continue;
        }
        let SessionRequest::Choose { sides, picked, .. } = session.request() else {
            panic!("{format}: {:?}", session.request());
        };
        assert_eq!(sides, vec![SideId::P1, SideId::P2]);
        let expected_picked = preview["teampreview"]
            .as_str()
            .map_or(6, |n| n.parse::<usize>().unwrap());
        assert_eq!(picked, [expected_picked; 2], "{format}");
        for (viewer, shown) in [(SideId::P2, "p1"), (SideId::P1, "p2")] {
            let entries = session.view(viewer).opponent_preview.unwrap();
            let lines = preview["poke"][shown].as_array().unwrap();
            assert_eq!(entries.len(), lines.len(), "{format}");
            for (index, (entry, line)) in entries.iter().zip(lines).enumerate() {
                previews += 1;
                assert_eq!(
                    entry.item,
                    line[1].as_str().unwrap(),
                    "{format} {shown} #{index}"
                );
                let mut line = line[0].as_str().unwrap().to_string();
                let mut details = entry.details.clone();
                if preview["drawn"][shown][index].as_bool().unwrap() {
                    for gender in [", M", ", F"] {
                        line = line.replace(gender, "");
                        details = details.replace(gender, "");
                    }
                }
                assert_eq!(details, line, "{format} {shown} #{index}");
                assert_eq!(entry.tera_type, None);
            }
        }
    }
    assert!(previews > 200, "{previews}");
}

/// `Open Team Sheets`: nothing until both sides accept, then each sees the other's sheet as
/// Showdown's `showOpenTeamSheets` writes it.
#[test]
fn open_team_sheets_show_what_showdown_shows() {
    let mut sheets = 0;
    for (format, entry) in fixture()["formats"].as_object().unwrap() {
        let expected = &entry["preview"]["sheets"];
        if expected.is_null() {
            continue;
        }
        let mut session = registered(format, 1);
        assert!(session.view(SideId::P2).opponent_sheet.is_none());
        session.accept_open_team_sheets(SideId::P1).unwrap();
        assert!(session.view(SideId::P2).opponent_sheet.is_none());
        session.accept_open_team_sheets(SideId::P2).unwrap();
        for (viewer, shown) in [(SideId::P2, "p1"), (SideId::P1, "p2")] {
            let sheet = session.view(viewer).opponent_sheet.unwrap();
            let drawn = &entry["preview"]["drawn"][shown];
            for (index, (set, expected)) in sheet
                .iter()
                .zip(expected[shown].as_array().unwrap())
                .enumerate()
            {
                sheets += 1;
                let text = |field: &str| to_id(expected[field].as_str().unwrap_or_default());
                let moves: Vec<String> = expected["moves"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| to_id(m.as_str().unwrap()))
                    .collect();
                let actual = json!([
                    to_id(&set.species),
                    to_id(&set.item),
                    to_id(&set.ability),
                    set.moves.iter().map(|m| to_id(m)).collect::<Vec<_>>(),
                    set.level,
                    to_id(&set.tera_type),
                    to_id(&set.hp_type),
                    set.nature,
                    set.evs.is_none(),
                ]);
                let wanted = json!([
                    text("species"),
                    text("item"),
                    text("ability"),
                    moves,
                    expected["level"],
                    text("teraType"),
                    text("hpType"),
                    "",
                    true,
                ]);
                assert_eq!(actual, wanted, "{format} {shown} #{index}");
                if !drawn[index].as_bool().unwrap() {
                    assert_eq!(set.gender, expected["gender"].as_str().unwrap_or_default());
                }
            }
        }
    }
    assert!(sheets >= 24, "{sheets}");
}

/// Each team choice is taken as Showdown's `chooseTeam` takes it: cut to the picks, completed
/// from the team's order, a slot twice or one the team lacks refused with its words; the battle
/// then starts from the Pokémon brought, in that order.
#[test]
fn every_team_choice_is_taken_as_showdown_takes_it() {
    let mut count = 0;
    for (format, entry) in fixture()["formats"].as_object().unwrap() {
        let preview = &entry["preview"];
        if preview.is_null() {
            continue;
        }
        for case in preview["choices"].as_array().unwrap() {
            count += 1;
            let text = case["choice"].as_str().unwrap();
            let mut session = registered(format, 1);
            match session.choose(SideId::P1, choice(text)) {
                Ok(()) => {
                    let brought: Vec<u64> = session
                        .view(SideId::P1)
                        .own_choice
                        .unwrap()
                        .iter()
                        .map(|slot| u64::from(slot.0))
                        .collect();
                    let expected: Vec<u64> = case["brought"]
                        .as_array()
                        .unwrap_or_else(|| panic!("{format} {text}: refused by Showdown"))
                        .iter()
                        .map(|i| i.as_u64().unwrap())
                        .collect();
                    assert_eq!(brought, expected, "{format} {text}");
                    session.choose(SideId::P2, TeamChoice::default()).unwrap();
                    let engine = session.engine().unwrap();
                    let team = &engine.state().side(SideId::P1).team;
                    assert_eq!(team.len(), expected.len(), "{format} {text}");
                    let registered = &session.view(SideId::P1).own.unwrap();
                    let lead = engine.state().pokemon(team[0]).unwrap().species.unwrap();
                    assert_eq!(
                        engine.dex().species_by_id(lead).unwrap().name,
                        session
                            .format()
                            .engine_set(&registered[expected[0] as usize])
                            .unwrap()
                            .species,
                        "{format} {text}"
                    );
                }
                Err(SessionError::Choice(error)) => {
                    assert_eq!(
                        Some(error.to_string().as_str()),
                        case["error"].as_str(),
                        "{format} {text}"
                    );
                }
                Err(error) => panic!("{format} {text}: {error:?}"),
            }
        }
    }
    assert!(count > 200, "{count}");
}

/// A side's choice stays its own until both have chosen; and neither the order the sides
/// register in nor the order they choose in changes the battle (Showdown builds P1, then P2).
#[test]
fn a_choice_stays_hidden_and_the_order_of_calls_does_not_matter() {
    let format = "gen9vgc2025regi";
    // Genders left open, for the draws to come in an order.
    let open = |index| {
        let mut team = fixture_team(format, index);
        for set in &mut team {
            set.gender.clear();
        }
        team
    };
    let mut first = BattleSession::new(SessionFormat::new(library(), format).unwrap(), 9);
    first.register(SideId::P1, open(0)).unwrap();
    first.register(SideId::P2, open(1)).unwrap();
    first.choose(SideId::P1, choice("3142")).unwrap();
    let seen = first.view(SideId::P2);
    assert!(seen.opponent_chosen);
    assert_eq!(seen.own_choice, None);
    assert_eq!(
        first.request(),
        SessionRequest::Choose {
            sides: vec![SideId::P2],
            picked: [4, 4],
            leads: 2
        }
    );
    assert_eq!(
        first.choose(SideId::P1, choice("12")),
        Err(SessionError::AlreadyDone(SideId::P1))
    );
    first.choose(SideId::P2, choice("65")).unwrap();

    let mut second = BattleSession::new(SessionFormat::new(library(), format).unwrap(), 9);
    second.register(SideId::P2, open(1)).unwrap();
    // Nothing of a team is shown before both are registered.
    let early = second.view(SideId::P1);
    assert!(early.opponent_registered && early.own.is_none());
    assert!(early.opponent_preview.is_none() && early.opponent_sheet.is_none());
    second.register(SideId::P1, open(0)).unwrap();
    second.choose(SideId::P2, choice("65")).unwrap();
    second.choose(SideId::P1, choice("3142")).unwrap();
    assert_eq!(
        first.engine().unwrap().snapshot(),
        second.engine().unwrap().snapshot()
    );
    assert_eq!(
        first.register(SideId::P1, fixture_team(format, 0)),
        Err(SessionError::WrongPhase(SessionPhase::InBattle))
    );
}

/// A session restores under its own registration only — the format's rules, its data and the
/// dex (`SessionFormat::fingerprint`) — and goes on as if never stopped. The battle rules'
/// fingerprint covers the engine's switches only: a ban the battle never reads leaves it.
#[test]
fn a_session_restores_under_its_own_registration_only() {
    let format = "gen9vgc2025regi";
    let mut whole = registered(format, 4);
    whole.choose(SideId::P1, choice("2413")).unwrap();
    let midway = whole.snapshot();
    whole.choose(SideId::P2, choice("1234")).unwrap();

    let mut resumed = BattleSession::restore(
        SessionFormat::new(library(), format).unwrap(),
        midway.clone(),
    )
    .unwrap();
    assert_eq!(resumed.view(SideId::P2), whole_view_before(&midway, format));
    resumed.choose(SideId::P2, choice("1234")).unwrap();
    assert_eq!(resumed.snapshot(), whole.snapshot());

    let in_battle = BattleSession::restore(
        SessionFormat::new(library(), format).unwrap(),
        whole.snapshot(),
    )
    .unwrap();
    assert_eq!(in_battle.snapshot(), whole.snapshot());

    let banned = SessionFormat::with_custom(library(), format, &["-Pikachu".to_string()]).unwrap();
    assert_ne!(
        banned.fingerprint(),
        SessionFormat::new(library(), format).unwrap().fingerprint()
    );
    assert_eq!(
        banned.rules().unwrap().fingerprint(),
        SessionFormat::new(library(), format)
            .unwrap()
            .rules()
            .unwrap()
            .fingerprint()
    );
    assert_eq!(
        BattleSession::restore(banned, midway).err(),
        Some(SessionError::FingerprintMismatch)
    );
}

/// The view a restored session gives: the same as the session it was taken of.
fn whole_view_before(
    snapshot: &pokemon_battle_engine::session::SessionSnapshot,
    format: &str,
) -> pokemon_battle_engine::session::SessionView {
    BattleSession::restore(
        SessionFormat::new(library(), format).unwrap(),
        snapshot.clone(),
    )
    .unwrap()
    .view(SideId::P2)
}

/// `TeamChoiceError` words its refusals as Showdown does.
#[test]
fn a_refused_choice_is_worded_as_showdown_words_it() {
    assert_eq!(
        TeamChoiceError::NoSuchSlot(TeamSlot(6)).to_string(),
        "Can't choose for Team Preview: You do not have a Pokémon in slot 7"
    );
}

/// Team Preview's hidden formes, per generation's own `teampreview` rule, on sets made to reach
/// them (unvalidated): Urshifu's, Silvally's, Arceus' … as `Urshifu-*`, Zacian short of its
/// Crowned forme as `Zacian-*`, Greninja's short of Greninja-Ash in Gen 4-7, Greninja-Bond and
/// Rockruff-Dusk as their base species, the item flag of Gen 4-7 (`mail` in Gen 5).
#[test]
fn team_preview_hides_the_formes_showdown_hides() {
    let mut count = 0;
    for (format, entry) in fixture()["targeted_previews"].as_object().unwrap() {
        let session = SessionFormat::new(library(), format).unwrap();
        for (set, line) in entry["sets"]
            .as_array()
            .unwrap()
            .iter()
            .zip(entry["poke"].as_array().unwrap())
        {
            count += 1;
            let mut set = team_set(set);
            set.species = session.validator().species(&set.species).name.clone();
            let shown = session.preview_entry(&set);
            assert_eq!(
                json!([shown.details, shown.item]),
                *line,
                "{format} {}",
                set.species
            );
        }
    }
    assert!(count >= 18, "{count}");
}

/// The session's start is the format's (`SessionFormat::start`) on the chosen Pokémon, what the
/// team preview showed given to it (`team_preview`, DESIGN.md §47.0 item 14), the leads then sent
/// in (`run_lead_switch_ins`).
#[test]
fn the_session_starts_the_battle_with_the_leads_in() {
    let format = "gen9vgc2025regi";
    let mut session = registered(format, 2);
    session.choose(SideId::P1, choice("2143")).unwrap();
    session.choose(SideId::P2, choice("3412")).unwrap();
    let snapshot = session.snapshot();

    let chosen = |side: usize, order: [usize; 4]| -> Vec<TeamSet> {
        let team = snapshot.registered[side].as_ref().unwrap();
        order.iter().map(|i| team.sets[*i].clone()).collect()
    };
    let (p1, p2) = (chosen(0, [1, 0, 3, 2]), chosen(1, [2, 3, 0, 1]));
    let before = registered(format, 2).snapshot().rng;
    let mut manual = session.format().start([&p1, &p2], before).unwrap();
    let teams = [0, 1].map(|side| snapshot.registered[side].as_ref().unwrap().sets.clone());
    let preview = session
        .format()
        .team_preview(
            [&teams[0], &teams[1]],
            [&[1, 0, 3, 2], &[2, 3, 0, 1]],
            false,
        )
        .unwrap()
        .unwrap();
    manual.show_team_preview(preview);
    let unsent = manual.snapshot();
    manual.run_lead_switch_ins();
    assert_eq!(snapshot.battle.as_ref(), Some(&manual.snapshot()));
    assert_ne!(snapshot.battle, Some(unsent));
}

/// The data directory's digest covers every file the library may read: a host's copy with one
/// file changed is another registration.
#[test]
fn a_session_s_fingerprint_follows_the_data() {
    let scratch = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session-data");
    let _ = std::fs::remove_dir_all(&scratch);
    for dir in ["formats", "legality"] {
        std::fs::create_dir_all(scratch.join(dir)).unwrap();
        for entry in std::fs::read_dir(library().root().join(dir)).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, scratch.join(dir).join(path.file_name().unwrap())).unwrap();
        }
    }
    let copy = FormatLibrary::open(&scratch).unwrap();
    let same = SessionFormat::new(&copy, "gen9ou").unwrap().fingerprint();
    assert_eq!(
        same,
        SessionFormat::new(library(), "gen9ou")
            .unwrap()
            .fingerprint()
    );
    let go = scratch.join("legality/pokemongo.json");
    let mut text = std::fs::read_to_string(&go).unwrap();
    text.push('\n');
    std::fs::write(&go, text).unwrap();
    let edited = FormatLibrary::open(&scratch).unwrap();
    assert_ne!(
        SessionFormat::new(&edited, "gen9ou").unwrap().fingerprint(),
        same
    );
}

/// A registration without the learnsets (`TeamValidator::without_learnsets`) says so: its
/// teams are not known to be legal, and it is another registration (DESIGN.md §45.6).
#[test]
fn a_registration_without_the_learnsets_says_so() {
    use pokemon_battle_engine::format::TeamValidator;
    let mut rng = BattleRng::new(1);
    let checked = SessionFormat::new(library(), "gen9ou").unwrap();
    let unchecked = SessionFormat::from_validator(
        TeamValidator::without_learnsets(library(), "gen9ou", &[]).unwrap(),
    )
    .unwrap();
    let team = fixture_team("gen9ou", 0);
    assert!(
        checked
            .register(SideId::P1, team.clone(), &mut rng)
            .unwrap()
            .learnsets_checked
    );
    assert!(
        !unchecked
            .register(SideId::P1, team, &mut rng)
            .unwrap()
            .learnsets_checked
    );
    assert_ne!(checked.fingerprint(), unchecked.fingerprint());
}

/// Mega Rayquaza Clause (`onBegin`: `canMegaEvo = null` for a Rayquaza, DESIGN.md §45.7): in
/// Gen 7 Ubers a Rayquaza that knows Dragon Ascent is offered no Mega Evolution; the same set
/// under the engine's free ruleset is.
#[test]
fn mega_rayquaza_clause_keeps_rayquaza_from_mega_evolving() {
    let session = SessionFormat::new(library(), "gen7ubers").unwrap();
    assert!(session.switches().clauses.mega_rayquaza_clause);
    let rayquaza = TeamSet {
        species: "Rayquaza".into(),
        ability: "Air Lock".into(),
        moves: vec!["Dragon Ascent".into(), "Extreme Speed".into()],
        nature: "Adamant".into(),
        evs: Some(Stats {
            atk: 252,
            spe: 252,
            hp: 4,
            ..Stats::default()
        }),
        ..TeamSet::default()
    };
    let mut rng = BattleRng::new(1);
    let p1 = session
        .register(SideId::P1, vec![rayquaza.clone()], &mut rng)
        .unwrap();
    let p2 = session
        .register(SideId::P2, fixture_team("gen7ubers", 0), &mut rng)
        .unwrap();
    let engine = session
        .start([&p1.sets, &p2.sets[..1]], rng.clone())
        .unwrap();
    assert_eq!(engine.mechanic_options(PokemonId(1)).mega, None);
    // Under the engine's own ruleset the same Rayquaza is offered its Mega Evolution.
    let state = session.state([&p1.sets, &p2.sets[..1]], rng).unwrap();
    let free =
        pokemon_battle_engine::RulesResolver::resolve(&pokemon_battle_engine::BattleSpec::new(
            pokemon_battle_engine::Generation::Gen7,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap();
    let engine = pokemon_battle_engine::BattleEngine::create_new(
        std::sync::Arc::new(free),
        session.dex().clone(),
        state,
    )
    .unwrap();
    assert!(engine.mechanic_options(PokemonId(1)).mega.is_some());
}

/// What a team preview shows of a member, for the battle's views (DESIGN.md §47.0 item 14), is
/// its Team Preview line read back — a hidden forme as its base species, marked as hidden; its
/// level and gender; in Gen 4-7 no item where the line marks none — and to its own side the
/// member itself; each member brought is the battle's Pokémon of its place.
#[test]
fn a_previewed_member_is_its_team_preview_line() {
    let (mut count, mut hidden_count, mut marked_none) = (0, 0, 0);
    for (format, entry) in fixture()["targeted_previews"].as_object().unwrap() {
        let session = SessionFormat::new(library(), format).unwrap();
        let generation = format[3..4].parse::<u8>().unwrap();
        // The sets the battle can build (made to reach a line, some are not of the generation's
        // dex: Mail, Greninja in Gen 4), with their lines.
        let (sets, lines): (Vec<TeamSet>, Vec<&Value>) = entry["sets"]
            .as_array()
            .unwrap()
            .iter()
            .zip(entry["poke"].as_array().unwrap())
            .map(|(set, line)| {
                let mut set = team_set(set);
                set.species = session.validator().species(&set.species).name.clone();
                (set, line)
            })
            .filter(|(set, _)| {
                session
                    .team_preview([std::slice::from_ref(set), &[]], [&[], &[]], false)
                    .is_ok()
            })
            .unzip();
        let all: Vec<usize> = (0..sets.len()).collect();
        let brought = [1, 0];
        let preview = session
            .team_preview([&sets, &sets], [&all, &brought], false)
            .unwrap()
            .unwrap();
        let dex = session.dex();
        for (index, ((member, line), set)) in
            preview.sides[0].iter().zip(lines).zip(&sets).enumerate()
        {
            count += 1;
            let details = line[0].as_str().unwrap();
            let name = details.split(',').next().unwrap();
            let (base, hidden) = match name.strip_suffix("-*") {
                Some(base) => (base, true),
                None => (name, false),
            };
            let shown = &member.shown;
            assert_eq!(
                dex.species_by_id(shown.species).unwrap().name,
                base,
                "{format} {details}"
            );
            assert_eq!(shown.forme_hidden, hidden, "{format} {details}");
            hidden_count += usize::from(hidden);
            let level = if set.level == 100 {
                String::new()
            } else {
                format!(", L{}", set.level)
            };
            assert!(details.contains(&level), "{format} {details}");
            assert_eq!(shown.level as u32, set.level);
            let marked = !line[1].as_str().unwrap().is_empty();
            let item = if (4..=7).contains(&generation) && !marked {
                Some(None)
            } else {
                None
            };
            assert_eq!(shown.item, item, "{format} {details}");
            marked_none += usize::from(item.is_some());
            assert_eq!((shown.ability, &shown.moves), (None, &None));
            assert_eq!(shown.tera_type.is_some(), generation < 9);
            // Its own side knows it all.
            let known = &member.known;
            assert!(!known.forme_hidden);
            assert_eq!(
                to_id(&dex.species_by_id(known.species).unwrap().name),
                to_id(&set.species),
                "{format}"
            );
            assert_eq!(known.moves.as_ref().map(Vec::len), Some(set.moves.len()));
            assert!(known.ability.is_some() && known.item.is_some());
            assert_eq!(member.pokemon, Some(PokemonId(index as u8 + 1)));
        }
        // With the open team sheets shown, each member is its sheet's species, nothing hidden
        // (Urshifu's forme, Zacian crowned by its Rusted Sword).
        let sheets = session
            .team_preview([&sets, &sets], [&all, &brought], true)
            .unwrap()
            .unwrap();
        for (member, set) in sheets.sides[0].iter().zip(&sets) {
            let sheet = session.open_sheet(set);
            assert_eq!(
                to_id(&dex.species_by_id(member.shown.species).unwrap().name),
                to_id(&sheet.species),
                "{format}"
            );
            assert!(!member.shown.forme_hidden);
        }
        // P2 brought its second member, then its first: the battle's next two.
        let p2: Vec<_> = preview.sides[1]
            .iter()
            .map(|member| member.pokemon)
            .collect();
        let first = sets.len() as u8 + 1;
        if sets.len() >= 2 {
            assert_eq!(
                p2[..2],
                [Some(PokemonId(first + 1)), Some(PokemonId(first))]
            );
            assert!(p2[2..].iter().all(Option::is_none));
        }
    }
    assert!(
        count >= 12 && hidden_count >= 4 && marked_none >= 1,
        "{count} {hidden_count} {marked_none}"
    );
}

/// Open team sheets in the battle (DESIGN.md §47.0 item 14): each side's view lists the other's
/// members as the preview showed them, or as the sheets showed them once both sides accepted —
/// and a member brought in then has its sheet's item, ability and all its moves known, none
/// seen used. A side knows its own members whole.
#[test]
fn open_team_sheets_are_known_in_the_battle() {
    use pokemon_battle_engine::view::{Observer, Seen};
    let format = "gen9vgc2025regi";
    for shown in [false, true] {
        // P2's lead (its fourth member) with three moves: a sheet shows them all.
        let mut session = BattleSession::new(SessionFormat::new(library(), format).unwrap(), 3);
        session
            .register(SideId::P1, fixture_team(format, 0))
            .unwrap();
        let mut p2 = fixture_team(format, 1);
        p2[3].moves.truncate(3);
        session.register(SideId::P2, p2).unwrap();
        if shown {
            session.accept_open_team_sheets(SideId::P1).unwrap();
            session.accept_open_team_sheets(SideId::P2).unwrap();
        }
        let entries = session.view(SideId::P1).opponent_preview.unwrap();
        let sheet = session.view(SideId::P1).opponent_sheet;
        session.choose(SideId::P1, choice("1234")).unwrap();
        session.choose(SideId::P2, choice("4321")).unwrap();
        let engine = session.engine().unwrap();
        let dex = session.format().dex();
        for observer in [Observer::Side(SideId::P1), Observer::Spectator] {
            let view = engine.view(observer);
            let p2 = view.side(SideId::P2);
            let members: Vec<_> = p2.previewed().collect();
            assert_eq!(members.len(), entries.len());
            for (index, (member, entry)) in members.iter().zip(&entries).enumerate() {
                let species = to_id(&dex.species_by_id(member.species()).unwrap().name);
                let name = |id: &str| to_id(id);
                match &sheet {
                    None => {
                        let shown = entry.details.split(',').next().unwrap();
                        assert_eq!(species, name(shown.trim_end_matches("-*")));
                        assert_eq!(member.moves(), None);
                        assert_eq!(member.ability(), Seen::Unknown);
                        assert_eq!(member.item(), Seen::Unknown);
                    }
                    Some(sheet) => {
                        let set = &sheet[index];
                        assert_eq!(species, name(&set.species));
                        let moves: Vec<String> = member
                            .moves()
                            .unwrap()
                            .iter()
                            .map(|id| name(&dex.move_by_id(*id).unwrap().name))
                            .collect();
                        let expected: Vec<String> = set.moves.iter().map(|m| name(m)).collect();
                        assert_eq!(moves, expected);
                        let Seen::Known(ability) = member.ability() else {
                            panic!("{set:?}");
                        };
                        assert_eq!(
                            name(&dex.ability_by_id(ability).unwrap().name),
                            name(&set.ability)
                        );
                        let item = match member.item() {
                            Seen::Known(item) => name(&dex.item_by_id(item).unwrap().name),
                            Seen::KnownNone => String::new(),
                            Seen::Unknown => panic!("{set:?}"),
                        };
                        assert_eq!(item, name(&set.item));
                    }
                }
            }
            // P2's lead, its fourth member, brought in.
            let (_, lead) = p2.active().next().unwrap();
            let lead = lead.unwrap();
            let moves: Vec<_> = lead.moves().revealed().collect();
            match &sheet {
                None => assert!(moves.is_empty() && !lead.moves().complete()),
                Some(_) => {
                    let expected: Vec<_> = members[3].moves().unwrap().to_vec();
                    let ids: Vec<_> = moves.iter().map(|m| m.move_id()).collect();
                    assert_eq!(ids, expected);
                    assert!(moves.iter().all(|m| m.observed_uses() == 0));
                    assert_eq!(ids.len(), 3);
                    assert!(lead.moves().complete());
                    assert_eq!(lead.ability(), members[3].ability());
                    assert_eq!(lead.item(), members[3].item());
                }
            }
        }
        // P1 knows its own members whole.
        let view = engine.view(Observer::Side(SideId::P1));
        for member in view.side(SideId::P1).previewed() {
            assert!(member.moves().is_some() && !member.forme_hidden());
            assert!(member.ability() != Seen::Unknown && member.item() != Seen::Unknown);
            assert!(member.tera_type() != Seen::Unknown);
        }
    }
}
