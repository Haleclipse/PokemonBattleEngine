use crate::dex::{MoveCategory, PrimaryEffect, ResolvedDex, ResolvedMove};
use crate::ids::{AbilityId, ItemId, MoveId, MoveSlot, PokemonId, PositionId, SideId};
use crate::rules::{DeclaredTarget, ResolvedRules, TargetError};
use crate::state::{
    BattleState, FieldConditionKind, MechanicUsage, PokemonState, VolatileConditionKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MechanicChoice {
    MegaEvolve,
    /// Ultra Burst (Q1: a mechanic of its own, counted apart from Mega Evolution).
    UltraBurst,
    ZMove,
    Dynamax,
    Terastallize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionChoice {
    Move {
        slot: MoveSlot,
        target: Option<DeclaredTarget>,
        mechanic: Option<MechanicChoice>,
    },
    Switch {
        pokemon: PokemonId,
    },
    Pass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotChoice {
    pub actor: PokemonId,
    pub action: ActionChoice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideChoice {
    pub side: SideId,
    pub actions: Vec<SlotChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForcedReplacement {
    pub position: PositionId,
    pub pokemon: PokemonId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForcedSwitchChoice {
    pub side: SideId,
    pub replacements: Vec<ForcedReplacement>,
}

/// The answer to `BattleRequest::Revive`: the fainted Pokémon that comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviveChoice {
    pub side: SideId,
    pub pokemon: PokemonId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum ChoiceError {
    WrongSide {
        expected: SideId,
        actual: SideId,
    },
    WrongActionCount {
        expected: usize,
        actual: usize,
    },
    ActorIsNotActive(PokemonId),
    DuplicateActor(PokemonId),
    InvalidMoveSlot {
        actor: PokemonId,
        slot: MoveSlot,
    },
    NoPp {
        actor: PokemonId,
        move_id: MoveId,
    },
    UnknownMove(MoveId),
    /// The move is disabled for this Pokémon right now (Showdown's request-side lock):
    /// Taunt (status moves), Encore (anything but the encored move), Disable (the sealed
    /// move), Heal Block (moves that heal), Throat Chop (sound moves), a Choice item or Gorilla
    /// Tactics (anything but the locked move: `ChoiceLock`), Torment (the last move), a foe's
    /// Imprison (the moves it knows).
    MoveDisabled {
        actor: PokemonId,
        move_id: MoveId,
        by: VolatileConditionKind,
    },
    /// Showdown `cantusetwice` (Gigaton Hammer, Blood Moon): the move its user used last is off
    /// the request.
    MoveUsedTwice {
        actor: PokemonId,
        move_id: MoveId,
    },
    /// The move's own `onDisableMove` keeps it off the request: Belch before its user has eaten
    /// a berry, Stuff Cheeks while it holds none.
    MoveDisabledByItself {
        actor: PokemonId,
        move_id: MoveId,
    },
    /// Showdown `maxMoveDisabled`: a Max Move whose base move is out of PP, or a status move
    /// under an Assault Vest or Taunt (R-M10-5).
    MaxMoveDisabled {
        actor: PokemonId,
        move_id: MoveId,
    },
    /// The move is off the request because of the Pokémon's item (Assault Vest: status moves).
    /// Showdown: the item's `onDisableMove`.
    MoveDisabledByItem {
        actor: PokemonId,
        move_id: MoveId,
        item: ItemId,
    },
    /// The move is off the request because of the field (Gravity: the airborne moves).
    /// Showdown: the pseudo-weather's `onDisableMove`.
    MoveDisabledByField {
        actor: PokemonId,
        move_id: MoveId,
        by: FieldConditionKind,
    },
    /// The dex resolved the move as `PrimaryEffect::Unsupported`.
    UnsupportedMove(MoveId),
    /// A multi-turn move in progress (Fly, Dig, Solar Beam) removes the option to switch
    /// (Showdown `trapped` on a `lockedMove` request).
    Trapped {
        actor: PokemonId,
        by: VolatileConditionKind,
    },
    /// The Revive answer names a Pokémon that is not one of the request's fainted candidates.
    NotRevivable(PokemonId),
    /// A Pokémon's ability keeps the Pokémon from switching (Shadow Tag, Arena Trap, Magnet
    /// Pull). Showdown: the ability's `onFoeTrapPokemon` / `onAnyTrapPokemon`, a hidden trap
    /// (`tryTrap(true)`) its request shows as maybe (`Trapped::Maybe`): the error names neither
    /// the Pokémon nor its ability (`Can't switch: The active Pokémon is trapped`, DESIGN.md §47.0
    /// item 14).
    TrappedByAbility {
        actor: PokemonId,
    },
    /// The field keeps the Pokémon from switching (Fairy Lock). Showdown: the
    /// pseudo-weather's `onTrapPokemon`.
    TrappedByField {
        actor: PokemonId,
        by: FieldConditionKind,
    },
    InvalidTarget(TargetError),
    SwitchTargetNotOnTeam(PokemonId),
    SwitchTargetAlreadyActive(PokemonId),
    SwitchTargetFainted(PokemonId),
    DuplicateSwitchTarget(PokemonId),
    MechanicUnavailable(MechanicChoice),
    DuplicateMechanic(MechanicChoice),
    /// This Pokémon or this move cannot use the mechanic (Showdown `canTerastallize`
    /// returning null for a Pokémon that already Terastallized).
    MechanicNotApplicable {
        actor: PokemonId,
        mechanic: MechanicChoice,
    },
    WrongReplacementCount {
        expected: usize,
        actual: usize,
    },
    InvalidReplacementPosition(PositionId),
    DuplicateReplacementPosition(PositionId),
}

/// What the choice layer offers one Pokémon this turn (DESIGN.md §44.9, the user's Q3 answer):
/// a read-only view of the gimmicks available to it, mirroring what `validate_side_choice`
/// accepts. The fields are gated on the rules' `SpecialMechanics`, so a mechanic the rules or
/// the format do not allow is never offered — and neither is one the engine has no semantic
/// implementation for yet (the batch that implements it fills its field in).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MechanicOptions {
    /// Mega Evolution (O.3): the forme it would become.
    pub mega: Option<crate::ids::SpeciesId>,
    /// Ultra Burst (O.3, Q1: a mechanic of its own).
    pub ultra_burst: Option<crate::ids::SpeciesId>,
    /// Z-Moves (O.4): one entry per move slot, `None` where the slot has no Z-Move.
    pub z_moves: Vec<Option<ZMoveOption>>,
    /// Dynamax (O.5a).
    pub dynamax: bool,
    /// The Max Moves its slots would become (O.5a), once Dynamax is implemented.
    pub max_moves: Option<Vec<MaxMoveOption>>,
    /// Terastallization (Design §44.6): the Tera type it would take.
    pub tera: Option<crate::dex::PokemonType>,
}

/// One move slot's Z-Move (DESIGN.md §44.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZMoveOption {
    pub move_id: crate::ids::MoveId,
    pub target: crate::rules::TargetPattern,
}

/// One move slot's Max Move (DESIGN.md §44.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxMoveOption {
    pub move_id: crate::ids::MoveId,
    pub target: crate::rules::TargetPattern,
    pub disabled: bool,
}

impl MechanicOptions {
    /// Whether the Pokémon has no gimmick at all.
    pub fn is_empty(&self) -> bool {
        self.mega.is_none()
            && self.ultra_burst.is_none()
            && self.z_moves.iter().all(Option::is_none)
            && !self.dynamax
            && self.tera.is_none()
    }
}

/// The gimmicks `state` offers `pokemon` right now (Showdown `getMoveRequestData`'s
/// `canMegaEvo` / `canZMove` / `canDynamax` / `canTerastallize`, and the conditions its
/// `chooseMove` refuses them under): a Pokémon that is locked, recharging or struggling is
/// offered none, and a mechanic its side has spent is gone.
pub fn mechanic_options(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    pokemon: PokemonId,
) -> MechanicOptions {
    let mut options = MechanicOptions::default();
    let Some(poke) = state.pokemon(pokemon) else {
        return options;
    };
    let side = state.side(poke.side);
    let locked = poke.hard_lock().is_some()
        || poke.semi_locked_move().is_some()
        || recharging(state, pokemon)
        || must_struggle(state, rules, dex, pokemon);
    if locked {
        return options;
    }
    // Terastallization: the side has not spent it and the Pokémon has not taken a Tera type.
    if rules.special.tera
        && side.mechanics.tera == MechanicUsage::Available
        && poke.terastallized.is_none()
    {
        options.tera = Some(poke.tera_type);
    }
    // Z-Moves: one entry per move slot, `None` where the slot has no Z-Move (Showdown
    // `canZMove`'s `zMoves` array).
    if rules.special.z_move && side.mechanics.z_move == MechanicUsage::Available {
        options.z_moves = poke
            .move_slots()
            .iter()
            .map(|slot| {
                if slot.pp == 0 {
                    return None;
                }
                let base = dex.move_by_id(slot.move_id)?;
                let z = crate::engine::z_move_execution(state, rules, dex, pokemon, base)?;
                Some(ZMoveOption {
                    move_id: z.move_def.id,
                    target: z.move_def.target,
                })
            })
            .collect();
    }
    // Dynamax (DESIGN.md §44.5): its conditions, and the Max Moves the slots would become.
    if rules.special.dynamax
        && side.mechanics.dynamax == MechanicUsage::Available
        && crate::engine::can_dynamax(state, dex, pokemon)
    {
        // Showdown's `maxMoves` has one entry per move slot, in order; one the engine cannot
        // name (no Max Move) is left out, and Dynamax is offered only when at least one
        // remains.
        let max_moves = poke
            .move_slots()
            .iter()
            .filter_map(|slot| {
                let base = dex.move_by_id(slot.move_id)?;
                let modified =
                    crate::engine::modified_move_cow(state, rules, pokemon, base, false).0;
                let max = crate::engine::max_move_for(dex, &modified)?;
                Some(MaxMoveOption {
                    move_id: max,
                    target: dex
                        .move_by_id(max)
                        .map_or(crate::rules::TargetPattern::SelectedAdjacent, |def| {
                            def.target
                        }),
                    disabled: slot.pp == 0,
                })
            })
            .collect::<Vec<_>>();
        if !max_moves.is_empty() {
            options.dynamax = true;
            options.max_moves = Some(max_moves);
        }
    }
    // Mega Evolution (DESIGN.md §44.3): what the Pokémon may become, fixed as the battle was
    // built (`canMegaEvo` / `canUltraBurst`), and the side has not spent the mechanic.
    if rules.special.mega {
        if side.mechanics.mega == MechanicUsage::Available {
            options.mega = poke.mega_evolves_into;
        }
        if side.mechanics.ultra_burst == MechanicUsage::Available {
            options.ultra_burst = poke.ultra_bursts_into;
        }
    }
    options
}

/// What a side can choose at its turn's request (DESIGN.md §47.0 items 10-11), one entry per
/// Pokémon on the field, in position order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOptions {
    pub side: SideId,
    pub actors: Vec<ActorOptions>,
}

/// What one Pokémon on the field can choose (Showdown's request, `active[i]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorOptions {
    pub actor: PokemonId,
    pub position: PositionId,
    /// Every move slot, choosable or not.
    pub moves: Vec<MoveOption>,
    /// The team members it can switch to: none while it is trapped for all to see
    /// (`Trapped::Yes`); while it may be (`Trapped::Maybe`), the switch is tried as it is chosen.
    pub switches: Vec<PokemonId>,
    pub trapped: Trapped,
    pub mechanics: MechanicOptions,
    /// The one action its request holds, when it is held to one (recharging, struggling, a
    /// move it is locked into, Gen 1's "Fight" alone): choose it, whatever `moves` says.
    pub locked: Option<ActionChoice>,
}

/// Whether a Pokémon is trapped, as its player knows (Showdown's request `trapped` /
/// `maybeTrapped`, DESIGN.md §47.0 item 14): a trap for all to see (Mean Look, a partial trap,
/// a lock, Fairy Lock …); one a foe's ability may set — its own, or one its species may have
/// (`MaybeTrappedBy`), whether or not it does; or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum Trapped {
    No,
    Maybe,
    Yes,
}

