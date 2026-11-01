//! L1 rule tests for M8 batch 8 (DESIGN.md §12.4): fractional priority and Quick Claw — the
//! parts a Showdown replay cannot see. The replay plans every priority chance and speed tie
//! from what the engine says it will draw, so a chance drawn where Showdown draws none, or a
//! speed tie drawn across two fractional brackets, replays the same; and the residual order
//! only shows in Showdown's log, not in the frames the replay compares
//! (`tests/fixtures/cases/gen{2,3}-quick-claw-turn-roll`, `gen{4,5}-doubles-lagging-tail-…`,
//! `gen9-quick-claw-quick-draw-stall` cover the rest).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, ResidualKind, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], ability: &str, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

/// Singles, one Pokémon a side: P1 = A, P2 = B.
fn singles(
    generation: Generation,
    [a, b]: [PokemonSet; 2],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &a).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &b).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(A), side(B)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn choice(&self, side: SideId, slot: usize) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
        let move_id = self.engine.state().pokemon(actor).unwrap().moves[slot].move_id;
        let target = match self.dex.move_by_id(move_id).unwrap().target {
            TargetPattern::SelectedAny
            | TargetPattern::SelectedAdjacent
            | TargetPattern::SelectedFoe
            | TargetPattern::SelectedAdjacentFoe => Some(DeclaredTarget(
                self.rules.topology().position(side.other(), 0).unwrap(),
            )),
            _ => None,
        };
        SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot as u8).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        }
    }

    /// Runs one turn on `tape`; returns where its trace starts and whether the tape was used up.
    fn turn(&mut self, p1: usize, p2: usize, tape: Vec<RandomOverride>) -> (usize, bool) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        (start, self.engine.state().rng.semantic_tape_exhausted())
    }

    fn movers(&self, from: usize) -> Vec<PokemonId> {
        self.engine.trace().events()[from..]
            .iter()
            .filter_map(|e| match e {
                TraceEvent::MoveUsed { actor, .. } => Some(*actor),
                _ => None,
            })
            .collect()
    }

    fn residuals(&self, from: usize) -> Vec<PokemonId> {
        self.engine.trace().events()[from..]
            .iter()
            .filter_map(|e| match e {
                TraceEvent::ResidualEffect {
                    holder,
                    kind: ResidualKind::Poison,
                    ..
                } => Some(*holder),
                _ => None,
            })
            .collect()
    }
}

fn chance(value: bool) -> RandomOverride {
    RandomOverride::bool(RandomRequestKind::PriorityChance, value)
}

#[test]
fn gen_3_quick_claw_speed_holds_for_the_whole_turn() {
    // Showdown's Gen 3 `getActionSpeed` returns 65535 for a Quick Claw holder in a turn whose
    // roll succeeded, and `updateSpeed` runs it again before the residual: the slow holder
    // moves first and is poisoned first (a probe with both burned: `|move|p1a: Snorlax` …
    // `|-damage|p1a: Snorlax|…|[from] brn` before Jolteon's on the turns Snorlax led).
    let roster = || {
        [
            set("Snorlax", &["Growl"], "Thick Fat", Some("Quick Claw")),
            set("Jolteon", &["Growl"], "Volt Absorb", None),
        ]
    };
    let poisoned = |pokemon: &mut Vec<PokemonState>| {
        for p in pokemon.iter_mut() {
            p.status = Some(StatusState::new(NonVolatileStatus::Poison));
        }
    };
    let mut rolled = singles(Generation::Gen3, roster(), poisoned);
    let (start, used) = rolled.turn(0, 0, vec![chance(true)]);
    assert!(used);
    assert_eq!(rolled.movers(start), vec![A, B]);
    assert_eq!(rolled.residuals(start), vec![A, B]);

    let mut missed = singles(Generation::Gen3, roster(), poisoned);
    let (start, used) = missed.turn(0, 0, vec![chance(false)]);
    assert!(used);
    assert_eq!(missed.movers(start), vec![B, A]);
    assert_eq!(missed.residuals(start), vec![B, A]);
}

#[test]
fn quick_draw_only_rolls_for_a_damaging_move() {
    // Quick Draw's `onFractionalPriority` checks `move.category !== "Status"` before its
    // `randomChance(3, 10)`: a status move draws nothing, so the planned success is left over.
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Slowbro-Galar", &["Growl"], "Quick Draw", None),
            set("Jolteon", &["Growl"], "Volt Absorb", None),
        ],
        |_| {},
    );
    let (start, used) = arena.turn(0, 0, vec![chance(true)]);
    assert!(!used, "a status move rolled Quick Draw");
    assert_eq!(arena.movers(start), vec![B, A]);
    assert!(
        !arena.engine.trace().events()[start..]
            .iter()
            .any(|e| matches!(e, TraceEvent::AbilityActivated { .. }))
    );
}

#[test]
fn a_fractional_priority_apart_is_no_speed_tie() {
    // Showdown sorts on `priority + fractionalPriority` before Speed: two equally fast
    // Pokémon, one holding Lagging Tail, sit in different brackets and draw no speed tie.
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Jolteon", &["Growl"], "Volt Absorb", Some("Lagging Tail")),
            set("Jolteon", &["Growl"], "Volt Absorb", None),
        ],
        |_| {},
    );
    let (start, used) = arena.turn(0, 0, Vec::new());
    assert!(used);
    assert_eq!(arena.movers(start), vec![B, A]);
    let orders: Vec<(i8, u64)> = arena.engine.trace().events()[start..]
        .iter()
        .filter_map(|e| match e {
            TraceEvent::ActionOrderResolved {
                fractional_priority,
                tie_break,
                ..
            } => Some((*fractional_priority, *tie_break)),
            _ => None,
        })
        .collect();
    // The turn's order, then the Gen 8+ re-sorts after each action: never a tie break.
    assert_eq!(orders[..2], [(-1, 0), (0, 0)]);
    assert!(orders.iter().all(|(_, tie_break)| *tie_break == 0));
}
