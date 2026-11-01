//! L1 rule tests for the hit sequence (ROADMAP §6.4 / §6.5, DESIGN.md §15.3): multi-hit count,
//! per-hit damage, Substitute per hit, stop on faint, recoil and drain with their generation
//! rounding. Every case goes through the shared Move Shell; nothing here is move-specific code.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, MultiHit, PokemonSet, PrimaryEffect, Recoil, ResolvedDex, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, EffectSource, EngineStep, HitBlockReason, MoveFailReason, SideChoice, SlotChoice,
    TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, Fraction, RecoilTiming, ResolvedRules, Rounding, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState,
};

const P1: PokemonId = PokemonId(1);
const P2: PokemonId = PokemonId(2);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn setup(
    generation: Generation,
    p1: (&str, &str),
    p2: (&str, &str),
    prepare: impl FnOnce(&mut PokemonState, &mut PokemonState),
) -> Arena {
    setup_sets(
        generation,
        PokemonSet::new(p1.0, [p1.1]),
        PokemonSet::new(p2.0, [p2.1]),
        prepare,
    )
}

fn setup_sets(
    generation: Generation,
    p1: PokemonSet,
    p2: PokemonSet,
    prepare: impl FnOnce(&mut PokemonState, &mut PokemonState),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut a = build_pokemon(&dex, P1, SideId::P1, &p1).unwrap();
    let mut b = build_pokemon(&dex, P2, SideId::P2, &p2).unwrap();
    prepare(&mut a, &mut b);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![a, b], [side(P1), side(P2)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn choice(&self, side: SideId, actor: PokemonId) -> SideChoice {
        let move_id = self.engine.state().pokemon(actor).unwrap().moves[0].move_id;
        let target = match self.dex.move_by_id(move_id).unwrap().target {
            TargetPattern::SelectedAny
            | TargetPattern::SelectedAdjacent
            | TargetPattern::SelectedFoe
            | TargetPattern::SelectedAdjacentFoe
            | TargetPattern::Ally
            | TargetPattern::AllyOrSelf => Some(DeclaredTarget(
                self.rules.topology().position(side.other(), 0).unwrap(),
            )),
            _ => None,
        };
        SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::FIRST,
                    target,
                    mechanic: None,
                },
            }],
        }
    }

    /// One turn, both sides using their first move. Speeds differ, so no SpeedTie is drawn.
    fn turn(&mut self, tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1, P1), self.choice(SideId::P2, P2)];
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

    /// (target, amount) of every HP loss committed since `from`, in order.
    fn damages(&self, from: usize) -> Vec<(PokemonId, u16)> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::DamageApplied { target, amount, .. } => Some((*target, *amount)),
                _ => None,
            })
            .collect()
    }

    fn heals(&self, from: usize) -> Vec<(PokemonId, u16)> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::HealApplied { target, amount, .. } => Some((*target, *amount)),
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

/// `hit` preceded by an Accuracy decision when the actor's move can miss (Gen 1-4 Tackle is
/// 95 %, Pin Missile 85 %).
fn attack(arena: &Arena, actor: PokemonId, roll: u16) -> Vec<RandomOverride> {
    let move_id = arena.pokemon(actor).moves[0].move_id;
    let mut tape = Vec::new();
    if arena
        .dex
        .move_by_id(move_id)
        .unwrap()
        .accuracy
        .is_some_and(|a| a < 100)
    {
        tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, true));
    }
    tape.extend(hit(roll));
    tape
}

fn hits(count: u16, roll: u16) -> Vec<RandomOverride> {
    let mut tape = vec![RandomOverride::range(
        RandomRequestKind::MultiHitCount,
        count,
    )];
    for _ in 0..count {
        tape.extend(hit(roll));
    }
    tape
}

#[test]
fn hit_sequence_classifies_from_snapshot_data() {
    let dex = DexResolver::resolve(Generation::Gen9);
    let get = |name: &str| dex.move_by_name(name).unwrap();
    assert_eq!(
        get("Icicle Spear").multi_hit,
        MultiHit::Range { min: 2, max: 5 }
    );
    assert_eq!(get("Double Kick").multi_hit, MultiHit::Fixed(2));
    assert_eq!(
        get("Double-Edge").recoil,
        Some(Recoil::Dealt(Fraction::new(33, 100)))
    );
    assert_eq!(
        get("Head Smash").recoil,
        Some(Recoil::Dealt(Fraction::new(1, 2)))
    );
    // Showdown hard-codes Chloroblast's recoil by id: no `recoil` key in the snapshot.
    assert_eq!(
        get("Chloroblast").recoil,
        Some(Recoil::UserMaxHp(Fraction::new(1, 2)))
    );
    assert_eq!(get("Giga Drain").drain, Some(Fraction::new(1, 2)));
    assert_eq!(get("Draining Kiss").drain, Some(Fraction::new(3, 4)));
    for name in [
        "Icicle Spear",
        "Double Kick",
        "Double-Edge",
        "Giga Drain",
        "Bullet Seed",
    ] {
        assert_eq!(get(name).primary_effect, PrimaryEffect::Damage, "{name}");
    }
    // Per-hit accuracy (Triple Kick, Population Bomb) and smart targeting (Dragon Darts) are
    // data.
    for name in ["Population Bomb", "Triple Kick"] {
        assert!(get(name).multi_accuracy, "{name}");
        assert!(
            get(name).unsupported.is_empty(),
            "{name}: {:?}",
            get(name).unsupported
        );
    }
    let darts = get("Dragon Darts");
    assert!(darts.smart_target);
    assert!(darts.unsupported.is_empty(), "{:?}", darts.unsupported);

    let hits = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .hits()
    };
    assert_eq!(hits(Generation::Gen9).multi_hit_weights, [35, 35, 15, 15]);
    assert_eq!(hits(Generation::Gen4).multi_hit_weights, [3, 3, 1, 1]);
    assert!(
        hits(Generation::Gen1).same_damage_each_hit && !hits(Generation::Gen2).same_damage_each_hit
    );
    assert_eq!(hits(Generation::Gen1).recoil_timing, RecoilTiming::PerHit);
    assert_eq!(hits(Generation::Gen4).recoil_rounding, Rounding::Floor);
    assert_eq!(hits(Generation::Gen5).recoil_rounding, Rounding::HalfUp);
    assert_eq!(
        (
            hits(Generation::Gen4).drain_minimum,
            hits(Generation::Gen5).drain_minimum
        ),
        (1, 0)
    );
}

