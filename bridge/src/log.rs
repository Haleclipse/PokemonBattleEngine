//! Public battle lines. Hidden numbers (exact foe HP, speeds, silent abilities) stay out.

use pokemon_battle_engine::dex::MoveCategory;
use pokemon_battle_engine::engine::TraceEvent;
use pokemon_battle_engine::view::Observer;
use pokemon_battle_engine::{
    BattleEngine, FieldConditionKind, NonVolatileStatus, PokemonId, SideConditionKind, SideId,
    Stat, VolatileConditionKind, WeatherKind,
};
use pokemon_battle_engine::{PokemonType, TerrainKind};

use pokemon_battle_engine::format::to_id;

use crate::catalog::Host;

pub struct Note {
    pub text: Option<String>,
    pub tone: &'static str,
    pub cue: Option<(u8, &'static str)>,
}

pub fn narrate(
    engine: &BattleEngine,
    observer: Observer,
    event: &TraceEvent,
    host: &Host,
) -> Vec<Note> {
    let mut notes = Vec::new();
    match event {
        TraceEvent::TurnStarted { turn } => {
            notes.push(line(format!("—— 第 {turn} 回合 ——"), "turn", None))
        }
        TraceEvent::LeadSentOut { seen_as, .. } | TraceEvent::Switched { seen_as, .. } => {
            let who = mon(engine, observer, *seen_as, host);
            notes.push(line(
                format!("{who} 出场了"),
                "switch",
                Some((seen_as.0, "Appear")),
            ));
        }
        TraceEvent::MoveUsed { actor, move_id, .. } => {
            let who = mon(engine, observer, *actor, host);
            let move_name = move_label(engine, host, *move_id);
            let clip = match engine
                .dex()
                .move_by_id(*move_id)
                .map(|found| found.category)
            {
                Some(MoveCategory::Status) => "Cast",
                _ => "Attack",
            };
            notes.push(line(
                format!("{who} 使出了{move_name}"),
                "move",
                Some((apparent(engine, observer, *actor).0, clip)),
            ));
        }
        TraceEvent::MoveFailed { actor, move_id, .. } => {
            let who = mon(engine, observer, *actor, host);
            let move_name = move_label(engine, host, *move_id);
            notes.push(line(format!("{who} 的{move_name}失败了"), "fail", None));
        }
        TraceEvent::AccuracyChecked {
            actor, hit: false, ..
        } => {
            notes.push(line(
                format!("{} 的攻击没有命中", mon(engine, observer, *actor, host)),
                "miss",
                None,
            ));
        }
        TraceEvent::DamageCalculated {
            critical,
            type_effectiveness,
            ..
        } => {
            let mut bits = Vec::new();
            if type_effectiveness.numerator == 0 {
                bits.push("没有效果");
            } else if type_effectiveness.numerator > type_effectiveness.denominator {
                bits.push("效果拔群");
            } else if type_effectiveness.numerator < type_effectiveness.denominator {
                bits.push("效果不好");
            }
            if *critical {
                bits.push("击中了要害");
            }
            if !bits.is_empty() {
                notes.push(line(bits.join("，"), "effect", None));
            }
        }
        TraceEvent::DamageApplied {
            target,
            amount,
            hp_before,
            hp_after,
            ..
        } if *amount > 0 => {
            let who = mon(engine, observer, *target, host);
            let shown = if knows(engine, observer, *target) {
                format!("{who} 失去了 {amount} HP（{hp_before}→{hp_after}）")
            } else {
                format!("{who} 受到了伤害")
            };
            notes.push(line(
                shown,
                "damage",
                Some((apparent(engine, observer, *target).0, "Wound")),
            ));
        }
        TraceEvent::HealApplied {
            target,
            amount,
            hp_before,
            hp_after,
            ..
        } if *amount > 0 => {
            let who = mon(engine, observer, *target, host);
            let shown = if knows(engine, observer, *target) {
                format!("{who} 回复了 {amount} HP（{hp_before}→{hp_after}）")
            } else {
                format!("{who} 回复了体力")
            };
            notes.push(line(shown, "heal", None));
        }
        TraceEvent::Fainted { pokemon, .. } => {
            let seen = apparent(engine, observer, *pokemon);
            notes.push(line(
                format!("{} 倒下了", mon(engine, observer, *pokemon, host)),
                "faint",
                Some((seen.0, "Dead")),
            ));
        }
        TraceEvent::StatusApplied { target, status, .. } => {
            notes.push(line(
                format!(
                    "{} {}",
                    mon(engine, observer, *target, host),
                    status_phrase(*status)
                ),
                "status",
                None,
            ));
        }
        TraceEvent::StatusCured { target, .. } => {
            notes.push(line(
                format!("{} 的异常状态解除了", mon(engine, observer, *target, host)),
                "status",
                None,
            ));
        }
        TraceEvent::StatChanged {
            target,
            stat,
            from,
            to,
            ..
        } if from != to => {
            let who = mon(engine, observer, *target, host);
            let dir = if to > from { "提高" } else { "降低" };
            let clip = if to > from {
                Some((apparent(engine, observer, *target).0, "Buff"))
            } else {
                None
            };
            notes.push(line(
                format!("{who} 的{}{dir}了", stat_zh(*stat)),
                "boost",
                clip,
            ));
        }
        TraceEvent::WeatherChanged { to, .. } => {
            let text = match to {
                Some(weather) => format!("天气变成了{}", weather_zh(*weather)),
                None => "天气恢复了".into(),
            };
            notes.push(line(text, "field", None));
        }
        TraceEvent::TerrainChanged { to, .. } => {
            let text = match to {
                Some(terrain) => format!("{}展开了", terrain_zh(*terrain)),
                None => "场地消失了".into(),
            };
            notes.push(line(text, "field", None));
        }
        TraceEvent::SideConditionStarted { side, kind } => {
            notes.push(line(
                format!("{} {}", side_zh(*side), side_zh_cond(*kind)),
                "field",
                None,
            ));
        }
        TraceEvent::FieldConditionStarted { kind } => {
            notes.push(line(field_zh(*kind).to_string(), "field", None));
        }
        TraceEvent::VolatileStarted { target, kind } => {
            if let Some(name) = volatile_zh(*kind) {
                notes.push(line(
                    format!("{} {}", mon(engine, observer, *target, host), name),
                    "volatile",
                    None,
                ));
            }
        }
        TraceEvent::AbilityActivated { source, ability } => {
            let names = engine.names();
            let name = names.ability(*ability).unwrap_or("特性");
            notes.push(line(
                format!(
                    "{} 的{}发动了",
                    mon(engine, observer, *source, host),
                    host.zh_ability(name)
                ),
                "ability",
                None,
            ));
        }
        TraceEvent::ItemActivated { holder, item }
        | TraceEvent::ItemConsumed {
            holder,
            item,
            seen: true,
        } => {
            let names = engine.names();
            let name = names.item(*item).unwrap_or("道具");
            notes.push(line(
                format!(
                    "{} 的{}发动了",
                    mon(engine, observer, *holder, host),
                    host.zh_item(name)
                ),
                "item",
                None,
            ));
        }
        TraceEvent::Terastallized { pokemon, tera_type } => {
            notes.push(line(
                format!(
                    "{} 太晶化成了{}",
                    mon(engine, observer, *pokemon, host),
                    type_zh(*tera_type, host)
                ),
                "gimmick",
                Some((apparent(engine, observer, *pokemon).0, "Buff")),
            ));
        }
        TraceEvent::MegaEvolved { pokemon, ultra, .. } => {
            let word = if *ultra {
                "究极爆发"
            } else {
                "超级进化"
            };
            notes.push(line(
                format!("{} {word}", mon(engine, observer, *pokemon, host)),
                "gimmick",
                Some((apparent(engine, observer, *pokemon).0, "Buff")),
            ));
        }
        TraceEvent::BattleFinished { outcome } => {
            let text = match outcome {
                pokemon_battle_engine::BattleOutcome::Winner(SideId::P1) => "对战结束：P1 获胜",
                pokemon_battle_engine::BattleOutcome::Winner(SideId::P2) => "对战结束：P2 获胜",
                pokemon_battle_engine::BattleOutcome::Draw => "对战结束：平局",
            };
            notes.push(line(text, "end", None));
        }
        _ => {}
    }
    notes
}

fn line(text: impl Into<String>, tone: &'static str, cue: Option<(u8, &'static str)>) -> Note {
    Note {
        text: Some(text.into()),
        tone,
        cue,
    }
}

