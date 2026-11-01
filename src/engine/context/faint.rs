//! The faint events of Showdown's `faintMessages` (DESIGN.md §20.2, R128): `runEvent('Faint')`
//! as each queued faint is processed, and `runEvent('AfterFaint')` once the queue is empty.

use crate::ids::PokemonId;
use crate::state::{FaintCause, Stat, VolatileConditionKind};

use super::BattleContext;
use crate::engine::{EffectSource, TraceEvent};

impl<'a> BattleContext<'a> {
    /// Showdown `runEvent('Faint', pokemon, source, effect)`, after the `faint` message and
    /// before the Pokémon's ability and item end and its volatiles clear: the `onAnyFaint`
    /// handlers (Soul-Heart) of the active Pokémon. Showdown asks only those standing, faster
    /// first; every handler so far raises its own holder, which a holder at 0 HP refuses, so
    /// field order does.
    pub(crate) fn faint_event(&mut self, cause: FaintCause) {
        if !cause.bonds_run {
            self.faint_bonds(cause, &[]);
        }
        if !cause.listeners_run {
            self.faint_listeners(cause);
        }
    }

    /// The `Faint` event's listeners: every Pokémon on the field's `onAnyFaint` (Soul-Heart),
    /// its allies' `onAllyFaint` (Receiver, Power of Alchemy).
    pub(crate) fn faint_listeners(&mut self, cause: FaintCause) {
        // The handlers are gathered first (Showdown `runEvent` finds them before running any): an
        // ability one of them gives starts nothing more in this event.
        let side = self.side_of(cause.target);
        let listeners: Vec<(PokemonId, crate::effects::abilities::AnyFaintHandler)> = self
            .active_pokemon()
            .into_iter()
            .flat_map(|holder| {
                let behavior = self
                    .running_ability(holder)
                    .map(|ability| crate::engine::ability_behavior(self.rules, ability));
                let ally = holder != cause.target && self.side_of(holder) == side;
                [
                    behavior.and_then(|behavior| behavior.any_faint),
                    behavior
                        .and_then(|behavior| behavior.ally_faint)
                        .filter(|_| ally),
                ]
                .into_iter()
                .flatten()
                .map(move |handler| (holder, handler))
            })
            .collect();
        for (holder, handler) in listeners {
            handler(self, holder, cause);
        }
    }

    /// The fainting Pokémon's own `onFaint` handlers (DESIGN.md §15.9), when a move knocked it
    /// out (`effect.effectType === 'Move'`) — not a foreseen one striking (`!effect.flags[
    /// 'futuremove']`, DESIGN.md R277). Destiny Bond takes a foe that did it down too (`source.faint()`, at once in
    /// the same faint queue); Grudge empties the PP of the move its source used last (every
    /// slot holding it), an ally's too. `reached`: the faints of the batch processed before this
    /// one that the engine has not taken off the field yet (`faint_processed`).
    fn faint_bonds(&mut self, cause: FaintCause, reached: &[PokemonId]) {
        let Some(source) = cause.source.filter(|_| cause.by_move && !cause.future_move) else {
            return;
        };
        let Some((bond, grudge)) = self.state.pokemon(cause.target).map(|pokemon| {
            (
                pokemon
                    .volatiles
                    .contains(VolatileConditionKind::DestinyBond),
                pokemon.volatiles.contains(VolatileConditionKind::Grudge),
            )
        }) else {
            return;
        };
        // `destinybond.onFaint`: a Dynamaxed source is immune to the bond (DESIGN.md §44.5).
        let source_dynamaxed = self
            .state
            .pokemon(source)
            .is_some_and(|pokemon| pokemon.volatiles.contains(VolatileConditionKind::Dynamax));
        if bond && !source_dynamaxed && self.side_of(source) != self.side_of(cause.target) {
            self.faint_directly(
                source,
                EffectSource::Volatile(VolatileConditionKind::DestinyBond),
            );
        }
        // Showdown's Grudge returns for a source that has fainted already (`source.fainted`):
        // a Pokémon knocked out by its own move — Self-Destruct's user — keeps its PP; one that
        // Rough Skin knocked out after its hit, its faint still queued, loses it (R-M10-11). A
        // Max Move is read through its `baseMove` (grudge's `move.isMax && move.baseMove`, §44.5).
        if grudge
            && !self.faint_processed(source, reached)
            && let Some(last) = self
                .state
                .pokemon(source)
                .and_then(|p| p.last_move_resolved())
        {
            let Some(pokemon) = self.state.pokemon_mut(source) else {
                return;
            };
            let mut emptied = false;
            for slot in pokemon
                .move_slots_mut()
                .iter_mut()
                .filter(|slot| slot.move_id == last)
            {
                slot.pp = 0;
                emptied = true;
            }
            if emptied {
                self.trace.push(TraceEvent::PpReduced {
                    pokemon: source,
                    move_id: last,
                    remaining: 0,
                    source: EffectSource::Volatile(VolatileConditionKind::Grudge),
                });
            }
        }
    }

