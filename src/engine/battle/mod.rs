use std::sync::Arc;

use crate::dex::{MoveCategory, ResolvedDex};
use crate::effects::{FractionalPriority, PriorityBoost};
use crate::ids::{PokemonId, PositionId, SideId};
use crate::rules::ResolvedRules;
use crate::state::{
    BattleOutcome, BattlePhase, BattleState, FieldConditionKind, MechanicUsage, RandomRequestKind,
    SelectedMove, VolatileConditionKind,
};

use super::stats::action_speed;
use super::{
    ActionChoice, ActionOrder, ActionQueue, ActionStep, BattleError, BattleInitError,
    BattleRequest, BattleSnapshot, BattleTrace, ChoiceError, EffectSource, EngineStep,
    ForcedSwitchChoice, MechanicChoice, MovePipeline, PendingExecution, PendingFaints,
    PriorityChance, QueuedAction, SideChoice, SlotChoice, TraceEvent, validate_side_choice,
};

mod binding;
mod durations;
pub use durations::DurationHandler;
mod endless;
mod faint;
mod future_attack;
mod lifecycle;
mod pursuit;
mod residual;
mod residual_effects;
mod slots;
mod switching;
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

    fn submit_turn_choices_inner(&mut self, choices: [SideChoice; 2]) -> Result<(), BattleError> {
        self.ensure_phase(BattlePhase::TurnChoice)?;
        self.validate_choice(SideId::P1, &choices[0])?;
        self.validate_choice(SideId::P2, &choices[1])?;

        self.trace.push(TraceEvent::TurnStarted {
            turn: self.state.turn,
        });
        self.set_phase(BattlePhase::TurnResolution);
        self.state.turn_actions_done = false;
        let queue = self.build_action_queue(&choices)?;
        self.record_selected_moves(&choices);
        let mut actions = queue.into_actions();
        // Gen 1 (`ActionOrderRules::recharge_acts_with_last_selected`): a recharging Pokémon
        // acts with its side's last selected move, which the recharge turn left as it was; so
        // does a frozen or sleeping one (`sleeping_frozen_act_with_last_selected`: its menu is
        // `Fight` alone, `commitChoices` sets `choice.moveid = lastSelectedMove`) — the move a
        // Pokémon thawed by a Fire move uses later in the turn (R-M8-145).
        let order = self.rules.action_order;
        self.state.fight_actors.clear();
        if order.recharge_acts_with_last_selected || order.sleeping_frozen_act_with_last_selected {
            for action in &mut actions {
                let fight = order.sleeping_frozen_act_with_last_selected
                    && self.state.pokemon(action.actor).is_some_and(|pokemon| {
                        pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
                            || pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                    });
                if let ActionChoice::Move { slot, .. } = &mut action.choice
                    && (fight
                        || (order.recharge_acts_with_last_selected
                            && super::choice::recharging(&self.state, action.actor)))
                    && let Some(pokemon) = self.state.pokemon(action.actor)
                    && let Some(last) = crate::ids::MoveSlot::new(
                        self.state.side(pokemon.side).last_selected_move_slot as u8,
                    )
                {
                    *slot = last;
                    if fight {
                        self.state.fight_actors.push(action.actor);
                    }
                }
            }
        }
        // Showdown `beforeTurnMove` (`priorityChargeCallback`): a chosen Chilly Reception puts
        // its announcement on its user as the turn begins, in action order.
        self.state.pending_movers = actions
            .iter()
            .filter(|action| matches!(action.choice, ActionChoice::Move { .. }))
            .map(|action| action.actor)
            .collect();
        self.state.struggling = self
            .state
            .pending_movers
            .iter()
            .copied()
            .filter(|actor| {
                super::choice::must_struggle(&self.state, &self.rules, &self.dex, *actor)
            })
            .collect();
        self.state.cannot_move = self.bound_movers();
        self.state.curse_glitched.clear();
        // The actors whose Dynamax is a step of this turn (`ActionStep::Dynamax`: their move
        // action carries no mechanic any more).
        let dynamaxing: Vec<PokemonId> = actions
            .iter()
            .filter(|action| action.step == ActionStep::Dynamax)
            .map(|action| action.actor)
            .collect();
        // What each move action names, for Sucker Punch / Upper Hand (`queue.willMove`): the
        // chosen move (Struggle when nothing was left), its category, its ordered priority.
        self.state.queued_moves = actions
            .iter()
            // The move's own step: a Mega Evolution's, a Dynamax's or a Terastallization's step
            // is no move (`queue.willMove` finds the `move` action, R-M9-32).
            .filter(|action| action.step == ActionStep::Act)
            .filter_map(|action| {
                let ActionChoice::Move { slot, mechanic, .. } = action.choice else {
                    return None;
                };
                // A locked Pokémon's action is its lock: Me First copies a rampage's next turn
                // (R-M8-66).
                let move_id = if self.state.struggling.contains(&action.actor) {
                    crate::ids::MoveId::STRUGGLE
                } else {
                    self.action_move(action.actor, slot)?
                };
                let category = self.dex.move_by_id(move_id)?.category;
                Some(crate::state::QueuedMove {
                    actor: action.actor,
                    move_id,
                    category,
                    priority: action.order.move_priority,
                    z_powered: mechanic == Some(MechanicChoice::ZMove),
                    max_move: mechanic == Some(MechanicChoice::Dynamax)
                        || dynamaxing.contains(&action.actor)
                        || self.state.pokemon(action.actor).is_some_and(|pokemon| {
                            pokemon.volatiles.contains(VolatileConditionKind::Dynamax)
                        }),
                    recharge: self.state.pokemon(action.actor).is_some_and(|pokemon| {
                        pokemon
                            .volatiles
                            .contains(VolatileConditionKind::MustRecharge)
                    }),
                })
            })
            .collect();
        // Showdown `beforeTurnMove` / `priorityChargeMove` (Chilly Reception, Focus Punch, Beak
        // Blast, Shell Trap, Counter, Mirror Coat): the move a Pokémon will use puts its volatile
        // on it as the turn begins, in action order (a Pokémon left to Struggle announces
        // nothing, nor does a recharging one: its action is the `recharge` pseudo-move,
        // R-M8-81). A Max Move or a Z-Move announces nothing either (`!action.maxMove &&
        // !action.zmove`, R-M9-30).
        let announced: Vec<(PokemonId, VolatileConditionKind)> = self
            .state
            .queued_moves
            .iter()
            .filter(|queued| queued.walked_move().is_some())
            .filter_map(|queued| {
                let kind = self
                    .dex
                    .move_by_id(queued.move_id)?
                    .behavior
                    .turn_start_volatile?;
                Some((queued.actor, kind))
            })
            .collect();
        for (actor, kind) in announced {
            if self
                .state
                .pokemon_mut(actor)
                .is_some_and(|pokemon| pokemon.volatiles.set_turn_start(kind))
            {
                self.trace.push(TraceEvent::VolatileStarted {
                    target: actor,
                    kind,
                });
            }
        }
        // Rage as the turn starts (§15.19): Gen 2-3 a raging Pokémon that did not choose Rage
        // stops raging; Gen 4 it loses what the cartridge's bug wipes instead.
        self.rage_turn_start();
        // Pursuit's `beforeTurnCallback` (§15.11).
        self.mark_pursuit_targets(&actions);
        self.pending = PendingExecution::Turn {
            actions,
            strategy: self.rules.action_order.strategy,
            residual: true,
            resort_ties: false,
        };
        Ok(())
    }

    /// Gen 2-4 on the cartridges (pokecrystal `ParsePlayerAction`, pokeemerald
    /// `TryClearRageStatuses`, pokeplatinum `PRE_MOVE_ACTION_STATE_CHECK_RAGE_FLAG`; §15.19):
    /// every raging Pokémon whose action this turn is not Rage — a switch, Struggle, any other
    /// move — stops raging (`RageEnd::TurnStart`) or loses the volatiles Gen 4's bug wipes
    /// (`RageEnd::Gen4`).
    fn rage_turn_start(&mut self) {
        let end = self.rules.conditions.rage.end;
        if end == crate::rules::RageEnd::BeforeMove {
            return;
        }
        let raging: Vec<PokemonId> =
            [SideId::P1, SideId::P2]
                .into_iter()
                .flat_map(|side| self.state.side(side).active.iter().flatten().copied())
                .filter(|id| {
                    self.state
                        .pokemon(*id)
                        .is_some_and(|p| !p.is_fainted() && p.volatiles.rage().is_some())
                })
                .filter(|id| {
                    !self.state.queued_moves.iter().any(|queued| {
                        queued.actor == *id && queued.move_id == crate::ids::MoveId::RAGE
                    })
                })
                .collect();
        let mut ctx =
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        for pokemon in raging {
            if end == crate::rules::RageEnd::TurnStart {
                ctx.end_rage(pokemon);
            } else {
                ctx.rage_wipe(pokemon);
            }
        }
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

    fn step_inner(&mut self) -> Result<EngineStep, BattleError> {
        if let Some(outcome) = self.state.outcome {
            return Ok(EngineStep::Finished(outcome));
        }

        match self.state.phase {
            BattlePhase::TurnChoice => {
                return Ok(EngineStep::Awaiting(BattleRequest::TurnChoices {
                    sides: [SideId::P1, SideId::P2],
                }));
            }
            BattlePhase::ForcedSwitch => {
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: self.replacement_requirements(),
                }));
            }
            BattlePhase::Revive => {
                if let Some(request) = self.revive_request() {
                    return Ok(EngineStep::Awaiting(request));
                }
                self.set_phase(BattlePhase::TurnResolution);
            }
            _ => {}
        }

        // A U-turn user, Eject Button holder … marked by Pursuit or Dynamaxed: its
        // `BeforeSwitchOut` before the replacement request (Showdown `runAction`), then the
        // faints (§15.11, R-M9-21).
        if self.state.phase == BattlePhase::TurnResolution
            && let Some(outgoing) = self.before_switch_out_due()
        {
            self.state.skip_before_switch_out.push(outgoing);
            self.before_switch_out(outgoing)?;
            self.state.clear_active_move();
            self.resolve_faints();
            if let Some(outcome) = self.state.outcome {
                self.pending = PendingExecution::Idle;
                return Ok(EngineStep::Finished(outcome));
            }
            if self.mid_turn_replacement_needed() {
                self.set_phase(BattlePhase::ForcedSwitch);
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: self.replacement_requirements(),
                }));
            }
            return Ok(EngineStep::Progressed {
                phase: self.state.phase,
            });
        }

        let action = self.take_next_pending_action();
        if let Some(action) = action {
            self.execute_queued_action(action)?;
            self.process_drag_queue();
            // Showdown `runAction`: `clearActiveMove` after the drags, before the faints.
            self.state.clear_active_move();
            self.resolve_faints();
            if let Some(outcome) = self.state.outcome {
                self.pending = PendingExecution::Idle;
                return Ok(EngineStep::Finished(outcome));
            }
            // Gen 1-3: Showdown's `checkFainted` runs after every action followed by a move or
            // the residual (a switch still queued goes first): a Pokémon knocked out by the
            // action loses its status to the faint whether or not a replacement exists, and is
            // replaced right away.
            if self.rules.faint.replace_after_each_move && !self.next_pending_is_switch() {
                self.check_fainted();
            }
            if self.rules.faint.replace_after_each_move
                && self.fainted_slot_to_fill()
                && !self.next_pending_is_switch()
            {
                self.feed_status_berries_before_request();
                self.set_phase(BattlePhase::ForcedSwitch);
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: self.replacement_requirements(),
                }));
            }
            // The action's `Update` (Showdown `runAction`): Gen 5+ before any replacement
            // request; Gen 1-4 only when none is asked for — a Gen 1-3 replacement requested
            // above returned already, a mid-turn one below returns before it.
            if self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }
            // Mid-turn replacement (U-turn / Volt Switch; Gen 4 Healing Wish's slot): the turn is
            // suspended with the remaining actions still pending; `resolve_forced_switches`
            // resumes it. A leaving Pokémon a pursuer marked, or a Dynamaxed one, meets its
            // `BeforeSwitchOut` first, at the next step (§15.11, R-M9-21).
            if self.before_switch_out_due().is_some() {
                return Ok(EngineStep::Progressed {
                    phase: self.state.phase,
                });
            }
            // Asked before a Revival Blessing owed at the same time: Showdown asks both in one
            // request and runs the switch first (`instaswitch` before `revivalblessing`), then
            // the revive (`resolve_forced_switches` resumes the turn on it, R-M8-171).
            if self.mid_turn_replacement_needed() {
                self.feed_status_berries_before_request();
                self.set_phase(BattlePhase::ForcedSwitch);
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: self.replacement_requirements(),
                }));
            }
            // Revival Blessing: the side names the fainted Pokémon that comes back, the turn
            // suspended until `resolve_revive`.
            if let Some(request) = self.revive_request() {
                self.set_phase(BattlePhase::Revive);
                return Ok(EngineStep::Awaiting(request));
            }
            if !self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }
            return Ok(EngineStep::Progressed {
                phase: self.state.phase,
            });
        }

        if self.state.phase == BattlePhase::TurnResolution {
            if !matches!(
                self.pending,
                PendingExecution::Turn {
                    residual: false,
                    ..
                }
            ) {
                self.run_end_turn_residual();
                // The residual's drags (a Red Card a future attack's hit set off) — Showdown's
                // `runAction` drags in after the residual action as after any other, before the
                // faints (R-M8-141).
                self.process_drag_queue();
            }
            self.resolve_faints();
            // Revival Blessing's partner back into its slot (queued behind the residual).
            self.return_revived();
            self.pending = PendingExecution::Idle;

            // A battle decided in the residual ends without its `Update`.
            if let Some(outcome) = self.state.outcome {
                return Ok(EngineStep::Finished(outcome));
            }
            if self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }
            self.check_fainted();

            let requirements = self.replacement_requirements();
            if requirements
                .iter()
                .any(|requirement| requirement.required_count > 0)
            {
                self.feed_status_berries_before_request();
                self.set_phase(BattlePhase::ForcedSwitch);
                return Ok(EngineStep::Awaiting(BattleRequest::ForcedSwitch {
                    replacements: requirements,
                }));
            }
            // No replacement asked for: the residual's `Update` (before Gen 5 it comes only now).
            if !self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }

            self.next_turn();
            // A rule at the turn's end may have ended the battle (DESIGN.md §45.7).
            if let Some(outcome) = self.state.outcome {
                return Ok(EngineStep::Finished(outcome));
            }
            self.set_phase(BattlePhase::TurnChoice);
            return Ok(EngineStep::Awaiting(BattleRequest::TurnChoices {
                sides: [SideId::P1, SideId::P2],
            }));
        }

        Ok(EngineStep::Progressed {
            phase: self.state.phase,
        })
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

    fn execute_queued_action(&mut self, action: QueuedAction) -> Result<(), BattleError> {
        // A move cancelled while queued (Showdown `queue.cancelMove`: Gravity grounding a
        // flyer, `BattleContext::cancel_move`) is no longer pending: it does not happen. Its
        // gimmick's step — queued separately — still runs (Showdown cancels the move action
        // alone).
        // An injected action (Instruct) is the target's extra use, not the turn's own: it
        // neither checks nor consumes the turn's pending action (DESIGN.md §42.9).
        if action.step == ActionStep::Act && !action.instructed {
            let cancelled = matches!(action.choice, ActionChoice::Move { .. })
                && !self.state.pending_movers.contains(&action.actor);
            self.state
                .pending_movers
                .retain(|actor| *actor != action.actor);
            if cancelled {
                return Ok(());
            }
        }
        let actor_can_act = self
            .state
            .pokemon(action.actor)
            .is_some_and(|pokemon| !pokemon.is_fainted())
            && self
                .state
                .active_position(action.actor, &self.rules.topology)
                .is_some();
        if !actor_can_act {
            return Ok(());
        }

        // A gimmick's own step (DESIGN.md §44.2): it happens before every move of the turn,
        // whatever the two Pokémon's speeds, and the move action that follows carries no
        // mechanic.
        if action.step != ActionStep::Act {
            if let ActionChoice::Move {
                mechanic: Some(mechanic),
                ..
            } = action.choice
            {
                self.apply_mechanic(action.actor, mechanic)?;
            }
            return Ok(());
        }
        let _ = &action.step;
        match action.choice {
            ActionChoice::Move {
                slot,
                target,
                mechanic,
            } => {
                // Gen 1 `cannotmove` (§30.9): bound as the choices were made, the Pokémon's
                // action is lost — only its after-action residual runs (`abortMove`).
                if let Some(index) = self
                    .state
                    .cannot_move
                    .iter()
                    .position(|pokemon| *pokemon == action.actor)
                {
                    self.state.cannot_move.remove(index);
                    self.run_after_own_action(action.actor, true, false);
                    return Ok(());
                }
                // Gen 4 Custap Berry (`onCustap`): the holder eats it as its move begins, if it
                // still holds it.
                if let Some(index) = self
                    .state
                    .custap_ready
                    .iter()
                    .position(|pokemon| *pokemon == action.actor)
                {
                    self.state.custap_ready.remove(index);
                    if super::held_item(&self.state, &self.rules, action.actor)
                        .is_some_and(|(_, behavior)| behavior.pinch_priority)
                    {
                        let mut ctx = crate::engine::BattleContext::new(
                            &mut self.state,
                            &self.rules,
                            &mut self.trace,
                        );
                        if let Some(item) = ctx.eat_item(action.actor) {
                            ctx.item_activated(action.actor, item);
                        }
                    }
                }
                // A queue built without the split (a hand-built queue, the preview) still
                // carries its mechanic on the move action; a Z-Move was spent above, as it
                // actually ran.
                if let Some(mechanic) = mechanic
                    && mechanic != MechanicChoice::ZMove
                {
                    self.apply_mechanic(action.actor, mechanic)?;
                }
                // The Curse glitch (`BattleContext::curse_glitch`): the action aims at the ally's
                // slot, the first of the side.
                let target = match self
                    .state
                    .curse_glitched
                    .iter()
                    .position(|pokemon| *pokemon == action.actor)
                {
                    Some(index) => {
                        self.state.curse_glitched.remove(index);
                        self.state
                            .pokemon(action.actor)
                            .and_then(|pokemon| self.rules.topology.position(pokemon.side, 0))
                            .map(crate::rules::DeclaredTarget)
                            .or(target)
                    }
                    None => target,
                };
                // A Z-Move: the slot's move is what the crystal converts, and the Z power is
                // spent as the move is used — once BeforeMove and the PP let it through (Showdown
                // `runMove`, DESIGN.md §44.4, R-M9-29; `MovePipeline::spend_z_power`).
                let z_move = match mechanic {
                    Some(MechanicChoice::ZMove) => {
                        let base = self
                            .state
                            .pokemon(action.actor)
                            .and_then(|pokemon| pokemon.move_slot(slot))
                            .and_then(|slot_state| {
                                self.dex.move_by_id(slot_state.move_id).cloned()
                            });
                        base.and_then(|base| {
                            crate::engine::z_move_execution(
                                &self.state,
                                &self.rules,
                                &self.dex,
                                action.actor,
                                &base,
                            )
                        })
                    }
                    _ => None,
                };
                // A Dynamaxed Pokémon's move runs as its Max Move (Showdown `chooseMove` +
                // `getActiveMaxMove`, DESIGN.md §44.5): the type as it will be used, the base
                // move's category, priority and power.
                let max_move = {
                    let base = self
                        .state
                        .pokemon(action.actor)
                        .and_then(|pokemon| pokemon.move_slot(slot))
                        .and_then(|slot_state| self.dex.move_by_id(slot_state.move_id));
                    base.and_then(|base| {
                        let modified = crate::engine::modified_move_cow(
                            &self.state,
                            &self.rules,
                            action.actor,
                            base,
                            false,
                        )
                        .0;
                        crate::engine::max_move_execution(
                            &self.state,
                            &self.dex,
                            action.actor,
                            &modified,
                        )
                    })
                };
                let mut pipeline =
                    MovePipeline::new(&mut self.state, &self.rules, &self.dex, &mut self.trace)
                        .with_action_priority(
                            action.order.move_priority,
                            action.order.prankster_boosted,
                        )
                        .with_prioritized_by(action.prioritized_by)
                        .with_original_target(action.original_target);
                if let Some(z) = z_move {
                    pipeline = pipeline.with_z_move(z);
                } else if let Some(max) = max_move {
                    pipeline = pipeline.with_max_move(max);
                }
                let execution = pipeline.execute(action.actor, slot, target)?;
                // What the action's runs asked of the queue (DESIGN.md §48.3).
                let crate::engine::move_pipeline::ActionEffects {
                    promote_pledge,
                    prioritize_move,
                    prioritize_actors: armed,
                } = pipeline.take_effects();
                // Gen 4 (Showdown restores `activeMove = oldActiveMove` at the end of the
                // outermost `runMove`): the battle's last move — what Copycat reads — is the
                // outermost of a chain, not the move it called (DESIGN.md §42.2).
                let outermost = pipeline.outermost();
                drop(pipeline);
                // A Pledge that waited for its ally's: that action goes to the front with this
                // move as its `sourceEffect` (DESIGN.md §42.9).
                if let Some(ally_move) = promote_pledge {
                    self.prioritize_ally_move(execution.move_id, execution.actor, ally_move);
                }
                if let Some(move_id) = prioritize_move {
                    self.prioritize_same_move(move_id);
                }
                if self.rules.conditions.calls.copycat_reads_outermost
                    && let Some(outermost) = outermost
                    && let Some(active) = &mut self.state.active_move
                {
                    active.move_id = outermost;
                }
                for actor in armed {
                    self.prioritize_actor(actor);
                }
                for request in std::mem::take(&mut self.state.queue_requests) {
                    match request {
                        crate::state::QueueRequest::Prioritize(actor) => {
                            self.prioritize_actor(actor)
                        }
                        crate::state::QueueRequest::Quash(actor) => self.quash_actor(actor),
                        crate::state::QueueRequest::Instruct {
                            actor,
                            slot,
                            target_position,
                            ..
                        } => self.instruct_action(actor, slot, target_position),
                    }
                }
                // Gen 2: a Roar's drag is over by then (`SwitchingRules::drag_within_the_move`).
                if self.rules.conditions.switching.drag_within_the_move {
                    self.process_drag_queue();
                }
                // Gen 2: the user's own after-action residual (`onAfterMoveSelf`), skipped
                // after a self-switching move.
                let used = self.dex.move_by_id(execution.move_id);
                let self_switch = used.is_some_and(|m| m.self_switch.is_some());
                let aimed_at_self =
                    used.is_some_and(|m| m.target == crate::rules::TargetPattern::SelfOnly);
                if execution.stopped || !self_switch {
                    self.run_after_own_action(action.actor, execution.stopped, aimed_at_self);
                }
            }
            ActionChoice::Switch { pokemon } => self.execute_switch(action.actor, pokemon)?,
            ActionChoice::Pass => {}
        }
        Ok(())
    }

    /// Showdown `checkFainted`, run once the turn's queue is empty and the battle goes on: a
    /// fainted Pokémon's status gives way to the faint (`fnt`). A battle decided mid-turn never
    /// reaches it, so the last Pokémon to faint keeps its status in the final state, as in
    /// Showdown.
    /// Showdown `endTurn`'s Dynamax sweep (DESIGN.md §44.5): every Pokémon whose counter reached
    /// 3 ends its Dynamax, fastest first (its own `speedSort`).
    fn expire_dynamax(&mut self) {
        let mut ending: Vec<(PokemonId, u32)> = self
            .active_pokemon()
            .into_iter()
            .filter(|pokemon| {
                self.state
                    .pokemon(*pokemon)
                    .is_some_and(|pokemon| pokemon.volatiles.dynamax_turns() == Some(3))
            })
            .map(|pokemon| {
                (
                    pokemon,
                    crate::engine::stats::action_speed(&self.state, &self.rules, pokemon, None),
                )
            })
            .collect();
        ending.sort_by_key(|(_, speed)| std::cmp::Reverse(*speed));
        for (pokemon, _) in ending {
            self.end_dynamax(pokemon);
        }
    }

    fn check_fainted(&mut self) {
        let fainted: Vec<(PokemonId, crate::state::NonVolatileStatus)> = self
            .state
            .all_pokemon()
            .iter()
            .filter(|p| p.is_fainted())
            .filter_map(|p| p.status.map(|status| (p.id(), status.kind)))
            .collect();
        for (pokemon, status) in fainted {
            if let Some(state) = self.state.pokemon_mut(pokemon) {
                state.status = None;
            }
            self.trace.push(TraceEvent::StatusCured {
                target: pokemon,
                status,
                reason: super::CureReason::Fainted,
            });
        }
    }

    /// The turn boundary (Showdown `endTurn`): every Pokémon on the field starts the coming
    /// turn one turn older (`activeTurns++`) and its event history ages (DESIGN.md §22.8);
    /// each side's faints of the turn become last turn's; the turn's last successful move is
    /// forgotten. Then the rules that end a battle at a turn's end (`end_turn_rules`).
    fn next_turn(&mut self) {
        // Showdown `endTurn`'s Dynamax sweep: once the turn is over — after the residual's
        // `Update` and after the replacements a faint asked for (a Pokémon Dynamaxed on its
        // third turn is still so while they come in).
        self.expire_dynamax();
        self.clean_up_binding();
        let active = self.active_pokemon();
        for &pokemon_id in &active {
            if let Some(pokemon) = self.state.pokemon_mut(pokemon_id) {
                pokemon.active_turns = pokemon.active_turns.saturating_add(1);
                pokemon.history.next_turn(&active);
            }
        }
        for side in [SideId::P1, SideId::P2] {
            let side = self.state.side_mut(side);
            side.fainted_last_turn = std::mem::take(&mut side.fainted_this_turn);
        }
        self.state.last_successful_move = None;
        self.state.skip_before_switch_out.clear();
        self.state.turn = self.state.turn.saturating_add(1);
        // Showdown `endTurn` → `DisableMove`: `choicelock.onDisableMove` drops a lock onto a move
        // the holder does not know (a lock onto Struggle), or held by a Pokémon that no longer
        // holds a Choice item (`getItem().isChoice`, set aside or not: Knock Off, Thief). Gen 4
        // drops it while the item is set aside as well (issues.md I55).
        let set_aside_ends_it = self.rules.conditions.items.choice_lock_ends_while_set_aside;
        for pokemon_id in self.active_pokemon() {
            let stale = self.state.pokemon(pokemon_id).is_some_and(|pokemon| {
                pokemon
                    .volatiles
                    .choice_locked_move()
                    .is_some_and(|locked| {
                        pokemon.move_slot_of(locked).is_none()
                            || !crate::engine::holds_choice_item(&self.rules, pokemon)
                            || (set_aside_ends_it
                                && crate::engine::held_item_of(&self.state, &self.rules, pokemon)
                                    .is_none())
                    })
            });
            if stale
                && let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
                && pokemon.volatiles.remove(VolatileConditionKind::ChoiceLock)
            {
                self.trace.push(TraceEvent::VolatileEnded {
                    target: pokemon_id,
                    kind: VolatileConditionKind::ChoiceLock,
                    seen: true,
                });
            }
        }
        self.end_turn_rules();
    }

    /// Round's `onTry` (Showdown `queue.prioritizeAction`): the next queued action using
    /// `move_id`, either side's, goes to the front — ahead of everything still queued, the
    /// Gen 8+ re-sort included (its order becomes 3 in Showdown) — and remembers what moved it.
    /// Nothing reads the queue between the `onTry` and the end of the move, so the move reports
    /// it and the queue changes afterwards.
    fn prioritize_same_move(&mut self, move_id: crate::ids::MoveId) {
        let state = &self.state;
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        // The move the action uses as the turn began (`action.move`): passed over when it is a
        // Max Move or a Z-Move (`action.maxMove || action.zmove`: a Dynamaxed ally's Round is Max
        // Strike, DESIGN.md R-M9-27) or a recharge, the slot's Round a recharging Pokémon names.
        let uses = |action: &QueuedAction| {
            action.step == ActionStep::Act
                && matches!(action.choice, ActionChoice::Move { .. })
                && walked_move(state, action) == Some(move_id)
        };
        let Some(index) = actions.iter().position(uses) else {
            return;
        };
        let mut action = actions.remove(index);
        action.order.action_priority = self.rules.action_order.prioritized_action_priority;
        action.prioritized_by = Some(move_id);
        actions.insert(0, action);
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: super::QueueReorderReason::Prioritized,
        });
    }

    /// Whether the queue is re-sorted before the next action (Gen 8+, a move comes next).
    fn resorts_before_next(&self) -> bool {
        matches!(
            &self.pending,
            PendingExecution::Turn {
                actions,
                strategy: crate::rules::ActionOrderStrategy::RecalculateAfterAction,
                ..
            } if !actions.is_empty() && next_is_move(actions)
        )
    }

    /// Instruct's injected action (DESIGN.md §42.9): the target uses the move in `slot` (its
    /// last move) at once — a fresh action at the front of the queue, its own action for the
    /// turn untouched (`queue.resolveAction` + `prioritizeAction`). Its order is the one the move
    /// would have had (its priority and the actor's speed), as Showdown's `resolveAction` builds
    /// it.
    fn instruct_action(
        &mut self,
        actor: PokemonId,
        slot: usize,
        target_position: Option<crate::ids::PositionId>,
    ) {
        let Some(slot) = crate::ids::MoveSlot::new(slot as u8) else {
            return;
        };
        if self.state.pokemon(actor).is_none()
            || self
                .state
                .active_position(actor, &self.rules.topology)
                .is_none()
        {
            return;
        }
        if !matches!(self.pending, PendingExecution::Turn { .. }) {
            return;
        }
        // Showdown builds the action as any (`resolveAction`): `ModifyPriority` (a Prankster
        // boost stays with the move), and `FractionalPriority`'s chances roll and announce
        // themselves (Quick Draw, Quick Claw; a Custap Berry at a pinch is eaten) — though
        // `prioritizeAction` puts it first whatever they give (R-M9-32).
        let choice = SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot,
                target: target_position.map(crate::rules::DeclaredTarget),
                mechanic: None,
            },
        };
        let Ok(prepared) = self.prepare_action(&choice, None, false) else {
            return;
        };
        let fractional = self.roll_fractional(&prepared);
        let order = ActionOrder {
            action_priority: self.rules.action_order.prioritized_action_priority,
            ..self.settled_order(&prepared, fractional)
        };
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        actions.insert(
            0,
            QueuedAction {
                actor,
                choice: ActionChoice::Move {
                    slot,
                    target: target_position.map(crate::rules::DeclaredTarget),
                    mechanic: None,
                },
                step: crate::engine::ActionStep::Act,
                order,
                prioritized_by: None,
                instructed: true,
                original_target: target_position.and_then(|position| {
                    self.state
                        .pokemon_at_position(position, &self.rules.topology)
                }),
            },
        );
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: super::QueueReorderReason::Prioritized,
        });
    }

    /// A Pledge's promotion (DESIGN.md §42.9): the first queued action of an *ally* using
    /// `move_id` goes to the front, with `by` (the waiting move) as its `sourceEffect`.
    fn prioritize_ally_move(
        &mut self,
        by_move: crate::ids::MoveId,
        by: PokemonId,
        move_id: crate::ids::MoveId,
    ) {
        let side = self.state.pokemon(by).map(|pokemon| pokemon.side);
        let state = &self.state;
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        let Some(index) = actions.iter().position(|action| {
            side == state.pokemon(action.actor).map(|pokemon| pokemon.side)
                && action.step == ActionStep::Act
                && matches!(action.choice, ActionChoice::Move { .. })
                && walked_move(state, action) == Some(move_id)
        }) else {
            return;
        };
        let mut action = actions.remove(index);
        action.order.action_priority = self.rules.action_order.prioritized_action_priority;
        action.prioritized_by = Some(by_move);
        actions.insert(0, action);
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: super::QueueReorderReason::Prioritized,
        });
    }

    /// Showdown `queue.prioritizeAction` for Shell Trap: `actor`'s pending action goes to the
    /// front of the queue.
    fn prioritize_actor(&mut self, actor: PokemonId) {
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        let Some(index) = actions.iter().position(|action| action.actor == actor) else {
            return;
        };
        let mut action = actions.remove(index);
        action.order.action_priority = self.rules.action_order.prioritized_action_priority;
        actions.insert(0, action);
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: super::QueueReorderReason::Prioritized,
        });
    }

    /// Quash's request (DESIGN.md §42.4): `actor`'s pending action goes last — to the end of
    /// the queue (Gen 5-7), or into the bracket after every move the next re-sort orders (Gen
    /// 8+).
    fn quash_actor(&mut self, actor: PokemonId) {
        let quash = self.rules.action_order.quash;
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        let Some(index) = actions.iter().position(|action| action.actor == actor) else {
            return;
        };
        match quash {
            crate::rules::QuashOrder::End => {
                let action = actions.remove(index);
                actions.push(action);
            }
            crate::rules::QuashOrder::LastBracket(priority) => {
                actions[index].order.action_priority = priority;
            }
        }
        self.trace.push(TraceEvent::ActionQueueReordered {
            reason: super::QueueReorderReason::Quashed,
        });
    }

    /// Whether the next queued action of the turn is a switch.
    fn next_pending_is_switch(&self) -> bool {
        matches!(
            &self.pending,
            PendingExecution::Turn { actions, .. }
                if actions
                    .first()
                    .is_some_and(|action| matches!(action.choice, ActionChoice::Switch { .. }))
        )
    }

    fn take_next_pending_action(&mut self) -> Option<QueuedAction> {
        // Gen 8+: every pending action's order as the re-sort will see it.
        let refreshed: Vec<ActionOrder> = match &self.pending {
            PendingExecution::Turn {
                actions,
                strategy: crate::rules::ActionOrderStrategy::RecalculateAfterAction,
                ..
            } => actions
                .iter()
                .map(|action| self.refreshed_order(action))
                .collect(),
            _ => Vec::new(),
        };
        let PendingExecution::Turn {
            actions,
            strategy,
            resort_ties,
            ..
        } = &mut self.pending
        else {
            return None;
        };
        if actions.is_empty() {
            return None;
        }

        if *strategy == crate::rules::ActionOrderStrategy::RecalculateAfterAction {
            // Gen 8+ `updateSpeed` after an action followed by a move.
            if *resort_ties && next_is_move(actions) {
                crate::engine::stats::update_handler_speeds(&mut self.state, &self.rules);
            }
            let trick_room = self.state.field.contains(FieldConditionKind::TrickRoom);
            for (action, order) in actions.iter_mut().zip(refreshed) {
                action.order = order;
            }
            // Showdown re-sorts the queue after every action followed by a move, and its
            // `speedSort` shuffles each group of tied actions anew: one SpeedTie decision per
            // tied action, in queue order (as at the start of the turn).
            if *resort_ties && next_is_move(actions) {
                let keys: Vec<_> = actions.iter().map(|action| action.order.key()).collect();
                for (index, action) in actions.iter_mut().enumerate() {
                    if tied_at(&keys, index) {
                        action.order.tie_break =
                            self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
                    }
                }
            }
            for action in actions.iter() {
                self.trace.push(TraceEvent::ActionOrderResolved {
                    actor: action.actor,
                    action_priority: action.order.action_priority,
                    move_priority: action.order.move_priority,
                    fractional_priority: action.order.fractional_priority,
                    effective_speed: action.order.speed,
                    trick_room,
                    tie_break: action.order.tie_break,
                });
            }
            ActionQueue::sort_actions(actions, trick_room);
            self.trace.push(TraceEvent::ActionQueueReordered {
                reason: super::QueueReorderReason::DynamicSpeed,
            });
        }

        *resort_ties = true;
        Some(actions.remove(0))
    }

    internal!(
        /// The Pokémon that draw a `SpeedTie` as Neutralizing Gas clears and `holder`'s `End`
        /// starts the others' abilities again (`BattleContext::end_ability`), in draw order: a
        /// replay plans them from the order the abilities announce themselves.
        pub fn neutralizing_gas_restart_ties(&self, holder: PokemonId) -> Vec<PokemonId> {
            let speeds: Vec<(PokemonId, u32)> = self
                .active_pokemon()
                .into_iter()
                .filter(|other| *other != holder)
                .map(|id| {
                    (
                        id,
                        crate::engine::stats::action_speed(&self.state, &self.rules, id, None),
                    )
                })
                .collect();
            crate::engine::context::restart_ties(&self.state, &self.rules, &speeds)
        }
    );

    internal!(
        /// The SpeedTie decisions of the step about to run (Gen 8+ re-sorts the queue before each
        /// first action, when a move comes next): each tied pending action with its group, in the
        /// order the SpeedTie decisions are drawn. Empty when the step draws none. Lets a replay
        /// plan those decisions from the order Showdown reported.
        pub fn pending_speed_ties(&self) -> Vec<(PokemonId, usize)> {
            let PendingExecution::Turn {
                actions,
                strategy,
                resort_ties,
                ..
            } = &self.pending
            else {
                return Vec::new();
            };
            if *strategy != crate::rules::ActionOrderStrategy::RecalculateAfterAction
                || !*resort_ties
                || !next_is_move(actions)
            {
                return Vec::new();
            }
            let keys: Vec<_> = actions
                .iter()
                .map(|action| self.refreshed_order(action).key())
                .collect();
            let mut groups: Vec<(i16, i8, i8, u32)> = Vec::new();
            let mut tied = Vec::new();
            for (index, action) in actions.iter().enumerate() {
                if !tied_at(&keys, index) {
                    continue;
                }
                let group = groups
                    .iter()
                    .position(|g| *g == keys[index])
                    .unwrap_or_else(|| {
                        groups.push(keys[index]);
                        groups.len() - 1
                    });
                tied.push((action.actor, group));
            }
            tied
        }
    );

    internal!(
        /// The Pokémon the switches still queued this turn will bring in (a turn a mid-turn
        /// request paused: an Eject Pack's). Lets a replay keep its party order for them.
        pub fn pending_switch_ins(&self) -> Vec<PokemonId> {
            let PendingExecution::Turn { actions, .. } = &self.pending else {
                return Vec::new();
            };
            actions
                .iter()
                .filter_map(|action| match action.choice {
                    ActionChoice::Switch { pokemon } => Some(pokemon),
                    _ => None,
                })
                .collect()
        }
    );

    internal!(
        pub fn replacement_requirements(&self) -> [ReplacementRequirement; 2] {
            [
                self.replacement_requirement(SideId::P1),
                self.replacement_requirement(SideId::P2),
            ]
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

    fn resolve_forced_switches_inner(
        &mut self,
        choices: [ForcedSwitchChoice; 2],
    ) -> Result<TurnResult, BattleError> {
        self.ensure_phase(BattlePhase::ForcedSwitch)?;
        let requirements = self.replacement_requirements();

        for (index, expected_side) in [SideId::P1, SideId::P2].into_iter().enumerate() {
            let choice = &choices[index];
            if choice.side != expected_side {
                return Err(ChoiceError::WrongSide {
                    expected: expected_side,
                    actual: choice.side,
                }
                .into());
            }
            self.validate_replacements(choice, &requirements[index])?;
        }

        let mut entrants = Vec::new();
        for side in [SideId::P1, SideId::P2] {
            self.state.side_mut(side).refill_mid_turn = None;
        }
        self.state.faint_refills_held = false;
        for (index, choice) in choices.into_iter().enumerate() {
            for replacement in choice.replacements {
                let slot = requirements[index]
                    .positions
                    .iter()
                    .find(|slot| slot.position == replacement.position)
                    .copied()
                    .ok_or(BattleError::InvalidReplacementPosition(
                        replacement.position,
                    ))?;
                // One placement path for both kinds of slot: an ordered-out Pokémon is cleaned
                // up and replaced, an empty slot is filled. Every entrant then runs its
                // switch-in below in one speed-ordered batch, as Showdown's runSwitch does.
                // Gen 4 (Showdown `switchIn` with the U-turn / Baton Pass as source effect): the
                // replacement takes over the user's last move.
                let passed_move = slot
                    .outgoing
                    .filter(|_| self.rules.conditions.self_switch_passes_last_move)
                    .and_then(|outgoing| self.state.pokemon(outgoing))
                    .and_then(|outgoing| outgoing.last_move);
                let mut handover = None;
                if let Some(outgoing) = slot.outgoing {
                    self.state.side_mut(choice.side).switch_out = None;
                    // Showdown `switchIn` → `queue.cancelAction`: a Pokémon Eject Button sent out
                    // before it acted loses its action, even if a drag brings it back this turn.
                    if let PendingExecution::Turn { actions, .. } = &mut self.pending {
                        actions.retain(|action| action.actor != outgoing);
                    }
                    self.run_switch_out(outgoing);
                    handover = self.take_handover(choice.side, outgoing, replacement.pokemon);
                    self.clear_on_switch(outgoing);
                }
                self.place(replacement.position, slot.outgoing, replacement.pokemon)?;
                if let (Some(outgoing), Some(handover)) = (slot.outgoing, handover) {
                    self.hand_over(outgoing, replacement.pokemon, handover);
                }
                if let Some(move_id) = passed_move
                    && let Some(incoming) = self.state.pokemon_mut(replacement.pokemon)
                {
                    incoming.last_move = Some(move_id);
                    incoming.last_move_encore = Some(move_id);
                    incoming.last_move_base = None;
                }
                entrants.push(replacement.pokemon);
            }
        }
        // The replacements' `switch` actions end with their `Update`.
        self.run_update(PendingFaints::Processed);

        // Simultaneous entries run their switch-in together, fastest first; only genuine speed
        // ties draw a SpeedTie decision (`run_switch_ins`: Gen 1-4 one after another, an
        // entrant the hazards knock out fainting before the next one enters; Gen 5+ one sorted
        // `SwitchIn` event). When a faint decides the battle nobody else enters.
        let entrants = self.speed_ordered(entrants);
        self.run_switch_ins(&entrants);
        self.resolve_faints();

        if self.state.outcome.is_none() {
            // The switch-ins' `runSwitch` action ends with its `Update`: Gen 5+ before any
            // replacement request, Gen 1-4 only when none follows (below).
            if self.rules.update.before_replacement {
                self.run_update(PendingFaints::Processed);
            }
            // Showdown's `runAction` ends the switch with `checkFainted` when nothing is queued
            // after it (an end-of-turn replacement) or, in Gen 1-3, when a move or the residual
            // comes next: a newcomer knocked out by the hazards loses its status to the faint.
            let mid_turn = matches!(&self.pending, PendingExecution::Turn { .. });
            if !mid_turn
                || (self.rules.faint.replace_after_each_move && !self.next_pending_is_switch())
            {
                self.check_fainted();
            }
            let requirements = self.replacement_requirements();
            if requirements
                .iter()
                .any(|requirement| requirement.required_count > 0)
            {
                self.feed_status_berries_before_request();
                self.set_phase(BattlePhase::ForcedSwitch);
            } else {
                if !self.rules.update.before_replacement {
                    self.run_update(PendingFaints::Processed);
                }
                // A turn suspended mid-way still owns its pending execution (even with no
                // action left, the end-of-turn residual has not run); a turn that ended with a
                // faint replacement has already released it.
                let resume_turn = matches!(&self.pending, PendingExecution::Turn { .. });
                if resume_turn {
                    // A Revival Blessing owed with the switch comes next (R-M8-171).
                    let phase = if self.revive_request().is_some() {
                        BattlePhase::Revive
                    } else {
                        BattlePhase::TurnResolution
                    };
                    self.set_phase(phase);
                } else if matches!(self.pending, PendingExecution::Start) {
                    // A lead ordered out as the battle started (Eject Pack): the start-of-battle
                    // `endTurn` comes now (`run_lead_switch_ins`), and the replacement starts the
                    // first turn as the leads do.
                    self.pending = PendingExecution::Idle;
                    self.state.turn = 1;
                    for pokemon in self.active_pokemon() {
                        if let Some(pokemon) = self.state.pokemon_mut(pokemon) {
                            pokemon.active_turns = 1;
                            pokemon.history.newly_switched = false;
                        }
                    }
                    self.set_phase(BattlePhase::TurnChoice);
                    self.end_turn_rules();
                } else {
                    self.next_turn();
                    if self.state.outcome.is_none() {
                        self.set_phase(BattlePhase::TurnChoice);
                    }
                }
            }
        }
        Ok(self.turn_result())
    }

    /// The ordering of one declared action before its priority chances are rolled: action
    /// priority, move priority, the fixed fractional priority and the chances after it
    /// (DESIGN.md §12.4), and the action speed (`quick_claw` overrides the turn's Gen 2-3
    /// Quick Claw roll for a preview).
    /// `struggles`: whether the move action is Struggle — decided as the action is chosen
    /// (`must_struggle`: Showdown's `chooseMove` with no move left), and from then on what the
    /// turn decided (`state.struggling`): a later re-sort keeps the chosen move's priority, a
    /// move sealed since included (R-M10-8).
    fn prepare_action(
        &self,
        slot_choice: &SlotChoice,
        quick_claw: Option<bool>,
        struggles: bool,
    ) -> Result<PreparedAction, BattleError> {
        let actor = slot_choice.actor;
        let speed = action_speed(&self.state, &self.rules, actor, quick_claw);
        // The move the action runs with (its priority and category): `None` for a switch or a
        // pass, Showdown's `recharge` pseudo-move for a recharging Pokémon (priority 0; not a
        // real move, so no category at all: Quick Draw, which only skips Status moves, still
        // rolls), Struggle (priority 0, Physical) for one with no move left.
        let mut prankster_boosted = false;
        let (action_priority, move_priority, category) = match slot_choice.action {
            ActionChoice::Move { .. }
                if self.state.pokemon(actor).is_some_and(|pokemon| {
                    pokemon
                        .volatiles
                        .contains(VolatileConditionKind::MustRecharge)
                }) =>
            {
                (self.rules.action_order.move_action_priority, 0, Some(None))
            }
            // Struggle as the action was chosen: an item lost since (an Assault Vest Corrosive
            // Gas took) leaves the action Struggle when the Gen 8+ queue re-sorts, and a move
            // sealed since (a Taunt on a Pokémon of status moves alone) leaves it its move.
            ActionChoice::Move { .. } if struggles => (
                self.rules.action_order.move_action_priority,
                0,
                Some(Some(MoveCategory::Physical)),
            ),
            // Gen 1 binding (§30.9): a bound Pokémon's action is `cannotmove` (priority 0) unless
            // it is held to a move (Bide: that move); one locked into its binding move uses that
            // move.
            ActionChoice::Move { .. }
                if self.rules.conditions.switching.binding_locks_user
                    && self.state.pokemon(actor).is_some_and(|pokemon| {
                        !pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
                            && !pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                            && ((pokemon
                                .volatiles
                                .contains(VolatileConditionKind::PartiallyTrapped)
                                && !binding::held_to_a_move(pokemon))
                                || pokemon.volatiles.partial_trapping_lock().is_some())
                    }) =>
            {
                let locked = self
                    .state
                    .pokemon(actor)
                    .and_then(|pokemon| pokemon.volatiles.partial_trapping_lock())
                    .and_then(|(_, move_id, ..)| self.dex.move_by_id(move_id));
                (
                    self.rules.action_order.move_action_priority,
                    locked.map_or(0, |move_def| move_def.priority),
                    Some(locked.map(|move_def| move_def.category)),
                )
            }
            // Gen 1: a frozen or sleeping Pokémon acts with its side's last selected move.
            ActionChoice::Move { .. }
                if self
                    .rules
                    .action_order
                    .sleeping_frozen_act_with_last_selected
                    && self.state.pokemon(actor).is_some_and(|pokemon| {
                        pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
                            || pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                    }) =>
            {
                let selected = self
                    .state
                    .pokemon(actor)
                    .and_then(|pokemon| self.state.side(pokemon.side).last_selected_move)
                    .and_then(SelectedMove::move_id)
                    .and_then(|move_id| self.dex.move_by_id(move_id));
                (
                    self.rules.action_order.move_action_priority,
                    selected.map_or(0, |move_def| move_def.priority),
                    Some(Some(selected.map_or(MoveCategory::Physical, |move_def| {
                        move_def.category
                    }))),
                )
            }
            ActionChoice::Move { slot, mechanic, .. } => {
                let move_id = self
                    .action_move(actor, slot)
                    .ok_or(BattleError::InvalidMoveSlot { actor, slot })?;
                let move_def = self
                    .dex
                    .move_by_id(move_id)
                    .ok_or(BattleError::UnknownMove(move_id))?;
                // A Dynamaxed Pokémon's move is its Max Move, and `getActionSpeed` reads the
                // priority of the move it will use — `dex.moves.get(maxMove.id).priority`, which
                // Max Guard has at +4 (DESIGN.md §44.5).
                let modified = crate::engine::modified_move_cow(
                    &self.state,
                    &self.rules,
                    actor,
                    move_def,
                    false,
                )
                .0;
                // A damaging move's Z-Move orders by its own priority too (`getActionSpeed`:
                // `dex.moves.get(zMove.id).priority`, 0 — Black Hole Eclipse from Sucker Punch
                // goes at 0, though as it runs it keeps the base move's, R-M9-35); a status
                // move's Z version is the move itself.
                let z_ordering = (mechanic == Some(MechanicChoice::ZMove))
                    .then(|| {
                        crate::engine::z_move_execution(
                            &self.state,
                            &self.rules,
                            &self.dex,
                            actor,
                            move_def,
                        )
                    })
                    .flatten()
                    .filter(|z| z.move_def.id != move_def.id)
                    .and_then(|z| self.dex.move_by_id(z.move_def.id));
                let ordering_def = (mechanic == Some(MechanicChoice::Dynamax)
                    || self.state.pokemon(actor).is_some_and(|pokemon| {
                        pokemon.volatiles.contains(VolatileConditionKind::Dynamax)
                    }))
                .then(|| crate::engine::max_move_for(&self.dex, &modified))
                .flatten()
                .and_then(|max| self.dex.move_by_id(max))
                .or(z_ordering)
                .unwrap_or(move_def);
                let (priority, prankster) = self.modified_priority(actor, ordering_def);
                prankster_boosted = prankster;
                (
                    self.rules.action_order.move_action_priority,
                    priority,
                    Some(Some(modified.category)),
                )
            }
            ActionChoice::Switch { .. } => {
                (self.rules.action_order.switch_action_priority, 0, None)
            }
            ActionChoice::Pass => (self.rules.action_order.pass_action_priority, 0, None),
        };
        let (fractional, chances) = match category {
            Some(category) => fractional_priority(&self.state, &self.rules, actor, category),
            None => (0, Vec::new()),
        };
        let custap = category.is_some() && self.custap_goes_off(actor);
        Ok(PreparedAction {
            actor,
            choice: slot_choice.action.clone(),
            action_priority,
            move_priority,
            prankster_boosted,
            fractional,
            chances,
            custap,
            speed,
        })
    }

    /// The move a move action uses: a locked Pokémon's lock (Showdown's request offers
    /// `getLockedMove` / `getSemiLockedMove` alone, and the action names it — a called Bide or
    /// two-turn move outside the slots too), else the slot's. Its priority orders the action
    /// (R-M8-72) and Me First / Sucker Punch read it (R-M8-66).
    fn action_move(
        &self,
        actor: PokemonId,
        slot: crate::ids::MoveSlot,
    ) -> Option<crate::ids::MoveId> {
        let pokemon = self.state.pokemon(actor)?;
        match pokemon
            .hard_lock()
            .map(|lock| lock.move_id)
            .or_else(|| pokemon.semi_locked_move())
        {
            Some(locked) => Some(locked),
            None => pokemon.move_slot(slot).map(|slot| slot.move_id),
        }
    }

    /// Custap Berry (Gen 4 `onBeforeTurn`, Gen 5+ `onFractionalPriority`, priority −2: after
    /// Stall and the chances): the holder is at a quarter of its HP (half with Gluttony) and would
    /// eat it — no foe's Unnerve refuses it.
    fn custap_goes_off(&self, actor: PokemonId) -> bool {
        let Some(pokemon) = self.state.pokemon(actor) else {
            return false;
        };
        let gluttony = super::ability_handler(&self.state, &self.rules, pokemon, |b| b.gluttony);
        super::held_item(&self.state, &self.rules, actor)
            .is_some_and(|(_, behavior)| behavior.pinch_priority)
            && self.rules.conditions.berries.reached(
                crate::rules::HpShare::Pinch,
                pokemon.hp,
                pokemon.max_hp,
                gluttony,
            )
            && !crate::engine::context::unnerved_by_a_foe(&self.state, &self.rules, actor)
    }

    /// A Custap Berry that went off (`PreparedAction::custap`) puts the action first in its
    /// bracket: Gen 5+ the holder eats it now (`-activate … [consumed]`), Gen 4 as its move
    /// begins (`BattleState::custap_ready`). False when the holder could not eat it after all.
    fn custap(&mut self, actor: PokemonId) -> bool {
        if self.rules.action_order.custap_eaten_as_the_move_begins {
            self.state.custap_ready.push(actor);
            return true;
        }
        let mut ctx =
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        match ctx.eat_item(actor) {
            Some(item) => {
                ctx.item_activated(actor, item);
                true
            }
            None => false,
        }
    }

    /// Showdown `ModifyPriority` (DESIGN.md §12.5) on the user's ability: Prankster +1 for a
    /// status move, which is then `pranksterBoosted`; Gale Wings +1 for a Flying move (from
    /// Gen 7 at full HP); Triage +3 for a healing move — after the move's own (Grassy Glide +1
    /// in Grassy Terrain for a grounded user).
    fn modified_priority(
        &self,
        actor: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) -> (i8, bool) {
        let mut priority = move_def.priority;
        let Some(pokemon) = self.state.pokemon(actor) else {
            return (priority, false);
        };
        if let Some(terrain) = move_def.behavior.terrain_priority
            && self.state.field.terrain.map(|t| t.kind) == Some(terrain)
            && crate::engine::environment::grounded(&self.state, &self.rules, pokemon)
        {
            priority += 1;
        }
        let mut prankster = false;
        match super::ability_handler(&self.state, &self.rules, pokemon, |b| b.modify_priority) {
            Some(PriorityBoost::Prankster) if move_def.category == MoveCategory::Status => {
                priority += 1;
                prankster = true;
            }
            Some(PriorityBoost::GaleWings)
                if move_def.move_type == crate::dex::PokemonType::Flying
                    && (!self.rules.action_order.gale_wings_full_hp
                        || pokemon.hp == pokemon.max_hp) =>
            {
                priority += 1;
            }
            Some(PriorityBoost::Triage) if move_def.flags.heal => priority += 3,
            _ => {}
        }
        (priority, prankster)
    }

    /// A queued action's order as a Gen 8+ re-sort sees it (Showdown `getActionSpeed` again):
    /// its move priority re-run (Gale Wings may have lost its full HP) and its speed updated;
    /// the fractional priority and tie break stay.
    fn refreshed_order(&self, action: &QueuedAction) -> ActionOrder {
        let mut order = action.order;
        order.speed = action_speed(&self.state, &self.rules, action.actor, None);
        let slot_choice = SlotChoice {
            actor: action.actor,
            action: action.choice.clone(),
        };
        let struggles = self.state.struggling.contains(&action.actor);
        if let Ok(prepared) = self.prepare_action(&slot_choice, None, struggles) {
            order.move_priority = prepared.move_priority;
            order.prankster_boosted = prepared.prankster_boosted;
        }
        order
    }

    /// Showdown `FractionalPriority` for a move action (DESIGN.md §12.4): Lagging Tail / Full
    /// Showdown `FractionalPriority` for a move action `resolveAction` builds (DESIGN.md §12.4):
    /// its chances in handler order, each that goes off announced, then a Custap Berry at a
    /// pinch (eaten now from Gen 5). The fractional priority they leave.
    fn roll_fractional(&mut self, action: &PreparedAction) -> i8 {
        let mut fractional = settle_fractional(action, |chance| {
            let (numerator, denominator) = chance.odds();
            let rolled = self.state.rng.roll_ratio_for(
                RandomRequestKind::PriorityChance,
                numerator,
                denominator,
            );
            if rolled {
                self.trace.push(activation(action.actor, chance));
                self.state.priority_chance_activated.push(action.actor);
            }
            rolled
        });
        if action.custap && fractional <= 0 && self.custap(action.actor) {
            fractional = 1;
        }
        fractional
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
    /// Declared actions in canonical order (P1 then P2, by position), as Showdown's
    /// `resolveAction` meets them (and rolls their priority chances).
    fn prepared_actions(
        &self,
        choices: &[SideChoice; 2],
        quick_claw: Option<bool>,
    ) -> Result<Vec<PreparedAction>, BattleError> {
        let mut prepared = Vec::new();
        for side_choice in choices {
            let mut canonical_actions = side_choice.actions.iter().collect::<Vec<_>>();
            canonical_actions.sort_by_key(|slot_choice| {
                self.state
                    .active_position(slot_choice.actor, &self.rules.topology)
                    .map(|position| position.index())
                    .unwrap_or(usize::MAX)
            });
            for slot_choice in canonical_actions {
                let struggles = super::choice::must_struggle(
                    &self.state,
                    &self.rules,
                    &self.dex,
                    slot_choice.actor,
                );
                prepared.push(self.prepare_action(slot_choice, quick_claw, struggles)?);
            }
        }
        Ok(prepared)
    }

    /// The ordering of a prepared action once its chances are settled (Gen 4: a delayed move
    /// orders slowest first).
    fn settled_order(&self, action: &PreparedAction, fractional: i8) -> ActionOrder {
        ActionOrder {
            action_priority: action.action_priority,
            move_priority: action.move_priority,
            fractional_priority: fractional,
            speed: action.speed,
            slowest_first: fractional < 0 && self.rules.action_order.delayed_slowest_first,
            fastest_first: fractional > 0 && self.rules.action_order.boosted_fastest_first,
            tie_break: 0,
            prankster_boosted: action.prankster_boosted,
        }
    }

    /// Whether some Pokémon of the battle could use this turn's Gen 2-3 Quick Claw roll: the
    /// roll is only drawn then.
    fn quick_claw_turn_holder(&self) -> Option<PokemonId> {
        self.rules.action_order.quick_claw_turn_roll?;
        self.state
            .all_pokemon()
            .iter()
            .find(|pokemon| {
                !pokemon.is_fainted()
                    && super::held_item_id(&self.state, &self.rules, pokemon.id())
                        == Some(crate::ids::ItemId::QUICK_CLAW)
            })
            .map(|pokemon| pokemon.id())
    }

    /// Showdown `side.commitChoices` (`lastSelectedMove`): a move picked from the menu becomes
    /// the side's last selected move, Struggle too; a partially trapped Pokémon's turn clears
    /// it (`cannotmove`); a frozen, sleeping, recharging or otherwise locked Pokémon leaves it.
    fn record_selected_moves(&mut self, choices: &[SideChoice; 2]) {
        for side_choice in choices {
            for slot_choice in &side_choice.actions {
                let ActionChoice::Move { slot, .. } = slot_choice.action else {
                    continue;
                };
                let actor = slot_choice.actor;
                let Some(pokemon) = self.state.pokemon(actor) else {
                    continue;
                };
                let mut selected_slot = None;
                let selected = if pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
                    || pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                    || binding::held_to_a_move(pokemon)
                    // The recharge turn's `recharge` choice has no slot (R-M8-107).
                    || super::choice::recharging(&self.state, actor)
                {
                    continue;
                } else if pokemon
                    .volatiles
                    .contains(VolatileConditionKind::PartiallyTrapped)
                {
                    Some(SelectedMove::CannotMove)
                } else if super::choice::must_struggle(&self.state, &self.rules, &self.dex, actor) {
                    Some(SelectedMove::Move(crate::ids::MoveId::STRUGGLE))
                } else {
                    selected_slot = Some(slot);
                    pokemon
                        .move_slot(slot)
                        .map(|slot| SelectedMove::Move(slot.move_id))
                };
                let side = pokemon.side;
                self.state.side_mut(side).last_selected_move = selected;
                // Gen 1 Mimic reads this back (`side.lastSelectedMoveSlot`): only a pick from the
                // menu sets it; frozen, sleeping and locked Pokémon leave it as it was.
                if let Some(slot) = selected_slot {
                    self.state.side_mut(side).last_selected_move_slot = slot.index();
                }
            }
        }
    }

    internal!(
        /// The turn order these choices would produce, with every priority chance decided by
        /// `activated` instead of drawn (DESIGN.md §12.4): a replay that knows the outcomes (from
        /// an oracle's log or action order) can pre-fill the semantic tape. The Gen 2-3 turn roll
        /// is asked of the first Quick Claw holder.
        pub fn preview_action_order(
            &self,
            choices: &[SideChoice; 2],
            activated: &dyn Fn(PokemonId, PriorityChance) -> bool,
        ) -> Result<ActionOrderPreview, BattleError> {
            let mut chances = Vec::new();
            let quick_claw = self.quick_claw_turn_holder().map(|holder| {
                let rolled = activated(holder, PriorityChance::QuickClawTurn);
                chances.push((holder, PriorityChance::QuickClawTurn, rolled));
                rolled
            });
            let prepared = self.prepared_actions(choices, Some(quick_claw.unwrap_or(false)))?;
            let mut orders = Vec::new();
            for action in &prepared {
                let mut fractional = settle_fractional(action, |chance| {
                    let rolled = activated(action.actor, chance);
                    chances.push((action.actor, chance, rolled));
                    rolled
                });
                if action.custap && fractional <= 0 {
                    fractional = 1;
                }
                orders.push((action.actor, self.settled_order(action, fractional)));
            }
            let keys: Vec<_> = orders.iter().map(|(_, order)| order.key()).collect();
            let mut groups: Vec<(i16, i8, i8, u32)> = Vec::new();
            let mut ties = Vec::new();
            for (index, (actor, _)) in orders.iter().enumerate() {
                if !tied_at(&keys, index) {
                    continue;
                }
                let group = groups
                    .iter()
                    .position(|g| *g == keys[index])
                    .unwrap_or_else(|| {
                        groups.push(keys[index]);
                        groups.len() - 1
                    });
                ties.push((*actor, group));
            }
            let trick_room = self.state.field.contains(FieldConditionKind::TrickRoom);
            let mut queued: Vec<QueuedAction> = prepared
                .iter()
                .zip(&orders)
                .map(|(action, (actor, order))| QueuedAction {
                    actor: *actor,
                    choice: action.choice.clone(),
                    step: ActionStep::of(&action.choice),
                    order: *order,
                    prioritized_by: None,
                    instructed: false,
                    original_target: crate::engine::action::original_target(
                        &self.state,
                        &self.rules.topology,
                        &action.choice,
                    ),
                })
                .collect();
            ActionQueue::sort_actions(&mut queued, trick_room);
            let mut order: Vec<Vec<PokemonId>> = Vec::new();
            let mut last_key = None;
            for action in &queued {
                let key = action.order.key();
                match order.last_mut() {
                    Some(bracket) if last_key == Some(key) => bracket.push(action.actor),
                    _ => order.push(vec![action.actor]),
                }
                last_key = Some(key);
            }
            Ok(ActionOrderPreview {
                chances,
                order,
                ties,
            })
        }
    );

    fn build_action_queue(
        &mut self,
        choices: &[SideChoice; 2],
    ) -> Result<ActionQueue, BattleError> {
        // Gen 2-3: the turn's one Quick Claw roll (Showdown draws it as the turn starts).
        self.state.quick_claw_roll = false;
        if let (Some(odds), Some(_)) = (
            self.rules.action_order.quick_claw_turn_roll,
            self.quick_claw_turn_holder(),
        ) {
            self.state.quick_claw_roll = self.state.rng.roll_ratio_for(
                RandomRequestKind::PriorityChance,
                odds.numerator,
                odds.denominator,
            );
        }
        // Showdown `commitChoices`: `updateSpeed` before the queue is sorted.
        crate::engine::stats::update_handler_speeds(&mut self.state, &self.rules);
        let prepared = self.prepared_actions(choices, None)?;

        // Showdown `resolveAction`: every move action rolls its priority chances, in canonical
        // order, before the queue is sorted; a chance that goes off announces itself.
        self.state.priority_chance_activated.clear();
        let mut orders = Vec::new();
        self.state.custap_ready.clear();
        for action in &prepared {
            let fractional = self.roll_fractional(action);
            orders.push(self.settled_order(action, fractional));
        }

        // A move choice with a gimmick becomes two queue entries (DESIGN.md §44.2): the gimmick's
        // step (order 106, its own action priority, the move's speed) and the move itself.
        let trick_room = self.state.field.contains(FieldConditionKind::TrickRoom);
        let mut entries: Vec<(ActionStep, ActionOrder, ActionChoice, PokemonId)> = Vec::new();
        for (action, order) in prepared.into_iter().zip(orders) {
            let step = ActionStep::of(&action.choice);
            if step != ActionStep::Act {
                entries.push((
                    step,
                    ActionOrder {
                        action_priority: step.action_priority(&self.rules.action_order, 0),
                        move_priority: 0,
                        fractional_priority: 0,
                        speed: order.speed,
                        slowest_first: false,
                        fastest_first: false,
                        tie_break: 0,
                        prankster_boosted: false,
                    },
                    action.choice.clone(),
                    action.actor,
                ));
                // The move action keeps no mechanic: the gimmick is its own step (a Dynamaxed
                // Pokémon's later moves convert on their own, Q2's answer).
                entries.push((
                    ActionStep::Act,
                    order,
                    without_mechanic(action.choice),
                    action.actor,
                ));
                continue;
            }
            entries.push((ActionStep::Act, order, action.choice, action.actor));
        }
        // A SpeedTie decision is drawn only for actions that genuinely tie (same priorities and
        // speed), one per tied action in canonical order; Showdown draws nothing otherwise.
        let keys: Vec<_> = entries.iter().map(|(_, order, _, _)| order.key()).collect();
        let mut queue = ActionQueue::default();
        for (index, (step, mut order, choice, actor)) in entries.into_iter().enumerate() {
            if tied_at(&keys, index) {
                order.tie_break = self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
            }
            self.trace.push(TraceEvent::ActionOrderResolved {
                actor,
                action_priority: order.action_priority,
                move_priority: order.move_priority,
                fractional_priority: order.fractional_priority,
                effective_speed: order.speed,
                trick_room,
                tie_break: order.tie_break,
            });
            let original_target =
                crate::engine::action::original_target(&self.state, &self.rules.topology, &choice);
            queue.push(QueuedAction {
                actor,
                choice,
                step,
                order,
                prioritized_by: None,
                instructed: false,
                original_target,
            });
        }

        queue.sort_with_trick_room(trick_room);
        Ok(queue)
    }

    /// The gimmick itself: it is used up for its side (`SideMechanicState`) and takes effect.
    fn apply_mechanic(
        &mut self,
        pokemon: PokemonId,
        mechanic: MechanicChoice,
    ) -> Result<(), BattleError> {
        match mechanic {
            MechanicChoice::Terastallize => {
                // A refused Terastallization (an Ogerpon of another type) spends nothing.
                if !self.terastallize(pokemon)? {
                    return Ok(());
                }
            }
            MechanicChoice::Dynamax => self.dynamax(pokemon)?,
            MechanicChoice::MegaEvolve => self.mega_evolve(pokemon, false)?,
            MechanicChoice::UltraBurst => self.mega_evolve(pokemon, true)?,
            // A Z-Move is spent as its move is used (the Z power).
            MechanicChoice::ZMove => {}
        }
        crate::engine::spend_mechanic(&mut self.state, &mut self.trace, pokemon, mechanic)
    }

    /// Showdown `runMegaEvo` (DESIGN.md §44.3): the Pokémon takes its mega forme — or Ultra
    /// Necrozma with the Ultranecrozium Z — through the forme-change primitive, sourced at the
    /// item it holds, so the forme's own ability starts with it (`-mega` / `-burst` are the
    /// item-sourced forme change's log, which the frames' species carry here). The side has no
    /// Mega Evolution left; Gen 7 recalculates the Pokémon's move action afterwards.
    fn mega_evolve(&mut self, pokemon: PokemonId, ultra: bool) -> Result<(), BattleError> {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return Err(BattleError::UnknownPokemon(pokemon));
        };
        let target = if ultra {
            holder.ultra_bursts_into
        } else {
            holder.mega_evolves_into
        };
        let Some(target) = target else {
            return Ok(());
        };
        let item = holder.held_item;
        // The line names the forme's `requiredItem` (`-mega|p|species|item`), the stone or the
        // Ultranecrozium Z held — none for Mega Rayquaza, whose forme asks a move instead
        // (`requiredMove`): the item it holds stays unseen (R-M10-62).
        let named = item.filter(|_| {
            !holder
                .formes
                .iter()
                .any(|forme| forme.species == target && forme.required_move.is_some())
        });
        let source = item.map_or(EffectSource::Rule, |item| EffectSource::Item {
            pokemon,
            item,
        });
        crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
            .forme_change(
                pokemon,
                target,
                crate::engine::FormeChangeKind::Permanent,
                source,
            );
        // Showdown sets `moveThisTurnResult = true`: the action counts as the Pokémon's for
        // Truant and Stomping Tantrum.
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.history.move_this_turn = Some(crate::state::MoveResult::Succeeded);
        }
        self.trace.push(TraceEvent::MegaEvolved {
            pokemon,
            species: target,
            ultra,
            item: named,
        });
        // Gen 7: the Pokémon's move action is taken out of the queue and put back at its new
        // speed (`updateSpeed`, then `insertChoice`), among the actions its order ties with.
        if self.rules.action_order.mega_reorders_move {
            crate::engine::stats::update_handler_speeds(&mut self.state, &self.rules);
            self.reinsert_after_mega(pokemon);
        }
        self.run_after_mega();
        Ok(())
    }

    /// Gen 7 `runAction`'s `insertChoice(queuedAction, true)` after a Mega Evolution: the
    /// actor's pending move action goes back into the queue at the draw's place among the
    /// actions its order ties with (one `QueueInsert` decision).
    fn reinsert_after_mega(&mut self, actor: PokemonId) {
        let PendingExecution::Turn { actions, .. } = &self.pending else {
            return;
        };
        let Some(index) = actions.iter().position(|action| {
            action.actor == actor
                && action.step == ActionStep::Act
                && matches!(action.choice, ActionChoice::Move { .. })
        }) else {
            return;
        };
        let mut action = actions[index].clone();
        action.order = self.refreshed_order(&action);
        let trick_room = self.state.field.contains(FieldConditionKind::TrickRoom);
        let earlier = |left: &ActionOrder, right: &ActionOrder| {
            crate::engine::action::order_before(left, right, trick_room)
        };
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        actions.remove(index);
        // Showdown's `insertChoice`: the equal range is [first, last] and the index is drawn
        // from it (`battle.random(firstIndex, lastIndex + 1)`).
        let mut first = actions.len();
        let mut last = actions.len();
        let mut seen_first = false;
        for (i, current) in actions.iter().enumerate() {
            if earlier(&current.order, &action.order) {
                if seen_first {
                    last = i;
                    break;
                }
                continue;
            }
            if !seen_first {
                first = i;
                seen_first = true;
            }
            if earlier(&action.order, &current.order) {
                last = i;
                break;
            }
        }
        let index = if first >= last {
            first
        } else {
            self.state.rng.range_inclusive_for(
                RandomRequestKind::QueueInsert,
                first as u16,
                (last - 1) as u16,
            ) as usize
        };
        actions.insert(index, action);
    }

    /// Showdown `runEvent('AfterMega')` of the Pokémon on the field: the abilities' and items'
    /// `onAnyAfterMega` (Opportunist, White Herb, Eject Pack, Mirror Herb) — the same handlers
    /// their `onAnyAfterMove` runs — fastest first, a holder's ability before its item.
    fn run_after_mega(&mut self) {
        let mut holders: Vec<(PokemonId, u32, u8, crate::effects::SwitchHandler)> = Vec::new();
        for pokemon in self.active_pokemon() {
            let speed = super::stats::handler_speed(&self.state, pokemon);
            if let Some(handler) = self.state.pokemon(pokemon).and_then(|p| {
                super::ability_handler(&self.state, &self.rules, p, |b| b.any_after_mega)
            }) {
                holders.push((pokemon, speed, 0, handler));
            }
            if let Some(handler) = super::held_item(&self.state, &self.rules, pokemon)
                .and_then(|(_, behavior)| behavior.any_after_mega)
            {
                holders.push((pokemon, speed, 1, handler));
            }
        }
        holders.sort_by_key(|(_, speed, sub, _)| (std::cmp::Reverse(*speed), *sub));
        for (pokemon, _, _, handler) in holders {
            let mut ctx =
                crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            handler(&mut ctx, pokemon);
        }
    }

    /// Showdown `runAction`'s `runDynamax` + `addVolatile('dynamax')` (DESIGN.md §44.5): the
    /// Pokémon Dynamaxes — Minimize and Substitute go, Torment ends silently, and the HP is
    /// multiplied by `1.5 + 0.05 × its Dynamax level` (Shedinja excepted).
    fn dynamax(&mut self, pokemon: PokemonId) -> Result<(), BattleError> {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return Err(BattleError::UnknownPokemon(pokemon));
        };
        let level = holder.dynamax_level;
        let base_max_hp = holder.max_hp;
        // `pokemon.baseSpecies.name === 'Shedinja'`: its own species, whatever it transformed into
        // (DESIGN.md R-M9-25: the check named national dex number 514, which is Simisear).
        let shedinja = holder.species == Some(crate::ids::SpeciesId::SHEDINJA);
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.volatiles.set_dynamax(base_max_hp);
            holder.volatiles.remove(VolatileConditionKind::Minimize);
            holder.volatiles.remove(VolatileConditionKind::Substitute);
            if holder.volatiles.remove(VolatileConditionKind::Torment) {
                self.trace.push(TraceEvent::VolatileEnded {
                    target: pokemon,
                    kind: VolatileConditionKind::Torment,
                    seen: true,
                });
            }
        }
        self.trace.push(TraceEvent::DynamaxStarted { pokemon });
        // Changes based on dynamax level, 2 is max (at LVL 10). Shedinja keeps its one point.
        if !shedinja {
            let ratio = 1.5 + f64::from(level.min(10)) * 0.05;
            if let Some(holder) = self.state.pokemon_mut(pokemon) {
                let max_hp = (f64::from(holder.max_hp) * ratio).floor() as u16;
                let hp = (f64::from(holder.hp) * ratio).floor() as u16;
                holder.max_hp = max_hp.max(1);
                holder.hp = hp.min(holder.max_hp);
            }
            self.trace.push(TraceEvent::HealApplied {
                target: pokemon,
                amount: 0,
                hp_before: 0,
                hp_after: self.state.pokemon(pokemon).map_or(0, |p| p.hp),
                source: EffectSource::Rule,
            });
        }
        Ok(())
    }

    /// Showdown `dynamax.onEnd` (DESIGN.md §44.5): the HP comes back to its undynamaxed value
    /// and the max HP returns.
    fn end_dynamax(&mut self, pokemon: PokemonId) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        let Some(base_max_hp) = holder.volatiles.dynamax_base_max_hp() else {
            return;
        };
        let hp = holder.undynamaxed_hp(None);
        self.trace.push(TraceEvent::DynamaxEnded { pokemon });
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.volatiles.remove(VolatileConditionKind::Dynamax);
            holder.max_hp = base_max_hp;
            holder.hp = hp.min(base_max_hp);
        }
        self.trace.push(TraceEvent::HealApplied {
            target: pokemon,
            amount: 0,
            hp_before: 0,
            hp_after: self.state.pokemon(pokemon).map_or(0, |p| p.hp),
            source: EffectSource::Rule,
        });
    }

    /// Showdown `actions.terastallize` (DESIGN.md §44.6): the Pokémon takes its Tera type, its
    /// added type goes, Ogerpon's `-Tera` forme and Terapagos's Stellar one follow, and the
    /// `AfterTerastallization` handlers run. False when nothing happened at all: an Ogerpon
    /// whose Tera type is none of Fire, Grass, Rock and Water (Showdown's hint and return) —
    /// its side's Terastallization is not spent then.
    fn terastallize(&mut self, pokemon: PokemonId) -> Result<bool, BattleError> {
        let Some((tera_type, own, battling)) = self
            .state
            .pokemon(pokemon)
            .map(|p| (p.tera_type, p.species, crate::engine::species_of(p)))
        else {
            return Err(BattleError::UnknownPokemon(pokemon));
        };
        // Ogerpon's Tera type is one of its masks' (Showdown's `hint`): anything else does
        // nothing at all (44.10 的 S2).
        if own.is_some_and(|own| own.same_species(crate::ids::SpeciesId::OGERPON))
            && !matches!(
                tera_type,
                crate::dex::PokemonType::Fire
                    | crate::dex::PokemonType::Grass
                    | crate::dex::PokemonType::Rock
                    | crate::dex::PokemonType::Water
            )
        {
            return Ok(false);
        }
        // An Illusion that wears an Ogerpon's or Terapagos's face ends first (Showdown ends the
        // ability as it terastallizes): the disguise would otherwise be the form the log names.
        let disguised = self
            .state
            .pokemon(pokemon)
            .and_then(|holder| holder.illusion)
            .and_then(|illusion| self.state.pokemon(illusion))
            .and_then(|illusion| illusion.species);
        if disguised.is_some_and(|species| {
            species.same_species(crate::ids::SpeciesId::OGERPON)
                || species.same_species(crate::ids::SpeciesId::TERAPAGOS)
        }) {
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .end_illusion(pokemon);
        }
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.terastallized = Some(tera_type);
            holder.added_type = None;
        }
        self.trace
            .push(TraceEvent::Terastallized { pokemon, tera_type });
        // Ogerpon's `-Tera` forme (its masks' forms, permanent and source-less): its ability
        // becomes the matching Embody Aspect, which starts and raises its stat.
        if let Some(forme) = battling.and_then(crate::effects::abilities::tera::ogerpon_tera_forme)
            && let Some(holder) = self.state.pokemon(pokemon)
            && holder.forme_of(forme).is_some()
        {
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .forme_change(
                    pokemon,
                    forme,
                    crate::engine::FormeChangeKind::Permanent,
                    EffectSource::Rule,
                );
        }
        // Morpeko keeps the forme it is in (Showdown marks it `formeRegression` and takes it
        // as the base): a faint puts the set's Morpeko back (DESIGN.md §44.6).
        if battling.is_some_and(|species| species.same_species(crate::ids::SpeciesId::MORPEKO))
            && let Some(holder) = self.state.pokemon(pokemon)
            && holder.transform.is_none()
            && holder.species != battling
            && let Some(forme) = battling.and_then(|species| holder.forme_of(species).copied())
            && let Some(holder) = self.state.pokemon_mut(pokemon)
        {
            holder.take_forme(&forme);
            holder.forme_regression = true;
        }
        // Terapagos-Terastal becomes its Stellar forme (Teraform Zero then runs below).
        if battling == Some(crate::ids::SpeciesId::TERAPAGOS_TERASTAL) {
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .forme_change(
                    pokemon,
                    crate::ids::SpeciesId::TERAPAGOS_STELLAR,
                    crate::engine::FormeChangeKind::Permanent,
                    EffectSource::Rule,
                );
        }
        self.run_after_terastallization();
        Ok(true)
    }

    /// Showdown `runEvent('AfterTerastallization')` of the Pokémon on the field: the abilities'
    /// (Opportunist) and items' (Mirror Herb) `onAnyAfterTerastallization`, fastest first, a
    /// holder's ability before its item.
    fn run_after_terastallization(&mut self) {
        let mut holders: Vec<(PokemonId, u32, u8, crate::effects::SwitchHandler)> = Vec::new();
        for pokemon in self.active_pokemon() {
            let speed = super::stats::handler_speed(&self.state, pokemon);
            // The Terastallized Pokémon's own `onAfterTerastallization` (Teraform Zero) runs
            // with the others'.
            if let Some(handler) = self.state.pokemon(pokemon).and_then(|p| {
                let behavior = super::ability_handlers(&self.state, &self.rules, p);
                behavior
                    .after_terastallization
                    .or(behavior.any_after_terastallization)
            }) {
                holders.push((pokemon, speed, 0, handler));
            }
            if let Some(handler) = super::held_item(&self.state, &self.rules, pokemon)
                .and_then(|(_, behavior)| behavior.any_after_terastallization)
            {
                holders.push((pokemon, speed, 1, handler));
            }
        }
        holders.sort_by_key(|(_, speed, sub, _)| (std::cmp::Reverse(*speed), *sub));
        for (pokemon, _, _, handler) in holders {
            let mut ctx =
                crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            handler(&mut ctx, pokemon);
        }
    }
}