/// One move slot (Showdown's request, `moves[i]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveOption {
    pub slot: MoveSlot,
    pub move_id: MoveId,
    pub pp: u8,
    pub max_pp: u8,
    pub target: crate::rules::TargetPattern,
    /// The declared targets `validate_choice` accepts for it (`None`: no target named), in
    /// position order; none when the move cannot be chosen.
    pub targets: Vec<Option<DeclaredTarget>>,
    /// Why it cannot be chosen, when it cannot.
    pub refused: Option<ChoiceError>,
}

/// What `side` can choose at a turn's request: built from `validate_side_choice`'s own checks
/// of one action (`validate_move_action`, `validate_switch_action`), so every move, target and
/// switch it lists is one `validate_choice` accepts, and none it leaves out is. What it does not
/// show are the side's own limits on its actions together: a mechanic of a kind once, a team
/// member switched in once. A `Pass` is accepted for any Pokémon.
pub fn choice_options(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    side: SideId,
) -> ChoiceOptions {
    // The targets a move may name: none, or any position on the field.
    let targets: Vec<Option<DeclaredTarget>> = std::iter::once(None)
        .chain(
            rules
                .topology
                .positions()
                .iter()
                .map(|position| Some(DeclaredTarget(position.id))),
        )
        .collect();
    let actors = rules
        .topology
        .positions()
        .iter()
        .filter(|position| position.side == side)
        .filter_map(|position| {
            let actor = state.pokemon_at_position(position.id, &rules.topology)?;
            let pokemon = state.pokemon(actor)?;
            let moves: Vec<MoveOption> = pokemon
                .move_slots()
                .iter()
                .enumerate()
                .filter_map(|(index, slot_state)| {
                    let slot = MoveSlot::new(u8::try_from(index).ok()?)?;
                    let outcomes: Vec<(Option<DeclaredTarget>, Result<(), ChoiceError>)> = targets
                        .iter()
                        .map(|target| {
                            let outcome = validate_move_action(
                                state, rules, dex, side, actor, slot, *target, None,
                            );
                            (*target, outcome)
                        })
                        .collect();
                    let accepted: Vec<Option<DeclaredTarget>> = outcomes
                        .iter()
                        .filter(|(_, outcome)| outcome.is_ok())
                        .map(|(target, _)| *target)
                        .collect();
                    // Why none is: the move's own refusal over a target's, when there is one.
                    let refused = accepted
                        .is_empty()
                        .then(|| {
                            let errors = || outcomes.iter().filter_map(|(_, o)| o.clone().err());
                            errors()
                                .find(|error| !matches!(error, ChoiceError::InvalidTarget(_)))
                                .or_else(|| errors().next())
                        })
                        .flatten();
                    let pattern = slot_state.target.unwrap_or_else(|| {
                        dex.move_by_id(slot_state.move_id)
                            .map_or(crate::rules::TargetPattern::SelectedAdjacent, |m| m.target)
                    });
                    Some(MoveOption {
                        slot,
                        move_id: slot_state.move_id,
                        pp: slot_state.pp,
                        max_pp: slot_state.max_pp,
                        target: pattern,
                        targets: accepted,
                        refused,
                    })
                })
                .collect();
            let trapped = if trapped_by(state, rules, actor).is_some()
                || field_traps(state, rules, actor).is_some()
            {
                Trapped::Yes
            } else if maybe_trapped(state, rules, dex, actor) {
                Trapped::Maybe
            } else {
                Trapped::No
            };
            // A trap an ability may set is the player's to find by switching.
            let switches: Vec<PokemonId> = state
                .side(side)
                .team
                .iter()
                .copied()
                .filter(|member| {
                    trapped != Trapped::Yes && switch_target_valid(state, side, *member).is_ok()
                })
                .collect();
            // Held to one action: the move it is locked into, else the first accepted.
            let held = recharging(state, actor)
                || must_struggle(state, rules, dex, actor)
                || gen1_fight_only(state, rules, actor)
                || pokemon.hard_lock().is_some()
                || pokemon.semi_locked_move().is_some();
            let locked = held
                .then(|| {
                    let lock = pokemon
                        .hard_lock()
                        .map(|lock| lock.move_id)
                        .or_else(|| pokemon.semi_locked_move());
                    let mut order: Vec<&MoveOption> = moves.iter().collect();
                    order.sort_by_key(|option| Some(option.move_id) != lock);
                    order.into_iter().find_map(|option| {
                        let target = option.targets.first().copied().or_else(|| {
                            // Recharging, struggling, "Fight" alone: any move is taken.
                            targets.iter().copied().find(|target| {
                                validate_move_action(
                                    state,
                                    rules,
                                    dex,
                                    side,
                                    actor,
                                    option.slot,
                                    *target,
                                    None,
                                )
                                .is_ok()
                            })
                        })?;
                        Some(ActionChoice::Move {
                            slot: option.slot,
                            target,
                            mechanic: None,
                        })
                    })
                })
                .flatten();
            Some(ActorOptions {
                actor,
                position: position.id,
                moves,
                switches,
                trapped,
                mechanics: mechanic_options(state, rules, dex, actor),
                locked,
            })
        })
        .collect();
    ChoiceOptions { side, actors }
}

