//! L1 rule tests for Metronome, the item (DESIGN.md R126; issues.md I21): how the
//! holder's count moves — Gen 4 as the cartridge counts, Gen 5+ as Showdown does — and what it
//! multiplies. A replay sees neither the count nor, within a damage roll, the boost's rounding.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, ModifierSource, ModifierStage, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, MoveResult, NonVolatileStatus, RandomOverride, RandomRequestKind,
    SideState, StatusState, VolatileConditionKind,
};

const HOLDER: PokemonId = PokemonId(1);
const WALL: PokemonId = PokemonId(2);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, level: u8, item: Option<&str>, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = level;
    set.ability = Some("Run Away".into());
    set.item = item.map(Into::into);
    set
}

/// A level-60 Mew holding Metronome, with `moves`, against a level-100 Blissey that Hardens.
fn arena(generation: Generation, moves: &[&str]) -> Arena {
    arena_against(generation, moves, "Harden")
}

/// The same against a Blissey that uses `wall_move`.
fn arena_against(generation: Generation, moves: &[&str], wall_move: &str) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(
            &dex,
            HOLDER,
            SideId::P1,
            &set("Mew", 60, Some("Metronome"), moves),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            WALL,
            SideId::P2,
            &set("Blissey", 100, None, &[wall_move]),
        )
        .unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(HOLDER), side(WALL)], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

impl Arena {
    fn count(&self) -> Option<(Option<MoveId>, u8)> {
        self.engine
            .state()
            .pokemon(HOLDER)
            .unwrap()
            .volatiles
            .metronome()
    }

    /// One turn: the holder uses its move in `slot`, Blissey Hardens; `edit` changes the state
    /// first (the count, a status), `tape` fixes the draws. What the turn traced.
    fn turn(
        &mut self,
        slot: u8,
        tape: Vec<RandomOverride>,
        edit: impl FnOnce(&mut BattleState),
    ) -> Vec<TraceEvent> {
        let mut snapshot = self.engine.snapshot();
        edit(&mut snapshot.state);
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let move_id = self.engine.state().pokemon(HOLDER).unwrap().moves[usize::from(slot)].move_id;
        let target = match self.dex.move_by_id(move_id).unwrap().target {
            TargetPattern::SelectedAdjacent | TargetPattern::SelectedAny => Some(DeclaredTarget(
                self.rules.topology().position(SideId::P2, 0).unwrap(),
            )),
            _ => None,
        };
        let choice = |side, actor, slot, target| SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        };
        self.engine
            .submit_turn_choices([
                choice(SideId::P1, HOLDER, slot, target),
                choice(SideId::P2, WALL, 0, None),
            ])
            .unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        self.engine.trace().events()[start..].to_vec()
    }
}

/// The holder's count set to `count` on `move_id`.
fn counted(move_id: MoveId, count: u8) -> impl FnOnce(&mut BattleState) {
    move |state| {
        state
            .pokemon_mut(HOLDER)
            .unwrap()
            .volatiles
            .set_metronome(Some(move_id), count);
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

/// The Metronome step of the Gen 4 damage: (from, to).
fn metronome_step(events: &[TraceEvent]) -> Option<(u32, u32)> {
    events.iter().find_map(|event| match event {
        TraceEvent::ModifierApplied {
            stage: ModifierStage::FinalDamage,
            source: ModifierSource::Volatile(VolatileConditionKind::Metronome),
            from,
            to,
            ..
        } => Some((*from, *to)),
        _ => None,
    })
}

#[test]
fn the_item_starts_the_count_as_its_holder_enters() {
    let arena = arena(Generation::Gen4, &["Swift"]);
    assert_eq!(arena.count(), Some((None, 0)));
}

#[test]
fn gen_4_counts_to_ten_and_multiplies_by_tenths_before_the_roll() {
    // Swift from 9 in a row: 10, `× 20 / 10`; then it stays at 10 (`metronomeTurns < 10`).
    let mut arena = arena(Generation::Gen4, &["Swift"]);
    let events = arena.turn(0, hit(100), counted(MoveId::SWIFT, 9));
    assert_eq!(arena.count(), Some((Some(MoveId::SWIFT), 10)));
    let (from, to) = metronome_step(&events).unwrap();
    assert_eq!(to, from * 20 / 10);
    arena.turn(0, hit(100), |_| {});
    assert_eq!(arena.count(), Some((Some(MoveId::SWIFT), 10)));
    // From 4: × 15 / 10, floored (Showdown's `damage * 1.5` float is floored with the rest of
    // the phase, and never caps).
    let events = arena.turn(0, hit(100), counted(MoveId::SWIFT, 4));
    let (from, to) = metronome_step(&events).unwrap();
    assert_eq!(to, from * 15 / 10);
}

#[test]
fn gen_4_another_move_starts_the_count_over() {
    let mut arena = arena(Generation::Gen4, &["Swift", "Tackle"]);
    arena.turn(
        1,
        [
            vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)],
            hit(100),
        ]
        .concat(),
        counted(MoveId::SWIFT, 6),
    );
    assert_eq!(arena.count(), Some((Some(MoveId::TACKLE), 0)));
}

#[test]
fn gen_4_a_move_that_did_nothing_takes_one_step_back() {
    // Tackle from 5: it counts 6 as it starts, misses, and the count is 5 again (Showdown would
    // start the next Tackle over).
    let mut arena = arena(Generation::Gen4, &["Tackle"]);
    arena.turn(
        0,
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)],
        counted(MoveId::TACKLE, 5),
    );
    assert_eq!(arena.count(), Some((Some(MoveId::TACKLE), 5)));
}

