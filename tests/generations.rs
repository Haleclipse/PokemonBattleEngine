//! L1 rule tests for M6 batch 1 (ROADMAP §10, DESIGN.md §31.1): the generation facts the Gen 8 →
//! Gen 3 corpora exposed — Showdown's residual order (order, then speed, then sub-order) and a
//! residual that stops once the battle is decided, Gen 1-3 faint replacement right after the
//! move, rolled Taunt / Encore / Disable durations, Toxic's sure hit for a Poison type (Gen 6+),
//! Gen 4's Life Orb, Gen 3 type-boosting items, the Gen 3-4 `substitutebroken` marker, and
//! Intimidate against a Substitute.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, EngineStep, ModifierSource, ModifierStage, SideChoice, SlotChoice,
    TraceEvent,
};
use pokemon_battle_engine::ids::{ItemId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    DeclaredTarget, ResidualKind, ResolvedRules, RulesResolver, TargetPattern, TimedDuration,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState, VolatileConditionKind,
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

fn set(species: &str, moves: &[&str], ability: &str, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

/// Singles: P1 = [A, C], P2 = [B, D]. Leads' switch-in runs first.
fn singles(
    generation: Generation,
    [a, b, c, d]: [PokemonSet; 4],
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

    fn has(&self, from: usize, pred: impl Fn(&TraceEvent) -> bool) -> bool {
        self.events(from).iter().any(pred)
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn residuals(&self, from: usize) -> Vec<(PokemonId, ResidualKind)> {
        self.events(from)
            .iter()
            .filter_map(|e| match e {
                TraceEvent::ResidualEffect { holder, kind, .. } => Some((*holder, *kind)),
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

fn filler() -> PokemonSet {
    set("Snorlax", &["Tackle"], "Run Away", None)
}

fn poisoned(ids: &[usize]) -> impl FnOnce(&mut Vec<PokemonState>) + '_ {
    move |pokemon| {
        for &i in ids {
            pokemon[i].status = Some(StatusState::new(NonVolatileStatus::Poison));
        }
    }
}

#[test]
fn residuals_run_by_order_then_speed_then_sub_order() {
    // Both leads hold Leftovers and are poisoned. Gen 4 puts both at order 10 (Leftovers 10.4,
    // poison 10.6): the faster Garchomp finishes both before Snorlax starts. Gen 9 orders them
    // apart (Leftovers 5.4, poison 9): both Leftovers first.
    let roster = || {
        [
            set("Garchomp", &["Swords Dance"], "Run Away", Some("Leftovers")),
            set("Snorlax", &["Swords Dance"], "Run Away", Some("Leftovers")),
            filler(),
            filler(),
        ]
    };
    let mut gen4 = singles(Generation::Gen4, roster(), poisoned(&[0, 1]));
    let (start, _) = gen4.turn(0, 0, Vec::new());
    assert_eq!(
        gen4.residuals(start),
        vec![
            (A, ResidualKind::Leftovers),
            (A, ResidualKind::Poison),
            (B, ResidualKind::Leftovers),
            (B, ResidualKind::Poison),
        ]
    );
    let mut gen9 = singles(Generation::Gen9, roster(), poisoned(&[0, 1]));
    let (start, _) = gen9.turn(0, 0, Vec::new());
    assert_eq!(
        gen9.residuals(start),
        vec![
            (A, ResidualKind::Leftovers),
            (B, ResidualKind::Leftovers),
            (A, ResidualKind::Poison),
            (B, ResidualKind::Poison),
        ]
    );
}

#[test]
fn the_residual_stops_once_the_battle_is_decided() {
    // Gen 4, both at residual order 10: P1's last Pokémon (faster, poisoned, 1 HP) faints to
    // poison before the slower P2 lead's Leftovers (sub-order 4 < poison's 6, but speed comes
    // first): the battle ends and the Leftovers never run.
    let mut arena = singles(
        Generation::Gen4,
        [
            set("Garchomp", &["Swords Dance"], "Run Away", None),
            set("Snorlax", &["Swords Dance"], "Run Away", Some("Leftovers")),
            filler(),
            filler(),
        ],
        |pokemon| {
            pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Poison));
            pokemon[0].hp = 1;
            pokemon[2].hp = 0;
            pokemon[1].hp -= 50;
        },
    );
    let (start, step) = arena.turn(0, 0, Vec::new());
    assert!(matches!(step, EngineStep::Finished(_)));
    assert_eq!(arena.residuals(start), vec![(A, ResidualKind::Poison)]);
}

#[test]
fn gen_3_replaces_a_fainted_pokemon_right_after_the_move() {
    // Snorlax knocks Pikachu (1 HP) out with Pound. Gen 3 asks P1 for a replacement right
    // away (Snorlax's Leftovers has not run yet); Gen 4 only after the residual.
    let roster = || {
        [
            set("Pikachu", &["Swords Dance"], "Run Away", None),
            set("Snorlax", &["Pound"], "Run Away", Some("Leftovers")),
            filler(),
            filler(),
        ]
    };
    let hurt = |pokemon: &mut Vec<PokemonState>| {
        pokemon[0].hp = 1;
        pokemon[1].hp -= 30;
    };
    for (generation, residual_first) in [(Generation::Gen3, false), (Generation::Gen4, true)] {
        let mut arena = singles(generation, roster(), hurt);
        let (start, step) = arena.turn(0, 0, hit(100));
        assert!(
            matches!(
                step,
                EngineStep::Awaiting(BattleRequest::ForcedSwitch { .. })
            ),
            "{generation:?}"
        );
        assert_eq!(
            arena
                .residuals(start)
                .contains(&(B, ResidualKind::Leftovers)),
            residual_first,
            "{generation:?}"
        );
    }
}

#[test]
fn taunt_encore_and_disable_durations_follow_the_generation() {
    let conditions = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .conditions()
    };
    assert_eq!(
        conditions(Generation::Gen9).taunt.duration,
        TimedDuration::Fixed(3)
    );
    assert_eq!(
        conditions(Generation::Gen4).taunt.duration,
        TimedDuration::Rolled { min: 3, max: 5 }
    );
    assert_eq!(
        conditions(Generation::Gen3).taunt.duration,
        TimedDuration::Fixed(2)
    );
    assert_eq!(
        conditions(Generation::Gen4).disable.duration,
        TimedDuration::Rolled { min: 4, max: 7 }
    );
    assert_eq!(
        conditions(Generation::Gen3).encore.duration,
        TimedDuration::Rolled { min: 3, max: 6 }
    );

    // Gen 4 Taunt: a Duration decision (5 here), no adjustment; the residual ticks it to 4.
    let mut arena = singles(
        Generation::Gen4,
        [
            set("Garchomp", &["Taunt"], "Run Away", None),
            set("Snorlax", &["Swords Dance"], "Run Away", None),
            filler(),
            filler(),
        ],
        |_| {},
    );
    arena.turn(
        0,
        0,
        vec![RandomOverride::range(RandomRequestKind::Duration, 5)],
    );
    assert_eq!(
        arena
            .pokemon(B)
            .volatiles
            .turns_left(VolatileConditionKind::Taunt),
        Some(4)
    );
}

#[test]
fn a_poison_types_toxic_never_misses_from_gen_6() {
    let roster = || {
        [
            set("Gengar", &["Toxic"], "Run Away", None),
            set("Snorlax", &["Swords Dance"], "Run Away", None),
            filler(),
            filler(),
        ]
    };
    // Gen 9: no Accuracy decision.
    let mut arena = singles(Generation::Gen9, roster(), |_| {});
    arena.turn(0, 0, Vec::new());
    assert_eq!(
        arena.pokemon(B).status.map(|s| s.kind),
        Some(NonVolatileStatus::Toxic)
    );
    // Gen 5: Toxic (90 %) rolls.
    let mut arena = singles(Generation::Gen5, roster(), |_| {});
    arena.turn(
        0,
        0,
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, false)],
    );
    assert_eq!(arena.pokemon(B).status, None);
}

#[test]
fn gen_4_life_orb_multiplies_before_the_random_roll_and_needs_a_body_hit() {
    let mut arena = singles(
        Generation::Gen4,
        [
            set("Garchomp", &["Dragon Claw"], "Run Away", Some("Life Orb")),
            set("Snorlax", &["Swords Dance"], "Run Away", None),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let max_hp = arena.pokemon(A).max_hp;
    let (start, _) = arena.turn(0, 0, hit(100));
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::ModifierApplied {
            stage: ModifierStage::FinalDamage,
            source: ModifierSource::ItemInstance {
                item: ItemId::LIFE_ORB,
                ..
            },
            from,
            to,
            ..
        } if *to == *from * 13 / 10
    )));
    assert_eq!(arena.pokemon(A).hp, max_hp - max_hp / 10);
}

