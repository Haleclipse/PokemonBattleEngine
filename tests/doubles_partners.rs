//! L1 rule tests for M5 batch 1 (ROADMAP §9 "Helping Hand", "ally Ability modifiers",
//! DESIGN.md §22.5): partners as participants in each other's damage stages — Helping Hand,
//! Power Spot and Battery in the BasePower chain, Friend Guard at ModifyDamage, Telepathy at
//! TryHit — and the BasePower chain itself (Showdown `chainModify` / `modify`); from M8's third
//! campaign round (DESIGN.md R40), Gen 3's target-by-target spread hits and foes-only spread
//! modifier, and Gen 1-3's faint check after every action.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, HitBlockReason, ModifierSource, ModifierStage, MoveFailReason,
    SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, ResolvedRules, RulesResolver, SemiInvulnerableKind,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, Stat, StatusState, VolatileConditionKind,
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

fn set(species: &str, moves: &[&str], ability: &str, item: Option<&str>, level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set.level = level;
    set
}

fn doubles(p1: [PokemonSet; 2], p2: [PokemonSet; 2]) -> Arena {
    doubles_in(Generation::Gen9, p1, p2, |_| {})
}

fn doubles_in(
    generation: Generation,
    p1: [PokemonSet; 2],
    p2: [PokemonSet; 2],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
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

/// (actor, move slot, target side and slot).
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

    fn has(&self, from: usize, pred: impl Fn(&TraceEvent) -> bool) -> bool {
        self.events(from).iter().any(pred)
    }

    /// The BasePower chain steps traced for `actor`'s move, as (source, from, to).
    fn base_power_steps(&self, from: usize) -> Vec<(ModifierSource, u32, u32)> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::ModifierApplied {
                    stage: ModifierStage::BasePower,
                    source,
                    from,
                    to,
                    ..
                } => Some((*source, *from, *to)),
                _ => None,
            })
            .collect()
    }

    fn final_damage(&self, from: usize, target: PokemonId) -> u16 {
        self.events(from)
            .iter()
            .find_map(|e| match e {
                TraceEvent::DamageCalculated {
                    target: t,
                    final_damage,
                    ..
                } if *t == target => Some(*final_damage),
                _ => None,
            })
            .unwrap()
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn foes() -> [PokemonSet; 2] {
    [
        set("Snorlax", &["Swords Dance"], "Run Away", None, 100),
        set("Blissey", &["Swords Dance"], "Run Away", None, 100),
    ]
}

#[test]
fn helping_hand_boosts_the_partners_move_and_fails_once_it_has_moved() {
    let mut arena = doubles(
        [
            set("Clefable", &["Helping Hand"], "Run Away", None, 50),
            set(
                "Garchomp",
                &["Dragon Claw", "Helping Hand"],
                "Run Away",
                None,
                50,
            ),
        ],
        foes(),
    );
    // Clefable's Helping Hand (+5) on Garchomp; Dragon Claw 80 × 1.5 = 120 in its chain.
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P1, 1))),
            (P1B, 0, Some((SideId::P2, 0))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        hit(100),
    );
    assert_eq!(
        arena.base_power_steps(start),
        vec![(
            ModifierSource::Volatile(VolatileConditionKind::HelpingHand),
            80,
            120
        )]
    );
    // The volatile lasts the turn only.
    assert!(
        !arena
            .engine
            .state()
            .pokemon(P1B)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::HelpingHand)
    );
    // Both help each other: the faster Garchomp's succeeds (Clefable still has to move), the
    // slower Clefable's fails (Garchomp has moved).
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P1, 1))),
            (P1B, 1, Some((SideId::P1, 0))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        Vec::new(),
    );
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::VolatileStarted {
            target: P1A,
            kind: VolatileConditionKind::HelpingHand
        }
    )));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: P1A,
            reason: MoveFailReason::Condition(_),
            ..
        }
    )));
}

