//! L1 rule tests for Showdown's `Update` (`eachEvent('Update')`, DESIGN.md §40): where the
//! generation's `UpdateRules` place it, and what eating a berry at one records (§39.1). A
//! replay sees an `Update` only through what it changes at a frame boundary; the order inside
//! an action, and the item history no frame shows, are pinned here.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{ItemId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, Gender, NonVolatileStatus, PokemonState, RandomOverride,
    RandomRequestKind, SideConditionKind, SideState, Stat, StatusState, VolatileConditionKind,
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

fn set(species: &str, moves: &[&str], ability: &str, gender: Option<Gender>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.gender = gender;
    set.level = 50;
    set
}

fn holding(mut set: PokemonSet, item: &str) -> PokemonSet {
    set.item = Some(item.into());
    set
}

/// P1 = [A, C], P2 = [B, D]; the first of each side leads.
fn setup(generation: Generation, p1: [PokemonSet; 2], p2: [PokemonSet; 2]) -> Arena {
    setup_with(generation, p1, p2, |_| {})
}

fn setup_with(
    generation: Generation,
    p1: [PokemonSet; 2],
    p2: [PokemonSet; 2],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1[0]).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2[0]).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &p1[1]).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &p2[1]).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side([A, C]), side([B, D])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn switch(&self, side: SideId, pokemon: PokemonId) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
        SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Switch { pokemon },
            }],
        }
    }

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

    /// One turn (speeds differ, so no SpeedTie is drawn). Returns the trace index it started at.
    fn turn(&mut self, p1: usize, p2: usize, tape: Vec<RandomOverride>) -> usize {
        let choices = [self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)];
        self.turn_with(choices, tape)
    }

    fn turn_with(&mut self, choices: [SideChoice; 2], tape: Vec<RandomOverride>) -> usize {
        let start = self.begin_turn(choices, tape);
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
        start
    }

    /// Submits the turn's choices without running any action.
    fn begin_turn(&mut self, choices: [SideChoice; 2], tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        self.engine.submit_turn_choices(choices).unwrap();
        start
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn position(&self, from: usize, found: impl Fn(&TraceEvent) -> bool) -> usize {
        self.engine.trace().events()[from..]
            .iter()
            .position(found)
            .expect("event traced")
    }
}

/// Showdown `switchIn` for a drag: Gen 1-4 queue the entrant's `runSwitch` as an action of
/// its own, so the dragging action's `Update` comes first — an infatuation with the Pokémon
/// blown away ends before the entrant's Spikes; from Gen 5 the entrant's switch-in runs at
/// once and the action's `Update` follows it.
#[test]
fn a_drag_runs_the_update_before_the_switch_in_until_gen_5() {
    for (generation, deferred) in [
        (Generation::Gen3, true),
        (Generation::Gen4, true),
        (Generation::Gen5, false),
        (Generation::Gen9, false),
    ] {
        let mut arena = setup(
            generation,
            [
                set(
                    "Skarmory",
                    &["Spikes", "Roar"],
                    "Keen Eye",
                    Some(Gender::Male),
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set(
                    "Clefable",
                    &["Attract", "Harden"],
                    "Cute Charm",
                    Some(Gender::Female),
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
        );
        // Skarmory lays Spikes, then Clefable infatuates it.
        arena.turn(0, 0, Vec::new());
        assert!(
            arena
                .engine
                .state()
                .pokemon(A)
                .unwrap()
                .volatiles
                .contains(VolatileConditionKind::Attract),
            "{generation:?}"
        );
        // Clefable hardens first (Roar is -6); Skarmory's infatuation lets it roar.
        let start = arena.turn(
            1,
            1,
            vec![RandomOverride::bool(RandomRequestKind::Infatuation, false)],
        );
        let ended = arena.position(start, |event| {
            matches!(
                event,
                TraceEvent::VolatileEnded {
                    target: A,
                    kind: VolatileConditionKind::Attract,
                    ..
                }
            )
        });
        let spikes = arena.position(start, |event| {
            matches!(
                event,
                TraceEvent::EntryHazardTriggered {
                    target: D,
                    kind: SideConditionKind::Spikes,
                    ..
                }
            )
        });
        assert_eq!(ended < spikes, deferred, "{generation:?}");
    }
}

fn hit() -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, 100),
    ]
}

/// Showdown `eatItem`: the berry is announced, its `onEat` cures, and then it is the holder's
/// last item, used this turn (until the turn boundary or a switch-out) and eaten (for good).
#[test]
fn eating_a_berry_cures_and_is_remembered() {
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Jolteon", &["Thunder Wave"], "Volt Absorb", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            holding(
                set("Snorlax", &["Harden"], "Thick Fat", None),
                "Cheri Berry",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
    );
    let choices = [arena.choice(SideId::P1, 0), arena.choice(SideId::P2, 0)];
    let start = arena.begin_turn(
        choices,
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)],
    );
    // Jolteon's Thunder Wave: the action's Update feeds Snorlax its Cheri Berry.
    arena.engine.step().unwrap();
    let eaten = arena.position(
        start,
        |e| matches!(e, TraceEvent::ItemEaten { holder: B, item } if *item == ItemId::CHERI_BERRY),
    );
    let cured = arena.position(start, |e| {
        matches!(e, TraceEvent::StatusCured { target: B, .. })
    });
    assert!(eaten < cured);
    let snorlax = arena.pokemon(B);
    assert_eq!(snorlax.held_item, None);
    assert_eq!(snorlax.status, None);
    assert_eq!(snorlax.history.last_item, Some(ItemId::CHERI_BERRY));
    assert!(snorlax.history.used_item_this_turn);
    assert!(snorlax.history.ate_berry);
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    // The turn boundary forgets the use, not the item nor the berry.
    let snorlax = arena.pokemon(B);
    assert!(!snorlax.history.used_item_this_turn);
    assert_eq!(snorlax.history.last_item, Some(ItemId::CHERI_BERRY));
    assert!(snorlax.history.ate_berry);
}

