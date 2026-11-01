//! L1 rule tests for execution-time targeting (ROADMAP §6.3, DESIGN.md §15.4): a declared
//! target that is gone by the time the move runs, Follow Me, and Lightning Rod / Storm Drain
//! as both a redirect (doubles) and an absorbing immunity with a Special Attack boost.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, ChoiceError, EffectSource, EngineStep, HitBlockReason,
    MoveFailReason, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, ResolvedRules, ResolvedTarget, RulesResolver, TargetError, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState, VolatileConditionKind,
};

const P1A: PokemonId = PokemonId(1);
const P1B: PokemonId = PokemonId(2);
const P2A: PokemonId = PokemonId(3);
const P2B: PokemonId = PokemonId(4);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], ability: Option<&str>, level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = ability.map(Into::into);
    set.level = level;
    set
}

fn doubles(
    p1: [PokemonSet; 2],
    p2: [PokemonSet; 2],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Doubles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let mut pokemon = vec![
        build_pokemon(&dex, P1A, SideId::P1, &p1[0]).unwrap(),
        build_pokemon(&dex, P1B, SideId::P1, &p1[1]).unwrap(),
        build_pokemon(&dex, P2A, SideId::P2, &p2[0]).unwrap(),
        build_pokemon(&dex, P2B, SideId::P2, &p2[1]).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let state = BattleState::new(pokemon, [side([P1A, P1B]), side([P2A, P2B])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

fn singles(p1: PokemonSet, p2: PokemonSet) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let pokemon = vec![
        build_pokemon(&dex, P1A, SideId::P1, &p1).unwrap(),
        build_pokemon(&dex, P2A, SideId::P2, &p2).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(P1A), side(P2A)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

/// (actor, move slot, target side, target slot) — `None` target for self / spread moves.
type Decl = (PokemonId, usize, Option<(SideId, u8)>);

impl Arena {
    fn choice(&self, side: SideId, decls: &[Decl]) -> SideChoice {
        SideChoice {
            side,
            actions: decls
                .iter()
                .map(|(actor, slot, target)| SlotChoice {
                    actor: *actor,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(*slot as u8).unwrap(),
                        target: target.map(|(s, i)| {
                            DeclaredTarget(self.rules.topology().position(s, i).unwrap())
                        }),
                        mechanic: None,
                    },
                })
                .collect(),
        }
    }

    fn turn(&mut self, p1: &[Decl], p2: &[Decl], tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
        start
    }

    fn events(&self, from: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[from..]
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn damaged_by(&self, from: usize, actor: PokemonId) -> Vec<PokemonId> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::DamageApplied {
                    target,
                    source: EffectSource::Move { pokemon, .. },
                    ..
                } if *pokemon == actor => Some(*target),
                _ => None,
            })
            .collect()
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

#[test]
fn a_move_aimed_at_a_fainted_foe_retargets_the_other_foe_but_a_fainted_ally_makes_it_fail() {
    // Garchomp (fast) knocks Pikachu (L5) out first; Snorlax's Tackle, declared at Pikachu's
    // slot, hits Metagross instead.
    let mut arena = doubles(
        [
            set("Snorlax", &["Tackle"], None, 50),
            set("Garchomp", &["Tackle"], None, 50),
        ],
        [
            set("Metagross", &["Swords Dance"], None, 50),
            set("Pikachu", &["Tackle"], Some("Run Away"), 5),
        ],
        |_| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P2, 1))),
            (P1B, 0, Some((SideId::P2, 1))),
        ],
        &[(P2A, 0, None), (P2B, 0, Some((SideId::P1, 0)))],
        tape,
    );
    assert!(arena.pokemon(P2B).is_fainted());
    assert_eq!(arena.damaged_by(start, P1A), vec![P2A]);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::TargetRedirected {
            actor: P1A,
            to: P2A,
            by: EffectSource::Rule,
            ..
        }
    )));

    // Aimed at an ally that fainted: no retarget, the move fails.
    let mut arena = doubles(
        [
            set("Snorlax", &["Tackle"], None, 50),
            set("Garchomp", &["Tackle"], None, 50),
        ],
        [
            set("Metagross", &["Tackle"], None, 50),
            set("Pikachu", &["Tackle"], Some("Run Away"), 50),
        ],
        |pokemon| pokemon[1].hp = 1,
    );
    // Metagross's Tackle knocks Garchomp (1 HP) out before Snorlax's ally-targeted Tackle.
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P1, 1))),
            (P1B, 0, Some((SideId::P2, 0))),
        ],
        &[
            (P2A, 0, Some((SideId::P1, 1))),
            (P2B, 0, Some((SideId::P1, 0))),
        ],
        tape,
    );
    assert!(arena.pokemon(P1B).is_fainted());
    assert!(arena.damaged_by(start, P1A).is_empty());
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: P1A,
            reason: MoveFailReason::NoLegalTarget,
            ..
        }
    )));
}

