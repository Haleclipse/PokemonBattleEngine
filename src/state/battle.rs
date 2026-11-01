use crate::ids::{MoveId, PokemonId, PositionId, SideId};
use crate::rules::BattleTopology;

use super::{BattleRng, FieldState, PokemonState, SideState};

/// The phases of a battle under way. What comes before it — registration, Team Preview, the
/// picks — is the pre-battle session's (DESIGN.md §45.5): a battle starts at
/// its first turn's choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattlePhase {
    TurnChoice,
    TurnResolution,
    ForcedSwitch,
    /// A side names the fainted Pokémon Revival Blessing brings back (`BattleRequest::Revive`).
    Revive,
    EndTurn,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleOutcome {
    Winner(SideId),
    Draw,
}

/// Marker in the id lookup table for ids that are not part of this battle.
const NO_POKEMON: u8 = u8::MAX;

/// Upper bound on Pokémon per battle imposed by the `u8` index table. A battle needs at most
/// twelve, so this is a structural limit rather than a practical one.
pub const MAX_POKEMON_PER_BATTLE: usize = NO_POKEMON as usize;

/// Who knocked a Pokémon out, recorded as its HP reaches 0 (Showdown's `faintQueue` entry:
/// `source`, and whether `effect` is a move). `faintMessages` hands it to the faint handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaintCause {
    pub target: PokemonId,
    /// The Pokémon behind the damage, when there is one.
    pub source: Option<PokemonId>,
    /// The damage was a move's (`effect.effectType === 'Move'`).
    pub by_move: bool,
    /// The move was a foreseen one striking (`effect.flags['futuremove']`).
    pub future_move: bool,
    /// Its `Faint` event's listeners (Soul-Heart, Receiver) already heard it, at the hit loop's
    /// end — before a knocked-out Neutralizing Gas ended (`knocked_out_gas_ends`, DESIGN.md
    /// R-M8-93), or with its batch's `AfterFaint` (R-M10-52): the faint at the action's end does
    /// not run them again.
    pub listeners_run: bool,
    /// Its own `onFaint` handlers (Destiny Bond, Grudge) already ran, at the hit loop's end
    /// (`knocked_out_bonds`): the faint at the action's end does not run them again — a Leppa
    /// Berry may have given the PP Grudge took back in between (R-M8-185).
    pub bonds_run: bool,
    /// Its batch's `AfterFaint` already ran, at the hit loop's end (`announce_knocked_out`,
    /// R-M10-52): the faint at the action's end leaves it out of its own batch.
    pub after_faint_run: bool,
    /// Its side's `totalFainted` counts it already, from the hit loop's end (`count_faint`): the
    /// faint at the action's end does not count it again (R-M10-61).
    pub counted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleState {
    pub turn: u32,
    pub phase: BattlePhase,
    /// Owned Pokémon storage. Kept private so the id → index table below can never go stale.
    /// Reads go through `all_pokemon()` / `pokemon()`, writes through `pokemon_mut()`.
    pokemon: Vec<PokemonState>,
    /// `PokemonId` → index into `pokemon`, built once at construction. Every rule stage
    /// addresses Pokémon by id, so lookup must be O(1) and allocation-free.
    lookup: Box<[u8; 256]>,
    pub sides: [SideState; 2],
    pub field: FieldState,
    pub rng: BattleRng,
    /// What the battle has shown its observers (DESIGN.md §47.0 item 12): the views' alone.
    pub knowledge: super::Knowledge,
    pub outcome: Option<BattleOutcome>,
    /// Pokémon whose HP reached 0, in the order it happened, until the faint step consumes
    /// them (Showdown `faintQueue`: decides which side wins a double knock-out from Gen 5).
    pub faint_queue: Vec<PokemonId>,
    /// Who knocked each `faint_queue` Pokémon out (the `source` and `effect` of its Showdown
    /// `faintQueue` entry), consumed with it.
    pub faint_causes: Vec<FaintCause>,
    /// Pokémon ordered out for a random bench Pokémon (Roar, Dragon Tail, Red Card), dragged
    /// at the end of the action (Showdown `forceSwitchFlag`).
    pub drag_queue: Vec<PokemonId>,
    /// Actors whose declared move has not run yet this turn (Showdown `queue.willMove`):
    /// Taunt / Encore / Disable durations depend on whether the target still moves.
    pub pending_movers: Vec<PokemonId>,
    /// Gen 1: the Pokémon whose action this turn is their side's last selected move, chosen as
    /// `Fight` (frozen or asleep as the turn began) — the only ones Showdown's Desync Clause Mod
    /// can stop (`PpRules::desync_clause`, R-M8-150).
    pub fight_actors: Vec<PokemonId>,
    /// The move slot the action under way chose, and whose action it is (Showdown's
    /// `action.moveSlot`): the slot Gen 2-3's Mimic and Sketch write when they are that slot's
    /// move — the second of two Mimics too (`MovesSlot::TheChosen`, issues.md I170). `None`
    /// between actions.
    pub acting_slot: Option<(PokemonId, usize)>,
    /// The Pokémon handed an item while that action was under way (Showdown `setItem`: a Trick,
    /// a Bestow, a Thief): Gen 3's Choice lock skips one its own move handed it (issues.md
    /// I184). Empty between actions — the engine reads its state, never its trace (R-M10-2).
    pub items_received_in_action: Vec<PokemonId>,
    /// Actors that had no move left to choose this turn (Showdown's request held Struggle
    /// alone): decided at the choice, their action is Struggle whatever slot it names.
    pub struggling: Vec<PokemonId>,
    /// The move each actor's action of this turn names (Showdown's queued `action.move`), with
    /// its category and priority as ordered (Gen 6+ `action.move.priority`): what Sucker Punch
    /// and Upper Hand read of a target that still has to move (`pending_movers`).
    pub queued_moves: Vec<QueuedMove>,
    /// The "Curse glitch" (Trick-or-Treat, Gen 6-8 doubles): Pokémon in the second slot whose
    /// queued Curse now aims at their ally's slot (`action.targetLoc = -1`), as their move runs.
    pub curse_glitched: Vec<crate::ids::PokemonId>,
    /// The fainted Pokémon of `faint_queue` Showdown's `faintMessages` has reached already — at
    /// the end of a move's hit loop — though the engine processes them at the end of the
    /// action: their side's `pokemonLeft` no longer counts them.
    pub announced_faints: Vec<crate::ids::PokemonId>,
    /// An Emergency Exit in the residual called off the switch flags `checkFainted` had just
    /// put on the fainted (Showdown `onEmergencyExit` clears every active's `switchFlag`): the
    /// emptied slots wait for a request of their own, after the exit's.
    pub faint_refills_held: bool,
    /// Gen 1 (Showdown `battle.lastDamage`): the last damage a move, a confusion self-hit, a
    /// crash or a drain dealt anywhere in the battle, before the HP cap — what Gen 1 Counter
    /// doubles; a move's use resets it (`RetaliationRules::keeps_last_damage` aside).
    pub last_damage: u16,
    /// The effect order the next ability state started takes (Showdown `battle.effectOrder`,
    /// counted here for ability states only: only their relative order is read).
    pub next_ability_order: u32,
    /// Showdown `beingCalledBack` / `switchFlag` while `BeforeSwitchOut` runs: the Pokémon
    /// leaving the field that Pursuit strikes at double power (DESIGN.md §15.11).
    pub called_back: Option<PokemonId>,
    /// Showdown `skipBeforeSwitchOutEventFlag`: Pokémon ordered out whose `BeforeSwitchOut` has
    /// already run, before their replacement request.
    pub skip_before_switch_out: Vec<PokemonId>,
    /// Gen 2 (Showdown `draggedIn`): the Pokémon a drag just brought in, until its switch-in has
    /// run (`SwitchingRules::dragged_skips_switch_in_status`).
    pub dragged_in: Vec<PokemonId>,
    /// Gen 1 (Showdown `cannotmove`): actors partially trapped as the turn's choices were made;
    /// their action is lost, whatever move it names (DESIGN.md §30.9).
    pub cannot_move: Vec<PokemonId>,
    /// Showdown `lastSuccessfulMoveThisTurn` (Fusion Flare / Fusion Bolt): the last move this
    /// turn that ran and did something; a move that fails, or finds everyone protected, clears
    /// it. A Pokémon that cannot act (BeforeMove) leaves it alone.
    pub last_successful_move: Option<MoveId>,
    /// Gen 2-3 Showdown `quickClawRoll`: this turn's Quick Claw roll succeeded, so every
    /// holder acts at Speed 65535 until the next turn's roll.
    pub quick_claw_roll: bool,
    /// The actors whose priority chance (Quick Claw, Quick Draw) went off as this turn's
    /// actions were ordered (`priority_item_pending` reads it).
    pub priority_chance_activated: Vec<PokemonId>,
    /// Gen 4 Custap Berry (`ActionOrderRules::custap_eaten_as_the_move_begins`): the holders it
    /// put first this turn, which eat it as their move begins.
    pub custap_ready: Vec<PokemonId>,
    /// Showdown `!queue.peek()` (Cud Chew): the turn's actions are over — from the residual on,
    /// through its `Update` and the replacements after it, until the next turn's choices.
    pub turn_actions_done: bool,
    /// Showdown `pokemon.speed`, which orders event handlers mid-turn (redirection): each
    /// active Pokémon's action speed (Trick Room inverted) as last refreshed — when a turn's
    /// choices are committed and, Gen 8+, after each action followed by a move
    /// (`engine::stats::update_handler_speeds`; the residual refreshes it too, and reads the
    /// speed of that moment directly). One never refreshed reads its Speed stat
    /// (`storedStats.spe`).
    handler_speeds: Vec<(PokemonId, u32)>,
    /// The move being used (Showdown `activeMove`, DESIGN.md §41.2): set as a move starts,
    /// cleared once its action is over (after its drags, before its faints) and as the residual
    /// starts — never set at a request boundary.
    pub active_move: Option<ActiveMove>,
    /// What the move in progress asked of the action queue (After You, Quash: Showdown
    /// `queue.prioritizeAction`, `action.order = 201`), applied once its action is over —
    /// nothing reads the queue in between. Never set at a request boundary.
    pub queue_requests: Vec<QueueRequest>,
    /// The move the effect in progress makes its user use next (a nested move, §42.1).
    pub pending_call: Option<MoveCall>,
    /// Showdown `battle.lastMove` (Copycat): the last move used in the battle — the innermost
    /// one of an action's chain from Gen 5, the outermost in Gen 1-4 (`activeMove =
    /// oldActiveMove`), written as the action ends (`clearActiveMove`). A BeforeMove that
    /// stopped the action leaves it alone (`clearActiveMove(true)`).
    pub last_move: Option<MoveId>,
    /// Showdown `battle.lastMove.baseMove`: the move a Max Move was made from, kept beside
    /// `last_move` (DESIGN.md §44.5).
    pub last_move_base: Option<MoveId>,
    /// Showdown `battle.lastMove.isZ` for a status move used as a Z-Move (its id is the move
    /// itself): Copycat refuses it (DESIGN.md R-M9-36).
    pub last_move_z: bool,
}