#[test]
fn gen_3_type_items_raise_the_offensive_stat() {
    let mut arena = singles(
        Generation::Gen3,
        [
            set("Snorlax", &["Body Slam"], "Run Away", Some("Silk Scarf")),
            set("Blissey", &["Softboiled"], "Run Away", None),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let attack = u32::from(arena.pokemon(A).stats.attack);
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    let (start, _) = arena.turn(0, 0, tape);
    assert!(arena.has(start, |e| matches!(
        e,
        TraceEvent::OffensiveStatResolved { after_item, .. }
            if *after_item == (attack * 4505 + 2047) / 4096
    )));
    assert!(!arena.has(start, |e| matches!(
        e,
        TraceEvent::ModifierApplied {
            stage: ModifierStage::BasePower,
            ..
        }
    )));
}

#[test]
fn a_broken_substitute_leaves_a_gen_3_4_marker_and_intimidate_skips_a_substitute() {
    let roster = || {
        [
            set("Garchomp", &["Dragon Claw"], "Run Away", None),
            set("Gengar", &["Substitute"], "Run Away", None),
            filler(),
            filler(),
        ]
    };
    for (generation, marker) in [(Generation::Gen4, true), (Generation::Gen9, false)] {
        let mut arena = singles(generation, roster(), |_| {});
        // Gengar (faster) sets a Substitute; Garchomp breaks it.
        arena.turn(0, 0, hit(100));
        assert_eq!(
            arena
                .pokemon(B)
                .volatiles
                .contains(VolatileConditionKind::SubstituteBroken),
            marker,
            "{generation:?}"
        );
    }
    // Intimidate: a Substitute shields the foe (every generation); without one, −1 Attack.
    let intimidated = |substitute: bool| {
        let mut arena = singles(
            Generation::Gen9,
            [
                set("Gengar", &["Substitute"], "Run Away", None),
                set("Snorlax", &["Swords Dance"], "Run Away", None),
                filler(),
                set("Gyarados", &["Swords Dance"], "Intimidate", None),
            ],
            |_| {},
        );
        if substitute {
            arena.turn(0, 0, Vec::new());
        }
        let mut snapshot = arena.engine.snapshot();
        snapshot.state.side_mut(SideId::P2).active[0] = Some(D);
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
        arena.engine.run_switch_in(D);
        arena.pokemon(A).stages.attack
    };
    assert_eq!(intimidated(true), 0);
    assert_eq!(intimidated(false), -1);
}

#[test]
fn gen_4_intimidate_skips_a_substitute_u_turn_just_broke_and_stays_silent() {
    // P1's Gengar carries the `substitutebroken` marker left by U-turn; Gyarados enters with
    // Intimidate. Gen 4: out of reach, and no announcement. Gen 3 has the marker but not the
    // rule: −1 Attack, announced.
    for (generation, drop, announced) in
        [(Generation::Gen4, 0, false), (Generation::Gen3, -1, true)]
    {
        let mut arena = singles(
            generation,
            [
                set("Gengar", &["Substitute"], "Levitate", None),
                set("Snorlax", &["Swords Dance"], "Immunity", None),
                filler(),
                set("Gyarados", &["Swords Dance"], "Intimidate", None),
            ],
            |_| {},
        );
        // After the leads' entry (Snorlax's own entry would clear a Gen 4 marker).
        let mut snapshot = arena.engine.snapshot();
        snapshot
            .state
            .pokemon_mut(A)
            .unwrap()
            .volatiles
            .mark_substitute_broken(MoveId::U_TURN);
        snapshot.state.side_mut(SideId::P2).active[0] = Some(D);
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
        let start = arena.engine.trace().events().len();
        arena.engine.run_switch_in(D);
        assert_eq!(arena.pokemon(A).stages.attack, drop, "{generation:?}");
        assert_eq!(
            arena.has(start, |e| matches!(e, TraceEvent::AbilityActivated { .. })),
            announced,
            "{generation:?}"
        );
        // Gen 4: Gyarados's entry cleared the marker, after its Intimidate looked at it.
        assert_eq!(
            arena.pokemon(A).volatiles.substitute_broken_by().is_some(),
            generation == Generation::Gen3,
            "{generation:?}"
        );
    }
}

#[test]
fn gen_3_modify_damage_comes_before_the_random_factor() {
    // Showdown gen3 `modifyDamage`: … type → `ModifyDamage` → `randomizer`; Gen 4 rolls before
    // STAB and type and runs `ModifyDamage` last.
    use pokemon_battle_engine::rules::DamageModifierKind::{ModifyDamage, Random};
    let order = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .damage()
            .modifier_order
    };
    let at = |order: &[pokemon_battle_engine::rules::DamageModifierKind], kind| {
        order.iter().position(|k| *k == kind).unwrap()
    };
    let gen3 = order(Generation::Gen3);
    assert!(at(gen3, ModifyDamage) < at(gen3, Random));
    let gen4 = order(Generation::Gen4);
    assert!(at(gen4, Random) < at(gen4, ModifyDamage));
}
