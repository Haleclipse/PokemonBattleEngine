//! Each observer's view against Showdown's client (DESIGN.md §47.0 items 9, 11 and 12, M10.4c):
//! the `BattleView` of P1, P2 and a spectator against what @pkmn/client knows, fed that
//! observer's protocol stream and, for a player, its requests (`views` in an oracle frame,
//! `tools/oracle-node/src/views.mjs`).
//!
//! - What a view claims to know (`Seen::Known`, `Seen::KnownNone`) must be what the client
//!   knows; anything else is a leak, or a wrong value.
//! - What is always public must match: who stands where, the members that have been on the
//!   field, the HP as the format shares it, status, stat stages, a faint, Terastallization,
//!   the side and field conditions.
//! - What a view does not know yet (`Seen::Unknown`) is no divergence: the client may have
//!   learned it (an ability that announced itself), and the view catches up as M10.4c tracks
//!   what is revealed.
//!
//! The names come from the engine's own frame at the same checkpoint (`SemanticFrame`, Showdown
//! ids) and from the dex.

use std::collections::BTreeSet;

use pokemon_battle_engine::ids::{PokemonId, SideId};
use pokemon_battle_engine::state::{Gender, Stat};
use pokemon_battle_engine::view::{Hp, Observer, PokemonView, Seen};
use serde_json::Value;

use super::scenario::{Compiled, ReplayReport};

/// The notation of the views compared (`views.mjs`'s `VERSION`): what an older recording holds is
/// written otherwise — R-M10-16 to R-M10-18's Nightlies have the client's weather names and the
/// cosmetic formes as they are, and no version — and is not compared.
const VIEWS_VERSION: u64 = 7;

const OBSERVERS: [(&str, Observer); 3] = [
    ("p1", Observer::Side(SideId::P1)),
    ("p2", Observer::Side(SideId::P2)),
    ("spectator", Observer::Spectator),
];

/// Side conditions left out: a foreseen attack is a slot's condition, which @pkmn/client files
/// under the side's and the view does not show yet; the guards of one turn @pkmn/client drops
/// at the turn's upkeep, and Showdown at the next residual.
const UNCOMPARED_SIDE_CONDITIONS: [&str; 6] = [
    "futuresight",
    "doomdesire",
    "quickguard",
    "wideguard",
    "craftyshield",
    "matblock",
];

const STATS: [(Stat, &str); 7] = [
    (Stat::Attack, "atk"),
    (Stat::Defense, "def"),
    (Stat::SpecialAttack, "spa"),
    (Stat::SpecialDefense, "spd"),
    (Stat::Speed, "spe"),
    (Stat::Accuracy, "accuracy"),
    (Stat::Evasion, "evasion"),
];

/// Compares every observer's view with the client's at a checkpoint; `rust` is the engine's
/// frame there, as compared. A recording made before the views (2026-10-24) has none, one made
/// before their present notation another (`VIEWS_VERSION`).
pub(super) fn compare_views(
    report: &mut ReplayReport,
    checkpoint: &str,
    compiled: &Compiled,
    rust: &Value,
    oracle: &Value,
) {
    let Some(views) = oracle
        .get("views")
        .filter(|views| views["version"].as_u64() == Some(VIEWS_VERSION))
    else {
        return;
    };
    let mut diffs = Vec::new();
    for (name, observer) in OBSERVERS {
        let client = &views[name];
        let view = compiled.engine.view(observer);
        for (index, side) in [SideId::P1, SideId::P2].into_iter().enumerate() {
            let at = format!("views.{name}.sides[{index}]");
            side_diffs(
                compiled,
                observer == Observer::Side(side),
                &view.side(side),
                &client["sides"][index],
                &at,
                &mut diffs,
            );
        }
        field_diffs(
            &view.field(),
            &client["field"],
            &rust["field"],
            &format!("views.{name}.field"),
            &mut diffs,
        );
    }
    for (path, actual, expected) in diffs.into_iter().take(10) {
        report.divergences.push(format!(
            "{checkpoint}: {path} rust={actual} showdown={expected}"
        ));
    }
}

type Diffs = Vec<(String, String, String)>;

/// A field's path under `at`.
fn path(at: &str, field: &str) -> String {
    format!("{at}.{field}")
}