/// Who made a Pokémon use a move it did not choose (DESIGN.md §42.1, Showdown `sourceEffect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSource {
    Move(MoveId),
    Ability(crate::ids::AbilityId),
}

/// Where a called move aims (Showdown `useMove`'s `target`): drawn at execution time, or the
/// Pokémon the caller named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallTarget {
    Random,
    Pokemon(PokemonId),
}

/// A move another effect makes its user use now (Showdown `useMove`, DESIGN.md §42.1): the
/// caller (Metronome, Sleep Talk, Magic Coat …) asks through `BattleContext::call_move` and the
/// Shell runs it once the caller's own effect is over. Never set at a request boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveCall {
    pub user: PokemonId,
    pub move_id: MoveId,
    pub target: CallTarget,
    pub source: CallSource,
    /// A reflection's own Prankster boost (`newMove.pranksterBoosted`: Magic Coat's, as it was
    /// used; none for Magic Bounce or a side reflection). A call that is no reflection takes the
    /// move it runs inside's instead (DESIGN.md R-M8-88).
    pub reflected_prankster: bool,
}

/// A change to the action queue a move asked for (`BattleState::queue_requests`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueRequest {
    /// After You: the Pokémon's pending action goes to the front.
    Prioritize(PokemonId),
    /// Quash: the Pokémon's pending action goes last (`ActionOrderRules::quash`).
    Quash(PokemonId),
    /// Instruct (DESIGN.md §42.9): a fresh action of its own for `actor`, using `move_id` from
    /// its slot, at the front of the queue (`queue.resolveAction` + `prioritizeAction`).
    Instruct {
        actor: PokemonId,
        move_id: MoveId,
        slot: usize,
        target_position: Option<crate::ids::PositionId>,
    },
}

