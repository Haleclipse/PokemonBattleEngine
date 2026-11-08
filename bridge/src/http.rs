//! Loopback JSON API. There is no account, ladder or matchmaking.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use serde::Deserialize;
use serde_json::{Value, json};
use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::battle::{ActionIn, BattleRoom, ReplacementIn};
use crate::teams::{SetInput, export_team, import_team, sets_from_preset, validate_sets};

static ROOMS: LazyLock<Mutex<HashMap<String, BattleRoom>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);

pub fn serve(port: &str) {
    let address = format!("127.0.0.1:{port}");
    let server = Server::http(&address).unwrap_or_else(|error| panic!("bind {address}: {error}"));
    eprintln!("battle bridge listening on http://{address}");
    for mut request in server.incoming_requests() {
        if *request.method() == Method::Options {
            let _ = request.respond(cors(
                Response::from_string(String::new()).with_status_code(StatusCode(204)),
            ));
            continue;
        }
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);
        let url = request.url().to_string();
        let (status, payload) = dispatch(request.method(), &url, &body);
        let response = cors(Response::from_string(payload).with_status_code(StatusCode(status)));
        let _ = request.respond(response);
    }
}

fn cors<R: std::io::Read>(mut response: Response<R>) -> Response<R> {
    let headers = [
        ("Content-Type", "application/json; charset=utf-8"),
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Headers", "content-type"),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
    ];
    for (name, value) in headers {
        response.add_header(Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("header"));
    }
    response
}