fn differ(diffs: &mut Diffs, path: String, actual: impl ToString, expected: impl ToString) {
    diffs.push((path, actual.to_string(), expected.to_string()));
}

/// Showdown `toID`.
fn to_id(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The name a client's `ident` gives (`p1: Floatzel`); empty for a member it has only seen in a
/// team preview.
fn ident_name(pokemon: &Value) -> Option<&str> {
    let ident = pokemon["ident"].as_str()?;
    ident
        .split_once(": ")
        .map(|(_, name)| name)
        .filter(|name| !name.is_empty())
}

/// The label Showdown names `id` by.
fn label(compiled: &Compiled, id: PokemonId) -> String {
    (0..2)
        .find_map(|side| {
            compiled.teams[side]
                .iter()
                .position(|member| *member == id)
                .map(|index| compiled.names[side][index].clone())
        })
        .unwrap_or_else(|| format!("{id:?}"))
}

fn side_diffs(
    compiled: &Compiled,
    own: bool,
    side: &pokemon_battle_engine::view::SideView<'_>,
    client: &Value,
    at: &str,
    diffs: &mut Diffs,
) {
    let team: Vec<&Value> = client["team"].as_array().into_iter().flatten().collect();
    // Two of one Pokémon stood on the field at once, and @pkmn/client guessed which is the
    // Illusion's holder (`views.mjs`): that side's who-is-where and members are its guess.
    if client["unresolved_illusion"].as_bool().unwrap_or(false) {
        return;
    }

    // The members the observer has seen, by name (an Illusion's disguise under its own).
    let shown: Vec<PokemonView<'_>> = side.team().collect();
    let rust_names: BTreeSet<String> = shown.iter().map(|p| label(compiled, p.id())).collect();
    let client_names: BTreeSet<String> = team
        .iter()
        .filter_map(|p| ident_name(p).map(str::to_owned))
        .collect();
    if rust_names != client_names {
        differ(
            diffs,
            path(at, "team"),
            format!("{rust_names:?}"),
            format!("{client_names:?}"),
        );
    }

    // Who stands where.
    let client_active: Vec<Option<&str>> = client["active"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|slot| {
            slot.as_u64()
                .and_then(|i| team.get(i as usize))
                .and_then(|p| ident_name(p))
        })
        .collect();
    for (slot, (_, standing)) in side.active().enumerate() {
        let actual = standing.map(|p| label(compiled, p.id()));
        let expected = client_active.get(slot).copied().flatten();
        if actual.as_deref() != expected {
            differ(
                diffs,
                format!("{at}.active[{slot}]"),
                format!("{actual:?}"),
                format!("{expected:?}"),
            );
        }
    }

    // Of its own side a player knows everything from its requests, and the engine's own frame
    // compares that; @pkmn/client's own team is no oracle for it — a member only a request
    // has shown has no HP (0/0), a lock or a charge drops the other moves (the request lists
    // the locked one alone), a move seen used is added once more. Its members and who stands
    // where are compared, nothing of them.
    // @pkmn/client keeps one entry per name, and a second one for a Pokémon that comes in under
    // that name with other details (an Illusion's disguise at its wearer's level): the one on
    // the field is the entry standing there; one off it, the only entry of its name.
    let standing: Vec<_> = side
        .active()
        .filter_map(|(_, p)| p.map(|p| p.id()))
        .collect();
    for pokemon in shown.iter().filter(|_| !own) {
        let name = label(compiled, pokemon.id());
        let named: Vec<&Value> = team
            .iter()
            .copied()
            .filter(|p| ident_name(p) == Some(name.as_str()))
            .collect();
        let seen = if standing.contains(&pokemon.id()) {
            client["active"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|slot| slot.as_u64().and_then(|i| team.get(i as usize)))
                .copied()
                .find(|p| ident_name(p) == Some(name.as_str()))
        } else {
            named.first().copied().filter(|_| named.len() == 1)
        };
        let Some(seen) = seen else {
            continue;
        };
        pokemon_diffs(compiled, pokemon, seen, &format!("{at}.{name}"), diffs);
    }
    // A real format's battle, started past its Team Preview (`Compiled::preview`): a custom game's
    // is built with no session, its preview the host's to give. The recording has it at the first
    // checkpoint after the start alone (it never changes after).
    if !own && compiled.preview.is_some() && !client["preview"].is_null() {
        preview_diffs(compiled, side, &client["preview"], at, diffs);
    }

    // The side's conditions as the view shows them, named as the engine's frame names them.
    let actual: BTreeSet<String> = side
        .conditions()
        .map(|kind| to_id(&format!("{kind:?}")))
        .collect();
    let expected: BTreeSet<String> = client["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(to_id)
        .filter(|id| !UNCOMPARED_SIDE_CONDITIONS.contains(&id.as_str()))
        .collect();
    let actual: BTreeSet<String> = actual
        .into_iter()
        .filter(|id| !UNCOMPARED_SIDE_CONDITIONS.contains(&id.as_str()))
        .collect();
    if actual != expected {
        differ(
            diffs,
            path(at, "conditions"),
            format!("{actual:?}"),
            format!("{expected:?}"),
        );
    }
}

/// What the team preview showed of a side the observer does not know (DESIGN.md §47.0 item 14),
/// against the protocol's lines (`views.mjs`'s `preview`): each member's `poke` line — species (a
/// hidden forme as its base species), level, gender, the Gen 4-7 item mark — or, the open team
/// sheets shown, its sheet: species, level, gender, item, ability, moves, Tera type.
fn preview_diffs(
    compiled: &Compiled,
    side: &pokemon_battle_engine::view::SideView<'_>,
    preview: &Value,
    at: &str,
    diffs: &mut Diffs,
) {
    let dex = &compiled.dex;
    let generation = compiled.engine.omniscient_view().generation().number();
    let pokes: Vec<&Value> = preview["pokes"].as_array().into_iter().flatten().collect();
    let sheet: Option<Vec<&Value>> = preview["sheet"]
        .as_array()
        .map(|sheet| sheet.iter().collect());
    let members: Vec<_> = side.previewed().collect();
    if members.len() != pokes.len() {
        differ(
            diffs,
            path(at, "previewed"),
            members.len(),
            format!("{} members", pokes.len()),
        );
        return;
    }
    let gender = |gender: Gender| match gender {
        Gender::Male => "M",
        Gender::Female => "F",
        Gender::Genderless => "",
    };
    for (index, (member, poke)) in members.iter().zip(&pokes).enumerate() {
        let at = format!("{at}.previewed[{index}]");
        let shown = sheet.as_ref().and_then(|sheet| sheet.get(index).copied());
        let species = dex
            .species_by_id(member.species())
            .map(|s| to_id(&s.name))
            .unwrap_or_default();
        let seen = shown.unwrap_or(poke);
        let mut expected = vec![
            ("species", to_id(seen["species"].as_str().unwrap_or(""))),
            ("level", seen["level"].as_u64().unwrap_or(100).to_string()),
            ("gender", seen["gender"].as_str().unwrap_or("").to_string()),
        ];
        let mut actual = vec![
            ("species", species),
            ("level", member.level().to_string()),
            ("gender", gender(member.gender()).to_string()),
        ];
        let seen_item = |item: Seen<pokemon_battle_engine::ids::ItemId>| match item {
            Seen::Known(item) => dex
                .item_by_id(item)
                .map(|i| to_id(&i.name))
                .unwrap_or_default(),
            Seen::KnownNone => "none".into(),
            Seen::Unknown => "unknown".into(),
        };
        match shown {
            Some(sheet) => {
                expected.push(("forme_hidden", "false".into()));
                let item = to_id(sheet["item"].as_str().unwrap_or(""));
                expected.push(("item", if item.is_empty() { "none".into() } else { item }));
                expected.push((
                    "ability",
                    as_one(&to_id(sheet["ability"].as_str().unwrap_or(""))),
                ));
                let moves: Vec<String> = sheet["moves"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|id| move_id(&to_id(id)))
                    .collect();
                expected.push(("moves", format!("{moves:?}")));
                let tera = to_id(sheet["tera_type"].as_str().unwrap_or(""));
                if !tera.is_empty() {
                    expected.push(("tera_type", tera));
                }
            }
            None => {
                expected.push((
                    "forme_hidden",
                    poke["forme_hidden"].as_bool().unwrap_or(false).to_string(),
                ));
                // Gen 4-7 mark the members that hold an item.
                let marked = !poke["item"].as_str().unwrap_or("").is_empty();
                expected.push((
                    "item",
                    if (4..=7).contains(&generation) && !marked {
                        "none".into()
                    } else {
                        "unknown".into()
                    },
                ));
                expected.push(("ability", "unknown".into()));
                expected.push(("moves", "unknown".into()));
            }
        }
        actual.push(("forme_hidden", member.forme_hidden().to_string()));
        actual.push(("item", seen_item(member.item())));
        actual.push((
            "ability",
            match member.ability() {
                Seen::Known(ability) => as_one(
                    &dex.ability_by_id(ability)
                        .map(|a| to_id(&a.name))
                        .unwrap_or_default(),
                ),
                _ => "unknown".into(),
            },
        ));
        actual.push((
            "moves",
            match member.moves() {
                Some(moves) => {
                    let moves: Vec<String> = moves
                        .iter()
                        .map(|id| {
                            move_id(
                                &dex.move_by_id(*id)
                                    .map(|m| to_id(&m.name))
                                    .unwrap_or_default(),
                            )
                        })
                        .collect();
                    format!("{moves:?}")
                }
                None => "unknown".into(),
            },
        ));
        if expected.iter().any(|(field, _)| *field == "tera_type") {
            actual.push((
                "tera_type",
                match member.tera_type() {
                    Seen::Known(tera) => to_id(tera.name()),
                    Seen::KnownNone => "none".into(),
                    Seen::Unknown => "unknown".into(),
                },
            ));
        }
        for ((field, actual), (_, expected)) in actual.iter().zip(&expected) {
            if actual != expected {
                differ(diffs, path(&at, field), actual, expected);
            }
        }
    }
}

fn pokemon_diffs(
    compiled: &Compiled,
    pokemon: &PokemonView<'_>,
    seen: &Value,
    at: &str,
    diffs: &mut Diffs,
) {
    let dex = &compiled.dex;
    let species = pokemon
        .species()
        .and_then(|id| dex.species_by_id(id))
        .map(|s| to_id(&s.name))
        .unwrap_or_default();
    let client_species = to_id(seen["species"].as_str().unwrap_or(""));
    if species != client_species {
        differ(diffs, path(at, "species"), &species, &client_species);
    }
    // A fainted member: the faint and its species are what is seen of it (`0 fnt`); @pkmn/client
    // keeps the last level, status and stages it saw — a disguise's details at its wearer's
    // level, for an Illusion's holder that fainted in it.
    let fainted = seen["fainted"].as_bool().unwrap_or(false);
    if pokemon.fainted() != fainted {
        differ(diffs, path(at, "fainted"), pokemon.fainted(), fainted);
    }
    if pokemon.fainted() {
        if seen["hp"].as_u64() != Some(0) {
            differ(diffs, path(at, "hp"), "0 fnt", &seen["hp"]);
        }
        return;
    }
    let forgotten = seen["illusion_reset"].as_bool().unwrap_or(false);
    if Some(u64::from(pokemon.level())) != seen["level"].as_u64() && !forgotten {
        differ(diffs, path(at, "level"), pokemon.level(), &seen["level"]);
    }
    let gender = match pokemon.gender() {
        Gender::Male => "M",
        Gender::Female => "F",
        Gender::Genderless => "N",
    };
    if seen["gender"]
        .as_str()
        .is_some_and(|g| !g.is_empty() && g != gender)
    {
        differ(diffs, path(at, "gender"), gender, &seen["gender"]);
    }
    let (hp, max) = (
        seen["hp"].as_u64().unwrap_or(0),
        seen["max_hp"].as_u64().unwrap_or(0),
    );
    let matches = match pokemon.hp() {
        _ if forgotten => true,
        Hp::Exact { hp: h, max: m } => (u64::from(h), u64::from(m)) == (hp, max),
        Hp::Percent(p) => (u64::from(p), 100) == (hp, max),
        Hp::Pixels(p) => (u64::from(p), 48) == (hp, max),
    };
    if !matches {
        differ(
            diffs,
            path(at, "hp"),
            format!("{:?}", pokemon.hp()),
            format!("{hp}/{max}"),
        );
    }

    let status = pokemon.status().map(super::scenario::status_id);
    let client_status = seen["status"].as_str().filter(|s| !s.is_empty());
    if status != client_status && !forgotten {
        differ(
            diffs,
            path(at, "status"),
            format!("{status:?}"),
            format!("{client_status:?}"),
        );
    }
    // Gen 1's one Special stage: @pkmn/client keeps it as `spa` (the protocol boosts both).
    let gen1 = compiled.engine.omniscient_view().generation()
        == pokemon_battle_engine::spec::Generation::Gen1;
    // Stages copied from one of the observer's own (Psych Up …), which @pkmn/client's own team
    // has not right, or from a side whose Illusion it guessed at (`views.mjs`, R-M11-162).
    let copied = seen["boosts_from_own"].as_bool().unwrap_or(false);
    for (stat, id) in STATS {
        if (gen1 && stat == Stat::SpecialDefense) || copied {
            continue;
        }
        let client = seen["boosts"][id].as_i64().unwrap_or(0);
        if i64::from(pokemon.boost(stat)) != client {
            differ(
                diffs,
                path(at, &format!("boosts.{id}")),
                pokemon.boost(stat),
                client,
            );
        }
    }
    let tera = pokemon.terastallized().map(|t| to_id(t.name()));
    let client_tera = seen["terastallized"].as_str().map(to_id);
    if tera != client_tera {
        differ(
            diffs,
            path(at, "terastallized"),
            format!("{tera:?}"),
            format!("{client_tera:?}"),
        );
    }

    // What the view claims to know of the hidden ones: the client must know the same — but for
    // an item @pkmn/client took a Z-Move's use for (`item_from_z_move`, an inference).
    // Gen 3-4 Knock Off: @pkmn/client keeps the item, marked knocked off; Showdown has taken it.
    let client_item = if seen["item_effect"].as_str() == Some("knocked off") {
        String::new()
    } else {
        to_id(seen["item"].as_str().unwrap_or(""))
    };
    let inferred = seen["item_from_z_move"].as_bool().unwrap_or(false);
    // A line that named the Pokémon in another's slot (`misplaced`): the client filed it wrong.
    // An entry the client made anew for it (`entry_renewed`) forgot what the old one knew.
    let misplaced = seen["misplaced"].as_bool().unwrap_or(false)
        || seen["entry_renewed"].as_bool().unwrap_or(false);
    // An entry @pkmn/client filed apart from another of its name (`filed_apart`: an Illusion's
    // disguise at its wearer's level) has what it guessed was its own — its item and ability as
    // well as its moves and volatiles (Gimmighoul's Illusion as Pincurchin at its own level
    // beside the real one, whose Download the view gives the Pincurchin it shows, R-M10-61).
    let filed_apart = seen["filed_apart"].as_bool().unwrap_or(false);
    // An Illusion broken: what its holder showed in the disguise @pkmn/client keeps on the
    // disguise's entry (`illusion_worn`), the view gives the holder (`illusion_unmasked`), which
    // the client then knows nothing of — an item it showed, or one it lost: the holder's entry
    // keeps the item it had before (a Gen 8 Zoroark's Metal Powder, which Corrosive Gas took
    // from it as Centiskorch, R-M10-70).
    let worn = seen["illusion_worn"].as_bool().unwrap_or(false);
    let unmasked = seen["illusion_unmasked"].as_bool().unwrap_or(false);
    match pokemon.item() {
        _ if inferred || worn || misplaced || filed_apart => {}
        Seen::Known(_) if unmasked && client_item.is_empty() => {}
        Seen::KnownNone if unmasked => {}
        Seen::Known(item) => {
            let name = dex
                .item_by_id(item)
                .map(|i| to_id(&i.name))
                .unwrap_or_default();
            if name != client_item {
                differ(diffs, path(at, "item"), &name, &client_item);
            }
        }
        Seen::KnownNone if !client_item.is_empty() => {
            differ(diffs, path(at, "item"), "none", &client_item);
        }
        _ => {}
    }
    // What the client knows and the view does not (DESIGN.md §47.0 item 16, M10.4e): the
    // other way of the same check — the views are whole as well as leak-free (R-M10-40).
    if matches!(pokemon.item(), Seen::Unknown)
        && !client_item.is_empty()
        && !(inferred || worn || misplaced || filed_apart)
    {
        differ(diffs, path(at, "item_missing"), "unknown", &client_item);
    }
    // A Gen 4 Skill Swap @pkmn/client skipped (`ability_swapped_unseen`): what it knows of the
    // ability since is stale. So is the ability a Pokémon took by Transform from one of the
    // observer's own before Gen 7 (`transformed_into_own`): the client copies its own team's,
    // whose ability a request names from Gen 7 alone (`getSwitchRequestData`: `baseAbility`
    // before), a change since it came in unknown to it (R-M10-40); and the ability it copied
    // from an entry the client made anew or filed wrong (`ability_copied_stale`, R-M11-148).
    let swapped_unseen = seen["ability_swapped_unseen"].as_bool().unwrap_or(false)
        || seen["ability_copied_stale"].as_bool().unwrap_or(false)
        || (seen["transformed_into_own"].as_bool().unwrap_or(false)
            && compiled.engine.omniscient_view().generation()
                < pokemon_battle_engine::spec::Generation::Gen7);
    if let Seen::Known(ability) = pokemon.ability()
        && !swapped_unseen
        && !worn
        && !misplaced
        && !filed_apart
        && !(unmasked && client_ability(seen).is_empty())
    {
        let name = as_one(
            &dex.ability_by_id(ability)
                .map(|a| to_id(&a.name))
                .unwrap_or_default(),
        );
        let client = as_one(&client_ability(seen));
        if name != client {
            differ(diffs, path(at, "ability"), &name, &client);
        }
    }
    if matches!(pokemon.ability(), Seen::Unknown)
        && !client_ability(seen).is_empty()
        && !swapped_unseen
        && !worn
        && !misplaced
        && !filed_apart
    {
        differ(
            diffs,
            path(at, "ability_missing"),
            "unknown",
            client_ability(seen),
        );
    }
    // The moves seen (DESIGN.md §47.0 item 13), in the order first seen — a transformed one's in
    // the client's order of its copy, one the observer's own took by Transform in its request's
    // (`moves_from_request`), compared as sets — with the uses seen of each; but where the client
    // knows nothing of a copy (`moves_copied`: Mimic, Sketch), and the uses where its count is
    // not of uses (`uses_uncompared`: a PP change, Gen 2's unmarked locks, Lunar Dance's PP, the
    // copies' PP in a request).
    // One transformed into the observer's own (`transformed_into_own`) has its slots in the
    // view, which Transform copied all of, and in the client a copy of its own team's moves — no
    // oracle (a lock's request leaves them out).
    if !(worn
        || unmasked
        || misplaced
        || filed_apart
        || seen["moves_copied"].as_bool().unwrap_or(false)
        || seen["transformed_into_own"].as_bool().unwrap_or(false))
    {
        let moves = pokemon.moves();
        let actual: Vec<(String, u16)> = moves
            .revealed()
            .map(|m| {
                let name = dex
                    .move_by_id(m.move_id())
                    .map(|m| to_id(&m.name))
                    .unwrap_or_default();
                (move_id(&name), m.observed_uses())
            })
            .collect();
        let client: Vec<&Value> = seen["moves"].as_array().into_iter().flatten().collect();
        let mut names: Vec<String> = actual.iter().map(|(name, _)| name.clone()).collect();
        let mut expected: Vec<String> = Vec::new();
        // `cant|p|nopp|recharge` (a recharge turn whose recharge a Dancer's attempt already took,
        // R-M11-131) has the client remember Showdown's `recharge` pseudo-move: no move it has.
        for id in client
            .iter()
            .filter_map(|slot| slot["id"].as_str())
            .filter(|id| *id != "recharge")
            .map(move_id)
        {
            // A request's `hiddenpowerdark70` is the `hiddenpower` a line named.
            if !expected.contains(&id) {
                expected.push(id);
            }
        }
        // What an observer's own took by Transform the client files under the Pokémon its request
        // says it copied (`moves_from_request`), and in a chain of Transforms (a custom game's)
        // under the wrong one: the view must know nothing the client does not.
        let from_request = seen["moves_from_request"].as_bool().unwrap_or(false);
        if seen["transformed"].as_bool().unwrap_or(false) || from_request {
            names.sort();
            expected.sort();
        }
        let differs = if from_request {
            names.iter().any(|name| !expected.contains(name))
        } else {
            names != expected
        };
        if differs {
            differ(
                diffs,
                path(at, "moves"),
                format!("{names:?}"),
                format!("{expected:?}"),
            );
        }
        let uses_uncompared = from_request || seen["uses_uncompared"].as_bool().unwrap_or(false);
        for (name, uses) in actual.iter().filter(|_| !uses_uncompared) {
            let Some(slot) = client
                .iter()
                .find(|slot| slot["id"].as_str().map(move_id).as_deref() == Some(name.as_str()))
            else {
                continue;
            };
            if slot["uses_uncompared"].as_bool().unwrap_or(false) {
                continue;
            }
            if let Some(used) = slot["pp_used"].as_u64()
                && used != u64::from(*uses)
            {
                differ(diffs, path(at, &format!("moves.{name}")), uses, used);
            }
        }
    }

    // The volatiles seen on it (DESIGN.md §47.0 item 13), by @pkmn/client's ids: those it keeps
    // as volatiles that the engine has a kind of (`VOLATILE_IDS`); its own (a type change, a
    // forme, Transform, an item gone) are other parts of the view.
    if !(worn || unmasked || misplaced || filed_apart) {
        let actual: BTreeSet<String> = pokemon
            .volatiles()
            .iter()
            .map(|kind| volatile_id(&format!("{kind:?}")))
            .filter(|id| VOLATILE_IDS.contains(&id.as_str()))
            .collect();
        let expected: BTreeSet<String> = seen["volatiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(client_volatile)
            .filter(|id| VOLATILE_IDS.contains(&id.as_str()))
            .collect();
        if actual != expected {
            differ(
                diffs,
                path(at, "volatiles"),
                format!("{actual:?}"),
                format!("{expected:?}"),
            );
        }
    }
}

/// The volatiles compared: the client's ids of the engine's kinds that show themselves.
const VOLATILE_IDS: [&str; 69] = [
    "substitute",
    "leechseed",
    "confusion",
    "taunt",
    "encore",
    "disable",
    "mustrecharge",
    "followme",
    "partiallytrapped",
    "ingrain",
    "helpinghand",
    "ragepowder",
    "magiccoat",
    "snatch",
    "reflect",
    "lightscreen",
    "mist",
    "attract",
    "quarkdrive",
    "protosynthesis",
    "mudsport",
    "watersport",
    "yawn",
    "nightmare",
    "healblock",
    "embargo",
    "saltcure",
    "syrupbomb",
    "throatchop",
    "tarshot",
    "aquaring",
    "stockpile",
    "powertrick",
    "focusenergy",
    "dragoncheer",
    "laserfocus",
    "charge",
    "noretreat",
    "roost",
    "powder",
    "electrify",
    "foresight",
    "miracleeye",
    "smackdown",
    "magnetrise",
    "telekinesis",
    "autotomize",
    "flashfire",
    "perishsong",
    "trapped",
    "octolock",
    "focuspunch",
    "beakblast",
    "shelltrap",
    "destinybond",
    "grudge",
    "spotlight",
    "bide",
    "dynamax",
    "uproar",
    "torment",
    "imprison",
    "lockon",
    "curse",
    "rage",
    "glaiverush",
    "skydrop",
    "protect",
    "endure",
];

/// The client's id of an engine kind (its `Debug` name).
fn volatile_id(kind: &str) -> String {
    to_id(kind)
}

/// An id @pkmn/client keeps a volatile under, as the engine names the kind: a partial trap under
/// the move that set it (`-start|p|move: Wrap`), a protection under its move, Flash Fire under
/// its ability (`ability:flashfire`), and an effect the client's data for the generation lacks
/// under its whole name (`movehealblock`: Heal Block is past in Gen 9).
fn client_volatile(id: &str) -> String {
    let id = to_id(id);
    let bare = ["ability", "move"]
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .filter(|rest| VOLATILE_IDS.contains(rest))
        .unwrap_or(&id);
    match bare {
        "bind" | "wrap" | "firespin" | "clamp" | "whirlpool" | "sandtomb" | "magmastorm"
        | "infestation" | "snaptrap" | "thundercage" => "partiallytrapped".into(),
        "maxguard" | "kingsshield" | "spikyshield" | "banefulbunker" | "obstruct" | "silktrap"
        | "burningbulwark" => "protect".into(),
        _ => bare.into(),
    }
}

/// The ability @pkmn/client knows (a suppressed one, `-endability`, `views.mjs` keeps as the
/// ability it is from notation 7; an older recording's blank falls back to the base ability).
/// As One shows itself as `As One`, then names its parts (`Unnerve`, `Chilling Neigh`, `Grim
/// Neigh`), which the client takes for the ability in turn.
fn client_ability(seen: &Value) -> String {
    // Gen 1-2's `No Ability` is none.
    let none = |id: String| if id == "noability" { String::new() } else { id };
    let ability = none(to_id(seen["ability"].as_str().unwrap_or("")));
    let base = none(to_id(seen["base_ability"].as_str().unwrap_or("")));
    // Transformed, its ability is the one it took (`-transform` copies what the client knew of
    // the target's, nothing included), not its own (R-M10-40).
    if ability.is_empty() && !seen["transformed"].as_bool().unwrap_or(false) {
        return base;
    }
    // As One, by its line's name or an open team sheet's (`As One (Spectrier)`).
    if as_one(&base) == "asone"
        && ["unnerve", "chillingneigh", "grimneigh"].contains(&ability.as_str())
    {
        return base;
    }
    ability
}

/// As One by the name its line shows (`As One`, whichever of Glastrier's or Spectrier's; an open
/// team sheet names which).
fn as_one(id: &str) -> String {
    if id.starts_with("asone") {
        "asone".into()
    } else {
        id.into()
    }
}

/// A move id as a request names it, without what it adds: Hidden Power's type and power,
/// Return's and Frustration's power (`hiddenpowerfire60`, `return102`, `frustrationnull`).
fn move_id(id: &str) -> String {
    if id.starts_with("hiddenpower") {
        return "hiddenpower".into();
    }
    let bare = id
        .trim_end_matches("null")
        .trim_end_matches(|c: char| c.is_ascii_digit());
    if bare == "return" || bare == "frustration" {
        bare.into()
    } else {
        id.into()
    }
}

/// The field as the view shows it; the weather and the terrain, which it shows as they are, named
/// as the engine's frame names them.
fn field_diffs(
    field: &pokemon_battle_engine::view::FieldView<'_>,
    client: &Value,
    rust: &Value,
    at: &str,
    diffs: &mut Diffs,
) {
    let weather = rust["weather"]["kind"].as_str().map(to_id);
    let client_weather = client["weather"]
        .as_str()
        .map(to_id)
        .filter(|w| !w.is_empty());
    if weather != client_weather {
        differ(
            diffs,
            path(at, "weather"),
            format!("{weather:?}"),
            format!("{client_weather:?}"),
        );
    }
    let terrain = rust["terrain"]["kind"].as_str().map(to_id);
    let client_terrain = client["terrain"]
        .as_str()
        .map(to_id)
        .filter(|t| !t.is_empty());
    if terrain != client_terrain {
        differ(
            diffs,
            path(at, "terrain"),
            format!("{terrain:?}"),
            format!("{client_terrain:?}"),
        );
    }
    let actual: BTreeSet<String> = field
        .conditions()
        .map(|kind| to_id(&format!("{kind:?}")))
        .collect();
    // Fairy Lock and Ion Deluge are announced with `-fieldactivate`, which @pkmn/client keeps no
    // condition for.
    let expected: BTreeSet<String> = client["pseudo_weather"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(to_id)
        .collect();
    let actual: BTreeSet<String> = actual
        .into_iter()
        .filter(|id| id != "fairylock" && id != "iondeluge")
        .collect();
    if actual != expected {
        differ(
            diffs,
            path(at, "pseudo_weather"),
            format!("{actual:?}"),
            format!("{expected:?}"),
        );
    }
}