/// Showdown `switchIn`: from Gen 5 the last item stays with the Pokémon that used it; Gen 1-4
/// hand it to the one that takes its position. The berry eaten stays with its eater.
#[test]
fn the_last_item_passes_to_the_entrant_until_gen_5() {
    for (generation, passes) in [(Generation::Gen4, true), (Generation::Gen5, false)] {
        let mut arena = setup(
            generation,
            [
                set("Jolteon", &["Thunder Wave"], "Volt Absorb", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                holding(
                    set("Snorlax", &["Harden"], "Thick Fat", None),
                    "Cheri Berry",
                ),
                set("Lapras", &["Harden"], "Shell Armor", None),
            ],
        );
        // Thunder Wave never misses before Gen 7.
        arena.turn(0, 0, Vec::new());
        assert_eq!(
            arena.pokemon(B).history.last_item,
            Some(ItemId::CHERI_BERRY),
            "{generation:?}"
        );
        let choices = [arena.choice(SideId::P1, 0), arena.switch(SideId::P2, D)];
        // Thunder Wave paralyses Lapras, which holds nothing.
        arena.turn_with(choices, Vec::new());
        let (left, entered) = (arena.pokemon(B), arena.pokemon(D));
        let expected = Some(ItemId::CHERI_BERRY);
        assert_eq!(
            entered.history.last_item,
            expected.filter(|_| passes),
            "{generation:?}"
        );
        assert_eq!(
            left.history.last_item,
            expected.filter(|_| !passes),
            "{generation:?}"
        );
        assert!(left.history.ate_berry && !entered.history.ate_berry);
    }
}

/// Showdown `useItem` (a gem here) records the item too, but eats no berry.
#[test]
fn a_used_up_item_is_the_last_item() {
    let mut arena = setup(
        Generation::Gen9,
        [
            holding(
                set("Jolteon", &["Tackle"], "Volt Absorb", None),
                "Normal Gem",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Lapras", &["Harden"], "Shell Armor", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
    );
    let choices = [arena.choice(SideId::P1, 0), arena.choice(SideId::P2, 0)];
    arena.begin_turn(choices, hit());
    arena.engine.step().unwrap();
    let user = arena.pokemon(A);
    assert_eq!(user.history.last_item, Some(ItemId::NORMAL_GEM));
    assert!(user.history.used_item_this_turn);
    assert!(!user.history.ate_berry);
}

/// Gen 1-4's `start` action ends with an `Update` before the leads' switch-in effects: a lead
/// with a status eats its berry before the foe's Intimidate; from Gen 5 only the leads'
/// `runSwitch` action ends with one, after it.
#[test]
fn leads_eat_before_their_switch_ins_until_gen_5() {
    for (generation, first) in [(Generation::Gen4, true), (Generation::Gen5, false)] {
        let mut arena = setup_with(
            generation,
            [
                holding(
                    set("Snorlax", &["Harden"], "Thick Fat", None),
                    "Cheri Berry",
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Gyarados", &["Harden"], "Intimidate", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| {
                pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Paralysis));
            },
        );
        arena.engine.run_lead_switch_ins();
        let eaten = arena.position(0, |e| matches!(e, TraceEvent::ItemEaten { holder: A, .. }));
        let intimidated = arena.position(0, |e| {
            matches!(e, TraceEvent::StatChanged { target: A, .. })
        });
        assert_eq!(eaten < intimidated, first, "{generation:?}");
    }
}

/// Gen 4+ run an `Update` after each hit, before the move's recoil: the attacker Static
/// paralysed eats its Cheri Berry before Double-Edge's recoil. Gen 3's comes once the target's
/// hit is over, recoil included.
#[test]
fn the_hit_update_comes_before_the_recoil_from_gen_4() {
    for (generation, before_recoil) in [(Generation::Gen3, false), (Generation::Gen4, true)] {
        let mut arena = setup(
            generation,
            [
                holding(
                    set("Tauros", &["Double-Edge"], "Intimidate", None),
                    "Cheri Berry",
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Electabuzz", &["Harden"], "Static", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
        );
        arena.engine.run_lead_switch_ins();
        let tape = [
            hit(),
            vec![RandomOverride::bool(
                RandomRequestKind::ContactReaction,
                true,
            )],
        ]
        .concat();
        let start = arena.turn(0, 0, tape);
        let eaten = arena.position(start, |e| {
            matches!(e, TraceEvent::ItemEaten { holder: A, .. })
        });
        let recoil = arena.position(start, |e| {
            matches!(e, TraceEvent::DamageApplied { target: A, .. })
        });
        assert_eq!(eaten < recoil, before_recoil, "{generation:?}");
    }
}

/// The HP a berry waits for (Showdown `hp <= maxhp / 2`, `hp <= maxhp / 4`, Gluttony's half;
/// Gen 2's cartridge: below half, I32), the confusion berries' and Sitrus Berry's numbers, and
/// where the status berries are eaten, by generation.
#[test]
fn berry_rules_by_generation() {
    use pokemon_battle_engine::rules::{BerryHeal, Fraction, HpShare, StatusBerryTiming};
    let rules = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .conditions()
            .berries
    };
    let gen2 = rules(Generation::Gen2);
    assert!(!gen2.reached(HpShare::Half, 50, 100, false));
    assert!(gen2.reached(HpShare::Half, 49, 100, false));
    assert!(gen2.reached(HpShare::Half, 50, 101, false));
    let gen9 = rules(Generation::Gen9);
    assert!(gen9.reached(HpShare::Half, 50, 100, false));
    assert!(!gen9.reached(HpShare::Half, 51, 100, false));
    assert!(gen9.reached(HpShare::Pinch, 25, 100, false));
    assert!(!gen9.reached(HpShare::Pinch, 26, 100, false));
    assert!(gen9.reached(HpShare::Pinch, 50, 100, true));
    assert!(!gen9.reached(HpShare::Pinch, 51, 100, true));
    for (generation, at, heal, sitrus, timing) in [
        (
            Generation::Gen2,
            HpShare::Half,
            Fraction::new(1, 8),
            BerryHeal::Fixed(30),
            StatusBerryTiming::InflictionOrTurnEnd,
        ),
        (
            Generation::Gen3,
            HpShare::Half,
            Fraction::new(1, 8),
            BerryHeal::Fixed(30),
            StatusBerryTiming::UpdateAndBeforeReplacement,
        ),
        (
            Generation::Gen4,
            HpShare::Half,
            Fraction::new(1, 8),
            BerryHeal::MaxHp(Fraction::new(1, 4)),
            StatusBerryTiming::Update,
        ),
        (
            Generation::Gen7,
            HpShare::Pinch,
            Fraction::new(1, 2),
            BerryHeal::MaxHp(Fraction::new(1, 4)),
            StatusBerryTiming::Update,
        ),
        (
            Generation::Gen9,
            HpShare::Pinch,
            Fraction::new(1, 3),
            BerryHeal::MaxHp(Fraction::new(1, 4)),
            StatusBerryTiming::Update,
        ),
    ] {
        let berries = rules(generation);
        assert_eq!(
            (
                berries.confusion_berry_at,
                berries.confusion_berry_heal,
                berries.sitrus_heal,
                berries.status_berries
            ),
            (at, heal, sitrus, timing),
            "{generation:?}"
        );
    }
}

/// Berry Juice is used up (`useItem`), not eaten: the last item, used this turn, no berry eaten.
#[test]
fn berry_juice_is_used_not_eaten() {
    let mut arena = setup_with(
        Generation::Gen5,
        [
            set("Chansey", &["Seismic Toss"], "Serene Grace", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            holding(
                set("Snorlax", &["Harden"], "Thick Fat", None),
                "Berry Juice",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        |pokemon| {
            pokemon[0].level = 100;
            pokemon[1].hp = pokemon[1].max_hp / 2 + 40;
        },
    );
    let choices = [arena.choice(SideId::P1, 0), arena.choice(SideId::P2, 0)];
    let start = arena.begin_turn(choices, Vec::new());
    arena.engine.step().unwrap();
    assert!(
        arena.engine.trace().events()[start..]
            .iter()
            .any(|e| matches!(
                e,
                TraceEvent::ItemConsumed { holder: B, item, seen: true } if *item == ItemId::BERRY_JUICE
            ))
    );
    let juice = arena.pokemon(B);
    assert_eq!(juice.held_item, None);
    assert_eq!(juice.history.last_item, Some(ItemId::BERRY_JUICE));
    assert!(juice.history.used_item_this_turn);
    assert!(!juice.history.ate_berry);
}

/// Gen 2 (I33): no `Update` feeds a status berry; one the holder already carries a status for
/// (no move of another Pokémon gave it: here the battle starts with it) waits for the end of the
/// turn (pokecrystal `HandleHealingItems`).
#[test]
fn a_gen2_status_berry_waits_for_the_end_of_the_turn() {
    let plain = |species: &str| {
        let mut set = set(species, &["Harden"], "", None);
        set.ability = None;
        set
    };
    let mut arena = setup_with(
        Generation::Gen2,
        [
            holding(plain("Snorlax"), "PRZ Cure Berry"),
            plain("Snorlax"),
        ],
        [plain("Lapras"), plain("Snorlax")],
        |pokemon| {
            pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Paralysis));
        },
    );
    arena.engine.run_lead_switch_ins();
    assert!(arena.pokemon(A).held_item.is_some());
    let start = arena.turn(
        0,
        0,
        vec![RandomOverride::bool(
            RandomRequestKind::FullParalysis,
            false,
        )],
    );
    let eaten = arena.position(start, |e| {
        matches!(e, TraceEvent::ItemEaten { holder: A, .. })
    });
    let residual = arena.position(start, |e| {
        matches!(
            e,
            TraceEvent::PhaseChanged {
                to: pokemon_battle_engine::state::BattlePhase::EndTurn,
                ..
            }
        )
    });
    assert!(residual < eaten);
    assert_eq!(arena.pokemon(A).status, None);
}

/// Unnerve and As One start ahead of the other switch-in handlers (Showdown
/// `onSwitchInPriority: 1`, Gen 5+ `SwitchInOrder::ByPriority`): the slower lead announces
/// itself before the faster lead's Intimidate. A replay sees no frame between the two.
#[test]
fn unnerve_starts_before_a_faster_leads_intimidate() {
    for (generation, species, ability) in [
        (Generation::Gen5, "Slowbro", "Unnerve"),
        (Generation::Gen9, "Slowbro", "Unnerve"),
        (Generation::Gen8, "Slowbro", "As One (Spectrier)"),
    ] {
        let mut arena = setup(
            generation,
            [
                set("Jolteon", &["Harden"], "Intimidate", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set(species, &["Harden"], ability, None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
        );
        arena.engine.run_lead_switch_ins();
        let unnerve = arena.position(0, |e| {
            matches!(e, TraceEvent::AbilityActivated { source: B, .. })
        });
        let intimidate = arena.position(0, |e| {
            matches!(e, TraceEvent::AbilityActivated { source: A, .. })
        });
        assert!(unnerve < intimidate, "{generation:?} {ability}");
    }
}

/// Starf Berry draws among the five stats below +6 (Showdown `this.sample` over them): with
/// four of them at +6 Speed rises by two whatever the PRNG gives; with all five at +6 the berry
/// is eaten for nothing. The replay names the drawn stat, so only here does the candidate list
/// show.
#[test]
fn starf_berry_draws_only_among_the_stats_below_six() {
    for (seed, all_maxed) in (0..16).map(|seed| (seed, false)).chain([(0, true)]) {
        let mut arena = setup_with(
            Generation::Gen9,
            [
                holding(
                    set("Jolteon", &["Harden"], "Volt Absorb", None),
                    "Starf Berry",
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Snorlax", &["Harden"], "Thick Fat", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| {
                let holder = &mut pokemon[0];
                holder.hp = holder.max_hp / 4;
                holder.stages.attack = 6;
                holder.stages.defense = 6;
                holder.stages.special_attack = 6;
                holder.stages.special_defense = 6;
                if all_maxed {
                    holder.stages.speed = 6;
                }
            },
        );
        let mut snapshot = arena.engine.snapshot();
        snapshot.state.rng = BattleRng::new(seed);
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
        arena.engine.run_lead_switch_ins();
        let holder = arena.pokemon(A);
        assert_eq!(holder.held_item, None, "seed {seed}");
        assert_eq!(
            holder.stages.speed,
            if all_maxed { 6 } else { 2 },
            "seed {seed}"
        );
        assert_eq!(holder.stages.attack, 6, "seed {seed}");
    }
}

/// Gen 3-4 feed a stat berry only while its effect has room (issues.md I37: pokeemerald
/// `ItemBattleEffects`, pokeplatinum `BattleSystem_TriggerHeldItem`): Liechi Berry at Attack +6,
/// Lansat Berry under Focus Energy, Starf Berry with all five stats at +6 stay held; from Gen 5
/// (Showdown) they are eaten for nothing.
#[test]
fn stat_berries_wait_for_room_in_gen_3_and_4() {
    for (generation, kept) in [(Generation::Gen4, true), (Generation::Gen5, false)] {
        for berry in ["Liechi Berry", "Lansat Berry", "Starf Berry"] {
            let mut arena = setup_with(
                generation,
                [
                    holding(set("Jolteon", &["Harden"], "Volt Absorb", None), berry),
                    set("Snorlax", &["Harden"], "Thick Fat", None),
                ],
                [
                    set("Snorlax", &["Harden"], "Thick Fat", None),
                    set("Snorlax", &["Harden"], "Thick Fat", None),
                ],
                |pokemon| {
                    let holder = &mut pokemon[0];
                    holder.hp = holder.max_hp / 4;
                    holder.stages.attack = 6;
                    holder.stages.defense = 6;
                    holder.stages.special_attack = 6;
                    holder.stages.special_defense = 6;
                    holder.stages.speed = 6;
                    holder
                        .volatiles
                        .set_marker(VolatileConditionKind::FocusEnergy);
                },
            );
            arena.engine.run_lead_switch_ins();
            assert_eq!(
                arena.pokemon(A).held_item.is_some(),
                kept,
                "{generation:?} {berry}"
            );
        }
    }
}

/// Ripen announces itself where Showdown's `onTryEatItem` (priority −1) and `onTryHeal` run:
/// before eating a berry nothing refused — not under a foe's Unnerve, which refuses it first —
/// and for Leftovers, at full HP too. No frame shows an announcement.
#[test]
fn ripen_announces_where_its_handlers_run() {
    let ripen = |item: &str| holding(set("Appletun", &["Harden"], "Ripen", None), item);
    let announced = |arena: &Arena, from: usize| {
        arena.engine.trace().events()[from..]
            .iter()
            .any(|e| matches!(e, TraceEvent::AbilityActivated { source: A, .. }))
    };
    // A Sitrus Berry at a quarter: announced and eaten; under Unnerve neither.
    for (foe, eaten) in [("Thick Fat", true), ("Unnerve", false)] {
        let mut arena = setup_with(
            Generation::Gen8,
            [
                ripen("Sitrus Berry"),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Slowbro", &["Harden"], foe, None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| pokemon[0].hp = pokemon[0].max_hp / 4,
        );
        arena.engine.run_lead_switch_ins();
        let ate = arena
            .engine
            .trace()
            .events()
            .iter()
            .any(|e| matches!(e, TraceEvent::ItemEaten { holder: A, .. }));
        assert_eq!(ate, eaten, "{foe}");
        assert_eq!(announced(&arena, 0), eaten, "{foe}");
    }
    // Leftovers at full HP: announced in the residual, nothing healed.
    let mut arena = setup(
        Generation::Gen8,
        [
            ripen("Leftovers"),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Snorlax", &["Harden"], "Thick Fat", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
    );
    arena.engine.run_lead_switch_ins();
    let start = arena.turn(0, 0, vec![]);
    assert!(announced(&arena, start));
    assert!(
        !arena.engine.trace().events()[start..]
            .iter()
            .any(|e| matches!(e, TraceEvent::HealApplied { target: A, .. }))
    );
}

/// The seeds' and Room Service's `onStart` run at `onSwitchInPriority` −1: on a field that has
/// Electric Terrain already, the slower lead's Intimidate (0) comes before the faster lead's
/// Electric Seed. No frame shows the order.
#[test]
fn a_seed_starts_after_the_other_switch_in_handlers() {
    let mut arena = setup(
        Generation::Gen9,
        [
            holding(
                set("Jolteon", &["Harden"], "Volt Absorb", None),
                "Electric Seed",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Gyarados", &["Harden"], "Intimidate", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
    );
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.field.terrain = Some(pokemon_battle_engine::state::TerrainState {
        kind: pokemon_battle_engine::state::TerrainKind::Electric,
        source: B,
        turns_remaining: 5,
    });
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    arena.engine.run_lead_switch_ins();
    let intimidated = arena.position(0, |e| {
        matches!(e, TraceEvent::StatChanged { target: A, .. })
    });
    let seed = arena.position(0, |e| {
        matches!(e, TraceEvent::ItemConsumed { holder: A, .. })
    });
    assert!(intimidated < seed);
}

fn tropius(item: Option<&str>) -> PokemonSet {
    let tropius = set("Tropius", &["Harden"], "Harvest", None);
    match item {
        Some(item) => holding(tropius, item),
        None => tropius,
    }
}

/// Harvest (Showdown `onResidual` 28.2): out of the sun its 1 in 2 is rolled at every turn's
/// end, whatever the holder holds; in the sun nothing is rolled. Only a berry comes back, to a
/// holder with no item, and it is then no longer the last item. The replay plans the roll as
/// optional and sees it only when a berry comes back, so the draws are pinned here.
#[test]
fn harvest_rolls_out_of_the_sun_whatever_the_holder_holds() {
    // (sun, held, last item, roll, held after)
    let cases = [
        (
            false,
            Some("Oran Berry"),
            ItemId::SITRUS_BERRY,
            true,
            Some(ItemId::ORAN_BERRY),
        ),
        (
            false,
            None,
            ItemId::SITRUS_BERRY,
            true,
            Some(ItemId::SITRUS_BERRY),
        ),
        (false, None, ItemId::SITRUS_BERRY, false, None),
        (false, None, ItemId::FOCUS_SASH, true, None),
        (
            true,
            None,
            ItemId::SITRUS_BERRY,
            false,
            Some(ItemId::SITRUS_BERRY),
        ),
    ];
    for (sun, held, last_item, roll, after) in cases {
        let mut arena = setup_with(
            Generation::Gen9,
            [
                tropius(held),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Snorlax", &["Rain Dance", "Sunny Day"], "Thick Fat", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| pokemon[0].history.last_item = Some(last_item),
        );
        let choices = [
            arena.choice(SideId::P1, 0),
            arena.choice(SideId::P2, usize::from(sun)),
        ];
        arena.begin_turn(
            choices,
            vec![RandomOverride::bool(RandomRequestKind::Harvest, roll)],
        );
        while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
        let case = format!("sun {sun}, {held:?}, {last_item:?}, roll {roll}");
        assert_eq!(
            arena.engine.state().rng.semantic_tape_exhausted(),
            !sun,
            "{case}"
        );
        let holder = arena.pokemon(A);
        assert_eq!(holder.held_item, after, "{case}");
        let restored = held.is_none() && after.is_some();
        assert_eq!(
            holder.history.last_item,
            (!restored).then_some(last_item),
            "{case}"
        );
    }
}

/// Cud Chew eats its berry again with the berry's `onEat` alone: a foe's Unnerve does not
/// refuse it, a status berry cures only the status it is for, and the holder has eaten a
/// berry. No recording shows these, so they are pinned here.
#[test]
fn cud_chew_eats_again_past_unnerve_and_only_for_the_berrys_status() {
    // (berry, burned, foe's Unnerve started)
    for (berry, burned, unnerved) in [
        (ItemId::SITRUS_BERRY, false, true),
        (ItemId::RAWST_BERRY, true, false),
        (ItemId::CHERI_BERRY, true, false),
    ] {
        let mut arena = setup_with(
            Generation::Gen9,
            [
                set("Farigiraf", &["Harden"], "Cud Chew", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Snorlax", &["Harden"], "Unnerve", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| {
                let farigiraf = &mut pokemon[0];
                farigiraf.hp = farigiraf.max_hp / 4;
                farigiraf.ability_state.cud_chew = Some((berry, 1));
                if burned {
                    farigiraf.status = Some(StatusState::new(NonVolatileStatus::Burn));
                }
                pokemon[1].ability_state.unnerved = unnerved;
            },
        );
        let before = arena.pokemon(A).hp;
        arena.turn(0, 0, vec![]);
        let farigiraf = arena.pokemon(A);
        let case = format!("{berry:?}");
        assert_eq!(farigiraf.ability_state.cud_chew, None, "{case}");
        assert!(farigiraf.history.ate_berry, "{case}");
        if berry == ItemId::SITRUS_BERRY {
            assert_eq!(farigiraf.hp, before + farigiraf.max_hp / 4, "{case}");
        } else {
            assert_eq!(
                farigiraf.status.is_some(),
                berry == ItemId::CHERI_BERRY,
                "{case}"
            );
        }
    }
}

/// Cud Chew counts two turn ends for a berry eaten while the turn still has actions queued — on
/// any turn, not just before the first residual (`BattleState::turn_actions_done` starts over
/// with each turn's choices).
#[test]
fn cud_chew_counts_two_turn_ends_for_a_berry_eaten_mid_turn() {
    let mut arena = setup(
        Generation::Gen9,
        [
            holding(
                set("Farigiraf", &["Harden"], "Cud Chew", None),
                "Sitrus Berry",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Snorlax", &["Harden"], "Thick Fat", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
    );
    arena.turn(0, 0, vec![]);
    let mut snapshot = arena.engine.snapshot();
    let farigiraf = snapshot.state.pokemon_mut(A).unwrap();
    farigiraf.hp = farigiraf.max_hp / 4;
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    arena.turn(0, 0, vec![]);
    let farigiraf = arena.pokemon(A);
    assert_eq!(farigiraf.held_item, None);
    assert_eq!(
        farigiraf.ability_state.cud_chew,
        Some((ItemId::SITRUS_BERRY, 1))
    );
}

/// Jaboca / Rowap Berry (DESIGN.md R208): only a move of the berry's category sets it off, and
/// not one from a Magic Guard attacker (`!source.hasAbility('magicguard')`) — the berry stays.
#[test]
fn recoil_berries_need_their_category_and_no_magic_guard() {
    // (the attacker's ability, its move: 0 Tackle / 1 Swift, the berry kept)
    for (ability, slot, kept) in [
        ("Magic Guard", 0, true),
        ("Cute Charm", 1, true),
        ("Cute Charm", 0, false),
    ] {
        let mut arena = setup(
            Generation::Gen9,
            [
                set("Clefable", &["Tackle", "Swift"], ability, None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                holding(
                    set("Snorlax", &["Harden"], "Thick Fat", None),
                    "Jaboca Berry",
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
        );
        let max_hp = arena.pokemon(A).max_hp;
        arena.turn(slot, 0, hit());
        let case = format!("{ability}, move {slot}");
        assert_eq!(arena.pokemon(B).held_item.is_some(), kept, "{case}");
        let lost = if kept { 0 } else { max_hp / 8 };
        assert_eq!(arena.pokemon(A).hp, max_hp - lost, "{case}");
    }
}

/// Enigma Berry (DESIGN.md R208): not for a fixed damage — Showdown never sets its `typeMod` —
/// and, from Gen 5 where Heal Block stops every heal (`TryHeal`), not under Heal Block; Gen 4's
/// stops only moves.
#[test]
fn enigma_berry_skips_fixed_damage_and_a_heal_block_from_gen_5() {
    let arena_for = |generation: Generation, heal_block: bool| {
        setup_with(
            generation,
            [
                set(
                    "Mismagius",
                    &["Night Shade", "Shadow Ball"],
                    "Levitate",
                    None,
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                holding(
                    set("Espeon", &["Harden"], "Synchronize", None),
                    "Enigma Berry",
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| {
                if heal_block {
                    pokemon[1].volatiles.set_heal_block(5);
                }
            },
        )
    };
    // Night Shade: Ghost on a Psychic type, but a fixed damage.
    let mut arena = arena_for(Generation::Gen9, false);
    arena.turn(0, 0, vec![]);
    assert_eq!(arena.pokemon(B).held_item, Some(ItemId::ENIGMA_BERRY));
    // Shadow Ball under Heal Block.
    for (generation, eaten) in [(Generation::Gen4, true), (Generation::Gen9, false)] {
        let mut arena = arena_for(generation, true);
        let mut tape = hit();
        tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
        arena.turn(1, 0, tape);
        assert_eq!(
            arena.pokemon(B).held_item.is_none(),
            eaten,
            "{generation:?}"
        );
    }
}

/// Mental Herb (DESIGN.md R209): from Gen 5 one use ends every condition it cures — here an
/// infatuation, Taunt, Encore, Disable and Heal Block at once; Gen 3-4 cure only the infatuation,
/// and the rest stay.
#[test]
fn mental_herb_cures_what_the_generation_lets_it_at_once() {
    for (generation, all) in [(Generation::Gen4, false), (Generation::Gen9, true)] {
        let mut arena = setup_with(
            generation,
            [
                holding(
                    set(
                        "Snorlax",
                        &["Harden", "Tackle"],
                        "Thick Fat",
                        Some(Gender::Male),
                    ),
                    "Mental Herb",
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Blissey", &["Harden"], "Natural Cure", Some(Gender::Female)),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| {
                let volatiles = &mut pokemon[0].volatiles;
                volatiles.set_attract(B);
                volatiles.set_taunt(3);
                volatiles.set_encore(MoveId::TACKLE, 3);
                volatiles.set_disable(MoveId::HARDEN, 4);
                volatiles.set_heal_block(5);
            },
        );
        // Blissey's Harden is the first action; its `Update` reaches Snorlax's herb (Snorlax,
        // encored, chose Tackle).
        let choices = [arena.choice(SideId::P1, 1), arena.choice(SideId::P2, 0)];
        arena.begin_turn(choices, vec![]);
        arena.engine.step().unwrap();
        let holder = arena.pokemon(A);
        assert_eq!(holder.held_item, None, "{generation:?}");
        assert!(!holder.volatiles.contains(VolatileConditionKind::Attract));
        for kind in [
            VolatileConditionKind::Taunt,
            VolatileConditionKind::Encore,
            VolatileConditionKind::Disable,
            VolatileConditionKind::HealBlock,
        ] {
            assert_eq!(
                holder.volatiles.contains(kind),
                !all,
                "{generation:?} {kind:?}"
            );
        }
    }
}

/// Adrenaline Orb (DESIGN.md R210): Intimidate's drop makes the holder use it — but not when its
/// Attack has no room left to fall (Showdown's `boost.atk === 0`), not with its Speed at +6, and
/// not for another drop (Growl).
#[test]
fn adrenaline_orb_needs_intimidate_and_room() {
    // (Attack stage, Speed stage, Intimidate (else Growl), used)
    for (attack, speed, intimidate, used) in [
        (0, 0, true, true),
        (-6, 0, true, false),
        (0, 6, true, false),
        (0, 0, false, false),
    ] {
        let mut arena = setup_with(
            Generation::Gen9,
            [
                holding(
                    set("Snorlax", &["Harden"], "Thick Fat", None),
                    "Adrenaline Orb",
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Blissey", &["Growl"], "Natural Cure", None),
                set("Arcanine", &["Harden"], "Intimidate", None),
            ],
            |pokemon| {
                pokemon[0].stages.attack = attack;
                pokemon[0].stages.speed = speed;
            },
        );
        let p2 = if intimidate {
            arena.switch(SideId::P2, D)
        } else {
            arena.choice(SideId::P2, 0)
        };
        arena.turn_with([arena.choice(SideId::P1, 0), p2], vec![]);
        assert_eq!(
            arena.pokemon(A).held_item.is_none(),
            used,
            "{attack} {speed} {intimidate}"
        );
    }
}

/// Blunder Policy (DESIGN.md R210): only the move's own accuracy check (`hitStepAccuracy`) sets
/// it off — a multi-accuracy move's later hit that misses leaves it held.
#[test]
fn blunder_policy_ignores_a_later_hits_miss() {
    let mut arena = setup(
        Generation::Gen9,
        [
            holding(
                set("Weavile", &["Triple Axel"], "Pressure", None),
                "Blunder Policy",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Snorlax", &["Harden"], "Thick Fat", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit());
    tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, false));
    arena.turn(0, 0, tape);
    assert_eq!(arena.pokemon(A).held_item, Some(ItemId::BLUNDER_POLICY));
    assert_eq!(arena.pokemon(A).stages.speed, 0);
}

/// Big Root (DESIGN.md R220, `onTryHealPriority` 1) boosts a drain before Liquid Ooze turns it
/// into damage: the user loses the boosted amount.
#[test]
fn big_root_boosts_a_drain_before_liquid_ooze() {
    for (item, gen9) in [
        (Some("Big Root"), true),
        (None, true),
        (Some("Big Root"), false),
    ] {
        let generation = if gen9 {
            Generation::Gen9
        } else {
            Generation::Gen4
        };
        let venusaur = set("Venusaur", &["Giga Drain"], "Overgrow", None);
        let mut arena = setup(
            generation,
            [
                match item {
                    Some(item) => holding(venusaur, item),
                    None => venusaur,
                },
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Tentacruel", &["Harden"], "Liquid Ooze", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
        );
        let before = arena.pokemon(A).hp;
        let foe_before = arena.pokemon(B).hp;
        arena.turn(0, 0, hit());
        // Showdown rounds a drain (`Math.round`) from Gen 5; the gen4 mod truncates.
        let dealt = foe_before - arena.pokemon(B).hp;
        let drained = if gen9 { dealt.div_ceil(2) } else { dealt / 2 };
        let lost = before - arena.pokemon(A).hp;
        let expected = match (item, gen9) {
            (None, _) => drained,
            (Some(_), true) => ((u32::from(drained) * 5324 + 2047) / 4096) as u16,
            (Some(_), false) => drained * 13 / 10,
        };
        assert_eq!(lost, expected, "{item:?} {generation:?}");
    }
}

/// Big Root on Leech Seed (DESIGN.md R220): Gen 4 truncates `12 × 1.3` to 15, from Gen 5 the
/// 4096-based 5324 rounds it to 16 — here as Liquid Ooze's damage on the seeder.
#[test]
fn big_root_rounds_as_the_generation_does() {
    for (generation, expected) in [(Generation::Gen4, 15), (Generation::Gen9, 16)] {
        let mut arena = setup_with(
            generation,
            [
                holding(
                    set("Venusaur", &["Leech Seed"], "Overgrow", None),
                    "Big Root",
                ),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                set("Tentacruel", &["Harden"], "Liquid Ooze", None),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| {
                // Max HP 96..=103: Leech Seed takes 12.
                pokemon[1].max_hp = 97;
                pokemon[1].hp = 97;
            },
        );
        let before = arena.pokemon(A).hp;
        arena.turn(
            0,
            0,
            vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)],
        );
        assert_eq!(before - arena.pokemon(A).hp, expected, "{generation:?}");
    }
}

/// Shell Bell (DESIGN.md R232) heals nothing for a user its target's Red Card is dragging out
/// (Showdown's `!pokemon.forceSwitchFlag`).
#[test]
fn shell_bell_skips_a_user_being_dragged_out() {
    for red_card in [false, true] {
        let blissey = set("Blissey", &["Harden"], "Natural Cure", None);
        let mut arena = setup_with(
            Generation::Gen9,
            [
                holding(set("Snorlax", &["Tackle"], "Thick Fat", None), "Shell Bell"),
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            [
                if red_card {
                    holding(blissey, "Red Card")
                } else {
                    blissey
                },
                set("Snorlax", &["Harden"], "Thick Fat", None),
            ],
            |pokemon| pokemon[0].hp = 100,
        );
        arena.turn(0, 0, hit());
        assert_eq!(arena.pokemon(A).hp > 100, !red_card, "{red_card}");
    }
}

/// Shell Bell heals at least 1 (Showdown `heal`, pokeplatinum `BattleSystem_Divide`): a level 5
/// Seismic Toss's 5 damage heals 1.
#[test]
fn shell_bell_heals_at_least_one() {
    let mut arena = setup_with(
        Generation::Gen9,
        [
            holding(
                set("Snorlax", &["Seismic Toss"], "Thick Fat", None),
                "Shell Bell",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Blissey", &["Harden"], "Natural Cure", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        |pokemon| {
            pokemon[0].level = 5;
            pokemon[0].hp = pokemon[0].max_hp - 3;
        },
    );
    let before = arena.pokemon(A).hp;
    arena.turn(0, 0, vec![]);
    assert_eq!(arena.pokemon(A).hp, before + 1);
}

/// White Herb at the end of the turn (DESIGN.md R233, `onResidual` 29): Syrup Bomb's residual
/// drop (order 14) is set back before the turn ends.
#[test]
fn white_herb_answers_a_residual_drop() {
    let mut arena = setup(
        Generation::Gen9,
        [
            holding(set("Snorlax", &["Harden"], "Thick Fat", None), "White Herb"),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Dipplin", &["Syrup Bomb"], "Gluttony", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit());
    arena.turn(0, 0, tape);
    let holder = arena.pokemon(A);
    assert_eq!(holder.held_item, None);
    assert_eq!(holder.stages.speed, 0);
}

/// Mirror Herb (DESIGN.md R247, `onFoeAfterBoost` → `onAnyAfterMove`) passes the foe's raises
/// on in the order they came (Showdown's `boosts` object): the holder's stages rise in the order
/// Shift Gear raised the foe's.
#[test]
fn mirror_herb_passes_raises_on_in_the_order_they_came() {
    let mut arena = setup(
        Generation::Gen9,
        [
            holding(
                set("Snorlax", &["Harden"], "Thick Fat", None),
                "Mirror Herb",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Corviknight", &["Shift Gear"], "Pressure", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
    );
    let start = arena.turn(0, 0, vec![]);
    let holder = arena.pokemon(A);
    assert_eq!(holder.held_item, None);
    assert_eq!((holder.stages.speed, holder.stages.attack), (2, 1));
    let raised = |who: PokemonId| -> Vec<Stat> {
        arena.engine.trace().events()[start..]
            .iter()
            .filter_map(|e| match e {
                TraceEvent::StatChanged { target, stat, .. } if *target == who => Some(*stat),
                _ => None,
            })
            .collect()
    };
    // The holder's own Harden comes after the herb.
    assert_eq!(raised(A), [raised(B), vec![Stat::Defense]].concat());
}

/// A set without an ability before Gen 3.
fn set_for(generation: Generation, species: &str, moves: &[&str], ability: &str) -> PokemonSet {
    let mut set = set(species, moves, ability, None);
    if generation < Generation::Gen3 {
        set.ability = None;
    }
    set
}

/// Leppa Berry and Mystery Berry (DESIGN.md R248, issues.md I49): the first move with no PP
/// left (Mean Look, 8 at most) gets 10 (Mystery Berry 5) back, up to its max — at the `Update`
/// after the holder's action from Gen 4, at the end of the turn in Gen 2-3 (the berry still held
/// when the slower foe moves).
#[test]
fn pp_berries_restore_the_first_move_with_no_pp() {
    for (generation, berry, restored, at_update) in [
        (Generation::Gen2, "Mystery Berry", 5, false),
        (Generation::Gen3, "Leppa Berry", 8, false),
        (Generation::Gen4, "Leppa Berry", 8, true),
        (Generation::Gen9, "Leppa Berry", 8, true),
    ] {
        let moves = &["Mean Look", "Harden"];
        let mut arena = setup_with(
            generation,
            [
                holding(set_for(generation, "Snorlax", moves, "Thick Fat"), berry),
                set_for(generation, "Snorlax", &["Harden"], "Thick Fat"),
            ],
            [
                set_for(generation, "Slowpoke", &["Harden"], "Oblivious"),
                set_for(generation, "Snorlax", &["Harden"], "Thick Fat"),
            ],
            |pokemon| pokemon[0].moves[0].pp = 0,
        );
        let start = arena.begin_turn(
            [arena.choice(SideId::P1, 1), arena.choice(SideId::P2, 0)],
            vec![],
        );
        // Snorlax moves first; step until the slower Slowpoke's move is used.
        while arena.engine.trace().events()[start..]
            .iter()
            .all(|e| !matches!(e, TraceEvent::MoveUsed { actor: B, .. }))
        {
            arena.engine.step().unwrap();
        }
        assert_eq!(
            arena.pokemon(A).held_item.is_none(),
            at_update,
            "{generation:?}: eaten at the Update after the holder's action"
        );
        while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
        let holder = arena.pokemon(A);
        assert_eq!(holder.held_item, None, "{generation:?}");
        assert_eq!(holder.moves[0].pp, restored, "{generation:?}");
    }
}

/// Ripen doubles Leppa Berry's PP (Gen 8+).
#[test]
fn ripen_doubles_a_leppa_berry() {
    let mut arena = setup_with(
        Generation::Gen9,
        [
            holding(
                set("Appletun", &["Defense Curl", "Harden"], "Ripen", None),
                "Leppa Berry",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Slowpoke", &["Harden"], "Oblivious", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        |pokemon| pokemon[0].moves[0].pp = 0,
    );
    arena.turn(1, 0, vec![]);
    assert_eq!(arena.pokemon(A).moves[0].pp, 20);
}

/// Cud Chew eats the Leppa Berry again at the second turn end after (`onEat` without its
/// trigger): no move is out of PP then, so the first below its max gets the 10.
#[test]
fn cud_chew_eats_a_leppa_berry_again_for_the_first_move_below_its_max() {
    let mut arena = setup_with(
        Generation::Gen9,
        [
            holding(
                set("Farigiraf", &["Defense Curl", "Harden"], "Cud Chew", None),
                "Leppa Berry",
            ),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        [
            set("Slowpoke", &["Harden"], "Oblivious", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        |pokemon| pokemon[0].moves[0].pp = 0,
    );
    arena.turn(1, 0, vec![]);
    assert_eq!(arena.pokemon(A).moves[0].pp, 10);
    arena.turn(1, 0, vec![]);
    assert_eq!(arena.pokemon(A).moves[0].pp, 20);
}
