//! The Move Shell (DESIGN.md §15): every move, ordinary or custom, runs the same sequence. One
//! file per stage:
//!
//! | file | stage |
//! |---|---|
//! | `shell` | the sequence itself (BeforeMove, PP, targeting, TryMove, the per-target loop), the move result (§22.8) |
//! | `targeting` | execution-time retargeting and redirection (§15.4) |
//! | `hit_gate` | semi-invulnerability, type immunity, TryHit, Substitute, one-hit KO immunity, accuracy |
//! | `primary` | declarative status primaries (§15.2) |
//! | `hit_sequence` | the damage primary's hits, contact reactions, recoil (§15.3) |
//! | `hit_reactions` | the chance reactions after the ordered ones: Effect Spore, Cursed Body, Poison Touch (§21) |
//! | `damage_calc` | one hit's damage: critical hit, fixed damage, the damage formulas (§19, §22.6) |
//! | `future_attack` | Future Sight / Doom Desire: the use foresees, the residual strikes (§15.22) |
//! | `base_power` | the BasePower stage (§22.5, §22.7) |
//! | `secondary` | secondary effects (§21) |
//! | `after_move` | self-switch, damaging phazing, item reactions, the user's item (§30) |

use crate::dex::ResolvedDex;
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedRules;
use crate::state::{BattleState, MoveResult};

use super::BattleTrace;
use super::damage::{DamageAmount, HitBlockReason};

mod after_move;
mod base_power;
mod bide;
mod binding;
mod cartridge_damage;
mod damage_calc;
mod future_attack;
mod hit_gate;
mod hit_reactions;
mod hit_sequence;
mod metronome;
mod primary;
mod rage;
mod rampage;
mod secondary;
mod shell;
mod targeting;
mod uproar;