    /// Showdown `pokemon.fainted`: `faintMessages` has processed `pokemon`'s faint — it sets the
    /// flag after the `Faint` event, so a Pokémon at 0 HP whose turn in the queue is still to
    /// come has not fainted yet. Processed: at the action's end, where the engine takes it off
    /// the field; at a hit loop's end (`announced_faints`); or earlier in the batch (`reached`).
    fn faint_processed(&self, pokemon: PokemonId, reached: &[PokemonId]) -> bool {
        self.is_fainted(pokemon)
            && (self
                .state
                .active_position(pokemon, &self.rules.topology)
                .is_none()
                || self.state.announced_faints.contains(&pokemon)
                || reached.contains(&pokemon))
    }

    /// Showdown `faintMessages` at the end of a move's hit loop (Gen 4+,
    /// `HitRules::bonds_at_hit_loop_end`), before its recoil and `AfterMoveSecondarySelf`: the
    /// Pokémon the move knocked out take their bonds along there — Destiny Bond knocks the user
    /// out before its recoil or Life Orb can touch it, Grudge empties its move. The rest of each
    /// faint waits for the action's end.
    pub(crate) fn knocked_out_bonds(&mut self) {
        if !self.rules.hits.bonds_at_hit_loop_end {
            return;
        }
        let causes: Vec<FaintCause> = self
            .state
            .faint_queue
            .iter()
            .map(|&target| self.queued_cause(target))
            .collect();
        // The faint at the action's end does not run them again (`bonds_run`): Grudge's PP may
        // have come back by then (a Leppa Berry eaten at the `Update` in between, R-M8-185).
        // The queue is taken in order: each faint is processed before the next one's bonds.
        let mut reached = Vec::new();
        for cause in causes {
            self.faint_bonds(cause, &reached);
            reached.push(cause.target);
            self.mark_cause(cause, |stored| stored.bonds_run = true);
        }
    }

    /// The hit loop's `faintMessages` (Showdown `hitStepMoveHitLoop`), after the move's own
    /// `self`: each faint the move's hits queued is announced in turn — no longer a Pokémon left
    /// for `foePokemonLeft`, counted in `totalFainted` (`count_faint`, R-M10-61) — and from Gen 4
    /// (`HitRules::bonds_at_hit_loop_end`) its `Faint` event listeners (Soul-Heart, Receiver)
    /// hear it then, the faints after it still left; the batch's `AfterFaint` (Moxie, Beast
    /// Boost, As One) follows — before the move's `AfterMove`, so a foe's Mirror Herb or
    /// Opportunist copies a knock-out's raise as the move ends (R-M10-52). Taking the Pokémon off
    /// the field waits for the action's end, which runs none of it again (`listeners_run`,
    /// `after_faint_run`, `counted`). `pending`: a user the recoil after the hits knocked out,
    /// left for the action's end.
    pub(crate) fn announce_knocked_out(&mut self, pending: Option<PokemonId>) {
        let faint_events = self.rules.hits.bonds_at_hit_loop_end;
        let mut batch = Vec::new();
        for pokemon in self.state.faint_queue.clone() {
            if Some(pokemon) == pending || self.state.announced_faints.contains(&pokemon) {
                continue;
            }
            self.state.announced_faints.push(pokemon);
            let cause = self.queued_cause(pokemon);
            self.count_faint(cause);
            if !faint_events || cause.after_faint_run {
                continue;
            }
            if !cause.listeners_run {
                self.faint_listeners(cause);
            }
            self.mark_cause(cause, |stored| {
                stored.listeners_run = true;
                stored.after_faint_run = true;
            });
            batch.push(cause);
        }
        if let Some(&last) = batch.last() {
            self.after_faint(last, batch.len());
        }
    }

