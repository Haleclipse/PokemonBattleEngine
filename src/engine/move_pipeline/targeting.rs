//! Execution-time targeting (DESIGN.md §15.4): retargeting when the chosen target is gone,
//! redirection (Follow Me, Lightning Rod), and the partners on the field.

use crate::dex::ResolvedMove;
use crate::ids::{MoveId, PokemonId};
use crate::rules::{DeclaredTarget, ResolvedRules, ResolvedTarget};
use crate::state::{BattleState, MoveResult, RandomRequestKind, VolatileConditionKind};

use crate::engine::{EffectSource, MoveFailReason, TraceEvent};

use super::{MoveExecution, MovePipeline};

impl<'a> MovePipeline<'a> {
    /// Showdown `getTarget` → `getRandomTarget` when the chosen target is no longer there: a
    /// single-target move aimed at a foe picks another adjacent foe (a `RandomTarget`
    /// decision when several qualify); aimed at a fainted ally it just fails — Acupressure
    /// but in Gen 5 goes to its user.
    pub(super) fn fallback_target(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        declared: Option<DeclaredTarget>,
        error: crate::rules::TargetError,
    ) -> Option<crate::rules::ResolvedTargets> {
        let candidates =
            retarget_candidates(self.state, self.rules, actor, move_def, declared, &error)?;
        let index = if candidates.len() > 1 {
            usize::from(self.state.rng.range_inclusive_for(
                RandomRequestKind::RandomTarget,
                0,
                (candidates.len() - 1) as u16,
            ))
        } else {
            0
        };
        let to = candidates[index];
        self.trace.push(TraceEvent::TargetRedirected {
            actor,
            move_id: move_def.id,
            from: declared.map(|d| d.0),
            to,
            by: EffectSource::Rule,
        });
        Some(crate::rules::ResolvedTargets {
            targets: vec![ResolvedTarget::Pokemon(to)],
        })
    }

    /// Showdown `RedirectTarget` (`deciding_redirector`): the move goes to the redirector that
    /// decides. Returns whether that redirector keeps Dragon Darts on its holder alone
    /// (`redirector_cancels_smart_target`).
    pub(super) fn redirect_target(
        &mut self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        declared: Option<DeclaredTarget>,
        targets: &mut crate::rules::ResolvedTargets,
    ) -> bool {
        let current = match targets.targets.as_slice() {
            [ResolvedTarget::Pokemon(current)] => Some(*current),
            [] => None,
            _ => return false,
        };
        let Some((holder, by)) =
            deciding_redirector(self.state, self.rules, self.dex, actor, move_def, current)
        else {
            return false;
        };
        if Some(holder) != current {
            if let EffectSource::Ability { ability, .. } = by {
                self.trace.push(TraceEvent::AbilityActivated {
                    source: holder,
                    ability,
                });
            }
            self.trace.push(TraceEvent::TargetRedirected {
                actor,
                move_id: move_def.id,
                from: declared.map(|d| d.0),
                to: holder,
                by,
            });
            targets.targets = vec![ResolvedTarget::Pokemon(holder)];
        }
        redirector_cancels_smart_target(by)
    }

    /// Showdown `useMoveInner`'s first no-target failure (`if (!target)`): `getTarget` handed
    /// the move no target at all. Only a move for an adjacent ally (`adjacentAlly`: Helping
    /// Hand, Aromatic Mist, Hold Hands) meets it: aimed at its user's own position (an Ally
    /// Switch moved it there), or with no partner chosen — none in a single battle, none for a
    /// caller or an Encore — and none standing beside it for `getRandomTarget` to draw (what
    /// `resolve` drew, `retarget_candidates`). A chosen partner keeps the move, fainted or not
    /// ("attack shouldn't retarget"); that one fails after TryMove, finding no target standing
    /// (R-M10-18).
    pub(super) fn aims_at_nothing(
        &self,
        actor: PokemonId,
        move_def: &ResolvedMove,
        declared: Option<DeclaredTarget>,
        resolved: Option<&crate::rules::ResolvedTargets>,
    ) -> bool {
        if move_def.target != crate::rules::TargetPattern::Ally {
            return false;
        }
        let topology = &self.rules.topology;
        let Some(position) = self.state.active_position(actor, topology) else {
            return false;
        };
        match declared {
            Some(DeclaredTarget(chosen)) if chosen == position => true,
            Some(DeclaredTarget(chosen))
                if topology.adjacent_allies(position).contains(&chosen) =>
            {
                false
            }
            _ => resolved.is_none_or(|targets| targets.targets.is_empty()),
        }
    }

