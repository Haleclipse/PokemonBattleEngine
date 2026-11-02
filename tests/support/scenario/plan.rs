//! The replay report, and the random plan's building blocks: staged tapes, a drawn target, a
//! hit's accuracy draws, a frame's members.

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

// ---------------------------------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ReplayReport {
    pub frames_compared: usize,
    pub actions_replayed: usize,
    pub divergences: Vec<String>,
    /// Rust frames at every compared boundary, in order (for the failure bundle).
    pub rust_frames: Vec<Value>,
    /// The engine's full trace after replay, one event per line.
    pub trace: Vec<String>,
    /// Rules / dex fingerprints the replay ran against.
    pub provenance: Value,
    /// The Gen 1 open choices this replay met and the ones it was told to take (R-M8-102).
    pub open_choices: OpenChoices,
}

/// Choices the log leaves open until later. Gen 1 (R-M8-102): a roll that broke a Substitute
/// sets `lastDamage` to the damage uncapped (Showdown's gen1 Substitute), which no frame shows
/// until a Bide release or a Counter reads it. Gen 4 (R-M8-164): the roll a Future Sight stored
/// its damage with, when another move struck the doll the strike lands on. Each such action is
/// a choice between the roll sets that leave another outcome; `replay_against_oracle` runs a
/// diverging replay again with them taken.
#[derive(Debug, Default, Clone)]
pub struct OpenChoices {
    /// The alternative to take at an open choice, by its place among the choices met.
    pub(super) pins: BTreeMap<usize, usize>,
    /// How many alternatives each choice met so far had, in order.
    pub(super) met: Vec<usize>,
}

pub(super) fn restore_with_tape(
    compiled: &Compiled,
    snapshot: &BattleSnapshot,
    tape: Vec<RandomOverride>,
) -> BattleEngine {
    let mut snapshot = snapshot.clone();
    snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
    // `SNAPSHOT_JSON=1`: every snapshot the replay restores goes through its JSON first, and
    // must read back equal and write the same bytes (DESIGN.md §48.2, R-M11-25).
    if std::env::var_os("SNAPSHOT_JSON").is_some() {
        let json = snapshot.to_json();
        let back = BattleSnapshot::from_json(&json).expect("a snapshot's own JSON reads back");
        assert!(
            back == snapshot,
            "a snapshot read back from its JSON differs"
        );
        assert_eq!(
            back.to_json(),
            json,
            "a snapshot read back writes other bytes"
        );
        snapshot = back;
    }
    BattleEngine::restore(compiled.rules.clone(), compiled.dex.clone(), snapshot).expect("restore")
}

/// Which of `candidates` a random target draw picked, from the target the `move` line names:
/// that one — unless the Lightning Rod / Storm Drain holder named there drew the move to itself
/// (`redirected_by`), and then the draw was another candidate, the first (any other is drawn
/// to the holder alike, and the holder's ability is announced either way, R-M11-7). `None`: the
/// named target is no candidate.
pub(super) fn drawn_target(
    state: &BattleState,
    candidates: &[PokemonId],
    named: Option<&Who>,
    redirected_by: Option<&Who>,
) -> Option<usize> {
    let named = named.and_then(|who| who_to_id(state, who))?;
    if redirected_by.and_then(|who| who_to_id(state, who)) == Some(named) {
        return candidates.iter().position(|candidate| *candidate != named);
    }
    candidates.iter().position(|candidate| *candidate == named)
}

pub(super) fn who_to_id(state: &BattleState, who: &Who) -> Option<PokemonId> {
    let side = if who.side == 0 {
        SideId::P1
    } else {
        SideId::P2
    };
    state.side(side).active.get(who.slot?).copied().flatten()
}

/// Where the Pokémon `who` names in `state` (the state an action began in) stands in a later
/// boundary `frame`: its party index there (the frame's `party` is in team order), if it is
/// still on the field. Its slot may hold another Pokémon by then — an Ally Switch later in the
/// turn swaps the two (R-M8-175), a switch brings a replacement in.
pub(super) fn frame_member(state: &BattleState, frame: &Value, who: &Who) -> Option<usize> {
    let id = who_to_id(state, who)?;
    let side = if who.side == 0 {
        SideId::P1
    } else {
        SideId::P2
    };
    let member = state
        .side(side)
        .team
        .iter()
        .position(|member| *member == id)?;
    frame["sides"][who.side]["active"]
        .as_array()?
        .iter()
        .any(|index| index.as_u64() == Some(member as u64))
        .then_some(member)
}

/// Whether two names are the same Pokémon: by side and name, whatever slot each was named in
/// (it may have moved within the turn, or come back into another slot after a switch).
pub(super) fn same_pokemon(a: &Who, b: &Who) -> bool {
    a.side == b.side && a.name == b.name
}

