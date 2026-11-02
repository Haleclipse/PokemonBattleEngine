use std::sync::Arc;

use crate::dex::{MoveCategory, ResolvedDex};
use crate::effects::FractionalPriority;
use crate::ids::{PokemonId, PositionId, SideId};
use crate::rules::ResolvedRules;
use crate::state::{BattleOutcome, BattlePhase, BattleState, MechanicUsage};

use super::{
    ActionChoice, BattleError, BattleInitError, BattleSnapshot, BattleTrace, ChoiceError,
    EngineStep, ForcedSwitchChoice, PendingExecution, PriorityChance, QueuedAction, SideChoice,
    TraceEvent, validate_side_choice,
};

mod binding;
mod durations;
pub use durations::DurationHandler;
mod endless;
mod faint;
mod future_attack;
mod lifecycle;
mod mechanics;
mod pursuit;
mod queue;
mod replacements;
mod residual;
mod residual_effects;
mod slots;
mod switching;
mod turn;
mod view;

pub use view::{SeenPokemon, SideView};

/// One declared action before its priority chances are rolled.
struct PreparedAction {
    actor: PokemonId,
    choice: ActionChoice,
    action_priority: i16,
    move_priority: i8,
    prankster_boosted: bool,
    /// The fixed fractional priority (tenths) and the chances rolled after it, in order.
    fractional: i8,
    chances: Vec<PriorityChance>,
    /// The actor's Custap Berry goes off (`ItemBehavior::pinch_priority`): after the chances, a
    /// fractional priority still 0 or below becomes +0.1.
    custap: bool,
    speed: u32,
}

/// The end-of-turn speed ties a replay has to plan (`BattleEngine::pending_residual_ties`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResidualTies {
    /// Tied residual handlers (holder, kind, tie group), in the order their decisions are drawn.
    pub handlers: Vec<(PokemonId, crate::rules::ResidualKind, usize)>,
    /// Tied duration-only handlers (handler, tie group), drawn after `handlers`; a tie group may
    /// hold both kinds.
    pub durations: Vec<(DurationHandler, usize)>,
    /// Pokémon tied for the order the weather chips them (Pokémon, tie group), drawn after.
    pub weather: Vec<(PokemonId, usize)>,
}

