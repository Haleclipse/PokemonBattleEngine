//! Whether a move may be used and may touch its target (DESIGN.md §15.5): the move's own
//! conditions (`try_use_holds`), immunities, powder, Prankster, the holders' TryHit.

use super::*;

/// Whether a move's `onTry` lets it go ahead (Fake Out: only the user's first move action since
/// it came in, Showdown `activeMoveActions`; Dark Void: only Darkrai; Poltergeist: only against
/// a held item; Aurora Veil: only in hail / snow; Sucker Punch: only against a target about to
/// attack; Last Resort: only once every other move was used).
pub fn try_use_holds(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: Option<crate::ids::PokemonId>,
    try_use: crate::effects::moves::TryUse,
) -> bool {
    match try_use {
        crate::effects::moves::TryUse::FirstAction => state
            .pokemon(user)
            .is_some_and(|pokemon| pokemon.active_move_actions <= 1),
        crate::effects::moves::TryUse::UserHpBelowTarget => user_hp_below(state, user, target),
        crate::effects::moves::TryUse::UserSpecies(species) => state
            .pokemon(user)
            .is_some_and(|pokemon| species_of(pokemon) == Some(species)),
        crate::effects::moves::TryUse::UserSpeciesFamily(species) => {
            state.pokemon(user).is_some_and(|pokemon| {
                species_of(pokemon).is_some_and(|battling| battling.same_species(species))
            })
        }
        crate::effects::moves::TryUse::TargetHoldsItem => target
            .and_then(|target| state.pokemon(target))
            .is_some_and(|target| target.held_item.is_some()),
        crate::effects::moves::TryUse::InWeather(kind) => field_weather(state, rules) == Some(kind),
        crate::effects::moves::TryUse::TargetProtected => target
            .and_then(|target| state.pokemon(target))
            .is_some_and(|target| {
                target
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::Protect)
            }),
        crate::effects::moves::TryUse::InTerrain => state.field.terrain.is_some(),
        crate::effects::moves::TryUse::TargetAttacksNext => target.is_some_and(|target| {
            let recharging = state.pokemon(target).is_some_and(|pokemon| {
                pokemon
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::MustRecharge)
            });
            !recharging
                && state.will_move_with(target).is_some_and(|queued| {
                    queued.category != crate::dex::MoveCategory::Status
                        || (rules.conditions.calls.me_first_counts_as_an_attack
                            && queued.move_id == crate::ids::MoveId::ME_FIRST)
                })
        }),
        crate::effects::moves::TryUse::TargetUsesPriorityMove => target
            .and_then(|target| state.will_move_with(target))
            .is_some_and(|queued| {
                queued.priority > 0 && queued.category != crate::dex::MoveCategory::Status
            }),
        crate::effects::moves::TryUse::StrikesBack(kind) => {
            retaliation_record(state, rules, user, kind).is_some()
        }
        crate::effects::moves::TryUse::OtherMovesUsed => {
            state.pokemon(user).is_some_and(|pokemon| {
                pokemon.move_slots().len() >= 2
                    && pokemon
                        .move_slots()
                        .iter()
                        .any(|slot| slot.move_id == crate::ids::MoveId::LAST_RESORT)
                    && pokemon
                        .move_slots()
                        .iter()
                        .all(|slot| slot.move_id == crate::ids::MoveId::LAST_RESORT || slot.used)
            })
        }
        crate::effects::moves::TryUse::Stockpiled => state
            .pokemon(user)
            .is_some_and(|pokemon| pokemon.volatiles.stockpile_layers() > 0),
        crate::effects::moves::TryUse::UserHoldsBerry => state
            .pokemon(user)
            .and_then(|pokemon| pokemon.held_item)
            .is_some_and(|item| item_behavior(rules, item).is_berry),
        crate::effects::moves::TryUse::UserAteBerry => state
            .pokemon(user)
            .is_some_and(|pokemon| pokemon.history.ate_berry),
        crate::effects::moves::TryUse::UserAsleep => state.pokemon(user).is_some_and(|pokemon| {
            pokemon.has_status(crate::state::NonVolatileStatus::Sleep)
                || has_ability(state, rules, pokemon, crate::ids::AbilityId::COMATOSE)
        }),
    }
}