/// The party index of the Pokémon in `who`'s slot in `frame` (the boundary `who` was read at).
pub(super) fn frame_member_at_slot(frame: &Value, who: &Who) -> Option<usize> {
    frame["sides"][who.side]["active"][who.slot?]
        .as_u64()
        .map(|index| index as usize)
}

/// The semantic decisions one action will ask for, in engine order (BeforeMove first, then the
/// move), with the damage roll left open (`None`) to be searched.
/// What Showdown reported for one Pokémon target of an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetFate {
    Hit,
    Miss,
    /// Not affected at all: no accuracy check is made (type immunity precedes it).
    Immune,
    /// In the air / underground and out of the move's reach: missed before any check.
    Dodged,
}

/// A target of a spread move that sent it back (Magic Coat, Magic Bounce): the reflection is
/// `called[called]` of the action, and strikes `fates[back]`, each checked by the reflector with
/// its own move (R-M8-176).
#[derive(Debug, Clone)]
pub(super) struct Reflection {
    pub(super) reflector: PokemonId,
    pub(super) called: usize,
    pub(super) back: std::ops::Range<usize>,
}

/// Which rolled duration a hint is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DurationKind {
    Confusion,
    Sleep,
    Bind,
    /// Taunt / Encore / Disable, rolled before Gen 5.
    Timed,
    /// Gen 1's binding lock, on its user (`partialtrappinglock`).
    BindingLock,
    /// Bide on its user: Gen 2's duration, Gen 1's time.
    Bide,
    /// Thrash and its kind on their user: the lock's length (Gen 1 uses, Gen 3-4 turns, Gen 5+
    /// hidden length).
    Rampage,
    /// Uproar on its user: the turns Gen 3-4 draw.
    Uproar,
}

/// The stages of a damaging move's decisions: for a spread move each stage covers every
/// target before the next begins (Showdown `spreadMoveHit`); otherwise one target's decisions
/// all come before the next target's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    Gate,
    SubstituteDamage,
    Damage,
    /// The damage landing on the target's body (a spread move lands every target's after all
    /// of them are computed): Focus Band.
    Land,
    Effect,
    Reaction,
}

pub(super) struct StagedTape {
    pub(super) spread: bool,
    /// Dragon Darts split between two targets: every target's hit steps come first, then each
    /// target's darts in turn.
    pub(super) gates_first: bool,
    pub(super) stages: [Vec<Option<RandomOverride>>; 6],
    pub(super) tape: Vec<Option<RandomOverride>>,
    pub(super) held: Vec<Option<RandomOverride>>,
}

impl StagedTape {
    pub(super) fn new(spread: bool) -> Self {
        Self {
            spread,
            gates_first: false,
            stages: Default::default(),
            tape: Vec::new(),
            held: Vec::new(),
        }
    }

    fn slot(&mut self, stage: Stage) -> &mut Vec<Option<RandomOverride>> {
        &mut self.stages[stage as usize]
    }

    pub(super) fn push(&mut self, stage: Stage, decision: RandomOverride) {
        self.slot(stage).push(Some(decision));
    }

    /// The damage roll, left open for the search.
    pub(super) fn push_roll(&mut self, stage: Stage) {
        self.slot(stage).push(None);
    }

    /// One hit (or one target) done: outside a spread move its decisions go out as they came.
    pub(super) fn end_hit(&mut self) {
        if self.spread {
            return;
        }
        if self.gates_first {
            let (gate, rest) = self.stages.split_at_mut(1);
            self.tape.append(&mut gate[0]);
            for stage in rest {
                self.held.append(stage);
            }
            return;
        }
        for stage in &mut self.stages {
            self.tape.append(stage);
        }
    }

    pub(super) fn finish(mut self) -> Vec<Option<RandomOverride>> {
        for stage in &mut self.stages {
            self.tape.append(stage);
        }
        self.tape.append(&mut self.held);
        self.tape
    }
}

/// Whether the move's own callbacks roll the stall counter (Showdown `runEvent('StallMove')`)
/// when its user has one: the stalling moves (`stallingMove`) whose `onPrepareHit` is the roll
/// (Protect and its variants, Endure) and, where the generation has it, Wide Guard / Quick
/// Guard's `onTry` (`ConditionRules::guards_roll_stall`). Mat Block is a stalling move without a
/// roll.
/// The engine's accuracy chance as the hit gate computes it, with the move under way
/// (`BattleState::active_move`): a move that ignores abilities (Mold Breaker's, Moongeist Beam)
/// leaves the target's breakable evasion abilities (Tangled Feet, Sand Veil …) out. The driver's
/// state is the one before the action, with no move under way.
/// Whether `actor`'s move draws an accuracy decision against `target` on `state` (no target:
/// the move's own accuracy decides).
pub(super) fn accuracy_draws(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    target: Option<PokemonId>,
    move_def: &ResolvedMove,
) -> bool {
    match target {
        None => {
            !pokemon_battle_engine::engine::sure_hit(state, rules, actor, move_def)
                && move_def.accuracy.is_some_and(|accuracy| accuracy < 100)
        }
        Some(id) => accuracy_in_move(state, rules, actor, id, move_def) < 10_000,
    }
}