/// The turn order some choices would produce (`BattleEngine::preview_action_order`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOrderPreview {
    /// Every priority chance the turn draws, in draw order, with the outcome it was given.
    pub chances: Vec<(PokemonId, PriorityChance, bool)>,
    /// The actions in queue order, grouped into brackets of equal keys (a bracket of more than
    /// one is settled by SpeedTie decisions).
    pub order: Vec<Vec<PokemonId>>,
    /// The actions that draw a SpeedTie decision, in draw order, each with its tie group.
    pub ties: Vec<(PokemonId, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementRequirement {
    pub side: SideId,
    /// Positions that must receive a replacement: fainted (empty) slots and slots whose occupant
    /// was ordered out mid-turn (U-turn / Volt Switch).
    pub positions: Vec<ReplacementSlot>,
    pub available_pokemon: Vec<PokemonId>,
    pub required_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplacementSlot {
    pub position: PositionId,
    /// `Some` when the slot is still occupied by a Pokémon ordered out; the replacement then
    /// runs the full switch lifecycle (cleanup, hazards, switch-in ability).
    pub outgoing: Option<PokemonId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnResult {
    pub phase: BattlePhase,
    pub outcome: Option<BattleOutcome>,
    pub replacements: [ReplacementRequirement; 2],
}

/// A battle (DESIGN.md §47): stepped with `step`, answered with the choices its requests ask
/// for. A clone is an independent battle from the same moment — the way a rollout forks one
/// without a snapshot (§47.5 item 8): the rules and the dex are shared, the state and the trace
/// copied.
#[derive(Clone)]
pub struct BattleEngine {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    state: BattleState,
    trace: BattleTrace,
    pending: PendingExecution,
    /// The speeds the end-of-turn residual orders by while it runs (Showdown's `residual` action
    /// starts with `updateSpeed`, and every sort in it reads `pokemon.speed`): taken before
    /// anything in it runs, empty outside it — the residual never stops at a request boundary
    /// (R-M9-46).
    residual_speeds: Vec<(PokemonId, u32)>,
}

impl BattleEngine {
    internal!(
        pub fn create_new(
            rules: Arc<ResolvedRules>,
            dex: Arc<ResolvedDex>,
            mut state: BattleState,
        ) -> Result<Self, BattleInitError> {
            // A battle starts in a coherent state (invariants), as `restore` does; what a format asks
            // of the teams before it (their size, the picks, Item Clause) is the pre-battle
            // session's, on the teams as registered (DESIGN.md §11.2, §45.5).
            validate_battle_invariants(&rules, &dex, &state)?;
            // Resolve the generation's behavior registry before the battle starts, so the first
            // lookup on a battle's path is a lookup, not a resolution (DESIGN.md §25.1).
            crate::effects::BehaviorRegistry::for_generation(rules.generation);

            // Mega Rayquaza Clause (`onBegin`: `canMegaEvo = null` for a Rayquaza, DESIGN.md §45.7).
            if rules.clauses.mega_rayquaza_clause {
                let ids: Vec<PokemonId> = state.pokemon_ids().collect();
                for id in ids {
                    if let Some(pokemon) = state.pokemon_mut(id)
                        && pokemon.species == Some(crate::ids::SpeciesId::RAYQUAZA)
                    {
                        pokemon.mega_evolves_into = None;
                    }
                }
            }
            for side in &mut state.sides {
                side.mechanics.mega = usage(rules.special.mega);
                // Ultra Burst is Mega Evolution's sibling (Q1): the same rules gate, apart counted.
                side.mechanics.ultra_burst = usage(rules.special.mega);
                side.mechanics.z_move = usage(rules.special.z_move);
                side.mechanics.dynamax = usage(rules.special.dynamax);
                side.mechanics.tera = usage(rules.special.tera);
            }
            // Showdown `activeTurns` counts at the start of every turn, the first included: a lead
            // is on its first turn at 1, a Pokémon switched in mid-turn at 0 until the next turn.
            let leads: Vec<PokemonId> = state
                .sides
                .iter()
                .flat_map(|side| side.active.iter().flatten().copied())
                .collect();
            // Showdown's `BattleStart` runs for **every** Pokémon of both teams before the first
            // switch-in: the Rusted Sword's and Shield's crowned formes are fixed here (DESIGN.md
            // §43.4, the `zacian` / `zamazenta` species conditions).
            let mut trace = BattleTrace::default();
            {
                // Nobody is on the field yet — Showdown runs `onBattleStart` before the leads'
                // `switchIn` — so the Gen 5+ bench rules apply while the forme is fixed (an
                // Intrepid Sword starts only as the holder enters).
                let benched: Vec<Vec<Option<PokemonId>>> =
                    [crate::ids::SideId::P1, crate::ids::SideId::P2]
                        .iter()
                        .map(|side| std::mem::take(&mut state.side_mut(*side).active))
                        .collect();
                let ids: Vec<PokemonId> = state.pokemon_ids().collect();
                let mut crowned = Vec::new();
                {
                    let mut ctx = crate::engine::BattleContext::new(&mut state, &rules, &mut trace)
                        .with_dex(&dex);
                    for id in ids {
                        let Some((item, forme)) =
                            crate::effects::items::battle_start_forme(&ctx, id)
                        else {
                            continue;
                        };
                        ctx.fix_own_forme(
                            id,
                            forme,
                            crate::engine::FormeChangeKind::Base,
                            crate::engine::EffectSource::Item { pokemon: id, item },
                        );
                        crowned.push((id, item));
                    }
                }
                for (id, item) in crowned {
                    if let Some((from, to)) = crate::effects::items::crowned_move_swap(item) {
                        let Some(move_def) = dex.move_by_id(to) else {
                            continue;
                        };
                        let max_pp = crate::dex::pp_with_ups(rules.generation, move_def);
                        crate::engine::BattleContext::new(&mut state, &rules, &mut trace)
                            .swap_move(id, from, to, max_pp);
                    }
                }
                for (side, slots) in [crate::ids::SideId::P1, crate::ids::SideId::P2]
                    .into_iter()
                    .zip(benched)
                {
                    state.side_mut(side).active = slots;
                }
            }
            // Showdown's start-of-battle `endTurn` also leaves the leads no longer newly switched.
            for lead in leads {
                if let Some(pokemon) = state.pokemon_mut(lead) {
                    pokemon.active_turns = 1;
                    pokemon.history.newly_switched = false;
                }
            }
            // Gen 1 keeps each stat's in-battle value on the Pokémon (`StatRules::modified_stats`).
            if rules.stats.modified_stats {
                let ids: Vec<PokemonId> = state.pokemon_ids().collect();
                for id in ids {
                    if let Some(pokemon) = state.pokemon_mut(id)
                        && pokemon.modified_stats.is_none()
                    {
                        pokemon.modified_stats = Some(*pokemon.battle_stats());
                        pokemon.apply_status_stat_cut();
                    }
                }
            }

            let mut engine = Self {
                rules,
                dex,
                state,
                trace,
                pending: PendingExecution::Idle,
                residual_speeds: Vec::new(),
            };
            engine.handed_back("create_new");
            Ok(engine)
        }
    );

    internal!(
        pub fn try_new(
            rules: Arc<ResolvedRules>,
            dex: Arc<ResolvedDex>,
            state: BattleState,
        ) -> Result<Self, BattleInitError> {
            Self::create_new(rules, dex, state)
        }
    );

    internal!(
        /// The battle back from `snapshot`, with an empty trace that keeps every event (DESIGN.md
        /// §47.0 item 6).
        pub fn restore(
            rules: Arc<ResolvedRules>,
            dex: Arc<ResolvedDex>,
            snapshot: BattleSnapshot,
        ) -> Result<Self, BattleInitError> {
            Self::restore_with_trace_mode(rules, dex, snapshot, super::TraceMode::Full)
        }
    );

    internal!(
        /// The battle back from `snapshot`, with an empty trace that keeps what `mode` says.
        pub fn restore_with_trace_mode(
            rules: Arc<ResolvedRules>,
            dex: Arc<ResolvedDex>,
            snapshot: BattleSnapshot,
            mode: super::TraceMode,
        ) -> Result<Self, BattleInitError> {
            // A snapshot is a moment of a battle already under way: its state need not be one the
            // format admits at the start (a Trick can leave a side two of an item under Item
            // Clause), only one a battle can be in (DESIGN.md §11.2).
            validate_snapshot_integrity(&rules, &dex, &snapshot)?;
            validate_battle_invariants(&rules, &dex, &snapshot.state)?;
            Ok(Self {
                rules,
                dex,
                state: snapshot.state,
                trace: BattleTrace::with_mode(mode),
                pending: snapshot.pending,
                residual_speeds: Vec::new(),
            })
        }
    );

    /// The battle back at `snapshot`, in place (DESIGN.md §47.5 item 8): a snapshot of a
    /// battle under the same rules and dex, checked as `restore` checks one, replaces the
    /// engine's state and what it was in the middle of; the trace starts empty, in the mode it
    /// had.
    pub fn rewind(&mut self, snapshot: &BattleSnapshot) -> Result<(), BattleInitError> {
        validate_snapshot_integrity(&self.rules, &self.dex, snapshot)?;
        validate_battle_invariants(&self.rules, &self.dex, &snapshot.state)?;
        self.state.clone_from(&snapshot.state);
        self.pending.clone_from(&snapshot.pending);
        self.trace = BattleTrace::with_mode(self.trace.mode());
        self.residual_speeds.clear();
        Ok(())
    }

    pub fn snapshot(&self) -> BattleSnapshot {
        // The state's knowledge copied once, with what the trace disclosed since control last
        // went back (none, at any moment a caller sees).
        let mut state = self.state.clone();
        self.learn_disclosed(&mut state.knowledge);
        BattleSnapshot {
            state,
            pending: self.pending.clone(),
            rules_fingerprint: self.rules.fingerprint.0.clone(),
            dex_fingerprint: self.dex.fingerprint().to_owned(),
        }
    }

    internal!(
        /// Convenience constructor for examples/tests. Production callers should prefer `try_new`.
        pub fn new(rules: Arc<ResolvedRules>, dex: Arc<ResolvedDex>, state: BattleState) -> Self {
            Self::create_new(rules, dex, state).expect("invalid initial battle state")
        }
    );

    internal!(
        pub fn rules(&self) -> &ResolvedRules {
            &self.rules
        }
    );

    internal!(
        pub fn dex(&self) -> &ResolvedDex {
            &self.dex
        }
    );

    /// The battle's dex, shared (`Names`).
    pub(crate) fn shared_dex(&self) -> Arc<ResolvedDex> {
        Arc::clone(&self.dex)
    }

    internal!(
        pub fn state(&self) -> &BattleState {
            &self.state
        }
    );

    internal!(
        pub fn trace(&self) -> &BattleTrace {
            &self.trace
        }
    );

    /// What the trace keeps (DESIGN.md §47.0 item 5): every event by default.
    pub fn trace_mode(&self) -> super::TraceMode {
        self.trace.mode()
    }

    /// What the trace keeps from now on. The battle runs the same whatever it keeps (the
    /// engine never reads its trace, R-M10-2).
    pub fn set_trace_mode(&mut self, mode: super::TraceMode) {
        self.trace.set_mode(mode);
    }

    internal!(
        /// The trace's events so far, leaving it empty: a host streaming the battle's log takes
        /// them after each step.
        pub fn take_trace_events(&mut self) -> Vec<TraceEvent> {
            self.trace.take_events()
        }
    );

    pub fn validate_choice(
        &self,
        expected_side: SideId,
        choice: &SideChoice,
    ) -> Result<(), ChoiceError> {
        validate_side_choice(&self.state, &self.rules, &self.dex, expected_side, choice)
    }

    /// What `side` can choose (DESIGN.md §47.0 items 10-11): at a turn's request, the options
    /// of each of its Pokémon on the field (`engine::choice_options`) — what `validate_choice`
    /// accepts, and why a move it refuses is refused; `None` while the battle asks for something
    /// else, or is over.
    pub fn choice_options(&self, side: SideId) -> Option<super::ChoiceOptions> {
        (self.state.phase == BattlePhase::TurnChoice)
            .then(|| super::choice_options(&self.state, &self.rules, &self.dex, side))
    }

    /// The generation gimmicks this Pokémon may ask for right now (DESIGN.md §44.9, the user's
    /// Q3 answer): a read-only view of what `validate_choice` would accept for it, so a client
    /// need not probe. Empty for a Pokémon that is not active.
    pub fn mechanic_options(&self, pokemon: PokemonId) -> super::MechanicOptions {
        if self
            .state
            .active_position(pokemon, &self.rules.topology)
            .is_none()
        {
            return super::MechanicOptions::default();
        }
        super::mechanic_options(&self.state, &self.rules, &self.dex, pokemon)
    }

    pub fn submit_turn_choices(&mut self, choices: [SideChoice; 2]) -> Result<(), BattleError> {
        let result = self.submit_turn_choices_inner(choices);
        self.handed_back("submit_turn_choices");
        result
    }

    pub fn step(&mut self) -> Result<EngineStep, BattleError> {
        let result = self.step_inner();
        if let Ok(step) = &result {
            self.handed_back(match step {
                EngineStep::Progressed { .. } => "step (progressed)",
                EngineStep::Awaiting(_) => "step (awaiting)",
                EngineStep::Finished(_) => "step (finished)",
            });
        }
        result
    }

    internal!(
        pub fn resolve_turn(
            &mut self,
            choices: [SideChoice; 2],
        ) -> Result<TurnResult, BattleError> {
            self.submit_turn_choices(choices)?;
            // Drive until the next input boundary (TurnChoices / ForcedSwitch) or the end of battle.
            while let EngineStep::Progressed { .. } = self.step()? {}
            Ok(self.turn_result())
        }
    );

    pub fn resolve_forced_switches(
        &mut self,
        choices: [ForcedSwitchChoice; 2],
    ) -> Result<TurnResult, BattleError> {
        let result = self.resolve_forced_switches_inner(choices);
        if result.is_ok() {
            self.handed_back("resolve_forced_switches");
        }
        result
    }
}

/// The fixed fractional priority of `actor`'s move action and the chances rolled after it
/// (DESIGN.md §12.4): Incense first (their generation's shift), then Stall (−0.1; in Gen 4 only
/// over a non-negative value), then the chances in handler order — Quick Draw for a damaging
/// move, Quick Claw (Gen 4+; Gen 2-3 roll once per turn instead, `action_speed`). Public: the
/// replay driver plans an instructed action's chances with it (R-M9-32).
pub fn fractional_priority(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    category: Option<MoveCategory>,
) -> (i8, Vec<PriorityChance>) {
    let Some(pokemon) = state.pokemon(actor) else {
        return (0, Vec::new());
    };
    let item = super::held_item(state, rules, actor).and_then(|(_, item)| item.fractional_priority);
    let ability = super::ability_handler(state, rules, pokemon, |b| b.fractional_priority);
    let mut fractional = 0;
    if item == Some(FractionalPriority::Delay) {
        fractional = rules.action_order.delay_item_tenths;
    }
    if ability == Some(FractionalPriority::Delay) && fractional >= 0 {
        fractional = -1;
    }
    let mut chances = Vec::new();
    if ability == Some(FractionalPriority::QuickDraw) && category != Some(MoveCategory::Status) {
        chances.push(PriorityChance::QuickDraw);
    }
    let delays_status =
        ability == Some(FractionalPriority::DelayStatus) && category == Some(MoveCategory::Status);
    if delays_status {
        fractional = -1;
    }
    if item == Some(FractionalPriority::QuickClaw) && !delays_status {
        chances.push(PriorityChance::QuickClaw);
    }
    (fractional, chances)
}

impl BattleEngine {
    fn ensure_phase(&self, expected: BattlePhase) -> Result<(), BattleError> {
        if self.state.phase != expected {
            return Err(BattleError::WrongPhase {
                expected,
                actual: self.state.phase,
            });
        }
        Ok(())
    }

    fn set_phase(&mut self, phase: BattlePhase) {
        if self.state.phase == phase {
            return;
        }
        let from = self.state.phase;
        self.state.phase = phase;
        self.trace
            .push(TraceEvent::PhaseChanged { from, to: phase });
    }

    fn turn_result(&self) -> TurnResult {
        TurnResult {
            phase: self.state.phase,
            outcome: self.state.outcome,
            replacements: self.replacement_requirements(),
        }
    }
}

/// What holds at every moment a battle can be observed — after `create_new` and after every
/// call that advances it, so of every snapshot a battle takes (DESIGN.md §11.2): the state's
/// structure (slots, ids, teams, sides), the types a battle can make, and what the moment's
/// phase fixes (`phase_invariants`).
fn validate_battle_invariants(
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    state: &BattleState,
) -> Result<(), BattleInitError> {
    if rules.generation != dex.generation {
        return Err(BattleInitError::DexGenerationMismatch {
            rules: rules.generation,
            dex: dex.generation,
        });
    }

    let expected_slots = rules.topology.active_slots_per_side() as usize;
    for side in [SideId::P1, SideId::P2] {
        let side_state = state.side(side);
        if side_state.active.len() != expected_slots {
            return Err(BattleInitError::ActiveSlotCountMismatch {
                side,
                expected: expected_slots,
                actual: side_state.active.len(),
            });
        }
    }

    let mut seen_ids = Vec::new();
    for pokemon in state.all_pokemon() {
        // `???` is no generation's type to choose, but a battle makes it (Gen 4 Roost on a pure
        // Flying type, Burn Up), so a restored state may hold it.
        for pokemon_type in crate::engine::types_of(state, rules, pokemon).iter() {
            if pokemon_type != crate::dex::PokemonType::Typeless
                && !rules.type_chart.is_available(pokemon_type)
            {
                return Err(BattleInitError::UnavailablePokemonType {
                    pokemon: pokemon.id(),
                    pokemon_type,
                });
            }
        }
    }

    for pokemon in state.all_pokemon() {
        if seen_ids.contains(&pokemon.id()) {
            return Err(BattleInitError::DuplicatePokemonId(pokemon.id()));
        }
        seen_ids.push(pokemon.id());
    }

    let phase = phase_invariants(state.phase);
    for side in [SideId::P1, SideId::P2] {
        let side_state = state.side(side);
        let mut seen_team = Vec::new();
        for pokemon_id in &side_state.team {
            if seen_team.contains(pokemon_id) {
                return Err(BattleInitError::DuplicateTeamPokemon {
                    side,
                    pokemon: *pokemon_id,
                });
            }
            seen_team.push(*pokemon_id);

            let pokemon =
                state
                    .pokemon(*pokemon_id)
                    .ok_or(BattleInitError::UnknownTeamPokemon {
                        side,
                        pokemon: *pokemon_id,
                    })?;
            if pokemon.side != side {
                return Err(BattleInitError::PokemonSideMismatch {
                    pokemon: *pokemon_id,
                    expected: side,
                    actual: pokemon.side,
                });
            }
        }

        let mut seen_active = Vec::new();
        for pokemon_id in side_state.active.iter().flatten() {
            if seen_active.contains(pokemon_id) {
                return Err(BattleInitError::DuplicateActivePokemon {
                    side,
                    pokemon: *pokemon_id,
                });
            }
            seen_active.push(*pokemon_id);

            if !side_state.team.contains(pokemon_id) {
                return Err(BattleInitError::ActivePokemonNotOnTeam {
                    side,
                    pokemon: *pokemon_id,
                });
            }
            if phase.faints_resolved
                && state
                    .pokemon(*pokemon_id)
                    .is_some_and(|pokemon| pokemon.is_fainted())
            {
                return Err(BattleInitError::ActivePokemonFainted(*pokemon_id));
            }
        }
    }

    for pokemon in state.all_pokemon() {
        let assigned = [SideId::P1, SideId::P2]
            .into_iter()
            .any(|side| state.side(side).team.contains(&pokemon.id()));
        if !assigned {
            return Err(BattleInitError::UnassignedPokemon(pokemon.id()));
        }
    }

    Ok(())
}

/// What a phase fixes of the state at an observable moment (DESIGN.md §11.2).
struct PhaseInvariants {
    /// Every faint is resolved: a fainted Pokémon has left its slot (`resolve_faints` runs
    /// before an action, a turn's end or a replacement hands control back).
    faints_resolved: bool,
}

fn phase_invariants(phase: BattlePhase) -> PhaseInvariants {
    match phase {
        // The moments control comes back at: a turn's choices, a replacement or a revive asked
        // for, a step between two actions, the battle over. Each comes after the faints.
        BattlePhase::TurnChoice
        | BattlePhase::ForcedSwitch
        | BattlePhase::Revive
        | BattlePhase::TurnResolution
        | BattlePhase::Finished => PhaseInvariants {
            faints_resolved: true,
        },
        // Never observable (`phase_admits`): nothing is asserted of it.
        BattlePhase::EndTurn => PhaseInvariants {
            faints_resolved: false,
        },
    }
}

/// Whether a battle hands control back in `phase` with `pending` execution — the pairs a
/// snapshot can hold (DESIGN.md §11.2).
fn phase_admits(phase: BattlePhase, pending: &PendingExecution) -> bool {
    matches!(
        (phase, pending),
        // A turn's choices, and the battle over.
        (BattlePhase::TurnChoice | BattlePhase::Finished, PendingExecution::Idle)
            // A step between two actions of the turn.
            | (BattlePhase::TurnResolution, PendingExecution::Turn { .. })
            // A replacement: after a turn, mid-turn (U-turn, Gen 1-3's after each move) or
            // before the first turn (a lead's Eject Pack).
            | (
                BattlePhase::ForcedSwitch,
                PendingExecution::Idle | PendingExecution::Turn { .. } | PendingExecution::Start
            )
            // Revival Blessing's choice, mid-turn.
            | (BattlePhase::Revive, PendingExecution::Turn { .. })
    )
}

/// A snapshot's own coherence (`restore`): taken under these rules and this dex, at a moment
/// a battle hands control back (`phase_admits`).
fn validate_snapshot_integrity(
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    snapshot: &BattleSnapshot,
) -> Result<(), BattleInitError> {
    if snapshot.rules_fingerprint.as_str() != rules.fingerprint.0.as_str() {
        return Err(BattleInitError::SnapshotRulesMismatch);
    }
    if snapshot.dex_fingerprint != dex.fingerprint() {
        return Err(BattleInitError::SnapshotDexMismatch);
    }
    if !phase_admits(snapshot.state.phase, &snapshot.pending) {
        return Err(BattleInitError::SnapshotPhaseMismatch {
            phase: snapshot.state.phase,
        });
    }
    Ok(())
}

impl BattleEngine {
    /// Control goes back to the caller after `after`: what its events disclosed joins the state's
    /// knowledge (DESIGN.md §47.0 item 12), and debug builds check the moment is one a snapshot
    /// may be taken at.
    pub(super) fn handed_back(&mut self, after: &str) {
        for disclosure in self.trace.drain_disclosed() {
            self.state
                .knowledge
                .learn(disclosure, &self.rules.conditions.volatile_visibility);
        }
        self.debug_assert_observable(after);
    }

    internal!(
        /// What the team preview showed before the battle (DESIGN.md §47.0 item 14), for the
        /// views: an open team sheet's item, ability and moves known of the Pokémon brought from
        /// the start. Call once, before `run_lead_switch_ins`.
        pub fn show_team_preview(&mut self, preview: crate::state::TeamPreview) {
            self.state.knowledge.show_team_preview(preview);
        }
    );

    /// The state's knowledge with what the trace disclosed since control last went back (none,
    /// at any moment a caller sees): what the views and a snapshot read.
    pub(crate) fn knowledge(&self) -> std::borrow::Cow<'_, crate::state::Knowledge> {
        if self.trace.disclosed().is_empty() {
            return std::borrow::Cow::Borrowed(&self.state.knowledge);
        }
        let mut knowledge = self.state.knowledge.clone();
        self.learn_disclosed(&mut knowledge);
        std::borrow::Cow::Owned(knowledge)
    }

    /// `knowledge` (a copy of the state's) learns what the trace disclosed since control last
    /// went back.
    fn learn_disclosed(&self, knowledge: &mut crate::state::Knowledge) {
        for disclosure in self.trace.disclosed() {
            knowledge.learn(*disclosure, &self.rules.conditions.volatile_visibility);
        }
    }

    /// Whether `pokemon` has been on the field (the state's knowledge and what the trace
    /// disclosed since control last went back).
    pub(crate) fn has_appeared(&self, pokemon: PokemonId) -> bool {
        self.state.knowledge.has_appeared(pokemon)
            || self.trace.disclosed().iter().any(|disclosure| {
                matches!(
                    disclosure,
                    crate::state::Disclosure::Appeared { seen_as, .. } if *seen_as == pokemon
                )
            })
    }

    /// Every moment a battle hands control back is one a snapshot may be taken at, and so one
    /// `restore` must accept: debug builds check it after every call that advances the battle
    /// (DESIGN.md §11.2).
    pub(super) fn debug_assert_observable(&self, after: &str) {
        #[cfg(debug_assertions)]
        {
            let admitted = phase_admits(self.state.phase, &self.pending);
            let invariants = validate_battle_invariants(&self.rules, &self.dex, &self.state);
            assert!(
                admitted && invariants.is_ok(),
                "an observable moment a snapshot of which restore refuses (after {after}): phase \
                 {:?}, pending {:?}, invariants {invariants:?}",
                self.state.phase,
                self.pending,
            );
        }
        #[cfg(not(debug_assertions))]
        let _ = after;
    }
}

fn usage(enabled: bool) -> MechanicUsage {
    if enabled {
        MechanicUsage::Available
    } else {
        MechanicUsage::Unavailable
    }
}

/// The move a walk of the queue finds `action` using (Showdown `action.move`, `QueuedMove::
/// walked_move`): the one the turn's start saw — a lock's, Struggle, the slot's — none for a Max
/// Move, a Z-Move or a recharge. An action Instruct injected, which the turn's start did not see,
/// uses its slot's move.
fn walked_move(state: &BattleState, action: &QueuedAction) -> Option<crate::ids::MoveId> {
    let ActionChoice::Move { slot, .. } = action.choice else {
        return None;
    };
    if action.instructed {
        return state
            .pokemon(action.actor)
            .and_then(|pokemon| pokemon.move_slot(slot))
            .map(|slot| slot.move_id);
    }
    state
        .queued_moves
        .iter()
        .find(|queued| queued.actor == action.actor)
        .and_then(|queued| queued.walked_move())
}

/// Whether the queue's next action is a move (no switch pending: a switch always goes first).
fn next_is_move(actions: &[QueuedAction]) -> bool {
    actions
        .iter()
        .all(|action| matches!(action.choice, ActionChoice::Move { .. }))
}

/// Whether the action at `index` shares its ordering key with another one.
fn tied_at(keys: &[(i16, i8, i8, u32)], index: usize) -> bool {
    keys.iter()
        .enumerate()
        .any(|(other, key)| other != index && *key == keys[index])
}

/// A prepared action's fractional priority once its chances are settled: each in handler
/// order, rolled by `roll` where it applies (Quick Claw only while nothing raised the value);
/// one that goes off puts the action first in its bracket (+0.1).
fn settle_fractional(action: &PreparedAction, mut roll: impl FnMut(PriorityChance) -> bool) -> i8 {
    let mut fractional = action.fractional;
    for &chance in &action.chances {
        let applies = chance != PriorityChance::QuickClaw || fractional <= 0;
        if applies && roll(chance) {
            fractional = 1;
        }
    }
    fractional
}

/// The announcement of a priority chance that went off (`-activate`).
fn activation(actor: PokemonId, chance: PriorityChance) -> TraceEvent {
    match chance {
        PriorityChance::QuickDraw => TraceEvent::AbilityActivated {
            source: actor,
            ability: crate::ids::AbilityId::QUICK_DRAW,
        },
        PriorityChance::QuickClaw | PriorityChance::QuickClawTurn => TraceEvent::ItemActivated {
            holder: actor,
            item: crate::ids::ItemId::QUICK_CLAW,
        },
    }
}

/// A move choice without its gimmick (`ActionStep::of` split it off into its own queue entry).
fn without_mechanic(choice: ActionChoice) -> ActionChoice {
    match choice {
        ActionChoice::Move { slot, target, .. } => ActionChoice::Move {
            slot,
            target,
            mechanic: None,
        },
        other => other,
    }
}