impl BattleEngine {
    fn replacement_requirement(&self, side: SideId) -> ReplacementRequirement {
        let side_state = self.state.side(side);
        // A turn suspended mid-way (U-turn, Eject Button) asks only for the slots ordered out;
        // a slot emptied by a faint is filled once the turn is over (Showdown `checkFainted`
        // runs only when the action queue is empty), and after a residual Emergency Exit's
        // replacement (`BattleState::faint_refills_held`).
        let mid_turn = (matches!(self.pending, PendingExecution::Turn { .. })
            && !self.rules.faint.replace_after_each_move)
            || self.state.faint_refills_held;
        let positions = self
            .rules
            .topology
            .positions()
            .iter()
            .filter(|position| position.side == side)
            .filter_map(|position| match side_state.active[position.slot as usize] {
                // Gen 4 Healing Wish / Lunar Dance: the user's slot is filled at once.
                None if mid_turn && side_state.refill_mid_turn != Some(position.slot as usize) => {
                    None
                }
                None => Some(ReplacementSlot {
                    position: position.id,
                    outgoing: None,
                }),
                Some(occupant) if side_state.switch_out == Some(occupant) => {
                    Some(ReplacementSlot {
                        position: position.id,
                        outgoing: Some(occupant),
                    })
                }
                Some(_) => None,
            })
            .collect::<Vec<_>>();
        let available_pokemon = self.state.healthy_bench(side);
        let required_count = positions.len().min(available_pokemon.len());

        ReplacementRequirement {
            side,
            positions,
            available_pokemon,
            required_count,
        }
    }

