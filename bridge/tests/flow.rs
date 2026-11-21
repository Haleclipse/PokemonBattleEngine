use battle_bridge::{
    BattleRoom, SetInput, export_team, host, import_team, preset_team, validate_sets,
};

fn sample(which: &str) -> Vec<SetInput> {
    preset_team("gen9ou", which)
        .expect("sample")
        .into_iter()
        .map(SetInput::from)
        .collect()
}

#[test]
fn showcase_leads_are_legal_in_ubers() {
    for which in ["pikachu", "charizard"] {
        let team = preset_team("gen9ubers", which)
            .expect(which)
            .into_iter()
            .map(SetInput::from)
            .collect();
        let check = validate_sets(host(), "gen9ubers", team).expect("validate");
        assert!(
            check.ok,
            "{which}: {:?}",
            check
                .problems
                .iter()
                .map(|problem| &problem.message)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn ou_sample_passes_the_engine_validator() {
    let check = validate_sets(host(), "gen9ou", sample("sample")).expect("validate");
    assert!(
        check.ok,
        "{:?}",
        check
            .problems
            .iter()
            .map(|problem| &problem.message)
            .collect::<Vec<_>>()
    );
    assert_eq!(check.team.len(), 6);
}

#[test]
fn showdown_paste_round_trips_species_and_moves() {
    let exported = export_team(&sample("sample"));
    let imported = import_team(&exported);
    assert_eq!(imported.len(), 6);
    assert_eq!(imported[0].species, "Great Tusk");
    assert_eq!(imported[0].moves[0], "Headlong Rush");
    let check = validate_sets(host(), "gen9ou", imported).expect("validate");
    assert!(check.ok, "{:?}", check.problems);
}

#[test]
fn illegal_team_is_rejected_by_the_engine() {
    let mut team = sample("sample");
    team[0].moves[0] = "Splash".into();
    let check = validate_sets(host(), "gen9ou", team).expect("validate");
    assert!(!check.ok);
    assert!(
        check
            .problems
            .iter()
            .any(|problem| !problem.message.is_empty())
    );
}

#[test]
fn player_view_hides_the_opponents_item_and_ability() {
    let mut room = BattleRoom::create(
        host(),
        "gen9ou",
        7,
        "ai",
        sample("sample"),
        sample("mirror"),
    )
    .expect("battle");
    room.choose(host(), "p1", vec![0, 1, 2, 3, 4, 5])
        .expect("preview");
    let state = room.state(host(), "p1").expect("state");
    let foe = &state["battle"]["sides"]["p2"]["pokemon"][0];
    assert_eq!(foe["item"]["known"], false);
    assert_eq!(foe["ability"]["known"], false);
    assert!(foe["moves"].as_array().expect("moves").is_empty());
    assert!(foe["hp"]["current"].is_null());
    assert!(state["battle"]["sides"]["p1"]["pokemon"][0]["hp"]["current"].is_number());
    let foe_text = foe.to_string();
    assert!(!foe_text.contains("Magic Guard"));
    assert!(!foe_text.contains("Leftovers"));
}

#[test]
fn a_turn_is_resolved_by_the_engine() {
    let mut room = BattleRoom::create(
        host(),
        "gen9ou",
        11,
        "ai",
        sample("sample"),
        sample("mirror"),
    )
    .expect("battle");
    let state = room.choose(host(), "p1", vec![]).expect("start");
    let turn = state["battle"]["turn"].as_u64().expect("turn");
    let actor = state["request"]["actors"][0]["actor"]
        .as_u64()
        .expect("actor") as u8;
    let slot = state["request"]["actors"][0]["moves"]
        .as_array()
        .expect("moves")
        .iter()
        .find(|move_| move_["disabled"] == false)
        .expect("a move")["slot"]
        .as_u64()
        .expect("slot") as u8;
    let after = room
        .act(
            host(),
            "p1",
            vec![battle_bridge::ActionIn {
                actor,
                kind: "move".into(),
                slot: Some(slot),
                target: None,
                mechanic: None,
                pokemon: None,
            }],
        )
        .expect("act");
    let advanced =
        after["battle"]["turn"].as_u64().unwrap_or(0) > turn || after["finished"].is_object();
    assert!(advanced, "{after}");
    assert!(!after["log"].as_array().expect("log").is_empty());
}

#[test]
fn restart_returns_to_team_preview() {
    let mut room = BattleRoom::create(
        host(),
        "gen9ou",
        3,
        "hotseat",
        sample("sample"),
        sample("mirror"),
    )
    .expect("battle");
    room.choose(host(), "p1", vec![]).expect("p1");
    room.choose(host(), "p2", vec![5, 4, 3, 2, 1, 0])
        .expect("p2");
    let restarted = room.restart(host(), "p1").expect("restart");
    assert_eq!(restarted["phase"], "preview");
    assert!(restarted["battle"].is_null());
}

#[test]
fn ai_observer_cannot_read_the_opponents_side() {
    let room = BattleRoom::create(
        host(),
        "gen9ou",
        1,
        "ai",
        sample("sample"),
        sample("mirror"),
    )
    .expect("battle");
    let error = room.state(host(), "p2").expect_err("hidden");
    assert!(error.contains("私有"));
}
