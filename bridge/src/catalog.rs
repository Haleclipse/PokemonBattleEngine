//! Format list, Chinese names and the species picker. Suggestions are not legality;
//! `TeamValidator` is.

use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;

use pokemon_battle_engine::format::{FormatLibrary, LegalityData, TeamValidator, to_id};
use pokemon_battle_engine::session::format_switches;
use serde::Deserialize;
use serde::Serialize;

use crate::data_dir;

#[derive(Clone)]
struct Label {
    zh: String,
}

pub struct Host {
    pub library: &'static FormatLibrary,
    species_zh: HashMap<String, Label>,
    move_zh: HashMap<String, Label>,
    ability_zh: HashMap<String, Label>,
    item_zh: HashMap<String, Label>,
    nature_zh: HashMap<String, Label>,
    type_zh: HashMap<String, Label>,
    dex_stats: HashMap<u8, HashMap<String, [i32; 6]>>,
    formats: Mutex<HashMap<String, FormatInfo>>,
    species_rows: Mutex<HashMap<String, Vec<SpeciesRow>>>,
}

#[derive(Clone)]
struct SpeciesRow {
    id: String,
    name: String,
    zh: String,
    num: i32,
    types: Vec<String>,
    abilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatInfo {
    pub id: String,
    pub name: String,
    pub generation: u8,
    pub game_type: String,
    pub min_team: u32,
    pub max_team: u32,
    pub picked: Option<u32>,
    pub preview: bool,
    pub leads: u8,
    pub ev_limit: Option<u32>,
    pub max_moves: u32,
    pub default_level: u32,
    pub tera: bool,
    pub mega: bool,
    pub z_move: bool,
    pub dynamax: bool,
    pub open_team_sheets: String,
}

#[derive(Deserialize)]
struct ZhFile {
    species: HashMap<String, ZhName>,
    #[serde(rename = "move")]
    moves: HashMap<String, ZhName>,
    ability: HashMap<String, ZhName>,
    item: HashMap<String, ZhName>,
    nature: HashMap<String, ZhName>,
    #[serde(rename = "type")]
    types: HashMap<String, ZhName>,
}

#[derive(Deserialize)]
struct ZhName {
    zh: String,
}

#[derive(Deserialize)]
struct DexFile {
    species: Vec<DexSpecies>,
}

#[derive(Deserialize)]
struct DexSpecies {
    id: String,
    #[serde(default)]
    base_stats: Option<RawStats>,
}

#[derive(Deserialize)]
struct RawStats {
    #[serde(default)]
    hp: i32,
    #[serde(default)]
    atk: i32,
    #[serde(default)]
    def: i32,
    #[serde(default)]
    spa: i32,
    #[serde(default)]
    spd: i32,
    #[serde(default)]
    spe: i32,
}

impl Host {
    pub fn load(library: &'static FormatLibrary) -> Self {
        let zh: ZhFile = serde_json::from_str(
            &fs::read_to_string(data_dir().join("dex/names-zh.json")).expect("names-zh"),
        )
        .expect("names-zh json");
        let mut dex_stats = HashMap::new();
        for generation in 1..=9 {
            let path = data_dir().join(format!("dex/gen{generation}.json"));
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(file) = serde_json::from_str::<DexFile>(&text) else {
                continue;
            };
            let mut table = HashMap::new();
            for species in file.species {
                if let Some(stats) = species.base_stats {
                    table.insert(
                        species.id,
                        [
                            stats.hp, stats.atk, stats.def, stats.spa, stats.spd, stats.spe,
                        ],
                    );
                }
            }
            dex_stats.insert(generation, table);
        }
        Self {
            library,
            species_zh: map_labels(zh.species),
            move_zh: map_labels(zh.moves),
            ability_zh: map_labels(zh.ability),
            item_zh: map_labels(zh.item),
            nature_zh: map_labels(zh.nature),
            type_zh: map_labels(zh.types),
            dex_stats,
            formats: Mutex::new(HashMap::new()),
            species_rows: Mutex::new(HashMap::new()),
        }
    }

    pub fn zh_species(&self, showdown_id: &str, english: &str) -> String {
        self.species_zh
            .get(showdown_id)
            .map(|label| label.zh.clone())
            .unwrap_or_else(|| english.to_string())
    }

    pub fn zh_move(&self, english: &str) -> String {
        labeled(&self.move_zh, english)
    }

    pub fn zh_ability(&self, english: &str) -> String {
        labeled(&self.ability_zh, english)
    }

    pub fn zh_item(&self, english: &str) -> String {
        labeled(&self.item_zh, english)
    }

    pub fn zh_type(&self, english: &str) -> String {
        labeled(&self.type_zh, english)
    }