#[test]
fn follow_me_pulls_the_foes_single_target_moves_and_fails_in_singles() {
    let mut arena = doubles(
        [
            set("Jolteon", &["Tackle"], None, 50),
            set("Garchomp", &["Tackle", "Earthquake"], None, 50),
        ],
        [
            // Not its first ability, Cute Charm: Tackle would roll it.
            set("Clefable", &["Follow Me"], Some("Run Away"), 50),
            set("Snorlax", &["Swords Dance"], None, 50),
        ],
        |_| {},
    );
    // Both P1 moves declared at Snorlax; Clefable's Follow Me (+2) goes first and takes them.
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P2, 1))),
            (P1B, 0, Some((SideId::P2, 1))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        tape,
    );
    assert_eq!(arena.damaged_by(start, P1A), vec![P2A]);
    assert_eq!(arena.damaged_by(start, P1B), vec![P2A]);
    assert_eq!(
        arena
            .events(start)
            .iter()
            .filter(|e| matches!(
                e,
                TraceEvent::TargetRedirected {
                    by: EffectSource::Volatile(VolatileConditionKind::FollowMe),
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(
        !arena
            .pokemon(P2A)
            .volatiles
            .contains(VolatileConditionKind::FollowMe),
        "single turn"
    );
    // A spread move is not redirected.
    let mut tape = hit(100);
    tape.extend(hit(100));
    tape.extend(hit(100));
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 1))), (P1B, 1, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        tape,
    );
    assert_eq!(
        arena.damaged_by(start, P1B).len(),
        3,
        "Earthquake still hits ally and both foes"
    );

    let mut arena = singles(
        set("Clefable", &["Follow Me"], None, 50),
        set("Snorlax", &["Swords Dance"], None, 50),
    );
    let start = arena.turn(&[(P1A, 0, None)], &[(P2A, 0, None)], Vec::new());
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveFailed { actor: P1A, .. }))
    );
    assert_eq!(
        DexResolver::resolve(Generation::Gen9)
            .move_by_name("Follow Me")
            .unwrap()
            .primary_effect,
        PrimaryEffect::Custom
    );
}

#[test]
fn lightning_rod_absorbs_electric_moves_with_a_special_attack_boost() {
    let mut arena = singles(
        set("Jolteon", &["Thunderbolt", "Tackle"], None, 50),
        set("Pikachu", &["Swords Dance"], Some("Lightning Rod"), 50),
    );
    // Thunderbolt: refused, +1 SpA, no accuracy or damage decision asked.
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0)))],
        &[(P2A, 0, None)],
        Vec::new(),
    );
    assert_eq!(arena.pokemon(P2A).stages.special_attack, 1);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::HitBlocked {
            target: P2A,
            reason: HitBlockReason::Ability(AbilityId::LIGHTNING_ROD),
            ..
        }
    )));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::AbilityActivated {
            source: P2A,
            ability: AbilityId::LIGHTNING_ROD
        }
    )));
    // Tackle goes through as usual.
    let start = arena.turn(
        &[(P1A, 1, Some((SideId::P2, 0)))],
        &[(P2A, 0, None)],
        hit(100),
    );
    assert_eq!(arena.damaged_by(start, P1A), vec![P2A]);
    // At +6 the move is still refused but nothing is announced as a boost.
    {
        let mut snapshot = arena.engine.snapshot();
        snapshot
            .state
            .pokemon_mut(P2A)
            .unwrap()
            .stages
            .special_attack = 6;
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    }
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0)))],
        &[(P2A, 0, None)],
        Vec::new(),
    );
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::AbilityActivated { source: P2A, .. }))
    );
    assert!(arena.damaged_by(start, P1A).is_empty());
}

