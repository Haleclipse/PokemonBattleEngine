//! L1 rule tests for M6 batch 3 (DESIGN.md §31.3): Gen 1's historical algorithms — the
//! modified stats (paralysis / burn cuts, stage changes from the stored stat, the other side's
//! stat move cutting them again), the base-Speed critical chance, every move rolling its
//! accuracy, round-up secondaries, a faint ending the turn, Leech Seed scaled by the bad-poison
//! counter, Hyper Beam without a recharge after a knock-out, and confusion's own formula; from
//! M8's third campaign round (DESIGN.md R40): the Substitute's short list of stopped status
//! moves, the recharge turn behind a freeze, the Hyper Beam sleep bug, and a frozen Pokémon
//! acting with its side's last selected move.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BeforeMoveBlockReason, CanMoveResult, EngineStep, SecondaryRoll, SideChoice,
    SlotChoice, TraceEvent, accuracy_roll, secondary_roll,
};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SelectedMove, SideState, StatusState, VolatileConditionKind,
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

fn gen1([a, b, c, d]: [PokemonSet; 4], prepare: impl FnOnce(&mut Vec<PokemonState>)) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen1,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen1));
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &a).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &b).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &c).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &d).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B, D])], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
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

    fn turn(&mut self, p1: usize, p2: usize, tape: Vec<RandomOverride>) -> (usize, EngineStep) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)];
        self.engine.submit_turn_choices(choices).unwrap();
        let step = loop {
            match self.engine.step().unwrap() {
                EngineStep::Progressed { .. } => {}
                other => break other,
            }
        };
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
        (start, step)
    }

    fn events(&self, from: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[from..]
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn modified_speed(&self, id: PokemonId) -> u16 {
        self.pokemon(id).modified_stats.unwrap().speed
    }
}

fn hits(yes: bool) -> RandomOverride {
    RandomOverride::bool(RandomRequestKind::Accuracy, yes)
}

