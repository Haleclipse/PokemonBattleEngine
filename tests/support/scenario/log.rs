//! Reading Showdown's log (`LogReading`, `extract_decisions_with`): who chose what, and what each
//! action did — its hits, misses, statuses, called moves, the frames' attributions.

use std::collections::BTreeMap;
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, FollowUp, MoveCategory, MultiHit, Nature, PokemonSet, PrimaryEffect, ResolvedDex,
    ResolvedMove, SecondaryEffect, SecondaryTarget, SecondaryTrigger, StatSpread, StatusMoveEffect,
    build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, BattleSnapshot, EngineStep, ForcedReplacement, ForcedSwitchChoice,
    MechanicChoice, PriorityChance, ReplacementRequirement, ReviveChoice, SideChoice, SlotChoice,
    TraceEvent, two_turn,
};
use pokemon_battle_engine::format::{FormatLibrary, Stats, TeamSet};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, HitStepOrder, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::session::{OpenTeamSheets, SessionFormat, TeamChoice, TeamSlot};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    ActiveMove, BattleRng, BattleState, Gender, NonVolatileStatus, RandomOverride,
    RandomRequestKind, SideState, VolatileCondition, VolatileConditionKind,
};
use serde::Deserialize;
use serde_json::Value;

use crate::support::{capture, compare_json};

use super::*;

pub(super) fn current(out: &mut Vec<ActionOracle>, user: Who) -> &mut ActionOracle {
    let fresh = match out.last() {
        None => true,
        Some(last) => last.user != user || last.move_name.is_some() || last.cant.is_some(),
    };
    if fresh {
        out.push(ActionOracle::new(user));
    }
    out.last_mut().expect("just pushed")
}

/// What reading a generation's log needs to know about its rules.
#[derive(Debug, Clone, Default)]
pub struct LogReading {
    /// The generation's poison / burn and Leech Seed act right after the user's own action
    /// (Gen 2, `StatusResidualTiming::AfterOwnAction`), so their HP commits before the residual
    /// marker belong to that action.
    pub status_after_action: bool,
    /// Moves whose hit thaws a frozen target (`engine::thaws_target_on_hit`), by name: their
    /// target's `-curestatus … frz` belongs to the hit, not to the target's own turn.
    pub thawing_moves: std::collections::BTreeSet<String>,
    /// Moves with a flinch secondary, by name: a victim's `cant … flinch` belongs to the last of
    /// them that hit it, not to a later hit that cannot flinch (Lava Plume after Stomp).
    pub flinching_moves: std::collections::BTreeSet<String>,
    /// Gen 1 (`ConditionRules::flinch_clears_recharge`, the gen1 mod's `flinch.onStart`): a
    /// flinch clears a recharge, so a frame without the `mustrecharge` a recharge move left
    /// shows a flinch nothing else does (R-M8-105).
    pub flinch_clears_recharge: bool,
    /// Moves their user must recharge after (Showdown `recharge`: Hyper Beam), by name.
    pub recharge_moves: std::collections::BTreeSet<String>,
    /// Moves with crash damage, by name: damage on their user with no source (Gen 1-4) or from
    /// a crash condition (`[from] highjumpkick`, Gen 5+) is their crash.
    pub crashing_moves: std::collections::BTreeSet<String>,
    /// Moves that cure their target's status (Purify; Jungle Healing and Lunar Blessing on their
    /// user's side), by name, each with the one status it cures, if only one (Worry Seed's
    /// sleep, R-M8-103): the target's `-curestatus` of it within them is the move's, not the
    /// target waking up or thawing on its own turn.
    pub curing_moves: BTreeMap<String, Option<&'static str>>,
    /// The curing moves aimed at the user's own side (Jungle Healing, Lunar Blessing: `allies`;
    /// Aromatherapy, Heal Bell: `allyTeam`): a foe's `-curestatus` within them is its own
    /// (R-M8-113).
    pub own_side_curing_moves: std::collections::BTreeSet<String>,
    /// Abilities whose `Update` cures a status (Immunity, Insomnia, Vital Spirit, Limber, Water
    /// Veil, Magma Armor …), by name: the `-curestatus` right after their `-activate` is the
    /// ability's, not the holder waking or thawing on its own turn. Another ability's
    /// `-activate` (Quick Draw's priority chance) says nothing about a cure that follows
    /// (DESIGN.md R-M8-49).
    pub curing_abilities: std::collections::BTreeSet<String>,
}

impl LogReading {
    pub fn for_rules(rules: &ResolvedRules, dex: &ResolvedDex) -> Self {
        Self {
            status_after_action: rules.status().residual_timing
                == pokemon_battle_engine::rules::StatusResidualTiming::AfterOwnAction,
            // (Gen 2 Tri Attack thaws when its `statusRoll` is a burn.)
            thawing_moves: dex
                .moves()
                .iter()
                .filter(|m| {
                    pokemon_battle_engine::engine::thaws_target_on_hit(rules, m, None)
                        || (m.behavior.status_pick_before_chance
                            && pokemon_battle_engine::engine::thaws_target_on_hit(
                                rules,
                                m,
                                Some(NonVolatileStatus::Burn),
                            ))
                })
                .map(|m| m.name.clone())
                .collect(),
            flinching_moves: dex
                .moves()
                .iter()
                .filter(|m| {
                    m.secondaries.iter().any(|s| {
                        s.effect == SecondaryEffect::Volatile(VolatileConditionKind::Flinch)
                    })
                })
                .map(|m| m.name.clone())
                .collect(),
            flinch_clears_recharge: rules.conditions().flinch_clears_recharge,
            recharge_moves: dex
                .moves()
                .iter()
                .filter(|m| m.flags.recharge)
                .map(|m| m.name.clone())
                .collect(),
            crashing_moves: dex
                .moves()
                .iter()
                .filter(|m| m.behavior.crash_damage)
                .map(|m| m.name.clone())
                .collect(),
            curing_abilities: dex
                .abilities()
                .iter()
                .filter(|ability| {
                    matches!(
                        pokemon_battle_engine::engine::ability_behavior(rules, ability.id).update,
                        Some(
                            pokemon_battle_engine::effects::abilities::AbilityUpdate::CureStatus(_)
                        )
                    )
                })
                .map(|ability| ability.name.clone())
                .collect(),
            own_side_curing_moves: dex
                .moves()
                .iter()
                .filter(|m| {
                    matches!(
                        m.target,
                        pokemon_battle_engine::rules::TargetPattern::UserAndAllies
                            | pokemon_battle_engine::rules::TargetPattern::UserTeam
                            | pokemon_battle_engine::rules::TargetPattern::UserSide
                    )
                })
                .map(|m| m.name.clone())
                .collect(),
            curing_moves: dex
                .moves()
                .iter()
                .filter_map(|m| {
                    let any = matches!(
                        m.primary_effect,
                        PrimaryEffect::Status(StatusMoveEffect::CureTarget(_))
                    ) || pokemon_battle_engine::effects::moves::cures_targets(m.id);
                    // Worry Seed cures the sleep its Insomnia would (`onHit`: `target.status ===
                    // 'slp'`); Wake-Up Slap, Smelling Salts their own status as they hit
                    // (R-M8-93); Gen 5+ Uproar wakes the sleepers as it hits.
                    let only = if m.id == MoveId::WORRY_SEED
                        || (m.behavior.uproar
                            && rules.conditions().uproar.wake
                                == pokemon_battle_engine::rules::UproarWake::OnHit)
                    {
                        Some("slp")
                    } else {
                        m.behavior.cures_target_status.map(status_id)
                    };
                    (any || only.is_some()).then(|| (m.name.clone(), only))
                })
                .collect(),
        }
    }
}

/// The Pokémon that entered the field during a boundary (`switch` / `drag`), each with the
/// number of actions `extract_decisions` had read before it: the entry precedes that action.
pub fn extract_entries(events: &[Value], reading: &LogReading) -> Vec<(usize, Who)> {
    events
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event["kind"].as_str(), Some("switch" | "drag")))
        .filter_map(|(position, event)| {
            let entrant = serde_json::from_value::<Who>(event["pokemon"].clone()).ok()?;
            Some((
                extract_decisions(&events[..position], reading).len(),
                entrant,
            ))
        })
        .collect()
}

pub fn extract_decisions(events: &[Value], reading: &LogReading) -> Vec<ActionOracle> {
    extract_decisions_with(events, reading, &|_, _| false, &|_, _, _| false)
}