/// The move being used (`BattleState::active_move`; Showdown `activeMove` / `activePokemon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveMove {
    pub user: PokemonId,
    /// The move's id (`activeMove.id`: Eject Pack's exception for Parting Shot).
    pub move_id: MoveId,
    /// The category the move has as it is used (`activeMove.category`: Shell Side Arm may have
    /// turned it physical; Ice Face asks it, DESIGN.md §43.4).
    pub category: crate::dex::MoveCategory,
    /// Showdown `move.stellarBoosted` (DESIGN.md §44.1): Stellar's boost was applied to this
    /// use of the move, so every other target of the same spread hit gets it too — the move
    /// object is shared, while `PokemonState::stellar_boosted_types` is spent per type.
    pub stellar_boosted: bool,
    /// Showdown `move.isZOrMaxPowered` (DESIGN.md §44.4): the move is being used through a Z
    /// Crystal (or, later, a Dynamax). It is what the exclusions read — Parental Bond's second
    /// hit, Gorilla Tactics' and the Choice items' lock, the `-ate` abilities' type change.
    pub z_powered: bool,
    /// `move.ignoreAbility`: from the move itself (Sunsteel Strike) or its user's ability
    /// (Mold Breaker); while it is used, the other Pokémon's breakable abilities do nothing
    /// (`engine::suppressing_ability`).
    pub ignores_ability: bool,
    /// Showdown `move.baseMove` (DESIGN.md §44.5): the move a Max Move was made from. Only Max
    /// Moves carry one; Copycat resolves the battle's last move through it.
    pub base_move: Option<MoveId>,
    /// Showdown `move.pranksterBoosted`: Prankster raised the move's priority (Magic Coat keeps
    /// its own for the move it sends back, DESIGN.md R-M8-88).
    pub prankster_boosted: bool,
}