#[test]
fn gen_4_a_lost_turn_takes_one_step_back_but_not_sleep() {
    // Fully paralysed with Tackle chosen: 5 → 4 (`MOVE_STATUS_NO_MORE_WORK`).
    let mut arena = arena(Generation::Gen4, &["Tackle"]);
    arena.turn(
        0,
        vec![RandomOverride::bool(RandomRequestKind::FullParalysis, true)],
        |state| {
            counted(MoveId::TACKLE, 5)(state);
            state.pokemon_mut(HOLDER).unwrap().status =
                Some(StatusState::new(NonVolatileStatus::Paralysis));
        },
    );
    assert_eq!(arena.count(), Some((Some(MoveId::TACKLE), 4)));
    // Asleep: the turn is lost to the sleep script, and the count stays.
    let mut arena = self::arena(Generation::Gen4, &["Tackle"]);
    arena.turn(0, vec![], |state| {
        counted(MoveId::TACKLE, 5)(state);
        let mut sleep = StatusState::new(NonVolatileStatus::Sleep);
        sleep.sleep_turns_remaining = Some(3);
        state.pokemon_mut(HOLDER).unwrap().status = Some(sleep);
    });
    assert_eq!(arena.count(), Some((Some(MoveId::TACKLE), 5)));
}

#[test]
fn gen_4_the_count_drops_to_0_while_embargo_sets_the_item_aside() {
    // `Battler_HeldItemEffect` names no Metronome under Embargo: Swift from 6 counts 0, the
    // counted move kept, and the next Swift once Embargo is over counts on from there.
    let mut arena = arena(Generation::Gen4, &["Swift"]);
    arena.turn(0, hit(100), |state| {
        counted(MoveId::SWIFT, 6)(state);
        state.pokemon_mut(HOLDER).unwrap().volatiles.set_embargo(3);
    });
    assert_eq!(arena.count(), Some((Some(MoveId::SWIFT), 0)));
    arena.turn(0, hit(100), |state| {
        state
            .pokemon_mut(HOLDER)
            .unwrap()
            .volatiles
            .remove(VolatileConditionKind::Embargo);
    });
    assert_eq!(arena.count(), Some((Some(MoveId::SWIFT), 1)));
}

#[test]
fn gen_5_plus_an_item_set_aside_ends_the_condition() {
    // `hasItem('metronome')` is false under Embargo (`ignoringItem()`): the condition ends as
    // the next move is tried.
    let mut arena = arena(Generation::Gen7, &["Swift"]);
    arena.turn(0, hit(100), |state| {
        counted(MoveId::SWIFT, 3)(state);
        state.pokemon_mut(HOLDER).unwrap().volatiles.set_embargo(3);
    });
    assert_eq!(arena.count(), None);
}

#[test]
fn gen_9_counts_what_succeeded_and_chains_at_most_double() {
    // Swift from 5 after a successful Swift: 6, and the chain takes [8192, 4096] (min(6, 5)).
    let mut arena = arena(Generation::Gen9, &["Swift"]);
    let events = arena.turn(0, hit(100), |state| {
        counted(MoveId::SWIFT, 5)(state);
        state.pokemon_mut(HOLDER).unwrap().history.move_last_turn = Some(MoveResult::Succeeded);
    });
    assert_eq!(arena.count(), Some((Some(MoveId::SWIFT), 6)));
    let chain = events
        .iter()
        .find_map(|event| match event {
            TraceEvent::ModifierApplied {
                stage: ModifierStage::FinalDamage,
                source: ModifierSource::Rule("ModifyDamage"),
                from,
                to,
                ..
            } => Some((*from, *to)),
            _ => None,
        })
        .unwrap();
    assert_eq!(chain.1, chain.0 * 2);
    // After a Swift that did not succeed (`moveLastTurnResult` not true): back to 0.
    let mut arena = self::arena(Generation::Gen9, &["Swift"]);
    arena.turn(0, hit(100), |state| {
        counted(MoveId::SWIFT, 5)(state);
        state.pokemon_mut(HOLDER).unwrap().history.move_last_turn = Some(MoveResult::Neither);
    });
    assert_eq!(arena.count(), Some((Some(MoveId::SWIFT), 0)));
}

#[test]
fn gen_4_a_two_turn_move_counts_on_its_charging_turn() {
    // Solar Beam from 3: the charging turn counts 4 (`BattleSystem_UpdateMetronomeCount` runs at
    // the end of the BeforeMove checks), the attacking turn (`SYSCTL_LAST_OF_MULTI_TURN`) leaves
    // it and hits × 14 / 10.
    let mut arena = arena(Generation::Gen4, &["Solar Beam"]);
    arena.turn(0, vec![], counted(MoveId::SOLAR_BEAM, 3));
    assert_eq!(arena.count(), Some((Some(MoveId::SOLAR_BEAM), 4)));
    let events = arena.turn(0, hit(100), |_| {});
    assert_eq!(arena.count(), Some((Some(MoveId::SOLAR_BEAM), 4)));
    let (from, to) = metronome_step(&events).unwrap();
    assert_eq!(to, from * 14 / 10);
}

#[test]
fn gen_4_a_move_into_a_protect_takes_one_step_back() {
    // `MOVE_STATUS_PROTECTED` is one of the no-effect flags: Swift from 5 counts 6, meets the
    // Protect and is at 5 again (Showdown's `null` would start the next Swift over).
    let mut arena = arena_against(Generation::Gen4, &["Swift"], "Protect");
    arena.turn(0, vec![], counted(MoveId::SWIFT, 5));
    assert_eq!(arena.count(), Some((Some(MoveId::SWIFT), 5)));
}