/// `extract_decisions`, knowing whose item or ability gives its moves a flinch of their own
/// (King's Rock, Razor Fang, Stench): a silent flinch goes to that Pokémon's hit before a later
/// hit that cannot flinch.
pub fn extract_decisions_with(
    events: &[Value],
    reading: &LogReading,
    adds_flinch: &dyn Fn(&Who, Option<&str>) -> bool,
    // Gen 1: whether this Pokémon's stat (`atk` …) reads 999 (`true`: a raise) or 1 (`false`:
    // a drop) — where its change prints an echo the other way (R-M9-42).
    stat_stuck: &dyn Fn(&Who, &str, bool) -> bool,
) -> Vec<ActionOracle> {
    let status_after_action = reading.status_after_action;
    let who = |v: &Value| serde_json::from_value::<Who>(v.clone()).ok();
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    let mut out: Vec<ActionOracle> = Vec::new();
    // `-crit|<target>` precedes the hit it belongs to; it is attached when that target's damage
    // (HP or Substitute) is reported. A spread move reports every target's `-crit` before any
    // damage (Showdown computes all the damage first), so the crit is matched by target.
    let mut pending_crits: Vec<Who> = Vec::new();
    let take_crit =
        |pending: &mut Vec<Who>, target: &Who| match pending.iter().position(|p| p == target) {
            Some(index) => {
                pending.remove(index);
                true
            }
            None => false,
        };
    // A cure announced by Heal Bell / Shed Skin / Natural Cure is not the actor waking up: the
    // side it covers (Heal Bell cures its user's team only; a foe thawing right after is its
    // own thaw).
    let mut announced_cure: Option<usize> = None;
    // After the `residual` marker nothing is an action's outcome any more.
    let mut in_residual = false;
    // A Pokémon dragged in by the current action (Roar, Dragon Tail, Red Card) takes its entry
    // hazards inside that action.
    let mut dragging = false;
    // So does the one whose switch an intercepting Pursuit held up (§15.11): the switch goes on
    // inside the Pursuit's step — Gen 2-4 even after the Pursuit knocked the Pokémon out.
    let mut pursuit_held_switch = false;
    let mut pursued_entrant: Option<Who> = None;
    // The holder whose Synchronize just announced it passes a status back.
    let mut synchronizing: Option<Who> = None;
    // The Pokémon whose own ability just announced a stat change (`-ability|p|Justified|boost`):
    // the stage changes on it that follow are the ability's, not the move's secondary.
    let mut ability_stat_change: Option<Who> = None;
    // An item used up (`-enditem|p|X` with no `[from]`: Snowball, Weakness Policy …): the stat
    // changes that follow on its holder are the item's, no secondary's.
    let mut item_stat_change: Option<Who> = None;
    // The Pokémon an Instruct just told to use its move again (`-singleturn|p|move: Instruct`).
    let mut instructed_next: Option<Who> = None;
    let mut instructed_chances: Vec<PriorityChance> = Vec::new();
    // Cotton Down's `-ability|holder|Cotton Down` and the stat drops right after it, on any
    // Pokémon: the ability's, not the move's secondary (DESIGN.md R269).
    let mut cotton_down = false;
    // Powder blowing up the user's Fire move (`-activate|user|move: Powder`): the silent
    // `-damage` on the user that follows is the action's HP commit (DESIGN.md R275).
    let mut powder_blowup: Option<Who> = None;
    // The Pokémon that just ate a berry (`-enditem|p|Berry|[eat]`): the cure that follows is
    // the berry's, not a wake-up or a thaw of its own.
    let mut ate_berry: Option<Who> = None;
    // The Pokémon that just ate a confusion berry: its `-start|p|confusion` (after the heal) is
    // the berry's.
    let mut confusion_berry: Option<Who> = None;
    // The Pokémon that just ate a berry, and the berry: a stage change right after is the
    // berry's.
    let mut eaten_berry: Option<(Who, String)> = None;
    // The Pokémon a switch (not a drag) brought in since the last move: its switch-in is not
    // part of an action.
    let mut entrant: Option<Who> = None;
    // `-activate|target|move: Pursuit`: the next Pursuit strikes a Pokémon as it switches out.
    let mut pursuit_intercept = false;
    // A berry Fling threw and the Pokémon it struck, who eats it with no `-enditem` of its own:
    // the thrower's `-enditem|user|X Berry|[from] move: Fling` only follows at the `Update`, so
    // it is read ahead as the move starts.
    let mut flung_berry: Option<(Who, String)> = None;
    // The Pokémon whose *curing* ability just announced itself (`-activate|p|ability: Vital
    // Spirit`): a cure right after is the ability's (its `Update`), not a wake-up of its own. A
    // `-activate` of any other ability (Quick Draw's priority chance) says nothing about a cure
    // that follows (DESIGN.md R-M8-49).
    let mut ability_announced: Option<Who> = None;
    for (position, event) in events.iter().enumerate() {
        let kind = event["kind"].as_str().unwrap_or("");
        match kind {
            "switch" => {
                entrant = who(&event["pokemon"]);
                pursued_entrant = std::mem::take(&mut pursuit_held_switch)
                    .then(|| entrant.clone())
                    .flatten();
            }
            "move" | "cant" | "turn" => {
                entrant = None;
                pursuit_held_switch = false;
                pursued_entrant = None;
            }
            _ => {}
        }
        if kind == "move" {
            flung_berry = (text(&event["move"]) == "Fling")
                .then(|| flung(&events[position + 1..], &event["target"]))
                .flatten();
        }
        let after_ability = ability_announced.take();
        let after_synchronize = synchronizing.take();
        let ability_boosting = ability_stat_change.take();
        let item_boosting = item_stat_change.take();
        let cotton_down_drops = std::mem::take(&mut cotton_down);
        let berry_eater = ate_berry.take();
        let confusion_berry_eater = confusion_berry.take();
        let previous_berry = eaten_berry.take().or_else(|| {
            // The flung berry's own stage change follows the throw's damage.
            (kind == "boost")
                .then(|| flung_berry.take())
                .flatten()
                .filter(|(target, _)| who(&event["pokemon"]).as_ref() == Some(target))
        });
        eaten_berry = berry_eaten(event);
        if event["args"][2] == "[from] stealeat"
            && let Some((eater, _)) = &eaten_berry
            && let Some(action) = out.last_mut()
        {
            action.stolen_berry = Some(eater.clone());
        }
        if let Some(eater) = confusion_berry_eaten(event) {
            confusion_berry = Some(eater);
        } else if kind == "heal"
            && confusion_berry_eater.is_some()
            && who(&event["pokemon"]) == confusion_berry_eater
        {
            confusion_berry = confusion_berry_eater.clone();
        } else if kind == "heal"
            && event["from"].as_str().is_some_and(|from| {
                from.strip_prefix("[from] item: ")
                    .is_some_and(|berry| CONFUSION_BERRIES.contains(&berry))
            })
        {
            // A confusion berry Fling threw: no `-enditem` of the eater's own, its heal names it
            // (the throw hurt the target, so there is always a heal).
            confusion_berry = who(&event["pokemon"]);
        }
        if kind == "start"
            && !in_residual
            && event["args"][1] == "confusion"
            && let Some(eater) = confusion_berry_eater
                .filter(|eater| who_from_text(&event["args"][0]).as_ref() == Some(eater))
        {
            if entrant.as_ref() != Some(&eater)
                && let Some(action) = out.last_mut()
            {
                let hits = action.crits.len();
                action
                    .berry_draws
                    .push((eater.clone(), hits, BerryDraw::Confusion));
                action.applied.push((eater, "start:confusion".into()));
            }
            continue;
        }
        // A pinch berry's stat change right after it was eaten (`-boost|p|atk|1|[from] item:
        // Liechi Berry`) is not the move's; Starf Berry's names the stat it drew.
        if !in_residual
            && let Some((eater, draw)) = BerryDraw::berry_boost(previous_berry.as_ref(), event)
        {
            if let Some(draw) = draw
                && entrant.as_ref() != Some(&eater)
                && let Some(action) = out.last_mut()
            {
                let hits = action.crits.len();
                action.berry_draws.push((eater, hits, draw));
            }
            continue;
        }
        // An item's own stat change (`-boost|p|spa|1|[from] item: Absorb Bulb`, Weakness Policy,
        // a berry eaten after a hit) is not the move's either (DESIGN.md R198).
        if matches!(kind, "boost" | "unboost")
            && event["from"]
                .as_str()
                .is_some_and(|from| from.starts_with("[from] item: "))
        {
            continue;
        }
        if kind == "residual" {
            in_residual = true;
            continue;
        }
        if kind == "turn" {
            in_residual = false;
        }
        // (A foreseen attack's strike reports its own `-miss`, `-immune`, `-crit` there too:
        // `extract_future_strikes` reads them, DESIGN.md R277.)
        if in_residual
            && matches!(
                kind,
                "damage"
                    | "heal"
                    | "status"
                    | "curestatus"
                    | "boost"
                    | "unboost"
                    | "start"
                    | "miss"
                    | "immune"
                    | "crit"
            )
        {
            continue;
        }
        // A foreseen attack striking a Substitute in the residual is the strike's, not the
        // last action's (R-M8-84).
        if in_residual
            && matches!(kind, "activate" | "end")
            && event["args"][1]
                .as_str()
                .is_some_and(|what| what.ends_with("Substitute"))
        {
            continue;
        }
        // So is a hold on 1 HP against it (`ShownHolds`, R-M10-41): a Focus Band's, Sturdy's, a
        // Focus Sash's (`-enditem|p|Focus Sash` with no `[from]`, R-M11-30).
        if in_residual
            && ((kind == "activate" && event["args"][1] == "item: Focus Band")
                || (kind == "ability"
                    && event["args"][1] == "Sturdy"
                    && event["args"].get(2).is_none())
                || (kind == "enditem"
                    && event["args"][1] == "Focus Sash"
                    && event["args"].get(2).is_none())
                // The user's Toxic Chain on the target it struck (R-M11-33).
                || (kind == "status"
                    && event["from"]
                        .as_str()
                        .is_some_and(|f| f.contains("ability: Toxic Chain"))))
        {
            continue;
        }
        match kind {
            "move" => {
                announced_cure = None;
                dragging = false;
                pending_crits.clear();
                // A called move (`[from] move: Metronome`, `[from] Magic Coat`, `[from] ability:
                // Magic Bounce`): Showdown prints it inside the calling action, and the user may
                // be another Pokémon than the actor's (a reflection, §42.8); the replay reads it
                // off that action.
                // A `[from]` marker the Shell runs inside the same action (Metronome and its
                // kind, `useMove`) attaches to the calling action; one that is its own action
                // (a combined Pledge's promotion, Dancer's `runMove`) starts a new one.
                // A locked move going on is the Pokémon's own action, not a move another one
                // called: Gen 1's `runMove` gives the move itself as its source (`|move|p|Skull
                // Bash|…|[from] Skull Bash`; no caller calls itself), Gen 2+ the lock
                // (`[from] lockedmove`: a two-turn move's attack, Outrage's later turns —
                // R-M8-63).
                let continues_itself = event["from"].as_str().is_some_and(|from| {
                    from == format!("[from] {}", text(&event["move"]))
                        || from == "[from] lockedmove"
                });
                // Shell Side Arm's category as the move starts (`[anim] Shell Side Arm
                // Physical`, its `onPrepareHit`): there even when a protection stops the move
                // before its `-hint` (R-M8-166; older recordings carry no `anim`).
                let shell_side_arm = event["anim"]
                    .as_str()
                    .and_then(|anim| anim.strip_prefix("Shell Side Arm "))
                    .map(|category| category == "Physical");
                if event["from"]
                    .as_str()
                    .is_some_and(|from| from.starts_with("[from]") && !acts_on_its_own(from))
                    && !continues_itself
                    && let Some(user) = who(&event["user"])
                    && let Some(action) = out.last_mut()
                    && action.move_name.is_some()
                {
                    let called = CalledMove::new(
                        user,
                        text(&event["move"]),
                        who(&event["target"]),
                        event["from"].as_str().map(str::to_string),
                        action,
                    );
                    action.called.push(called);
                    action.move_target = who(&event["target"]);
                    if shell_side_arm.is_some() {
                        action.physical_hint = shell_side_arm;
                    }
                    // A called move whose target is named without a position (Metronome's Poison
                    // Gas at a foe Healing Wish took down, `p2: Mareep|[notarget]`) draws
                    // nothing either (R-M8-86).
                    if action
                        .move_target
                        .as_ref()
                        .is_some_and(|t| t.slot.is_none())
                    {
                        action.failed = true;
                        action.no_target = true;
                        action.hit = false;
                    }
                    continue;
                }
                if let Some(user) = who(&event["user"]) {
                    sleeps_through(&mut out, &user);
                    let instructed = instructed_next.take().as_ref() == Some(&user);
                    let chances = std::mem::take(&mut instructed_chances);
                    let action = current(&mut out, user);
                    action.instructed |= instructed;
                    if instructed {
                        action.instructed_chances = chances;
                    }
                    // A Z status move is logged as `Z-<move>` with the move as its `[anim]`
                    // (Showdown `useMoveInner`): it is that move, Z-powered — Z-Rest heals as Rest
                    // does (DESIGN.md R-M9-30).
                    let logged = text(&event["move"]);
                    action.move_name = Some(
                        event["anim"]
                            .as_str()
                            .filter(|anim| logged.strip_prefix("Z-") == Some(*anim))
                            .map_or(logged.clone(), str::to_owned),
                    );
                    action.intercepted =
                        std::mem::take(&mut pursuit_intercept) && text(&event["move"]) == "Pursuit";
                    pursuit_held_switch = action.intercepted;
                    action.move_target = who(&event["target"]);
                    action.own_target = who(&event["target"]);
                    if logged == "Fling" {
                        action.flung_berry = flung_berry.as_ref().map(|(target, _)| target.clone());
                    }
                    if shell_side_arm.is_some() {
                        action.physical_hint = shell_side_arm;
                    }
                    // A target named without a position (`p1: Maushold|[notarget]`): the one
                    // it was aimed at is gone and nobody took its place; the move draws nothing.
                    // A foreseen attack goes on at the fallen foe (`getMoveTargets` keeps it for
                    // a `futuremove`, R277): Gen 4 draws its damage then (R-M8-89).
                    if action
                        .move_target
                        .as_ref()
                        .is_some_and(|t| t.slot.is_none())
                        && !matches!(
                            action.move_name.as_deref(),
                            Some("Future Sight" | "Doom Desire")
                        )
                    {
                        action.failed = true;
                        action.no_target = true;
                        action.hit = false;
                    }
                }
            }
            "cant" => {
                let detail = text(&event["detail"]);
                // Damp stopping the move in progress (`cant|holder|ability: Damp|move|[of] user`):
                // the user's action, not the holder's, is the one that does not go ahead.
                // Dazzling, Queenly Majesty, Armor Tail stop a foe's priority move the same way.
                let stopper = TRY_MOVE_STOPPERS
                    .into_iter()
                    .find(|ability| detail.starts_with(ability));
                if let Some(stopper) = stopper
                    && let Some(action) = out.last_mut()
                    && action.move_name.is_some()
                {
                    action.cant = Some(stopper.to_string());
                    action.hit = false;
                } else if let Some(user) = who(&event["pokemon"])
                    && let Some(action) = out.last_mut()
                    && action.user == user
                    && action.move_name.as_deref() == detail.split('|').next()
                {
                    // The move stopping itself once it started (Gen 3-4 Focus Punch's `onTry`,
                    // Shell Trap's `onTryMove`: `move|user|Shell Trap` then `cant|user|Shell
                    // Trap|Shell Trap`): that very action does not go ahead.
                    action.cant = detail.split('|').next().map(str::to_string);
                    action.hit = false;
                } else if let Some(user) = who(&event["pokemon"]) {
                    let reason = detail.split('|').next().unwrap_or("").to_string();
                    if reason != "slp" {
                        sleeps_through(&mut out, &user);
                    }
                    // Pollen Puff stopped by Heal Block after it was announced (its user's, at
                    // TryMove, or its ally target's, as the heal fails): the move's own failure.
                    if reason == "move: Heal Block"
                        && let Some(last) = out.last_mut()
                        && last.user == user
                        && last.move_name.as_deref() == detail.split('|').nth(1)
                    {
                        last.failed = true;
                        continue;
                    }
                    // A move a caller called, refused as it is modified (Heal Block, Gravity:
                    // `onModifyMove`, before its own `move` line): the caller's action goes on
                    // to its end, the move it called having failed (R-M8-70).
                    // Gen 3 Sleep Talk drawing a move with no PP left (`cant|user|nopp|sketch`,
                    // issues.md I116): the move it called, which does not go ahead (R-M8-72).
                    // Another move's (an instructed one right before) is not: the `cant` is
                    // the user's own action (R-M8-84).
                    if reason == "nopp"
                        && let Some(last) = out.last_mut()
                        && last.user == user
                        && last.move_name.as_deref() == Some("Sleep Talk")
                        && last.called.is_empty()
                        && let Some(called) = detail.split('|').nth(1)
                    {
                        let called = CalledMove::new(user, called.to_string(), None, None, last);
                        last.called.push(called);
                        last.cant = Some(reason);
                        last.hit = false;
                        continue;
                    }
                    // Throat Chop's refusal names no move (`cant|user|move: Throat Chop`): after
                    // the user's caller, the move it called was a sound move (R-M8-148).
                    if reason == "move: Throat Chop"
                        && detail.split('|').nth(1).is_none()
                        && let Some(last) = out.last_mut()
                        && last.user == user
                        && last.move_name.is_some()
                        && last.called.is_empty()
                    {
                        let called =
                            CalledMove::new(user, UNNAMED_SOUND_MOVE.to_string(), None, None, last);
                        last.called.push(called);
                        last.failed = true;
                        last.hit = false;
                        continue;
                    }
                    if matches!(reason.as_str(), "move: Heal Block" | "move: Gravity")
                        && let Some(last) = out.last_mut()
                        && last.user == user
                        && last.move_name.is_some()
                        && let Some(called) = detail.split('|').nth(1)
                    {
                        let called = CalledMove::new(user, called.to_string(), None, None, last);
                        last.called.push(called);
                        last.failed = true;
                        last.hit = false;
                        continue;
                    }
                    let action = current(&mut out, user);
                    action.cant = Some(reason);
                    action.hit = false;
                }
            }
            "curestatus" if berry_eater.is_some() && berry_eater == who(&event["pokemon"]) => {}
            "enditem" if berry_eaten(event).is_some() => {
                ate_berry = berry_eaten(event).map(|(eater, _)| eater);
            }
            // A Focus Sash that held (no `[from]`: one taken by Knock Off or a thief says so).
            "enditem" if event["args"].get(2).is_none() && event["args"][1] != "Focus Sash" => {
                item_stat_change = event["args"].get(0).and_then(who_from_text);
            }
            "enditem" if event["args"][1] == "Focus Sash" && event["args"].get(2).is_none() => {
                if let (Some(action), Some(holder)) =
                    (out.last_mut(), event["args"].get(0).and_then(who_from_text))
                {
                    action.focus_sash_held.push(holder);
                }
            }
            "curestatus" => {
                let status = text(&event["status"]);
                let from = event["from"].as_str().unwrap_or("");
                // Gen 3-4 (issues.md I63): an uproar wakes the sleeper as it is about to move —
                // no wake-up of its own (`sleep_duration_hint`).
                if from == "[from] move: Uproar"
                    && let Some(woken) = who(&event["pokemon"])
                {
                    current(&mut out, woken.clone())
                        .applied
                        .push((woken, format!("uproar:{status}")));
                    continue;
                }
                if status == "frz"
                    && let Some(cured) = who(&event["pokemon"])
                {
                    // Thawed by the hit of the move in progress (a Fire move, Scald …): the
                    // frame checks it, no action of its own. A move that is Fire only as it is
                    // used (Hidden Power, Weather Ball …) is not in `thawing_moves`: a thaw that
                    // does not start the cured Pokémon's own action (`own_action_follows`) is that
                    // hit's too (R-M9-52). Where its own action does follow, the thaw
                    // roll planned there asks the engine whether it is still frozen.
                    if let Some(action) = out.last_mut()
                        && action.user != cured
                        && action.damages.iter().any(|(who, _)| *who == cured)
                        && (action
                            .move_name
                            .as_ref()
                            .is_some_and(|m| reading.thawing_moves.contains(m))
                            || (from == "[msg]"
                                && berry_eater.as_ref() != Some(&cured)
                                && !own_action_follows(&events[position + 1..], &cured)))
                    {
                        action.thawed.push(cured);
                        continue;
                    }
                    // Thawed by the berry a Fling just threw at it (Aspear Berry: the flung
                    // berry's effect, `[msg]` like a thaw roll, before the thrower's
                    // `-enditem`): the frame checks it, no action of its own (R-M8-139). After
                    // that `-enditem`, or with no berry flung, it is the target's own thaw roll as
                    // its action starts (a Gen 9 Dunsparce after a Miracle Seed, R-M10-67).
                    if let Some(action) = out.last_mut()
                        && action.user != cured
                        && action.move_name.as_deref() == Some("Fling")
                        && action.damages.iter().any(|(who, _)| *who == cured)
                        && flung(&events[position + 1..], &event["pokemon"]).is_some()
                    {
                        continue;
                    }
                    // Gen 2: thawed by its own `defrost` move once it landed.
                    if out
                        .last()
                        .is_some_and(|action| action.user == cured && action.move_name.is_some())
                    {
                        continue;
                    }
                    // Gen 3+: its `defrost` move thaws it as the move starts (`[from] move:`),
                    // which is not the thaw roll.
                    if from.starts_with("[from] move:") {
                        current(&mut out, cured).defrosted = true;
                        continue;
                    }
                }
                // A wake-up or thaw of its own says `[msg]`; a `[silent]` cure is an effect's (Gen 1
                // Haze curing the foe, R-M8-93).
                let by_effect = announced_cure
                    .is_some_and(|side| who(&event["pokemon"]).is_some_and(|w| w.side == side))
                    || from == "[silent]"
                    || from.contains("ability:")
                    || from.contains("item:")
                    || (after_ability.is_some() && after_ability == who(&event["pokemon"]));
                // Cured by the move in progress (Purify on it; a Worry Seed Metronome called): the
                // sleep or freeze ended without a wake-up or thaw of its own. Not a status the
                // move does not cure: a freeze the Worry Seed target thaws from at its own
                // BeforeMove is its thaw (R-M8-103).
                // Nor a foe's cure within a move on the user's own side (a frozen foe thawing at
                // its BeforeMove after the user's Jungle Healing, R-M8-113), nor one after the
                // move failed (`-fail|user`: a Purify at a target with no status cured nothing;
                // the frozen foe that thaws next is thawing at its own BeforeMove, R-M11-55).
                if let Some(cured) = who(&event["pokemon"])
                    && let Some(action) = out.last_mut()
                    && action.user != cured
                    && !action.failed
                    && action
                        .used_move()
                        .and_then(|m| reading.curing_moves.get(m))
                        .is_some_and(|only| only.is_none_or(|only| only == status))
                    && !(cured.side != action.user.side
                        && action
                            .used_move()
                            .is_some_and(|m| reading.own_side_curing_moves.contains(m)))
                {
                    action.applied.push((cured, format!("curestatus:{status}")));
                    continue;
                }
                // Psycho Shift cures its user once the status passed on (`source.cureStatus()`,
                // `[msg]` like a wake-up): a sleeper's, through Sleep Talk, is the move's, not a
                // wake-up of its own (R-M8-141).
                if let Some(cured) = who(&event["pokemon"])
                    && out.last().is_some_and(|action| {
                        action.user == cured
                            && action.used_move().map(String::as_str) == Some("Psycho Shift")
                    })
                {
                    continue;
                }
                if matches!(status.as_str(), "slp" | "frz")
                    && !by_effect
                    && let Some(user) = who(&event["pokemon"])
                {
                    current(&mut out, user).cured = Some(status);
                }
            }
            "activate" | "end"
                if {
                    let args = event["args"].as_array().cloned().unwrap_or_default();
                    args.get(1)
                        .and_then(Value::as_str)
                        .is_some_and(|w| w.ends_with("Substitute"))
                } =>
            {
                // A hit absorbed by (or breaking) a Substitute is a landed hit without an HP
                // commit; it still consumed a critical-hit roll.
                if let Some(action) = out.last_mut() {
                    let holder = who_from_text(&event["args"][0]);
                    action.crits.push(
                        holder
                            .as_ref()
                            .is_some_and(|h| take_crit(&mut pending_crits, h)),
                    );
                    action.substitute_hits.push(true);
                    action.hit_targets.push(holder.clone());
                    action.hit_hp.push(None);
                    if let Some(holder) = holder {
                        action
                            .substitute_struck
                            .push((holder, event["kind"] == "end"));
                    }
                }
            }
            // Bide's release (`-end|user|move: Bide`, Gen 1 `-end|user|Bide`), not its silent
            // removal after: the holder's action, whose hit the damage that follows is.
            "end"
                if {
                    let args = event["args"].as_array().cloned().unwrap_or_default();
                    matches!(
                        args.get(1).and_then(Value::as_str),
                        Some("move: Bide" | "Bide")
                    ) && args.get(2).and_then(Value::as_str) != Some("[silent]")
                } =>
            {
                if let Some(user) = who_from_text(&event["args"][0]) {
                    current(&mut out, user).move_name = Some("Bide".into());
                }
            }
            "activate" if event["args"].get(1).and_then(Value::as_str) == Some("move: Powder") => {
                if let Some(user) = who_from_text(&event["args"][0])
                    && let Some(action) = out.last_mut()
                    && action.user == user
                {
                    action.cant = Some("move: Powder".into());
                    action.hit = false;
                    powder_blowup = Some(user);
                }
            }
            // A Pledge that waits does nothing at all (`-waiting`): no roll, no hit.
            "waiting" => {
                if let Some(action) = out.last_mut() {
                    action.failed = true;
                    action.no_target = true;
                    action.hit = false;
                }
            }
            "singleturn" => {
                if event["move"] == "move: Instruct" {
                    instructed_next = who(&event["pokemon"]);
                } else if let (Some(action), Some(user)) = (out.last_mut(), who(&event["pokemon"]))
                    && action.user == user
                {
                    action.protected = true;
                }
            }
            "activate" => {
                let args = event["args"].as_array().cloned().unwrap_or_default();
                let what = args.get(1).and_then(Value::as_str).unwrap_or("");
                // A priority chance of the action Instruct is building for its target.
                if instructed_next.is_some()
                    && args.first().and_then(who_from_text) == instructed_next
                    && let Some(chance) = announced_chance(what)
                {
                    instructed_chances.push(chance);
                }
                if what == "ability: Forewarn"
                    && let Some(action) = out.last_mut()
                    && (action.ability_traded
                        || matches!(
                            action
                                .called
                                .last()
                                .map(|called| called.name.as_str())
                                .or(action.move_name.as_deref()),
                            Some(
                                "Skill Swap" | "Role Play" | "Entrainment" | "Doodle" | "Transform"
                            )
                        ))
                {
                    action.forewarned.push(
                        args.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect(),
                    );
                }
                if let Some(action) = out.last_mut()
                    && args.first().and_then(who_from_text).as_ref() == Some(&action.user)
                {
                    match what {
                        "move: Magnitude" => {
                            action.magnitude = args
                                .get(2)
                                .and_then(Value::as_str)
                                .and_then(|n| n.parse().ok());
                        }
                        "move: Fickle Beam" => action.power_chance = true,
                        _ => {}
                    }
                }
                if what == "ability: Synchronize" {
                    synchronizing = args.first().and_then(who_from_text);
                }
                // Lightning Rod / Storm Drain drew the move whose `move` line came last
                // (`onAnyRedirectTarget` announces only a target it changed): the line names
                // the holder, and the move was aimed at another first (R-M11-7).
                if matches!(what, "ability: Lightning Rod" | "ability: Storm Drain")
                    && let Some(action) = out.last_mut()
                    && let Some(holder) = args.first().and_then(who_from_text)
                {
                    match action.called.last_mut() {
                        Some(called) => called.redirected_by = Some(holder),
                        None => action.redirected_by = Some(holder),
                    }
                }
                // A contact reaction taking the user's ability away for the rest of its move.
                if matches!(
                    what,
                    "ability: Mummy" | "ability: Lingering Aroma" | "ability: Wandering Spirit"
                ) && let Some(action) = out.last_mut()
                    && args.get(2).and_then(who_from_text).as_ref() == Some(&action.user)
                    && action.ability_taken_after.is_none()
                {
                    action.ability_taken_after = Some(action.crits.len());
                }
                // Wandering Spirit trades through Showdown's `skillSwap`, which names the attacker
                // first: `-activate|user|Skill Swap|…|[of] holder` — not the Skill Swap move's
                // own trade (R-M8-133).
                if what == "Skill Swap"
                    && let Some(action) = out.last_mut()
                    && action.move_name.as_deref() != Some("Skill Swap")
                    && args.first().and_then(who_from_text).as_ref() == Some(&action.user)
                    && action.ability_taken_after.is_none()
                {
                    action.ability_taken_after = Some(action.crits.len());
                    action.ability_traded = true;
                }
                // A Dancer about to dance along: its dance (or the BeforeMove that stops it) is
                // an action of its own.
                if what == "ability: Dancer"
                    && let Some(dancer) = args.first().and_then(who_from_text)
                {
                    let mut dance = ActionOracle::new(dancer);
                    dance.danced = true;
                    out.push(dance);
                }
                if let Some(name) = what.strip_prefix("ability: ") {
                    ability_announced = reading
                        .curing_abilities
                        .contains(name)
                        .then(|| args.first().and_then(who_from_text))
                        .flatten();
                }
                if what == "move: Spite"
                    && let Some(action) = out.last_mut()
                    && action.used_move().is_some_and(|name| name == "Spite")
                {
                    action.spite_drop = args
                        .get(3)
                        .and_then(Value::as_str)
                        .and_then(|n| n.parse().ok());
                }
                if what == "item: Focus Band"
                    && let (Some(action), Some(holder)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                {
                    let hits = action.crits.len();
                    action.focus_band_held.push((holder, hits));
                }
                if what == "move: Pursuit" {
                    pursuit_intercept = true;
                }
                // Bide storing energy (`-activate|user|move: Bide`, Gen 1 `-activate|user|Bide`):
                // the holder's action, with no move line.
                if matches!(what, "move: Bide" | "Bide")
                    && args.len() == 2
                    && let Some(user) = args.first().and_then(who_from_text)
                {
                    current(&mut out, user).move_name = Some("Bide".into());
                }
                if what == "confusion"
                    && let Some(user) = args.first().and_then(who_from_text)
                {
                    sleeps_through(&mut out, &user);
                    current(&mut out, user);
                }
                // `-activate|target|move: Bind|[of] source`: the target got bound by this hit.
                // (`-activate|p|move: Attract|[of] …` is an infatuated Pokémon's BeforeMove.)
                if what.starts_with("move: ")
                    && what != "move: Attract"
                    && args
                        .get(2)
                        .and_then(Value::as_str)
                        .is_some_and(|of| of.starts_with("[of]"))
                    && let (Some(action), Some(target)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                {
                    if action.damages.iter().any(|(t, _)| *t == target) {
                        action.secondary_applied = true;
                        action.secondary_on.push((
                            target.clone(),
                            "start:partiallytrapped".to_string(),
                            action.crits.len(),
                        ));
                    }
                    action
                        .applied
                        .push((target, "start:partiallytrapped".to_string()));
                }
                if matches!(
                    what,
                    "move: Heal Bell" | "move: Aromatherapy" | "ability: Shed Skin"
                ) {
                    announced_cure = args.first().and_then(who_from_text).map(|w| w.side);
                }
                // Protect / Detect / Telepathy stopped the move at TryHit on this target:
                // like an immunity, no accuracy, critical or damage decision is drawn for it.
                // (Gen 2-4 report the guard without the `move: ` prefix.)
                if matches!(
                    what.strip_prefix("move: ").unwrap_or(what),
                    "Protect"
                        | "Detect"
                        // Max Guard (DESIGN.md §44.5): Dynamax's protection, `-activate|p|move:
                        // Max Guard`.
                        | "Max Guard"
                        | "Wide Guard"
                        | "Quick Guard"
                        | "Crafty Shield"
                        | "Mat Block"
                        | "Psychic Terrain"
                        | "ability: Telepathy"
                        | "item: Safety Goggles"
                ) && let (Some(action), Some(target)) =
                    (out.last_mut(), args.first().and_then(who_from_text))
                    && target != action.user
                {
                    action.guarded.push(target.clone());
                    action.try_hit_blocked.push(target.clone());
                    action.immune.push(target);
                }
            }
            "crit" => {
                if let Some(target) = who(&event["pokemon"]) {
                    pending_crits.push(target);
                }
            }
            // Lightning Rod / Storm Drain / Motor Drive / Well-Baked Body / Sap Sipper / Wind Rider
            // took the move instead of being hit (`-ability|target|Lightning Rod|boost`): like an
            // immunity, the target draws no decision.
            "ability" => {
                let args = event["args"].as_array().cloned().unwrap_or_default();
                let ability = args.get(1).and_then(Value::as_str).unwrap_or("");
                // `-ability|p|New|Old|[from] …` changes an ability (Role Play, Trace …): it is
                // no absorption, and no Mirror Armor sending a drop back.
                let changes_ability = args
                    .iter()
                    .any(|arg| arg.as_str().is_some_and(|arg| arg.starts_with("[from]")));
                if !changes_ability
                    && matches!(
                        ability,
                        "Lightning Rod"
                            | "Storm Drain"
                            | "Motor Drive"
                            | "Well-Baked Body"
                            | "Sap Sipper"
                            | "Wind Rider"
                    )
                    && let (Some(action), Some(target)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                    && target != action.user
                    && action.move_name.is_some()
                {
                    action.hit = false;
                    action.try_hit_blocked.push(target.clone());
                    action.immune.push(target);
                }
                if ability == "Cotton Down" {
                    cotton_down = true;
                }
                if ability == "Sturdy"
                    && args.len() == 2
                    && let (Some(action), Some(holder)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                {
                    action.sturdy_held.push(holder);
                }
                if args.get(2).and_then(Value::as_str) == Some("boost")
                    && !matches!(
                        ability,
                        "Lightning Rod"
                            | "Storm Drain"
                            | "Motor Drive"
                            | "Well-Baked Body"
                            | "Sap Sipper"
                            | "Wind Rider"
                    )
                {
                    ability_stat_change = args.first().and_then(who_from_text);
                }
                // Mirror Armor sent a stat drop back (`-ability|target|Mirror Armor`): on a
                // target the move damaged, that drop was the move's secondary landing, though
                // the protocol then shows it on the user or, when the user refuses it too, not
                // at all.
                if ability == "Mirror Armor"
                    && !changes_ability
                    && let (Some(action), Some(target)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                    && target != action.user
                    && action.damages.iter().any(|(t, _)| *t == target)
                {
                    action.secondary_applied = true;
                    let hits = action.crits.len();
                    action
                        .secondary_on
                        .push((target, "reflected".to_string(), hits));
                }
            }
            // `-fail|target|heal`: a move's heal found its target at full HP (Present's heal;
            // Gen 2's `moveHit` prints a bare `-fail|target`, below).
            "fail" if event["detail"].as_str() == Some("heal") => {
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"])) {
                    action.applied.push((target, "fail:heal".to_string()));
                }
            }
            "fail" => {
                // `-fail|user|tox`: a status bounced back on the user (Synchronize) found it
                // already statused — the user's move did not fail. Showdown `setStatus` prints
                // that when the status is the same; a different one it refuses with a bare
                // `-fail|user` (and `[still]` on the user's move line) right after the user's
                // own Synchronize announced itself. So is `-fail|user|slp|[from] Uproar`: the
                // sleep a reaction to its hit (Effect Spore) would have put on it, refused by an
                // uproar (§15.12).
                let status_refused = matches!(
                    event["detail"]
                        .as_str()
                        .and_then(|detail| detail.split('|').next()),
                    Some("psn" | "tox" | "brn" | "par" | "slp" | "frz")
                );
                // `-fail|user|unboost|[from] ability: Clear Body`: a stat drop on the user was
                // refused (the Intimidate of a Pokémon its Dragon Tail dragged in), not its move.
                let stat_change_refused = event["detail"].as_str().is_some_and(|detail| {
                    detail.starts_with("unboost") || detail.starts_with("boost")
                });
                // A bare `-fail|target` on someone else: the move's effect failed on that target
                // (Gen 2's `moveHit`: Present's heal at full HP).
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"]))
                    && action.user != target
                    && action.move_name.is_some()
                    && event["detail"].is_null()
                {
                    action.applied.push((target, "fail".to_string()));
                }
                if let (Some(action), Some(user)) = (out.last_mut(), who(&event["pokemon"]))
                    && action.user == user
                    && action.move_name.is_some()
                    && !status_refused
                    && !stat_change_refused
                    && after_synchronize.as_ref() != Some(&user)
                {
                    action.failed = true;
                    if event["detail"]
                        .as_str()
                        .is_some_and(|detail| detail.starts_with("move: "))
                    {
                        action.fail_named_the_move = true;
                    }
                }
            }
            "disableslot" => {
                if let Some(action) = out.last_mut() {
                    action.disabled_slot = event["slot"].as_u64().map(|slot| slot as usize);
                }
            }
            // No Pokémon left to aim at (`-notarget`: the target fainted earlier in the turn and
            // nobody replaced it): the move draws nothing, like a failure.
            "notarget" => {
                if let (Some(action), Some(user)) = (out.last_mut(), who(&event["pokemon"]))
                    && action.user == user
                {
                    action.failed = true;
                    action.no_target = true;
                    action.hit = false;
                }
            }
            // A format clause refusing the status the action rolled (Freeze Clause Mod, Sleep
            // Clause Mod, M9.5): the roll landed, and the clause stopped it.
            "message" => {
                let status = match event["text"].as_str().unwrap_or("") {
                    "Freeze Clause activated." => Some("frz"),
                    "Sleep Clause Mod activated." => Some("slp"),
                    _ => None,
                };
                if let (Some(status), Some(action)) = (status, out.last_mut())
                    && let Some(target) = action
                        .damages
                        .last()
                        .map(|(target, _)| target.clone())
                        .or_else(|| action.move_target.clone())
                {
                    action.secondary_applied = true;
                    let hits = action.crits.len();
                    action
                        .secondary_on
                        .push((target, format!("status:{status}"), hits));
                }
            }
            // Gen 1-3 print no `-fail` when a move's `onTry` fails; Fake Out leaves its hint.
            // Gen 1: a confusion self-hit behind a Substitute that hurts nothing on the user's
            // side is only a hint too. The same hint follows a crash behind a Substitute (Hi
            // Jump Kick), which comes after the move line; a self-hit comes before any.
            "hint" => {
                let text = event["text"].as_str().unwrap_or("");
                if let Some(action) = out.last_mut() {
                    if action.move_name.is_some()
                        && text.contains("only works on your first turn out")
                    {
                        action.failed = true;
                    }
                    if action.move_name.is_some() && text.ends_with(" Shell Side Arm") {
                        action.physical_hint = Some(text.starts_with("Physical"));
                    }
                    if action.move_name.is_none() && text.contains("hurts itself due to confusion")
                    {
                        action.cant = Some("confusion".into());
                        action.hit = false;
                    }
                }
            }
            // A drag after the residual (a Red Card a foreseen attack set off) is the residual's
            // (`residual_drag_tape`), not the last action's (R-M8-141).
            "drag" if in_residual => {}
            "drag" => {
                if let (Some(action), Some(entrant)) = (out.last_mut(), who(&event["pokemon"])) {
                    action.dragged_in.push(entrant);
                    action
                        .berry_draws_at_drag
                        .get_or_insert(action.berry_draws.len());
                    dragging = true;
                }
            }
            "hitcount" => {
                if let Some(action) = out.last_mut() {
                    action.hit_count = event["count"].as_u64().map(|n| n as u16);
                }
            }
            "miss" => {
                if let Some(action) = out.last_mut() {
                    action.hit = false;
                    if let Some(target) = who_from_text(&event["detail"]) {
                        action.missed.push(target);
                    }
                }
            }
            "immune" => {
                if let Some(action) = out.last_mut() {
                    let immune = who(&event["pokemon"]);
                    // The user shrugging off a reaction to its own move (Synchronize passing a
                    // paralysis back to an Electric type) is not the move being blocked.
                    let reaction_on_user = immune.as_ref().is_some_and(|who| {
                        *who == action.user && action.move_target.as_ref() != Some(who)
                    });
                    // Nor is one on a target the move already hit (Synchronize passing the
                    // attacker's Static paralysis on to an Electric target).
                    let already_hit = immune
                        .as_ref()
                        .is_some_and(|who| action.damages.iter().any(|(hit, _)| hit == who));
                    // Own Tempo's `onHit` (`-immune|p|confusion|[from] ability: Own Tempo`, the
                    // one `-immune` naming an effect) speaks once the move landed: Swagger's
                    // boost stands, and its accuracy was drawn (DESIGN.md R-M9-29). So did a
                    // move that dragged a Pokémon in: an `-immune` after the drag is the
                    // entrant's entry (its Intimidate meeting a Substitute), the move's last
                    // effect long done (R-M10-9).
                    let landed = dragging
                        || event["detail"]
                            .as_str()
                            .is_some_and(|detail| detail.starts_with("confusion|"));
                    if !reaction_on_user && !already_hit && !landed {
                        action.hit = false;
                        if let Some(target) = immune {
                            // An absorbing ability's refusal (`[from] ability: Volt Absorb` at
                            // full HP) is its TryHit; Levitate's is the natural immunity.
                            if event["detail"].as_str().is_some_and(|d| {
                                d.contains("[from] ability:") && !d.contains("Levitate")
                            }) {
                                action.try_hit_blocked.push(target.clone());
                            }
                            action.immune.push(target);
                        }
                    }
                }
            }
            "damage"
                if powder_blowup.is_some()
                    && powder_blowup.as_ref() == who(&event["pokemon"]).as_ref() =>
            {
                let hp = event["hp"].as_u64().unwrap_or(0) as u16;
                if let (Some(action), Some(target)) = (out.last_mut(), powder_blowup.take()) {
                    action.damages.push((target, hp));
                }
            }
            "damage" => {
                let hp = event["hp"].as_u64().unwrap_or(0) as u16;
                let from = event["from"].as_str();
                // Gen 1-2: a Pokémon whose turn shows nothing (Gen 1: held by a trapping move that
                // goes on) still takes its after-own-action residual (Leech Seed, poison, burn) as
                // its turn ends: that is its own silent action, not the one before (R-M8-93);
                // first in the turn too, ahead of a tied foe's move (R-M8-108) — not right after
                // an entry (a Baton Pass's: the passer's own residual on its replacement).
                // Gen 2's own switch-in residual (`onAfterSwitchInSelf` of poison and burn) on the
                // Pokémon a Pursuit held back as it came in: the pursuer's action, as its
                // hazards are (R-M8-168).
                let switch_in_residual = status_after_action
                    && from.is_some_and(|f| f == "[from] psn" || f == "[from] brn");
                if status_after_action
                    && let Some(target) = who(&event["pokemon"])
                    && pursued_entrant.as_ref() != Some(&target)
                    && out
                        .last()
                        .map_or(entrant.is_none(), |action| action.user != target)
                    && !out.iter().any(|action| action.user == target)
                    && from.is_some_and(|f| {
                        f == "[from] psn" || f == "[from] brn" || f.contains("Leech Seed")
                    })
                {
                    current(&mut out, target.clone()).damages.push((target, hp));
                    continue;
                }
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"])) {
                    let hazard = from
                        .is_some_and(|f| ["Stealth Rock", "Spikes"].iter().any(|h| f.contains(h)));
                    let pursued = pursued_entrant.as_ref() == Some(&target);
                    if ((dragging || pursued) && hazard) || (pursued && switch_in_residual) {
                        action.damages.push((target, hp));
                    } else if from.is_some_and(|f| f.contains("confusion")) {
                        action.damages.push((target, hp));
                        action.cant = Some("confusion".into());
                        action.hit = false;
                    } else if target == action.user
                        && action
                            .used_move()
                            .is_some_and(|name| reading.crashing_moves.contains(name))
                        && from.is_none_or(|f| {
                            reading.crashing_moves.iter().any(|name| {
                                f == format!("[from] {}", pokemon_battle_engine::data::to_id(name))
                            })
                        })
                    {
                        // The move's crash (Axe Kick's is `[from] highjumpkick`).
                        action.damages.push((target, hp));
                        action.crashed = true;
                    } else if from.is_none()
                        && action.called.last().is_some_and(|called| {
                            called.user == target && called.user != action.user
                        })
                    {
                        // What a move another Pokémon took over costs its new user (a snatched
                        // Belly Drum or Substitute: `directDamage`, no `[from]`): its own HP cost,
                        // as the actor's own would be — no hit (R-M8-79).
                    } else if from.is_some_and(|f| f.starts_with("[from] pokemon: ")) {
                        // A busted Disguise's price (`this.damage`, `[from] pokemon:
                        // Mimikyu-Busted`) right after the hit that broke it: an HP commit of
                        // the action, no hit (DESIGN.md R-M8-91).
                        action.damages.push((target, hp));
                    } else if from.is_none() && target != action.user {
                        action.damages.push((target.clone(), hp));
                        action.crits.push(take_crit(&mut pending_crits, &target));
                        action.substitute_hits.push(false);
                        action.hit_targets.push(Some(target.clone()));
                        action.hit_hp.push(Some(hp));
                    } else if target != action.user
                        && action.used_move().is_some_and(|name| {
                            from == Some(
                                format!("[from] {}", pokemon_battle_engine::data::to_id(name))
                                    .as_str(),
                            )
                        })
                    {
                        // The move's own damage on someone it did not hit (Flame Burst's
                        // splash on the target's partner: `[from] flameburst|[of] user`).
                        action.damages.push((target, hp));
                    } else if target == action.user
                        // (Struggle's recoil is `[from] recoil`, lowercase.)
                        && (from.is_some_and(|f| f.eq_ignore_ascii_case("[from] recoil"))
                            || from.is_some_and(|f| f.contains("item: Life Orb"))
                            // The move's own after-move self damage (Mind Blown, Steel Beam:
                            // `[from] mindblown`), a called one's too.
                            || action.used_move().is_some_and(|name| {
                                from == Some(
                                    format!("[from] {}", pokemon_battle_engine::data::to_id(name))
                                        .as_str(),
                                )
                            })
                            // Gen 2: poison / burn, Leech Seed, Nightmare and Curse right after
                            // the user's own action (`onAfterMoveSelf`).
                            || (status_after_action
                                && from.is_some_and(|f| {
                                    f == "[from] psn"
                                        || f == "[from] brn"
                                        || f.contains("Leech Seed")
                                        || f == "[from] Nightmare"
                                        || f == "[from] Curse"
                                }))
                            || (event["of"].as_str().is_some()
                                && from.is_some_and(|f| {
                                    f.contains("ability:")
                                        || f.contains("item:")
                                        // A protection's contact punishment.
                                        || f == "[from] Spiky Shield"
                                })))
                    {
                        // Recoil, Life Orb, the move's own self damage, or a reaction the target
                        // dealt back (`[from]
                        // ability: Rough Skin|[of] …`): the action's own HP commits on its
                        // user. Residual drains come after the `residual` marker and are
                        // skipped above.
                        action.damages.push((target, hp));
                    }
                }
            }
            "boost" | "unboost"
                if ability_boosting.is_some() && ability_boosting == who(&event["pokemon"]) =>
            {
                ability_stat_change = ability_boosting;
            }
            "unboost" if cotton_down_drops => cotton_down = true,
            // Gen 1's raise of a stat at 999 (a drop of one at 1) prints the change and then its
            // echo the other way by 1 (the gen1 `boost`): the echo is the same change's, no effect
            // of its own — not an Aurora Beam drop after a Rage boost (R-M9-42).
            "boost" | "unboost"
                if position > 0 && {
                    let previous = &events[position - 1];
                    let raised = previous["kind"] == "boost";
                    previous["kind"] != event["kind"]
                        && matches!(previous["kind"].as_str(), Some("boost" | "unboost"))
                        && previous["pokemon"] == event["pokemon"]
                        && previous["stat"] == event["stat"]
                        && event["amount"].as_u64() == Some(1)
                        && who(&event["pokemon"]).is_some_and(|target| {
                            stat_stuck(&target, event["stat"].as_str().unwrap_or(""), raised)
                        })
                } => {}
            "block"
                if event["detail"]
                    .as_str()
                    .is_some_and(|detail| detail.starts_with("ability: Sweet Veil")) =>
            {
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"]))
                    && target == action.user
                    && action.move_name.is_some()
                    && !action.crits.is_empty()
                {
                    action.contact_sleep_blocked.push(action.crits.len());
                }
                // On a target the move hit: its secondary's sleep was drawn, and stopped (Relic
                // Song's 10% on a Sweet Veil holder's partner, R-M11-43).
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"]))
                    && target != action.user
                    && action.move_name.is_some()
                    && action.damages.iter().any(|(hit, _)| *hit == target)
                {
                    action.secondary_applied = true;
                    let hits = action.crits.len();
                    action
                        .secondary_on
                        .push((target, "status:slp".to_string(), hits));
                }
            }
            "status" | "boost" | "unboost" | "heal" => {
                let kind = event["kind"].as_str().unwrap_or("");
                if let (Some(action), Some(target)) = (out.last_mut(), who(&event["pokemon"])) {
                    // The target's ability statused the actor on contact (Static …). Gen 3-4
                    // print Effect Spore's sleep bare (their `slp` names no ability): a sleep on
                    // the actor after one of its own hits is that.
                    let bare_sleep_after_a_hit = event["from"].is_null()
                        && event["status"].as_str() == Some("slp")
                        && action.move_name.is_some()
                        && !action.crits.is_empty();
                    if kind == "status"
                        && target == action.user
                        && (event["from"]
                            .as_str()
                            .is_some_and(|f| f.contains("ability:"))
                            || bare_sleep_after_a_hit)
                    {
                        action.contact_status = Some(text(&event["status"]));
                        action.contact_status_after_hits = action.crits.len();
                        action.contact_status_hits.push(action.crits.len());
                        action.contact_status_of = event["of"].as_str().and_then(|of| {
                            who_from_text(&Value::String(of.trim_start_matches("[of] ").into()))
                        });
                    }
                    // Drain and self-heal moves commit HP the engine traces as HealApplied, and so
                    // does an absorbing ability on a target (Water Absorb, Volt Absorb), which
                    // also stops the move there like an immunity. Residual heals come after the
                    // `residual` marker and are skipped above.
                    if kind == "heal" {
                        let from = event["from"].as_str();
                        let absorbed = from.is_some_and(|f| {
                            f.contains("ability: Water Absorb")
                                || f.contains("ability: Volt Absorb")
                                || f.contains("ability: Dry Skin")
                                || f.contains("ability: Earth Eater")
                        });
                        // Gen 2's Leech Seed heals the seeder silently within the action.
                        let seeded = status_after_action && from == Some("[silent]");
                        // A berry (or Berry Juice) an `Update` inside the action fed its holder
                        // (a recording without the residual marker shows Leftovers here too);
                        // Shell Bell's heal after the user's move (DESIGN.md R232).
                        let item = from.is_some_and(|f| {
                            f.strip_prefix("[from] item: ").is_some_and(|item| {
                                item.ends_with(" Berry")
                                    || item == "Berry Juice"
                                    || item == "Shell Bell"
                            })
                        });
                        // Rest heals its user silently (DESIGN.md R170) — a called one too
                        // (R-M8-70), a snatched one its snatcher.
                        let rested = from == Some("[silent]")
                            && (target == action.user
                                || action
                                    .called
                                    .last()
                                    .is_some_and(|called| called.user == target))
                            && action.used_move().is_some_and(|name| name == "Rest");
                        // Cheek Pouch after a berry the action's `Update` fed its holder.
                        let pouch = from == Some("[from] ability: Cheek Pouch");
                        // Hospitality a Transform (or another ability change) started within
                        // the action heals the partner (R-M8-84).
                        let hospitality = from == Some("[from] ability: Hospitality");
                        // A wish waiting on a slot an Ally Switch moved a Pokémon into (Gen 8+
                        // `onSwap`, R-M8-86).
                        let wished = matches!(
                            from,
                            Some("[from] move: Lunar Dance" | "[from] move: Healing Wish")
                        );
                        // A Z status move's own heal (Z-Refresh's `zMove.effect: 'heal'`) as the
                        // move begins (DESIGN.md R-M9-22).
                        let z_effect = from == Some("[zeffect]");
                        if from.is_none()
                            || from.is_some_and(|f| f.contains("drain"))
                            || absorbed
                            || seeded
                            || item
                            || rested
                            || pouch
                            || hospitality
                            || wished
                            || z_effect
                        {
                            let hp = event["hp"].as_u64().unwrap_or(0) as u16;
                            action.damages.push((target.clone(), hp));
                        }
                        // Stopped there, by the move it absorbed: one sent back at the action's
                        // own user (Magic Bounce) is its reflector's, so the user absorbs it too.
                        let mover = action.called.last().map_or(&action.user, |c| &c.user);
                        if absorbed && target != *mover {
                            action.try_hit_blocked.push(target.clone());
                            action.immune.push(target.clone());
                        }
                    }
                    // A stat change names its stat: Flash Cannon's Special Defense drop is not the
                    // Attack boost a Contrary target takes from an Intimidate in the same action.
                    let label = match kind {
                        "status" => format!("status:{}", text(&event["status"])),
                        "boost" | "unboost" => format!("{kind}:{}", text(&event["stat"])),
                        // A heal from something else within the action (Gen 2's Leech Seed after
                        // the user's move heals the seeder `[silent]`) is not the move's own.
                        "heal" if !event["from"].is_null() => "heal:from".to_string(),
                        other => other.to_string(),
                    };
                    // Poison Touch / Toxic Chain on a target: the user's reaction to the hit, not
                    // the move's own (a Poison Tail's poison, R-M10-41).
                    let dealt = kind == "status"
                        && target != action.user
                        && event["from"].as_str().is_some_and(|f| {
                            f.contains("ability: Poison Touch")
                                || f.contains("ability: Toxic Chain")
                        });
                    if dealt {
                        action
                            .dealt_status
                            .push((target.clone(), action.crits.len()));
                    }
                    // No status an ability gave is the move's roll: a Synchronize sending the
                    // user's status back at the target the move hit (a Gen 3 Kadabra paralysed
                    // by Static after Thunder Punch, R-M10-60) among them.
                    let by_ability = kind == "status"
                        && event["from"]
                            .as_str()
                            .is_some_and(|f| f.contains("ability: "));
                    let item_stat = matches!(kind, "boost" | "unboost")
                        && item_boosting.as_ref() == Some(&target);
                    if item_stat {
                        item_stat_change = item_boosting.clone();
                    }
                    // A drag ends the move's effects (Red Card's after them all, Dragon Tail's and
                    // Roar's after the hit): a stat change after it is the entrant's entry — its
                    // Intimidate — not the move's secondary nor its own on the user (R-M11-42).
                    let entry = dragging && matches!(kind, "boost" | "unboost");
                    if action.damages.iter().any(|(t, _)| *t == target)
                        && kind != "heal"
                        && !item_stat
                        && !by_ability
                        && !entry
                    {
                        action.secondary_applied = true;
                        let hits = action.crits.len();
                        action
                            .secondary_on
                            .push((target.clone(), label.clone(), hits));
                    }
                    if target == action.user && !entry {
                        let hits = action.crits.len();
                        action.user_applied_hits.push((label.clone(), hits));
                    }
                    action.applied.push((target, label));
                }
            }
            "start" => {
                let args = event["args"].as_array().cloned().unwrap_or_default();
                // Flash Fire took the move (`-start|target|ability: Flash Fire`): like an
                // absorbing ability's `-ability … boost`, the target draws no decision
                // (DESIGN.md R172). The user's own Flash Fire too when the move in progress is
                // aimed at it — Magic Bounce or Magic Coat sent its Will-O-Wisp back (R-M9-54).
                if args.get(1).and_then(Value::as_str) == Some("ability: Flash Fire")
                    && let (Some(action), Some(target)) =
                        (out.last_mut(), args.first().and_then(who_from_text))
                    && (target != action.user || action.move_target.as_ref() == Some(&target))
                    && action.move_name.is_some()
                {
                    action.hit = false;
                    action.try_hit_blocked.push(target.clone());
                    action.immune.push(target);
                }
                if let (Some(action), Some(target)) =
                    (out.last_mut(), args.first().and_then(who_from_text))
                {
                    let what = args.get(1).and_then(Value::as_str).unwrap_or("");
                    // Gen 1 Mimic learnt a move (`-start|p|Mimic|Metronome`): which of the
                    // target's slots it drew, read off the move it learned.
                    if what == "Mimic" {
                        action.mimicked = args.get(2).and_then(Value::as_str).map(str::to_owned);
                        continue;
                    }
                    // Uproar starting on its user: the move's own lock, no secondary.
                    if what == "Uproar" && target == action.user {
                        continue;
                    }
                    // A lock's fatigue (Thrash and its kind): the holder's own confusion, no
                    // effect of the move's. Two locks may end on the same turn, so the line's
                    // Pokémon is matched to *its* action, not the last one (DESIGN.md R-M8-44).
                    if what == "confusion"
                        && args
                            .iter()
                            .any(|arg| matches!(arg.as_str(), Some("[fatigue]" | "[silent]")))
                    {
                        let owner = out
                            .iter()
                            .rposition(|action| action.user == target)
                            .unwrap_or(out.len().saturating_sub(1));
                        out[owner].fatigue = true;
                        continue;
                    }
                    // Cursed Body on the user: the target's reaction to the hit.
                    if what == "Disable"
                        && target == action.user
                        && args
                            .iter()
                            .any(|arg| arg.as_str() == Some("[from] ability: Cursed Body"))
                    {
                        let of = args.iter().find_map(|arg| {
                            arg.as_str()?
                                .strip_prefix("[of] ")
                                .map(|of| who_from_text(&Value::String(of.to_string())))
                        });
                        action.cursed_body = Some((action.crits.len(), of.flatten()));
                    }
                    // Cute Charm on the user: the target's reaction to the hit.
                    if what == "Attract"
                        && target == action.user
                        && args
                            .iter()
                            .any(|arg| arg.as_str() == Some("[from] ability: Cute Charm"))
                    {
                        let of = args.iter().find_map(|arg| {
                            arg.as_str()?
                                .strip_prefix("[of] ")
                                .map(|of| who_from_text(&Value::String(of.to_string())))
                        });
                        action.cute_charm = Some((action.crits.len(), of.flatten()));
                        action.cute_charm_hits.push(action.crits.len());
                    }
                    // (Or past a doll the hit left standing: Gen 1's confusion, R-M8-93.)
                    if action.damages.iter().any(|(t, _)| *t == target)
                        || action
                            .substitute_struck
                            .iter()
                            .any(|(struck, broken)| *struck == target && !broken)
                    {
                        action.secondary_applied = true;
                        action.secondary_on.push((
                            target.clone(),
                            format!("start:{what}"),
                            action.crits.len(),
                        ));
                    }
                    if what == "Disable" {
                        action.disabled_move =
                            args.get(2).and_then(Value::as_str).map(str::to_owned);
                    }
                    if what == "typechange"
                        && !args.iter().any(|arg| {
                            arg.as_str().is_some_and(|a| {
                                a == "[silent]" || a.starts_with("[from] ability:")
                            })
                        })
                        && let Some(type_name) = args.get(2).and_then(Value::as_str)
                    {
                        action
                            .typechanges
                            .push((target.clone(), type_name.to_owned()));
                    }
                    if target == action.user {
                        let hits = action.crits.len();
                        action
                            .user_applied_hits
                            .push((format!("start:{what}"), hits));
                    }
                    action.applied.push((target, format!("start:{what}")));
                }
            }
            // `-item|user|item|[from] move: Thief|[of] target`: Gen 2 Thief's secondary took the
            // target's item; `[from] ability: Magician`: the actor's Magician did.
            "item" => {
                let args = event["args"].as_array().cloned().unwrap_or_default();
                let from = |source: &str| args.iter().any(|arg| arg.as_str() == Some(source));
                let of = args.iter().find_map(|arg| {
                    arg.as_str()?
                        .strip_prefix("[of] ")
                        .and_then(|of| who_from_text(&Value::String(of.to_string())))
                });
                if from("[from] ability: Magician")
                    && let Some(action) = out.last_mut()
                {
                    action.magician_from = of.clone();
                }
                if from("[from] move: Thief")
                    && let Some(target) = of
                    && let Some(action) = out.last_mut()
                {
                    action.secondary_applied = true;
                    action
                        .secondary_on
                        .push((target, "stolen".to_string(), action.crits.len()));
                }
            }
            _ => {}
        }
    }
    // A flinch is silent in the protocol (no `-start`), so the secondary that caused it is only
    // visible through the victim's later `cant flinch`: mark the most recent action that
    // damaged the victim.
    for index in 0..out.len() {
        if out[index].cant.as_deref() != Some("flinch") {
            continue;
        }
        let victim = out[index].user.clone();
        let (earlier, _) = out.split_at_mut(index);
        if let Some(cause) =
            flinch_cause(earlier, &victim, reading, adds_flinch).map(|at| &mut earlier[at])
        {
            cause.secondary_applied = true;
            cause
                .secondary_on
                .push((victim, "flinch".to_string(), usize::MAX));
        }
    }
    out
}

/// Which of `actions` flinched `victim`: the last hit on it that can flinch — a flinching
/// move (the one the action used: Copycat's Bite, R-M8-86), or one its user's item or ability
/// adds a flinch to (`adds_flinch`: King's Rock only on the Gen 2-4 list, R-M9-34); failing
/// that, the last hit on it.
pub(super) fn flinch_cause(
    actions: &[ActionOracle],
    victim: &Who,
    reading: &LogReading,
    adds_flinch: &dyn Fn(&Who, Option<&str>) -> bool,
) -> Option<usize> {
    let hit_victim =
        |action: &ActionOracle| action.damages.iter().any(|(target, _)| target == victim);
    actions
        .iter()
        .rposition(|action| {
            hit_victim(action)
                && (action
                    .used_move()
                    .is_some_and(|name| reading.flinching_moves.contains(name))
                    || adds_flinch(&action.user, action.used_move().map(String::as_str)))
        })
        .or_else(|| actions.iter().rposition(hit_victim))
}

/// A flinch is silent; besides the victim's later `cant flinch`, a frame taken before the turn
/// ends (a mid-turn request) shows it on a victim that had already moved, and a Gen 1 frame by
/// the recharge it cleared. Only a hit after the victim's own action can have left it there (an
/// earlier one would have stopped that action).
pub(super) fn attribute_frame_flinches(
    decisions: &mut [ActionOracle],
    frame: &Value,
    reading: &LogReading,
    adds_flinch: &dyn Fn(&Who, Option<&str>) -> bool,
) {
    let mut victims: Vec<Who> = Vec::new();
    for (victim, _) in decisions.iter().flat_map(|action| &action.damages) {
        if !victims.contains(victim) {
            victims.push(victim.clone());
        }
    }
    for victim in victims {
        let side = &frame["sides"][victim.side];
        let volatiles = victim
            .slot
            .and_then(|slot| side["active"][slot].as_u64())
            .map(|party| &side["party"][party as usize]["volatiles"]);
        // A victim whose own action something checked before the flinch stopped (recharge 11,
        // sleep and freeze 10, Truant 9; the flinch is 8 in `BeforeMove`) never ran the flinch's
        // `onBeforeMove`: a hit before that action may have left it too (DESIGN.md R-M9-28).
        let since = decisions
            .iter()
            .rposition(|action| action.user == victim)
            .map_or(0, |at| {
                let stopped_first = decisions[at].cant.as_deref().is_some_and(|reason| {
                    matches!(reason, "recharge" | "slp" | "frz" | "ability: Truant")
                });
                if stopped_first { 0 } else { at + 1 }
            });
        // Gen 1: a flinch clears the recharge the victim's own recharge move left this boundary
        // (`flinch.onStart`): the frame shows no `mustrecharge` after a later flinching hit. A
        // recharge the move never left (a knock-out, a miss) makes the flinch as invisible as it
        // is to a Pokémon that already moved (R-M8-105).
        let recharge_cleared = reading.flinch_clears_recharge
            && volatiles.is_some_and(|volatiles| volatiles.get("mustrecharge").is_none())
            && decisions[..since].iter().any(|action| {
                action.user == victim
                    && action
                        .used_move()
                        .is_some_and(|name| reading.recharge_moves.contains(name))
            })
            && decisions[since..].iter().any(|action| {
                action.damages.iter().any(|(target, _)| *target == victim)
                    && action
                        .used_move()
                        .is_some_and(|name| reading.flinching_moves.contains(name))
            });
        let flinched = volatiles.is_some_and(|volatiles| !volatiles["flinch"].is_null());
        if !(flinched || recharge_cleared) {
            continue;
        }
        let later = &mut decisions[since..];
        if let Some(cause) =
            flinch_cause(later, &victim, reading, adds_flinch).map(|at| &mut later[at])
            && !cause
                .secondary_on
                .iter()
                .any(|(who, label, _)| *who == victim && label == "flinch")
        {
            cause
                .secondary_on
                .push((victim, "flinch".to_string(), usize::MAX));
        }
    }
}

/// What Showdown reported for the end of the turn that needs a decision: who Shed Skin cured
/// (`-activate|p|ability: Shed Skin`), who thawed in the residual (Gen 2:
/// `-curestatus|p|frz|[msg]` after the turn's `residual` marker), who fell asleep there
/// (`-status|p|slp`: Yawn ran out), whom a Healer cured (the `-curestatus` right after its
/// `-activate|p|ability: Healer`), and who Harvest gave its berry back
/// (`-item|p|X|[from] ability: Harvest`).
pub fn extract_residual_cures(events: &[Value]) -> Vec<(Who, ResidualKind)> {
    let mut out = Vec::new();
    let mut in_residual = false;
    let mut healer_activated = false;
    for event in events {
        let healer = std::mem::take(&mut healer_activated);
        if event["kind"] == "residual" {
            in_residual = true;
        } else if event["kind"] == "activate"
            && let Some(args) = event["args"].as_array()
            && args.get(1).and_then(Value::as_str) == Some("ability: Healer")
        {
            healer_activated = true;
        } else if healer
            && event["kind"] == "curestatus"
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
        {
            out.push((who, ResidualKind::Healer));
        } else if event["kind"] == "activate"
            && let Some(args) = event["args"].as_array()
            && args.get(1).and_then(Value::as_str) == Some("ability: Shed Skin")
            && let Some(who) = args.first().and_then(who_from_text)
        {
            out.push((who, ResidualKind::ShedSkin));
        } else if event["kind"] == "item"
            && let Some(args) = event["args"].as_array()
            && args.get(2).and_then(Value::as_str) == Some("[from] ability: Harvest")
            && let Some(who) = args.first().and_then(who_from_text)
        {
            out.push((who, ResidualKind::Harvest));
        } else if in_residual
            && event["kind"] == "curestatus"
            && event["status"] == "frz"
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
        {
            out.push((who, ResidualKind::FreezeThaw));
        } else if in_residual
            && event["kind"] == "curestatus"
            && event["from"] == "[from] move: Uproar"
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
        {
            // Gen 3-4 (issues.md I63): woken by an uproar's turn-end step.
            out.push((who, ResidualKind::Uproar));
        } else if in_residual
            && event["kind"] == "status"
            && event["status"] == "slp"
            && let Ok(who) = serde_json::from_value::<Who>(event["pokemon"].clone())
        {
            out.push((who, ResidualKind::Yawn));
        } else if in_residual
            && event["kind"] == "start"
            && event["args"][1] == "confusion"
            && event["args"][2] == "[fatigue]"
            && let Some(who) = who_from_text(&event["args"][0])
        {
            // A lock (Thrash and its kind) that ended in the residual: its fatigue.
            out.push((who, ResidualKind::Rampage));
        }
    }
    out
}

/// Who fell in the residual, and at which handler (whose holder): the source of the last damage
/// the log shows it before its `faint` — the weather (Sandstorm, Hail, Dry Skin, Solar Power), its
/// own poison, burn, Leech Seed, binding, Nightmare, Curse, Salt Cure, Sticky Barb, Black Sludge,
/// a foreseen strike, a foe's Bad Dreams — or Perish Song's count. A fall the log does not tie to
/// a handler is left out (R-M9-51).
pub fn extract_residual_faints(events: &[Value]) -> Vec<(Who, ResidualKind, Who)> {
    let mut out = Vec::new();
    let mut in_residual = false;
    let mut last: Vec<(Who, ResidualKind, Who)> = Vec::new();
    let mut foreseen: Option<Who> = None;
    for event in events {
        let kind = event["kind"].as_str().unwrap_or("");
        if kind == "residual" {
            in_residual = true;
            continue;
        }
        if !in_residual {
            continue;
        }
        let pokemon = serde_json::from_value::<Who>(event["pokemon"].clone()).ok();
        let args = event["args"].as_array().cloned().unwrap_or_default();
        let arg = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or("");
        let mut mark = |who: Who, what: ResidualKind, holder: Who| {
            last.retain(|(w, _, _)| *w != who);
            last.push((who, what, holder));
        };
        match kind {
            "end" if arg(1).contains("Future Sight") || arg(1).contains("Doom Desire") => {
                foreseen = who_from_text(&args[0]);
            }
            "damage" => {
                let Some(who) = pokemon else { continue };
                let from = event["from"].as_str().unwrap_or("");
                let of = event["of"].as_str().unwrap_or("");
                let what = if from.is_empty() && foreseen.as_ref() == Some(&who) {
                    Some(ResidualKind::FutureAttack)
                } else if ["Sandstorm", "Hail", "Dry Skin", "Solar Power"]
                    .iter()
                    .any(|weather| from.contains(weather))
                {
                    Some(ResidualKind::Weather)
                } else if from == "[from] psn" {
                    Some(ResidualKind::Poison)
                } else if from == "[from] brn" {
                    Some(ResidualKind::Burn)
                } else if from.contains("Leech Seed") {
                    Some(ResidualKind::LeechSeed)
                } else if of == "[partiallytrapped]" {
                    Some(ResidualKind::PartiallyTrapped)
                } else if from.contains("Nightmare") {
                    Some(ResidualKind::Nightmare)
                } else if from.contains("Curse") {
                    Some(ResidualKind::Curse)
                } else if from.contains("Salt Cure") {
                    Some(ResidualKind::SaltCure)
                } else if from.contains("Sticky Barb") {
                    Some(ResidualKind::StickyBarb)
                } else if from.contains("Black Sludge") {
                    Some(ResidualKind::BlackSludge)
                } else if from.contains("Bad Dreams") {
                    // The handler is the foe's: its holder is the `[of]`.
                    if let Some(holder) =
                        who_from_text(&Value::from(of.strip_prefix("[of] ").unwrap_or(of)))
                    {
                        mark(who, ResidualKind::BadDreams, holder);
                    }
                    continue;
                } else {
                    None
                };
                match what {
                    Some(what) => mark(who.clone(), what, who),
                    None => last.retain(|(w, _, _)| *w != who),
                }
            }
            "start" if arg(1) == "perish0" => {
                if let Some(who) = who_from_text(&args[0]) {
                    mark(who.clone(), ResidualKind::PerishSong, who);
                }
            }
            "faint" => {
                if let Some(who) = pokemon
                    && let Some(fall) = last.iter().find(|(w, _, _)| *w == who)
                {
                    out.push(fall.clone());
                }
            }
            _ => {}
        }
    }
    out
}

/// A foreseen attack's strike as the log shows it (`-end|<target>|move: Future Sight`, DESIGN.md
/// §15.22), with what followed it for that target.
#[derive(Debug, Clone)]
pub(super) struct FutureStrikeOracle {
    pub(super) target: Who,
    pub(super) missed: bool,
    pub(super) immune: bool,
    pub(super) critical: bool,
    /// The target's Focus Band held it (`-activate|<target>|item: Focus Band`).
    pub(super) focus_band_held: bool,
    /// What a berry the strike's `Update` fed the target drew (Starf Berry's stat).
    pub(super) berry_draw: Option<BerryDraw>,
    /// The user's Toxic Chain statused the target the strike damaged (`-status|target|tox|[from]
    /// ability: Toxic Chain`, its `onSourceDamagingHit`; R-M11-33).
    pub(super) dealt_status: bool,
    /// The target's HP after the strike's damage; none when it took none (a miss, an immunity,
    /// a Substitute).
    pub(super) hp_after: Option<u16>,
}

/// The foreseen attacks this boundary's residual struck with, in log order.
pub(super) fn extract_future_strikes(events: &[Value]) -> Vec<FutureStrikeOracle> {
    let mut out: Vec<FutureStrikeOracle> = Vec::new();
    let mut open = false;
    let mut fed = false;
    let mut eaten: Option<(Who, String)> = None;
    for event in events {
        let kind = event["kind"].as_str().unwrap_or("");
        if kind == "end"
            && let Some(args) = event["args"].as_array()
            && matches!(
                args.get(1).and_then(Value::as_str),
                Some("move: Future Sight" | "move: Doom Desire")
            )
            && let Some(target) = args.first().and_then(who_from_text)
        {
            out.push(FutureStrikeOracle {
                target,
                missed: false,
                immune: false,
                critical: false,
                focus_band_held: false,
                berry_draw: None,
                dealt_status: false,
                hp_after: None,
            });
            open = true;
            fed = true;
            continue;
        }
        // The strike's `Update` (DESIGN.md R277): a berry it fed its target draws.
        let previous = eaten.take();
        eaten = berry_eaten(event);
        if fed
            && let Some(strike) = out.last_mut()
            && let Some((eater, Some(draw))) = BerryDraw::berry_boost(previous.as_ref(), event)
            && eater == strike.target
        {
            strike.berry_draw = Some(draw);
        }
        // The user's Toxic Chain on the target the strike damaged: after the damage line.
        if fed
            && kind == "status"
            && event["from"]
                .as_str()
                .is_some_and(|f| f.contains("ability: Toxic Chain"))
            && let Some(strike) = out.last_mut()
            && strike.hp_after.is_some()
            && serde_json::from_value::<Who>(event["pokemon"].clone())
                .ok()
                .is_some_and(|who| who.side == strike.target.side && who.name == strike.target.name)
        {
            strike.dealt_status = true;
        }
        if matches!(kind, "residual" | "turn" | "move") {
            fed = false;
        }
        if !open {
            continue;
        }
        let Some(strike) = out.last_mut() else {
            continue;
        };
        let about = |field: &str| {
            serde_json::from_value::<Who>(event[field].clone())
                .ok()
                .is_some_and(|who| who.side == strike.target.side && who.name == strike.target.name)
        };
        match kind {
            // `-miss|source|target`: the target in `detail`.
            "miss" => {
                strike.missed = true;
                open = false;
            }
            "immune" if about("pokemon") => {
                strike.immune = true;
                open = false;
            }
            "crit" if about("pokemon") => strike.critical = true,
            "activate"
                if event["args"].get(1).and_then(Value::as_str) == Some("item: Focus Band")
                    && event["args"]
                        .get(0)
                        .and_then(who_from_text)
                        .is_some_and(|who| who == strike.target) =>
            {
                strike.focus_band_held = true;
            }
            "damage" if about("pokemon") && event["from"].is_null() => {
                strike.hp_after = event["hp"].as_u64().map(|hp| hp as u16);
                open = false;
            }
            "residual" | "turn" => open = false,
            _ => {}
        }
    }
    out
}

/// Whether an action broke a Substitute: a Pokémon that had one before has none after.
pub(super) fn broke_a_doll(before: &BattleState, after: &BattleState) -> bool {
    before.all_pokemon().iter().any(|pokemon| {
        substitute_hp(before, pokemon.id()).is_some()
            && substitute_hp(after, pokemon.id()).is_none()
    })
}

/// Whether Gen 1's `lastDamage` is read from boundary `from` on: a Bide release (`-end|p|Bide`),
/// a Counter (R-M8-102), or a binding lock's next turn (`move|p|Wrap|…|[from] Wrap`), which
/// repeats the damage its first hit left there (R-M8-143).
pub(super) fn last_damage_read_later(frames: &[Option<&Value>], from: usize) -> bool {
    frames[from.min(frames.len())..]
        .iter()
        .flatten()
        .filter_map(|frame| frame["events"].as_array())
        .flatten()
        .any(|event| {
            (event["kind"] == "end"
                && matches!(
                    event["args"].get(1).and_then(Value::as_str),
                    Some("Bide" | "move: Bide")
                ))
                || (event["kind"] == "move" && event["move"] == "Counter")
                || (event["kind"] == "move"
                    && event["move"]
                        .as_str()
                        .is_some_and(|name| event["from"] == format!("[from] {name}").as_str()))
        })
}

/// Gen 1's open choice after a roll that broke a Substitute (`OpenChoices`, R-M8-102): the
/// other roll sets that reproduce the action as well as `chosen` — the same HP commits and
/// Substitutes, the plan used up — but leave another `lastDamage`, one per value.
pub(super) fn other_last_damages(
    compiled: &Compiled,
    before: &BattleSnapshot,
    trace_len: usize,
    tape: &[RandomOverride],
    chosen: &BattleEngine,
) -> Vec<Vec<RandomOverride>> {
    let damage = compiled.rules.damage();
    let dolls = |engine: &BattleEngine| {
        let state = engine.state();
        state
            .all_pokemon()
            .iter()
            .map(|pokemon| substitute_hp(state, pokemon.id()))
            .collect::<Vec<_>>()
    };
    let dealt = damages_in(chosen, trace_len);
    // The roll's own range: Psywave's damage is the roll (`psywave_range` by its user's level),
    // any other move's roll the damage formula's random factor (R-M9-48).
    let psywave_level = chosen.trace().events()[trace_len..]
        .iter()
        .find_map(|event| match event {
            TraceEvent::MoveUsed { actor, move_id, .. }
                if compiled.dex.move_by_id(*move_id).is_some_and(|m| {
                    m.fixed_damage == Some(pokemon_battle_engine::dex::FixedDamage::Psywave)
                }) =>
            {
                before.state.pokemon(*actor).map(|pokemon| pokemon.level)
            }
            _ => None,
        });
    let (min, max) = psywave_level.map_or((damage.random_min, damage.random_max), |level| {
        pokemon_battle_engine::engine::psywave_range(&compiled.rules, level)
    });
    let mut values = vec![chosen.state().last_damage];
    let mut found = Vec::new();
    for (at, decision) in tape.iter().enumerate() {
        if decision.kind != RandomRequestKind::DamageRoll {
            continue;
        }
        for roll in (min..=max).rev() {
            let mut other = tape.to_vec();
            other[at] = RandomOverride::range(RandomRequestKind::DamageRoll, roll);
            if decision.optional {
                other[at] = other[at].optional();
            }
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut candidate = restore_with_tape(compiled, before, other.clone());
                candidate.step().ok().map(|_| candidate)
            }));
            let Ok(Some(candidate)) = run else {
                continue;
            };
            let last_damage = candidate.state().last_damage;
            if damages_in(&candidate, trace_len) == dealt
                && dolls(&candidate) == dolls(chosen)
                && candidate.state().rng.semantic_tape_exhausted()
                && !values.contains(&last_damage)
            {
                values.push(last_damage);
                found.push(other);
            }
        }
    }
    found
}