    /// The cause the faint queue's `target` fell to, or one with no source (a Pokémon at 0 HP
    /// that no `DamageRequest` recorded).
    fn queued_cause(&self, target: PokemonId) -> FaintCause {
        self.state
            .faint_causes
            .iter()
            .find(|cause| cause.target == target)
            .copied()
            .unwrap_or(FaintCause {
                target,
                source: None,
                by_move: false,
                future_move: false,
                listeners_run: false,
                bonds_run: false,
                after_faint_run: false,
                counted: false,
            })
    }

    /// Showdown `faintMessages` taking `cause`'s faint: `side.totalFainted++`, before its `Faint`
    /// event and its ability's `End` — once, wherever the engine takes it first (`counted`): at
    /// the hit loop's end, a knocked-out gas holder is counted by the time the Supreme Overlord
    /// its gas kept back starts again (R-M10-61).
    pub(crate) fn count_faint(&mut self, cause: FaintCause) {
        if self
            .state
            .faint_causes
            .iter()
            .any(|stored| stored.target == cause.target && stored.counted)
        {
            return;
        }
        if let Some(side) = self.side_of(cause.target) {
            let side = self.state.side_mut(side);
            side.total_fainted = side.total_fainted.saturating_add(1).min(100);
        }
        self.mark_cause(cause, |stored| stored.counted = true);
    }

    /// Marks `cause`'s stored entry (`mark`), storing it first where none was.
    fn mark_cause(&mut self, cause: FaintCause, mark: impl Fn(&mut FaintCause)) {
        match self
            .state
            .faint_causes
            .iter_mut()
            .find(|stored| stored.target == cause.target)
        {
            Some(stored) => mark(stored),
            None => {
                let mut stored = cause;
                mark(&mut stored);
                self.state.faint_causes.push(stored);
            }
        }
    }

    /// Showdown `runEvent('AfterFaint', target, source, effect, length)` once `faintMessages`
    /// has emptied the queue: the `onSourceAfterFaint` handlers (Moxie, Beast Boost) of the
    /// Pokémon behind the last faint processed, told how many the batch held.
    pub(crate) fn after_faint(&mut self, cause: FaintCause, length: usize) {
        let Some(source) = cause.source else {
            return;
        };
        let Some(handler) = self.running_ability(source).and_then(|ability| {
            crate::engine::ability_behavior(self.rules, ability).source_after_faint
        }) else {
            return;
        };
        handler(self, source, cause, i8::try_from(length).unwrap_or(i8::MAX));
    }

    /// Showdown `side.foePokemonLeft()` for `pokemon`'s side: a foe team member still standing,
    /// or one whose faint `faintMessages` has not reached yet (`pokemonLeft` drops only there:
    /// once a move's hits are over, `announce_knocked_out` / `announce_faints`, or as the engine
    /// processes it).
    pub fn foe_pokemon_left(&self, pokemon: PokemonId) -> bool {
        let Some(side) = self.side_of(pokemon) else {
            return false;
        };
        self.state.side(side.other()).team.iter().any(|member| {
            !self.is_fainted(*member)
                || (self.state.faint_queue.contains(member)
                    && !self.state.announced_faints.contains(member))
        })
    }

    /// A foreseen strike's hits are over (Showdown's `faintMessages` at the end of
    /// `hitStepMoveHitLoop`): the queued faints count as reached for `foe_pokemon_left`, and in
    /// their side's `totalFainted` (`count_faint`).
    pub fn announce_faints(&mut self) {
        for pokemon in self.state.faint_queue.clone() {
            if !self.state.announced_faints.contains(&pokemon) {
                self.state.announced_faints.push(pokemon);
                let cause = self.queued_cause(pokemon);
                self.count_faint(cause);
            }
        }
    }

    /// Showdown `getBestStat(true, true)` (Beast Boost): the highest stored Attack, Defense,
    /// Special Attack, Special Defense or Speed — no stage, no modifier; a tie goes to the
    /// first in that order.
    pub fn best_stored_stat(&self, pokemon: PokemonId) -> Stat {
        let mut best = (Stat::Attack, 0);
        if let Some(pokemon) = self.state.pokemon(pokemon) {
            for stat in [
                Stat::Attack,
                Stat::Defense,
                Stat::SpecialAttack,
                Stat::SpecialDefense,
                Stat::Speed,
            ] {
                let value = pokemon.battle_stats().get(stat).unwrap_or(0);
                if value > best.1 {
                    best = (stat, value);
                }
            }
        }
        best.0
    }
}
