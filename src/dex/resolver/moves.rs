//! Moves: the typed move model, its generation's behavior and its support classification
//! (DESIGN.md §22.6-22.8, §25.1, §27.2).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::data::{EffectSnapshot, MoveSnapshot};
use crate::dex::{
    EffectChance, FixedDamage, FollowUp, MoveCategory, MoveFlags, MultiHit, Ohko, PokemonType,
    PrimaryEffect, Recoil, ResolvedMove, SecondaryEffect, SecondaryEffectSpec, SecondaryTarget,
    SecondaryTrigger, SelfDestruct, SelfSwitch, StatSources, StatusMoveEffect,
};
use crate::effects;
use crate::ids::MoveId;
use crate::rules::{Fraction, GenerationCapabilities, TargetPattern};
use crate::state::{NonVolatileStatus, PassKind, Stat, VolatileConditionKind, WeatherKind};

/// Maps one snapshot move onto the engine's typed move model and classifies its support.
///
/// The rule is conservative on purpose: anything Showdown declares that the engine does not
/// consume (a callback, a mechanic key, an unknown target pattern, a secondary it cannot express)
/// makes the move `Unsupported`. A custom primary consumes exactly the callbacks and keys it
/// names (`CustomPrimary::{source_callbacks, source_keys}`); anything else keeps the move
/// `Unsupported` like any other.
pub(super) fn resolve_move(
    snapshot: &MoveSnapshot,
    capabilities: &GenerationCapabilities,
    items: &BTreeMap<String, crate::ids::ItemId>,
) -> Option<ResolvedMove> {
    let id = MoveId(snapshot.eid);
    let move_type = PokemonType::from_name(&snapshot.type_name)?;
    let category = match snapshot.category.as_str() {
        "Physical" => MoveCategory::Physical,
        "Special" => MoveCategory::Special,
        "Status" => MoveCategory::Status,
        _ => return None,
    };
    let behavior = effects::moves::definition(id).restricted_to(&snapshot.declared_callbacks());
    let mut unsupported = Vec::new();
    let custom = effects::moves::custom(id);

    let (target, is_spread) = match parse_target(&snapshot.target) {
        Some(target) => target,
        None => {
            unsupported.push(format!("target:{}", snapshot.target));
            (TargetPattern::SelectedAdjacent, false)
        }
    };

    let mut secondaries = Vec::new();
    // A secondary's `self` part rolls with the secondaries, hit by hit; the move's own `self`
    // lands once after the move.
    for secondary in &snapshot.secondaries {
        // A secondary that is only a chance and an `onHit` the behavior spells out.
        if let Some(on_hit) = behavior.secondary_on_hit
            && secondary.callbacks == ["onHit"]
            && secondary.status.is_none()
            && secondary.boosts.is_none()
            && secondary.volatile_status.is_none()
            && secondary.self_effect.is_none()
            && secondary.other.is_empty()
        {
            secondaries.push(SecondaryEffectSpec {
                chance: secondary
                    .chance
                    .map_or(EffectChance::CERTAIN, EffectChance::percent),
                target: SecondaryTarget::Target,
                trigger: SecondaryTrigger::PerTargetHit,
                effect: on_hit.effect,
                requires: on_hit.requires,
                shares_roll: false,
                in_secondaries: true,
                from_item: false,
            });
            continue;
        }
        // A secondary's `self` callback a behavior component stands for (`self:<name>`:
        // Genesis Supernova's `self.onHit` laying its terrain) is theirs, as the move's own
        // `self` is.
        let mut secondary = secondary.clone();
        if let Some(inner) = &mut secondary.self_effect {
            let covered = behavior.covered_callbacks();
            inner
                .callbacks
                .retain(|callback| !covered.contains(&format!("self:{callback}").as_str()));
        }
        let empty_self = secondary.self_effect.as_ref().is_some_and(|inner| {
            inner.chance.is_none()
                && inner.status.is_none()
                && inner.boosts.is_none()
                && inner.volatile_status.is_none()
                && inner.self_effect.is_none()
                && inner.callbacks.is_empty()
                && inner.other.is_empty()
        });
        if empty_self {
            secondary.self_effect = None;
        }
        // A secondary left with nothing but its chance has nothing to roll: the behavior
        // component that covered its `self` is what the move does (Genesis Supernova). A
        // chance below 100 % would still be a decision of its own, so it stays.
        if secondary.callbacks.is_empty()
            && secondary.status.is_none()
            && secondary.boosts.is_none()
            && secondary.volatile_status.is_none()
            && secondary.self_effect.is_none()
            && secondary.other.is_empty()
            && secondary.chance.is_none_or(|chance| chance >= 100)
        {
            continue;
        }
        resolve_effect_table(
            &secondary,
            "secondary",
            true,
            SecondaryTrigger::PerTargetHit,
            &mut secondaries,
            &mut unsupported,
        );
    }
    // A `self` callback a behavior component or the custom primary stands for (`self:<name>`:
    // Gen 2-4 Rapid Spin's `onHit`, Baton Pass's) is theirs; a table left with nothing else in
    // it has nothing to resolve. Psycho Shift's `self` (`onHit`: the user's cure) belongs to its
    // status transfer.
    let self_effect = snapshot
        .self_effect
        .as_ref()
        .filter(|_| !behavior.status_transfer)
        .and_then(|self_effect| {
            let mut covered = behavior.covered_callbacks();
            if let Some(primary) = custom {
                covered.extend(primary.source_callbacks);
            }
            let mut inner = self_effect.clone();
            inner
                .callbacks
                .retain(|callback| !covered.contains(&format!("self:{callback}").as_str()));
            // Gen 1's binding lock is the behavior's (`MoveBehavior::binding_lock`), and so is
            // Thrash's (`MoveBehavior::rampage`, §15.12) and Uproar's (`MoveBehavior::uproar`).
            if (behavior.binding_lock
                && inner.volatile_status.as_deref() == Some("partialtrappinglock"))
                || (behavior.rampage && inner.volatile_status.as_deref() == Some("lockedmove"))
                || (behavior.uproar && inner.volatile_status.as_deref() == Some("uproar"))
                || (behavior.rage && inner.volatile_status.as_deref() == Some("rage"))
                || (behavior.glaive_rush && inner.volatile_status.as_deref() == Some("glaiverush"))
            {
                inner.volatile_status = None;
            }
            let empty = inner.chance.is_none()
                && inner.status.is_none()
                && inner.boosts.is_none()
                && inner.volatile_status.is_none()
                && inner.self_effect.is_none()
                && inner.callbacks.is_empty()
                && inner.other.is_empty();
            (!empty).then_some(inner)
        });
    if let Some(self_effect) = &self_effect {
        // Showdown's `self` is a user-directed effect once the move lands (`selfDrops`): certain,
        // unless its boosts carry a chance of their own (Gen 7+ Diamond Storm), rolled then.
        let mut inner = self_effect.clone();
        let chance = match inner.chance {
            Some(chance) if inner.boosts.is_some() => {
                inner.chance = None;
                chance
            }
            _ => 100,
        };
        let table = EffectSnapshot {
            chance: Some(chance),
            self_effect: Some(Box::new(inner)),
            ..EffectSnapshot::default()
        };
        resolve_effect_table(
            &table,
            "self",
            false,
            SecondaryTrigger::OnceAfterMove,
            &mut secondaries,
            &mut unsupported,
        );
    }
    // Gen 4-5 Chatter: its secondaries need the user's species.
    if let Some(species) = behavior.secondaries_need_user_species {
        for secondary in &mut secondaries {
            secondary.requires = Some(crate::dex::SecondaryRequirement::UserSpecies(species));
        }
    }
    let mut other = snapshot.other.clone();
    // Plasma Fists' `pseudoWeather` is its `starts_ion_deluge`.
    if behavior.starts_ion_deluge
        && other.get("pseudoWeather").and_then(Value::as_str) == Some("iondeluge")
    {
        other.remove("pseudoWeather");
    }
    // A move whose secondary or `self` adds the volatile its own `condition` describes (Salt
    // Cure, Syrup Bomb, Throat Chop; Roost): the condition is that volatile's lifecycle.
    let own_condition = match snapshot.id.as_str() {
        "saltcure" => Some(VolatileConditionKind::SaltCure),
        "syrupbomb" => Some(VolatileConditionKind::SyrupBomb),
        "throatchop" => Some(VolatileConditionKind::ThroatChop),
        "roost" => Some(VolatileConditionKind::Roost),
        _ => None,
    };
    if own_condition.is_some_and(|kind| {
        secondaries
            .iter()
            .any(|secondary| secondary.effect == SecondaryEffect::Volatile(kind))
    }) {
        other.remove("condition");
    }
    // `selfBoost` lands on the user once the move is over, whatever the hit count.
    if let Some(value) = other.remove("selfBoost") {
        match serde_json::from_value::<EffectSnapshot>(value) {
            Ok(self_boost) => resolve_effect_table(
                &EffectSnapshot {
                    self_effect: Some(Box::new(self_boost)),
                    ..EffectSnapshot::default()
                },
                "selfBoost",
                false,
                SecondaryTrigger::AfterMove,
                &mut secondaries,
                &mut unsupported,
            ),
            Err(_) => unsupported.push("mechanic:selfBoost".into()),
        }
    }

    // `ignoreImmunity` is a fact about the hit gate, not an unsupported mechanic: damaging moves
    // check the type chart unless it is `true`; status moves only when it is `false`.
    let type_immunity = match other.remove("ignoreImmunity") {
        None => category != MoveCategory::Status,
        Some(Value::Bool(ignore)) => !ignore,
        // Thousand Arrows' `{Ground: true}`: its own effectiveness step reaches the airborne.
        Some(Value::Object(map))
            if behavior.effectiveness
                == Some(effects::moves::TypeEffectiveness::GroundReachesTheAirborne)
                && map.len() == 1
                && map.get("Ground") == Some(&Value::Bool(true)) =>
        {
            true
        }
        Some(_) => {
            unsupported.push("mechanic:ignoreImmunity".into());
            category != MoveCategory::Status
        }
    };

    // `selfSwitch: true` (U-turn, Volt Switch, Flip Turn); Baton Pass's "copyvolatile" and
    // Shed Tail's "shedtail" also hand things on to the replacement. A custom primary that
    // consumes `selfSwitch` (Parting Shot, Revival Blessing) sends its user out itself, or not.
    let self_switch = match other.remove("selfSwitch") {
        Some(_) if custom.is_some_and(|primary| primary.source_keys.contains(&"selfSwitch")) => {
            None
        }
        None => None,
        Some(Value::Bool(true)) => Some(SelfSwitch::Plain),
        Some(Value::String(kind)) if kind == "copyvolatile" => {
            Some(SelfSwitch::Pass(PassKind::BatonPass))
        }
        Some(Value::String(kind)) if kind == "shedtail" => {
            Some(SelfSwitch::Pass(PassKind::ShedTail))
        }
        Some(other_value) => {
            unsupported.push(format!("mechanic:selfSwitch:{other_value}"));
            None
        }
    };

    // Hit sequence: multi-hit count, recoil and drain are declarative data.
    let multi_hit = match other.remove("multihit") {
        None => MultiHit::Single,
        Some(Value::Number(n)) => match n.as_u64() {
            Some(count @ 1..=10) => MultiHit::Fixed(count as u8),
            _ => {
                unsupported.push(format!("mechanic:multihit:{n}"));
                MultiHit::Single
            }
        },
        Some(Value::Array(range)) => match range.as_slice() {
            [min, max]
                if min.as_u64().is_some_and(|m| (1..=10).contains(&m))
                    && max.as_u64().is_some_and(|m| (1..=10).contains(&m))
                    && min.as_u64() < max.as_u64() =>
            {
                MultiHit::Range {
                    min: min.as_u64().unwrap_or(1) as u8,
                    max: max.as_u64().unwrap_or(1) as u8,
                }
            }
            _ => {
                unsupported.push(format!("mechanic:multihit:{}", Value::Array(range)));
                MultiHit::Single
            }
        },
        Some(other_value) => {
            unsupported.push(format!("mechanic:multihit:{other_value}"));
            MultiHit::Single
        }
    };
    // Triple Kick / Triple Axel / Population Bomb roll accuracy per hit (Gen 2's Triple Kick
    // says `false`).
    let multi_accuracy = match other.remove("multiaccuracy") {
        None => false,
        Some(Value::Bool(multi)) => multi,
        Some(other_value) => {
            unsupported.push(format!("mechanic:multiaccuracy:{other_value}"));
            false
        }
    };
    let breaks_protect = match other.remove("breaksProtect") {
        None => false,
        Some(Value::Bool(breaks)) => breaks,
        Some(other_value) => {
            unsupported.push(format!("mechanic:breaksProtect:{other_value}"));
            false
        }
    };
    let steals_boosts = match other.remove("stealsBoosts") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:stealsBoosts:{other_value}"));
            false
        }
    };
    let non_ghost_target = match other.remove("nonGhostTarget") {
        None => None,
        Some(Value::String(pattern)) => match parse_target(&pattern) {
            Some((pattern, _)) => Some(pattern),
            None => {
                unsupported.push(format!("mechanic:nonGhostTarget:{pattern}"));
                None
            }
        },
        Some(other_value) => {
            unsupported.push(format!("mechanic:nonGhostTarget:{other_value}"));
            None
        }
    };
    let tracks_target = match other.remove("tracksTarget") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:tracksTarget:{other_value}"));
            false
        }
    };
    // Read, not taken: a custom primary consumes the key below (a move with none keeps it as an
    // unsupported reason).
    let calls_move = other.get("callsMove") == Some(&Value::Bool(true));
    let smart_target = match other.remove("smartTarget") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:smartTarget:{other_value}"));
            false
        }
    };
    let struggle_recoil = other.remove("struggleRecoil") == Some(Value::Bool(true));
    let sheer_force_boost = other.remove("hasSheerForceBoost") == Some(Value::Bool(true));
    // Mind Blown / Steel Beam: `mindBlownRecoil` marks the behavior's after-move self damage.
    if behavior.self_damage_after_move.is_some() {
        other.remove("mindBlownRecoil");
    }
    // High Jump Kick …: `hasCrashDamage` marks the behavior's crash on failure.
    if behavior.crash_damage {
        other.remove("hasCrashDamage");
    }
    let recoil = match behavior.max_hp_recoil {
        Some(fraction) => Some(Recoil::UserMaxHp(fraction)),
        None if struggle_recoil => Some(Recoil::Struggle),
        None => parse_fraction(&mut other, "recoil", &mut unsupported).map(Recoil::Dealt),
    };
    let drain = parse_fraction(&mut other, "drain", &mut unsupported);
    // A damaging move's own `volatileStatus` (Bind, Wrap, Fire Spin …: `partiallytrapped`; Smack
    // Down, Thousand Arrows: `smackdown`, whose condition is `BattleContext::smack_down`) is
    // applied after each hit that reaches the target, like a certain secondary; other top-level
    // volatiles stay unmodelled.
    let damaging_volatile = match other.get("volatileStatus").and_then(Value::as_str) {
        Some("partiallytrapped") => Some(VolatileConditionKind::PartiallyTrapped),
        Some("smackdown") => Some(VolatileConditionKind::SmackDown),
        _ => None,
    };
    if category != MoveCategory::Status
        && let Some(kind) = damaging_volatile
    {
        other.remove("volatileStatus");
        if kind == VolatileConditionKind::SmackDown {
            other.remove("condition");
        }
        secondaries.push(SecondaryEffectSpec {
            chance: EffectChance::CERTAIN,
            target: SecondaryTarget::Target,
            trigger: SecondaryTrigger::PerTargetHit,
            effect: SecondaryEffect::Volatile(kind),
            requires: None,
            shares_roll: false,
            in_secondaries: false,
            from_item: false,
        });
    }

    // Damage-formula data (M7, DESIGN.md §22.6): critical-hit ratio, fixed damage, one-hit
    // KOs, stat sources, thawing.
    let crit_ratio = match other.remove("critRatio") {
        None => 1,
        Some(Value::Number(n)) if n.as_u64().is_some_and(|r| r <= 5) => {
            n.as_u64().unwrap_or(1) as u8
        }
        Some(other_value) => {
            unsupported.push(format!("mechanic:critRatio:{other_value}"));
            1
        }
    };
    let will_crit = match other.remove("willCrit") {
        None => None,
        Some(Value::Bool(will)) => Some(will),
        Some(other_value) => {
            unsupported.push(format!("mechanic:willCrit:{other_value}"));
            None
        }
    };
    let no_damage_variance = match other.remove("noDamageVariance") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:noDamageVariance:{other_value}"));
            false
        }
    };
    // Gen 1 Roar / Whirlwind: the generation takes the phazing away (`forceSwitch: false`)
    // and leaves nothing — a move that does nothing (`StatusMoveEffect::Inert`, §15.17).
    let force_switch_taken_away = other.get("forceSwitch") == Some(&Value::Bool(false));
    // An explicit `false` on a sparse key (a generation's override) is the key's absence.
    for key in ["forceSwitch", "thawsTarget", "struggleRecoil"] {
        if other.get(key) == Some(&Value::Bool(false)) {
            other.remove(key);
        }
    }
    let mut fixed_damage = match other.remove("damage") {
        None => None,
        Some(Value::String(level)) if level == "level" => Some(FixedDamage::Level),
        Some(Value::Number(n)) if n.as_u64().is_some_and(|d| (1..=999).contains(&d)) => {
            Some(FixedDamage::Amount(n.as_u64().unwrap_or(1) as u16))
        }
        Some(other_value) => {
            unsupported.push(format!("mechanic:damage:{other_value}"));
            None
        }
    };
    if let Some(callback) = behavior.damage_override {
        fixed_damage = Some(callback);
    }
    let self_destruct = match other.remove("selfdestruct") {
        None => None,
        Some(Value::String(when)) if when == "always" => Some(SelfDestruct::Always),
        Some(Value::String(when)) if when == "ifHit" => Some(SelfDestruct::IfHit),
        Some(other_value) => {
            unsupported.push(format!("mechanic:selfdestruct:{other_value}"));
            None
        }
    };
    let ohko = match other.remove("ohko") {
        None => None,
        Some(Value::Bool(true)) => Some(Ohko { immune_type: None }),
        Some(Value::String(type_name)) => match PokemonType::from_name(&type_name) {
            Some(immune) => Some(Ohko {
                immune_type: Some(immune),
            }),
            None => {
                unsupported.push(format!("mechanic:ohko:{type_name}"));
                None
            }
        },
        Some(other_value) => {
            unsupported.push(format!("mechanic:ohko:{other_value}"));
            None
        }
    };
    let mut stat_sources = StatSources::default();
    match other.remove("overrideOffensivePokemon") {
        None => {}
        Some(Value::String(who)) if who == "target" => stat_sources.offense_from_target = true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:overrideOffensivePokemon:{other_value}"))
        }
    }
    for (key, slot) in [
        ("overrideOffensiveStat", &mut stat_sources.offense_stat),
        ("overrideDefensiveStat", &mut stat_sources.defense_stat),
    ] {
        match other.remove(key) {
            None => {}
            Some(Value::String(stat)) => match parse_stat(&stat) {
                Some(stat) if !matches!(stat, Stat::Accuracy | Stat::Evasion) => *slot = Some(stat),
                _ => unsupported.push(format!("mechanic:{key}:{stat}")),
            },
            Some(other_value) => unsupported.push(format!("mechanic:{key}:{other_value}")),
        }
    }
    for (key, slot) in [
        ("ignoreDefensive", &mut stat_sources.ignore_defensive_stage),
        ("ignoreEvasion", &mut stat_sources.ignore_evasion),
    ] {
        match other.remove(key) {
            None => {}
            Some(Value::Bool(value)) => *slot = value,
            Some(other_value) => unsupported.push(format!("mechanic:{key}:{other_value}")),
        }
    }
    let ignores_ability = match other.remove("ignoreAbility") {
        None => false,
        Some(Value::Bool(true)) => true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:ignoreAbility:{other_value}"));
            false
        }
    };
    // `sleepUsable` (Snore, Sleep Talk): the sleeping user's BeforeMove lets it go ahead.
    let sleep_usable = match other.remove("sleepUsable") {
        None => false,
        Some(Value::Bool(true)) => true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:sleepUsable:{other_value}"));
            false
        }
    };
    let thaws_target = match other.remove("thawsTarget") {
        None => false,
        Some(Value::Bool(true)) => true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:thawsTarget:{other_value}"));
            false
        }
    };

    // A sure-hit rule for the user's type (Toxic): Gen 6-7 implement it as an `onPrepareHit`
    // volatile (`condition`), Gen 3-5 snapshots keep that key without the callback; both are
    // `MoveBehavior::sure_hit_user_type` + `ConditionRules::typed_sure_hit`.
    if behavior.sure_hit_user_type.is_some() {
        other.remove("condition");
    }
    // Echoed Voice's `condition` is the field condition `FieldState::echo` keeps; Gen 3-4 Beat
    // Up's is the one-turn volatile whose stat overrides `DamageRules::beat_up` describes. Focus
    // Punch's, Beak Blast's, Shell Trap's is the turn-start volatile (§15.8).
    // Counter's / Mirror Coat's is the volatile that remembers the hit (the attack record).
    // Pursuit's is its mark on the foes (`VolatileCondition::Pursuit`); Rollout's / Ice Ball's
    // the chain (`VolatileCondition::Rolling`). Fling's is the volatile that loses the user its
    // item at the next `Update` (`VolatileCondition::Fling`).
    // Uproar's is its volatile (`VolatileCondition::Uproar`). Fury Cutter's is its chain
    // (`VolatileCondition::FuryCutter`, §15.18).
    // Rage's is its volatile (`VolatileCondition::Rage`, §15.19) — Gen 2-7's `onStart`,
    // `onHit`, `onBeforeMove`; Gen 1's `onStart`, `onHit` and its lock (`onLockMove`,
    // `RageEnd::Locked`).
    if behavior.rage
        && other
            .get("condition")
            .and_then(Value::as_array)
            .is_some_and(|callbacks| {
                let mut callbacks: Vec<&str> = callbacks.iter().filter_map(Value::as_str).collect();
                callbacks.sort_unstable();
                callbacks == ["onBeforeMove", "onHit", "onStart"]
                    || callbacks == ["onHit", "onLockMove", "onStart"]
            })
    {
        other.remove("condition");
    }
    // Glaive Rush's is its drawback (`VolatileCondition::GlaiveRush`, §15.20).
    if behavior.glaive_rush
        && other
            .get("condition")
            .and_then(Value::as_array)
            .is_some_and(|callbacks| {
                let mut callbacks: Vec<&str> = callbacks.iter().filter_map(Value::as_str).collect();
                callbacks.sort_unstable();
                callbacks
                    == [
                        "onAccuracy",
                        "onBeforeMove",
                        "onSourceModifyDamage",
                        "onStart",
                    ]
            })
    {
        other.remove("condition");
    }
    // Sky Drop's is the `skydrop` volatile (§15.23).
    if behavior.pledge_combo
        || behavior.echoes
        || behavior.sky_drop
        || behavior.power_formula == Some(crate::effects::moves::PowerFormula::FuryCutter)
        || behavior.beat_up
        || behavior.fling
        || behavior.marks_pursuit
        || behavior.rolling
        || behavior.uproar
        || behavior.turn_start_volatile.is_some()
        || matches!(behavior.damage_override, Some(FixedDamage::Retaliation(_)))
    {
        other.remove("condition");
    }

    // Status moves: exactly one declarative effect the engine can express.
    let status_effect = if category == MoveCategory::Status && custom.is_none() {
        match (behavior.weather_heal, behavior.cure_target) {
            (Some(heal), _) => Some(StatusMoveEffect::WeatherHeal(heal)),
            (_, Some(heal)) => Some(StatusMoveEffect::CureTarget(heal)),
            _ if let Some(heal) = behavior.on_hit_heal => Some(StatusMoveEffect::Heal(heal)),
            _ if let Some(heal) = behavior.target_heal => Some(StatusMoveEffect::TargetHeal(heal)),
            _ if behavior.status_transfer => Some(StatusMoveEffect::TransferStatus),
            _ => match resolve_status_effect(&mut other, &mut unsupported) {
                None if force_switch_taken_away
                    && unsupported.first().map(String::as_str)
                        == Some("status-move:no-declarative-effect") =>
                {
                    unsupported.remove(0);
                    Some(StatusMoveEffect::Inert)
                }
                effect => effect,
            },
        }
    } else {
        None
    };
    // Damaging phazers (Dragon Tail, Circle Throw) drag the target out after the hit.
    let force_switch = match other.remove("forceSwitch") {
        None => false,
        Some(Value::Bool(true)) => true,
        Some(other_value) => {
            unsupported.push(format!("mechanic:forceSwitch:{other_value}"));
            false
        }
    };

    // A two-turn move's `onTryMove` (charge / release) and its condition's callbacks
    // (`onInvulnerability`, `onSourceModifyDamage` / Gen 2 `onSourceBasePower`; `onImmunity`
    // is the sandstorm / hail immunity, which has no weather damage to matter to yet) are the
    // TryMove stage and the hit gate of the Shell, parameterised by `TwoTurnSpec` and the
    // generation's `TwoTurnRules`. Gen 1-2 also declare `onPrepareHit` (the Sleep Talk rule),
    // which nothing covers: their two-turn moves stay unsupported.
    let two_turn = behavior.two_turn;
    if two_turn.is_some() {
        let condition = other
            .remove("condition")
            .unwrap_or(Value::Array(Vec::new()));
        let covered = |callback: &Value| {
            matches!(
                callback.as_str(),
                Some(
                    "onInvulnerability"
                        | "onSourceModifyDamage"
                        | "onSourceBasePower"
                        | "onImmunity"
                )
            )
        };
        if !condition
            .as_array()
            .is_some_and(|callbacks| callbacks.iter().all(covered))
        {
            unsupported.push(format!("mechanic:condition:{condition}"));
        }
    }

    // A custom primary consumes the keys it names, and only those. Showdown's own `callsMove`
    // marker (Metronome, Sleep Talk, Copycat …) goes with them: the call is the primary's work
    // (DESIGN.md §42.1).
    if let Some(primary) = custom {
        for key in primary.source_keys {
            other.remove(*key);
        }
        other.remove("callsMove");
    }
    // The gimmick marks are data, not unsupported reasons: they must be read out before the
    // leftover keys are reported (DESIGN.md §44.4).
    let (z_crystal, is_max) = gimmick_marks(items, &mut other, &mut unsupported);
    for key in other.keys() {
        unsupported.push(format!("mechanic:{key}"));
    }
    // Every behavior component stands for its callbacks (DESIGN.md §25.1), and so does a
    // custom primary for the ones it names.
    let mut covered = behavior.covered_callbacks();
    if let Some(primary) = custom {
        covered.extend(primary.source_callbacks);
    }
    for callback in &snapshot.callbacks {
        if !covered.contains(&callback.as_str()) {
            unsupported.push(format!("callback:{callback}"));
        }
    }
    let has_flag = |flag: &str| snapshot.flags.iter().any(|f| f == flag);

    let primary_effect = if custom.is_some() {
        if unsupported.is_empty() {
            PrimaryEffect::Custom
        } else {
            PrimaryEffect::Unsupported
        }
    } else if category == MoveCategory::Status {
        match status_effect {
            Some(effect) if unsupported.is_empty() => PrimaryEffect::Status(effect),
            _ => PrimaryEffect::Unsupported,
        }
    } else if unsupported.is_empty() {
        PrimaryEffect::Damage
    } else {
        PrimaryEffect::Unsupported
    };
    let (z_power, z_status, max_power) = gimmick_data(
        snapshot,
        capabilities,
        category,
        move_type,
        multi_hit,
        z_crystal.is_some(),
        is_max,
        &mut unsupported,
    );
    Some(ResolvedMove {
        id,
        name: snapshot.name.clone(),
        base_power: snapshot.bp,
        accuracy: snapshot.acc,
        priority: snapshot.pri,
        category,
        data_category: category,
        move_type: if behavior.typeless {
            PokemonType::Typeless
        } else {
            move_type
        },
        target,
        is_spread,
        max_pp: snapshot.pp,
        primary_effect,
        flags: MoveFlags {
            protectable: has_flag("protect"),
            bypass_substitute: has_flag("bypasssub"),
            contact: has_flag("contact"),
            type_immunity,
            // Set by the user's ability as it uses the move (`engine::modified_move`).
            force_stab: false,
            immunity_ignored: false,
            sheer_force: false,
            sheer_force_boost,
            parental_bond: false,
            parental_bond_exempt: has_flag("noparentalbond")
                || has_flag("charge")
                || has_flag("futuremove"),
            future_move: has_flag("futuremove"),
            powder: has_flag("powder"),
            defrost: has_flag("defrost"),
            fail_encore: has_flag("failencore"),
            heal: has_flag("heal"),
            cant_use_twice: has_flag("cantusetwice"),
            sound: has_flag("sound"),
            bullet: has_flag("bullet"),
            punch: has_flag("punch"),
            bite: has_flag("bite"),
            pulse: has_flag("pulse"),
            slicing: has_flag("slicing"),
            gravity: has_flag("gravity"),
            stalling: snapshot.other.get("stallingMove") == Some(&Value::Bool(true)),
            wind: has_flag("wind"),
            minimize: has_flag("minimize"),
            must_pressure: has_flag("mustpressure"),
            sleep_usable,
            callable: has_flag("metronome"),
            no_assist: has_flag("noassist"),
            no_sleep_talk: has_flag("nosleeptalk"),
            fail_copycat: has_flag("failcopycat"),
            fail_mimic: has_flag("failmimic"),
            no_sketch: has_flag("nosketch"),
            fail_instruct: has_flag("failinstruct"),
            fail_me_first: has_flag("failmefirst"),
            mirror: has_flag("mirror"),
            reflectable: has_flag("reflectable"),
            snatch: has_flag("snatch"),
            dance: has_flag("dance"),
            charge: has_flag("charge"),
            recharge: has_flag("recharge"),
            pledge_combo: has_flag("pledgecombo"),
        },
        secondaries,
        has_secondaries: !snapshot.secondaries.is_empty(),
        multi_hit,
        recoil,
        drain,
        self_switch,
        force_switch,
        crit_ratio,
        will_crit,
        no_damage_variance,
        multi_accuracy,
        tracks_target,
        calls_move,
        non_ghost_target,
        breaks_protect,
        steals_boosts,
        smart_target,
        fixed_damage,
        self_destruct,
        ohko,
        stat_sources,
        thaws_target,
        ignores_ability,
        z_crystal,
        is_max,
        z_power,
        z_status,
        max_power,
        behavior,
        unsupported,
    })
}