    pub(super) fn fail_no_target(&mut self, actor: PokemonId, move_id: MoveId) -> MoveExecution {
        self.trace.push(TraceEvent::MoveFailed {
            actor,
            move_id,
            reason: MoveFailReason::NoLegalTarget,
        });
        MoveExecution {
            actor,
            move_id,
            hits: Vec::new(),
            stopped: false,
            aborted: false,
            result: MoveResult::Failed,
        }
    }

    /// The Pokémon's partners on the field (doubles and up), standing.
    pub(super) fn active_allies(&self, pokemon: PokemonId) -> Vec<PokemonId> {
        let Some(side) = self.state.pokemon(pokemon).map(|p| p.side) else {
            return Vec::new();
        };
        self.state
            .side(side)
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|ally| *ally != pokemon)
            .filter(|ally| self.state.pokemon(*ally).is_some_and(|p| !p.is_fainted()))
            .collect()
    }
}

/// Showdown `RedirectTarget` (doubles only, single-target moves aimed at someone else): the
/// redirectors in handler order — Spotlight (priority 2), Follow Me / Rage Powder on the foe's
/// side (1), then a type-absorbing ability (0), each by speed — and the first one that names a
/// target decides (a priority event's fast exit), even when it names the target already chosen.
/// `current` is the move's target (none: Showdown aims it at a fainted slot, and any redirector
/// takes it). Shared with the replay.
pub fn deciding_redirector(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &crate::dex::ResolvedDex,
    actor: PokemonId,
    move_def: &ResolvedMove,
    current: Option<PokemonId>,
) -> Option<(PokemonId, EffectSource)> {
    if rules.topology.active_slots_per_side() < 2
        || !is_single_target(move_def.target)
        || move_def.tracks_target
    {
        return None;
    }
    if current == Some(actor) {
        return None;
    }
    let actor_position = state.active_position(actor, &rules.topology)?;
    let actor_side = rules.topology.spec(actor_position).map(|spec| spec.side);
    // Showdown's `validTargetLoc`: a `randomNormal` or `scripted` move (Thrash, Counter,
    // Comeuppance) may be drawn to any adjacent Pokémon — its target is drawn, not declared
    // (R-M8-135).
    let pattern = match move_def.target {
        crate::rules::TargetPattern::RandomAdjacentFoe => {
            crate::rules::TargetPattern::SelectedAdjacent
        }
        pattern => pattern,
    };
    // Lightning Rod / Storm Drain read an `adjacentFoe` move as `normal` as well (their
    // `redirectTarget`): an ally of the user may draw a Max Move (DESIGN.md R-M9-24). The centers
    // of attention keep the move's own target.
    let pattern_for = |by: &EffectSource| match (by, pattern) {
        (EffectSource::Ability { .. }, crate::rules::TargetPattern::SelectedAdjacentFoe) => {
            crate::rules::TargetPattern::SelectedAdjacent
        }
        _ => pattern,
    };
    let eligible = |state: &BattleState,
                    rules: &ResolvedRules,
                    holder: PokemonId,
                    pattern: crate::rules::TargetPattern| {
        state
            .active_position(holder, &rules.topology)
            .is_some_and(|position| {
                rules
                    .targeting
                    .resolve(
                        &rules.topology,
                        state,
                        actor,
                        pattern,
                        Some(DeclaredTarget(position)),
                    )
                    .is_ok()
            })
    };

    // Showdown `priorityEvent('RedirectTarget')`: every redirector in handler order —
    // priority first (Spotlight 2, Follow Me / Rage Powder 1, Lightning Rod / Storm Drain 0),
    // then speed —
    // and the first that names a target decides (fast exit), even when it names the target
    // already chosen: a second, slower Follow Me never takes a move aimed at the first.
    let powder_immune = crate::engine::powder_immunity(state, rules, actor).is_some();
    let foes_only = rules.conditions.redirect_ability_foes_only;
    let redirected_type = if rules.conditions.redirect_reads_dex_type {
        dex.move_by_id(move_def.id)
            .map_or(move_def.move_type, |dex_move| dex_move.move_type)
    } else {
        move_def.move_type
    };
    let mut redirectors: Vec<(i8, u32, PokemonId, EffectSource)> = Vec::new();
    // Sky Drop on the user (its condition's `onRedirectTarget`, priority 99): the Pokémon
    // it carries, wherever that stands now — an Ally Switch moved it (DESIGN.md R-M8-81).
    if let Some(carried) = state
        .pokemon(actor)
        .and_then(|user| user.volatiles.sky_drop_target())
        .filter(|carried| state.pokemon(*carried).is_some_and(|p| !p.is_fainted()))
    {
        redirectors.push((
            99,
            crate::engine::stats::handler_speed(state, actor),
            carried,
            EffectSource::Volatile(VolatileConditionKind::SkyDrop),
        ));
    }
    for pokemon in state.all_pokemon().iter().filter(|p| !p.is_fainted()) {
        let foe = Some(pokemon.side) != actor_side;
        // Handlers are ordered by `pokemon.speed` (Trick Room turns it around), refreshed as
        // the turn started — until Gen 8, a Speed drop this turn does not move them.
        let speed = crate::engine::stats::handler_speed(state, pokemon.id());
        // Follow Me / Rage Powder on the user's foes (a powder-immune user ignores Rage
        // Powder).
        let follow = if pokemon.volatiles.contains(VolatileConditionKind::FollowMe) {
            Some(VolatileConditionKind::FollowMe)
        } else if pokemon
            .volatiles
            .contains(VolatileConditionKind::RagePowder)
            && !powder_immune
        {
            Some(VolatileConditionKind::RagePowder)
        } else {
            None
        };
        // Gen 3: the Follow Me slot draws the moves to whoever stands in it now.
        let side = state.side(pokemon.side);
        let on_follow_me_slot = side
            .follow_me_slots
            .iter()
            .any(|slot| side.active.get(*slot) == Some(&Some(pokemon.id())));
        let follow = follow.or(on_follow_me_slot.then_some(VolatileConditionKind::FollowMe));
        // Follow Me / Rage Powder draw nothing to a Pokémon Sky Drop holds in the air
        // (`isSkyDropped`).
        let follow = follow.filter(|_| !crate::engine::context::sky_dropped(state, pokemon.id()));
        if foe && let Some(kind) = follow {
            redirectors.push((1, speed, pokemon.id(), EffectSource::Volatile(kind)));
        }
        // Spotlight on the user's foes (`onFoeRedirectTargetPriority` 2: ahead of Follow Me).
        if foe && pokemon.volatiles.contains(VolatileConditionKind::Spotlight) {
            redirectors.push((
                2,
                speed,
                pokemon.id(),
                EffectSource::Volatile(VolatileConditionKind::Spotlight),
            ));
        }
        // Lightning Rod / Storm Drain anywhere on the field (Gen 3: the foes'). They draw no
        // Pledge (`onAnyRedirectTarget`: `move.flags['pledgecombo']`, R-M8-154).
        if pokemon.id() != actor
            && (foe || !foes_only)
            && !move_def.behavior.pledge_combo
            && crate::engine::ability_handler(state, rules, pokemon, |b| b.redirect_type)
                == Some(redirected_type)
        {
            redirectors.push((
                0,
                speed,
                pokemon.id(),
                EffectSource::Ability {
                    pokemon: pokemon.id(),
                    ability: crate::engine::raw_ability(pokemon),
                },
            ));
        }
    }
    // Equal speeds (a Transform copies them): the holder whose ability state started first
    // (Showdown `compareRedirectOrder`'s `abilityState.effectOrder`, R-M8-182) — a Pokémon that
    // transformed into a Lightning Rod holder comes after the holder itself.
    let effect_order = |holder: PokemonId| {
        state
            .pokemon(holder)
            .map_or(0, |pokemon| pokemon.ability_state.effect_order)
    };
    redirectors.sort_by_key(|(priority, speed, holder, _)| {
        (
            std::cmp::Reverse(*priority),
            std::cmp::Reverse(*speed),
            effect_order(*holder),
        )
    });
    redirectors
        .into_iter()
        .find(|(_, _, holder, by)| eligible(state, rules, *holder, pattern_for(by)))
        .map(|(_, _, holder, by)| (holder, by))
}