/// Whether `target` is immune to `user`'s move by the move's own `onTryImmunity`
/// (`MoveBehavior::try_immunity`: Endeavor, Dream Eater, Synchronoise, Octolock). Shared by the hit gate
/// and the replay driver.
pub fn move_immune(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> bool {
    let Some(rule) = move_def.behavior.try_immunity else {
        return false;
    };
    let (Some(attacker), Some(foe)) = (state.pokemon(user), state.pokemon(target)) else {
        return false;
    };
    let holds = match rule {
        crate::effects::moves::TryImmunity::UserHpBelowTarget => {
            user_hp_below(state, user, Some(target))
        }
        crate::effects::moves::TryImmunity::TargetAsleep => {
            let asleep = counts_as_asleep(state, rules, foe);
            let behind_substitute = rules.hits.dream_eater_substitute_immune
                && foe
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::Substitute);
            asleep && !behind_substitute
        }
        crate::effects::moves::TryImmunity::SharesUserType => {
            crate::engine::types_of(state, rules, attacker)
                .iter()
                .any(|t| crate::engine::types_of(state, rules, foe).contains(t))
        }
        crate::effects::moves::TryImmunity::OppositeGender => foe.gender.opposite(attacker.gender),
        crate::effects::moves::TryImmunity::Trappable => {
            !(rules.conditions.ghost_escapes_trapping
                && crate::engine::types_of(state, rules, foe)
                    .contains(crate::dex::PokemonType::Ghost))
        }
        crate::effects::moves::TryImmunity::NoStickyHold => {
            !crate::engine::has_ability(state, rules, foe, crate::ids::AbilityId::STICKY_HOLD)
        }
        crate::effects::moves::TryImmunity::NotTruantOrInsomnia => !matches!(
            crate::engine::raw_ability(foe),
            crate::ids::AbilityId::TRUANT | crate::ids::AbilityId::INSOMNIA
        ),
        crate::effects::moves::TryImmunity::NotGrass => {
            !crate::engine::types_of(state, rules, foe).contains(crate::dex::PokemonType::Grass)
        }
    };
    !holds
}

/// Whether `pokemon` shrugs off powder moves (Spore, Rage Powder's redirection) and, for the
/// ability / item, weather chip: a Grass type from Gen 6, Overcoat from Gen 6, Safety Goggles.
/// Returns the source of the immunity.
pub fn powder_immunity(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: crate::ids::PokemonId,
) -> Option<EffectSource> {
    if let Some(by) = powder_holds(state, rules, pokemon) {
        return Some(by);
    }
    let p = state.pokemon(pokemon)?;
    if !rules.conditions.powder_immunity {
        return None;
    }
    crate::engine::types_of(state, rules, p)
        .contains(crate::dex::PokemonType::Grass)
        .then_some(EffectSource::Rule)
}

/// Whether `pokemon`'s ability or item shrugs powder moves off at the TryHit step (Overcoat,
/// Safety Goggles; Showdown's `onTryHit` handlers, DESIGN.md R279). The Grass type's powder
/// immunity is the TryImmunity step's, after the abilities: Sap Sipper absorbs a Sleep Powder
/// before the powder is even looked at.
pub fn powder_holds(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pokemon: crate::ids::PokemonId,
) -> Option<EffectSource> {
    // Showdown's handlers ask `getImmunity('powder', …)`, which only exists from Gen 6; before
    // it Overcoat and Safety Goggles let every powder move through (their weather half stays).
    if !rules.conditions.powder_immunity {
        return None;
    }
    let p = state.pokemon(pokemon)?;
    // The same question passes a Grass type over (`getImmunity` is the type's own immunity):
    // the TryImmunity step turns the powder away, its `-immune` naming nothing (R-M10-39).
    if crate::engine::types_of(state, rules, p).contains(crate::dex::PokemonType::Grass) {
        return None;
    }
    if let Some((item, behavior)) = held_item(state, rules, pokemon)
        && behavior.weather_and_powder_immune
    {
        return Some(EffectSource::Item { pokemon, item });
    }
    if ability_handler(state, rules, p, |b| b.weather_and_powder_immune) {
        return Some(EffectSource::Ability {
            pokemon,
            ability: crate::engine::raw_ability(p),
        });
    }
    None
}

/// Gen 7+ `hitStepTryImmunity`: a Dark-type foe is immune to a move Prankster boosted
/// (`boosted`, Showdown `pranksterBoosted`) while its user still has Prankster.
pub fn prankster_immune(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    boosted: bool,
) -> bool {
    let (Some(user), Some(foe)) = (state.pokemon(user), state.pokemon(target)) else {
        return false;
    };
    rules.action_order.prankster_dark_immunity
        && boosted
        && has_ability(state, rules, user, crate::ids::AbilityId::PRANKSTER)
        && user.side != foe.side
        && crate::engine::types_of(state, rules, foe).contains(crate::dex::PokemonType::Dark)
}

/// The TryHit event of the target's own ability and item (Good as Gold, Lightning Rod …):
/// the reason one of them refuses `move_def` from `user`, if one does. The hit gate runs it
/// after the side's protections; a field move that runs TryHit per Pokémon (Perish Song's
/// `onHitField`) runs it alone.
pub(crate) fn holders_try_hit(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> Option<HitBlockReason> {
    let pokemon = state.pokemon(target)?;
    let mut ctx = TryHitContext::new(state, rules, user, target, move_def);
    if let TryHitResult::Blocked(reason) =
        crate::effects::abilities::try_hit(ability_handlers(state, rules, pokemon), &mut ctx)
    {
        return Some(reason);
    }
    let (_, item) = held_item(state, rules, target)?;
    let mut ctx = TryHitContext::new(state, rules, user, target, move_def);
    match crate::effects::items::try_hit(item, &mut ctx) {
        TryHitResult::Blocked(reason) => Some(reason),
        _ => None,
    }
}
