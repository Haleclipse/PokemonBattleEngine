//! L1 rule tests for M7 batch 3 (DESIGN.md §22.8): the event history — what happened to and by
//! each Pokémon this turn and the last, each side's faints, the turn's last successful move —
//! with Showdown's set and reset points, and the power formulas that read it.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, MoveCategory, PokemonSet, ResolvedDex, ResolvedMove, build_pokemon,
};
use pokemon_battle_engine::effects::moves::PowerCondition;
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, ForcedReplacement, ForcedSwitchChoice, MoveUse, SideChoice,
    SlotChoice, formula_power, power_condition_holds,
};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    AttackRecord, BattleRng, BattleState, MoveResult, PokemonHistory, RandomOverride,
    RandomRequestKind, SideState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);
const D: PokemonId = PokemonId(4);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set
}

fn filler() -> PokemonSet {
    set("Snorlax", &["Swords Dance"])
}

/// A (P1) and B (P2) lead; C and D wait on the bench.
fn singles(
    generation: Generation,
    [a, b, c, d]: [PokemonSet; 4],
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
        build_pokemon(&dex, C, SideId::P1, &c).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &d).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let mut state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B, D])], 1);
    prepare(&mut state);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_switch_in(A);
    engine.run_switch_in(B);
    Arena { rules, dex, engine }
}

impl Arena {
    fn state(&self) -> &BattleState {
        self.engine.state()
    }

    fn history(&self, id: PokemonId) -> &PokemonHistory {
        &self.state().pokemon(id).unwrap().history
    }

    fn move_def(&self, id: PokemonId, slot: usize) -> ResolvedMove {
        let move_id = self.state().pokemon(id).unwrap().moves[slot].move_id;
        self.dex.move_by_id(move_id).unwrap().clone()
    }

