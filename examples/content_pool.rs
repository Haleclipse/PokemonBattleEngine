//! Prints, as JSON, what a differential campaign may put into a generated battle for one
//! generation (ROADMAP §12.1): the moves the engine supports, the abilities and items it
//! implements, and the species (base formes). `tools/campaign.py` samples teams from it.
//! Every move also carries its mechanic tags (primary effect, secondaries, hit structure,
//! priority, targeting, flags …): the campaign counts the tags of the moves actually used and
//! samples toward the least exercised ones and toward unseen pairs of them.
//!
//! Run: cargo run --example content_pool -- 9
use pokemon_battle_engine::dex::{
    DexResolver, FormeKind, MultiHit, PrimaryEffect, ResolvedMove, SecondaryEffect,
    SecondaryTarget, StatusMoveEffect,
};
use pokemon_battle_engine::spec::Generation;

/// Moves the policy cannot choose like any other (Struggle is what a Pokémon without PP uses).
const NOT_CHOOSABLE: &[&str] = &["Struggle"];

fn main() {
    let generation = match std::env::args().nth(1).as_deref() {
        Some("1") => Generation::Gen1,
        Some("2") => Generation::Gen2,
        Some("3") => Generation::Gen3,
        Some("4") => Generation::Gen4,
        Some("5") => Generation::Gen5,
        Some("6") => Generation::Gen6,
        Some("7") => Generation::Gen7,
        Some("8") => Generation::Gen8,
        _ => Generation::Gen9,
    };
    let dex = DexResolver::resolve(generation);
    let moves: Vec<&str> = dex
        .moves()
        .iter()
        .filter(|m| m.primary_effect != PrimaryEffect::Unsupported)
        .map(|m| m.name.as_str())
        .filter(|name| !NOT_CHOOSABLE.contains(name))
        .collect();
    let abilities: Vec<&str> = dex
        .abilities()
        .iter()
        .filter(|a| a.implemented)
        .map(|a| a.name.as_str())
        .collect();
    let items: Vec<&str> = dex
        .items()
        .iter()
        .filter(|i| i.implemented)
        .map(|i| i.name.as_str())
        .collect();
    let species: Vec<&str> = dex
        .species()
        .iter()
        .filter(|s| s.forme_kind == FormeKind::Base)
        .map(|s| s.name.as_str())
        .collect();
    let move_tags: serde_json::Map<String, serde_json::Value> = dex
        .moves()
        .iter()
        .filter(|m| moves.contains(&m.name.as_str()))
        .map(|m| (m.name.clone(), serde_json::json!(tags(m))))
        .collect();
    let json = serde_json::json!({
        "generation": generation.number(),
        "moves": moves,
        "move_tags": move_tags,
        "abilities": abilities,
        "items": items,
        "species": species,
    });
    println!("{json}");
}

/// The mechanics a move exercises, as lowercase tags.
fn tags(m: &ResolvedMove) -> Vec<String> {
    let mut tags = Vec::new();
    match &m.primary_effect {
        PrimaryEffect::Damage => tags.push(format!("damage:{:?}", m.category)),
        PrimaryEffect::Status(effect) => tags.push(match effect {
            StatusMoveEffect::SetStatus(status) => format!("status:{status:?}"),
            StatusMoveEffect::StatStages(_) => "stat-stages".into(),
            StatusMoveEffect::Volatile(kind) => format!("volatile:{kind:?}"),
            StatusMoveEffect::Heal(_) => "heal".into(),
            StatusMoveEffect::ForceSwitch => "force-switch".into(),
            StatusMoveEffect::Weather(kind) => format!("weather:{kind:?}"),
            StatusMoveEffect::WeatherHeal(_) => "weather-heal".into(),
            StatusMoveEffect::StagesThen(_, follow_up) => format!("stat-stages-then:{follow_up:?}"),
            StatusMoveEffect::TransferStatus => "transfer-status".into(),
            StatusMoveEffect::CureTarget(_) => "cure-target".into(),
            StatusMoveEffect::TargetHeal(_) => "target-heal".into(),
            StatusMoveEffect::Inert => "inert".into(),
        }),
        PrimaryEffect::Custom => tags.push(format!("custom:{}", m.name)),
        PrimaryEffect::Unsupported => {}
    }
    for secondary in &m.secondaries {
        tags.push(match (secondary.target, secondary.effect) {
            (_, SecondaryEffect::Status(status)) => format!("secondary:status:{status:?}"),
            (SecondaryTarget::User, SecondaryEffect::StatStage { .. }) => {
                "secondary:self-stat".into()
            }
            (SecondaryTarget::Target, SecondaryEffect::StatStage { .. }) => {
                "secondary:foe-stat".into()
            }
            (_, SecondaryEffect::Volatile(kind)) => format!("secondary:volatile:{kind:?}"),
            (_, SecondaryEffect::RandomStatus(_)) => "secondary:random-status".into(),
            (_, SecondaryEffect::StealItem) => "secondary:steal-item".into(),
            (_, SecondaryEffect::EerieSpell) => "secondary:eerie-spell".into(),
        });
    }
    let behavior = &m.behavior;
    for (holds, tag) in [
        (m.multi_hit != MultiHit::Single, "multi-hit"),
        (m.recoil.is_some(), "recoil"),
        (m.drain.is_some(), "drain"),
        (m.self_switch.is_some(), "self-switch"),
        (m.force_switch, "force-switch"),
        (m.priority > 0, "priority+"),
        (m.priority < 0, "priority-"),
        (m.is_spread, "spread"),
        (m.flags.contact, "contact"),
        (m.flags.bypass_substitute, "bypass-substitute"),
        (m.flags.powder, "powder"),
        (!m.flags.protectable, "unprotectable"),
        (m.fixed_damage.is_some(), "fixed-damage"),
        (m.ohko.is_some(), "ohko"),
        (m.crit_ratio > 1, "high-crit"),
        (m.will_crit == Some(true), "always-crit"),
        (m.thaws_target || m.flags.defrost, "thaw"),
        (behavior.two_turn.is_some(), "two-turn"),
        (behavior.power_formula.is_some(), "power-formula"),
        (behavior.power_boost.is_some(), "power-boost"),
        (behavior.weather_accuracy.is_some(), "weather-accuracy"),
        (behavior.try_use.is_some(), "try-use"),
    ] {
        if holds {
            tags.push(tag.into());
        }
    }
    tags.push(format!("target:{:?}", m.target));
    let mut tags: Vec<String> = tags.into_iter().map(|tag| tag.to_lowercase()).collect();
    tags.sort();
    tags.dedup();
    tags
}