pub use future_attack::future_strike;
pub(crate) use hit_gate::chart_effectiveness;
pub use hit_gate::type_immunity;
pub use targeting::{
    curse_redirect_candidates, deciding_redirector, random_retarget_candidates,
    redirector_cancels_smart_target, smart_targets,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitOutcome {
    Missed {
        target: PokemonId,
    },
    Blocked {
        target: PokemonId,
        reason: HitBlockReason,
    },
    Damaged {
        target: PokemonId,
        amount: DamageAmount,
    },
    /// A status move's effect landed on the target.
    Effect {
        target: PokemonId,
    },
    /// Magic Coat / Magic Bounce took the move over: it runs again with the reflector as its
    /// user (DESIGN.md §42.8).
    Reflected {
        target: PokemonId,
    },
    SubstituteDamaged {
        target: PokemonId,
        amount: DamageAmount,
        broken: bool,
    },
    NoEffect {
        target: PokemonId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveExecution {
    pub actor: PokemonId,
    pub move_id: MoveId,
    pub hits: Vec<HitOutcome>,
    /// BeforeMove stopped the action (sleep, freeze, paralysis, flinch, confusion …), or the
    /// move's own `beforeMoveCallback` did (Focus Punch without its focus, Bide).
    pub stopped: bool,
    /// Showdown's `MoveAborted` ran: a BeforeMove handler refused the action (Bide's release
    /// too), not the move's own `beforeMoveCallback`.
    pub aborted: bool,
    /// How the move went (Showdown `moveThisTurnResult`, DESIGN.md §22.8).
    pub result: MoveResult,
}

pub(crate) struct MovePipeline<'a> {
    state: &'a mut BattleState,
    rules: &'a ResolvedRules,
    dex: &'a ResolvedDex,
    trace: &'a mut BattleTrace,
    /// The move priority the action was ordered with (after `ModifyPriority`), when the move
    /// runs as a queued action, and whether Prankster raised it (DESIGN.md §12.5).
    action_priority: Option<i8>,
    prankster_boosted: bool,
    /// The hit of the sequence being resolved, 1 for the first (Showdown `move.hit`: Triple
    /// Kick's power).
    hit: u8,
    /// The hits the sequence was decided to have (Showdown `hits`), 1 outside a sequence.
    hit_count: u8,
    /// What the move drew as it started (Magnitude's power, Present's power or heal).
    drawn: Option<crate::effects::moves::DrawnPower>,
    /// The user's ability that retyped the move and boosts it (`typeChangerBoosted`).
    type_changer: Option<crate::ids::AbilityId>,
    /// The move that moved this action up the queue (Showdown `sourceEffect`: Round).
    prioritized_by: Option<crate::ids::MoveId>,
    /// The action's `originalTarget` (`QueuedAction::original_target`), for the outermost move.
    original_target: Option<PokemonId>,
    /// Round's `onTry` ran: the next queued use of this move is to go to the front.
    prioritizes: Option<crate::ids::MoveId>,
    /// Shell Trap armed by this move's hit (`queue.prioritizeAction`): these Pokémon's actions
    /// go to the front, the last armed first.
    prioritized_actors: Vec<PokemonId>,
    /// The targets whose Tera Shell resisted this move (Showdown `abilityState.resisted`, reset
    /// at a move's first hit): its later hits are resisted too (R-M9-45).
    tera_shell_resisted: Vec<PokemonId>,
    /// Showdown `move.mindBlownRecoil = false`: the move stopped at TryMove (Damp) owes no
    /// after-move self damage.
    self_damage_cancelled: bool,
    /// The action was the recharge turn (BeforeMove's `Recharge`): the move it aborted is
    /// Showdown's `recharge` (`mustrecharge.onLockMove`, a Normal move), not the one chosen
    /// (R-M10-57).
    recharged: bool,
    /// Mind Blown / Steel Beam's self damage has been paid (Showdown `mindBlownRecoil`, cleared
    /// once it is): after the first hit that struck, before a Parental Bond second one.
    self_damage_paid: bool,
    /// The recoil taken once the hits were over (Showdown `hitStepMoveHitLoop`: after its
    /// `faintMessages`) knocked the user out: its faint is still pending through the move's
    /// `Update` (R-M8-166).
    recoil_knocked_out: bool,
    /// Showdown `move.isExternal` (`runMove` with `externalMove`): a dance Dancer copies (an
    /// instructed action runs as any move). Disable from Cursed Body takes no turn off for it
    /// (DESIGN.md R-M8-93).
    external: bool,
    /// Beat Up's members, one hit each, fixed as the move starts (`engine::beat_up_members`).
    beat_up_members: Vec<PokemonId>,
    /// Dragon Darts split between two targets: the one dart this target takes.
    smart_hit: bool,
    /// The gem this move used up (`ItemBehavior::gem`): the move's own power boost, for its
    /// every hit and target and for nothing after it (issues.md I10).
    gem: Option<crate::ids::ItemId>,
    /// Every active Pokémon's HP as the hit loop starts, for the targets' Emergency Exit check
    /// once the move is over (Showdown `hurtThisTurn` + the move's damage).
    hp_before_hits: Vec<(PokemonId, u16)>,
    /// The focus items armed for this move (`ItemBehavior::holds_for_the_move`: Gen 4 Focus
    /// Sash, Gen 3-4 Focus Band).
    focus_items: Vec<ArmedFocusItem>,
    /// The action is the attacking turn of a two-turn move (the charge paid for it).
    releasing: bool,
    /// Gen 3-4 Pursuit striking a Pokémon as it switches out (Showdown `useMove` without
    /// `runMove`, §15.11): no BeforeMove, and it is no move action of its user's.
    skips_before_move: bool,
    /// A hard lock held the user to the move as it started (no PP spent, §15.12).
    continuing_lock: bool,
    /// Gen 3 (`HitStepOrder::AccuracyFirst`): a target immune to the move was missed by the
    /// accuracy roll too — Showdown reports `-immune` either way; Rage's flag needs which
    /// (§15.19).
    immune_target_missed: bool,
    /// Gen 3: Wonder Guard turned away a move the type chart had already made no effect
    /// (pokeemerald `Cmd_typecalc` checks it after the chart; `DOESNT_AFFECT_FOE` stays and sets
    /// `targetNotAffected`): Thrash's and Uproar's turn failed as against any immune target,
    /// though the block shows Wonder Guard (R-M8-135).
    type_immune_target_guarded: bool,
    /// The Z-Move this action runs in place of its slot's move (DESIGN.md §44.4): the resolved
    /// move (the Z-Move, or the powered base move for a status one) and what its power does.
    z_move: Option<ZMoveExecution>,
    /// The Z-Move above once the action runs it, its power not spent yet: that waits for the
    /// move to be used (`spend_z_power`, R-M9-29).
    z_power: Option<ZMoveExecution>,
    /// The Max Move this action runs in place of its slot's move (DESIGN.md §44.5).
    max_move: Option<crate::dex::MaxMove>,
    /// Showdown `maxMove.baseMove`: the move the Max Move above was made from, recorded as the
    /// move's and the battle's last move (DESIGN.md §44.5).
    max_base: Option<MoveId>,
    /// The move being used is Z-powered (`move.isZOrMaxPowered`): it goes through a protection
    /// for a quarter of its damage (`-zbroken`), and the exclusions below read it.
    z_powered: bool,
    /// This called move's caller is a Z status move (`sourceEffect.isZ`): a damaging one runs as
    /// its Z-Move (`called_z_move`, R-M9-41).
    z_call: bool,
    /// This run is a nested move (DESIGN.md §42.1): another effect made the Pokémon use it
    /// (Showdown `useMove(move, user, {sourceEffect})`), so the action layer is skipped.
    call: Option<CallFrame>,
    /// The move being used came out of Magic Coat or Magic Bounce (`move.hasBounced`): it is not
    /// reflected again (DESIGN.md §42.8).
    bounced: bool,
    /// The outermost move of this run's chain (Showdown `activeMove` at the action's end,
    /// Gen 4's Copycat): set by the first `run_move`.
    outermost: Option<MoveId>,
    /// The ally's Pledge this move waits for (Showdown `queue.prioritizeAction(action, move)`,
    /// DESIGN.md §42.9): the engine promotes its action with this move as its `sourceEffect`.
    pledge_promotes: Option<MoveId>,
}

/// A Z-Move as the pipeline runs it (DESIGN.md §44.4). Public: the replay driver asks the
/// engine what it will run, exactly as it asks for `modified_move` (TESTING.md §6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZMoveExecution {
    /// The move to run in place of the slot's: the Z-Move, or the base move for a status one.
    pub move_def: crate::dex::ResolvedMove,
    /// Showdown `isZ` on the move being used: an ability does not retype it, Encore refuses it,
    /// Gorilla Tactics and the Choice items leave it alone, and it breaks Protect.
    pub powered: bool,
    /// The base move the crystal converted (`move.baseMove`): its PP pays, and the slot's.
    pub base: crate::ids::MoveId,
    /// What a status move's Z power does before the move (`runZPower`).
    pub status_effect: Option<crate::dex::ZStatusEffect>,
}

/// A nested move's caller (Showdown `sourceEffect`): what asked for it (§42.1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallFrame {
    pub(crate) source: crate::state::CallSource,
    /// The move the action used, at the head of the chain (Gen 3's cartridge charges a called
    /// move's Pressure to its slot, `CalledMovePressure::TheActionsMove`); none for a move
    /// that comes outside the action (Dancer).
    pub(crate) action_move: Option<MoveId>,
}

