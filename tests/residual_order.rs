//! L1 rule tests for the end-of-turn order (M8 batch 8, DESIGN.md §12.4, §31.1): Showdown
//! shuffles residual handlers that tie on order, speed and sub-order, and the Pokémon the
//! weather chips at the same speed (`speedSort`), one `SpeedTie` decision each; and Gen 3's
//! Disable, which has no residual order, ticks after everything else. The order only shows in
//! a replay when a faint decides the battle; these pin it directly.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{ResidualKind, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, RandomOverride, RandomRequestKind,
    SideConditionKind, SideState, StatusState, VolatileConditionKind, WeatherKind,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some("Run Away".into());
    set.level = 50;
    set
}

/// Singles, one Pokémon a side (P1 = A, P2 = B), the state prepared before the battle starts.
fn singles(
    generation: Generation,
    [a, b]: [PokemonSet; 2],
    prepare: impl FnOnce(&mut BattleState),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &a).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &b).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(pokemon, [side(A), side(B)], 1);
    prepare(&mut state);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    /// Both sides use their first move (a self-targeted one: no target, no decision).
    fn turn(&mut self, tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choice = |side: SideId, actor: PokemonId| SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(0).unwrap(),
                    target: None,
                    mechanic: None,
                },
            }],
        };
        self.engine
            .submit_turn_choices([choice(SideId::P1, A), choice(SideId::P2, B)])
            .unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(self.engine.state().rng.semantic_tape_exhausted());
        start
    }

    fn residuals(&self, from: usize) -> Vec<(PokemonId, ResidualKind)> {
        self.engine.trace().events()[from..]
            .iter()
            .filter_map(|e| match e {
                TraceEvent::ResidualEffect { holder, kind, .. } => Some((*holder, *kind)),
                _ => None,
            })
            .collect()
    }
}

fn tie(value: u64) -> RandomOverride {
    RandomOverride::raw(RandomRequestKind::SpeedTie, value)
}

fn poison(state: &mut BattleState, id: PokemonId) {
    state.pokemon_mut(id).unwrap().status = Some(StatusState::new(NonVolatileStatus::Poison));
}

#[test]
fn tied_residual_handlers_are_shuffled() {
    // Two equally fast poisoned Pokémon: Showdown's `speedSort` shuffles their poison handlers.
    // The engine draws one SpeedTie per tied handler (A's, then B's); the larger goes first.
    let roster = || [set("Snorlax", &["Harden"]), set("Snorlax", &["Harden"])];
    let poisoned = |state: &mut BattleState| {
        poison(state, A);
        poison(state, B);
    };
    // The action order's own tie comes first.
    let mut b_first = singles(Generation::Gen9, roster(), poisoned);
    let start = b_first.turn(vec![tie(2), tie(1), tie(1), tie(2)]);
    assert_eq!(
        b_first.residuals(start),
        vec![(B, ResidualKind::Poison), (A, ResidualKind::Poison)]
    );
    let mut a_first = singles(Generation::Gen9, roster(), poisoned);
    let start = a_first.turn(vec![tie(2), tie(1), tie(2), tie(1)]);
    assert_eq!(
        a_first.residuals(start),
        vec![(A, ResidualKind::Poison), (B, ResidualKind::Poison)]
    );
}

#[test]
fn the_weather_chips_tied_pokemon_in_a_shuffled_order() {
    // Showdown `eachEvent('Weather')` sorts every active Pokémon by speed and shuffles ties.
    let roster = || [set("Snorlax", &["Harden"]), set("Snorlax", &["Harden"])];
    let sand = |state: &mut BattleState| {
        state.field.set_weather(WeatherKind::Sandstorm, A, 5);
    };
    let mut arena = singles(Generation::Gen9, roster(), sand);
    let start = arena.turn(vec![tie(2), tie(1), tie(1), tie(2)]);
    let chipped: Vec<PokemonId> = arena.engine.trace().events()[start..]
        .iter()
        .filter_map(|e| match e {
            TraceEvent::WeatherDamage { target, .. } => Some(*target),
            _ => None,
        })
        .collect();
    assert_eq!(chipped, vec![B, A]);
}

