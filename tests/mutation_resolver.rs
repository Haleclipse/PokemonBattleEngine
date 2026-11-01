//! L1 rule tests for the Resolve stage of mutation requests (ROADMAP Batch E, DESIGN.md §20):
//! stat changes go ChangeBoost (Contrary / Simple) → cap → TryBoost (Clear Body) → commit, and
//! HP loss goes through the target's damage behavior (Magic Guard → Sturdy → Focus Sash) before
//! the commit. Every effect call site is unchanged: the behavior live on the request boundary.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EffectSource, EngineStep, SideChoice, SlotChoice, StatChangeBlockReason,
    TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, Stat, StatusState,
};

const P1: PokemonId = PokemonId(1);
const P2: PokemonId = PokemonId(2);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(
    species: &str,
    mv: &str,
    ability: Option<&str>,
    item: Option<&str>,
    level: u8,
) -> PokemonSet {
    let mut set = PokemonSet::new(species, [mv]);
    set.ability = ability.map(Into::into);
    set.item = item.map(Into::into);
    set.level = level;
    set
}

fn setup(
    p1: PokemonSet,
    p2: PokemonSet,
    prepare: impl FnOnce(&mut PokemonState, &mut PokemonState),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
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

    fn damages(&self, from: usize) -> Vec<(PokemonId, u16)> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::DamageApplied { target, amount, .. } => Some((*target, *amount)),
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

// ---------------------------------------------------------------------------------------------
// StatChangeRequest
// ---------------------------------------------------------------------------------------------

#[test]
fn clear_body_refuses_drops_from_others_but_not_its_own() {
    let mut arena = setup(
        set("Gyarados", "Growl", Some("Intimidate"), None, 50),
        set("Metagross", "Close Combat", Some("Clear Body"), None, 50),
        |_, _| {},
    );
    // Intimidate on entry is refused.
    arena.engine.run_switch_in_ability(P1);
    assert_eq!(arena.pokemon(P2).stages.attack, 0);
    assert!(arena.engine.trace().events().iter().any(|e| matches!(
        e,
        TraceEvent::StatChangeBlocked {
            target: P2,
            stat: Stat::Attack,
            stages: -1,
            reason: StatChangeBlockReason::Ability(AbilityId::CLEAR_BODY),
            ..
        }
    )));

    // Growl (from the foe) is refused; Close Combat's own drops go through.
    let start = arena.turn(hit(100));
    assert_eq!(arena.pokemon(P2).stages.attack, 0);
    assert_eq!(arena.pokemon(P2).stages.defense, -1);
    assert_eq!(arena.pokemon(P2).stages.special_defense, -1);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: P1,
            reason: pokemon_battle_engine::engine::MoveFailReason::NoEffect,
            ..
        }
    )));
}

#[test]
fn mirror_armor_sends_drops_back_but_lets_its_own_through() {
    let mut arena = setup(
        set("Gyarados", "Growl", Some("Intimidate"), None, 50),
        set("Corviknight", "Superpower", Some("Mirror Armor"), None, 50),
        |_, _| {},
    );
    // Intimidate goes back to its holder.
    arena.engine.run_switch_in_ability(P1);
    assert_eq!(arena.pokemon(P2).stages.attack, 0);
    assert_eq!(arena.pokemon(P1).stages.attack, -1);
    assert!(arena.engine.trace().events().iter().any(|e| matches!(
        e,
        TraceEvent::StatChanged {
            target: P1,
            stat: Stat::Attack,
            source: EffectSource::Ability {
                pokemon: P2,
                ability: AbilityId::MIRROR_ARMOR
            },
            ..
        }
    )));

    // Growl goes back too; the holder's own Superpower drops are neither refused nor sent
    // anywhere (Showdown `target === source` returns before Mirror Armor looks at them).
    let start = arena.turn(hit(100));
    assert_eq!(arena.pokemon(P1).stages.attack, -2);
    assert_eq!(arena.pokemon(P2).stages.attack, -1);
    assert_eq!(arena.pokemon(P2).stages.defense, -1);
    let refused: Vec<Stat> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::StatChangeBlocked {
                target: P2, stat, ..
            } => Some(*stat),
            _ => None,
        })
        .collect();
    assert_eq!(refused, vec![Stat::Attack], "only Growl's drop is refused");
}