/// One actor's move action of the turn (`BattleState::queued_moves`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuedMove {
    pub actor: PokemonId,
    pub move_id: MoveId,
    pub category: crate::dex::MoveCategory,
    pub priority: i8,
    /// The action asks for the move's Z-Power (Showdown `action.zmove`): Me First refuses it.
    pub z_powered: bool,
    /// The action is a Max Move (Showdown `action.maxMove`): the actor Dynamaxes this turn or
    /// already has. Round and the Pledges pass it over.
    pub max_move: bool,
    /// The actor must recharge: the action is Showdown's `recharge` pseudo-move (its lock), not
    /// `move_id`, the move its slot names.
    pub recharge: bool,
}

impl QueuedMove {
    /// The move a walk of the queue finds the action using (Showdown `action.move`, Round's
    /// `onTry`, a Pledge's `onPrepareHit`): none for a Max Move, a Z-Move (`action.maxMove ||
    /// action.zmove`, passed over) or a recharge.
    pub fn walked_move(&self) -> Option<MoveId> {
        (!self.max_move && !self.z_powered && !self.recharge).then_some(self.move_id)
    }
}

impl BattleState {
    /// pokeplatinum's `moveEffectsData.quickClaw` / `custapBerry` (issues.md I52, I53): as this
    /// turn's actions were ordered, `pokemon`'s Quick Claw went off (`priority_chance_activated`;
    /// Gen 4 has no other chance) or its Custap Berry did (`custap_ready`), and it has not begun
    /// its move — the cartridge clears both flags then. Gen 4 Embargo, Knock Off, Thief and Covet
    /// read it.
    pub fn priority_item_pending(&self, pokemon: PokemonId) -> bool {
        self.pending_movers.contains(&pokemon)
            && (self.priority_chance_activated.contains(&pokemon)
                || self.custap_ready.contains(&pokemon))
    }

