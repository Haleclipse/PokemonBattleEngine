//! Thrash and its kind in the Move Shell (Showdown `lockedmove`, DESIGN.md §15.12): where the
//! lock starts and counts as a move runs, and how the move it ends with leaves it.

use crate::dex::ResolvedMove;
use crate::engine::{BattleContext, BeforeMoveBlockReason, HitBlockReason};
use crate::ids::PokemonId;
use crate::rules::RampageCounter;

use super::{HitOutcome, MoveExecution, MovePipeline};

impl<'a> MovePipeline<'a> {
    /// Showdown `selfDrops` on a hit (body or Substitute), before the hit's secondaries: Gen 3+
    /// starts the lock, or (Gen 5+) keeps it going (`RampageRules::counter`).
    pub(super) fn rampage_self_drop(&mut self, actor: PokemonId, move_def: &ResolvedMove) {
        if move_def.behavior.rampage && !self.rules.conditions.rampage.counter.counts_uses() {
            BattleContext::new(self.state, self.rules, self.trace).rampage_hit(actor, move_def.id);
        }
    }

    /// Gen 1-2, before the hit test (Gen 1 `tryMoveHit`; Gen 2 the cartridge's
    /// `checkrampage` / `rampage`, which Showdown's gen2 `onMoveFail` stands in for on a miss):
    /// the lock starts, or counts a use down (the last one confusing its user).
    pub(super) fn counted_rampage_use(&mut self, actor: PokemonId, move_def: &ResolvedMove) {
        let counter = self.rules.conditions.rampage.counter;
        if move_def.behavior.rampage
            && (counter == RampageCounter::Gen1Uses
                || (counter == RampageCounter::Gen2Uses && move_def.behavior.rampage_lock_at_start))
        {
            BattleContext::new(self.state, self.rules, self.trace)
                .counted_rampage_use(actor, move_def.id);
        }
    }

    /// A move stopped by BeforeMove: Gen 1's full paralysis and confusion self-hit end the lock
    /// (`par` / `confusion` `onBeforeMove`), Gen 2's every stop (issues.md I109); Gen 3-4 mark the
    /// turn failed for the cartridge's reasons (`RampageRules::failing_blocks`).
    pub(super) fn rampage_blocked(&mut self, actor: PokemonId, reason: BeforeMoveBlockReason) {
        let rules = self.rules.conditions.rampage;
        match rules.counter {
            RampageCounter::Gen1Uses
                if matches!(
                    reason,
                    BeforeMoveBlockReason::Paralysis | BeforeMoveBlockReason::Confusion
                ) =>
            {
                BattleContext::new(self.state, self.rules, self.trace).end_rampage(actor, false);
            }
            // Gen 2 (pokecrystal `CantMove`): every stop ends it silently.
            RampageCounter::Gen2Uses => {
                BattleContext::new(self.state, self.rules, self.trace).end_rampage(actor, false);
            }
            RampageCounter::Cartridge if rules.failing_blocks.contains(&reason) => {
                if let Some(rampage) = self
                    .state
                    .pokemon_mut(actor)
                    .and_then(|p| p.volatiles.rampage_mut())
                {
                    rampage.failed = true;
                }
            }
            _ => {}
        }
    }

    /// Showdown `onAfterMove` (the move ran): Gen 1-2 let go of a lock whose last use is over;
    /// Gen 5+ ends one whose `duration` reads 1, confusing its user on its last turn. Gen 3-4
    /// (the cartridge): a type immunity fails the turn; Gen 3's Protect ends the lock at once.
    pub(super) fn rampage_after_move(&mut self, actor: PokemonId, execution: &MoveExecution) {
        if execution.stopped {
            return;
        }
        let rules = self.rules.conditions.rampage;
        let Some(rampage) = self
            .state
            .pokemon(actor)
            .and_then(|p| p.volatiles.rampage())
            .copied()
        else {
            return;
        };
        if rampage.move_id != execution.move_id {
            return;
        }
        let blocked = |wanted: HitBlockReason| {
            execution
                .hits
                .iter()
                .any(|hit| matches!(hit, HitOutcome::Blocked { reason, .. } if *reason == wanted))
        };
        // Gen 3's check is the target's Protect itself (`DEFENDER_IS_PROTECTED`, ahead of the
        // hit test): a move that missed a protected target ends the lock too.
        let protected = self.met_protect(execution);
        let mut ctx = BattleContext::new(self.state, self.rules, self.trace);
        match rules.counter {
            RampageCounter::Gen1Uses | RampageCounter::Gen2Uses => {
                if rampage.turns == 0 {
                    ctx.end_rampage(actor, false);
                }
            }
            RampageCounter::HiddenLength => {
                if rampage.turns == 1 {
                    ctx.end_rampage(actor, rampage.left <= 1);
                }
            }
            RampageCounter::Cartridge => {
                if rules.protect_cancels && protected {
                    ctx.end_rampage(actor, false);
                } else if (blocked(HitBlockReason::TypeImmunity) || self.type_immune_target_guarded)
                    && let Some(rampage) = self
                        .state
                        .pokemon_mut(actor)
                        .and_then(|p| p.volatiles.rampage_mut())
                {
                    rampage.failed = true;
                }
            }
        }
    }
}
