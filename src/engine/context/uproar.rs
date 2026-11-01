//! `BattleContext`'s side of Uproar (Showdown `uproar`, DESIGN.md §15.12): the uproar as it
//! starts and ends, who it keeps awake, and the Pokémon it wakes.

use crate::ids::{AbilityId, MoveId, PokemonId, SideId};
use crate::rules::ResolvedRules;
use crate::state::{
    BattleState, NonVolatileStatus, RandomRequestKind, UproarState, VolatileConditionKind,
};

use super::BattleContext;
use crate::engine::{CureReason, TraceEvent};

/// Whether an uproar on the field keeps `target` from falling asleep (Showdown `uproar`'s
/// `onAnySetStatus`): any active Pokémon makes one — Gen 3-4 unless `target` is out of it,
/// Soundproof (`UproarRules::soundproof_sleeps`).
pub(crate) fn kept_awake(state: &BattleState, rules: &ResolvedRules, target: PokemonId) -> bool {
    !keeping_awake(state, rules, target).is_empty()
}

/// The Pokémon whose uproar keeps `target` awake (`kept_awake`), each running its own
/// `onAnySetStatus`.
pub(crate) fn keeping_awake(
    state: &BattleState,
    rules: &ResolvedRules,
    target: PokemonId,
) -> Vec<PokemonId> {
    if rules.conditions.uproar.soundproof_sleeps && soundproof(state, rules, target) {
        return Vec::new();
    }
    uproarers(state)
}

/// Gen 3-4: whether `mover`, asleep, wakes as it is about to move because an uproar goes on
/// (`BattleContext::uproar_wakes_mover`). Shared with the replay, which plans the move as an
/// awake user's (R-M8-160).
pub fn uproar_wakes_before_move(
    state: &BattleState,
    rules: &ResolvedRules,
    mover: PokemonId,
) -> bool {
    rules.conditions.uproar.wake == crate::rules::UproarWake::Cartridge
        && state
            .pokemon(mover)
            .is_some_and(|p| p.has_status(NonVolatileStatus::Sleep))
        && kept_awake(state, rules, mover)
}

/// The active Pokémon making an uproar.
fn uproarers(state: &BattleState) -> Vec<PokemonId> {
    [SideId::P1, SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().flatten().copied())
        .filter(|id| {
            state.pokemon(*id).is_some_and(|pokemon| {
                !pokemon.is_fainted() && pokemon.volatiles.uproar().is_some()
            })
        })
        .collect()
}

fn soundproof(state: &BattleState, rules: &ResolvedRules, target: PokemonId) -> bool {
    state.pokemon(target).is_some_and(|pokemon| {
        crate::engine::has_ability(state, rules, pokemon, AbilityId::SOUNDPROOF)
    })
}

impl<'a> BattleContext<'a> {
    /// Showdown `selfDrops` → `addVolatile('uproar')`: a hit by Uproar starts it, for the turns
    /// the generation draws (Gen 3-4) or 3 (Gen 5+) — unless its user makes one already or has
    /// fainted.
    pub fn start_uproar(&mut self, user: PokemonId) {
        let (low, high) = self.rules.conditions.uproar.turns;
        if self
            .state
            .pokemon(user)
            .is_none_or(|p| p.is_fainted() || p.volatiles.uproar().is_some())
        {
            return;
        }
        let turns = if low == high {
            low
        } else {
            self.state.rng.range_inclusive_for(
                RandomRequestKind::Duration,
                u16::from(low),
                u16::from(high),
            ) as u8
        };
        if self.state.pokemon_mut(user).is_some_and(|p| {
            p.volatiles.start_uproar(UproarState {
                turns,
                failed: false,
            })
        }) {
            self.trace.push(TraceEvent::VolatileStarted {
                target: user,
                kind: VolatileConditionKind::Uproar,
            });
        }
    }

    /// The uproar ends (`removeVolatile('uproar')`: `-end|holder|Uproar`).
    pub fn end_uproar(&mut self, holder: PokemonId) {
        self.remove_uproar(holder, true);
    }

    /// Gen 3-4, the cartridge's cancel (pokeemerald `CancelMultiTurnMoves`, issues.md I63): the
    /// uproar ends with no word (`VolatileEnded::seen`, DESIGN.md R-M10-31).
    pub fn cancel_uproar(&mut self, holder: PokemonId) {
        self.remove_uproar(holder, false);
    }

    fn remove_uproar(&mut self, holder: PokemonId, seen: bool) {
        if self
            .state
            .pokemon_mut(holder)
            .is_some_and(|p| p.volatiles.remove(VolatileConditionKind::Uproar))
        {
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind: VolatileConditionKind::Uproar,
                seen,
            });
        }
    }

    /// Gen 5+ Uproar's `onTryHit` as it hits `target`: every active Pokémon asleep wakes —
    /// `target`'s side and its foes', slot by slot.
    pub fn uproar_wakes_on_hit(&mut self, target: PokemonId) {
        let Some(side) = self.state.pokemon(target).map(|p| p.side) else {
            return;
        };
        self.wake_in_uproar([side, side.other()], false);
    }

    /// Gen 3-4 (the cartridge's turn-end step of an uproar's holder): every active Pokémon
    /// asleep wakes, in battler order (the player's first slot, the foe's, the player's second,
    /// the foe's) — a Soundproof one stays asleep.
    pub fn uproar_wakes_at_turn_end(&mut self) {
        self.wake_in_uproar([SideId::P1, SideId::P2], true);
    }

    /// Gen 3-4: a sleeping Pokémon about to move wakes while an uproar goes on (pokeemerald
    /// `CANCELLER_ASLEEP`, pokeplatinum `CHECK_STATUS_STATE_SLEEP`). True when it woke.
    pub fn uproar_wakes_mover(&mut self, mover: PokemonId) -> bool {
        uproar_wakes_before_move(self.state, self.rules, mover)
            && self.cure_status(mover, CureReason::Move(MoveId::UPROAR))
    }

    fn wake_in_uproar(&mut self, sides: [SideId; 2], spare_soundproof: bool) {
        let slots = usize::from(self.rules.topology.active_slots_per_side());
        let sleepers: Vec<PokemonId> = (0..slots)
            .flat_map(|slot| sides.map(|side| self.state.side(side).active.get(slot).copied()))
            .flatten()
            .flatten()
            .filter(|id| {
                self.state
                    .pokemon(*id)
                    .is_some_and(|p| !p.is_fainted() && p.has_status(NonVolatileStatus::Sleep))
                    && !(spare_soundproof && soundproof(self.state, self.rules, *id))
            })
            .collect();
        for sleeper in sleepers {
            self.cure_status(sleeper, CureReason::Move(MoveId::UPROAR));
        }
    }
}