/// What the generation's gimmicks make of the move (DESIGN.md §44.1): its Z-Move's power or its
/// status Z version's extra effect (Gen 7), its Max Move's power (Gen 8). A Z-Move, a Max Move
/// and Struggle have neither (Showdown `dex-moves.mjs`).
/// The `isZ` / `isMax` marks (DESIGN.md §44.4): a move that *is* a Z-Move or a Max Move. Both
/// are ordinary, executable moves — the crystal or the Dynamax that produces one is the
/// mechanic — so they are data, not an unsupported reason. `isZ` names the Z-Crystal the move
/// belongs to (Showdown writes the item's id there).
fn gimmick_marks(
    items: &BTreeMap<String, crate::ids::ItemId>,
    other: &mut BTreeMap<String, Value>,
    unsupported: &mut Vec<String>,
) -> (Option<crate::ids::ItemId>, bool) {
    let crystal = match other.remove("isZ") {
        None => None,
        Some(Value::String(name)) => match items.get(&name).copied() {
            // A Z-Move's own data names the crystal it belongs to; the generic 18 all point at
            // their type's crystal, so the data is what identifies them.
            Some(item) => Some(item),
            None => {
                unsupported.push(format!("mechanic:isZ:{name}"));
                None
            }
        },
        Some(other_value) => {
            unsupported.push(format!("mechanic:isZ:{other_value}"));
            None
        }
    };
    let is_max = match other.remove("isMax") {
        None => false,
        // `true` for the generic Max Moves, a species' name for a G-Max move (unsupported: the
        // dex snapshots do not carry them, DESIGN.md §44.5).
        Some(Value::Bool(true)) => true,
        Some(Value::String(species)) => {
            unsupported.push(format!("mechanic:isMax:{species}"));
            false
        }
        Some(other_value) => {
            unsupported.push(format!("mechanic:isMax:{other_value}"));
            false
        }
    };
    (crystal, is_max)
}