/// Gen 4 Future Sight / Doom Desire (§15.22): the damage the strike dealt to the foreseen slot
/// (`target`'s side and slot), read from the boundary frames after `from` — its occupant's HP
/// before the strike (its last HP in that boundary's events, or the frame before's) less the HP
/// after — and whether it knocked the occupant out (any larger damage then fits). None when no
/// strike shows HP damage (a miss, an immunity, a battle over before it). A Substitute that took
/// the strike shows it by its HP; one another move struck earlier that turn leaves the strike's
/// share open, at most the turn's whole loss (`ForeseenDamage::AtMost`, R-M8-164).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ForeseenDamage {
    /// What the strike dealt, and whether it knocked out (or broke the doll).
    Dealt(u16, bool),
    /// A doll struck earlier in the strike's turn too: the strike took at most this much (or,
    /// when it broke the doll, anything).
    AtMost(u16, bool),
}

pub(super) fn future_damage_hint(
    frames: &[Option<&Value>],
    from: usize,
    target: &Who,
) -> Option<ForeseenDamage> {
    let same = |event: &Value, who: &Who| {
        serde_json::from_value::<Who>(event["pokemon"].clone())
            .ok()
            .is_some_and(|other| other.side == who.side && other.name == who.name)
    };
    for index in from + 1..frames.len() {
        let Some(frame) = frames[index] else {
            continue;
        };
        let events = frame["events"].as_array()?;
        for (at, event) in events.iter().enumerate() {
            let Some(args) = event["args"].as_array() else {
                continue;
            };
            if event["kind"] != "end"
                || !matches!(
                    args.get(1).and_then(Value::as_str),
                    Some("move: Future Sight" | "move: Doom Desire")
                )
            {
                continue;
            }
            let Some(struck) = args.first().and_then(who_from_text) else {
                continue;
            };
            // A target named without a place (a fainted foe, `p2: Slaking`): the slot the use
            // aimed at, the first (`foreseen_spot`, R-M8-89).
            if struck.side != target.side || struck.slot != target.slot.or(Some(0)) {
                continue;
            }
            // A strike a Substitute took (`-activate|p|Substitute|[damage]`, `-end|p|Substitute`
            // as it breaks): the doll's HP the frames show, before and at this boundary — a
            // broken doll took at least what it had (R-M8-101).
            let on_doll = events[at + 1..]
                .iter()
                .take_while(|later| later["kind"] != "residual" && later["kind"] != "damage")
                .find_map(|later| {
                    let args = later["args"].as_array()?;
                    let doll = args.first().and_then(who_from_text)? == struck
                        && args.get(1).and_then(Value::as_str) == Some("Substitute");
                    match later["kind"].as_str() {
                        Some("activate") if doll => Some(false),
                        Some("end") if doll => Some(true),
                        _ => None,
                    }
                });
            if let Some(broken) = on_doll {
                let doll_hp = |frame: &Value| {
                    frame_pokemon(frame, &struck)?["volatiles"]["substitute"].as_u64()
                };
                let previous = frames[..index].iter().rev().find_map(|frame| *frame)?;
                // A doll put up earlier in the strike's turn (`-start|p|Substitute`) is in no
                // earlier frame: it started with a quarter of the max HP (`floor(maxhp / 4)`,
                // DESIGN.md R-M9-28).
                let made_this_turn = || {
                    events[..at].iter().any(|earlier| {
                        earlier["kind"] == "start"
                            && earlier["args"].as_array().is_some_and(|args| {
                                args.first().and_then(who_from_text).as_ref() == Some(&struck)
                                    && args.get(1).and_then(Value::as_str) == Some("Substitute")
                            })
                    })
                };
                let had = match doll_hp(previous) {
                    Some(hp) => hp as u16,
                    None if made_this_turn() => {
                        (frame_pokemon(frame, &struck)?["max_hp"].as_u64()? / 4) as u16
                    }
                    None => return None,
                };
                // Another move struck the doll earlier that turn: the loss is both hits'.
                let struck_before = events[..at].iter().any(|earlier| {
                    earlier["kind"] == "activate"
                        && earlier["args"].as_array().is_some_and(|args| {
                            args.first().and_then(who_from_text).as_ref() == Some(&struck)
                                && args.get(1).and_then(Value::as_str) == Some("Substitute")
                        })
                });
                let lost = if broken {
                    had
                } else {
                    had.saturating_sub(doll_hp(frame)? as u16)
                };
                return Some(if struck_before {
                    ForeseenDamage::AtMost(lost, broken)
                } else {
                    ForeseenDamage::Dealt(lost, broken)
                });
            }
            let before = events[..at]
                .iter()
                .rev()
                .filter(|earlier| same(earlier, &struck))
                .find_map(|earlier| earlier["hp"].as_u64())
                .or_else(|| {
                    let previous = frames[..index].iter().rev().find_map(|frame| *frame)?;
                    frame_pokemon(previous, &struck)?["hp"].as_u64()
                })? as u16;
            let after = events[at + 1..]
                .iter()
                .take_while(|later| later["kind"] != "residual" && later["kind"] != "end")
                .find(|later| {
                    later["kind"] == "damage" && same(later, &struck) && later["from"].is_null()
                })?["hp"]
                .as_u64()? as u16;
            return Some(ForeseenDamage::Dealt(
                before.saturating_sub(after),
                after == 0,
            ));
        }
    }
    None
}