pub fn validate_side_choice(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    expected_side: SideId,
    choice: &SideChoice,
) -> Result<(), ChoiceError> {
    if choice.side != expected_side {
        return Err(ChoiceError::WrongSide {
            expected: expected_side,
            actual: choice.side,
        });
    }

    let side = state.side(choice.side);
    let expected_actions = side.active.iter().flatten().count();
    if choice.actions.len() != expected_actions {
        return Err(ChoiceError::WrongActionCount {
            expected: expected_actions,
            actual: choice.actions.len(),
        });
    }

    let mut seen_actors = Vec::new();
    let mut seen_switch_targets = Vec::new();
    let mut seen_mechanics = Vec::new();

    for slot_choice in &choice.actions {
        if !side.is_active(slot_choice.actor) {
            return Err(ChoiceError::ActorIsNotActive(slot_choice.actor));
        }
        if seen_actors.contains(&slot_choice.actor) {
            return Err(ChoiceError::DuplicateActor(slot_choice.actor));
        }
        seen_actors.push(slot_choice.actor);

        match &slot_choice.action {
            ActionChoice::Move {
                slot,
                target,
                mechanic,
            } => {
                validate_move_action(
                    state,
                    rules,
                    dex,
                    choice.side,
                    slot_choice.actor,
                    *slot,
                    *target,
                    *mechanic,
                )?;
                if let Some(mechanic) = mechanic {
                    if seen_mechanics.contains(mechanic) {
                        return Err(ChoiceError::DuplicateMechanic(*mechanic));
                    }
                    seen_mechanics.push(*mechanic);
                }
            }
            ActionChoice::Switch { pokemon } => {
                validate_switch_action(state, rules, choice.side, slot_choice.actor, *pokemon)?;
                if seen_switch_targets.contains(pokemon) {
                    return Err(ChoiceError::DuplicateSwitchTarget(*pokemon));
                }
                seen_switch_targets.push(*pokemon);
            }
            ActionChoice::Pass => {}
        }
    }

    Ok(())
}

