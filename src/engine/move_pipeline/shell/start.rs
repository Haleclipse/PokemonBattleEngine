//! `run_move`'s first stage (DESIGN.md §48.3): the move as it starts — Showdown `ModifyType` /
//! `ModifyMove`, `activeMove`, its targets anew, its drawn power.

use super::*;

impl MovePipeline<'_> {
    /// The move as it starts: the user's forme and the move's type, category and target as
    /// `ModifyType` / `ModifyMove` leave them, the move made active, its targets anew where it
    /// changed whom it aims at, its random power drawn.
    pub(super) fn start_move(&mut self, input: MoveRunInput) -> MoveRun {
        let MoveRunInput {
            actor,
            slot,
            move_id,
            move_def,
            declared_target,
            action_target,
            overridden,
            resolved,
            struggling,
        } = input;
        if self.outermost.is_none() {
            self.outermost = Some(move_id);
        }
        // Gen 1: the binding lock goes on only against the Pokémon it caught (§30.9) — and it
        // is the action's business, not a called move's.
        let continuing_bind = self.call.is_none()
            && self.binding_continues(
                actor,
                resolved.as_ref().and_then(|t| t.pokemon_ids().next()),
            );

        // A frozen user of a `defrost` move thaws as the move starts (Showdown `frz`
        // `onModifyMove`, Gen 3+); Gen 2 waits until the move has landed.
        let defrost_after = self.defrost_user(actor, &move_def);
        // Shell Side Arm (its own `onModifyMove`, before the ability's: Long Reach then takes the
        // contact away): physical when its estimates against the target say so; a tie flips a
        // coin.
        let physical = move_def.behavior.category_by_estimate
            && resolved
                .as_ref()
                .and_then(|t| t.pokemon_ids().next())
                .is_some_and(|target| {
                    crate::engine::physical_by_estimate(
                        self.state, self.rules, actor, target, &move_def,
                    )
                    .unwrap_or_else(|| {
                        self.state
                            .rng
                            .roll_ratio_for(RandomRequestKind::CategoryTie, 1, 2)
                    })
                });
        // Gen 7+ Stance Change (`onModifyMove`, priority 1): the forme for the move, before the
        // handlers that edit it (DESIGN.md §43.4).
        if let Some(stance) = self.state.pokemon(actor).and_then(|pokemon| {
            crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.move_stance)
        }) {
            BattleContext::new(self.state, self.rules, self.trace).stance_change(
                actor,
                stance,
                move_def.id,
                move_def.category,
            );
        }
        // A combined Pledge (the move's own `onModifyMove` with another Pledge as its
        // `sourceEffect`, DESIGN.md §42.9): the pair's type and `forceSTAB` — before the user's
        // ability retypes it and before TryMove, where a strong weather judges the combined type
        // (R-M8-153). Its power and side condition come at PrepareHit, below.
        let mut move_def = move_def;
        if move_def.behavior.pledge_combo
            && let Some(by) = self.prioritized_by
            && let Some(combined) = crate::engine::pledges::combo(by, move_id)
        {
            move_def.move_type = combined.move_type;
            move_def.flags.force_stab = true;
        }
        // Showdown `ModifyType` / `ModifyMove` as the move starts: its own type and category,
        // then the user's ability retyping and editing it.
        let declared_pattern = move_def.target;
        let (mut move_def, type_changer) =
            crate::engine::modified_move(self.state, self.rules, actor, &move_def, physical);
        self.type_changer = type_changer;
        // Gen 9 Curse (`onModifyMove`): a Ghost type's aimed at an ally goes to a random adjacent
        // foe, drawn there (`getRandomTarget` after ModifyMove, §15.16).
        let resolved = match crate::engine::curse_redirect_candidates(
            self.state,
            self.rules,
            actor,
            &move_def,
            declared_target,
        ) {
            Some(candidates) if !candidates.is_empty() => {
                move_def.target = crate::rules::TargetPattern::RandomAdjacentFoe;
                let index = if candidates.len() > 1 {
                    usize::from(self.state.rng.range_inclusive_for(
                        RandomRequestKind::RandomTarget,
                        0,
                        (candidates.len() - 1) as u16,
                    ))
                } else {
                    0
                };
                Some(crate::rules::ResolvedTargets {
                    targets: vec![ResolvedTarget::Pokemon(candidates[index])],
                })
            }
            _ => resolved,
        };
        // Showdown `activeMove` (§41.2): the move being used, whether it ignores abilities as
        // `ModifyMove` left it (Sunsteel Strike's own, Mold Breaker's). A move another one calls
        // takes its place; the action's end clears it.
        self.state.active_move = Some(crate::state::ActiveMove {
            user: actor,
            move_id: move_def.id,
            category: move_def.category,
            ignores_ability: move_def.ignores_ability,
            stellar_boosted: false,
            z_powered: self.z_powered,
            base_move: if move_def.is_max { self.max_base } else { None },
            prankster_boosted: self.prankster_boosted,
        });
        // A move that changed whom it aims at (Expanding Force: all adjacent foes) is targeted
        // anew (Showdown `getMoveTargets` after `ModifyMove`).
        let resolved = if move_def.target != declared_pattern {
            self.rules
                .targeting
                .resolve(
                    &self.rules.topology,
                    self.state,
                    actor,
                    move_def.target,
                    None,
                )
                .ok()
                .or(resolved)
        } else {
            resolved
        };
        // Beat Up (`onModifyMove`): its members are fixed as the move starts.
        self.beat_up_members = if move_def.behavior.beat_up {
            crate::engine::beat_up_members(self.state, self.rules, actor)
        } else {
            Vec::new()
        };
        // Showdown `onModifyMove` (Magnitude, Present): the power is drawn as the move starts.
        self.drawn = move_def.behavior.random_power.map(|random| {
            let (values, weights) = random.table();
            let drawn =
                self.state
                    .rng
                    .sample_weighted_for(RandomRequestKind::Sample, values, weights);
            random.effect(drawn)
        });

        // The attacking turn of a two-turn move is the locked action Showdown runs without
        // `deductPP`; the charging turn paid for both. It aims where the charge aimed (the
        // lock's target, `before_move`: Showdown `twoturnmove.targetLoc`).
        let releasing = self
            .state
            .pokemon(actor)
            .is_some_and(|p| p.volatiles.charging_move() == Some(move_id));
        self.releasing = releasing;
        MoveRun {
            actor,
            slot,
            move_id,
            move_def,
            declared_target,
            action_target,
            overridden,
            resolved,
            struggling,
            continuing_bind,
            defrost_after,
            releasing,
        }
    }
}