/// `args` entries are raw protocol fields such as `p2a: Mew`.
/// Neutralizing Gas clearing in this boundary (`-end|<holder>|ability: Neutralizing Gas`,
/// DESIGN.md R203): the Pokémon that draw a `SpeedTie` as the others' abilities start again
/// (`BattleEngine::neutralizing_gas_restart_ties`), each ranked by the order the restarted
/// abilities announce themselves after it (`-ability`, a weather's or terrain's `[of]`).
/// Optional: the step may not be the one the gas clears in.
pub(super) fn neutralizing_gas_tape(
    compiled: &Compiled,
    before: &BattleSnapshot,
    oracle_frame: &Value,
) -> Vec<RandomOverride> {
    let events = oracle_frame["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let Some(at) = events
        .iter()
        .position(|e| e["kind"] == "end" && e["args"][1] == "ability: Neutralizing Gas")
    else {
        return Vec::new();
    };
    let Some(holder) =
        who_from_text(&events[at]["args"][0]).and_then(|who| who_to_id(&before.state, &who))
    else {
        return Vec::new();
    };
    let drawing =
        restore_with_tape(compiled, before, Vec::new()).neutralizing_gas_restart_ties(holder);
    let mut shown: Vec<PokemonId> = Vec::new();
    for event in &events[at + 1..] {
        let args = event["args"].as_array();
        let who = match event["kind"].as_str() {
            Some("ability") => args.and_then(|args| args.first()).and_then(who_from_text),
            Some("weather" | "fieldstart") => args
                .into_iter()
                .flatten()
                .find_map(|arg| arg.as_str()?.strip_prefix("[of] ").map(str::to_owned))
                .and_then(|text| who_from_text(&Value::String(text))),
            _ => None,
        };
        if let Some(id) = who.and_then(|who| who_to_id(&before.state, &who))
            && !shown.contains(&id)
        {
            shown.push(id);
        }
    }
    let mut ranked = drawing.clone();
    ranked.sort_by_key(|id| shown.iter().position(|s| s == id).unwrap_or(usize::MAX));
    drawing
        .iter()
        .map(|id| {
            let rank = ranked.iter().position(|r| r == id).unwrap_or(0);
            RandomOverride::raw(RandomRequestKind::SpeedTie, (drawing.len() - rank) as u64)
                .optional()
        })
        .collect()
}

pub(super) fn who_from_text(value: &Value) -> Option<Who> {
    let text = value.as_str()?;
    let (position, name) = text.split_once(": ")?;
    let side = match position.chars().nth(1)? {
        '1' => 0,
        '2' => 1,
        _ => return None,
    };
    let slot = position.chars().nth(2).map(|c| usize::from(c as u8 - b'a'));
    Some(Who {
        side,
        slot,
        name: name.to_string(),
    })
}