/// One move action on its own, as `validate_side_choice` checks it — all but the side's own
/// limit of one mechanic of a kind. Shared with `choice_options`.
#[allow(clippy::too_many_arguments)]
fn validate_move_action(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    side: SideId,
    actor: PokemonId,
    slot: MoveSlot,
    target: Option<DeclaredTarget>,
    mechanic: Option<MechanicChoice>,
) -> Result<(), ChoiceError> {
    // A recharging Pokémon's request holds "Recharge" alone (Showdown `mustrecharge`'s
    // `onLockMove`): BeforeMove spends the action whatever it declares.
    // With no move left, the request holds Struggle alone (`must_struggle`). A Gen 1
    // Pokémon asleep or frozen is offered "Fight" alone (Showdown gen1
    // `getMoveRequestData`): whatever slot it names — a disabled one too — it acts
    // with its side's last selected move.
    if !recharging(state, actor)
        && !must_struggle(state, rules, dex, actor)
        && !gen1_fight_only(state, rules, actor)
    {
        validate_move(state, rules, dex, actor, slot, target, mechanic)?;
    }

    if let Some(mechanic) = mechanic {
        // A locked, recharging or struggling Pokémon's request offers no mechanic
        // (Showdown `getMoveRequestData`: none with a `lockedMove`).
        if state
            .pokemon(actor)
            .is_some_and(|p| p.hard_lock().is_some() || p.semi_locked_move().is_some())
            || recharging(state, actor)
            || must_struggle(state, rules, dex, actor)
        {
            return Err(ChoiceError::MechanicUnavailable(mechanic));
        }
        validate_mechanic(state, rules, side, actor, mechanic)?;
        // A Z-Move needs the chosen move to have one: the crystal applies to it and
        // the slot still has PP (Showdown `getZMove`'s checks, DESIGN.md §44.4).
        // Dynamax: the Pokémon must be able to (44.5's conditions) — a forme the
        // dex forbids, a Z-Crystal held or a Mega Evolution available refuse it,
        // and so does one that has Dynamaxed already.
        if mechanic == MechanicChoice::Dynamax && !crate::engine::can_dynamax(state, dex, actor) {
            return Err(ChoiceError::MechanicNotApplicable { actor, mechanic });
        }
        if mechanic == MechanicChoice::ZMove {
            let convertible = state
                .pokemon(actor)
                .and_then(|pokemon| pokemon.move_slot(slot))
                .filter(|slot_state| slot_state.pp > 0)
                .and_then(|slot_state| dex.move_by_id(slot_state.move_id))
                .and_then(|base| crate::engine::z_move_execution(state, rules, dex, actor, base))
                .is_some();
            if !convertible {
                return Err(ChoiceError::MechanicNotApplicable { actor, mechanic });
            }
        }
    }
    Ok(())
}

/// One switch on its own, as `validate_side_choice` checks it — all but the side's own limit
/// of a team member switched in once. Shared with `choice_options`.
fn validate_switch_action(
    state: &BattleState,
    rules: &ResolvedRules,
    side: SideId,
    actor: PokemonId,
    pokemon: PokemonId,
) -> Result<(), ChoiceError> {
    if let Some(by) = trapped_by(state, rules, actor) {
        return Err(ChoiceError::Trapped { actor, by });
    }
    if let Some(by) = field_traps(state, rules, actor) {
        return Err(ChoiceError::TrappedByField { actor, by });
    }
    if trapped_by_ability(state, rules, actor).is_some() {
        return Err(ChoiceError::TrappedByAbility { actor });
    }
    switch_target_valid(state, side, pokemon)
}

/// The team member a switch names: on the team, off the field, standing.
fn switch_target_valid(
    state: &BattleState,
    side: SideId,
    pokemon: PokemonId,
) -> Result<(), ChoiceError> {
    let side = state.side(side);
    if !side.team.contains(&pokemon) {
        return Err(ChoiceError::SwitchTargetNotOnTeam(pokemon));
    }
    if side.is_active(pokemon) {
        return Err(ChoiceError::SwitchTargetAlreadyActive(pokemon));
    }
    if state.pokemon(pokemon).is_none_or(|p| p.is_fainted()) {
        return Err(ChoiceError::SwitchTargetFainted(pokemon));
    }
    Ok(())
}

/// Gen 1: an asleep, frozen or bound (`partiallytrapped`) Pokémon's request is the "Fight"
/// button alone; one locked into its binding move uses that move (Showdown `chooseMove`'s
/// `lockedMove`), whatever the action names (DESIGN.md §30.9).
fn gen1_fight_only(state: &BattleState, rules: &ResolvedRules, actor: PokemonId) -> bool {
    let bound = rules.conditions.switching.binding_locks_user
        && state.pokemon(actor).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .contains(VolatileConditionKind::PartiallyTrapped)
                || pokemon.volatiles.partial_trapping_lock().is_some()
        });
    bound
        || (rules.action_order.sleeping_frozen_act_with_last_selected
            && state.pokemon(actor).is_some_and(|pokemon| {
                pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                    || pokemon.has_status(crate::state::NonVolatileStatus::Freeze)
            }))
}