/// Whether the redirector that decided keeps Dragon Darts on its holder alone (`smartTarget`
/// off): the center of attention — Follow Me, Rage Powder (their `onFoeRedirectTarget`; Bulbapedia's
/// Dragon Darts: "If an opponent is the center of attention (such as due to Follow Me), both
/// strikes will target that Pokémon"), Spotlight by the same rule though no generation has both it
/// and Dragon Darts — and Lightning Rod / Storm Drain; not Sky Drop's carried Pokémon (R-M8-169).
pub fn redirector_cancels_smart_target(by: EffectSource) -> bool {
    matches!(
        by,
        EffectSource::Volatile(
            VolatileConditionKind::FollowMe
                | VolatileConditionKind::RagePowder
                | VolatileConditionKind::Spotlight
        ) | EffectSource::Ability { .. }
    )
}

/// Dragon Darts (`smartTarget`, Showdown `getSmartTargets`): the target and the partner beside
/// it, one dart each — just the target when that partner is missing, down or the user itself
/// (singles has none). The target is standing: execution-time targeting has already moved a
/// move off a fainted one (Showdown `getTarget`). Shared with the replay.
pub fn smart_targets(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: PokemonId,
    target: PokemonId,
) -> Vec<PokemonId> {
    let standing = |id: PokemonId| state.pokemon(id).is_some_and(|p| !p.is_fainted());
    let partner = state
        .active_position(target, &rules.topology)
        .and_then(|position| rules.topology.adjacent_allies(position).into_iter().next())
        .and_then(|position| state.pokemon_at_position(position, &rules.topology))
        .filter(|partner| *partner != user && standing(*partner));
    match partner {
        Some(partner) => vec![target, partner],
        None => vec![target],
    }
}