#[test]
fn two_to_five_hits_decide_the_count_once_and_roll_every_hit() {
    let mut arena = setup(
        Generation::Gen9,
        ("Cloyster", "Icicle Spear"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    // Count 4, then four independent (critical, roll) pairs; Snorlax's Tackle afterwards.
    let mut tape = hits(4, 100);
    tape.extend(hit(100));
    let start = arena.turn(tape);

    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::MultiHitResolved {
            actor: P1,
            target: P2,
            count: 4,
            ..
        }
    )));
    let on_snorlax: Vec<u16> = arena
        .damages(start)
        .into_iter()
        .filter(|(t, _)| *t == P2)
        .map(|(_, a)| a)
        .collect();
    assert_eq!(on_snorlax.len(), 4, "four hits, four HP commits");
    assert!(
        on_snorlax.iter().all(|a| *a == on_snorlax[0]),
        "same roll → same damage"
    );
    let crits = arena
        .events(start)
        .iter()
        .filter(|e| matches!(e, TraceEvent::CriticalChecked { actor: P1, .. }))
        .count();
    assert_eq!(crits, 4, "every hit rolls its own critical");
}

#[test]
fn a_fixed_count_move_strikes_exactly_that_many_times_without_a_count_roll() {
    let mut arena = setup(
        Generation::Gen9,
        ("Hitmonlee", "Double Kick"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MultiHitResolved { count: 2, .. }))
    );
    assert_eq!(
        arena
            .damages(start)
            .iter()
            .filter(|(t, _)| *t == P2)
            .count(),
        2
    );
}

#[test]
fn a_faint_stops_the_sequence_early() {
    let mut arena = setup(
        Generation::Gen9,
        ("Cloyster", "Icicle Spear"),
        ("Snorlax", "Tackle"),
        |_, b| {
            b.hp = 1;
        },
    );
    // Five hits were decided, one lands: the target is down, no more rolls, and Snorlax
    // never gets to act.
    let mut tape = vec![RandomOverride::range(RandomRequestKind::MultiHitCount, 5)];
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert_eq!(arena.damages(start), vec![(P2, 1)]);
    assert!(arena.pokemon(P2).is_fainted());
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::Fainted { pokemon: P2, .. }))
    );
}

#[test]
fn every_hit_goes_through_the_substitute_separately() {
    let mut arena = setup(
        Generation::Gen9,
        ("Cloyster", "Icicle Spear"),
        ("Snorlax", "Tackle"),
        |_, b| {
            b.volatiles.set_substitute(P2, 1);
        },
    );
    let mut tape = hits(3, 100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    let substitute_hits: Vec<bool> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::SubstituteDamaged {
                target: P2, broken, ..
            } => Some(*broken),
            _ => None,
        })
        .collect();
    assert_eq!(substitute_hits, vec![true], "hit 1 breaks the Substitute");
    assert_eq!(
        arena
            .damages(start)
            .iter()
            .filter(|(t, _)| *t == P2)
            .count(),
        2,
        "hits 2 and 3 reach the body"
    );
}