    /// Some side has an empty slot and a healthy Pokémon to fill it.
    fn fainted_slot_to_fill(&self) -> bool {
        [SideId::P1, SideId::P2].into_iter().any(|side| {
            let side_state = self.state.side(side);
            side_state.active.iter().any(Option::is_none)
                && !self.state.healthy_bench(side).is_empty()
        })
    }

    /// An ordered-out Pokémon with a bench to come from suspends the turn; without a bench the
    /// order lapses (Showdown: the switch flag is dropped when nothing can switch in).
    fn mid_turn_replacement_needed(&mut self) -> bool {
        let mut needed = false;
        for side in [SideId::P1, SideId::P2] {
            if let Some(slot) = self.state.side(side).refill_mid_turn {
                if self
                    .state
                    .side(side)
                    .active
                    .get(slot)
                    .is_some_and(Option::is_none)
                    && !self.state.healthy_bench(side).is_empty()
                {
                    needed = true;
                } else {
                    self.state.side_mut(side).refill_mid_turn = None;
                }
            }
            if self.state.side(side).switch_out.is_none() {
                continue;
            }
            if self.state.healthy_bench(side).is_empty() {
                self.state.side_mut(side).switch_out = None;
            } else {
                needed = true;
            }
        }
        needed
    }

    fn validate_replacements(
        &self,
        choice: &ForcedSwitchChoice,
        requirement: &ReplacementRequirement,
    ) -> Result<(), BattleError> {
        if choice.replacements.len() != requirement.required_count {
            return Err(ChoiceError::WrongReplacementCount {
                expected: requirement.required_count,
                actual: choice.replacements.len(),
            }
            .into());
        }

        let mut positions = Vec::new();
        let mut pokemon = Vec::new();
        for replacement in &choice.replacements {
            if !requirement
                .positions
                .iter()
                .any(|slot| slot.position == replacement.position)
            {
                return Err(ChoiceError::InvalidReplacementPosition(replacement.position).into());
            }
            if positions.contains(&replacement.position) {
                return Err(ChoiceError::DuplicateReplacementPosition(replacement.position).into());
            }
            positions.push(replacement.position);

            if !requirement.available_pokemon.contains(&replacement.pokemon) {
                return Err(ChoiceError::SwitchTargetNotOnTeam(replacement.pokemon).into());
            }
            if pokemon.contains(&replacement.pokemon) {
                return Err(ChoiceError::DuplicateSwitchTarget(replacement.pokemon).into());
            }
            pokemon.push(replacement.pokemon);
        }

        Ok(())
    }

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