fn knows(engine: &BattleEngine, observer: Observer, pokemon: PokemonId) -> bool {
    let Some(state) = engine.state().pokemon(pokemon) else {
        return false;
    };
    match observer {
        Observer::Side(side) => state.side == side,
        Observer::Spectator => false,
    }
}

fn apparent(engine: &BattleEngine, observer: Observer, pokemon: PokemonId) -> PokemonId {
    let Some(state) = engine.state().pokemon(pokemon) else {
        return pokemon;
    };
    if knows(engine, observer, pokemon) {
        pokemon
    } else {
        state.illusion.unwrap_or(pokemon)
    }
}

fn mon(engine: &BattleEngine, observer: Observer, pokemon: PokemonId, host: &Host) -> String {
    let shown = apparent(engine, observer, pokemon);
    let view = engine.view(observer);
    for side in [SideId::P1, SideId::P2] {
        for member in view.side(side).team() {
            if member.id() == shown {
                let names = engine.names();
                let english = member
                    .species()
                    .and_then(|species| names.species(species))
                    .unwrap_or("宝可梦");
                let name = host.zh_species(&to_id(english), english);
                return format!("{}{name}", side_tag(side));
            }
        }
    }
    "宝可梦".into()
}

fn side_tag(side: SideId) -> &'static str {
    match side {
        SideId::P1 => "P1 ",
        SideId::P2 => "P2 ",
    }
}

fn move_label(engine: &BattleEngine, host: &Host, id: pokemon_battle_engine::MoveId) -> String {
    let names = engine.names();
    let english = names.move_(id).unwrap_or("招式");
    host.zh_move(english)
}