#[test]
fn helping_hand_reaches_a_partner_in_the_air_from_gen_5() {
    let mut arena = doubles(
        [
            set(
                "Clefable",
                &["Helping Hand", "Swords Dance"],
                "Run Away",
                None,
                100,
            ),
            set("Corviknight", &["Fly"], "Run Away", None, 50),
        ],
        foes(),
    );
    // Turn 1: Corviknight rises; turn 2: the faster Clefable helps it while it is still up.
    arena.turn(
        &[(P1A, 1, None), (P1B, 0, Some((SideId::P2, 0)))],
        &[(P2A, 0, None), (P2B, 0, None)],
        Vec::new(),
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P1, 1))),
            (P1B, 0, Some((SideId::P2, 0))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        tape,
    );
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::SemiInvulnerableDodged { target: P1B, .. }
    )));
    assert_eq!(
        arena.base_power_steps(start),
        vec![(
            ModifierSource::Volatile(VolatileConditionKind::HelpingHand),
            90,
            135
        )]
    );
    // Showdown's gen4 `hitStepInvulnerabilityEvent` has no Helping Hand exception.
    for (generation, reaches) in [(Generation::Gen4, false), (Generation::Gen5, true)] {
        let rules =
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
                .unwrap();
        assert_eq!(
            rules
                .two_turn()
                .bypasses(SemiInvulnerableKind::Flying, MoveId::HELPING_HAND),
            reaches,
            "{generation:?}"
        );
    }
}

#[test]
fn power_spot_and_battery_join_the_partners_chain_but_not_the_holders() {
    // Battery: a partner's special move gets [5325, 4096]; a physical one does not.
    let mut arena = doubles(
        [
            set("Rotom", &["Thunderbolt", "Tackle"], "Levitate", None, 50),
            set("Pikachu", &["Thunderbolt"], "Battery", None, 50),
        ],
        foes(),
    );
    let start = arena.turn(
        &[
            (P1A, 0, Some((SideId::P2, 0))),
            (P1B, 0, Some((SideId::P2, 1))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        {
            let mut tape = hit(100);
            tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
            tape.extend(hit(100));
            tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
            tape
        },
    );
    let steps = arena.base_power_steps(start);
    // Rotom's Thunderbolt 90 → 117 (modify(90, 5325)); Pikachu's own is not boosted.
    assert_eq!(
        steps,
        vec![(
            ModifierSource::AbilityInstance {
                holder: P1B,
                ability: AbilityId::BATTERY
            },
            90,
            117
        )]
    );
    let start = arena.turn(
        &[
            (P1A, 1, Some((SideId::P2, 0))),
            (P1B, 0, Some((SideId::P2, 1))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        {
            let mut tape = hit(100);
            tape.extend(hit(100));
            tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
            tape
        },
    );
    assert!(arena.base_power_steps(start).is_empty());

    // Power Spot: any move of the partner.
    let mut arena = doubles(
        [
            set("Rotom", &["Tackle"], "Levitate", None, 50),
            set("Clefairy", &["Swords Dance"], "Power Spot", None, 50),
        ],
        foes(),
    );
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        hit(100),
    );
    assert_eq!(
        arena.base_power_steps(start),
        vec![(
            ModifierSource::AbilityInstance {
                holder: P1B,
                ability: AbilityId::POWER_SPOT
            },
            40,
            52
        )]
    );
}

#[test]
fn the_base_power_chain_combines_before_it_rounds() {
    // Technician (6144, priority 30) then a type booster (4915, priority 15) on a 40-power
    // Bullet Punch: chained to (6144 × 4915 + 2048) >> 12 = 7373, applied once:
    // modify(40, 7373) = 72. Step by step it would be modify(modify(40, 6144), 4915) = 72 as
    // well here, but the trace shows the chain's running power.
    let mut arena = doubles(
        [
            set(
                "Scizor",
                &["Bullet Punch"],
                "Technician",
                Some("Metal Coat"),
                50,
            ),
            set("Clefairy", &["Swords Dance"], "Run Away", None, 50),
        ],
        foes(),
    );
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        hit(100),
    );
    assert_eq!(
        arena.base_power_steps(start),
        vec![
            (
                ModifierSource::AbilityInstance {
                    holder: P1A,
                    ability: AbilityId::TECHNICIAN
                },
                40,
                60
            ),
            (
                ModifierSource::ItemInstance {
                    holder: P1A,
                    item: ItemId::METAL_COAT
                },
                60,
                72
            ),
        ]
    );
}

#[test]
fn friend_guard_softens_hits_on_its_partner_and_telepathy_refuses_the_partners_move() {
    let quake = |friend_guard: bool| {
        let mut arena = doubles(
            [
                set("Garchomp", &["Earthquake"], "Rough Skin", None, 50),
                set("Metagross", &["Swords Dance"], "Telepathy", None, 50),
            ],
            [
                set(
                    "Snorlax",
                    &["Swords Dance"],
                    if friend_guard {
                        "Friend Guard"
                    } else {
                        "Run Away"
                    },
                    None,
                    100,
                ),
                set("Blissey", &["Swords Dance"], "Run Away", None, 100),
            ],
        );
        let mut tape = hit(100);
        tape.extend(hit(100));
        let start = arena.turn(
            &[(P1A, 0, None), (P1B, 0, None)],
            &[(P2A, 0, None), (P2B, 0, None)],
            tape,
        );
        // Telepathy: the partner is refused at TryHit, no decision drawn for it.
        assert!(arena.has(start, |e| matches!(
            e,
            TraceEvent::HitBlocked {
                target: P1B,
                reason: HitBlockReason::Ability(AbilityId::TELEPATHY),
                ..
            }
        )));
        (
            arena.final_damage(start, P2A),
            arena.final_damage(start, P2B),
        )
    };
    let (snorlax_plain, blissey_plain) = quake(false);
    let (snorlax_guarded, blissey_guarded) = quake(true);
    // The holder takes full damage; its partner ×0.75 (3072 / 4096, Showdown `modify`).
    assert_eq!(snorlax_guarded, snorlax_plain);
    assert_eq!(
        u32::from(blissey_guarded),
        (u32::from(blissey_plain) * 3072 + 2047) / 4096
    );
}

#[test]
fn a_spread_move_lands_on_every_target_before_any_reaction() {
    // Showdown `spreadMoveHit`: every target's damage lands before the `DamagingHit`
    // reactions, so Rough Skin answers after both foes are hit.
    let mut arena = doubles(
        [
            set("Kommo-o", &["Breaking Swipe"], "Run Away", None, 50),
            set("Clefable", &["Swords Dance"], "Run Away", None, 50),
        ],
        [
            set("Garchomp", &["Swords Dance"], "Rough Skin", None, 50),
            set("Snorlax", &["Swords Dance"], "Run Away", None, 50),
        ],
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        tape,
    );
    let damaged: Vec<PokemonId> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::DamageApplied { target, .. } => Some(*target),
            _ => None,
        })
        .collect();
    assert_eq!(damaged, vec![P2A, P2B, P1A]);
}