/// The adjacent foes a single-target move picks from when its declared target is gone, or it
/// has none (a move overridden by Encore in a double battle): Showdown `getRandomTarget`, in the
/// engine's order; the engine draws a `RandomTarget` decision when there are several. `None`
/// when the move is not retargeted (aimed at a fainted ally, it just fails). Shared with the
/// replay driver.
pub fn random_retarget_candidates(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: PokemonId,
    move_def: &ResolvedMove,
    declared: Option<DeclaredTarget>,
) -> Option<Vec<PokemonId>> {
    let error = rules
        .targeting
        .resolve(&rules.topology, state, actor, move_def.target, declared)
        .err()?;
    retarget_candidates(state, rules, actor, move_def, declared, &error)
}

/// Gen 9 Curse (`onModifyMove`, §15.16): a Ghost type's aimed at an ally goes to a random
/// adjacent foe, drawn as the move starts (`getRandomTarget` after ModifyMove) — the foes it may
/// go to, when that happens. Shared with the replay.
pub fn curse_redirect_candidates(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: PokemonId,
    move_def: &ResolvedMove,
    declared: Option<DeclaredTarget>,
) -> Option<Vec<PokemonId>> {
    if !rules.conditions.curse.ghost_ally_target_goes_to_a_foe
        || move_def.non_ghost_target.is_none()
    {
        return None;
    }
    let user = state.pokemon(actor)?;
    if !crate::engine::has_type(state, rules, user, crate::dex::PokemonType::Ghost) {
        return None;
    }
    let target = state.pokemon_at_position(declared?.0, &rules.topology)?;
    if target == actor || state.pokemon(target)?.side != user.side {
        return None;
    }
    let mut foe_bound = move_def.clone();
    foe_bound.target = crate::rules::TargetPattern::RandomAdjacentFoe;
    retarget_candidates(
        state,
        rules,
        actor,
        &foe_bound,
        None,
        &crate::rules::TargetError::TargetRequired,
    )
}