    /// Both sides use a move (by slot), under `tape`; returns before any action ran.
    fn choose(&mut self, slots: [usize; 2], tape: Vec<RandomOverride>) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let choice = |arena: &Arena, side: SideId, slot: usize| {
            let actor = arena.state().side(side).active[0].unwrap();
            let move_def = arena.move_def(actor, slot);
            let target = matches!(move_def.target, TargetPattern::SelectedAdjacent)
                .then(|| DeclaredTarget(arena.rules.topology().position(side.other(), 0).unwrap()));
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
        };
        let choices = [
            choice(self, SideId::P1, slots[0]),
            choice(self, SideId::P2, slots[1]),
        ];
        self.engine.submit_turn_choices(choices).unwrap();
    }

    /// Runs to the next request.
    fn finish(&mut self) -> EngineStep {
        loop {
            let step = self.engine.step().unwrap();
            if !matches!(step, EngineStep::Progressed { .. }) {
                assert!(
                    self.state().rng.semantic_tape_exhausted(),
                    "tape not fully consumed"
                );
                return step;
            }
        }
    }

    fn turn(&mut self, slots: [usize; 2], tape: Vec<RandomOverride>) -> EngineStep {
        self.choose(slots, tape);
        self.finish()
    }

    fn power(&self, attacker: PokemonId, target: PokemonId, slot: usize) -> u16 {
        let move_def = self.move_def(attacker, slot);
        let formula = move_def.behavior.power_formula.expect("a power formula");
        formula_power(
            self.state(),
            &self.rules,
            &self.dex,
            attacker,
            target,
            &move_def,
            formula,
            MoveUse::FIRST,
        )
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn accurate(hit_it: bool) -> RandomOverride {
    RandomOverride::bool(RandomRequestKind::Accuracy, hit_it)
}

#[test]
fn the_history_starts_over_when_a_pokemon_leaves_but_rage_fist_remembers() {
    let mut history = PokemonHistory::default();
    assert!(history.newly_switched, "built off the field");
    history.newly_switched = false;
    history.move_this_turn = Some(MoveResult::Failed);
    history.stats_lowered_this_turn = true;
    history.hurt_this_turn = true;
    history.attacked_this_turn.push(AttackRecord {
        source: B,
        damage: Some(10),
        move_id: MoveId::TACKLE,
        position: None,
        category: MoveCategory::Physical,
        damaging_hit_only: false,
    });
    history.times_attacked = 3;
    assert!(history.damaged_this_turn_by(B));
    assert!(!history.damaged_this_turn_by(C));
    // A Gen 5+ Bide release only answers Counter / Mirror Coat: it is no damaging attack here.
    history.attacked_this_turn.push(AttackRecord {
        source: C,
        damage: Some(10),
        move_id: MoveId::BIDE,
        position: None,
        category: MoveCategory::Physical,
        damaging_hit_only: true,
    });
    assert!(!history.damaged_this_turn_by(C));

    // Across turns, each attacker's latest record; the boundary drops an attacker no longer
    // on the field (Showdown `attackedBy.splice`), and the last record left is the one Gen 3-4
    // Mirror Move reads.
    for source in [B, C] {
        history.attacked_by.push(AttackRecord {
            source,
            damage: Some(10),
            move_id: MoveId::TACKLE,
            position: None,
            category: MoveCategory::Physical,
            damaging_hit_only: false,
        });
    }

    // The turn boundary: this turn's result becomes last turn's, the turn's flags clear.
    let mut aged = history.clone();
    aged.next_turn(&[A, B]);
    assert_eq!(
        aged.attacked_by
            .iter()
            .map(|r| r.source)
            .collect::<Vec<_>>(),
        vec![B],
        "C left the field"
    );
    assert_eq!(aged.move_last_turn, Some(MoveResult::Failed));
    assert_eq!(aged.move_this_turn, None);
    assert!(!aged.stats_lowered_this_turn && !aged.hurt_this_turn);
    assert!(aged.attacked_this_turn.is_empty());
    assert_eq!(aged.times_attacked, 3);

    // Leaving the field: everything but the hit count starts over, newly switched again.
    history.leave_field();
    assert_eq!(
        history,
        PokemonHistory {
            times_attacked: 3,
            ..PokemonHistory::default()
        }
    );

    // A status move reaching the Pokémon is an attack, not damage (Revenge ignores it).
    let mut status = PokemonHistory::default();
    status.attacked_this_turn.push(AttackRecord {
        source: B,
        damage: None,
        move_id: MoveId::GROWL,
        position: None,
        category: MoveCategory::Status,
        damaging_hit_only: false,
    });
    assert!(!status.damaged_this_turn_by(B));
}

#[test]
fn move_results_follow_showdowns_classification() {
    // Skarmory (90 Spe at L50) moves before Camerupt (60).
    let mut arena = singles(
        Generation::Gen9,
        [
            set(
                "Camerupt",
                &["Temper Flare", "Thunder Wave", "Moonlight", "Hyper Beam"],
            ),
            set(
                "Skarmory",
                &["Protect", "Substitute", "Recover", "Swords Dance"],
            ),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let last = |arena: &Arena, id| arena.history(id).move_last_turn;

    // Moonlight at full HP reports `NOT_FAIL`, which Showdown's hit loop counts as a hit;
    // Recover at full HP fails.
    arena.turn([2, 2], vec![]);
    assert_eq!(last(&arena, A), Some(MoveResult::Succeeded));
    assert_eq!(last(&arena, B), Some(MoveResult::Failed));

    // A move every target protected against is neither.
    arena.turn([0, 0], vec![]);
    assert_eq!(last(&arena, A), Some(MoveResult::Neither));
    assert_eq!(last(&arena, B), Some(MoveResult::Succeeded));

    // A status move stopped by a Substitute — after its accuracy check — counts as a hit.
    arena.turn([1, 1], vec![accurate(true)]);
    assert_eq!(last(&arena, A), Some(MoveResult::Succeeded));

    // Hyper Beam lands (on the Substitute); the recharge turn is neither.
    let mut tape = vec![accurate(true)];
    tape.extend(hit(100));
    arena.turn([3, 3], tape);
    assert_eq!(last(&arena, A), Some(MoveResult::Succeeded));
    arena.turn([3, 3], vec![]);
    assert_eq!(last(&arena, A), Some(MoveResult::Neither));

    // A miss is a failure.
    arena.turn([1, 3], vec![accurate(false)]);
    assert_eq!(last(&arena, A), Some(MoveResult::Failed));
}

#[test]
fn hits_and_their_sources_are_recorded_for_the_turn() {
    // Jolteon (L50, 150 Spe) puts up a Substitute before Snorlax's Tackle.
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Jolteon", &["Substitute", "Tackle"]),
            set("Snorlax", &["Tackle"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    arena.choose([0, 0], hit(100));
    arena.engine.step().unwrap();
    // Paying for a Substitute is a direct cost, not a hurt (Showdown `directDamage`).
    assert!(!arena.history(A).hurt_this_turn);
    arena.engine.step().unwrap();
    // A hit on the Substitute is not recorded: neither hurt nor attacked.
    assert!(!arena.history(A).hurt_this_turn);
    assert!(arena.history(A).attacked_this_turn.is_empty());
    assert_eq!(arena.history(A).times_attacked, 0);
    arena.finish();

    // Jolteon's own Tackle reaches Snorlax's HP: hurt, attacked by Jolteon, one hit.
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Jolteon", &["Tackle"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    arena.choose([0, 0], hit(100));
    arena.engine.step().unwrap();
    let history = arena.history(B);
    assert!(history.hurt_this_turn);
    assert!(history.damaged_this_turn_by(A));
    assert_eq!(history.times_attacked, 1);
    arena.finish();
    // The boundary clears the turn's record; the hit count stays.
    let history = arena.history(B);
    assert!(!history.hurt_this_turn && history.attacked_this_turn.is_empty());
    assert_eq!(history.times_attacked, 1);
}

#[test]
fn a_lead_intimidate_still_counts_on_turn_one() {
    // Showdown's start-of-battle boundary clears `newlySwitched` but keeps
    // `statsLoweredThisTurn` (it runs with turn 1); the next boundary clears it.
    let mut gyarados = set("Gyarados", &["Swords Dance"]);
    gyarados.ability = Some("Intimidate".into());
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Grimmsnarl", &["Lash Out", "Swords Dance"]),
            gyarados,
            filler(),
            filler(),
        ],
        |_| {},
    );
    assert!(arena.history(A).stats_lowered_this_turn);
    assert!(!arena.history(A).newly_switched && !arena.history(B).newly_switched);
    let lash_out = arena.move_def(A, 0).behavior.power_boost.unwrap();
    assert!(power_condition_holds(
        arena.state(),
        &arena.rules,
        A,
        B,
        lash_out.condition,
        false
    ));

    arena.turn([1, 0], vec![]);
    assert!(!arena.history(A).stats_lowered_this_turn);
}

#[test]
fn faints_age_into_last_turn_and_add_up() {
    // Snorlax knocks out B (1 HP); P2 sends D. At the boundary P2's faint becomes last turn's;
    // a turn later it is gone, the total stays.
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Snorlax", &["Tackle"]),
            set("Pichu", &["Swords Dance"]),
            filler(),
            set("Tauros", &["Retaliate", "Last Respects"]),
        ],
        |state| state.pokemon_mut(B).unwrap().hp = 1,
    );
    let step = arena.turn([0, 0], hit(100));
    assert!(matches!(step, EngineStep::Awaiting(_)));
    let position = arena.rules.topology().position(SideId::P2, 0).unwrap();
    arena
        .engine
        .resolve_forced_switches([
            ForcedSwitchChoice {
                side: SideId::P1,
                replacements: vec![],
            },
            ForcedSwitchChoice {
                side: SideId::P2,
                replacements: vec![ForcedReplacement {
                    position,
                    pokemon: D,
                }],
            },
        ])
        .unwrap();
    let p2 = arena.state().side(SideId::P2);
    assert!(p2.fainted_last_turn && !p2.fainted_this_turn);
    assert_eq!(p2.total_fainted, 1);
    let retaliate = arena.move_def(D, 0).behavior.power_boost.unwrap();
    assert!(power_condition_holds(
        arena.state(),
        &arena.rules,
        D,
        A,
        retaliate.condition,
        false
    ));
    assert_eq!(arena.power(D, A, 1), 100, "Last Respects: 50 + 50 × 1");
    // The newcomer entered after the turn's moves: the boundary already made it an old hand.
    assert!(!arena.history(D).newly_switched);

    arena.turn([0, 0], hit(100));
    let p2 = arena.state().side(SideId::P2);
    assert!(!p2.fainted_last_turn);
    assert_eq!(p2.total_fainted, 1);
}

#[test]
fn payback_counts_a_newcomer_as_moved_only_in_gen_4() {
    // B came in this turn and has no move left to make. (Leads are settled at construction,
    // so the newcomer flag goes on afterwards.)
    let power = |generation| {
        let mut arena = singles(
            generation,
            [set("Umbreon", &["Payback"]), filler(), filler(), filler()],
            |_| {},
        );
        let mut snapshot = arena.engine.snapshot();
        snapshot
            .state
            .pokemon_mut(B)
            .unwrap()
            .history
            .newly_switched = true;
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
        arena.power(A, B, 0)
    };
    assert_eq!(power(Generation::Gen4), 100);
    assert_eq!(power(Generation::Gen9), 50);
}

#[test]
fn history_formulas_read_their_record() {
    let mut arena = singles(
        Generation::Gen9,
        [
            set(
                "Annihilape",
                &["Rage Fist", "Stomping Tantrum", "Assurance", "Avalanche"],
            ),
            set("Snorlax", &["Last Respects", "Fusion Bolt"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let with = |arena: &mut Arena, edit: &dyn Fn(&mut BattleState)| {
        let mut snapshot = arena.engine.snapshot();
        edit(&mut snapshot.state);
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    };

    // Rage Fist: 50 + 50 per hit taken, at most 350.
    assert_eq!(arena.power(A, B, 0), 50);
    with(&mut arena, &|s| {
        s.pokemon_mut(A).unwrap().history.times_attacked = 3
    });
    assert_eq!(arena.power(A, B, 0), 200);
    with(&mut arena, &|s| {
        s.pokemon_mut(A).unwrap().history.times_attacked = 9
    });
    assert_eq!(arena.power(A, B, 0), 350);

    // Stomping Tantrum doubles only after a failure (not after "neither").
    with(&mut arena, &|s| {
        s.pokemon_mut(A).unwrap().history.move_last_turn = Some(MoveResult::Neither)
    });
    assert_eq!(arena.power(A, B, 1), 75);
    with(&mut arena, &|s| {
        s.pokemon_mut(A).unwrap().history.move_last_turn = Some(MoveResult::Failed)
    });
    assert_eq!(arena.power(A, B, 1), 150);

    // Assurance: the target lost HP this turn. Avalanche: the target damaged the user.
    assert_eq!(arena.power(A, B, 2), 60);
    with(&mut arena, &|s| {
        s.pokemon_mut(B).unwrap().history.hurt_this_turn = true
    });
    assert_eq!(arena.power(A, B, 2), 120);
    assert_eq!(arena.power(A, B, 3), 60);
    with(&mut arena, &|s| {
        s.pokemon_mut(A)
            .unwrap()
            .history
            .attacked_this_turn
            .push(AttackRecord {
                source: B,
                damage: Some(20),
                move_id: MoveId::TACKLE,
                position: None,
                category: MoveCategory::Physical,
                damaging_hit_only: false,
            })
    });
    assert_eq!(arena.power(A, B, 3), 120);

    // Last Respects: 50 per faint on the user's side.
    with(&mut arena, &|s| s.side_mut(SideId::P2).total_fainted = 3);
    assert_eq!(arena.power(B, A, 0), 200);

    // Fusion Bolt: the turn's last successful move was Fusion Flare.
    let bolt = arena.move_def(B, 1).behavior.power_boost.unwrap();
    assert_eq!(
        bolt.condition,
        PowerCondition::AfterMove(MoveId::FUSION_FLARE)
    );
    assert!(!power_condition_holds(
        arena.state(),
        &arena.rules,
        B,
        A,
        bolt.condition,
        false
    ));
    with(&mut arena, &|s| {
        s.last_successful_move = Some(MoveId::FUSION_FLARE)
    });
    assert!(power_condition_holds(
        arena.state(),
        &arena.rules,
        B,
        A,
        bolt.condition,
        false
    ));
}

#[test]
fn sets_carry_showdowns_three_pp_ups() {
    let pp = |generation, species: &str, name: &str| {
        let dex = DexResolver::resolve(generation);
        build_pokemon(&dex, A, SideId::P1, &set(species, &[name]))
            .unwrap()
            .moves[0]
            .pp
    };
    // `pp × 8 / 5`; before Gen 3 a 40-PP move stops at 61.
    assert_eq!(pp(Generation::Gen9, "Snorlax", "Recover"), 8);
    assert_eq!(pp(Generation::Gen9, "Snorlax", "Tackle"), 56);
    assert_eq!(pp(Generation::Gen1, "Snorlax", "Growl"), 61);
    assert_eq!(pp(Generation::Gen3, "Snorlax", "Growl"), 64);
}

/// Pollen Puff from P1's lead on its full-HP partner, everyone else using Swords Dance (no
/// speed ties: Ribombee, Blissey, Chansey, Snorlax): the user's result at the end of the turn.
fn pollen_puff_on_a_full_hp_ally(generation: Generation) -> Option<MoveResult> {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let sets = [
        set("Ribombee", &["Pollen Puff"]),
        set("Chansey", &["Swords Dance"]),
        set("Snorlax", &["Swords Dance"]),
        set("Blissey", &["Swords Dance"]),
    ];
    let pokemon = sets
        .iter()
        .enumerate()
        .map(|(index, set)| {
            let side = if index < 2 { SideId::P1 } else { SideId::P2 };
            build_pokemon(&dex, PokemonId(index as u8 + 1), side, set).unwrap()
        })
        .collect();
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 2);
        side.active = vec![Some(team[0]), Some(team[1])];
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, B]), side(vec![C, D])], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex, state).unwrap();
    for id in [A, B, C, D] {
        engine.run_switch_in(id);
    }
    let slot = |actor, target| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target,
            mechanic: None,
        },
    };
    let partner = DeclaredTarget(rules.topology().position(SideId::P1, 1).unwrap());
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![slot(A, Some(partner)), slot(B, None)],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![slot(C, None), slot(D, None)],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    engine.state().pokemon(A).unwrap().history.move_last_turn
}

#[test]
fn a_heal_in_place_of_damage_at_full_hp_follows_showdowns_classification() {
    // Present drawing its heal against a full-HP target fails (`-fail|target|heal`).
    let mut arena = singles(
        Generation::Gen9,
        [
            set("Delibird", &["Present"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    arena.turn(
        [0, 0],
        vec![
            RandomOverride::range(RandomRequestKind::Sample, 0),
            accurate(true),
        ],
    );
    assert_eq!(arena.history(A).move_last_turn, Some(MoveResult::Failed));
    assert_eq!(
        arena.state().pokemon(B).unwrap().hp,
        arena.state().pokemon(B).unwrap().max_hp
    );

    // Pollen Puff on a full-HP ally: Gen 7's `onHit` reports `-immune` and fails; Gen 8+ returns
    // `NOT_FAIL`, which the hit loop counts as a hit.
    assert_eq!(
        pollen_puff_on_a_full_hp_ally(Generation::Gen7),
        Some(MoveResult::Failed)
    );
    assert_eq!(
        pollen_puff_on_a_full_hp_ally(Generation::Gen9),
        Some(MoveResult::Succeeded)
    );
}