#[test]
fn in_doubles_lightning_rod_and_storm_drain_pull_the_move_onto_the_holder() {
    // Thunderbolt declared at Metagross; Pikachu (Lightning Rod) pulls it, is immune, +1 SpA.
    let mut arena = doubles(
        [
            set("Jolteon", &["Thunderbolt"], None, 50),
            set("Garchomp", &["Surf"], None, 50),
        ],
        [
            set("Metagross", &["Swords Dance"], None, 50),
            set("Pikachu", &["Swords Dance"], Some("Lightning Rod"), 50),
        ],
        |_| {},
    );
    // Surf is a spread move: not redirected, Pikachu takes it normally (Storm Drain is not on
    // the field). Jolteon's Thunderbolt: redirected, absorbed.
    let mut tape = hit(100);
    tape.extend(hit(100));
    tape.extend(hit(100));
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        tape,
    );
    assert!(arena.damaged_by(start, P1A).is_empty());
    assert_eq!(arena.pokemon(P2B).stages.special_attack, 1);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::TargetRedirected {
            actor: P1A,
            to: P2B,
            by: EffectSource::Ability {
                ability: AbilityId::LIGHTNING_ROD,
                ..
            },
            ..
        }
    )));
    assert_eq!(arena.damaged_by(start, P1B).len(), 3);

    // Storm Drain on the user's own ally pulls the user's Water move too.
    let mut arena = doubles(
        [
            set("Garchomp", &["Water Gun"], None, 50),
            set("Gastrodon", &["Swords Dance"], Some("Storm Drain"), 50),
        ],
        [
            set("Metagross", &["Swords Dance"], None, 50),
            set("Snorlax", &["Swords Dance"], None, 50),
        ],
        |_| {},
    );
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        Vec::new(),
    );
    assert!(arena.damaged_by(start, P1A).is_empty());
    assert_eq!(
        arena.pokemon(P1B).stages.special_attack,
        1,
        "+1 from Storm Drain (Swords Dance is Attack)"
    );
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::TargetRedirected {
            actor: P1A,
            to: P1B,
            by: EffectSource::Ability {
                ability: AbilityId::STORM_DRAIN,
                ..
            },
            ..
        }
    )));
}

#[test]
fn only_doubles_ask_for_a_target() {
    // Showdown `chooseMove` requires a target once a side has two actives; in singles the one
    // foe is implied, an ally-or-self move takes the user and an ally move has nobody.
    let mut arena = singles(
        set("Snorlax", &["Tackle"], None, 50),
        set("Metagross", &["Swords Dance"], None, 50),
    );
    let start = arena.turn(&[(P1A, 0, None)], &[(P2A, 0, None)], hit(100));
    assert_eq!(arena.damaged_by(start, P1A), vec![P2A]);
    let resolve = |pattern| {
        arena
            .rules
            .targeting()
            .resolve(
                arena.rules.topology(),
                arena.engine.state(),
                P1A,
                pattern,
                None,
            )
            .map(|resolved| resolved.targets)
    };
    assert_eq!(
        resolve(TargetPattern::AllyOrSelf),
        Ok(vec![ResolvedTarget::Pokemon(P1A)])
    );
    assert_eq!(resolve(TargetPattern::Ally), Ok(Vec::new()));

    let mut arena = doubles(
        [
            set("Snorlax", &["Tackle"], None, 50),
            set("Garchomp", &["Swords Dance"], None, 50),
        ],
        [
            set("Metagross", &["Swords Dance"], None, 50),
            set("Pikachu", &["Swords Dance"], Some("Run Away"), 50),
        ],
        |_| {},
    );
    let choices = [
        arena.choice(SideId::P1, &[(P1A, 0, None), (P1B, 0, None)]),
        arena.choice(SideId::P2, &[(P2A, 0, None), (P2B, 0, None)]),
    ];
    assert_eq!(
        arena.engine.submit_turn_choices(choices),
        Err(BattleError::Choice(ChoiceError::InvalidTarget(
            TargetError::TargetRequired
        )))
    );
}

