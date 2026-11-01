//! L1 rule tests for Gen 4's damage formula as the cartridge computes it (DESIGN.md R125;
//! issues.md I14-I20): pokeplatinum multiplies one modifier at a time and floors each, in its
//! own order, the stat modifiers on the stored stat before its stage. The values pinned here are
//! the steps a replay's damage-roll search can absorb; each comment gives the cartridge's
//! arithmetic and what Showdown's chain would have given.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EngineStep, ModifierSource, ModifierStage, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
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

fn set(species: &str, level: u8, ability: &str, item: Option<&str>, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = level;
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set
}

fn arena(a: PokemonSet, b: PokemonSet, prepare: impl FnOnce(&mut Vec<PokemonState>)) -> Arena {
    let generation = Generation::Gen4;
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
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

impl Arena {
    fn choice(&self, side: SideId) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
        let move_id = self.engine.state().pokemon(actor).unwrap().moves[0].move_id;
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
                    slot: MoveSlot::new(0).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        }
    }

    /// One turn — both use their first move — with `tape` for the random draws: what it traced.
    fn turn(&mut self, tape: Vec<RandomOverride>) -> Vec<TraceEvent> {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1), self.choice(SideId::P2)];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        self.engine.trace().events()[start..].to_vec()
    }
}

/// A's hit: no critical hit, the top damage roll, no secondary.
fn plain_hit() -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
        RandomOverride::bool(RandomRequestKind::Secondary, false),
    ]
}

/// `tape` after a landing accuracy check.
fn accurate(tape: Vec<RandomOverride>) -> Vec<RandomOverride> {
    std::iter::once(RandomOverride::bool(RandomRequestKind::Accuracy, true))
        .chain(tape)
        .collect()
}

/// The modifiers `stage` traced, as (source, from, to).
fn steps(events: &[TraceEvent], stage: ModifierStage) -> Vec<(ModifierSource, u32, u32)> {
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::ModifierApplied {
                stage: traced,
                source,
                from,
                to,
                ..
            } if *traced == stage => Some((*source, *from, *to)),
            _ => None,
        })
        .collect()
}