/// How deep a chain of called moves may go (Metronome → Copycat → …): nothing in the games
/// reaches further, and a cycle would otherwise recurse forever.
pub(super) const MAX_CALL_DEPTH: u8 = 16;

/// A focus item armed for the move: it holds every hit of it that would knock `holder` out.
#[derive(Debug, Clone, Copy)]
pub(super) struct ArmedFocusItem {
    pub(super) holder: PokemonId,
    pub(super) item: crate::ids::ItemId,
    pub(super) how: crate::effects::items::HoldsForTheMove,
    /// It held a hit: a Focus Sash goes after the move.
    pub(super) held: bool,
}

impl<'a> MovePipeline<'a> {
    pub(crate) fn new(
        state: &'a mut BattleState,
        rules: &'a ResolvedRules,
        dex: &'a ResolvedDex,
        trace: &'a mut BattleTrace,
    ) -> Self {
        Self {
            state,
            rules,
            dex,
            trace,
            action_priority: None,
            prankster_boosted: false,
            hit: 1,
            hit_count: 1,
            drawn: None,
            type_changer: None,
            prioritized_by: None,
            original_target: None,
            prioritizes: None,
            prioritized_actors: Vec::new(),
            tera_shell_resisted: Vec::new(),
            self_damage_cancelled: false,
            recharged: false,
            self_damage_paid: false,
            recoil_knocked_out: false,
            external: false,
            beat_up_members: Vec::new(),
            smart_hit: false,
            gem: None,
            z_move: None,
            z_power: None,
            max_move: None,
            max_base: None,
            z_powered: false,
            z_call: false,
            hp_before_hits: Vec::new(),
            focus_items: Vec::new(),
            releasing: false,
            skips_before_move: false,
            continuing_lock: false,
            immune_target_missed: false,
            type_immune_target_guarded: false,
            call: None,
            bounced: false,
            outermost: None,
            pledge_promotes: None,
        }
    }