#[test]
fn recoil_is_taken_once_from_the_total_damage_rounded_half_up() {
    let mut arena = setup(
        Generation::Gen9,
        ("Tauros", "Double-Edge"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    let damages = arena.damages(start);
    let dealt = damages.iter().find(|(t, _)| *t == P2).unwrap().1;
    let recoil = damages.iter().find(|(t, _)| *t == P1).unwrap().1;
    assert_eq!(u32::from(recoil), (u32::from(dealt) * 33 + 50) / 100);
    // Attributed to the move, ordered right after the hit, before Snorlax acts.
    let position =
        |pred: &dyn Fn(&TraceEvent) -> bool| arena.events(start).iter().position(pred).unwrap();
    let recoil_at = position(&|e| {
        matches!(
            e,
            TraceEvent::DamageApplied {
                target: P1,
                source: EffectSource::Move { pokemon: P1, .. },
                ..
            }
        )
    });
    let tackle_at = position(&|e| matches!(e, TraceEvent::MoveUsed { actor: P2, .. }));
    assert!(recoil_at < tackle_at);
}

#[test]
fn recoil_floors_before_gen5_and_never_drops_below_one() {
    let mut arena = setup(
        Generation::Gen4,
        ("Tauros", "Double-Edge"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let Some(Recoil::Dealt(fraction)) = arena.dex.move_by_name("Double-Edge").unwrap().recoil
    else {
        panic!("Double-Edge recoils from the damage dealt");
    };
    let mut tape = attack(&arena, P1, 100);
    tape.extend(attack(&arena, P2, 100));
    let start = arena.turn(tape);
    let damages = arena.damages(start);
    let dealt = damages.iter().find(|(t, _)| *t == P2).unwrap().1;
    let recoil = damages.iter().find(|(t, _)| *t == P1).unwrap().1;
    assert_eq!(
        u32::from(recoil),
        (u32::from(dealt) * u32::from(fraction.numerator) / u32::from(fraction.denominator)).max(1)
    );
}

#[test]
fn chloroblast_recoils_half_the_users_max_hp_whatever_it_dealt() {
    let mut arena = setup(
        Generation::Gen9,
        ("Venusaur", "Chloroblast"),
        ("Blissey", "Tackle"),
        |_, _| {},
    );
    let max_hp = arena.pokemon(P1).max_hp;
    let mut tape = attack(&arena, P1, 85);
    tape.extend(attack(&arena, P2, 85));
    let start = arena.turn(tape);
    let damages = arena.damages(start);
    let recoil = damages.iter().find(|(t, _)| *t == P1).unwrap().1;
    assert_eq!(recoil, max_hp.div_ceil(2), "Math.round(maxhp / 2)");
}

#[test]
fn gen1_multi_hit_deals_the_first_hits_damage_on_every_hit() {
    let mut arena = setup(
        Generation::Gen1,
        ("Beedrill", "Pin Missile"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    // Accuracy once, one count roll, one (critical, roll) pair for the whole sequence, then
    // Snorlax.
    let mut tape = vec![
        RandomOverride::bool(RandomRequestKind::Accuracy, true),
        RandomOverride::range(RandomRequestKind::MultiHitCount, 3),
    ];
    // Gen 1-2 roll 217..=255.
    tape.extend(hit(255));
    tape.extend(attack(&arena, P2, 255));
    let start = arena.turn(tape);
    let on_snorlax: Vec<u16> = arena
        .damages(start)
        .into_iter()
        .filter(|(t, _)| *t == P2)
        .map(|(_, a)| a)
        .collect();
    assert_eq!(on_snorlax.len(), 3);
    assert!(on_snorlax.iter().all(|a| *a == on_snorlax[0]));
    assert_eq!(
        arena
            .events(start)
            .iter()
            .filter(|e| matches!(e, TraceEvent::CriticalChecked { actor: P1, .. }))
            .count(),
        1
    );
}

#[test]
fn drain_heals_half_the_damage_rounded_half_up_and_nothing_at_full_hp() {
    let mut arena = setup(
        Generation::Gen9,
        ("Venusaur", "Giga Drain"),
        ("Snorlax", "Tackle"),
        |a, _| {
            a.hp = a.max_hp - 60;
        },
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    let dealt = arena
        .damages(start)
        .iter()
        .find(|(t, _)| *t == P2)
        .unwrap()
        .1;
    assert_eq!(arena.heals(start), vec![(P1, dealt.div_ceil(2))]);

    // At full HP the heal request is refused and no HealApplied is traced (Showdown prints no
    // `-heal` either).
    let mut arena = setup(
        Generation::Gen9,
        ("Venusaur", "Giga Drain"),
        ("Snorlax", "Tackle"),
        |_, _| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert!(arena.heals(start).is_empty());
}

#[test]
fn drain_floors_before_gen5_with_a_minimum_of_one() {
    let mut arena = setup(
        Generation::Gen4,
        ("Venusaur", "Giga Drain"),
        ("Snorlax", "Tackle"),
        |a, _| {
            a.hp = a.max_hp - 60;
        },
    );
    let mut tape = attack(&arena, P1, 100);
    tape.extend(attack(&arena, P2, 100));
    let start = arena.turn(tape);
    let dealt = arena
        .damages(start)
        .iter()
        .find(|(t, _)| *t == P2)
        .unwrap()
        .1;
    assert_eq!(arena.heals(start), vec![(P1, (dealt / 2).max(1))]);
}

#[test]
fn drain_off_a_substitute_rounds_up() {
    let mut arena = setup(
        Generation::Gen9,
        ("Venusaur", "Giga Drain"),
        ("Snorlax", "Tackle"),
        |a, b| {
            a.hp = a.max_hp - 60;
            b.volatiles.set_substitute(P2, 200);
        },
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    let absorbed = arena
        .events(start)
        .iter()
        .find_map(|e| match e {
            TraceEvent::SubstituteDamaged {
                target: P2, amount, ..
            } => Some(*amount),
            _ => None,
        })
        .unwrap();
    assert!(
        arena.damages(start).iter().all(|(t, _)| *t != P2),
        "the body was not touched"
    );
    assert_eq!(arena.heals(start), vec![(P1, absorbed.div_ceil(2))]);
}

// ---------------------------------------------------------------------------------------------
// Contact reactions (ROADMAP §6.5): the target's ability / item strike back per hit.
// ---------------------------------------------------------------------------------------------

fn with(species: &str, mv: &str, ability: Option<&str>, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, [mv]);
    set.ability = ability.map(Into::into);
    set.item = item.map(Into::into);
    set
}

/// Sources of every HP loss `victim` took since `from`, in order.
fn sources_on(arena: &Arena, from: usize, victim: PokemonId) -> Vec<EffectSource> {
    arena
        .events(from)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::DamageApplied { target, source, .. } if *target == victim => Some(*source),
            _ => None,
        })
        .collect()
}

#[test]
fn contact_reactions_fire_per_hit_ability_before_item() {
    let mut arena = setup_sets(
        Generation::Gen9,
        with("Snorlax", "Double Kick", None, None),
        with(
            "Garchomp",
            "Tackle",
            Some("Rough Skin"),
            Some("Rocky Helmet"),
        ),
        |_, _| {},
    );
    let max_hp = arena.pokemon(P1).max_hp;
    // Garchomp (faster) Tackles first, then Double Kick's two hits each take Rough Skin then
    // Rocky Helmet.
    let mut tape = hit(100);
    tape.extend(hit(100));
    tape.extend(hit(100));
    let start = arena.turn(tape);

    let rough_skin = EffectSource::Ability {
        pokemon: P2,
        ability: AbilityId::ROUGH_SKIN,
    };
    let helmet = EffectSource::Item {
        pokemon: P2,
        item: ItemId::ROCKY_HELMET,
    };
    let sources = sources_on(&arena, start, P1);
    assert!(matches!(sources[0], EffectSource::Move { pokemon: P2, .. }));
    assert_eq!(sources[1..], [rough_skin, helmet, rough_skin, helmet]);
    let amounts: Vec<u16> = arena
        .damages(start)
        .into_iter()
        .filter(|(t, _)| *t == P1)
        .map(|(_, a)| a)
        .collect();
    assert_eq!(
        amounts[1..],
        [max_hp / 8, max_hp / 6, max_hp / 8, max_hp / 6]
    );
    // Each shows on its damage's line alone (the source above), nothing before it: one Magic
    // Guard keeps off shows nothing (R-M10-37).
    assert_eq!(
        arena
            .events(start)
            .iter()
            .filter(|e| matches!(
                e,
                TraceEvent::AbilityActivated {
                    source: P2,
                    ability: AbilityId::ROUGH_SKIN
                } | TraceEvent::ItemActivated {
                    holder: P2,
                    item: ItemId::ROCKY_HELMET
                }
            ))
            .count(),
        0
    );
}

#[test]
fn non_contact_hits_and_hits_on_a_substitute_do_not_trigger_contact_reactions() {
    // Icicle Spear has no contact flag.
    let mut arena = setup_sets(
        Generation::Gen9,
        with("Snorlax", "Icicle Spear", None, None),
        with(
            "Garchomp",
            "Tackle",
            Some("Rough Skin"),
            Some("Rocky Helmet"),
        ),
        |_, _| {},
    );
    let mut tape = hit(100);
    tape.extend(hits(2, 100));
    let start = arena.turn(tape);
    assert!(
        sources_on(&arena, start, P1)
            .iter()
            .all(|s| matches!(s, EffectSource::Move { .. }))
    );

    // Tackle makes contact but only reaches the Substitute.
    let mut arena = setup_sets(
        Generation::Gen9,
        with("Snorlax", "Tackle", None, None),
        with(
            "Garchomp",
            "Tackle",
            Some("Rough Skin"),
            Some("Rocky Helmet"),
        ),
        |_, b| b.volatiles.set_substitute(P2, 300),
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert!(
        sources_on(&arena, start, P1)
            .iter()
            .all(|s| matches!(s, EffectSource::Move { .. }))
    );
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::SubstituteDamaged { target: P2, .. }))
    );
}

#[test]
fn a_reaction_that_faints_the_attacker_ends_the_sequence() {
    let mut arena = setup_sets(
        Generation::Gen9,
        with("Snorlax", "Double Kick", None, None),
        with(
            "Garchomp",
            "Swords Dance",
            Some("Rough Skin"),
            Some("Rocky Helmet"),
        ),
        |a, _| a.hp = a.max_hp / 8 + a.max_hp / 6,
    );
    // Swords Dance asks nothing; Double Kick's first hit is answered by Rough Skin, then Rocky
    // Helmet finishes Snorlax, and the second hit never happens.
    let start = arena.turn(hit(100));
    assert_eq!(
        arena
            .damages(start)
            .iter()
            .filter(|(t, _)| *t == P2)
            .count(),
        1
    );
    assert!(arena.pokemon(P1).is_fainted());
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::BattleFinished { .. }))
    );
}

#[test]
fn gen3_rough_skin_costs_one_sixteenth() {
    let mut arena = setup_sets(
        Generation::Gen3,
        with("Snorlax", "Tackle", None, None),
        with("Sharpedo", "Tackle", Some("Rough Skin"), None),
        |_, _| {},
    );
    let max_hp = arena.pokemon(P1).max_hp;
    let mut tape = attack(&arena, P2, 100);
    tape.extend(attack(&arena, P1, 100));
    let start = arena.turn(tape);
    let reaction = arena
        .events(start)
        .iter()
        .find_map(|e| match e {
            TraceEvent::DamageApplied {
                target: P1,
                amount,
                source:
                    EffectSource::Ability {
                        ability: AbilityId::ROUGH_SKIN,
                        ..
                    },
                ..
            } => Some(*amount),
            _ => None,
        })
        .expect("Rough Skin reaction");
    assert_eq!(reaction, max_hp / 16);
}

#[test]
fn the_drain_heals_as_the_damage_lands_before_the_targets_reactions() {
    // Showdown `spreadDamage` heals the drain right after the damage; Rough Skin answers in
    // DamagingHit afterwards. A user at full HP heals nothing, whatever Rough Skin then takes.
    let mut arena = setup_sets(
        Generation::Gen9,
        with("Breloom", "Drain Punch", None, None),
        with("Garchomp", "Swords Dance", Some("Rough Skin"), None),
        |_, _| {},
    );
    let start = arena.turn(hit(100));
    let user_events: Vec<&str> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::HealApplied { target: P1, .. } => Some("heal"),
            TraceEvent::DamageApplied { target: P1, .. } => Some("rough skin"),
            _ => None,
        })
        .collect();
    assert_eq!(user_events, vec!["rough skin"]);
    // Now hurt, the next Drain Punch heals first, then Rough Skin strikes.
    let start = arena.turn(hit(100));
    let user_events: Vec<&str> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::HealApplied { target: P1, .. } => Some("heal"),
            TraceEvent::DamageApplied { target: P1, .. } => Some("rough skin"),
            _ => None,
        })
        .collect();
    assert_eq!(user_events, vec!["heal", "rough skin"]);
}

