//! Reactions to a damaging hit that roll a chance after the ordered ones (Showdown
//! `onDamagingHit` / `onSourceDamagingHit` without an order, DESIGN.md §21): the holder's
//! Effect Spore, Cursed Body or Cute Charm, then the attacker's Poison Touch or Toxic Chain. The hit
//! sequence draws them and the replay asks these same predicates whether a decision is due.

use crate::dex::ResolvedMove;
use crate::effects::abilities::{DealtHitStatus, HitReaction};
use crate::ids::{AbilityId, MoveId, PokemonId};
use crate::rules::{EffectSporeRoll, Fraction, ResolvedRules};
use crate::state::{BattleState, NonVolatileStatus, VolatileConditionKind};

/// Cursed Body's, Poison Touch's and Toxic Chain's chance (`randomChance(3, 10)`).
pub const HIT_REACTION_CHANCE: Fraction = Fraction::new(3, 10);

/// Gen 3-4 Effect Spore's `sample` list: the `Sample` decision indexes it.
pub const EFFECT_SPORE_SAMPLE: [NonVolatileStatus; 3] = [
    NonVolatileStatus::Sleep,
    NonVolatileStatus::Paralysis,
    NonVolatileStatus::Poison,
];

/// The chance reactions a hit's `DamagingHit` event found — the holder's `onDamagingHit`, the
/// attacker's `onSourceDamagingHit` — read before any of its handlers runs: an ordered reaction
/// that changes an ability (Mummy, Wandering Spirit) calls none of them off and adds none
/// (Showdown `runEvent` runs the handlers it collected).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChanceReactors {
    pub holder: Option<HitReaction>,
    /// The attacker's ability and what it inflicts (the ability names the status's source).
    pub attacker: Option<(AbilityId, DealtHitStatus)>,
}

/// `ChanceReactors` for a hit by `attacker` on `holder`, as the two are now.
pub fn chance_reactors(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    holder: PokemonId,
) -> ChanceReactors {
    ChanceReactors {
        holder: state
            .pokemon(holder)
            .and_then(|p| super::ability_handler(state, rules, p, |b| b.hit_reaction)),
        attacker: state.pokemon(attacker).and_then(|p| {
            let dealt = super::ability_handler(state, rules, p, |b| b.dealt_hit_status)?;
            Some((super::ability_running(state, rules, p)?, dealt))
        }),
    }
}

/// Whether `pokemon`'s ability or item keeps secondaries off it (Shield Dust, Covert Cloak;
/// Showdown `onModifySecondaries`). Poison Touch and Toxic Chain check the same two.
pub fn shields_secondaries(state: &BattleState, rules: &ResolvedRules, pokemon: PokemonId) -> bool {
    state.pokemon(pokemon).is_some_and(|p| {
        super::ability_handler(state, rules, p, |b| b.shields_secondaries)
            || super::held_item(state, rules, pokemon)
                .is_some_and(|(_, item)| item.shields_secondaries)
    })
}

/// Showdown `checkMoveMakesContact(move, attacker, defender)`: whether `move_def` used by
/// `attacker` counts as contact — its `contact` flag, unless the attacker holds Protective Pads
/// (DESIGN.md §43.4). The effects that read the flag itself (Tough Claws, Punching Glove's own
/// edit) do not go through this.
pub fn move_makes_contact(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    move_def: &ResolvedMove,
) -> bool {
    move_def.flags.contact
        && !crate::engine::held_item(state, rules, attacker)
            .is_some_and(|(_, behavior)| behavior.pads_no_contact)
}

/// How Effect Spore draws on a hit by `move_def` that dealt `damage` to its holder, when it
/// does: a contact move; Gen 3-4 a hit that dealt damage, Gen 5+ an attacker not immune to
/// powder (Showdown `runStatusImmunity('powder')`).
pub fn effect_spore_roll(
    state: &BattleState,
    rules: &ResolvedRules,
    reactors: ChanceReactors,
    attacker: PokemonId,
    move_def: &ResolvedMove,
    damage: u16,
) -> Option<EffectSporeRoll> {
    if reactors.holder != Some(HitReaction::EffectSpore)
        || !move_makes_contact(state, rules, attacker, move_def)
    {
        return None;
    }
    let roll = rules.hits.effect_spore;
    let draws = match roll {
        EffectSporeRoll::ChanceThenSample(_) => {
            !(rules.hits.contact_reaction_needs_damage && damage == 0)
        }
        EffectSporeRoll::Percent => super::powder_immunity(state, rules, attacker).is_none(),
    };
    draws.then_some(roll)
}

/// The status Gen 5+ Effect Spore's `random(100)` gives: below 11 sleep, below 21 paralysis,
/// below 30 poison.
pub fn effect_spore_status(roll: u16) -> Option<NonVolatileStatus> {
    match roll {
        0..11 => Some(NonVolatileStatus::Sleep),
        11..21 => Some(NonVolatileStatus::Paralysis),
        21..30 => Some(NonVolatileStatus::Poison),
        _ => None,
    }
}

/// Whether Cursed Body rolls on a hit by `move_def`: its holder's ability, an attacker without a
/// Disable already, a move other than Struggle, a Max Move or a foreseen attack's strike
/// (`!move.isMax && !move.flags['futuremove']`: a Doom Desire striking draws nothing,
/// R-M8-144).
pub fn cursed_body_rolls(
    state: &BattleState,
    reactors: ChanceReactors,
    attacker: PokemonId,
    move_def: &ResolvedMove,
) -> bool {
    reactors.holder == Some(HitReaction::CursedBody)
        && move_def.id != MoveId::STRUGGLE
        && !move_def.is_max
        && !move_def.behavior.future_attack
        && state
            .pokemon(attacker)
            .is_some_and(|p| !p.volatiles.contains(VolatileConditionKind::Disable))
}

/// Whether Cute Charm rolls on a hit by `move_def` that dealt `damage` to its holder: a contact
/// move (Gen 3-4 a hit that dealt damage), whatever the two genders — Attract's `onStart` checks
/// them after the roll.
pub fn cute_charm_rolls(
    state: &BattleState,
    rules: &ResolvedRules,
    reactors: ChanceReactors,
    attacker: PokemonId,
    move_def: &ResolvedMove,
    damage: u16,
) -> bool {
    reactors.holder == Some(HitReaction::CuteCharm)
        && move_makes_contact(state, rules, attacker, move_def)
        && !(rules.hits.contact_reaction_needs_damage && damage == 0)
}

/// The attacker's Poison Touch / Toxic Chain and the status it may inflict on `target` with this
/// hit, when it rolls: not on a target Shield Dust or Covert Cloak protects, and Poison Touch
/// only with a contact move — whoever holds Protective Pads (the Sun / Moon research, issues.md
/// I8: Pads neither keep the holder from being poisoned nor stop its own Poison Touch; Showdown
/// checks the target's Pads, R-M8-104).
pub fn dealt_hit_status(
    state: &BattleState,
    rules: &ResolvedRules,
    reactors: ChanceReactors,
    target: PokemonId,
    move_def: &ResolvedMove,
) -> Option<(AbilityId, NonVolatileStatus)> {
    let (ability, dealt) = reactors.attacker?;
    if (dealt.contact_only && !move_def.flags.contact) || shields_secondaries(state, rules, target)
    {
        return None;
    }
    Some((ability, dealt.status))
}