    /// The ally's Pledge this move promoted (the action's `sourceEffect`), if any.
    pub(crate) fn pledge_promotes(&self) -> Option<MoveId> {
        self.pledge_promotes
    }

    /// The outermost move of the chain (`MovePipeline::outermost`).
    pub(crate) fn outermost(&self) -> Option<MoveId> {
        self.outermost
    }

    /// The pipeline runs a move another effect called (§42.1): no action layer, the move comes
    /// from the call, and per-action records stay the caller's.
    pub(crate) fn called(mut self, frame: CallFrame) -> Self {
        self.call = Some(frame);
        // A called move is never a Max Move: the outer action's base does not carry over.
        self.max_base = None;
        // Showdown's `useMove` runs no BeforeMove; Dancer's `runMove` re-enables it
        // (`with_before_move`).
        self.skips_before_move = true;
        self
    }

    /// The called move runs as a full `runMove` (Dancer, §42.9): BeforeMove and its checks
    /// apply, and a blocked action is the caller's answer — PP and the last move still do not.
    pub(crate) fn with_before_move(mut self, before_move: bool) -> Self {
        self.skips_before_move = !before_move;
        self
    }

    /// This run's move came out of Magic Coat or Magic Bounce (`move.hasBounced`).
    /// The call comes from a Z status move (Z-Nature Power, Z-Metronome): Showdown's
    /// `useMoveInner` makes a damaging called move its Z-Move (R-M9-41).
    pub(crate) fn with_z_caller(mut self, z_call: bool) -> Self {
        self.z_call = z_call;
        self
    }

    pub(crate) fn reflected(mut self, bounced: bool) -> Self {
        self.bounced = bounced;
        self
    }

    /// The move runs straight through `useMove`: no BeforeMove (§15.11).
    pub(crate) fn without_before_move(mut self, skip: bool) -> Self {
        self.skips_before_move = skip;
        self
    }

    /// Gen 2 Fury Cutter (§15.18) and Rage (§15.19): what their damage is multiplied by —
    /// Fury Cutter doubles once less than its count (at most 5), Rage multiplies by its user's
    /// rage count + 1.
    fn gen2_damage_multiplier(
        &self,
        actor: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) -> Option<(crate::ids::MoveId, u32)> {
        let user = self.state.pokemon(actor)?;
        if move_def.behavior.power_formula == Some(crate::effects::moves::PowerFormula::FuryCutter)
            && matches!(
                self.rules.conditions.fury_cutter.model,
                crate::rules::FuryCutterModel::Count {
                    damage_stage: true,
                    ..
                }
            )
        {
            let count = user.volatiles.fury_cutter().map_or(0, |(count, _)| count);
            return Some((move_def.id, 1 << (count.clamp(1, 5) - 1)));
        }
        if move_def.behavior.rage
            && self.rules.conditions.rage.build == crate::rules::RageBuild::Counter
        {
            return Some((
                move_def.id,
                u32::from(user.volatiles.rage().unwrap_or(0)) + 1,
            ));
        }
        None
    }