#[test]
fn gen_1_paralysis_cuts_the_modified_speed_and_the_foes_stat_moves_cut_it_again() {
    // Tauros (faster) raises its Speed; Snorlax paralyses it: ⌊2s / 4⌋. Next turn Tauros
    // raises it again — recomputed from the stored stat, the cut is gone (3s) — and then
    // Snorlax's Swords Dance cuts the paralysed foe's Speed again: ⌊3s / 4⌋.
    let mut arena = gen1(
        [
            set("Tauros", &["Agility"]),
            set("Snorlax", &["Thunder Wave", "Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let stored = u32::from(arena.pokemon(A).stats.speed);
    assert_eq!(u32::from(arena.modified_speed(A)), stored);
    arena.turn(0, 0, vec![hits(true)]);
    assert!(arena.pokemon(A).has_status(NonVolatileStatus::Paralysis));
    assert_eq!(u32::from(arena.modified_speed(A)), stored * 2 / 4);
    arena.turn(
        0,
        1,
        vec![RandomOverride::bool(
            RandomRequestKind::FullParalysis,
            false,
        )],
    );
    assert_eq!(u32::from(arena.modified_speed(A)), stored * 3 / 4);
    // A switch-out resets the modified stats and re-applies the cut.
    let mut state = arena.engine.state().clone();
    let pokemon = state.pokemon_mut(A).unwrap();
    pokemon.reset_modified_stats();
    assert_eq!(u32::from(pokemon.modified_stats.unwrap().speed), stored / 4);
}

#[test]
fn gen_1_critical_chance_comes_from_the_base_speed() {
    // Tauros: base Speed 110 → ⌊110 / 2⌋ = 55 out of 256.
    let mut arena = gen1(
        [
            set("Tauros", &["Body Slam"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let tape = vec![
        hits(true),
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 217),
        // Body Slam's paralysis never rolls against a Normal type in Gen 1.
    ];
    let (start, _) = arena.turn(0, 0, tape);
    let chance = arena.events(start).iter().find_map(|e| match e {
        TraceEvent::CriticalChecked { chance, .. } => Some(*chance),
        _ => None,
    });
    assert_eq!(
        chance,
        Some(pokemon_battle_engine::rules::Fraction::new(55, 256))
    );
}

#[test]
fn gen_1_chances_always_roll_and_secondaries_round_up() {
    let arena = gen1(
        [
            set(
                "Tauros",
                &["Body Slam", "Psybeam", "Blizzard", "Sand-Attack"],
            ),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            set("Lapras", &["Swords Dance"]),
        ],
        |pokemon| pokemon[1].stages.evasion = 1,
    );
    let state = arena.engine.state();
    let rules = &arena.rules;
    let move_def = |slot: usize| {
        arena
            .dex
            .move_by_id(state.pokemon(A).unwrap().moves[slot].move_id)
            .unwrap()
            .clone()
    };
    // 100 % → 255 out of 256; the target's +1 evasion takes it to ⌊255 × 66 / 100⌋ = 168.
    assert_eq!(
        accuracy_roll(state, rules, A, B, &move_def(0)),
        Some((168, 256))
    );
    assert_eq!(
        accuracy_roll(state, rules, A, C, &move_def(0)),
        Some((255, 256))
    );
    // Body Slam's 30 %: ⌈76.8⌉ = 77 — but never against a Normal type (Snorlax); Psybeam's
    // 10 % confusion: ⌈25.6⌉ − 1 = 25; Blizzard never freezes an Ice type.
    let body_slam = move_def(0);
    assert_eq!(
        secondary_roll(state, rules, A, B, &body_slam, &body_slam.secondaries[0]),
        SecondaryRoll::Skipped
    );
    assert_eq!(
        secondary_roll(state, rules, A, D, &body_slam, &body_slam.secondaries[0]),
        SecondaryRoll::Chance(77, 256)
    );
    let psybeam = move_def(1);
    assert_eq!(
        secondary_roll(state, rules, A, B, &psybeam, &psybeam.secondaries[0]),
        SecondaryRoll::Chance(25, 256)
    );
    let blizzard = move_def(2);
    assert_eq!(
        secondary_roll(state, rules, A, D, &blizzard, &blizzard.secondaries[0]),
        SecondaryRoll::Skipped
    );
}

#[test]
fn a_gen_1_faint_ends_the_turn_on_the_spot() {
    // Tauros (faster, 1 HP) knocks itself out with Double-Edge's recoil: Snorlax's move and
    // the whole end of the turn are skipped.
    let mut arena = gen1(
        [
            set("Tauros", &["Double-Edge"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[0].hp = 1,
    );
    let tape = vec![
        hits(true),
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 217),
    ];
    let (start, _) = arena.turn(0, 0, tape);
    assert!(arena.pokemon(A).is_fainted());
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveUsed { actor, .. } if *actor == B))
    );
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::ActionsCancelledByFaint { .. }))
    );
}

#[test]
fn gen_1_leech_seed_is_scaled_by_the_bad_poison_counter_and_heals_in_full() {
    // Snorlax is badly poisoned (counter 0) and seeded by Exeggutor. After its move the
    // counter reaches 1 (bad poison: 1/16), then 2 (Leech Seed: 2/16, all healed).
    let mut arena = gen1(
        [
            set("Exeggutor", &["Swords Dance"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[0].hp = 1,
    );
    // After the leads' entry (a badly poisoned lead would become poisoned on the way in).
    let seeder_position = arena.rules.topology().position(SideId::P1, 0).unwrap();
    let mut snapshot = arena.engine.snapshot();
    let snorlax = snapshot.state.pokemon_mut(B).unwrap();
    snorlax.status = Some(StatusState::new(NonVolatileStatus::Toxic));
    snorlax.volatiles.start_residual_damage();
    snorlax.volatiles.set_leech_seed(A, seeder_position);
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let max_hp = arena.pokemon(B).max_hp;
    let sixteenth = max_hp / 16;
    arena.turn(0, 0, Vec::new());
    assert_eq!(
        arena.pokemon(B).volatiles.residual_damage_counter(),
        Some(2)
    );
    assert_eq!(arena.pokemon(B).hp, max_hp - sixteenth - 2 * sixteenth);
    assert_eq!(arena.pokemon(A).hp, 1 + 2 * sixteenth);
}

#[test]
fn gen_1_hyper_beam_needs_no_recharge_after_a_knock_out() {
    for (foe_hp, recharges) in [(1, false), (300, true)] {
        let mut arena = gen1(
            [
                set("Tauros", &["Hyper Beam"]),
                set("Snorlax", &["Swords Dance"]),
                filler(),
                filler(),
            ],
            |pokemon| pokemon[1].hp = foe_hp.min(pokemon[1].max_hp),
        );
        let tape = vec![
            hits(true),
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, 217),
        ];
        arena.turn(0, 0, tape);
        assert_eq!(
            arena
                .pokemon(A)
                .volatiles
                .contains(VolatileConditionKind::MustRecharge),
            recharges,
            "foe at {foe_hp}"
        );
    }
}

#[test]
fn gen_1_confusion_hits_with_its_own_formula() {
    let mut arena = gen1(
        [
            set("Snorlax", &["Swords Dance"]),
            set("Tauros", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| {
            pokemon[0].volatiles.set_confusion(B, 3);
        },
    );
    let pokemon = arena.pokemon(A);
    let (level, attack, defense) = (
        u32::from(pokemon.level),
        u32::from(pokemon.stats.attack),
        u32::from(pokemon.stats.defense),
    );
    let before = pokemon.hp;
    arena.turn(
        0,
        0,
        vec![RandomOverride::bool(
            RandomRequestKind::ConfusionSelfHit,
            true,
        )],
    );
    let expected = ((2 * level / 5 + 2) * attack * 40 / defense / 50 + 2) as u16;
    assert_eq!(before - arena.pokemon(A).hp, expected);
}

#[test]
fn one_secondary_with_several_effects_rolls_once() {
    // Gen 1 Psychic lowers Special Attack and Special Defense on one 33 % roll; Gen 9 Ancient
    // Power raises five stats on one 10 % roll.
    let shares = |generation, name: &str| -> Vec<bool> {
        DexResolver::resolve(generation)
            .move_by_name(name)
            .unwrap()
            .secondaries
            .iter()
            .map(|secondary| secondary.shares_roll)
            .collect()
    };
    assert_eq!(shares(Generation::Gen1, "Psychic"), vec![false, true]);
    assert_eq!(
        shares(Generation::Gen9, "Ancient Power"),
        vec![false, true, true, true, true]
    );
    assert_eq!(shares(Generation::Gen9, "Fire Fang"), vec![false, false]);
}

#[test]
fn a_gen_1_substitute_holds_one_hp_more_than_it_cost() {
    // Showdown gen1 `substitute`: `hp = floor(maxhp / 4) + 1` for a cost of `maxhp / 4`.
    let mut arena = gen1(
        [
            set("Snorlax", &["Substitute"]),
            filler(),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let max_hp = arena.pokemon(A).max_hp;
    arena.turn(0, 0, Vec::new());
    assert_eq!(arena.pokemon(A).hp, max_hp - max_hp / 4);
    assert_eq!(
        arena.pokemon(A).volatiles.substitute_hp(),
        Some(max_hp / 4 + 1)
    );
}

#[test]
fn a_gen_1_confusion_self_hit_behind_a_substitute_hurts_only_a_foes_substitute() {
    // Showdown `directDamage` (Gen 1): the confused Pokémon's own HP is untouched; the foe's
    // Substitute takes the hit if there is one.
    for foe_has_substitute in [false, true] {
        let mut arena = gen1(
            [
                set("Snorlax", &["Swords Dance"]),
                set("Chansey", &["Swords Dance"]),
                filler(),
                filler(),
            ],
            |pokemon| {
                pokemon[0].volatiles.set_substitute(A, 30);
                pokemon[0].volatiles.set_confusion(B, 3);
                if foe_has_substitute {
                    pokemon[1].volatiles.set_substitute(B, 50);
                }
            },
        );
        let hp = arena.pokemon(A).hp;
        let tape = vec![RandomOverride::bool(
            RandomRequestKind::ConfusionSelfHit,
            true,
        )];
        arena.turn(0, 0, tape);
        assert_eq!(arena.pokemon(A).hp, hp);
        assert_eq!(arena.pokemon(A).volatiles.substitute_hp(), Some(30));
        let foe_substitute = arena.pokemon(B).volatiles.substitute_hp();
        if foe_has_substitute {
            assert!(foe_substitute.is_some_and(|left| left < 50));
        } else {
            assert_eq!(foe_substitute, None);
        }
    }
}

#[test]
fn gen_1_disable_seals_a_random_move_and_counts_down_at_before_move() {
    // Showdown gen1 `disable`: `onStart` samples one of the target's moves with PP (here the
    // second, Harden) and a `time` of 1-8 (2); the holder's every BeforeMove counts it down
    // before anything else is checked, and at 0 it ends and the move goes ahead. No residual.
    let mut arena = gen1(
        [
            set("Jolteon", &["Disable", "Agility"]),
            set("Snorlax", &["Swords Dance", "Harden"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let slot = RandomOverride::range(RandomRequestKind::DisableSlot, 1);
    let time = RandomOverride::range(RandomRequestKind::Duration, 2);
    arena.turn(0, 0, vec![hits(true), slot, time]);
    let disable = |arena: &Arena| {
        let volatiles = &arena.pokemon(B).volatiles;
        (
            volatiles.disabled_move(),
            volatiles.turns_left(VolatileConditionKind::Disable),
        )
    };
    // Snorlax's Swords Dance that turn counted it to 1.
    assert_eq!(disable(&arena), (Some(MoveId::HARDEN), Some(1)));
    // Harden cannot be chosen; the next BeforeMove counts the Disable out.
    arena.turn(1, 0, Vec::new());
    assert_eq!(disable(&arena), (None, None));
    let (start, _) = arena.turn(1, 1, Vec::new());
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::MoveUsed {
            actor: B,
            move_id: MoveId::HARDEN,
            ..
        }
    )));
}

#[test]
fn gen_1_2_an_entry_makes_every_pokemon_forget_its_last_move() {
    // Gen 1-2 `runSwitch`: whoever enters, every active Pokémon's `lastMove` is cleared — a
    // later Encore or (Gen 2) Disable finds nothing to seal.
    let mut arena = gen1(
        [
            set("Jolteon", &["Agility"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    arena.turn(0, 0, Vec::new());
    assert_eq!(arena.pokemon(A).last_move, Some(MoveId::AGILITY));
    // Jolteon falls asleep, so nothing it does this turn sets its last move again.
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(1, Vec::new());
    let mut sleep = StatusState::new(NonVolatileStatus::Sleep);
    sleep.sleep_turns_remaining = Some(3);
    snapshot.state.pokemon_mut(A).unwrap().status = Some(sleep);
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let switch = SideChoice {
        side: SideId::P2,
        actions: vec![SlotChoice {
            actor: B,
            action: ActionChoice::Switch { pokemon: D },
        }],
    };
    arena
        .engine
        .submit_turn_choices([arena.choice(SideId::P1, 0), switch])
        .unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert_eq!(arena.pokemon(A).last_move, None);
    assert!(
        arena
            .engine
            .trace()
            .events()
            .iter()
            .any(|e| matches!(e, TraceEvent::Switched { incoming: D, .. }))
    );
}

#[test]
fn a_gen_1_multi_hit_move_stops_when_it_breaks_the_substitute() {
    // Showdown gen1 `subFainted`: the hit that breaks the doll ends the sequence.
    let mut arena = gen1(
        [
            set("Beedrill", &["Twineedle"]),
            set("Chansey", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[1].volatiles.set_substitute(B, 1),
    );
    let hp = arena.pokemon(B).hp;
    let tape = vec![
        hits(true),
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 255).optional(),
    ];
    arena.turn(0, 0, tape);
    assert_eq!(arena.pokemon(B).volatiles.substitute_hp(), None);
    assert_eq!(arena.pokemon(B).hp, hp);
}

#[test]
fn a_gen_1_substitute_lets_paralysis_through_but_stops_stat_drops() {
    // Showdown gen1 `substitute` `onTryHit`: of the status moves only poison, stat drops and
    // confusion are stopped.
    for (moves, paralysed, attack_stage) in [(["Stun Spore"], true, 0), (["Growl"], false, 0)] {
        let mut arena = gen1(
            [
                set("Jolteon", &["Substitute"]),
                set("Snorlax", &moves),
                filler(),
                filler(),
            ],
            |_| {},
        );
        arena.turn(0, 0, vec![hits(true)]);
        let jolteon = arena.pokemon(A);
        assert!(
            jolteon
                .volatiles
                .contains(VolatileConditionKind::Substitute)
        );
        assert_eq!(
            jolteon.has_status(NonVolatileStatus::Paralysis),
            paralysed,
            "{moves:?}"
        );
        assert_eq!(jolteon.stages.attack, attack_stage, "{moves:?}");
    }
}

#[test]
fn gen_1_recharge_waits_behind_a_freeze_and_a_sleep_move_overrides_both() {
    // Gen 1 `mustrecharge` has `onBeforeMovePriority` 7, below `frz`'s 12: a frozen Pokémon
    // keeps owing its recharge. A sleep move never misses it, puts it to sleep over the freeze
    // and clears the recharge (gen1 `moveHit` bug).
    let mut arena = gen1(
        [
            set("Lapras", &["Growl", "Sleep Powder"]),
            set("Tauros", &["Hyper Beam"]),
            filler(),
            filler(),
        ],
        |pokemon| {
            pokemon[1].status = Some(StatusState::new(NonVolatileStatus::Freeze));
            pokemon[1].volatiles.set_must_recharge(None);
        },
    );
    let (start, _) = arena.turn(0, 0, vec![hits(true)]);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::BeforeMoveResolved { actor, result: CanMoveResult::Blocked(BeforeMoveBlockReason::Freeze), .. }
            if *actor == B
    )));
    let tauros = arena.pokemon(B);
    assert!(tauros.has_status(NonVolatileStatus::Freeze));
    assert!(
        tauros
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
    );

    // No Accuracy decision: only the sleep's duration.
    arena.turn(
        1,
        0,
        vec![RandomOverride::range(RandomRequestKind::Duration, 3)],
    );
    let tauros = arena.pokemon(B);
    assert!(tauros.has_status(NonVolatileStatus::Sleep));
    assert!(
        !tauros
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
    );
}

#[test]
fn a_gen_1_frozen_pokemon_acts_with_its_sides_last_selected_move() {
    // Showdown `side.lastSelectedMove`: the frozen Slowbro's action is its side's last pick
    // from the menu, here Quick Attack, so it goes before the faster Jolteon.
    for (last_selected, slowbro_first) in [(None, false), (Some("Quick Attack"), true)] {
        let mut arena = gen1(
            [
                set("Jolteon", &["Swords Dance"]),
                set("Slowbro", &["Growl"]),
                filler(),
                filler(),
            ],
            |pokemon| pokemon[1].status = Some(StatusState::new(NonVolatileStatus::Freeze)),
        );
        let quick_attack = last_selected.map(|name| arena.dex.move_by_name(name).unwrap().id);
        let mut snapshot = arena.engine.snapshot();
        snapshot.state.side_mut(SideId::P2).last_selected_move =
            quick_attack.map(SelectedMove::Move);
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();

        let (start, _) = arena.turn(0, 0, Vec::new());
        let first = arena.events(start).iter().find_map(|e| match e {
            TraceEvent::BeforeMoveResolved { actor, .. } => Some(*actor),
            _ => None,
        });
        assert_eq!(first, Some(if slowbro_first { B } else { A }));
        // A pick from the menu is recorded; a frozen Pokémon's turn leaves its side's alone.
        let swords_dance = arena.dex.move_by_name("Swords Dance").unwrap().id;
        let state = arena.engine.state();
        assert_eq!(
            state.side(SideId::P1).last_selected_move,
            Some(SelectedMove::Move(swords_dance))
        );
        assert_eq!(
            state.side(SideId::P2).last_selected_move,
            quick_attack.map(SelectedMove::Move)
        );
    }
}

#[test]
fn a_gen_1_drain_off_a_substitute_floors_like_any_drain() {
    // Showdown gen1 `substitute`: `clampIntRange(floor(damage × drain), 1)`; from Gen 3 the doll
    // drains `ceil`. An odd hit on the doll tells the two apart.
    for roll in 217..=255u16 {
        let mut arena = gen1(
            [
                set("Venusaur", &["Mega Drain"]),
                set("Snorlax", &["Swords Dance"]),
                filler(),
                filler(),
            ],
            |pokemon| {
                pokemon[0].hp = 10;
                pokemon[1].volatiles.set_substitute(B, 200);
            },
        );
        let tape = vec![
            hits(true),
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, roll),
        ];
        let (start, _) = arena.turn(0, 0, tape);
        let dealt = arena.events(start).iter().find_map(|e| match e {
            TraceEvent::SubstituteDamaged { amount, .. } => Some(*amount),
            _ => None,
        });
        let Some(dealt) = dealt.filter(|d| d % 2 == 1) else {
            continue;
        };
        let healed = arena.events(start).iter().find_map(|e| match e {
            TraceEvent::HealApplied { target, amount, .. } if *target == A => Some(*amount),
            _ => None,
        });
        assert_eq!(healed, Some((dealt / 2).max(1)), "{dealt} dealt");
        return;
    }
    panic!("no roll hit the doll for an odd amount");
}

#[test]
fn a_gen_1_hit_that_breaks_a_substitute_skips_the_recharge() {
    // Showdown gen1 `moveHit`'s `doSelf`: the move's `self` effects are skipped when this hit
    // broke the doll, so Hyper Beam owes no recharge.
    for (doll, recharges) in [(1, false), (500, true)] {
        let mut arena = gen1(
            [
                set("Tauros", &["Hyper Beam"]),
                set("Snorlax", &["Swords Dance"]),
                filler(),
                filler(),
            ],
            |pokemon| pokemon[1].volatiles.set_substitute(B, doll),
        );
        let tape = vec![
            hits(true),
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, 217),
        ];
        arena.turn(0, 0, tape);
        assert_eq!(
            arena
                .pokemon(A)
                .volatiles
                .contains(VolatileConditionKind::MustRecharge),
            recharges,
            "doll {doll}"
        );
    }
}

#[test]
fn gen_1_leech_seed_feeds_the_seeder_after_its_holder_faints_to_poison() {
    // gen1 `leechseed` `onAfterMoveSelf` has no HP check: once the holder's poison knocked it
    // out, the seeder still heals the full amount.
    let mut arena = gen1(
        [
            set("Venusaur", &["Swords Dance"]),
            set("Snorlax", &["Swords Dance"]),
            filler(),
            filler(),
        ],
        |pokemon| {
            pokemon[0].hp = 50;
            // Poison takes 1/16 on entry and 1/16 after the move: the second one knocks it out.
            pokemon[1].hp = 2 * (pokemon[1].max_hp / 16);
            pokemon[1].status = Some(StatusState::new(NonVolatileStatus::Poison));
        },
    );
    let position = arena.rules.topology().position(SideId::P1, 0).unwrap();
    let mut snapshot = arena.engine.snapshot();
    snapshot
        .state
        .pokemon_mut(B)
        .unwrap()
        .volatiles
        .set_leech_seed(A, position);
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let drain = arena.pokemon(B).max_hp / 16;
    arena.turn(0, 0, Vec::new());
    assert!(arena.pokemon(B).is_fainted());
    assert_eq!(arena.pokemon(A).hp, 50 + drain);
}

/// Gen 1 binding (DESIGN.md §30.9): the lock repeats the move — whatever the action names —
/// without PP, accuracy or damage decisions, and leaves the side's last selected move as it
/// was; the bound target's turns are lost.
#[test]
fn gen1_binding_move_that_knocks_out_rolls_but_locks_nothing() {
    // Snorlax at 1 HP: the Wrap knocks it out. The lock's turns are rolled all the same
    // (`durationCallback` before `onStart`), then `onStart` finds no foe with HP: no lock
    // (DESIGN.md R142).
    let mut arena = gen1(
        [
            set("Mew", &["Wrap", "Harden"]),
            set("Snorlax", &["Harden"]),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[1].hp = 1,
    );
    let tape = vec![
        hits(true),
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 217),
        RandomOverride::range(RandomRequestKind::Duration, 3),
    ];
    let (start, _) = arena.turn(0, 0, tape);
    assert!(arena.pokemon(B).is_fainted());
    assert!(!arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::VolatileStarted {
            kind: VolatileConditionKind::PartialTrappingLock,
            ..
        }
    )));
}

#[test]
fn gen1_binding_lock_repeats_the_move_without_pp() {
    let mut arena = gen1(
        [
            set("Mew", &["Wrap", "Harden"]),
            set("Snorlax", &["Harden"]),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let wrap_pp = |arena: &Arena| arena.engine.state().pokemon(A).unwrap().moves[0].pp;
    let full = wrap_pp(&arena);
    // The first Wrap: accuracy, critical hit, damage, then the lock's turns (3).
    let tape = vec![
        hits(true),
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 217),
        RandomOverride::range(RandomRequestKind::Duration, 3),
    ];
    arena.turn(0, 0, tape);
    assert_eq!(wrap_pp(&arena), full - 1);
    let lock = arena
        .engine
        .state()
        .pokemon(A)
        .unwrap()
        .volatiles
        .partial_trapping_lock();
    assert!(matches!(lock, Some((B, MoveId::WRAP, _, 2, 3))), "{lock:?}");
    // Harden named, Wrap used again: no PP, no decision; Snorlax's action is lost.
    let (start, _) = arena.turn(1, 0, Vec::new());
    assert_eq!(wrap_pp(&arena), full - 1);
    assert_eq!(
        arena.engine.state().side(SideId::P1).last_selected_move,
        Some(SelectedMove::Move(MoveId::WRAP))
    );
    let used: Vec<MoveId> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::MoveUsed { actor, move_id, .. } => Some((*actor, *move_id)),
            _ => None,
        })
        .map(|(actor, move_id)| {
            assert_eq!(actor, A, "the bound Snorlax does not move");
            move_id
        })
        .collect();
    assert_eq!(used, vec![MoveId::WRAP]);
    // Nothing is rolled against the Pokémon it holds (`partiallytrapped`'s `onAccuracy`).
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::AccuracyChecked {
            actor: A,
            chance: 100,
            hit: true,
            ..
        }
    )));
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::CriticalChecked { .. }))
    );
}
