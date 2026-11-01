//! Future Sight and Doom Desire (DESIGN.md §15.22): the use foresees an attack on the target's
//! slot (Showdown's `futuremove` slot condition, `SideState::future_attacks`), and two turns
//! later the residual strikes whoever stands there (`BattleEngine::run_future_attack`) — with
//! the move itself, computed then (Gen 5+), or with the damage computed at use (Gen 2-4,
//! `FutureAttackRules::damage_at_use`).

use crate::dex::PokemonType;
use crate::dex::{FixedDamage, ResolvedMove};
use crate::ids::{PokemonId, SideId};
use crate::rules::{DamageAlgorithm, DeclaredTarget, ForeseenRoll, Fraction, ResolvedTargets};
use crate::state::{FutureAttack, MoveResult, RandomRequestKind};

use super::{HitOutcome, MoveExecution, MovePipeline};
use crate::engine::{MoveFailReason, TraceEvent};

/// Residuals from the use to the strike: the use's turn, the next, then the strike's.
const FUTURE_ATTACK_TURNS: u8 = 3;

impl<'a> MovePipeline<'a> {
    /// The Pokémon a future move is aimed at and its slot: its target's — or, with none
    /// standing, the fainted Pokémon Showdown aims it at (`getMoveTargets` keeps a fainted
    /// target for a `futuremove`, DESIGN.md R277): the fainted ally it was aimed at, else
    /// `getRandomTarget`'s foe in the first slot (`foe.active[0]`, singles and doubles).
    pub(super) fn foreseen_spot(
        &self,
        actor: PokemonId,
        declared: Option<DeclaredTarget>,
        targets: &ResolvedTargets,
    ) -> Option<(PokemonId, SideId, usize)> {
        let topology = &self.rules.topology;
        if let Some(target) = targets.pokemon_ids().next() {
            let spec = topology.spec(self.state.active_position(target, topology)?)?;
            return Some((target, spec.side, usize::from(spec.slot)));
        }
        let user = topology.spec(self.state.active_position(actor, topology)?)?;
        let (side, slot) = match declared.and_then(|declared| topology.spec(declared.0)) {
            Some(ally) if ally.side == user.side => (ally.side, usize::from(ally.slot)),
            _ => (user.side.other(), 0),
        };
        // Showdown's `side.active[slot]`: whoever stands there, else the fainted Pokémon left.
        let side_state = self.state.side(side);
        let aimed = side_state.active.get(slot).copied().flatten();
        Some((
            aimed.or(side_state.fallen.get(slot).copied().flatten())?,
            side,
            slot,
        ))
    }

    /// The move's `onTry` (after targeting, before any hit step): an attack waiting on the
    /// target's slot already makes it fail; otherwise it is foreseen — Gen 2-4 with its damage
    /// computed now, typeless and never critical (Gen 2-3 on the cartridge without the random
    /// factor, which the strike draws) — and the move stops there.
    pub(super) fn foresee(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        spot: Option<(PokemonId, SideId, usize)>,
    ) -> MoveExecution {
        let rules = self.rules.conditions.future_attack;
        let execution = |result| MoveExecution {
            actor,
            move_id: move_def.id,
            hits: Vec::new(),
            stopped: false,
            aborted: false,
            result,
        };
        let Some((target, side, slot)) = spot.filter(|(_, side, slot)| {
            !self
                .state
                .side(*side)
                .future_attacks
                .iter()
                .any(|attack| attack.slot == *slot)
        }) else {
            self.trace.push(TraceEvent::MoveFailed {
                actor,
                move_id: move_def.id,
                reason: MoveFailReason::NoEffect,
            });
            return execution(MoveResult::Failed);
        };
        let damage = rules.damage_at_use.map(|roll| {
            let mut at_use = move_def.clone();
            at_use.move_type = PokemonType::Typeless;
            at_use.will_crit = Some(false);
            at_use.no_damage_variance = roll != ForeseenRoll::AtUse;
            self.resolve_standard_damage(actor, target, &at_use, 1, Fraction::ONE)
                .final_damage
                .0
        });
        self.state.side_mut(side).future_attacks.push(FutureAttack {
            slot,
            source: actor,
            move_id: move_def.id,
            damage,
            turns_remaining: FUTURE_ATTACK_TURNS,
        });
        self.trace.push(TraceEvent::FutureAttackForeseen {
            actor,
            target,
            move_id: move_def.id,
        });
        // Gen 6+ `NOT_FAIL`; Gen 2-5 `null`, which `trySpreadMoveHit` reports as a failure.
        execution(if rules.use_succeeds {
            MoveResult::Succeeded
        } else {
            MoveResult::Failed
        })
    }

    /// Gen 2-3 on the cartridge (`ForeseenRoll`): the damage computed at use, with the random
    /// factor drawn as it strikes — pokecrystal's `damagevariation` (none for a damage
    /// of 1), pokeemerald's `ApplyRandomDmgMultiplier` (at least 1).
    pub(super) fn foreseen_damage(&mut self, base: u16) -> u16 {
        let damage = self.rules.damage;
        let legacy = matches!(
            damage.algorithm,
            DamageAlgorithm::Gen1Legacy | DamageAlgorithm::Gen2
        );
        if base == 0 || (legacy && base <= 1) {
            return base;
        }
        let roll = self.state.rng.range_inclusive_for(
            RandomRequestKind::DamageRoll,
            damage.random_min,
            damage.random_max,
        );
        let scaled =
            u32::from(base) * u32::from(roll) / u32::from(damage.random_denominator.max(1));
        u16::try_from(scaled).unwrap_or(u16::MAX).max(1)
    }