#[test]
fn a_spread_moves_hit_on_a_substitute_pops_its_holders_air_balloon() {
    // Air Balloon's `onAfterSubDamage`: a move's hit on its holder's Substitute pops it — a
    // spread move's too, whose Substitutes take their hits ahead of the other targets (R-M10-50,
    // a Self-Destruct into a Bonsly's doll).
    let mut arena = doubles_in(
        Generation::Gen9,
        [
            set("Kommo-o", &["Breaking Swipe"], "Run Away", None, 50),
            set("Clefable", &["Swords Dance"], "Run Away", None, 50),
        ],
        [
            set(
                "Snorlax",
                &["Swords Dance"],
                "Run Away",
                Some("Air Balloon"),
                50,
            ),
            set("Blissey", &["Swords Dance"], "Run Away", None, 50),
        ],
        |pokemon| pokemon[2].volatiles.set_substitute(P2A, 200),
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        tape,
    );
    let snorlax = arena.engine.state().pokemon(P2A).unwrap();
    assert!(
        snorlax
            .volatiles
            .contains(VolatileConditionKind::Substitute)
    );
    assert_eq!(snorlax.held_item, None);
}

#[test]
fn a_knock_outs_raises_come_before_the_moves_after_move() {
    // Showdown's hit loop ends with `faintMessages`: the knocked-out foe's `Faint` (Soul-Heart)
    // and the batch's `AfterFaint` (Moxie) come before the move's `AfterMove`, where a foe's
    // Mirror Herb copies both — ahead of the Haze the next move brings (R-M10-52; Grim Neigh's
    // raise, a Lycanroc's Mirror Herb and a Revavroom's Haze in the Nightly's battle).
    let mut arena = doubles_in(
        Generation::Gen9,
        [
            set("Gyarados", &["Tackle"], "Moxie", None, 50),
            set("Clefable", &["Haze"], "Soul-Heart", None, 50),
        ],
        [
            set("Blissey", &["Splash"], "Natural Cure", None, 50),
            set("Snorlax", &["Splash"], "Thick Fat", Some("Mirror Herb"), 50),
        ],
        |pokemon| pokemon[2].hp = 1,
    );
    let start = arena.turn(
        &[(P1A, 0, Some((SideId::P2, 0))), (P1B, 0, None)],
        &[(P2A, 0, None), (P2B, 0, None)],
        hit(100),
    );
    let raised: Vec<(PokemonId, Stat)> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::StatChanged {
                target,
                stat,
                from,
                to,
                ..
            } if to > from => Some((*target, *stat)),
            _ => None,
        })
        .collect();
    assert_eq!(
        raised,
        vec![
            (P1B, Stat::SpecialAttack),
            (P1A, Stat::Attack),
            (P2B, Stat::SpecialAttack),
            (P2B, Stat::Attack),
        ]
    );
    let snorlax = arena.engine.state().pokemon(P2B).unwrap();
    assert_eq!(
        (snorlax.stages.attack, snorlax.stages.special_attack),
        (0, 0)
    );
    assert_eq!(snorlax.held_item, None);
}