#[test]
fn a_secondarys_self_part_lands_through_a_substitute() {
    // Flame Charge's certain Speed boost is a secondary's `self` part: it rolls in the
    // secondaries step and reaches the user even when the hit only struck a Substitute.
    let mut arena = setup_sets(
        Generation::Gen9,
        with("Lucario", "Flame Charge", None, None),
        with("Snorlax", "Swords Dance", None, None),
        |_, b| b.volatiles.set_substitute(P2, 300),
    );
    arena.turn(hit(100));
    assert_eq!(arena.pokemon(P1).stages.speed, 1);
}

#[test]
fn a_fire_hit_thaws_its_target_only_after_the_secondaries() {
    // `frz`'s `onDamagingHit` thaws after the secondaries step: Inferno's certain burn finds
    // the target still frozen, so it ends neither frozen nor burned.
    let mut arena = setup_sets(
        Generation::Gen9,
        with("Arcanine", "Inferno", None, None),
        with("Snorlax", "Swords Dance", None, None),
        |_, b| b.status = Some(StatusState::new(NonVolatileStatus::Freeze)),
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(tape);
    assert_eq!(arena.pokemon(P2).status, None);
}

#[test]
fn from_gen_7_tryhit_comes_before_the_type_immunity() {
    // Showdown `moveSteps`: Gen 7+ run TryHit (Volt Absorb) before the type immunity, Gen 5-6
    // after it. A hurt Ground type with Volt Absorb heals from an Electric move only in Gen 7+.
    for (generation, heals) in [(Generation::Gen6, false), (Generation::Gen9, true)] {
        let mut arena = setup_sets(
            generation,
            with("Pikachu", "Thunder Shock", None, None),
            with("Whiscash", "Tackle", Some("Volt Absorb"), None),
            |_, b| b.hp = b.max_hp / 2,
        );
        let start = arena.turn(hit(100));
        assert_eq!(
            arena
                .events(start)
                .iter()
                .any(|e| matches!(e, TraceEvent::HealApplied { target: P2, .. })),
            heals,
            "{generation:?}"
        );
    }
}

#[test]
fn a_burn_halves_a_confusion_self_hit_only_in_gen_3_and_4() {
    // Gen 3-4 hit themselves through the whole formula (`getDamage`), from Gen 5 through
    // `getConfusionDamage`, which reads the stats alone.
    for (generation, halved) in [(Generation::Gen4, true), (Generation::Gen9, false)] {
        let self_hit = |burned: bool| {
            let mut arena = setup_sets(
                generation,
                with("Snorlax", "Tackle", None, None),
                with("Chansey", "Swords Dance", None, None),
                |a, _| {
                    a.volatiles.set_confusion(P2, 3);
                    if burned {
                        a.status = Some(StatusState::new(NonVolatileStatus::Burn));
                    }
                },
            );
            let tape = vec![
                RandomOverride::bool(RandomRequestKind::ConfusionSelfHit, true),
                RandomOverride::range(RandomRequestKind::DamageRoll, 100),
            ];
            let start = arena.turn(tape);
            arena
                .events(start)
                .iter()
                .find_map(|e| match e {
                    TraceEvent::ConfusionSelfHit { damage, .. } => Some(*damage),
                    _ => None,
                })
                .unwrap()
        };
        let (plain, burned) = (self_hit(false), self_hit(true));
        assert_eq!(burned < plain, halved, "{generation:?}");
    }
}

#[test]
fn a_gen_4_confusion_self_hit_takes_the_attack_modifiers() {
    // Showdown gen4 `getDamage(pokemon, pokemon, 40)`: the whole formula, so Choice Band's
    // Attack boost raises the self-hit; from Gen 5 `getConfusionDamage` ignores items.
    for (generation, boosted) in [(Generation::Gen4, true), (Generation::Gen9, false)] {
        let self_hit = |item: Option<&str>| {
            let mut arena = setup_sets(
                generation,
                with("Snorlax", "Tackle", None, item),
                with("Chansey", "Swords Dance", None, None),
                |a, _| {
                    a.volatiles.set_confusion(P2, 3);
                },
            );
            let tape = vec![
                RandomOverride::bool(RandomRequestKind::ConfusionSelfHit, true),
                RandomOverride::range(RandomRequestKind::DamageRoll, 100),
            ];
            let start = arena.turn(tape);
            arena
                .events(start)
                .iter()
                .find_map(|e| match e {
                    TraceEvent::ConfusionSelfHit { damage, .. } => Some(*damage),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(
            self_hit(Some("Choice Band")) > self_hit(None),
            boosted,
            "{generation:?}"
        );
    }
}

#[test]
fn a_gen_4_expert_belt_boosts_a_super_effective_hit_in_modify_damage() {
    // Only Life Orb moves to Gen 4's `ModifyDamagePhase2`; Expert Belt stays in ModifyDamage.
    let damage = |item: Option<&str>| {
        let mut jolteon = PokemonSet::new("Jolteon", ["Thunderbolt"]);
        jolteon.item = item.map(Into::into);
        let mut arena = setup_sets(
            Generation::Gen4,
            jolteon,
            PokemonSet::new("Milotic", ["Growl"]),
            |_, _| {},
        );
        let start = arena.turn(hit(100));
        arena.damages(start)[0]
    };
    let (target, plain) = damage(None);
    let (_, belted) = damage(Some("Expert Belt"));
    assert_eq!(target, P2);
    assert!(
        belted > plain && belted <= plain * 6 / 5 + 1,
        "{plain} → {belted}"
    );
}

#[test]
fn from_gen_4_ingrain_grounds_a_flying_type() {
    // Showdown `isGrounded`: Ingrain roots its user from Gen 4, and a Ground move's immunity is
    // groundedness alone — Skarmory takes Earthquake, the Flying type counted neutral.
    for (generation, tape, hit_lands) in [
        (Generation::Gen3, Vec::new(), false),
        (Generation::Gen4, hit(100), true),
    ] {
        let mut arena = setup(
            generation,
            ("Marowak", "Earthquake"),
            ("Skarmory", "Growl"),
            |_, skarmory| {
                skarmory.volatiles.set_ingrain();
            },
        );
        let start = arena.turn(tape);
        assert_eq!(
            !arena.damages(start).is_empty(),
            hit_lands,
            "{generation:?}"
        );
        if hit_lands {
            assert!(arena.events(start).iter().any(|e| matches!(
                e,
                TraceEvent::TypeEffectivenessResolved { target, modifier, .. }
                    if *target == P2 && *modifier == Fraction::new(2, 1)
            )));
        }
    }
}

#[test]
fn moves_that_ignore_evasion_roll_no_accuracy_against_raised_evasion() {
    // Showdown `ignoreEvasion` (Chip Away, Darkest Lariat, Sacred Sword): the accuracy check
    // leaves the target's evasion stage out, so a 100 % move never rolls against +6 evasion; a
    // plain 100 % move rolls at 100 / 3, truncated to 33 from Gen 5 (R83). The replay cannot see an unrolled hit (it asks the engine
    // whether to plan one), so this pins it directly.
    for (generation, sacred_sword_like) in [
        (Generation::Gen5, "Chip Away"),
        (Generation::Gen7, "Darkest Lariat"),
        (Generation::Gen9, "Sacred Sword"),
    ] {
        let arena = setup(
            generation,
            ("Lucario", sacred_sword_like),
            ("Snorlax", "Tackle"),
            |_, snorlax| snorlax.stages.evasion = 6,
        );
        let state = arena.engine.state();
        let rules = &arena.rules;
        let ignoring = arena.dex.move_by_name(sacred_sword_like).unwrap();
        assert_eq!(
            pokemon_battle_engine::engine::accuracy_basis_points(state, rules, P1, P2, ignoring),
            10_000,
            "{sacred_sword_like}"
        );
        let plain = arena.dex.move_by_name("Close Combat").unwrap();
        assert_eq!(
            pokemon_battle_engine::engine::accuracy_basis_points(state, rules, P1, P2, plain),
            3_300,
            "{generation:?}"
        );
    }
}

/// How the turn's Dream Eater / Endeavor ended: blocked at the hit gate or failed before it.
fn stopped(arena: &Arena, from: usize) -> Option<Result<HitBlockReason, MoveFailReason>> {
    arena.events(from).iter().find_map(|e| match e {
        TraceEvent::HitBlocked {
            target: P2, reason, ..
        } => Some(Ok(*reason)),
        TraceEvent::MoveFailed {
            actor: P1, reason, ..
        } => Some(Err(*reason)),
        _ => None,
    })
}

#[test]
fn a_moves_own_immunity_comes_before_tryhit_only_in_gen_1_2() {
    // Showdown `onTryImmunity`: Gen 1-2's `tryMoveHit` asks it right after the type immunity,
    // Gen 5-6 after TryHit (`hitStepTryImmunity` follows `hitStepTryHitEvent`), Gen 7+ too.
    // An awake Protect user facing Dream Eater is immune in Gen 2 and protected from Gen 5 —
    // both leave nothing the replay can see.
    for (generation, reason) in [
        (Generation::Gen2, HitBlockReason::MoveRule),
        (Generation::Gen6, HitBlockReason::Protect),
        (Generation::Gen9, HitBlockReason::Protect),
    ] {
        let mut arena = setup(
            generation,
            ("Hypno", "Dream Eater"),
            ("Snorlax", "Protect"),
            |_, _| {},
        );
        let start = arena.turn(Vec::new());
        assert_eq!(stopped(&arena, start), Some(Ok(reason)), "{generation:?}");
    }
}

#[test]
fn gen_3_4_endeavor_fails_before_the_hit_gate_later_ones_report_immunity() {
    // The gen4 mod (Gen 3 inherits it) adds an `onTry` to Endeavor: with no less HP than the
    // target the move fails (`-fail`) before any hit step; from Gen 5 only the `onTryImmunity`
    // is left (`-immune`). Neither changes any HP.
    for (generation, outcome) in [
        (Generation::Gen3, Err(MoveFailReason::NoEffect)),
        (Generation::Gen4, Err(MoveFailReason::NoEffect)),
        (Generation::Gen5, Ok(HitBlockReason::MoveRule)),
    ] {
        let mut arena = setup(
            generation,
            ("Hypno", "Endeavor"),
            ("Snorlax", "Growl"),
            |hypno, snorlax| snorlax.hp = hypno.hp,
        );
        let start = arena.turn(Vec::new());
        assert_eq!(stopped(&arena, start), Some(outcome), "{generation:?}");
    }
}

#[test]
fn gen_4_5_chatter_rolls_its_confusion_only_for_chatot() {
    // The gen5 mod's `onModifyMove` (Gen 4 inherits it) zeroes Chatter's confusion chance for
    // any user but Chatot: nothing is rolled. From Gen 6 every user's Chatter confuses. The
    // replay plans the roll through the same `secondary_roll`, so only this pins it.
    use pokemon_battle_engine::engine::SecondaryRoll;
    for (generation, user, expected) in [
        (
            Generation::Gen4,
            "Chatot",
            SecondaryRoll::Chance(3_100, 10_000),
        ),
        (Generation::Gen4, "Pidgeot", SecondaryRoll::Skipped),
        (
            Generation::Gen5,
            "Chatot",
            SecondaryRoll::Chance(1_000, 10_000),
        ),
        (Generation::Gen5, "Pidgeot", SecondaryRoll::Skipped),
        (Generation::Gen6, "Pidgeot", SecondaryRoll::Certain),
    ] {
        let arena = setup(
            generation,
            (user, "Chatter"),
            ("Snorlax", "Growl"),
            |_, _| {},
        );
        let chatter = arena.dex.move_by_name("Chatter").unwrap();
        assert_eq!(
            pokemon_battle_engine::engine::secondary_roll(
                arena.engine.state(),
                &arena.rules,
                P1,
                P2,
                chatter,
                &chatter.secondaries[0],
            ),
            expected,
            "{generation:?} {user}"
        );
    }
}

#[test]
fn tri_attack_draws_its_status_after_the_chance_but_before_it_in_gen_2() {
    // Showdown `this.sample` in the secondary's `onHit` (Gen 3+): drawn only once the 20 %
    // succeeds. Gen 2 draws it in the move's `onHit` (`move.statusRoll`) on every hit, before
    // the chance. A failed chance shows nothing either way, so only the tape tells.
    let failed_chance = |generation: Generation, tape: Vec<RandomOverride>| {
        let mut arena = setup(
            generation,
            ("Porygon2", "Tri Attack"),
            ("Snorlax", "Harden"),
            |_, _| {},
        );
        arena.turn(tape);
        assert_eq!(arena.pokemon(P2).status, None, "{generation:?}");
    };
    let mut gen9 = hit(100);
    gen9.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    failed_chance(Generation::Gen9, gen9);
    // Gen 1-2 roll the damage in 217..=255.
    let mut gen2 = vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 255),
    ];
    gen2.push(RandomOverride::range(RandomRequestKind::Sample, 1));
    gen2.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    failed_chance(Generation::Gen2, gen2);
}

#[test]
fn gen1_multi_hit_secondaries_roll_on_the_last_hit_only() {
    // Gen 1-2 `moveHit` (`!move.multihit || move.lastHit`) and Gen 3's cartridge (issues.md
    // I35): Twineedle's poison rolls once, after its second hit; from Gen 4 every hit rolls. The
    // replay cannot tell one roll after the last hit from a failed one before it.
    for (generation, rolls) in [
        (Generation::Gen1, 1),
        (Generation::Gen2, 1),
        (Generation::Gen3, 1),
        (Generation::Gen4, 2),
    ] {
        let mut arena = setup(
            generation,
            ("Beedrill", "Twineedle"),
            ("Snorlax", "Harden"),
            |_, _| {},
        );
        // The seeded draws land both hits (the count of rolls does not depend on their value).
        let start = arena.turn(Vec::new());
        let events = arena.events(start);
        let damage_at: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches!(
                    e,
                    TraceEvent::DamageApplied {
                        target: P2,
                        source: pokemon_battle_engine::engine::EffectSource::Move { .. },
                        ..
                    }
                )
            })
            .map(|(at, _)| at)
            .collect();
        let rolled_at: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, TraceEvent::SecondaryEffectRolled { .. }))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(damage_at.len(), 2, "{generation:?}");
        assert_eq!(rolled_at.len(), rolls, "{generation:?}");
        assert!(rolled_at[0] > damage_at[2 - rolls], "{generation:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Focus Band and Focus Sash (DESIGN.md R196, issues.md I38 / I39).
// ---------------------------------------------------------------------------------------------

/// Gen 3-4 Focus Band (I38): a success on a hit that would not knock the holder out still arms
/// the band, and the next hit, which would, is held without a roll of its own. Gen 5 rolls only
/// on the hit that would knock it out. No frame shows the first hit's roll: the replay plans it
/// as a failure.
#[test]
fn a_gen_4_focus_band_armed_by_any_hit_holds_the_rest_of_the_move() {
    let band = |generation| {
        setup_sets(
            generation,
            with("Machamp", "Double Kick", None, None),
            with("Snorlax", "Harden", None, Some("Focus Band")),
            |_, _| {},
        )
    };
    // How much one hit deals (no band roll: nothing would knock Snorlax out).
    let mut probe = band(Generation::Gen5);
    let start = probe.turn([hit(100), hit(100)].concat());
    let one_hit = probe.damages(start)[0].1;
    for (generation, tape, left) in [
        (
            Generation::Gen4,
            [
                hit(100),
                vec![RandomOverride::bool(RandomRequestKind::FocusBand, true)],
                hit(100),
            ]
            .concat(),
            1,
        ),
        (
            Generation::Gen5,
            [
                hit(100),
                hit(100),
                vec![RandomOverride::bool(RandomRequestKind::FocusBand, false)],
            ]
            .concat(),
            0,
        ),
    ] {
        let mut arena = setup_sets(
            generation,
            with("Machamp", "Double Kick", None, None),
            with("Snorlax", "Harden", None, Some("Focus Band")),
            |_, snorlax| snorlax.hp = one_hit + 1,
        );
        arena.turn(tape);
        assert_eq!(arena.pokemon(P2).hp, left, "{generation:?}");
        assert_eq!(
            arena.pokemon(P2).held_item,
            Some(ItemId::FOCUS_BAND),
            "{generation:?}"
        );
    }
}

/// Gen 4 Focus Sash: the sash that held a hit goes once the move is over; and a confusion
/// self-hit from full HP is held and uses it up too (I39). Frames show no item.
#[test]
fn a_gen_4_focus_sash_goes_after_the_move_and_holds_a_confusion_self_hit() {
    let mut magikarp = with("Magikarp", "Harden", None, Some("Focus Sash"));
    magikarp.level = 5;
    let mut arena = setup_sets(
        Generation::Gen4,
        with("Machamp", "Double Kick", None, None),
        magikarp,
        |_, _| {},
    );
    arena.turn([hit(100), hit(100)].concat());
    assert_eq!(arena.pokemon(P2).hp, 1);
    assert_eq!(arena.pokemon(P2).held_item, None);

    let mut arena = setup_sets(
        Generation::Gen4,
        with("Snorlax", "Harden", None, None),
        with(
            "Shedinja",
            "Harden",
            Some("Wonder Guard"),
            Some("Focus Sash"),
        ),
        |_, shedinja| {
            shedinja.volatiles.set_confusion(P1, 3);
        },
    );
    arena.turn(vec![
        RandomOverride::bool(RandomRequestKind::ConfusionSelfHit, true),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ]);
    assert_eq!(arena.pokemon(P2).hp, 1);
    assert_eq!(arena.pokemon(P2).held_item, None);
}

/// Focus Band draws only where its roll can change something (a move's damage that would knock
/// the holder out, Gen 2 / Gen 5+): not on a hit it survives anyway, and not on a Gen 2
/// confusion self-hit (a `directDamage`, `confusion_self_hit_direct`). The replay plans nothing
/// there, so an extra draw would fall back to the PRNG: over many seeds the band must never hold.
#[test]
fn focus_band_draws_nothing_it_cannot_use() {
    for seed in 0..64 {
        // Gen 9: Seismic Toss takes 50 of Snorlax's HP; it survives, the band has nothing to do.
        let mut arena = setup_sets(
            Generation::Gen9,
            with("Jolteon", "Seismic Toss", None, None),
            with("Snorlax", "Harden", None, Some("Focus Band")),
            |jolteon, _| jolteon.level = 50,
        );
        let max_hp = arena.pokemon(P2).max_hp;
        let level = u16::from(arena.pokemon(P1).level);
        arena.turn_seeded(seed, vec![]);
        assert_eq!(arena.pokemon(P2).hp, max_hp - level, "seed {seed}");

        // Gen 2: a confusion self-hit that knocks the holder out.
        let mut arena = setup_sets(
            Generation::Gen2,
            with("Snorlax", "Harden", None, None),
            with("Magikarp", "Harden", None, Some("Focus Band")),
            |_, magikarp| {
                magikarp.hp = 1;
                magikarp.volatiles.set_confusion(P1, 3);
            },
        );
        arena.turn_seeded(
            seed,
            vec![RandomOverride::bool(
                RandomRequestKind::ConfusionSelfHit,
                true,
            )],
        );
        assert_eq!(arena.pokemon(P2).hp, 0, "seed {seed}");
    }
}

impl Arena {
    /// `turn` with the PRNG seeded `seed` behind the tape: a draw the tape does not plan comes
    /// from it.
    fn turn_seeded(&mut self, seed: u64, tape: Vec<RandomOverride>) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(seed, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let choices = [self.choice(SideId::P1, P1), self.choice(SideId::P2, P2)];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
    }
}

// ---------------------------------------------------------------------------------------------
// Micle Berry and Custap Berry (DESIGN.md R197, issues.md I42 / I43).
// ---------------------------------------------------------------------------------------------

/// Micle Berry's boost on the accuracy no frame shows: Gen 5+ `chainModify([4915, 4096])` once the
/// stages are in (Thunder at −1: 70 → 52 → 62, not 84 → 63); Gen 4 ×120 / 100 after the stage
/// ratio (70 × 75 / 100 = 52 → 62).
#[test]
fn micle_berry_raises_the_next_accuracy_check() {
    for (generation, boosted) in [(Generation::Gen9, 6_200), (Generation::Gen4, 6_200)] {
        for micle in [false, true] {
            let arena = setup(
                generation,
                ("Jolteon", "Thunder"),
                ("Snorlax", "Harden"),
                |jolteon, _| {
                    jolteon.stages.accuracy = -1;
                    if micle {
                        jolteon.volatiles.set_micle_berry(None);
                    }
                },
            );
            let thunder = arena.dex.move_by_name("Thunder").unwrap();
            assert_eq!(
                pokemon_battle_engine::engine::accuracy_basis_points(
                    arena.engine.state(),
                    &arena.rules,
                    P1,
                    P2,
                    thunder
                ),
                if micle { boosted } else { 5_200 },
                "{generation:?} {micle}"
            );
        }
    }
}

/// A Poison type's Toxic, sure to hit, uses the boost up in Gen 6-7: the sure hit is the `toxic`
/// volatile's `onSourceAccuracy`, in the `Accuracy` event beside Micle's own (`-enditem|p|Micle
/// Berry`). From Gen 8 the hit steps skip the event, and the boost stays (R-M10-63).
#[test]
fn a_poison_types_toxic_uses_micle_berry_up_in_gen_6_and_7_alone() {
    for (generation, spent) in [
        (Generation::Gen6, true),
        (Generation::Gen7, true),
        (Generation::Gen8, false),
    ] {
        let mut arena = setup(
            generation,
            ("Gengar", "Toxic"),
            ("Blissey", "Harden"),
            |gengar, _| {
                gengar.volatiles.set_micle_berry(Some(2));
            },
        );
        arena.turn(Vec::new());
        let micle = arena
            .pokemon(P1)
            .volatiles
            .contains(pokemon_battle_engine::state::VolatileConditionKind::MicleBerry);
        assert_eq!(micle, !spent, "{generation:?}");
        assert!(arena.pokemon(P2).status.is_some(), "{generation:?}");
    }
}

/// Custap Berry: Gen 5+ eats it as the turn's actions are ordered, Gen 4 as the holder's move
/// begins — so a holder a faster priority move knocks out first ate it only in Gen 9. No frame
/// shows the berry.
#[test]
fn custap_berry_is_eaten_as_the_turn_starts_from_gen_5() {
    for (generation, eaten) in [(Generation::Gen9, true), (Generation::Gen4, false)] {
        let mut arena = setup_sets(
            generation,
            with("Snorlax", "Seismic Toss", None, Some("Custap Berry")),
            with("Jolteon", "Quick Attack", None, None),
            |snorlax, _| snorlax.hp = 1,
        );
        let start = arena.turn(hit(100));
        assert_eq!(arena.pokemon(P1).hp, 0, "{generation:?}");
        let ate = arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::ItemEaten { holder: P1, .. }));
        assert_eq!(ate, eaten, "{generation:?}");
    }
}