#[test]
fn contrary_inverts_every_stat_change() {
    let mut arena = setup(
        set("Gyarados", "Growl", Some("Intimidate"), None, 50),
        set("Serperior", "Swords Dance", Some("Contrary"), None, 50),
        |_, _| {},
    );
    arena.engine.run_switch_in_ability(P1);
    assert_eq!(
        arena.pokemon(P2).stages.attack,
        1,
        "Intimidate raises a Contrary target"
    );
    assert!(arena.engine.trace().events().iter().any(|e| matches!(
        e,
        TraceEvent::StatChangeModified {
            target: P2,
            requested: -1,
            resolved: 1,
            by: EffectSource::Ability {
                ability: AbilityId::CONTRARY,
                ..
            },
            ..
        }
    )));
    // Serperior (faster) Swords Dances: -2; Growl: +1. The turn runs on a battle restored for
    // its tape, whose trace is that turn's alone (DESIGN.md §47.0 item 6).
    arena.turn(Vec::new());
    assert_eq!(arena.pokemon(P2).stages.attack, 0);
    let changes: Vec<(i8, i8)> = arena
        .engine
        .trace()
        .events()
        .iter()
        .filter_map(|e| match e {
            TraceEvent::StatChanged {
                target: P2,
                stat: Stat::Attack,
                from,
                to,
                ..
            } => Some((*from, *to)),
            _ => None,
        })
        .collect();
    assert_eq!(changes, vec![(1, -1), (-1, 0)]);
}

#[test]
fn simple_doubles_the_change_and_the_cap_still_holds() {
    let mut arena = setup(
        set("Snorlax", "Growl", None, None, 50),
        set("Numel", "Swords Dance", Some("Simple"), None, 50),
        |_, _| {},
    );
    // Numel (35 Spe) outspeeds Snorlax (30): Swords Dance +4 first, then Growl -2.
    arena.turn(Vec::new());
    assert_eq!(arena.pokemon(P2).stages.attack, 2);
    // +4 from +2 caps at +6 (the request is capped to +4 → +6), then -2.
    arena.turn(Vec::new());
    assert_eq!(arena.pokemon(P2).stages.attack, 4);
    assert!(arena.engine.trace().events().iter().any(|e| matches!(
        e,
        TraceEvent::StatChanged {
            target: P2,
            stat: Stat::Attack,
            from: 2,
            to: 6,
            ..
        }
    )));
}

// ---------------------------------------------------------------------------------------------
// DamageRequest
// ---------------------------------------------------------------------------------------------

#[test]
fn sturdy_leaves_one_hp_from_full_hp_only() {
    let mut arena = setup(
        set("Snorlax", "Tackle", None, None, 100),
        set("Magnemite", "Tackle", Some("Sturdy"), None, 5),
        |_, _| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert_eq!(arena.pokemon(P2).hp, 1);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::DamageReduced { target: P2, applied, by: EffectSource::Ability { ability: AbilityId::STURDY, .. }, .. } if *applied == arena.pokemon(P2).max_hp - 1
    )));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::AbilityActivated {
            source: P2,
            ability: AbilityId::STURDY
        }
    )));
    // Not at full HP any more: the next hit is fatal.
    let start = arena.turn(hit(100));
    assert!(arena.pokemon(P2).is_fainted());
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::DamageReduced { .. }))
    );
}

#[test]
fn focus_sash_is_consumed_and_sturdy_keeps_it() {
    let mut arena = setup(
        set("Snorlax", "Tackle", None, None, 100),
        set(
            "Magnemite",
            "Tackle",
            Some("Run Away"),
            Some("Focus Sash"),
            5,
        ),
        |_, _| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert_eq!(arena.pokemon(P2).hp, 1);
    assert_eq!(arena.pokemon(P2).held_item, None);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::ItemConsumed {
            holder: P2,
            item: ItemId::FOCUS_SASH,
            seen: true,
        }
    )));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::DamageReduced {
            target: P2,
            by: EffectSource::Item {
                item: ItemId::FOCUS_SASH,
                ..
            },
            ..
        }
    )));

    // Sturdy resolves first and leaves 1 HP; the sash then sees a non-fatal hit and stays.
    let mut arena = setup(
        set("Snorlax", "Tackle", None, None, 100),
        set("Magnemite", "Tackle", Some("Sturdy"), Some("Focus Sash"), 5),
        |_, _| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    assert_eq!(arena.pokemon(P2).hp, 1);
    assert_eq!(arena.pokemon(P2).held_item, Some(ItemId::FOCUS_SASH));
    assert_eq!(
        arena
            .events(start)
            .iter()
            .filter(|e| matches!(e, TraceEvent::DamageReduced { .. }))
            .count(),
        1
    );
}