pub(super) fn accuracy_in_move(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
) -> u32 {
    pokemon_battle_engine::engine::accuracy_basis_points(
        &move_under_way(state, rules, actor, move_def),
        rules,
        actor,
        target,
        move_def,
    )
}

/// `state` as `actor`'s move sees it: the move under way (`active_move`), so an ability the
/// move ignores (Mold Breaker, Teravolt …) answers nothing — accuracy, a critical hit, a
/// secondary's roll (Shield Dust) are asked on it (DESIGN.md R251); and the user's Protean /
/// Libero has given it the move's type (`onPrepareHit`, before the hit steps), so what the move
/// then draws (Conversion's candidates) is planned on the types it leaves (DESIGN.md R263).
pub(super) fn move_under_way(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    move_def: &ResolvedMove,
) -> BattleState {
    let mut state = state.clone();
    state.active_move = Some(ActiveMove {
        user: actor,
        move_id: move_def.id,
        category: move_def.category,
        ignores_ability: move_def.ignores_ability,
        stellar_boosted: false,
        z_powered: false,
        base_move: None,
        prankster_boosted: false,
    });
    if let Some(type_) =
        pokemon_battle_engine::engine::protean_type(&state, rules, actor, move_def.move_type)
        && let Some(user) = state.pokemon_mut(actor)
    {
        user.set_types(pokemon_battle_engine::state::PokemonTypes::mono(type_));
    }
    // Explosion and its kin (`selfdestruct: 'always'`, Gen 2-9): the user is down before the hit
    // steps, and its abilities with it — an Unaware user no longer hides its target's evasion
    // from the accuracy check (R-M8-146).
    if move_def.self_destruct == Some(pokemon_battle_engine::dex::SelfDestruct::Always)
        && rules.conditions().self_faint.timing
            != pokemon_battle_engine::rules::SelfDestructTiming::AfterMove
        && let Some(user) = state.pokemon_mut(actor)
    {
        user.hp = 0;
    }
    state
}

pub(super) fn rolls_stall(rules: &ResolvedRules, move_def: &ResolvedMove) -> bool {
    move_def.flags.stalling
        && pokemon_battle_engine::effects::moves::custom(move_def.id).is_some_and(|primary| {
            primary.source_callbacks.contains(&"onPrepareHit")
                || (rules.conditions().guards_roll_stall
                    && matches!(move_def.id, MoveId::WIDE_GUARD | MoveId::QUICK_GUARD))
        })
}

/// Whether the caller a called move's `[from]` names draws the move's target
/// (`engine::called_move_aims_at_random`): an ability's call is a reflection (Magic Bounce),
/// aimed back at the user; a move's, by the move (R-M8-144).
pub(super) fn called_at_random(
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    from: Option<&str>,
) -> bool {
    let Some(from) = from.and_then(|from| from.strip_prefix("[from] ")) else {
        return true;
    };
    if from.starts_with("ability:") {
        return false;
    }
    dex.move_by_name(from.trim_start_matches("move: "))
        .is_none_or(|caller| {
            pokemon_battle_engine::engine::called_move_aims_at_random(rules, caller.id)
        })
}

/// The priority chances an instructed use rolls as Instruct builds it (`resolveAction`'s
/// `FractionalPriority`, DESIGN.md R-M9-32): the ones `engine::fractional_priority` gives its
/// user's move, Quick Claw only while nothing went off before it; each went off when the log
/// announced it between the `-singleturn` and the move.
pub(super) fn instructed_chance_tape(
    compiled: &Compiled,
    state: &BattleState,
    instructed: &ActionOracle,
) -> Vec<Option<RandomOverride>> {
    let Some(user) = who_to_id(state, &instructed.user) else {
        return Vec::new();
    };
    let category = instructed
        .move_name
        .as_deref()
        .and_then(|name| compiled.dex.move_by_name(name))
        .map(|def| def.category);
    let (mut fractional, chances) =
        pokemon_battle_engine::engine::fractional_priority(state, &compiled.rules, user, category);
    let mut rolls = Vec::new();
    for chance in chances {
        if chance == PriorityChance::QuickClaw && fractional > 0 {
            continue;
        }
        let rolled = instructed.instructed_chances.contains(&chance);
        rolls.push(Some(RandomOverride::bool(
            RandomRequestKind::PriorityChance,
            rolled,
        )));
        if rolled {
            fractional = 1;
        }
    }
    rolls
}

/// The priority chance an `-activate` announces (`item: Quick Claw`, `ability: Quick Draw`).
pub(super) fn announced_chance(what: &str) -> Option<PriorityChance> {
    match what {
        "item: Quick Claw" => Some(PriorityChance::QuickClaw),
        "ability: Quick Draw" => Some(PriorityChance::QuickDraw),
        _ => None,
    }
}