#[test]
fn a_random_normal_move_ignores_its_declaration() {
    // Showdown `randomNormal` (Struggle): the foe in a single battle whatever was declared; with
    // two foes no target is fixed (the Shell picks one at random, `random_retarget_candidates`).
    let arena = singles(
        set("Snorlax", &["Tackle"], None, 50),
        set("Metagross", &["Swords Dance"], None, 50),
    );
    let resolve = |arena: &Arena, declared| {
        arena.rules.targeting().resolve(
            arena.rules.topology(),
            arena.engine.state(),
            P1A,
            TargetPattern::RandomAdjacentFoe,
            declared,
        )
    };
    let own_slot = DeclaredTarget(arena.rules.topology().position(SideId::P1, 0).unwrap());
    assert_eq!(
        resolve(&arena, Some(own_slot)).map(|r| r.targets),
        Ok(vec![ResolvedTarget::Pokemon(P2A)])
    );
    let arena = doubles(
        [
            set("Snorlax", &["Tackle"], None, 50),
            set("Garchomp", &["Swords Dance"], None, 50),
        ],
        [
            set("Metagross", &["Swords Dance"], None, 50),
            set("Pikachu", &["Swords Dance"], Some("Run Away"), 50),
        ],
        |_| {},
    );
    assert_eq!(
        resolve(&arena, None).map(|r| r.targets),
        Err(TargetError::TargetRequired)
    );
}

#[test]
fn the_faster_follow_me_decides_even_for_a_move_already_aimed_at_it() {
    // Showdown `priorityEvent('RedirectTarget')` stops at the first handler that names a target
    // (priority, then speed): Jolteon's Follow Me keeps the Tackle aimed at Jolteon, and the
    // slower Slowpoke's takes nothing.
    let mut arena = doubles(
        [
            set("Snorlax", &["Tackle"], None, 50),
            set("Blissey", &["Swords Dance"], None, 50),
        ],
        [
            set("Jolteon", &["Follow Me"], None, 50),
            set("Slowpoke", &["Follow Me"], None, 50),
        ],
        |_| {},
    );
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        hit(100),
    );
    assert_eq!(arena.damaged_by(start, P1A), vec![P2A]);
}

#[test]
fn follow_me_redirects_before_lightning_rod() {
    // Follow Me's handler has priority 1, Lightning Rod's none: the Thunderbolt aimed at the
    // Lightning Rod holder goes to the Follow Me user.
    let mut arena = doubles(
        [
            set("Jolteon", &["Thunderbolt"], None, 50),
            set("Blissey", &["Swords Dance"], None, 50),
        ],
        [
            set("Snorlax", &["Follow Me"], None, 50),
            set("Pikachu", &["Swords Dance"], Some("Lightning Rod"), 50),
        ],
        |_| {},
    );
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 1))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        tape,
    );
    assert_eq!(arena.damaged_by(start, P1A), vec![P2A]);
}