#[test]
fn sturdy_saves_the_first_hit_of_a_multi_hit_only() {
    let mut arena = setup(
        set("Snorlax", "Double Kick", None, None, 100),
        set("Magnemite", "Tackle", Some("Sturdy"), None, 5),
        |_, _| {},
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    let on_magnemite: Vec<u16> = arena
        .damages(start)
        .into_iter()
        .filter(|(t, _)| *t == P2)
        .map(|(_, a)| a)
        .collect();
    assert_eq!(on_magnemite, vec![arena.pokemon(P2).max_hp - 1, 1]);
    assert!(arena.pokemon(P2).is_fainted());
}

#[test]
fn magic_guard_refuses_everything_but_move_damage() {
    let mut arena = setup(
        set("Garchomp", "Tackle", Some("Rough Skin"), None, 50),
        set("Clefable", "Double-Edge", Some("Magic Guard"), None, 50),
        |_, b| b.status = Some(StatusState::new(NonVolatileStatus::Burn)),
    );
    // Garchomp's Tackle lands on Clefable (a move: allowed). Clefable's Double-Edge then
    // triggers Rough Skin and recoil, both refused; the burn residual at the end of the turn
    // is refused too.
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.turn(tape);
    let on_clefable: Vec<(PokemonId, u16)> = arena
        .damages(start)
        .into_iter()
        .filter(|(t, _)| *t == P2)
        .collect();
    assert_eq!(
        on_clefable.len(),
        1,
        "only Tackle reached Clefable's HP: {on_clefable:?}"
    );
    let prevented: Vec<EffectSource> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::DamagePrevented {
                target: P2,
                source,
                by:
                    EffectSource::Ability {
                        ability: AbilityId::MAGIC_GUARD,
                        ..
                    },
                ..
            } => Some(*source),
            _ => None,
        })
        .collect();
    assert_eq!(prevented.len(), 3, "{prevented:?}");
    assert!(matches!(
        prevented[0],
        EffectSource::Ability {
            ability: AbilityId::ROUGH_SKIN,
            ..
        }
    ));
    assert!(
        matches!(prevented[1], EffectSource::Move { pokemon: P2, .. }),
        "recoil"
    );
    assert!(matches!(
        prevented[2],
        EffectSource::Status(NonVolatileStatus::Burn)
    ));
    assert!(
        arena.pokemon(P2).has_status(NonVolatileStatus::Burn),
        "the status itself stays"
    );
}

#[test]
fn magic_guard_lets_a_confusion_self_hit_through() {
    // Showdown's confusion self-hit is a Move (`{id: 'confused', effectType: 'Move'}`), so Magic
    // Guard does not refuse it.
    let mut arena = setup(
        set("Clefable", "Tackle", Some("Magic Guard"), None, 50),
        set("Garchomp", "Swords Dance", None, None, 50),
        |a, _| {
            a.volatiles.set_confusion(P1, 3);
        },
    );
    let tape = vec![
        RandomOverride::bool(RandomRequestKind::ConfusionSelfHit, true),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ];
    let start = arena.turn(tape);
    assert!(
        arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::ConfusionSelfHit { target: P1, .. }))
    );
    assert!(
        arena.pokemon(P1).hp < arena.pokemon(P1).max_hp,
        "the self-hit reached HP"
    );
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::DamagePrevented { .. }))
    );
}

#[test]
fn sturdy_only_survives_from_full_hp_from_gen5() {
    // Gen 4 Sturdy has no `onDamage` (Showdown gen4 mod): Magnemite is simply knocked out.
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen4,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen4));
    let a = build_pokemon(
        &dex,
        P1,
        SideId::P1,
        &set("Snorlax", "Tackle", None, None, 100),
    )
    .unwrap();
    let b = build_pokemon(
        &dex,
        P2,
        SideId::P2,
        &set("Magnemite", "Tackle", Some("Sturdy"), None, 5),
    )
    .unwrap();
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(vec![a, b], [side(P1), side(P2)], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    let mut arena = Arena { rules, dex, engine };
    // Gen 4 Tackle is 95 % accurate.
    let tape = vec![
        RandomOverride::bool(RandomRequestKind::Accuracy, true),
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ];
    let start = arena.turn(tape);
    assert!(arena.pokemon(P2).is_fainted());
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::DamageReduced { .. }))
    );
    assert!(!arena.rules.hits().sturdy_survives_from_full_hp);
}