/// Every traced application of the spread modifier since `from`, as (from, to).
fn spread_steps(arena: &Arena, from: usize) -> Vec<(u32, u32)> {
    arena
        .events(from)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::ModifierApplied {
                source: ModifierSource::Rule("Spread"),
                from,
                to,
                ..
            } => Some((*from, *to)),
            _ => None,
        })
        .collect()
}

#[test]
fn gen_3_hits_each_spread_target_in_full_and_halves_only_a_move_aimed_at_both_foes() {
    // Showdown gen3 `useMoveInner` runs `tryMoveHit` target by target (accuracy, critical,
    // damage, secondaries, then the next target), and its `modifyDamage` halves only an
    // `allAdjacentFoes` spread: Earthquake, which also reaches the partner, keeps full damage.
    let mut arena = doubles_in(
        Generation::Gen3,
        [
            set(
                "Marowak",
                &["Earthquake", "Rock Slide"],
                "Rock Head",
                None,
                50,
            ),
            set("Skarmory", &["Growl"], "Keen Eye", None, 50),
        ],
        [
            set("Snorlax", &["Growl"], "Thick Fat", None, 100),
            set("Blissey", &["Growl"], "Natural Cure", None, 100),
        ],
        |_| {},
    );
    let foes: &[Decl] = &[(P2A, 0, None), (P2B, 0, None)];
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(&[(P1A, 0, None), (P1B, 0, None)], foes, tape);
    let steps = spread_steps(&arena, start);
    assert_eq!(steps.len(), 2);
    assert!(steps.iter().all(|(from, to)| from == to), "{steps:?}");

    // Rock Slide's decisions come target by target; a staged spread would ask both
    // accuracies first and desynchronise this tape.
    let per_target = || {
        let mut decisions = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
        decisions.extend(hit(100));
        decisions.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
        decisions
    };
    let mut tape = per_target();
    tape.extend(per_target());
    let start = arena.turn(&[(P1A, 1, None), (P1B, 0, None)], foes, tape);
    let steps = spread_steps(&arena, start);
    assert_eq!(steps.len(), 2);
    assert!(steps.iter().all(|(from, to)| to < from), "{steps:?}");
}

#[test]
fn gen_1_to_3_clear_a_fainted_pokemons_status_after_every_action() {
    // Showdown `checkFainted` after every action followed by a move (Gen 1-3), replacement or
    // not: Magikarp, knocked out first, loses its paralysis; Slowpoke, whose faint ends the
    // battle before the check, keeps it.
    let mut arena = doubles_in(
        Generation::Gen3,
        [
            set("Mewtwo", &["Strength"], "Pressure", None, 100),
            set("Mew", &["Strength"], "Synchronize", None, 100),
        ],
        [
            set("Magikarp", &["Growl"], "Swift Swim", None, 10),
            set("Slowpoke", &["Growl"], "Oblivious", None, 10),
        ],
        |pokemon| {
            for fish in &mut pokemon[2..] {
                fish.status = Some(StatusState::new(NonVolatileStatus::Paralysis));
            }
        },
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    arena.turn(
        &[
            (P1A, 0, Some((SideId::P2, 0))),
            (P1B, 0, Some((SideId::P2, 1))),
        ],
        &[(P2A, 0, None), (P2B, 0, None)],
        tape,
    );
    let state = arena.engine.state();
    assert!(state.outcome.is_some());
    assert_eq!(state.pokemon(P2A).unwrap().status, None);
    assert!(
        state
            .pokemon(P2B)
            .unwrap()
            .has_status(NonVolatileStatus::Paralysis)
    );
}