/// The declared move itself: its slot, PP, support, the locks that keep it off the request and
/// its target position — the Max Move's when the actor is Dynamaxed or Dynamaxes now, the
/// Z-Move's when the choice asks for one (`mechanic`).
fn validate_move(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    actor: PokemonId,
    slot: MoveSlot,
    target: Option<DeclaredTarget>,
    mechanic: Option<MechanicChoice>,
) -> Result<(), ChoiceError> {
    let dynamax = mechanic == Some(MechanicChoice::Dynamax);
    let move_slot = state
        .pokemon(actor)
        .and_then(|pokemon| pokemon.move_slot(slot))
        .ok_or(ChoiceError::InvalidMoveSlot { actor, slot })?;

    // A lock onto a move the slots do not hold (Outrage, Rollout … called by Metronome, DESIGN.md
    // §42.1; Gen 1-2 Bide a Mirror Move called, `getSemiLockedMove`, R-M8-60): Showdown's request
    // offers that move alone, so the named slot means it, whatever it holds — and it is offered
    // whatever its PP (its own move has none to spend).
    let locked_outside = state
        .pokemon(actor)
        .and_then(|pokemon| {
            pokemon
                .hard_lock()
                .map(|lock| lock.move_id)
                .or_else(|| pokemon.semi_locked_move())
        })
        .filter(|locked| pokemon_has_no_slot(state, actor, *locked));
    let move_id = locked_outside.unwrap_or(move_slot.move_id);
    let locked = locked_outside.is_some()
        || state
            .pokemon(actor)
            .and_then(|p| p.locked_move().or_else(|| p.semi_locked_move()))
            == Some(move_id);
    if move_slot.pp == 0 && !locked {
        return Err(ChoiceError::NoPp { actor, move_id });
    }

    let move_def = dex
        .move_by_id(move_id)
        .ok_or(ChoiceError::UnknownMove(move_id))?;

    // A move the engine has not implemented must never reach the pipeline and silently do
    // nothing; the dex marks it and validation refuses it here.
    if move_def.primary_effect == PrimaryEffect::Unsupported {
        return Err(ChoiceError::UnsupportedMove(move_slot.move_id));
    }
    // A Max Move — the actor Dynamaxed, or Dynamaxing with this choice — answers to
    // `maxMoveDisabled` alone (Showdown `chooseMove`): its base move's PP, and a status move
    // under an Assault Vest or Taunt. Disable, Encore, Torment, Imprison, Heal Block, Throat
    // Chop, Gravity, a Choice lock leave it be (R-M10-5).
    let max_move = dynamax
        || state
            .pokemon(actor)
            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Dynamax));
    if max_move {
        if !locked
            && let Some(pokemon) = state.pokemon(actor)
            && max_move_disabled(state, rules, pokemon, move_def, move_slot.pp)
        {
            return Err(ChoiceError::MaxMoveDisabled { actor, move_id });
        }
    } else {
        // The locked move is offered whatever seals it (Showdown `getMoves(lockedMove)`); BeforeMove
        // then refuses it (a Disable on Solar Beam mid-charge: `cant … Disable`).
        if !locked
            && let Some(pokemon) = state.pokemon(actor)
            && let Some(by) = disabled_by(
                state,
                rules,
                pokemon,
                move_def,
                locked_outside.is_none().then(|| slot.index()),
            )
        {
            return Err(ChoiceError::MoveDisabled { actor, move_id, by });
        }
        if !locked
            && let Some(pokemon) = state.pokemon(actor)
            && let Some(item) = item_disables(state, rules, pokemon, move_def)
        {
            return Err(ChoiceError::MoveDisabledByItem {
                actor,
                move_id,
                item,
            });
        }
        if !locked && let Some(by) = field_disables(state, move_def) {
            return Err(ChoiceError::MoveDisabledByField { actor, move_id, by });
        }
        if !locked
            && let Some(pokemon) = state.pokemon(actor)
            && used_last(pokemon, move_def)
        {
            return Err(ChoiceError::MoveUsedTwice { actor, move_id });
        }
        if !locked && disables_itself(state, rules, actor, move_def) {
            return Err(ChoiceError::MoveDisabledByItself { actor, move_id });
        }
    }

    // A locked two-turn move aims where its charge aimed (Showdown fills the locked action's
    // `targetLoc` itself); the submitted target is not asked.
    // Showdown validates the target *position* at choice time; a slot emptied by a faint is a
    // legal choice, settled at execution (retarget or fail).
    if !locked {
        // Showdown `chooseMove`: a Pokémon Dynamaxed, or Dynamaxing with this choice, aims its
        // Max Move (`getMaxMove`: Max Guard for a status move, the type's Max Move otherwise),
        // whatever the base move's target.
        let dynamaxed = dynamax
            || state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Dynamax));
        let max_target = dynamaxed
            .then(|| crate::dex::max_move_for(dex, move_def))
            .flatten()
            .and_then(|max_move| dex.move_by_id(max_move))
            .map(|max_move| max_move.target);
        // ... and a Z-Move its own (`targetType = dex.moves.get(zMove).target`): a damaging
        // move's Z-Move takes a target where the base move had none (Mirror Coat's Shattered
        // Psyche, R-M9-34); a status move's is the move itself.
        let z_target = (mechanic == Some(MechanicChoice::ZMove))
            .then(|| crate::engine::z_move_execution(state, rules, dex, actor, move_def))
            .flatten()
            .map(|z| z.move_def.target);
        // Showdown `getMoves`: Pollen Puff aims only at a foe while its user is under Heal Block.
        let pattern = if let Some(pattern) = max_target.or(z_target) {
            pattern
        } else if move_def.behavior.ally_heal.is_some()
            && state
                .pokemon(actor)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::HealBlock))
        {
            crate::rules::TargetPattern::SelectedAdjacentFoe
        } else if let Some(pattern) = move_def.non_ghost_target
            && state.pokemon(actor).is_some_and(|p| {
                !crate::engine::has_type(state, rules, p, crate::dex::PokemonType::Ghost)
            })
        {
            // Showdown `getMoves`: Curse as its user's type has it (`nonGhostTarget`).
            pattern
        } else {
            // Showdown `moveSlot.target`: a copy keeps where the use it copied went
            // (`MoveSlotState::target`).
            locked_outside
                .is_none()
                .then_some(move_slot.target)
                .flatten()
                .unwrap_or(move_def.target)
        };
        match rules
            .targeting
            .resolve(&rules.topology, state, actor, pattern, target)
        {
            Ok(_) => {}
            // Showdown `randomNormal` / `scripted` (Counter, Metal Burst): no target is chosen;
            // the Shell draws a random adjacent foe as the move starts.
            Err(TargetError::TargetRequired)
                if pattern == crate::rules::TargetPattern::RandomAdjacentFoe
                    && target.is_none() => {}
            Err(TargetError::TargetUnavailable(_)) => rules
                .targeting
                .validate_position(&rules.topology, state, actor, pattern, target)
                .map_err(ChoiceError::InvalidTarget)?,
            Err(error) => return Err(ChoiceError::InvalidTarget(error)),
        }
    }
    Ok(())
}

/// Showdown `getMoves`: when no move is left to choose (each out of PP, or kept off the request
/// by a Choice lock, Encore, Disable, Taunt, `cantusetwice`, an Assault Vest or the move's own
/// `onDisableMove`), the request
/// holds Struggle alone. A recharge or a two-turn move in progress is a lock of its own. Shared
/// with the replay driver.
pub fn must_struggle(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    actor: PokemonId,
) -> bool {
    let Some(pokemon) = state.pokemon(actor) else {
        return false;
    };
    if pokemon.locked_move().is_some()
        || pokemon.semi_locked_move().is_some()
        || pokemon
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
    {
        return false;
    }
    // Dynamaxed (Showdown `getMoves`): a slot is off when its Max Move is (`maxMoveDisabled`),
    // or when one of the effects that can leave a Dynamax Pokémon to Struggle keeps its base move
    // off (`canCauseStruggle`: Encore, Disable, Taunt, Assault Vest, Belch, Stuff Cheeks) —
    // Torment, Imprison, Heal Block, Throat Chop, Gravity never do (R-M10-5).
    if pokemon.volatiles.contains(VolatileConditionKind::Dynamax) {
        return !pokemon.move_slots().is_empty()
            && pokemon
                .move_slots()
                .iter()
                .enumerate()
                .all(|(index, slot)| {
                    dex.move_by_id(slot.move_id).is_none_or(|move_def| {
                        max_move_disabled(state, rules, pokemon, move_def, slot.pp)
                            || matches!(
                                disabled_by(state, rules, pokemon, move_def, Some(index)),
                                Some(
                                    VolatileConditionKind::Encore
                                        | VolatileConditionKind::Disable
                                        | VolatileConditionKind::Taunt
                                )
                            )
                            || item_disables(state, rules, pokemon, move_def).is_some()
                            || disables_itself(state, rules, actor, move_def)
                    })
                });
    }
    !pokemon.move_slots().is_empty()
        && pokemon
            .move_slots()
            .iter()
            .enumerate()
            .all(|(index, slot)| {
                slot.pp == 0
                    || dex.move_by_id(slot.move_id).is_none_or(|move_def| {
                        disabled_by(state, rules, pokemon, move_def, Some(index)).is_some()
                            || item_disables(state, rules, pokemon, move_def).is_some()
                            || field_disables(state, move_def).is_some()
                            || used_last(pokemon, move_def)
                            || disables_itself(state, rules, actor, move_def)
                    })
            })
}