    /// Gen 2-4 Beat Up's stats for the current hit: the member's base Attack and level and the
    /// target's base Defense (`DamageRules::beat_up`); `None` for any other move or Gen 5+.
    fn beat_up_stats(
        &self,
        target: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) -> Option<(u32, u8, u32)> {
        if !move_def.behavior.beat_up
            || self.rules.damage.beat_up == crate::rules::BeatUpStyle::Modern
        {
            return None;
        }
        // Gen 2 reads the species they battle as (`species.baseStats`), Gen 3-4 their own
        // (`set.species`).
        let battling = self.rules.damage.beat_up == crate::rules::BeatUpStyle::Gen2;
        let base = |pokemon: PokemonId| {
            let pokemon = self.state.pokemon(pokemon)?;
            let species = if battling {
                crate::engine::species_of(pokemon)
            } else {
                pokemon.species
            };
            let species = self.dex.species_by_id(species?)?;
            Some((species.base_stats, pokemon.level))
        };
        let (member_stats, level) = base(self.hit_member()?)?;
        let (target_stats, _) = base(target)?;
        Some((
            u32::from(member_stats.attack),
            level,
            u32::from(target_stats.defense),
        ))
    }

    /// The party member the current hit is for (Beat Up).
    fn hit_member(&self) -> Option<PokemonId> {
        self.beat_up_members
            .get(usize::from(self.hit.saturating_sub(1)))
            .copied()
    }

    /// The move runs as an action another move moved up the queue (Round).
    pub(crate) fn with_prioritized_by(mut self, move_id: Option<crate::ids::MoveId>) -> Self {
        self.prioritized_by = move_id;
        self
    }

    /// The action's `originalTarget` (Showdown `getTarget`'s target tracking).
    pub(crate) fn with_original_target(mut self, target: Option<PokemonId>) -> Self {
        self.original_target = target;
        self
    }

    /// The move whose next queued use the executed move moved to the front (Round's `onTry`).
    pub(crate) fn prioritizes(&self) -> Option<crate::ids::MoveId> {
        self.prioritizes
    }

    /// The Pokémon whose Shell Trap this move armed, in the order it did.
    pub(crate) fn prioritized_actors(&self) -> &[PokemonId] {
        &self.prioritized_actors
    }

    /// The move runs as a queued action ordered with this priority (Showdown writes it back to
    /// the move from Gen 6, `action.move.priority`) and Prankster flag.
    pub(crate) fn with_action_priority(mut self, priority: i8, prankster_boosted: bool) -> Self {
        self.action_priority = Some(priority);
        self.prankster_boosted = prankster_boosted;
        self
    }

    /// The move is an external one (`MovePipeline::external`).
    pub(crate) fn with_external(mut self) -> Self {
        self.external = true;
        self
    }

    /// A nested move's priority and Prankster boost, lent by the move it runs inside
    /// (`MovePipeline::lent_priority`).
    pub(crate) fn with_lent_priority(self, lent: Option<(i8, bool)>) -> Self {
        match lent {
            Some((priority, prankster_boosted)) => {
                self.with_action_priority(priority, prankster_boosted)
            }
            None => self,
        }
    }

    /// The action runs this Z-Move instead of its slot's move (Showdown `runMove`'s `zMove`
    /// option, DESIGN.md §44.4).
    pub(crate) fn with_z_move(mut self, z_move: ZMoveExecution) -> Self {
        self.z_powered = z_move.powered;
        self.z_move = Some(z_move);
        self
    }

    /// The action runs this Max Move instead of its slot's move (Showdown `runMove`'s `maxMove`
    /// option, DESIGN.md §44.5).
    pub(crate) fn with_max_move(mut self, max_move: crate::dex::MaxMove) -> Self {
        self.max_base = Some(max_move.base);
        self.max_move = Some(max_move);
        self.z_powered = true;
        self
    }

    /// Whether the move under way is Z-powered (Showdown `move.isZOrMaxPowered`).
    pub(crate) const fn z_powered(&self) -> bool {
        self.z_powered
    }
}