#[allow(clippy::too_many_arguments)]
fn gimmick_data(
    snapshot: &MoveSnapshot,
    capabilities: &GenerationCapabilities,
    category: MoveCategory,
    move_type: PokemonType,
    multi_hit: MultiHit,
    is_z: bool,
    is_max: bool,
    unsupported: &mut Vec<String>,
) -> (Option<u16>, Option<crate::dex::ZStatusEffect>, Option<u16>) {
    let converts = !is_z && !is_max && snapshot.id != "struggle";
    let damaging = converts && category != MoveCategory::Status;
    let declared = snapshot.z_move.as_ref();
    let z_power = (capabilities.z_move && damaging).then(|| {
        declared
            .and_then(|z| z.base_power)
            .unwrap_or_else(|| crate::dex::z_move_power(snapshot.bp, multi_hit))
    });
    let z_status = match declared {
        Some(z) if capabilities.z_move && converts && category == MoveCategory::Status => {
            if let Some(boosts) = &z.boost {
                let stages: Option<Vec<_>> = boosts
                    .iter()
                    .map(|(stat, amount)| parse_stat(stat).map(|stat| (stat, *amount)))
                    .collect();
                match stages {
                    Some(stages) => Some(crate::dex::ZStatusEffect::Boosts(stages)),
                    None => {
                        unsupported.push(format!("zMove:boost:{boosts:?}"));
                        None
                    }
                }
            } else if let Some(effect) = &z.effect {
                let parsed = crate::dex::ZStatusEffect::from_effect(effect);
                if parsed.is_none() {
                    unsupported.push(format!("zMove:effect:{effect}"));
                }
                parsed
            } else {
                None
            }
        }
        _ => None,
    };
    let max_power = (capabilities.dynamax && damaging).then(|| {
        snapshot.max_move.map_or_else(
            || crate::dex::max_move_power(move_type, snapshot.bp),
            |max| max.base_power,
        )
    });
    (z_power, z_status, max_power)
}