#[test]
fn gen_3_disable_ticks_after_everything_else() {
    // Gen 3's Disable has no `onResidualOrder` (the gen3 mod drops it) and runs after every
    // numbered handler; in Gen 4 (10.13) the faster Pokémon finishes all its residuals before
    // the slower one's poison (10.6). The slower Pokémon's poison ends the battle: Gen 3 never
    // reaches the faster one's Disable, Gen 4 already did.
    let roster = || {
        [
            set("Jolteon", &["Harden", "Growl"]),
            set("Snorlax", &["Harden"]),
        ]
    };
    let setup = |state: &mut BattleState| {
        let jolteon = state.pokemon_mut(A).unwrap();
        assert!(jolteon.volatiles.set_disable(MoveId::GROWL, 3));
        let snorlax = state.pokemon_mut(B).unwrap();
        snorlax.hp = 1;
        poison(state, B);
    };
    for (generation, left) in [(Generation::Gen3, 3), (Generation::Gen4, 2)] {
        let mut arena = singles(generation, roster(), setup);
        arena.turn(Vec::new());
        let disable = arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .turns_left(VolatileConditionKind::Disable);
        assert_eq!(disable, Some(left), "{generation:?}");
    }
}

/// A count that runs out in the residual leaves a waiting faint waiting (Showdown `fieldEvent`:
/// `handler.end`, then on with no `faintMessages()`): Perish Song takes P1's last Pokémon, the
/// Roost it used ends at 0 HP, and P2's Light Screen still runs out before the faint decides
/// the battle (a Gen 7 Vivillon, R-M10-66).
#[test]
fn a_count_running_out_leaves_a_waiting_faint_waiting() {
    let mut arena = singles(
        Generation::Gen7,
        [set("Vivillon", &["Roost"]), set("Blissey", &["Harden"])],
        |state| {
            let vivillon = state.pokemon_mut(A).unwrap();
            vivillon.hp = vivillon.max_hp / 2;
            assert!(vivillon.volatiles.set_perish_song());
            for _ in 0..3 {
                vivillon.volatiles.tick_perish_song();
            }
            assert!(
                state
                    .side_mut(SideId::P2)
                    .conditions
                    .set_timed(SideConditionKind::LightScreen, 1)
            );
        },
    );
    arena.turn(Vec::new());
    let state = arena.engine.state();
    assert!(state.pokemon(A).unwrap().is_fainted());
    assert!(state.outcome.is_some());
    assert!(
        !state
            .side(SideId::P2)
            .conditions
            .contains(SideConditionKind::LightScreen)
    );
}

/// The same with the count on a Pokémon still standing: P2's Roost ends after Perish Song took
/// P1's last Pokémon, and the faint waits for the Light Screen (R-M10-66).
#[test]
fn a_standing_pokemons_count_running_out_leaves_the_faint_waiting() {
    let mut arena = singles(
        Generation::Gen7,
        [set("Vivillon", &["Harden"]), set("Blissey", &["Roost"])],
        |state| {
            let vivillon = state.pokemon_mut(A).unwrap();
            assert!(vivillon.volatiles.set_perish_song());
            for _ in 0..3 {
                vivillon.volatiles.tick_perish_song();
            }
            let blissey = state.pokemon_mut(B).unwrap();
            blissey.hp = blissey.max_hp / 2;
            assert!(
                state
                    .side_mut(SideId::P2)
                    .conditions
                    .set_timed(SideConditionKind::LightScreen, 1)
            );
        },
    );
    arena.turn(Vec::new());
    let state = arena.engine.state();
    assert!(state.pokemon(A).unwrap().is_fainted());
    assert!(state.outcome.is_some());
    assert!(
        !state
            .side(SideId::P2)
            .conditions
            .contains(SideConditionKind::LightScreen)
    );
}

/// `timed_turns_left` reads the count `tick_timed` decrements — a condition with 1 left is one
/// the next residual ends (`count_runs_out`, R-M10-66) — and none for one with no count.
#[test]
fn a_timed_condition_has_the_turns_tick_timed_counts_down() {
    let mut arena = singles(
        Generation::Gen7,
        [set("Vivillon", &["Harden"]), set("Blissey", &["Harden"])],
        |state| {
            let vivillon = state.pokemon_mut(A).unwrap();
            assert!(vivillon.volatiles.set_embargo(2));
            assert!(vivillon.volatiles.set_heal_block(3));
            assert!(vivillon.volatiles.set_magnet_rise(1));
            assert!(vivillon.volatiles.set_micle_berry(Some(2)));
        },
    );
    let kinds = [
        VolatileConditionKind::Embargo,
        VolatileConditionKind::HealBlock,
        VolatileConditionKind::MagnetRise,
        VolatileConditionKind::MicleBerry,
    ];
    let left = |arena: &Arena| {
        let volatiles = &arena.engine.state().pokemon(A).unwrap().volatiles;
        kinds.map(|kind| volatiles.timed_turns_left(kind))
    };
    assert_eq!(left(&arena), [Some(2), Some(3), Some(1), Some(2)]);
    arena.turn(Vec::new());
    assert_eq!(left(&arena), [Some(1), Some(2), None, Some(1)]);
    assert_eq!(
        arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .timed_turns_left(VolatileConditionKind::Roost),
        None
    );
}