    /// `pokemon`'s move action of this turn, if it still has to move (Showdown
    /// `queue.willMove`).
    pub fn will_move_with(&self, pokemon: PokemonId) -> Option<QueuedMove> {
        if !self.pending_movers.contains(&pokemon) {
            return None;
        }
        self.queued_moves
            .iter()
            .copied()
            .find(|queued| queued.actor == pokemon)
    }

    pub fn new(pokemon: Vec<PokemonState>, sides: [SideState; 2], seed: u64) -> Self {
        assert!(
            pokemon.len() <= MAX_POKEMON_PER_BATTLE,
            "a battle cannot address more than {MAX_POKEMON_PER_BATTLE} Pokémon"
        );

        let mut lookup = Box::new([NO_POKEMON; 256]);
        for (index, entry) in pokemon.iter().enumerate() {
            let slot = &mut lookup[usize::from(entry.id().0)];
            // Duplicate ids are rejected by engine validation; keeping the first occurrence
            // lets the validator observe a consistent view while it reports the duplicate.
            if *slot == NO_POKEMON {
                *slot = index as u8;
            }
        }

        Self {
            turn: 1,
            phase: BattlePhase::TurnChoice,
            pokemon,
            lookup,
            sides,
            field: FieldState::default(),
            rng: BattleRng::new(seed),
            knowledge: super::Knowledge::default(),
            outcome: None,
            faint_queue: Vec::new(),
            faint_causes: Vec::new(),
            drag_queue: Vec::new(),
            pending_movers: Vec::new(),
            fight_actors: Vec::new(),
            acting_slot: None,
            items_received_in_action: Vec::new(),
            quick_claw_roll: false,
            priority_chance_activated: Vec::new(),
            custap_ready: Vec::new(),
            turn_actions_done: false,
            struggling: Vec::new(),
            cannot_move: Vec::new(),
            queued_moves: Vec::new(),
            curse_glitched: Vec::new(),
            announced_faints: Vec::new(),
            faint_refills_held: false,
            last_damage: 0,
            next_ability_order: 1,
            called_back: None,
            skip_before_switch_out: Vec::new(),
            dragged_in: Vec::new(),
            last_successful_move: None,
            handler_speeds: Vec::new(),
            active_move: None,
            queue_requests: Vec::new(),
            pending_call: None,
            last_move: None,
            last_move_base: None,
            last_move_z: false,
        }
    }

    /// Showdown `clearActiveMove`: the move in progress is over. The move that ran last (of a
    /// chain, the innermost) becomes the battle's last move — a BeforeMove that stopped the
    /// action never set one (`clearActiveMove(true)`).
    pub fn clear_active_move(&mut self) {
        if let Some(active) = self.active_move.take() {
            self.last_move = Some(active.move_id);
            self.last_move_base = active.base_move;
            // A Max Move carries its base move; any other Z- or Max-powered move is a Z-Move.
            self.last_move_z = active.z_powered && active.base_move.is_none();
        }
    }

    /// The speed `pokemon`'s event handlers are ordered by (Showdown `pokemon.speed`).
    pub fn handler_speed(&self, pokemon: PokemonId) -> u32 {
        self.handler_speeds
            .iter()
            .find(|(id, _)| *id == pokemon)
            .map(|(_, speed)| *speed)
            .or_else(|| {
                self.pokemon(pokemon)
                    .map(|p| u32::from(p.battle_stats().speed))
            })
            .unwrap_or(0)
    }

    /// Showdown `updateSpeed` for one Pokémon.
    pub fn set_handler_speed(&mut self, pokemon: PokemonId, speed: u32) {
        match self
            .handler_speeds
            .iter_mut()
            .find(|(id, _)| *id == pokemon)
        {
            Some(entry) => entry.1 = speed,
            None => self.handler_speeds.push((pokemon, speed)),
        }
    }

    /// Every Pokémon in the battle, in construction order.
    pub fn all_pokemon(&self) -> &[PokemonState] {
        &self.pokemon
    }