fn status_phrase(status: NonVolatileStatus) -> &'static str {
    match status {
        NonVolatileStatus::Burn => "灼伤了",
        NonVolatileStatus::Paralysis => "麻痹了",
        NonVolatileStatus::Poison => "中毒了",
        NonVolatileStatus::Toxic => "中了剧毒",
        NonVolatileStatus::Sleep => "睡着了",
        NonVolatileStatus::Freeze => "冻住了",
    }
}

fn stat_zh(stat: Stat) -> &'static str {
    match stat {
        Stat::Attack => "攻击",
        Stat::Defense => "防御",
        Stat::SpecialAttack => "特攻",
        Stat::SpecialDefense => "特防",
        Stat::Speed => "速度",
        Stat::Accuracy => "命中",
        Stat::Evasion => "闪避",
    }
}

pub fn weather_zh(weather: WeatherKind) -> &'static str {
    match weather {
        WeatherKind::Sun => "大晴天",
        WeatherKind::Rain => "下雨",
        WeatherKind::Sandstorm => "沙暴",
        WeatherKind::Snow => "下雪",
        WeatherKind::StrongWinds => "乱流",
    }
}

pub fn terrain_zh(terrain: TerrainKind) -> &'static str {
    match terrain {
        TerrainKind::Electric => "电气场地",
        TerrainKind::Grassy => "青草场地",
        TerrainKind::Psychic => "精神场地",
        TerrainKind::Misty => "薄雾场地",
    }
}

pub fn status_name(status: NonVolatileStatus) -> &'static str {
    match status {
        NonVolatileStatus::Burn => "灼伤",
        NonVolatileStatus::Paralysis => "麻痹",
        NonVolatileStatus::Poison => "中毒",
        NonVolatileStatus::Toxic => "剧毒",
        NonVolatileStatus::Sleep => "睡眠",
        NonVolatileStatus::Freeze => "冰冻",
    }
}

fn side_zh(side: SideId) -> &'static str {
    match side {
        SideId::P1 => "P1",
        SideId::P2 => "P2",
    }
}

fn side_zh_cond(kind: SideConditionKind) -> &'static str {
    match kind {
        SideConditionKind::StealthRock => "撒下了隐形岩",
        SideConditionKind::Spikes => "撒下了撒菱",
        SideConditionKind::ToxicSpikes => "撒下了毒菱",
        SideConditionKind::StickyWeb => "撒下了黏黏网",
        SideConditionKind::Tailwind => "起了顺风",
        SideConditionKind::Reflect => "张开了反射壁",
        SideConditionKind::LightScreen => "张开了光墙",
        SideConditionKind::AuroraVeil => "张开了极光幕",
        SideConditionKind::Safeguard => "张开了神秘守护",
        _ => "场地状态变化了",
    }
}

fn field_zh(kind: FieldConditionKind) -> &'static str {
    match kind {
        FieldConditionKind::TrickRoom => "戏法空间展开了",
        FieldConditionKind::Gravity => "重力加强了",
        FieldConditionKind::MagicRoom => "魔法空间展开了",
        FieldConditionKind::WonderRoom => "奇妙空间展开了",
        _ => "场地发生了变化",
    }
}

pub fn volatile_label(kind: VolatileConditionKind) -> Option<&'static str> {
    volatile_zh(kind)
}

fn volatile_zh(kind: VolatileConditionKind) -> Option<&'static str> {
    Some(match kind {
        VolatileConditionKind::Protect => "守住了",
        VolatileConditionKind::Substitute => "制造了替身",
        VolatileConditionKind::Confusion => "混乱了",
        VolatileConditionKind::Taunt => "被挑衅了",
        VolatileConditionKind::Encore => "被再来一次了",
        VolatileConditionKind::Disable => "的招式被定身了",
        VolatileConditionKind::LeechSeed => "被寄生了",
        VolatileConditionKind::PartiallyTrapped => "被困住了",
        VolatileConditionKind::Ingrain => "扎下了根",
        VolatileConditionKind::MustRecharge => "必须休息",
        _ => return None,
    })
}

pub fn type_zh(kind: PokemonType, host: &Host) -> String {
    let english = match kind {
        PokemonType::Normal => "Normal",
        PokemonType::Fire => "Fire",
        PokemonType::Water => "Water",
        PokemonType::Electric => "Electric",
        PokemonType::Grass => "Grass",
        PokemonType::Ice => "Ice",
        PokemonType::Fighting => "Fighting",
        PokemonType::Poison => "Poison",
        PokemonType::Ground => "Ground",
        PokemonType::Flying => "Flying",
        PokemonType::Psychic => "Psychic",
        PokemonType::Bug => "Bug",
        PokemonType::Rock => "Rock",
        PokemonType::Ghost => "Ghost",
        PokemonType::Dragon => "Dragon",
        PokemonType::Dark => "Dark",
        PokemonType::Steel => "Steel",
        PokemonType::Fairy => "Fairy",
        PokemonType::Stellar => "Stellar",
        PokemonType::Typeless => "???",
    };
    host.zh_type(english)
}