    /// The foreseen attack strikes `target` (the condition's `onEnd`, `trySpreadMoveHit` for a
    /// user that need not be on the field): the hit steps and the damage of `strike`, as any
    /// move's. Every generation's strike runs the main `hitStepMoveHitLoop` (no generation's mod
    /// replaces it), which ends with an `Update` once the faints are processed: a Gen 2-4
    /// infatuation whose source the strike knocked out ends there (Gen 5+ runs one at the end
    /// of the action as well). Its `Update` after the hit, the faints pending, shows nowhere a
    /// generation's own move `Update`s would not.
    pub(crate) fn strike_future_attack(
        &mut self,
        attack: FutureAttack,
        target: PokemonId,
    ) -> Vec<HitOutcome> {
        // Gen 2 on the cartridge: the random factor before the strike's accuracy check.
        let mut attack = attack;
        if self.rules.conditions.future_attack.damage_at_use == Some(ForeseenRoll::BeforeAccuracy) {
            attack.damage = attack.damage.map(|base| self.foreseen_damage(base));
        }
        let Some(mut strike) = future_strike(self.rules, self.dex, &attack) else {
            return Vec::new();
        };
        // Gen 6+ (the condition's `onEnd`): a user on the field with Normalize as it strikes
        // makes the strike Normal — the use's own retyping is not kept (R-M8-151).
        if self
            .rules
            .conditions
            .future_attack
            .source_normalize_at_strike
            && self
                .state
                .active_position(attack.source, &self.rules.topology)
                .is_some()
            && self.state.pokemon(attack.source).is_some_and(|pokemon| {
                crate::engine::has_ability(
                    self.state,
                    self.rules,
                    pokemon,
                    crate::ids::AbilityId::NORMALIZE,
                )
            })
        {
            strike.move_type = PokemonType::Normal;
        }
        // Gen 6+ (the same `onEnd`): a user with Infiltrator in effect as it strikes — on the
        // field: off it, its ability is not (`hasAbility`) — makes the strike go past a Substitute
        // (`moveData.infiltrates`), as its own moves do (R-M10-81).
        if self.rules.conditions.substitute_honors_infiltrates
            && self.state.pokemon(attack.source).is_some_and(|pokemon| {
                crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.infiltrates)
            })
        {
            strike.flags.bypass_substitute = true;
        }
        self.hp_before_hits = self
            .state
            .all_pokemon()
            .iter()
            .map(|pokemon| (pokemon.id(), pokemon.hp))
            .collect();
        let source = attack.source;
        let outcomes = match self.resolve_hit_gate(source, target, &strike) {
            Ok(type_effectiveness) => {
                self.apply_damage_sequence(source, target, &strike, 1, type_effectiveness)
            }
            Err(outcome) => vec![outcome],
        };
        self.record_attack(source, target, &strike, &outcomes);
        let mut ctx = crate::engine::BattleContext::new(self.state, self.rules, self.trace);
        ctx.announce_faints();
        // `hitStepMoveHitLoop`'s `Update` once the faints are processed, when the strike did
        // something (`damage.some(val => !!val || val === 0)`).
        if outcomes.iter().any(|outcome| {
            matches!(
                outcome,
                HitOutcome::Damaged { .. }
                    | HitOutcome::SubstituteDamaged { .. }
                    | HitOutcome::Effect { .. }
            )
        }) {
            ctx.run_update(crate::engine::PendingFaints::Processed);
        }
        // `trySpreadMoveHit`'s close: the target's `AfterMoveSecondary` (Maranga Berry, Red Card,
        // Eject Button …) and its Emergency Exit (DESIGN.md R277).
        self.after_move_secondary(source, &strike, &outcomes);
        outcomes
    }
}

/// The move a foreseen attack strikes with (shared with the replay): Gen 5+ the move itself;
/// Gen 2-4 its damage computed at use, typeless (Showdown's `type: '???'`, `damage`) — Gen 3
/// drawing its random factor once the strike hit (`FixedDamage::Foreseen`; Gen 2 drew it before,
/// `strike_future_attack`).
pub fn future_strike(
    rules: &crate::rules::ResolvedRules,
    dex: &crate::dex::ResolvedDex,
    attack: &FutureAttack,
) -> Option<ResolvedMove> {
    let mut strike = dex.move_by_id(attack.move_id)?.clone();
    // The use ignores the type chart (Future Sight's `ignoreImmunity: true`), the strike does not
    // (its `moveData`: a Dark type is immune from Gen 5).
    strike.flags.type_immunity = true;
    if let Some(damage) = attack.damage {
        strike.move_type = PokemonType::Typeless;
        strike.fixed_damage = Some(
            if rules.conditions.future_attack.damage_at_use == Some(ForeseenRoll::AfterAccuracy) {
                FixedDamage::Foreseen(damage)
            } else {
                FixedDamage::Amount(damage)
            },
        );
    }
    Some(strike)
}