/// Showdown `maxMoveDisabled` (it reads the base move): out of PP, or a status move while the
/// Pokémon holds an Assault Vest (`hasItem`: not one set aside) or is taunted — Me First too.
fn max_move_disabled(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    move_def: &ResolvedMove,
    pp: u8,
) -> bool {
    pp == 0
        || (move_def.category == MoveCategory::Status
            && (crate::engine::held_item(state, rules, pokemon.id())
                .is_some_and(|(_, behavior)| behavior.disables_status_moves)
                || pokemon.volatiles.contains(VolatileConditionKind::Taunt)))
}

/// Taunt's refusal (`onDisableMove`, `onBeforeMove`): a status move, but Me First from Gen 5.
pub(crate) fn taunt_refuses(rules: &ResolvedRules, move_def: &ResolvedMove) -> bool {
    move_def.category == MoveCategory::Status
        && !(rules.conditions.taunt_spares_me_first && move_def.id == MoveId::ME_FIRST)
}

/// Showdown `mustrecharge`: the turn after Hyper Beam holds nothing but the recharge.
pub(crate) fn recharging(state: &BattleState, actor: PokemonId) -> bool {
    state
        .pokemon(actor)
        .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::MustRecharge))
}

fn validate_mechanic(
    state: &BattleState,
    rules: &ResolvedRules,
    side: SideId,
    actor: PokemonId,
    mechanic: MechanicChoice,
) -> Result<(), ChoiceError> {
    let enabled = match mechanic {
        MechanicChoice::MegaEvolve | MechanicChoice::UltraBurst => rules.special.mega,
        MechanicChoice::ZMove => rules.special.z_move,
        MechanicChoice::Dynamax => rules.special.dynamax,
        MechanicChoice::Terastallize => rules.special.tera,
    };

    let usage = match mechanic {
        MechanicChoice::MegaEvolve => state.side(side).mechanics.mega,
        MechanicChoice::UltraBurst => state.side(side).mechanics.ultra_burst,
        MechanicChoice::ZMove => state.side(side).mechanics.z_move,
        MechanicChoice::Dynamax => state.side(side).mechanics.dynamax,
        MechanicChoice::Terastallize => state.side(side).mechanics.tera,
    };

    if !enabled || usage != MechanicUsage::Available {
        return Err(ChoiceError::MechanicUnavailable(mechanic));
    }
    // Showdown `chooseMove`'s `canMegaEvo` / `canUltraBurst`: the Pokémon must have a forme to
    // change into (`runMegaEvo` returns false else).
    if matches!(
        mechanic,
        MechanicChoice::MegaEvolve | MechanicChoice::UltraBurst
    ) {
        let able = state.pokemon(actor).is_some_and(|pokemon| {
            if mechanic == MechanicChoice::MegaEvolve {
                pokemon.mega_evolves_into.is_some()
            } else {
                pokemon.ultra_bursts_into.is_some()
            }
        });
        if !able {
            return Err(ChoiceError::MechanicNotApplicable { actor, mechanic });
        }
    }
    // Showdown `canTerastallize`: one that has Terastallized already cannot again (its side's
    // use is spent, and the Pokémon itself would change nothing).
    if mechanic == MechanicChoice::Terastallize
        && state
            .pokemon(actor)
            .is_some_and(|pokemon| pokemon.terastallized.is_some())
    {
        return Err(ChoiceError::MechanicNotApplicable { actor, mechanic });
    }
    Ok(())
}

/// Showdown `TrapPokemon`: what keeps the Pokémon from switching. A hard lock (a two-turn move
/// in progress, a Rollout chain, Gen 3+ Bide: Showdown's `getLockedMove`) or a recharge always
/// does; Bind / Wrap … while the trapper
/// is still on the field, Ingrain and Mean Look's `trapped` do so unless the Pokémon is a Ghost
/// type from Gen 6 (`ghost_escapes_trapping`).
pub fn trapped_by(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon_id: PokemonId,
) -> Option<VolatileConditionKind> {
    let pokemon = state.pokemon(pokemon_id)?;
    if let Some(lock) = pokemon.hard_lock() {
        return Some(lock.by);
    }
    if recharging(state, pokemon_id) {
        return Some(VolatileConditionKind::MustRecharge);
    }
    trapping_condition(state, rules, pokemon_id)
}

/// Showdown `pokemon.trapped` as `endTurn` leaves it (`runEvent('TrapPokemon')`): a condition,
/// Fairy Lock or an ability around holds the Pokémon. A lock into a move or a recharge is not
/// counted: it keeps only the request from switching. Endless Battle Clause reads this
/// (DESIGN.md §45.7).
pub fn trapped_at_turn_end(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon_id: PokemonId,
) -> bool {
    trapping_condition(state, rules, pokemon_id).is_some()
        || field_traps(state, rules, pokemon_id).is_some()
        || trapped_by_ability(state, rules, pokemon_id).is_some()
}