    pub fn pokemon_ids(&self) -> impl Iterator<Item = PokemonId> + '_ {
        self.pokemon.iter().map(|pokemon| pokemon.id())
    }

    fn index_of(&self, id: PokemonId) -> Option<usize> {
        let index = self.lookup[usize::from(id.0)];
        (index != NO_POKEMON).then_some(usize::from(index))
    }

    pub fn pokemon(&self, id: PokemonId) -> Option<&PokemonState> {
        self.index_of(id).and_then(|index| self.pokemon.get(index))
    }

    pub fn pokemon_mut(&mut self, id: PokemonId) -> Option<&mut PokemonState> {
        let index = self.index_of(id)?;
        self.pokemon.get_mut(index)
    }

    /// Starts `id`'s ability state over (Showdown `initEffectState` for `abilityState`: a
    /// switch-in, an ability set or swapped, a Transform): cleared, and stamped with the battle's
    /// next effect order.
    pub fn restart_ability_state(&mut self, id: PokemonId) {
        if let Some(pokemon) = self.pokemon_mut(id) {
            pokemon.ability_state = crate::state::AbilityState::default();
        }
        self.stamp_ability_order(id);
    }

    /// Stamps `id`'s ability state with the battle's next effect order, keeping the rest: a
    /// lead's state, fresh as the battle starts (`restart_ability_state` for the others).
    pub fn stamp_ability_order(&mut self, id: PokemonId) {
        let effect_order = self.next_ability_order;
        if let Some(pokemon) = self.pokemon_mut(id) {
            pokemon.ability_state.effect_order = effect_order;
            self.next_ability_order += 1;
        }
    }

    pub fn side(&self, side: SideId) -> &SideState {
        &self.sides[side.index()]
    }

    pub fn side_mut(&mut self, side: SideId) -> &mut SideState {
        &mut self.sides[side.index()]
    }

    pub fn active_position(
        &self,
        pokemon: PokemonId,
        topology: &BattleTopology,
    ) -> Option<PositionId> {
        for side in [SideId::P1, SideId::P2] {
            for (slot, active) in self.side(side).active.iter().enumerate() {
                if *active == Some(pokemon) {
                    return topology.position(side, slot as u8);
                }
            }
        }
        None
    }

    pub fn pokemon_at_position(
        &self,
        position: PositionId,
        topology: &BattleTopology,
    ) -> Option<PokemonId> {
        let position = topology.spec(position)?;
        self.side(position.side)
            .active
            .get(position.slot as usize)
            .copied()
            .flatten()
    }

    pub fn living_team_members(&self, side: SideId) -> Vec<PokemonId> {
        self.side(side)
            .team
            .iter()
            .copied()
            .filter(|pokemon| self.pokemon(*pokemon).is_some_and(|p| !p.is_fainted()))
            .collect()
    }

    /// Whether `pokemon` still has a move to make this turn.
    /// Ally Switch (Showdown `swapPosition`): the two Pokémon trade their slots on their side.
    /// False when either is not active. Position-keyed state (Wish, a foreseen attack, a trap)
    /// stays with the slot, as Showdown's does.
    pub fn swap_positions(&mut self, side: SideId, a: PokemonId, b: PokemonId) -> bool {
        let side = &mut self.sides[usize::from(side.ordinal())];
        let (Some(first), Some(second)) = (
            side.active.iter().position(|slot| *slot == Some(a)),
            side.active.iter().position(|slot| *slot == Some(b)),
        ) else {
            return false;
        };
        side.active.swap(first, second);
        // Showdown `swapPosition` trades their places in `side.pokemon` too (the party order
        // Beat Up, Illusion and the like read).
        if let (Some(first), Some(second)) = (
            side.battle_order.iter().position(|p| *p == a),
            side.battle_order.iter().position(|p| *p == b),
        ) {
            side.battle_order.swap(first, second);
        }
        true
    }

    pub fn will_move(&self, pokemon: PokemonId) -> bool {
        self.pending_movers.contains(&pokemon)
    }

    pub fn healthy_bench(&self, side: SideId) -> Vec<PokemonId> {
        let active = &self.side(side).active;
        self.living_team_members(side)
            .into_iter()
            .filter(|pokemon| !active.contains(&Some(*pokemon)))
            .collect()
    }
}