    pub fn list_formats(&self) -> Vec<serde_json::Value> {
        self.library
            .formats()
            .formats
            .iter()
            .filter(|entry| entry.effect_type == "Format")
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "name": entry.name,
                    "generation": entry.generation.unwrap_or(9),
                    "gameType": entry.game_type.clone().unwrap_or_else(|| "singles".into()),
                })
            })
            .collect()
    }

    pub fn format_info(&self, id: &str) -> Result<FormatInfo, String> {
        if let Some(info) = self.formats.lock().expect("formats").get(id).cloned() {
            return Ok(info);
        }
        let validator =
            TeamValidator::new(self.library, id).map_err(|error| format!("{error:?}"))?;
        let (_, rules, policy) = format_switches(validator.format, &validator.table)
            .map_err(|error| format!("{error:?}"))?;
        let numbers = validator.table.numbers;
        let sheets = if validator.table.has("forceopenteamsheets") {
            "forced"
        } else if validator.table.has("openteamsheets") {
            "offered"
        } else {
            "no"
        };
        let info = FormatInfo {
            id: validator.format.id.clone(),
            name: validator.format.name.clone(),
            generation: validator.format.generation.unwrap_or(9),
            game_type: validator
                .format
                .game_type
                .clone()
                .unwrap_or_else(|| "singles".into()),
            min_team: numbers.min_team_size.max(u32::from(policy.min_team_size)),
            max_team: u32::from(policy.max_team_size),
            picked: policy.picked_team_size.map(u32::from),
            preview: policy.preview,
            leads: if validator.format.game_type.as_deref() == Some("doubles") {
                2
            } else {
                1
            },
            ev_limit: numbers.ev_limit,
            max_moves: numbers.max_move_count,
            default_level: numbers.default_level,
            tera: rules.mechanics.tera,
            mega: rules.mechanics.mega,
            z_move: rules.mechanics.z_move,
            dynamax: rules.mechanics.dynamax,
            open_team_sheets: sheets.into(),
        };
        self.formats
            .lock()
            .expect("formats")
            .insert(id.to_string(), info.clone());
        Ok(info)
    }

    pub fn search(
        &self,
        format: &str,
        kind: &str,
        query: &str,
        limit: usize,
    ) -> Result<serde_json::Value, String> {
        let info = self.format_info(format)?;
        let limit = limit.clamp(1, 40);
        match kind {
            "species" => {
                let rows = self.species(format)?;
                let hits: Vec<_> = rows
                    .iter()
                    .filter(|row| species_hit(row, query))
                    .take(limit)
                    .map(|row| {
                        serde_json::json!({
                            "id": row.id,
                            "name": row.name,
                            "nameZh": row.zh,
                            "num": row.num,
                            "types": row.types,
                            "typeZh": row.types.iter().map(|ty| self.zh_type(ty)).collect::<Vec<_>>(),
                            "abilities": row.abilities.iter().map(|ability| serde_json::json!({
                                "name": ability,
                                "nameZh": self.zh_ability(ability),
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                Ok(
                    serde_json::json!({ "kind": "species", "results": hits, "generation": info.generation }),
                )
            }
            "move" => {
                if query.trim().is_empty() {
                    return Ok(serde_json::json!({ "kind": "move", "results": [] }));
                }
                Ok(serde_json::json!({
                    "kind": "move",
                    "results": self.thing_hits(format, "move", query, limit)?,
                }))
            }
            "ability" | "item" | "nature" => {
                if kind != "nature" && query.trim().is_empty() {
                    return Ok(serde_json::json!({ "kind": kind, "results": [] }));
                }
                Ok(serde_json::json!({
                    "kind": kind,
                    "results": self.thing_hits(format, kind, query, if kind == "nature" { 40 } else { limit })?,
                }))
            }
            _ => Err("未知的检索类型".into()),
        }
    }

    pub fn species_detail(&self, format: &str, name: &str) -> Result<serde_json::Value, String> {
        let info = self.format_info(format)?;
        let validator =
            TeamValidator::new(self.library, format).map_err(|error| format!("{error:?}"))?;
        let legality: &LegalityData = validator.legality;
        let found = legality.species_named(validator.aliases, name);
        let Some(species) = found.found() else {
            return Err(format!("没有这个宝可梦：{name}"));
        };
        let abilities: Vec<_> = species
            .abilities
            .values()
            .filter(|ability| !ability.is_empty())
            .map(|ability| {
                serde_json::json!({
                    "name": ability,
                    "nameZh": self.zh_ability(ability),
                })
            })
            .collect();
        let learnsets = self
            .library
            .learnsets(validator.mod_name)
            .map_err(|error| error.to_string())?;
        let generation = info.generation.to_string();
        let mut moves = Vec::new();
        if let Some(sources) = learnsets.data(&species.id).learnset.as_ref() {
            for (move_id, origins) in sources {
                if origins.iter().any(|source| source.starts_with(&generation)) {
                    let english = legality
                        .moves
                        .iter()
                        .find(|row| row.key == *move_id || row.id == *move_id)
                        .map(|row| row.name.clone())
                        .unwrap_or_else(|| move_id.clone());
                    moves.push(serde_json::json!({
                        "id": move_id,
                        "name": english,
                        "nameZh": self.zh_move(&english),
                    }));
                }
            }
        }
        moves.sort_by(|left, right| {
            left["nameZh"]
                .as_str()
                .unwrap_or("")
                .cmp(right["nameZh"].as_str().unwrap_or(""))
        });
        let stats = self
            .dex_stats
            .get(&info.generation)
            .and_then(|table| table.get(&species.id))
            .copied();
        Ok(serde_json::json!({
            "id": species.id,
            "name": species.name,
            "nameZh": self.zh_species(&species.id, &species.name),
            "num": species.num,
            "types": species.types,
            "typeZh": species.types.iter().map(|ty| self.zh_type(ty)).collect::<Vec<_>>(),
            "abilities": abilities,
            "baseStats": stats.map(|[hp, atk, def, spa, spd, spe]| serde_json::json!({
                "hp": hp, "atk": atk, "def": def, "spa": spa, "spd": spd, "spe": spe
            })),
            "moves": moves,
            "note": "学习集只是候选。招式、特性、道具和努力值是否合法，以引擎校验为准。",
        }))
    }

    fn species(&self, format: &str) -> Result<Vec<SpeciesRow>, String> {
        if let Some(rows) = self
            .species_rows
            .lock()
            .expect("species")
            .get(format)
            .cloned()
        {
            return Ok(rows);
        }
        let validator =
            TeamValidator::new(self.library, format).map_err(|error| format!("{error:?}"))?;
        let mut rows = Vec::new();
        for species in &validator.legality.species {
            if !species.exists
                || species.num <= 0
                || species.nonstandard.is_some()
                || species.battle_only.is_some()
                || species.is_cosmetic_forme
            {
                continue;
            }
            let abilities: Vec<String> = species
                .abilities
                .values()
                .filter(|ability| !ability.is_empty())
                .cloned()
                .collect();
            rows.push(SpeciesRow {
                zh: self.zh_species(&species.id, &species.name),
                id: species.id.clone(),
                name: species.name.clone(),
                num: species.num,
                types: species.types.clone(),
                abilities,
            });
        }
        rows.sort_by_key(|row| row.num);
        self.species_rows
            .lock()
            .expect("species")
            .insert(format.to_string(), rows.clone());
        Ok(rows)
    }

    fn thing_hits(
        &self,
        format: &str,
        kind: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, String> {
        let validator =
            TeamValidator::new(self.library, format).map_err(|error| format!("{error:?}"))?;
        let generation = validator.format.generation.unwrap_or(9);
        let rows: &[pokemon_battle_engine::format::ThingFacts] = match kind {
            "move" => &validator.legality.moves,
            "ability" => &validator.legality.abilities,
            "item" => &validator.legality.items,
            "nature" => &validator.legality.natures,
            _ => return Err("未知的检索类型".into()),
        };
        let query_fold = query.trim().to_lowercase();
        let query_id = to_id(query);
        let mut hits = Vec::new();
        for row in rows {
            if kind != "nature" {
                if row.nonstandard.is_some() || row.generation > generation {
                    continue;
                }
                if kind == "move" && (row.z || row.max.is_some()) {
                    continue;
                }
            }
            let zh = match kind {
                "move" => self.zh_move(&row.name),
                "ability" => self.zh_ability(&row.name),
                "item" => self.zh_item(&row.name),
                _ => labeled(&self.nature_zh, &row.name),
            };
            let matched = query_fold.is_empty()
                || row.name.to_lowercase().contains(&query_fold)
                || row.key.contains(&query_id)
                || zh.contains(query.trim());
            if !matched {
                continue;
            }
            hits.push(serde_json::json!({
                "id": row.key,
                "name": row.name,
                "nameZh": zh,
                "type": row.move_type,
            }));
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }
}

fn map_labels(source: HashMap<String, ZhName>) -> HashMap<String, Label> {
    source
        .into_iter()
        .map(|(id, name)| (id, Label { zh: name.zh }))
        .collect()
}

fn labeled(table: &HashMap<String, Label>, english: &str) -> String {
    table
        .get(&to_id(english))
        .map(|label| label.zh.clone())
        .filter(|zh| !zh.is_empty())
        .unwrap_or_else(|| english.to_string())
}

fn species_hit(row: &SpeciesRow, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let fold = query.to_lowercase();
    row.name.to_lowercase().contains(&fold)
        || row.id.contains(&to_id(query))
        || row.zh.contains(query)
        || row.num.to_string() == query
}