/// The final-damage step named `name`, as (from, to).
fn final_step(events: &[TraceEvent], name: &str) -> (u32, u32) {
    steps(events, ModifierStage::FinalDamage)
        .into_iter()
        .find_map(|(source, from, to)| match source {
            ModifierSource::Rule(rule) if rule == name => Some((from, to)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {name} step"))
}

fn damage_dealt_by(events: &[TraceEvent], attacker: PokemonId) -> u16 {
    events
        .iter()
        .find_map(|event| match event {
            TraceEvent::DamageCalculated {
                actor,
                final_damage,
                ..
            } if *actor == attacker => Some(*final_damage),
            _ => None,
        })
        .expect("a damage calculation")
}

#[test]
fn the_power_takes_technician_then_the_type_item_floored_at_each() {
    // Tackle 35: Technician 35 × 15 / 10 = 52, then the Silk Scarf 52 × 120 / 100 = 62.
    // Showdown chains them (6144 × 4915 → 7372) and rounds once: 63.
    let mut arena = arena(
        set("Mew", 100, "Technician", Some("Silk Scarf"), &["Tackle"]),
        set("Snorlax", 60, "Thick Fat", None, &["Harden"]),
        |_| {},
    );
    let events = arena.turn(accurate(plain_hit()[..2].to_vec()));
    let power: Vec<(u32, u32)> = steps(&events, ModifierStage::BasePower)
        .into_iter()
        .map(|(_, from, to)| (from, to))
        .collect();
    assert_eq!(power, [(35, 52), (52, 62)]);
}

#[test]
fn the_stat_modifiers_multiply_the_stored_stat_before_its_stage() {
    // A level-53 Mew's 127 Attack: the Choice Band's × 150 / 100 = 190, then Guts' = 285, then
    // the −1 stage's × 10 / 15 = 190. Showdown stages first (⌊127 / 1.5⌋ = 84) and chains the
    // two ×1.5 (9216): 189.
    let mut arena = arena(
        set("Mew", 53, "Guts", Some("Choice Band"), &["Megahorn"]),
        set("Snorlax", 60, "Thick Fat", None, &["Harden"]),
        |pokemon| {
            pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Poison));
            pokemon[0].stages.attack = -1;
        },
    );
    let events = arena.turn(accurate(plain_hit()[..2].to_vec()));
    let offense = events
        .iter()
        .find_map(|event| match event {
            TraceEvent::OffensiveStatResolved {
                actor,
                base,
                after_ability,
                after_item,
                ..
            } if *actor == A => Some((*base, *after_ability, *after_item)),
            _ => None,
        })
        .unwrap();
    assert_eq!(offense, (127, 285, 190));
}

#[test]
fn sniper_makes_the_critical_hit_triple() {
    // `BattleSystem_CalcCriticalMulti`: 2, or 3 with Sniper, right after the `+ 2`. Showdown
    // doubles there and adds ×1.5 in ModifyDamage, after the roll, STAB and type.
    let mut arena = arena(
        set("Mew", 50, "Sniper", None, &["Tackle"]),
        set("Snorlax", 60, "Thick Fat", None, &["Harden"]),
        |_| {},
    );
    let events = arena.turn(accurate(vec![
        RandomOverride::bool(RandomRequestKind::Critical, true),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ]));
    let (from, to) = final_step(&events, "Critical");
    assert_eq!(to, from * 3);
}

#[test]
fn a_dual_type_meets_the_chart_one_type_at_a_time_in_its_order() {
    // Mew's Flamethrower on Ludicolo: 43 after the roll. Fire meets Water (½) before Grass (×2)
    // in `sTypeMatchupMultipliers`: ⌊43 / 2⌋ × 2 = 42. Electric meets Water (×2) first: 86,
    // then 43. Showdown adds the two up (neutral): 43 both times.
    for (attack, expected) in [("Flamethrower", (43, 42)), ("Thunderbolt", (43, 43))] {
        let mut fight = arena(
            set("Mew", 50, "Synchronize", None, &[attack]),
            set("Ludicolo", 50, "Swift Swim", None, &["Harden"]),
            |_| {},
        );
        let events = fight.turn(plain_hit());
        assert_eq!(
            final_step(&events, "TypeEffectiveness"),
            expected,
            "{attack}"
        );
    }
}

#[test]
fn filter_then_the_expert_belt_then_the_resist_berry_last() {
    // Flamethrower on Scizor, ×4: 152. Filter `BattleSystem_Divide(152 × 3, 4)` = 114, the
    // Expert Belt 114 × 120 / 100 = 136 (the end of `BattleSystem_ApplyTypeChart`), then the
    // Occa Berry of the HP update, `Divide(136, 2)` = 68.
    let mut arena = arena(
        set(
            "Mew",
            50,
            "Synchronize",
            Some("Expert Belt"),
            &["Flamethrower"],
        ),
        set("Scizor", 70, "Filter", Some("Occa Berry"), &["Harden"]),
        |_| {},
    );
    let events = arena.turn(plain_hit());
    assert_eq!(final_step(&events, "TypeEffectiveness").1, 152);
    assert_eq!(final_step(&events, "ModifyDamage"), (152, 136));
    assert_eq!(final_step(&events, "ResistBerry"), (136, 68));
    assert_eq!(damage_dealt_by(&events, A), 68);
}

#[test]
fn the_confusion_self_hit_borrows_struggle() {
    // `CALC_SELF_HIT(MOVE_STRUGGLE, 40)`: Struggle's data type, Normal — the Silk Scarf raises
    // the 40 to 48 — and Technician skips Struggle. 120 Attack on 120 Defense: ⌊1056 / 50⌋ = 21,
    // + 2 = 23 at the top roll. Showdown's typeless self-hit takes Technician (60): 28.
    let mut arena = arena(
        set("Mew", 50, "Technician", Some("Silk Scarf"), &["Harden"]),
        set("Mew", 45, "Synchronize", None, &["Harden"]),
        |pokemon| {
            pokemon[0].volatiles.set_confusion(B, 3);
        },
    );
    let events = arena.turn(vec![
        RandomOverride::bool(RandomRequestKind::ConfusionSelfHit, true),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ]);
    let power: Vec<(u32, u32)> = steps(&events, ModifierStage::BasePower)
        .into_iter()
        .map(|(_, from, to)| (from, to))
        .collect();
    assert_eq!(power, [(40, 48)]);
    assert_eq!(damage_dealt_by(&events, A), 23);
}

/// The screen step of Snorlax's Strength on a Reflect-screened Mew of `mew_level` whose partner
/// stands (doubles): `(from, to)`.
fn crowded_reflect_step(mew_level: u8) -> (u32, u32) {
    use pokemon_battle_engine::state::SideConditionKind;
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen4,
            BattleMode::Doubles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen4));
    let (mew, partner, lax, chansey) = (A, B, PokemonId(3), PokemonId(4));
    let pokemon = vec![
        build_pokemon(
            &dex,
            mew,
            SideId::P1,
            &set("Mew", mew_level, "Synchronize", None, &["Harden"]),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            partner,
            SideId::P1,
            &set("Magikarp", 50, "Swift Swim", None, &["Harden"]),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            lax,
            SideId::P2,
            &set("Snorlax", 100, "Thick Fat", None, &["Strength"]),
        )
        .unwrap(),
        build_pokemon(
            &dex,
            chansey,
            SideId::P2,
            &set("Chansey", 50, "Natural Cure", None, &["Harden"]),
        )
        .unwrap(),
    ];
    let mut p1 = SideState::new(vec![mew, partner], 2);
    p1.active = vec![Some(mew), Some(partner)];
    p1.conditions.set_timed(SideConditionKind::Reflect, 5);
    let mut p2 = SideState::new(vec![lax, chansey], 2);
    p2.active = vec![Some(lax), Some(chansey)];
    let mut engine = BattleEngine::try_new(
        rules.clone(),
        dex.clone(),
        BattleState::new(pokemon, [p1, p2], 1),
    )
    .unwrap();
    let mut snapshot = engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, 100),
        ],
    );
    engine = BattleEngine::restore(rules.clone(), dex.clone(), snapshot).unwrap();
    let harden = |actor| SlotChoice {
        actor,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        },
    };
    let strength = SlotChoice {
        actor: lax,
        action: ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: Some(DeclaredTarget(
                rules.topology().position(SideId::P1, 0).unwrap(),
            )),
            mechanic: None,
        },
    };
    let start = engine.trace().events().len();
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: vec![harden(mew), harden(partner)],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![strength, harden(chansey)],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
    final_step(&engine.trace().events()[start..], "ModifyDamagePhase1")
}

#[test]
fn a_crowded_reflect_takes_two_thirds_floored() {
    // `damage * 2 / 3` when both of the target's side stand (pokeplatinum
    // `BattleSystem_CalcMoveDamage`); Showdown's `chainModify(2, 3)` (2730) rounds instead, one
    // more whenever the damage is one more than a multiple of 3.
    let steps: Vec<(u32, u32)> = (1..=6).map(crowded_reflect_step).collect();
    for (from, to) in &steps {
        assert_eq!(*to, from * 2 / 3, "{from}");
    }
    assert!(
        steps.iter().any(|(from, _)| from % 3 == 1),
        "no damage that tells the floor from the rounding: {steps:?}"
    );
}