fn dispatch(method: &Method, url: &str, body: &str) -> (u16, String) {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let query = parse_query(query);
    let result = (|| match (method, path) {
        (Method::Get, "/api/health") => Ok(json!({ "ok": true, "service": "battle-bridge" })),
        (Method::Get, "/api/formats") => Ok(json!({ "formats": crate::host().list_formats() })),
        (Method::Get, path) if path.starts_with("/api/formats/") => {
            let id = &path["/api/formats/".len()..];
            crate::host().format_info(id).map(|info| json!(info))
        }
        (Method::Get, "/api/search") => crate::host().search(
            query.get("format").map(String::as_str).unwrap_or("gen9ou"),
            query.get("kind").map(String::as_str).unwrap_or("species"),
            query.get("q").map(String::as_str).unwrap_or(""),
            query
                .get("limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(24),
        ),
        (Method::Get, "/api/species") => crate::host().species_detail(
            query.get("format").map(String::as_str).unwrap_or("gen9ou"),
            query.get("name").map(String::as_str).unwrap_or(""),
        ),
        (Method::Get, "/api/presets") => {
            let format = query.get("format").map(String::as_str).unwrap_or("gen9ou");
            let mut presets = Vec::new();
            for which in ["sample", "mirror", "pikachu", "charizard"] {
                if let Ok(team) = sets_from_preset(format, which) {
                    presets.push(json!({ "id": which, "team": team }));
                }
            }
            Ok(json!({ "format": format, "presets": presets }))
        }
        (Method::Post, "/api/teams/validate") => {
            let body: TeamBody = serde_json::from_str(body).map_err(|error| error.to_string())?;
            validate_sets(crate::host(), &body.format, body.team).map(|check| json!(check))
        }
        (Method::Post, "/api/teams/import") => {
            let body: ImportBody = serde_json::from_str(body).map_err(|error| error.to_string())?;
            let team = import_team(&body.text);
            let check = validate_sets(crate::host(), &body.format, team.clone())?;
            Ok(json!({ "team": team, "check": check }))
        }
        (Method::Post, "/api/teams/export") => {
            let body: ExportBody = serde_json::from_str(body).map_err(|error| error.to_string())?;
            Ok(json!({ "text": export_team(&body.team) }))
        }
        (Method::Post, "/api/battles") => create_battle(body),
        (Method::Get, path) if path.starts_with("/api/battles/") && path.ends_with("/replay") => {
            let id = battle_id(path, "/replay");
            with_room(&id, |room| {
                room.replay(
                    crate::host(),
                    query.get("observer").map(String::as_str).unwrap_or("p1"),
                )
                .map(|frames| json!({ "frames": frames }))
            })
        }
        (Method::Get, path) if path.starts_with("/api/battles/") => {
            let id = path["/api/battles/".len()..].to_string();
            with_room(&id, |room| {
                room.state(
                    crate::host(),
                    query.get("observer").map(String::as_str).unwrap_or("p1"),
                )
            })
        }
        (Method::Post, path) if path.ends_with("/choose") => {
            post_room(path, "/choose", body, |room, body| {
                let body: ChooseBody =
                    serde_json::from_str(body).map_err(|error| error.to_string())?;
                room.choose(crate::host(), &body.observer, body.order)
            })
        }
        (Method::Post, path) if path.ends_with("/sheets") => {
            post_room(path, "/sheets", body, |room, body| {
                let body: ObserverBody =
                    serde_json::from_str(body).map_err(|error| error.to_string())?;
                room.sheets(crate::host(), &body.observer)
            })
        }
        (Method::Post, path) if path.ends_with("/act") => {
            post_room(path, "/act", body, |room, body| {
                let body: ActBody =
                    serde_json::from_str(body).map_err(|error| error.to_string())?;
                room.act(crate::host(), &body.observer, body.actions)
            })
        }
        (Method::Post, path) if path.ends_with("/switch") => {
            post_room(path, "/switch", body, |room, body| {
                let body: SwitchBody =
                    serde_json::from_str(body).map_err(|error| error.to_string())?;
                room.force(crate::host(), &body.observer, body.replacements)
            })
        }
        (Method::Post, path) if path.ends_with("/revive") => {
            post_room(path, "/revive", body, |room, body| {
                let body: ReviveBody =
                    serde_json::from_str(body).map_err(|error| error.to_string())?;
                room.revive(crate::host(), &body.observer, body.pokemon)
            })
        }
        (Method::Post, path) if path.ends_with("/restart") => {
            post_room(path, "/restart", body, |room, body| {
                let body: ObserverBody =
                    serde_json::from_str(body).map_err(|error| error.to_string())?;
                room.restart(crate::host(), &body.observer)
            })
        }
        _ => Err("没有这个接口".into()),
    })();
    match result {
        Ok(value) => (200, value.to_string()),
        Err(error) => (400, json!({ "error": error }).to_string()),
    }
}

fn create_battle(body: &str) -> Result<Value, String> {
    let body: CreateBody = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let room = BattleRoom::create(
        crate::host(),
        &body.format,
        body.seed.unwrap_or(20261108),
        &body.mode,
        body.p1,
        body.p2,
    )?;
    let id = format!("b{:08x}", NEXT.fetch_add(1, Ordering::Relaxed));
    let state = room.state(crate::host(), "p1")?;
    rooms().insert(id.clone(), room);
    Ok(json!({ "id": id, "state": state }))
}

fn with_room(
    id: &str,
    f: impl FnOnce(&mut BattleRoom) -> Result<Value, String>,
) -> Result<Value, String> {
    let mut rooms = rooms();
    let room = rooms.get_mut(id).ok_or("没有这场对战")?;
    f(room)
}

fn post_room(
    path: &str,
    suffix: &str,
    body: &str,
    f: impl FnOnce(&mut BattleRoom, &str) -> Result<Value, String>,
) -> Result<Value, String> {
    let id = battle_id(path, suffix);
    let mut rooms = rooms();
    let room = rooms.get_mut(&id).ok_or("没有这场对战")?;
    f(room, body)
}

fn battle_id(path: &str, suffix: &str) -> String {
    path.trim_start_matches("/api/battles/")
        .trim_end_matches(suffix)
        .trim_end_matches('/')
        .to_string()
}

fn rooms() -> std::sync::MutexGuard<'static, HashMap<String, BattleRoom>> {
    ROOMS.lock().expect("rooms")
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((decode(key), decode(value)))
        })
        .collect()
}

fn decode(text: &str) -> String {
    let text = text.replace('+', " ");
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Deserialize)]
struct TeamBody {
    format: String,
    team: Vec<SetInput>,
}

#[derive(Deserialize)]
struct ImportBody {
    format: String,
    text: String,
}

#[derive(Deserialize)]
struct ExportBody {
    team: Vec<SetInput>,
}

#[derive(Deserialize)]
struct CreateBody {
    format: String,
    #[serde(default)]
    seed: Option<u64>,
    mode: String,
    p1: Vec<SetInput>,
    p2: Vec<SetInput>,
}

#[derive(Deserialize)]
struct ObserverBody {
    observer: String,
}

#[derive(Deserialize)]
struct ChooseBody {
    observer: String,
    #[serde(default)]
    order: Vec<u8>,
}

#[derive(Deserialize)]
struct ActBody {
    observer: String,
    actions: Vec<ActionIn>,
}

#[derive(Deserialize)]
struct SwitchBody {
    observer: String,
    replacements: Vec<ReplacementIn>,
}

#[derive(Deserialize)]
struct ReviveBody {
    observer: String,
    pokemon: u8,
}