/// Custap Berry under a foe's Unnerve: the holder cannot eat it, so it does not go first — in
/// the order the engine previews for a replay too (the queue's own eating would refuse it
/// anyway, so only the preview shows the check).
#[test]
fn unnerve_keeps_custap_berry_from_putting_its_holder_first() {
    for (ability, first) in [("Thick Fat", P1), ("Unnerve", P2)] {
        let arena = setup_sets(
            Generation::Gen9,
            with("Snorlax", "Seismic Toss", None, Some("Custap Berry")),
            with("Jolteon", "Seismic Toss", Some(ability), None),
            |snorlax, jolteon| {
                snorlax.hp = 50;
                jolteon.ability_state.unnerved = true;
            },
        );
        let choices = [arena.choice(SideId::P1, P1), arena.choice(SideId::P2, P2)];
        let preview = arena
            .engine
            .preview_action_order(&choices, &|_, _| false)
            .unwrap();
        assert_eq!(
            preview.order.first().and_then(|b| b.first()),
            Some(&first),
            "{ability}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Items a hit uses up (DESIGN.md R198).
// ---------------------------------------------------------------------------------------------

/// Absorb Bulb (`useItem`): used up whatever the stages (at +6 Special Attack the boost does
/// nothing, the bulb is gone all the same), and not by a holder the hit knocked out (`useItem`
/// needs HP). Neither shows in a frame.
#[test]
fn a_hit_item_is_used_whatever_the_stages_but_not_by_a_fainted_holder() {
    for (special_attack, hp, kept) in [(6, None, false), (0, Some(1), true)] {
        let mut arena = setup_sets(
            Generation::Gen9,
            with("Vaporeon", "Water Gun", None, None),
            with("Snorlax", "Harden", None, Some("Absorb Bulb")),
            |_, snorlax| {
                snorlax.stages.special_attack = special_attack;
                if let Some(hp) = hp {
                    snorlax.hp = hp;
                }
            },
        );
        arena.turn(hit(100));
        let snorlax = arena.pokemon(P2);
        assert_eq!(snorlax.held_item.is_some(), kept, "{special_attack} {hp:?}");
        assert_eq!(snorlax.stages.special_attack, special_attack);
    }
}
