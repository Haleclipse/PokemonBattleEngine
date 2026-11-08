//! Showdown paste import and export. Legality is the engine's validator, not this parser.

use pokemon_battle_engine::format::{Problem, SetField, TeamSet, TeamValidator};
use pokemon_battle_engine::{SessionError, Stats};
use serde::{Deserialize, Serialize};

use crate::catalog::Host;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatBlock {
    #[serde(default)]
    pub hp: i32,
    #[serde(default)]
    pub atk: i32,
    #[serde(default)]
    pub def: i32,
    #[serde(default)]
    pub spa: i32,
    #[serde(default)]
    pub spd: i32,
    #[serde(default)]
    pub spe: i32,
}

impl From<Stats> for StatBlock {
    fn from(stats: Stats) -> Self {
        Self {
            hp: stats.hp,
            atk: stats.atk,
            def: stats.def,
            spa: stats.spa,
            spd: stats.spd,
            spe: stats.spe,
        }
    }
}

impl From<StatBlock> for Stats {
    fn from(stats: StatBlock) -> Self {
        Self {
            hp: stats.hp,
            atk: stats.atk,
            def: stats.def,
            spa: stats.spa,
            spd: stats.spd,
            spe: stats.spe,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub species: String,
    #[serde(default)]
    pub item: String,
    #[serde(default)]
    pub ability: String,
    #[serde(default)]
    pub moves: Vec<String>,
    #[serde(default)]
    pub nature: String,
    #[serde(default)]
    pub gender: String,
    #[serde(default)]
    pub evs: Option<StatBlock>,
    #[serde(default)]
    pub ivs: Option<StatBlock>,
    #[serde(default)]
    pub level: u32,
    #[serde(default)]
    pub shiny: bool,
    #[serde(default)]
    pub happiness: Option<f64>,
    #[serde(default)]
    pub hp_type: String,
    #[serde(default)]
    pub gigantamax: bool,
    #[serde(default)]
    pub tera_type: String,
}

impl From<SetInput> for TeamSet {
    fn from(set: SetInput) -> Self {
        Self {
            name: set.name,
            species: set.species,
            item: set.item,
            ability: set.ability,
            moves: set.moves,
            nature: set.nature,
            gender: set.gender,
            evs: set.evs.map(Stats::from),
            ivs: set.ivs.map(Stats::from),
            level: set.level,
            shiny: set.shiny,
            happiness: set.happiness,
            hp_type: set.hp_type,
            gigantamax: set.gigantamax,
            tera_type: set.tera_type,
        }
    }
}

impl From<TeamSet> for SetInput {
    fn from(set: TeamSet) -> Self {
        Self {
            name: set.name,
            species: set.species,
            item: set.item,
            ability: set.ability,
            moves: set.moves,
            nature: set.nature,
            gender: set.gender,
            evs: set.evs.map(StatBlock::from),
            ivs: set.ivs.map(StatBlock::from),
            level: set.level,
            shiny: set.shiny,
            happiness: set.happiness,
            hp_type: set.hp_type,
            gigantamax: set.gigantamax,
            tera_type: set.tera_type,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemOut {
    pub message: String,
    pub set: Option<usize>,
    pub field: Option<String>,
}

impl From<&Problem> for ProblemOut {
    fn from(problem: &Problem) -> Self {
        Self {
            message: problem.message.clone(),
            set: problem.location.set,
            field: problem.location.field.map(|field| match field {
                SetField::Name => "name".to_string(),
                SetField::Species => "species".to_string(),
                SetField::Item => "item".to_string(),
                SetField::Ability => "ability".to_string(),
                SetField::Nature => "nature".to_string(),
                SetField::Moves => "moves".to_string(),
                SetField::Move(index) => format!("move:{index}"),
                SetField::Evs => "evs".to_string(),
                SetField::Ivs => "ivs".to_string(),
                SetField::Level => "level".to_string(),
                SetField::Gender => "gender".to_string(),
                SetField::Happiness => "happiness".to_string(),
                SetField::HpType => "hpType".to_string(),
                SetField::TeraType => "teraType".to_string(),
                SetField::Origin => "origin".to_string(),
            }),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamCheck {
    pub ok: bool,
    pub problems: Vec<ProblemOut>,
    pub team: Vec<SetInput>,
}

pub fn validate_sets(host: &Host, format: &str, sets: Vec<SetInput>) -> Result<TeamCheck, String> {
    let validator =
        TeamValidator::new(host.library, format).map_err(|error| format!("{error:?}"))?;
    let mut team: Vec<TeamSet> = sets.into_iter().map(TeamSet::from).collect();
    let problems = validator.validate_team(&mut team);
    Ok(TeamCheck {
        ok: problems.is_empty(),
        problems: problems.iter().map(ProblemOut::from).collect(),
        team: team.into_iter().map(SetInput::from).collect(),
    })
}

pub fn session_team_error(error: &SessionError) -> Option<Vec<ProblemOut>> {
    match error {
        SessionError::Team { problems, .. } => {
            Some(problems.iter().map(ProblemOut::from).collect())
        }
        _ => None,
    }
}

pub fn export_team(sets: &[SetInput]) -> String {
    sets.iter().map(export_set).collect::<Vec<_>>().join("\n\n")
}

fn export_set(set: &SetInput) -> String {
    let mut lines = Vec::new();
    let mut header = if set.name.is_empty() {
        set.species.clone()
    } else {
        format!("{} ({})", set.name, set.species)
    };
    if matches!(set.gender.as_str(), "M" | "F") {
        header.push_str(&format!(" ({})", set.gender));
    }
    if !set.item.is_empty() {
        header.push_str(" @ ");
        header.push_str(&set.item);
    }
    lines.push(header);
    if !set.ability.is_empty() {
        lines.push(format!("Ability: {}", set.ability));
    }
    if set.level > 0 {
        lines.push(format!("Level: {}", set.level));
    }
    if set.shiny {
        lines.push("Shiny: Yes".into());
    }
    if set.gigantamax {
        lines.push("Gigantamax: Yes".into());
    }
    if let Some(happiness) = set.happiness {
        lines.push(format!("Happiness: {happiness}"));
    }
    if let Some(evs) = &set.evs {
        if let Some(line) = stat_line("EVs", evs) {
            lines.push(line);
        }
    }
    if let Some(ivs) = &set.ivs {
        if let Some(line) = stat_line("IVs", ivs) {
            lines.push(line);
        }
    }
    if !set.nature.is_empty() {
        lines.push(format!("{} Nature", set.nature));
    }
    if !set.hp_type.is_empty() {
        lines.push(format!("Hidden Power: {}", set.hp_type));
    }
    if !set.tera_type.is_empty() {
        lines.push(format!("Tera Type: {}", set.tera_type));
    }
    for move_name in &set.moves {
        if !move_name.is_empty() {
            lines.push(format!("- {move_name}"));
        }
    }
    lines.join("\n")
}

fn stat_line(label: &str, stats: &StatBlock) -> Option<String> {
    let parts = [
        (stats.hp, "HP"),
        (stats.atk, "Atk"),
        (stats.def, "Def"),
        (stats.spa, "SpA"),
        (stats.spd, "SpD"),
        (stats.spe, "Spe"),
    ]
    .into_iter()
    .filter(|(value, _)| *value != 0)
    .map(|(value, name)| format!("{value} {name}"))
    .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(format!("{label}: {}", parts.join(" / ")))
    }
}

pub fn import_team(text: &str) -> Vec<SetInput> {
    let text = text.replace("\r\n", "\n");
    text.split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(parse_block)
        .filter(|set| !set.species.is_empty() || !set.moves.is_empty())
        .collect()
}

fn parse_block(block: &str) -> SetInput {
    let mut set = SetInput {
        name: String::new(),
        species: String::new(),
        item: String::new(),
        ability: String::new(),
        moves: Vec::new(),
        nature: String::new(),
        gender: String::new(),
        evs: None,
        ivs: None,
        level: 0,
        shiny: false,
        happiness: None,
        hp_type: String::new(),
        gigantamax: false,
        tera_type: String::new(),
    };
    let mut lines = block.lines().map(str::trim).filter(|line| !line.is_empty());
    if let Some(header) = lines.next() {
        parse_header(header, &mut set);
    }
    for line in lines {
        if let Some(move_name) = line.strip_prefix("- ") {
            set.moves.push(move_name.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Ability:") {
            set.ability = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("Level:") {
            set.level = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("Happiness:") {
            set.happiness = value.trim().parse().ok();
        } else if line.eq_ignore_ascii_case("Shiny: Yes") {
            set.shiny = true;
        } else if line.eq_ignore_ascii_case("Gigantamax: Yes") {
            set.gigantamax = true;
        } else if let Some(value) = line.strip_prefix("Tera Type:") {
            set.tera_type = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("Hidden Power:") {
            set.hp_type = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("EVs:") {
            set.evs = Some(parse_stats(value));
        } else if let Some(value) = line.strip_prefix("IVs:") {
            set.ivs = Some(parse_stats(value));
        } else if let Some(nature) = line.strip_suffix(" Nature") {
            set.nature = nature.trim().to_string();
        } else if let Some(nature) = line.strip_prefix("Nature:") {
            set.nature = nature.trim().to_string();
        }
    }
    set
}

fn parse_header(line: &str, set: &mut SetInput) {
    let (left, item) = match line.split_once('@') {
        Some((left, item)) => (left.trim(), item.trim().to_string()),
        None => (line.trim(), String::new()),
    };
    set.item = item;
    let (left, gender) = strip_gender(left);
    set.gender = gender;
    let (name, species) = split_nickname(left);
    set.name = name;
    set.species = species;
}

fn strip_gender(line: &str) -> (&str, String) {
    let line = line.trim();
    for (suffix, gender) in [(" (M)", "M"), (" (F)", "F")] {
        if let Some(rest) = line.strip_suffix(suffix) {
            return (rest.trim(), gender.to_string());
        }
    }
    (line, String::new())
}

fn split_nickname(line: &str) -> (String, String) {
    let line = line.trim();
    if let Some(open) = line.rfind('(')
        && line.ends_with(')')
        && open > 0
    {
        let name = line[..open].trim();
        let species = line[open + 1..line.len() - 1].trim();
        if !name.is_empty() && !species.is_empty() {
            return (name.to_string(), species.to_string());
        }
    }
    (String::new(), line.to_string())
}

fn parse_stats(text: &str) -> StatBlock {
    let mut stats = StatBlock {
        hp: 0,
        atk: 0,
        def: 0,
        spa: 0,
        spd: 0,
        spe: 0,
    };
    for part in text.split('/') {
        let mut tokens = part.split_whitespace();
        let Some(value) = tokens.next().and_then(|token| token.parse::<i32>().ok()) else {
            continue;
        };
        let Some(name) = tokens.next() else {
            continue;
        };
        match name {
            "HP" => stats.hp = value,
            "Atk" => stats.atk = value,
            "Def" => stats.def = value,
            "SpA" => stats.spa = value,
            "SpD" => stats.spd = value,
            "Spe" => stats.spe = value,
            _ => {}
        }
    }
    stats
}

pub fn sets_from_preset(format: &str, which: &str) -> Result<Vec<SetInput>, String> {
    let team = crate::presets::preset_team(format, which)
        .ok_or_else(|| "没有该赛制的样本队伍".to_string())?;
    Ok(team.into_iter().map(SetInput::from).collect())
}