/// The Pokémon's own trapping conditions (`TrapPokemon` from its volatiles), in Showdown's
/// priority order.
fn trapping_condition(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon_id: PokemonId,
) -> Option<VolatileConditionKind> {
    let pokemon = state.pokemon(pokemon_id)?;
    // Sky Drop's carried Pokémon (`onFoeTrapPokemon`, priority −15: after Shed Shell's −10, no
    // Ghost exception).
    if pokemon
        .volatiles
        .contains(VolatileConditionKind::SkyDropped)
    {
        return Some(VolatileConditionKind::SkyDropped);
    }
    // Commander (`onTrapPokemon` priority −11: before Shed Shell's −10): neither the Tatsugiri
    // in its ally's mouth nor the Dondozo holding it can leave (DESIGN.md §43.4).
    if pokemon
        .volatiles
        .contains(VolatileConditionKind::Commanding)
        || pokemon.volatiles.contains(VolatileConditionKind::Commanded)
    {
        return Some(VolatileConditionKind::Commanding);
    }
    if sheds_traps(state, rules, pokemon_id) {
        return None;
    }
    if rules.conditions.ghost_escapes_trapping
        && crate::engine::types_of(state, rules, pokemon).contains(crate::dex::PokemonType::Ghost)
    {
        return None;
    }
    // Gen 1's binding keeps the target from moving, not from switching (§30.9).
    if let Some((trapper, _)) = pokemon.volatiles.partially_trapped_by()
        && !rules.conditions.switching.binding_locks_user
        && state.active_position(trapper, &rules.topology).is_some()
        && state.pokemon(trapper).is_some_and(|t| !t.is_fainted())
    {
        return Some(VolatileConditionKind::PartiallyTrapped);
    }
    if pokemon.volatiles.contains(VolatileConditionKind::Ingrain) {
        return Some(VolatileConditionKind::Ingrain);
    }
    // No Retreat (`onTrapPokemon`).
    if pokemon.volatiles.contains(VolatileConditionKind::NoRetreat) {
        return Some(VolatileConditionKind::NoRetreat);
    }
    // Mean Look, Block, Jaw Lock …: released as soon as the trapper leaves the field.
    if pokemon.volatiles.contains(VolatileConditionKind::Trapped) {
        return Some(VolatileConditionKind::Trapped);
    }
    // Octolock (`onTrapPokemon`: only while its source is on the field).
    if let Some(source) = pokemon.volatiles.octolocked_by()
        && state.active_position(source, &rules.topology).is_some()
        && state.pokemon(source).is_some_and(|s| !s.is_fainted())
    {
        return Some(VolatileConditionKind::Octolock);
    }
    None
}

/// Shed Shell (`onTrapPokemon` / `onMaybeTrapPokemon`, priority −10): whatever trapped the
/// Pokémon — a move's volatile, Ingrain, an ability, the field — it is free again.
fn sheds_traps(state: &BattleState, rules: &ResolvedRules, pokemon_id: PokemonId) -> bool {
    crate::engine::held_item(state, rules, pokemon_id)
        .is_some_and(|(_, behavior)| behavior.frees_from_traps)
}

/// Showdown `TrapPokemon` from the abilities around (`onFoeTrapPokemon`, Gen 3 Magnet Pull's
/// `onAnyTrapPokemon`): the first active Pokémon adjacent to `pokemon_id` whose ability's filter
/// takes it (Shadow Tag: anyone, from Gen 4 no other Shadow Tag; Arena Trap: a grounded
/// Pokémon; Magnet Pull: a Steel type). A Ghost type escapes from Gen 6 (`tryTrap` asks
/// `runStatusImmunity('trapped')`).
pub fn trapped_by_ability(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon_id: PokemonId,
) -> Option<(PokemonId, AbilityId)> {
    let pokemon = state.pokemon(pokemon_id)?;
    if sheds_traps(state, rules, pokemon_id)
        || (rules.conditions.ghost_escapes_trapping
            && crate::engine::types_of(state, rules, pokemon)
                .contains(crate::dex::PokemonType::Ghost))
    {
        return None;
    }
    let position = state.active_position(pokemon_id, &rules.topology)?;
    let around = rules.topology.all_adjacent(position);
    around.into_iter().find_map(|other| {
        let holder_id = state.pokemon_at_position(other, &rules.topology)?;
        let holder = state.pokemon(holder_id).filter(|p| !p.is_fainted())?;
        let behavior = crate::engine::ability_handlers(state, rules, holder);
        let filter = if holder.side == pokemon.side {
            behavior.traps_any
        } else {
            behavior.traps_foes.or(behavior.traps_any)
        }?;
        let trapped = match filter {
            crate::effects::abilities::TrapFilter::Anyone => {
                !rules.conditions.switching.shadow_tag_spares_its_like
                    || crate::engine::ability_active(state, rules, pokemon)
                        != crate::engine::ability_active(state, rules, holder)
            }
            crate::effects::abilities::TrapFilter::Grounded => {
                crate::engine::environment::grounded(state, rules, pokemon)
            }
            crate::effects::abilities::TrapFilter::SteelType => {
                crate::engine::types_of(state, rules, pokemon)
                    .contains(crate::dex::PokemonType::Steel)
            }
        };
        trapped.then_some((holder_id, crate::engine::raw_ability(holder)))
    })
}

/// Showdown's `maybeTrapped` (`Battle`'s turn start): a foe's trapping ability may keep the
/// Pokémon in — the one it has (`trapped_by_ability`), or, from Gen 3 and where the format says
/// a species tells what it may have (`MaybeTrappedBy::SpeciesAbilities`), one of those of the
/// species it shows (an Illusion's disguise's) that the format does not ban and that would trap
/// this Pokémon (Shadow Tag but a Shadow Tag holder, Arena Trap a grounded one, Magnet Pull a
/// Steel type). A Gen 6+ Ghost type is never trapped.
fn maybe_trapped(
    state: &BattleState,
    rules: &ResolvedRules,
    dex: &ResolvedDex,
    actor: PokemonId,
) -> bool {
    if trapped_by_ability(state, rules, actor).is_some() {
        return true;
    }
    let crate::rules::MaybeTrappedBy::SpeciesAbilities { banned } = &rules.maybe_trapped_by else {
        return false;
    };
    let Some(pokemon) = state.pokemon(actor) else {
        return false;
    };
    if rules.generation < crate::spec::Generation::Gen3
        || sheds_traps(state, rules, actor)
        || (rules.conditions.ghost_escapes_trapping
            && crate::engine::types_of(state, rules, pokemon)
                .contains(crate::dex::PokemonType::Ghost))
    {
        return false;
    }
    let Some(position) = state.active_position(actor, &rules.topology) else {
        return false;
    };
    rules
        .topology
        .all_adjacent(position)
        .into_iter()
        .filter_map(|other| state.pokemon_at_position(other, &rules.topology))
        .filter_map(|foe| state.pokemon(foe))
        .filter(|foe| foe.side != pokemon.side && !foe.is_fainted())
        .any(|foe| {
            let shown = foe
                .illusion
                .and_then(|disguise| state.pokemon(disguise))
                .unwrap_or(foe);
            let Some(species) =
                crate::engine::species_of(shown).and_then(|species| dex.species_by_id(species))
            else {
                return false;
            };
            species.abilities.iter().any(|ability| {
                let behavior = crate::engine::ability_behavior(rules, *ability);
                !banned.contains(ability)
                    && match behavior.traps_foes.or(behavior.traps_any) {
                        Some(crate::effects::abilities::TrapFilter::Anyone) => {
                            !rules.conditions.switching.shadow_tag_spares_its_like
                                || crate::engine::ability_active(state, rules, pokemon)
                                    != Some(*ability)
                        }
                        Some(crate::effects::abilities::TrapFilter::Grounded) => {
                            crate::engine::environment::grounded(state, rules, pokemon)
                        }
                        Some(crate::effects::abilities::TrapFilter::SteelType) => {
                            crate::engine::types_of(state, rules, pokemon)
                                .contains(crate::dex::PokemonType::Steel)
                        }
                        None => false,
                    }
            })
        })
}