/// Showdown's `[numerator, denominator]` pairs (recoil, drain).
fn parse_fraction(
    other: &mut BTreeMap<String, Value>,
    key: &str,
    unsupported: &mut Vec<String>,
) -> Option<Fraction> {
    let value = other.remove(key)?;
    if let Some([numerator, denominator]) = value.as_array().map(Vec::as_slice)
        && let (Some(n), Some(d)) = (numerator.as_u64(), denominator.as_u64())
        && d > 0
        && n <= u64::from(u16::MAX)
        && d <= u64::from(u16::MAX)
    {
        return Some(Fraction::new(n as u16, d as u16));
    }
    unsupported.push(format!("mechanic:{key}:{value}"));
    None
}

/// Reads the declarative status-move keys out of `other`. Exactly one recognised effect makes
/// the move expressible; zero or several leave it unsupported with the reason recorded.
fn resolve_status_effect(
    other: &mut BTreeMap<String, Value>,
    unsupported: &mut Vec<String>,
) -> Option<StatusMoveEffect> {
    let mut effects = Vec::new();
    if let Some(value) = other.remove("status") {
        match value.as_str().and_then(parse_status) {
            Some(status) => effects.push(StatusMoveEffect::SetStatus(status)),
            None => unsupported.push(format!("status-move:status:{value}")),
        }
    }
    if let Some(value) = other.remove("boosts") {
        let mut stages = Vec::new();
        let mut ok = true;
        if let Some(map) = value.as_object() {
            for (stat, delta) in map {
                match (parse_stat(stat), delta.as_i64()) {
                    (Some(stat), Some(delta)) => stages.push((stat, delta as i8)),
                    _ => ok = false,
                }
            }
        } else {
            ok = false;
        }
        if ok && !stages.is_empty() {
            effects.push(StatusMoveEffect::StatStages(stages));
        } else {
            unsupported.push(format!("status-move:boosts:{value}"));
        }
    }
    if let Some(value) = other.remove("volatileStatus") {
        match value.as_str() {
            Some("confusion") => {
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Confusion))
            }
            // Taunt / Encore carry their condition inline (`condition: [onStart, onBeforeMove
            // …]`); the engine implements those handlers as the volatile's lifecycle, so the
            // key is consumed here rather than reported as an unmodelled mechanic.
            Some("taunt") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Taunt));
            }
            Some("encore") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Encore));
            }
            // Yawn's condition (`onStart`, `onEnd`: the sleep when it runs out) and
            // Nightmare's (`onStart`, the residual chip; Gen 2 `onAfterMoveSelf`) are the
            // volatiles' lifecycle (`set_yawn`, `set_nightmare`, the residual schedule).
            Some("yawn") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Yawn));
            }
            Some("nightmare") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Nightmare));
            }
            // Heal Block's condition (`durationCallback`, `onStart`, `onDisableMove`,
            // `onBeforeMove`, `onModifyMove` for a move another one calls — none does in the
            // engine —, the residual tick, `onEnd`, `onTryHeal`, `onRestart`) is the volatile's
            // lifecycle (`set_heal_block`, the request, BeforeMove, `apply_heal`).
            Some("healblock") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::HealBlock));
            }
            // Embargo's condition (`duration`, `onStart`, the residual tick, `onEnd`) is the
            // volatile's lifecycle (`set_embargo`, the residual schedule); the suppression
            // itself is `engine::held_item`'s.
            Some("embargo") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Embargo));
            }
            // Gastro Acid's condition (`onStart`: `-endability` and the ability's `End`; `onCopy`)
            // is the volatile's lifecycle (`set_gastro_acid`, `BattleEngine::hand_over`); the
            // suppression itself is `engine::ignoring_ability`'s.
            Some("gastroacid") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(
                    VolatileConditionKind::GastroAcid,
                ));
            }
            // Tar Shot's condition (`onStart`, `onEffectiveness`) is the volatile's lifecycle and
            // the type-effectiveness step (`hit_gate::tar_shot_factor`).
            Some("tarshot") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::TarShot));
            }
            // Attract's condition (`onStart`, `onBeforeMove`, `onUpdate`, `onEnd`) is the
            // infatuation volatile's lifecycle (`BattleContext::set_attract`, BeforeMove, the
            // source leaving the field).
            Some("attract") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Attract));
            }
            // Defense Curl's condition (`noCopy`, `onRestart` → null: a second one adds
            // nothing) is the mark itself; Aqua Ring's (`onStart`, the residual heal) its
            // lifecycle (`set_marker_volatile`, `ResidualKind::AquaRing`).
            Some("defensecurl") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(
                    VolatileConditionKind::DefenseCurl,
                ));
            }
            // Minimize's condition (`noCopy`, `onRestart` → null, `onSourceModifyDamage`,
            // `onAccuracy`) is the mark, the ModifyDamage chain and `engine::accuracy_roll`.
            Some("minimize") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Minimize));
            }
            Some("aquaring") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::AquaRing));
            }
            // Torment's condition (`noCopy`, `onStart`, `onEnd`, `onDisableMove`) and
            // Imprison's (`noCopy`, `onStart`, `onFoeDisableMove`, `onFoeBeforeMove`) are the
            // volatiles' lifecycle, the request (`choice::disabled_by`) and BeforeMove.
            Some("torment") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Torment));
            }
            Some("imprison") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Imprison));
            }
            // The critical-hit volatiles' conditions (`onStart`, `onModifyCritRatio`, Laser
            // Focus's `onRestart` / `onEnd`) are their lifecycle and `engine::critical_roll`.
            Some("focusenergy") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(
                    VolatileConditionKind::FocusEnergy,
                ));
            }
            Some("dragoncheer") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(
                    VolatileConditionKind::DragonCheer,
                ));
            }
            // Charge's condition (`onStart`, `onRestart`, `onBasePower`, `onAfterMove`,
            // `onMoveAborted`, `onEnd`) is the volatile's lifecycle, the BasePower chain and
            // `MovePipeline::execute`.
            Some("charge") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Charge));
            }
            Some("laserfocus") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(
                    VolatileConditionKind::LaserFocus,
                ));
            }
            // Powder's condition (`onStart`, `onTryMove`) is the volatile and the Shell's TryMove
            // check (DESIGN.md R275).
            Some("powder") => {
                other.remove("condition");
                effects.push(StatusMoveEffect::Volatile(VolatileConditionKind::Powder));
            }
            _ => unsupported.push(format!("status-move:volatile:{value}")),
        }
    }
    if other.remove("forceSwitch") == Some(Value::Bool(true)) {
        effects.push(StatusMoveEffect::ForceSwitch);
    }
    if let Some(value) = other.remove("weather") {
        // Showdown ids; hail (Gen 3-8) and snow (Gen 9) are the one `WeatherKind::Snow`.
        let kind = match value.as_str().map(str::to_ascii_lowercase).as_deref() {
            Some("sunnyday") => Some(WeatherKind::Sun),
            Some("raindance") => Some(WeatherKind::Rain),
            Some("sandstorm") => Some(WeatherKind::Sandstorm),
            Some("hail" | "snowscape") => Some(WeatherKind::Snow),
            _ => None,
        };
        match kind {
            Some(kind) => effects.push(StatusMoveEffect::Weather(kind)),
            None => unsupported.push(format!("status-move:weather:{value}")),
        }
    }
    if let Some(value) = other.remove("heal") {
        let parts = value
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_u64).collect::<Vec<_>>());
        match parts.as_deref() {
            Some([numerator, denominator]) if *denominator > 0 => effects.push(
                StatusMoveEffect::Heal(Fraction::new(*numerator as u16, *denominator as u16)),
            ),
            _ => unsupported.push(format!("status-move:heal:{value}")),
        }
    }
    match effects.as_slice() {
        [_] => effects.pop(),
        [] => {
            unsupported.insert(0, "status-move:no-declarative-effect".into());
            None
        }
        // Stat stages with a status (Toxic Thread) or confusion (Swagger, Flatter).
        [
            StatusMoveEffect::SetStatus(status),
            StatusMoveEffect::StatStages(stages),
        ] => Some(StatusMoveEffect::StagesThen(
            stages.clone(),
            FollowUp::Status(*status),
        )),
        // Stat stages with confusion (Swagger, Flatter), tar (Tar Shot), Defense Curl's or
        // Minimize's mark.
        [
            StatusMoveEffect::StatStages(stages),
            StatusMoveEffect::Volatile(
                kind @ (VolatileConditionKind::Confusion
                | VolatileConditionKind::TarShot
                | VolatileConditionKind::DefenseCurl
                | VolatileConditionKind::Minimize
                | VolatileConditionKind::Charge),
            ),
        ] => Some(StatusMoveEffect::StagesThen(
            stages.clone(),
            FollowUp::Volatile(*kind),
        )),
        _ => {
            unsupported.push("status-move:multiple-effects".into());
            None
        }
    }
}