fn retarget_candidates(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    actor: PokemonId,
    move_def: &ResolvedMove,
    declared: Option<DeclaredTarget>,
    error: &crate::rules::TargetError,
) -> Option<Vec<PokemonId>> {
    if !matches!(
        error,
        crate::rules::TargetError::TargetUnavailable(_) | crate::rules::TargetError::TargetRequired
    ) {
        return None;
    }
    let actor_position = state.active_position(actor, &rules.topology)?;
    let actor_side = rules.topology.spec(actor_position)?.side;
    // Aimed at a fainted ally: the move fails, but for one aimed at "an ally or itself"
    // (Acupressure), which goes to its user (`ConditionRules::fainted_ally_retargets_self`).
    if let Some(DeclaredTarget(position)) = declared
        && rules.topology.spec(position)?.side == actor_side
    {
        return (move_def.target == crate::rules::TargetPattern::AllyOrSelf
            && rules.conditions.fainted_ally_retargets_self)
            .then(|| vec![actor]);
    }
    // Showdown `getRandomTarget` with no target chosen (an overriding Encore, a caller that
    // names none): a move for an ally or its user goes to its user (Metronome's Acupressure in
    // a double battle, R-M8-73); one that needs an ally samples the adjacent allies standing
    // (Helping Hand).
    match move_def.target {
        crate::rules::TargetPattern::AllyOrSelf if declared.is_none() => {
            return Some(vec![actor]);
        }
        crate::rules::TargetPattern::Ally if declared.is_none() => {
            let allies: Vec<PokemonId> = rules
                .topology
                .adjacent_allies(actor_position)
                .into_iter()
                .filter_map(|position| state.pokemon_at_position(position, &rules.topology))
                .filter(|id| state.pokemon(*id).is_some_and(|p| !p.is_fainted()))
                .collect();
            return (!allies.is_empty()).then_some(allies);
        }
        _ if !is_single_target(move_def.target) => return None,
        _ => {}
    }
    let candidates: Vec<PokemonId> = rules
        .topology
        .adjacent_foes(actor_position)
        .into_iter()
        .filter_map(|position| state.pokemon_at_position(position, &rules.topology))
        .filter(|id| state.pokemon(*id).is_some_and(|p| !p.is_fainted()))
        .collect();
    (!candidates.is_empty()).then_some(candidates)
}

pub(super) fn is_single_target(pattern: crate::rules::TargetPattern) -> bool {
    use crate::rules::TargetPattern;
    matches!(
        pattern,
        TargetPattern::SelectedAny
            | TargetPattern::SelectedAdjacent
            | TargetPattern::SelectedFoe
            | TargetPattern::SelectedAdjacentFoe
            | TargetPattern::RandomAdjacentFoe
    )
}