/// The field's `onTrapPokemon`: under Fairy Lock no Pokémon switches — but a Ghost type from
/// Gen 6 (`tryTrap` asks `runStatusImmunity('trapped')`: `ghost_escapes_trapping`).
fn field_traps(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon_id: PokemonId,
) -> Option<FieldConditionKind> {
    let pokemon = state.pokemon(pokemon_id)?;
    if !state.field.contains(FieldConditionKind::FairyLock)
        || sheds_traps(state, rules, pokemon_id)
        || (rules.conditions.ghost_escapes_trapping
            && crate::engine::types_of(state, rules, pokemon)
                .contains(crate::dex::PokemonType::Ghost))
    {
        return None;
    }
    Some(FieldConditionKind::FairyLock)
}

/// The field's `onDisableMove`: Gravity keeps the airborne moves (`gravity` flag) off the
/// request.
fn field_disables(state: &BattleState, move_def: &ResolvedMove) -> Option<FieldConditionKind> {
    (move_def.flags.gravity && state.field.contains(FieldConditionKind::Gravity))
        .then_some(FieldConditionKind::Gravity)
}

/// The item's `onDisableMove`: an Assault Vest keeps status moves off the request — all but
/// Me First (`move.id !== 'mefirst'`, R-M8-117).
fn item_disables(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    move_def: &ResolvedMove,
) -> Option<ItemId> {
    let (item, behavior) = crate::engine::held_item(state, rules, pokemon.id())?;
    (behavior.disables_status_moves
        && move_def.category == MoveCategory::Status
        && move_def.id != crate::ids::MoveId::ME_FIRST)
        .then_some(item)
}

/// Showdown's `DisableMove` pass (`cantusetwice`): Gigaton Hammer or Blood Moon, just used.
fn used_last(pokemon: &PokemonState, move_def: &ResolvedMove) -> bool {
    move_def.flags.cant_use_twice && pokemon.last_move == Some(move_def.id)
}

/// The move's own `onDisableMove` (Belch, Stuff Cheeks): off the request while its `onTry`
/// would refuse it (`MoveBehavior::self_disables`).
fn disables_itself(
    state: &BattleState,
    rules: &ResolvedRules,
    actor: PokemonId,
    move_def: &ResolvedMove,
) -> bool {
    move_def.behavior.self_disables
        && move_def.behavior.try_use.is_some_and(|try_use| {
            !crate::engine::try_use_holds(state, rules, actor, None, try_use)
        })
}

/// Showdown's `onDisableMove` handlers: which condition, if any, keeps `move_def` off the
/// request.
fn disabled_by(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &PokemonState,
    move_def: &ResolvedMove,
    slot: Option<usize>,
) -> Option<VolatileConditionKind> {
    let volatiles = &pokemon.volatiles;
    // A hard lock's move is the only move on the request (Showdown `getLockedMove`).
    if let Some(lock) = pokemon.hard_lock()
        && lock.move_id != move_def.id
    {
        return Some(lock.by);
    }
    // Gen 1-2 Bide (`onSemiLockMove`): the request holds Bide alone.
    if let Some(locked) = pokemon.semi_locked_move()
        && locked != move_def.id
    {
        return Some(VolatileConditionKind::Bide);
    }
    // A Dynamaxed holder's Choice and Gorilla Tactics locks hold nothing at all
    // (`choicelock.onDisableMove` / `gorillatactics.onDisableMove`: `if
    // (pokemon.volatiles['dynamax']) return;`, DESIGN.md §44.5): the request offers every move.
    let dynamaxed = volatiles.contains(VolatileConditionKind::Dynamax);
    // `choicelock.onDisableMove` holds nothing while the Choice item is set aside
    // (`ignoringItem()`), the lock kept (Gen 4 drops it at the turn's end: issues.md I55).
    if !dynamaxed
        && let Some(locked) = volatiles.choice_locked_move()
        && locked != move_def.id
        && pokemon.move_slot_of(locked).is_some()
        && super::held_item_of(state, rules, pokemon).is_some()
    {
        return Some(VolatileConditionKind::ChoiceLock);
    }
    // Gorilla Tactics' `onDisableMove`: its lock is the ability's, not a volatile — and only
    // while the ability runs (Gastro Acid, Neutralizing Gas set the handler aside).
    if !dynamaxed
        && let Some(locked) = pokemon.ability_state.choice_lock
        && locked != move_def.id
        && super::ability_handler(state, rules, pokemon, |b| b.choice_lock)
    {
        return Some(VolatileConditionKind::ChoiceLock);
    }
    if let Some(encored) = volatiles.encored_move()
        && encored != move_def.id
        && pokemon.move_slot_of(encored).is_some()
    {
        return Some(VolatileConditionKind::Encore);
    }
    // Gen 1 greys out the slot Disable drew (a copy of its move in another slot stays on the
    // request, to be refused at BeforeMove); later generations every slot holding the move.
    let disabled = match (volatiles.disabled_slot(), slot) {
        (Some(disabled), Some(slot)) => usize::from(disabled) == slot,
        _ => volatiles.disabled_move() == Some(move_def.id),
    };
    if disabled {
        return Some(VolatileConditionKind::Disable);
    }
    if volatiles.contains(VolatileConditionKind::Taunt) && taunt_refuses(rules, move_def) {
        return Some(VolatileConditionKind::Taunt);
    }
    // Torment (`onDisableMove`): the last move; Imprison (`onFoeDisableMove`, hidden from the
    // request but refused as it is chosen): a move a foe holding it knows.
    if super::context::tormented(state, pokemon.id(), move_def.id) {
        return Some(VolatileConditionKind::Torment);
    }
    if super::context::imprisoned_by(state, pokemon.id(), move_def.id).is_some() {
        return Some(VolatileConditionKind::Imprison);
    }
    if volatiles.contains(VolatileConditionKind::HealBlock) && move_def.flags.heal {
        return Some(VolatileConditionKind::HealBlock);
    }
    if volatiles.contains(VolatileConditionKind::ThroatChop) && move_def.flags.sound {
        return Some(VolatileConditionKind::ThroatChop);
    }
    None
}

/// Whether the Pokémon's own move slots lack `move_id` (a locked move a caller brought in).
fn pokemon_has_no_slot(state: &BattleState, pokemon: PokemonId, move_id: MoveId) -> bool {
    state.pokemon(pokemon).is_some_and(|pokemon| {
        !pokemon
            .move_slots()
            .iter()
            .any(|slot| slot.move_id == move_id)
    })
}