#[test]
fn a_random_retarget_is_drawn_before_the_paralysis_check() {
    // Showdown `runMove`: `getTarget`, the Encore override's `getRandomTarget`, then
    // `BeforeMove`. Encored this turn into Tackle, the paralysed Snorlax draws its target among
    // both foes first, then is fully paralysed.
    let mut arena = doubles(
        [
            set("Snorlax", &["Tackle", "Swords Dance"], None, 50),
            set("Blissey", &["Swords Dance"], None, 50),
        ],
        [
            set("Jolteon", &["Encore"], None, 50),
            set("Slowpoke", &["Swords Dance"], None, 50),
        ],
        |pokemon| {
            let tackle = pokemon[0].moves[0].move_id;
            pokemon[0].last_move = Some(tackle);
            pokemon[0].last_move_encore = Some(tackle);
            pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Paralysis));
        },
    );
    let tape = vec![
        RandomOverride::range(RandomRequestKind::RandomTarget, 1),
        RandomOverride::bool(RandomRequestKind::FullParalysis, true),
    ];
    let start = arena.turn(
        &[(P1A, 1, None), (P1B, 0, None)],
        &[(P2A, 0, Some((SideId::P1, 0))), (P2B, 0, None)],
        tape,
    );
    assert!(arena.damaged_by(start, P1A).is_empty());
    assert!(
        arena
            .pokemon(P1A)
            .volatiles
            .contains(VolatileConditionKind::Encore)
    );
}

#[test]
fn a_locked_attack_aims_as_its_lock_whatever_target_the_choice_submits() {
    // Showdown `chooseMove` gives a locked action the lock's `targetLoc` (a charge's
    // `twoturnmove.targetLoc`) and refuses a target the choice names; validation does not ask
    // for one here, and one submitted steers nothing (R-M10-20).
    let mut arena = doubles(
        [
            set("Snorlax", &["Solar Beam"], None, 50),
            set("Garchomp", &["Swords Dance"], None, 50),
        ],
        [
            set("Metagross", &["Swords Dance"], None, 50),
            set("Blissey", &["Swords Dance"], None, 50),
        ],
        |_| {},
    );
    let foes = [(P2A, 0, None), (P2B, 0, None)];
    arena.turn(
        &[(P1A, 0, Some((SideId::P2, 1))), (P1B, 0, None)],
        &foes,
        Vec::new(),
    );
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &foes,
        hit(100),
    );
    assert_eq!(arena.damaged_by(start, P1A), vec![P2B]);
}

#[test]
fn an_encored_charge_from_a_move_without_a_target_draws_its_target_at_the_release() {
    // Snorlax chose Stealth Rock, which `resolveAction` gives a foe drawn among the two standing
    // as its `targetLoc`; Jolteon's Encore turns the action into Solar Beam, whose charge draws a
    // target of its own. The attack goes at the first draw's foe, not the charge's: the release
    // draws again, among the foes standing in those slots, which stands for it (R-M10-20) — a
    // target the locked choice names counts for nothing.
    let mut arena = doubles(
        [
            set("Snorlax", &["Solar Beam", "Stealth Rock"], None, 50),
            set("Garchomp", &["Swords Dance"], None, 50),
        ],
        [
            set("Jolteon", &["Encore", "Swords Dance"], None, 50),
            set("Blissey", &["Swords Dance"], None, 50),
        ],
        |pokemon| {
            let solar_beam = pokemon[0].moves[0].move_id;
            pokemon[0].last_move = Some(solar_beam);
            pokemon[0].last_move_encore = Some(solar_beam);
        },
    );
    arena.turn(
        &[(P1A, 1, None), (P1B, 0, None)],
        &[(P2A, 0, Some((SideId::P1, 0))), (P2B, 0, None)],
        vec![RandomOverride::range(RandomRequestKind::RandomTarget, 0)],
    );
    assert!(
        arena
            .pokemon(P1A)
            .volatiles
            .contains(VolatileConditionKind::TwoTurnMove)
    );
    let mut tape = vec![RandomOverride::range(RandomRequestKind::RandomTarget, 1)];
    tape.extend(hit(100));
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 1, None), (P2B, 0, None)],
        tape,
    );
    assert_eq!(arena.damaged_by(start, P1A), vec![P2B]);
}