/// Showdown target string → engine target pattern (+ spread flag). `None` = not modelled yet.
fn parse_target(target: &str) -> Option<(TargetPattern, bool)> {
    Some(match target {
        "normal" => (TargetPattern::SelectedAdjacent, false),
        "adjacentFoe" => (TargetPattern::SelectedAdjacentFoe, false),
        "randomNormal" => (TargetPattern::RandomAdjacentFoe, false),
        // Counter, Metal Burst …: aimed like Struggle, then at the attacker they answer
        // (`MoveBehavior::strikes_back_at`).
        "scripted" => (TargetPattern::RandomAdjacentFoe, false),
        "any" => (TargetPattern::SelectedAny, false),
        "self" => (TargetPattern::SelfOnly, false),
        "adjacentAlly" => (TargetPattern::Ally, false),
        "adjacentAllyOrSelf" => (TargetPattern::AllyOrSelf, false),
        "allAdjacentFoes" => (TargetPattern::AllAdjacentFoes, true),
        "allAdjacent" => (TargetPattern::AllAdjacent, true),
        "allySide" => (TargetPattern::UserSide, false),
        "allyTeam" => (TargetPattern::UserTeam, false),
        "foeSide" => (TargetPattern::OpponentSide, false),
        "all" => (TargetPattern::Field, false),
        "allies" => (TargetPattern::UserAndAllies, false),
        _ => return None,
    })
}

fn parse_status(status: &str) -> Option<NonVolatileStatus> {
    Some(match status {
        "brn" => NonVolatileStatus::Burn,
        "par" => NonVolatileStatus::Paralysis,
        "psn" => NonVolatileStatus::Poison,
        "tox" => NonVolatileStatus::Toxic,
        "slp" => NonVolatileStatus::Sleep,
        "frz" => NonVolatileStatus::Freeze,
        _ => return None,
    })
}

fn parse_stat(stat: &str) -> Option<Stat> {
    Some(match stat {
        "atk" => Stat::Attack,
        "def" => Stat::Defense,
        "spa" => Stat::SpecialAttack,
        "spd" => Stat::SpecialDefense,
        "spe" => Stat::Speed,
        "accuracy" => Stat::Accuracy,
        "evasion" => Stat::Evasion,
        _ => return None,
    })
}

/// Expresses one Showdown effect table as typed `SecondaryEffectSpec`s where the engine can,
/// and records what it cannot. `in_secondaries`: the table is one of the move's `secondaries`;
/// `self_trigger`: when the table's user-directed part lands.
fn resolve_effect_table(
    table: &EffectSnapshot,
    label: &str,
    in_secondaries: bool,
    self_trigger: SecondaryTrigger,
    out: &mut Vec<SecondaryEffectSpec>,
    unsupported: &mut Vec<String>,
) {
    let chance = table
        .chance
        .map_or(EffectChance::CERTAIN, EffectChance::percent);
    // Everything one table declares is decided by a single roll.
    let first = out.len();
    let mut recognised = false;

    if let Some(status) = &table.status {
        match parse_status(status) {
            Some(status) => {
                out.push(SecondaryEffectSpec {
                    chance,
                    target: SecondaryTarget::Target,
                    trigger: SecondaryTrigger::PerTargetHit,
                    effect: SecondaryEffect::Status(status),
                    requires: None,
                    shares_roll: out.len() > first,
                    in_secondaries,
                    from_item: false,
                });
                recognised = true;
            }
            None => unsupported.push(format!("{label}:status:{status}")),
        }
    }
    if let Some(boosts) = &table.boosts {
        for (stat, delta) in boosts {
            match parse_stat(stat) {
                Some(stat) => {
                    out.push(SecondaryEffectSpec {
                        chance,
                        target: SecondaryTarget::Target,
                        trigger: SecondaryTrigger::PerTargetHit,
                        effect: SecondaryEffect::StatStage {
                            stat,
                            delta: *delta,
                        },
                        requires: None,
                        shares_roll: out.len() > first,
                        in_secondaries,
                        from_item: false,
                    });
                    recognised = true;
                }
                None => unsupported.push(format!("{label}:boost:{stat}")),
            }
        }
    }
    if let Some(inner) = &table.self_effect {
        // A `self` with its own chance inside a secondary would roll apart from the table's (the
        // move's own `self` has had it moved to the table).
        if inner.chance.is_some() {
            unsupported.push(format!("{label}:self:chance"));
        }
        if let Some(boosts) = &inner.boosts {
            for (stat, delta) in boosts {
                match parse_stat(stat) {
                    Some(stat) => {
                        out.push(SecondaryEffectSpec {
                            chance,
                            target: SecondaryTarget::User,
                            trigger: self_trigger,
                            effect: SecondaryEffect::StatStage {
                                stat,
                                delta: *delta,
                            },
                            requires: None,
                            shares_roll: out.len() > first,
                            in_secondaries,
                            from_item: false,
                        });
                        recognised = true;
                    }
                    None => unsupported.push(format!("{label}:self:boost:{stat}")),
                }
            }
        }
        if let Some(volatile) = &inner.volatile_status {
            // Hyper Beam's recharge turn; Roost's grounding for the turn (only once its heal
            // went through: a status move's `self` lands after an effect).
            let kind = match volatile.as_str() {
                "mustrecharge" => Some(VolatileConditionKind::MustRecharge),
                "roost" => Some(VolatileConditionKind::Roost),
                _ => None,
            };
            if let Some(kind) = kind {
                out.push(SecondaryEffectSpec {
                    chance: EffectChance::CERTAIN,
                    target: SecondaryTarget::User,
                    trigger: self_trigger,
                    effect: SecondaryEffect::Volatile(kind),
                    requires: None,
                    shares_roll: out.len() > first,
                    in_secondaries,
                    from_item: false,
                });
                recognised = true;
            } else {
                unsupported.push(format!("{label}:self:volatile:{volatile}"));
            }
        }
        for callback in &inner.callbacks {
            unsupported.push(format!("{label}:self:callback:{callback}"));
        }
        for key in inner.other.keys() {
            unsupported.push(format!("{label}:self:{key}"));
        }
    }
    if let Some(volatile) = &table.volatile_status {
        let kind = match volatile.as_str() {
            "flinch" => Some(VolatileConditionKind::Flinch),
            "confusion" => Some(VolatileConditionKind::Confusion),
            // Psychic Noise (Heal Block's condition is the move Heal Block's).
            "healblock" => Some(VolatileConditionKind::HealBlock),
            "saltcure" => Some(VolatileConditionKind::SaltCure),
            "syrupbomb" => Some(VolatileConditionKind::SyrupBomb),
            // Sparkling Aria's mark has no condition of its own (the move's `onAfterMove` reads
            // it).
            "sparklingaria" => Some(VolatileConditionKind::SparklingAria),
            _ => None,
        };
        match kind {
            Some(kind) => {
                out.push(SecondaryEffectSpec {
                    chance,
                    target: SecondaryTarget::Target,
                    trigger: SecondaryTrigger::PerTargetHit,
                    effect: SecondaryEffect::Volatile(kind),
                    requires: None,
                    shares_roll: out.len() > first,
                    in_secondaries,
                    from_item: false,
                });
                recognised = true;
            }
            None => unsupported.push(format!("{label}:volatile:{volatile}")),
        }
    }
    for callback in &table.callbacks {
        unsupported.push(format!("{label}:callback:{callback}"));
    }
    for key in table.other.keys() {
        unsupported.push(format!("{label}:{key}"));
    }
    if !recognised && table.self_effect.is_none() && table.volatile_status.is_none() {
        // An effect table the engine cannot see into (e.g. only an `onHit` that was dropped
        // upstream) must not pass as harmless — except `secondary: {}` itself (Gen 7+ Diamond
        // Storm, Stone Axe): nothing to roll for, only there so Sheer Force applies
        // (`ResolvedMove::has_secondaries`).
        let bare_secondary = in_secondaries && table.chance.is_none();
        if table.callbacks.is_empty() && table.other.is_empty() && !bare_secondary {
            unsupported.push(format!("{label}:empty"));
        }
    }
}
