//! L1 rule tests for M7 batch 1 (DESIGN.md §22.6): the damage-formula data components — the
//! critical-hit table by ratio, `willCrit`, crit-ratio boosts and critical-hit immunity, fixed
//! damage, one-hit KOs, stat sources, thawing and the user-and-allies target.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, CriticalRoll, EngineStep, SideChoice, SlotChoice, TraceEvent, accuracy_roll,
    critical_roll, fixed_damage, ohko_blocked,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{
    CriticalTable, DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern,
};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, StatusState,
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

fn set(species: &str, moves: &[&str], ability: Option<&str>, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set.ability = ability.map(Into::into);
    set.item = item.map(Into::into);
    set
}

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

    fn turn(&mut self, p1: usize, p2: usize, tape: Vec<RandomOverride>) -> usize {
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

    fn move_def(&self, id: PokemonId, slot: usize) -> pokemon_battle_engine::dex::ResolvedMove {
        let move_id = self.pokemon(id).moves[slot].move_id;
        self.dex.move_by_id(move_id).unwrap().clone()
    }
}

fn filler() -> PokemonSet {
    set("Snorlax", &["Tackle"], None, None)
}

#[test]
fn the_critical_hit_table_follows_the_generation_and_the_ratio() {
    let table = |generation| {
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap()
            .damage()
            .critical
    };
    // Ratio 1 (ordinary), 2 (Slash), 3, 4.
    let chances = |generation| -> Vec<Option<(u16, u16)>> {
        (1..=4)
            .map(|ratio| table(generation).chance(ratio))
            .collect()
    };
    assert_eq!(
        chances(Generation::Gen9),
        vec![Some((1, 24)), Some((1, 8)), Some((1, 2)), Some((1, 1))]
    );
    assert_eq!(
        chances(Generation::Gen6),
        vec![Some((1, 16)), Some((1, 8)), Some((1, 2)), Some((1, 1))]
    );
    assert_eq!(
        chances(Generation::Gen4),
        vec![Some((1, 16)), Some((1, 8)), Some((1, 4)), Some((1, 3))]
    );
    assert_eq!(
        chances(Generation::Gen2),
        vec![
            Some((17, 256)),
            Some((32, 256)),
            Some((64, 256)),
            Some((85, 256))
        ]
    );
    assert_eq!(table(Generation::Gen1), CriticalTable::Gen1BaseSpeed);
    // Gen 1 Persian (base 115): ⌊115 / 2⌋ × 2 = 114 → 57 ordinary, 255 for Slash.
    assert_eq!(CriticalTable::gen1_chance(115, 1, false), (57, 256));
    assert_eq!(CriticalTable::gen1_chance(115, 2, false), (255, 256));
    // Under Focus Energy (the cartridge's bug) the chance is halved instead of doubled:
    // ⌊⌊115 / 2⌋ / 2⌋ = 28 → 14 ordinary, 112 for Slash.
    assert_eq!(CriticalTable::gen1_chance(115, 1, true), (14, 256));
    assert_eq!(CriticalTable::gen1_chance(115, 2, true), (112, 256));
}

#[test]
fn crit_ratio_boosts_stack_and_battle_armor_turns_a_critical_hit_back() {
    // Gen 9 Night Slash (2) + Scope Lens (+1) + Super Luck (+1) = 4: always, no roll.
    let arena = singles(
        Generation::Gen9,
        [
            set(
                "Honchkrow",
                &["Night Slash", "Frost Breath", "Tackle"],
                Some("Super Luck"),
                Some("Scope Lens"),
            ),
            set("Corviknight", &["Tackle"], Some("Battle Armor"), None),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let state = arena.engine.state();
    let dex = &arena.dex;
    let rules = &arena.rules;
    assert_eq!(
        critical_roll(state, rules, dex, A, B, &arena.move_def(A, 0)),
        CriticalRoll::Always
    );
    // Frost Breath: `willCrit`.
    assert_eq!(
        critical_roll(state, rules, dex, A, B, &arena.move_def(A, 1)),
        CriticalRoll::Always
    );
    // Tackle: 1 + 2 = 3 → 1/2.
    assert_eq!(
        critical_roll(state, rules, dex, A, B, &arena.move_def(A, 2)),
        CriticalRoll::Chance(1, 2)
    );
    // Battle Armor: the landed critical hit is turned back.
    let mut arena = arena;
    let start = arena.turn(
        0,
        0,
        vec![
            RandomOverride::range(RandomRequestKind::DamageRoll, 100),
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, 100),
        ],
    );
    let critical: Vec<bool> = arena
        .events(start)
        .iter()
        .filter_map(|e| match e {
            TraceEvent::CriticalChecked {
                actor, critical, ..
            } if *actor == A => Some(*critical),
            _ => None,
        })
        .collect();
    assert_eq!(critical, vec![false]);
}

#[test]
fn fixed_damage_ignores_the_formula() {
    let arena = singles(
        Generation::Gen9,
        [
            set("Chansey", &["Seismic Toss", "Super Fang"], None, None),
            set("Snorlax", &["Tackle"], None, None),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[1].hp = 3,
    );
    let state = arena.engine.state();
    assert_eq!(
        fixed_damage(state, &arena.rules, A, B, &arena.move_def(A, 0)),
        Some(50)
    );
    // Super Fang: half the current HP, rounded down, at least 1.
    assert_eq!(
        fixed_damage(state, &arena.rules, A, B, &arena.move_def(A, 1)),
        Some(1)
    );
    // Gen 1 Seismic Toss ignores type immunity (a Ghost takes it); Gen 9 does not.
    for (generation, hits) in [(Generation::Gen1, true), (Generation::Gen9, false)] {
        let mut arena = singles(
            generation,
            [
                set("Chansey", &["Seismic Toss"], None, None),
                set("Gengar", &["Hypnosis"], None, None),
                filler(),
                filler(),
            ],
            |_| {},
        );
        let before = arena.pokemon(B).hp;
        let mut tape = Vec::new();
        if generation == Generation::Gen1 {
            // Gengar is faster: Hypnosis (60 %) rolls; Seismic Toss (100 %) rolls in Gen 1.
            tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, false));
            tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, true));
        } else {
            tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, false));
        }
        arena.turn(0, 0, tape);
        assert_eq!(arena.pokemon(B).hp == before - 50, hits, "{generation:?}");
    }
}

#[test]
fn one_hit_ko_rules_follow_the_generation() {
    // Gen 9 Sheer Cold from a non-Ice user: 20 + level gap; against an Ice type: immune.
    let arena = singles(
        Generation::Gen9,
        [
            set("Snorlax", &["Sheer Cold", "Horn Drill"], None, None),
            set("Glaceon", &["Tackle"], None, None),
            filler(),
            set("Garchomp", &["Tackle"], None, None),
        ],
        |pokemon| pokemon[0].level = 55,
    );
    let state = arena.engine.state();
    let rules = &arena.rules;
    let sheer_cold = arena.move_def(A, 0);
    let horn_drill = arena.move_def(A, 1);
    assert!(ohko_blocked(state, rules, A, B, &sheer_cold));
    assert!(!ohko_blocked(state, rules, A, B, &horn_drill));
    assert_eq!(
        accuracy_roll(state, rules, A, B, &horn_drill),
        Some((35, 100))
    );
    assert_eq!(
        accuracy_roll(state, rules, A, D, &sheer_cold),
        Some((25, 100))
    );
    // Gen 3 deals the target's current HP, Gen 4 its max HP.
    for (generation, current) in [(Generation::Gen3, true), (Generation::Gen4, false)] {
        let arena = singles(
            generation,
            [
                set("Snorlax", &["Horn Drill"], None, None),
                set("Blissey", &["Tackle"], None, None),
                filler(),
                filler(),
            ],
            |pokemon| pokemon[1].hp = 10,
        );
        let state = arena.engine.state();
        let max_hp = arena.pokemon(B).max_hp;
        assert_eq!(
            fixed_damage(state, &arena.rules, A, B, &arena.move_def(A, 0)),
            Some(if current { 10 } else { max_hp }),
            "{generation:?}"
        );
    }
    // Gen 2: a byte with twice the level gap; Gen 1: a faster target is immune.
    let arena = singles(
        Generation::Gen2,
        [
            set("Snorlax", &["Horn Drill"], None, None),
            set("Golem", &["Tackle"], None, None),
            filler(),
            filler(),
        ],
        |pokemon| pokemon[0].level = 55,
    );
    let horn_drill = arena.move_def(A, 0);
    assert_eq!(
        accuracy_roll(arena.engine.state(), &arena.rules, A, B, &horn_drill),
        Some((76 + 10, 256))
    );
    let arena = singles(
        Generation::Gen1,
        [
            set("Dugtrio", &["Fissure"], None, None),
            set("Jolteon", &["Tackle"], None, None),
            filler(),
            filler(),
        ],
        |_| {},
    );
    let fissure = arena.move_def(A, 0);
    assert!(ohko_blocked(
        arena.engine.state(),
        &arena.rules,
        A,
        B,
        &fissure
    ));
}

#[test]
fn a_gen_2_defrost_move_thaws_its_user_only_once_it_lands() {
    // Frozen Slowbro uses Flame Wheel (defrost): no thaw roll at BeforeMove; missing leaves it
    // frozen, landing thaws it afterwards.
    for (lands, thawed) in [(false, false), (true, true)] {
        let mut arena = singles(
            Generation::Gen2,
            [
                set("Slowbro", &["Flame Wheel"], None, None),
                set("Chansey", &["Growl"], None, None),
                filler(),
                filler(),
            ],
            |pokemon| {
                pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Freeze));
                pokemon[0].stages.accuracy = -1;
            },
        );
        let mut tape = vec![
            // Growl (100 %) never rolls in Gen 2; Flame Wheel at −1 accuracy does.
            RandomOverride::bool(RandomRequestKind::Accuracy, lands),
        ];
        if lands {
            tape.push(RandomOverride::bool(RandomRequestKind::Critical, false));
            tape.push(RandomOverride::range(RandomRequestKind::DamageRoll, 217));
            tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
        }
        if !thawed {
            // Still frozen: the residual thaw roll (Gen 2, 25/256).
            tape.push(RandomOverride::bool(RandomRequestKind::Thaw, false));
        }
        arena.turn(0, 0, tape);
        assert_eq!(arena.pokemon(A).status.is_none(), thawed, "lands: {lands}");
    }
}

#[test]
fn howl_and_life_dew_reach_the_user_and_its_allies_in_position_order() {
    let rules = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen9,
        BattleMode::Doubles,
        "custom",
    ))
    .unwrap();
    let dex = DexResolver::resolve(Generation::Gen9);
    let pokemon: Vec<PokemonState> = (1..=4)
        .map(|n| {
            let side = if n <= 2 { SideId::P1 } else { SideId::P2 };
            build_pokemon(
                &dex,
                PokemonId(n),
                side,
                &set("Slowbro", &["Life Dew"], None, None),
            )
            .unwrap()
        })
        .collect();
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 2);
        side.active = vec![Some(team[0]), Some(team[1])];
        side
    };
    let state = BattleState::new(
        pokemon,
        [
            side(vec![PokemonId(1), PokemonId(2)]),
            side(vec![PokemonId(3), PokemonId(4)]),
        ],
        1,
    );
    let targets = rules
        .targeting()
        .resolve(
            rules.topology(),
            &state,
            PokemonId(2),
            TargetPattern::UserAndAllies,
            None,
        )
        .unwrap();
    assert_eq!(
        targets.pokemon_ids().collect::<Vec<_>>(),
        vec![PokemonId(1), PokemonId(2)]
    );
}

#[test]
fn the_attacking_stat_chains_its_modifiers_and_rounds_once() {
    // Mew's Special Attack at L51 is 122 (Hardy, 31 IVs); Gen 9 Transistor chains 5325 / 4096:
    // Showdown `modify` gives ⌊(122 × 5325 + 2047) / 4096⌋ = 159 (158.6 floored would be 158).
    // With Choice Specs the chain is 6144 ⊗ 5325 = 7988: 238 (237 floored).
    for (item, expected) in [(None, 159), (Some("Choice Specs"), 238)] {
        let mut mew = set("Mew", &["Thunderbolt"], Some("Transistor"), item);
        mew.level = 51;
        let mut arena = singles(
            Generation::Gen9,
            [
                mew,
                set("Snorlax", &["Harden"], None, None),
                filler(),
                filler(),
            ],
            |_| {},
        );
        let start = arena.turn(
            0,
            0,
            vec![
                RandomOverride::bool(RandomRequestKind::Critical, false),
                RandomOverride::range(RandomRequestKind::DamageRoll, 100),
                RandomOverride::bool(RandomRequestKind::Secondary, false),
            ],
        );
        let offense = arena
            .events(start)
            .iter()
            .find_map(|event| match event {
                TraceEvent::OffensiveStatResolved {
                    base, after_item, ..
                } => Some((*base, *after_item)),
                _ => None,
            })
            .expect("an offensive stat");
        assert_eq!(offense, (122, expected), "{item:?}");
    }
}

#[test]
fn unaware_and_weight_modifiers() {
    use pokemon_battle_engine::engine::{accuracy_roll, weight_hg};
    // Unaware sets the other side's accuracy and evasion stages aside — invisible to the
    // replay, which asks the engine whether to plan the roll.
    let roll = |user: PokemonSet, target: PokemonSet, stages: (i8, i8)| {
        let arena = singles(
            Generation::Gen9,
            [user, target, filler(), filler()],
            |pokemon| {
                pokemon[0].stages.accuracy = stages.0;
                pokemon[1].stages.evasion = stages.1;
            },
        );
        accuracy_roll(
            arena.engine.state(),
            &arena.rules,
            A,
            B,
            &arena.move_def(A, 0),
        )
    };
    let wall = |ability: Option<&str>| set("Snorlax", &["Harden"], ability, None);
    // The holder's Body Slam against +6 evasion: no roll; without Unaware, 100 / 3.
    assert_eq!(
        roll(
            set("Mew", &["Body Slam"], Some("Unaware"), None),
            wall(None),
            (0, 6)
        ),
        None
    );
    assert_eq!(
        roll(set("Mew", &["Body Slam"], None, None), wall(None), (0, 6)),
        Some((33, 100))
    );
    // Zap Cannon at +6 accuracy would never roll; against Unaware it rolls its 50.
    assert_eq!(
        roll(
            set("Mew", &["Zap Cannon"], None, None),
            wall(Some("Unaware")),
            (6, 0)
        ),
        Some((50, 100))
    );
    assert_eq!(
        roll(set("Mew", &["Zap Cannon"], None, None), wall(None), (6, 0)),
        None
    );
    // Weight: Heavy Metal (priority 1) before Float Stone (0), each step truncated: Jolteon's
    // 245 doubles to 490 and halves back to 245 (the other way round: 244).
    let weight = |ability: Option<&str>, item: Option<&str>| {
        let arena = singles(
            Generation::Gen9,
            [
                set("Jolteon", &["Harden"], ability, item),
                wall(None),
                filler(),
                filler(),
            ],
            |_| {},
        );
        weight_hg(arena.engine.state(), &arena.rules, &arena.dex, A)
    };
    assert_eq!(weight(None, None), 245);
    assert_eq!(weight(Some("Heavy Metal"), None), 490);
    assert_eq!(weight(Some("Light Metal"), None), 122);
    assert_eq!(weight(Some("Heavy Metal"), Some("Float Stone")), 245);
}

#[test]
fn abilities_edit_the_move_as_it_starts() {
    // Showdown `onModifyMove` as data, through `modified_move` (the engine and the replay both
    // read it). The replay cannot see an accuracy check Showdown skips nor one per hit it
    // skips, so those are pinned here.
    use pokemon_battle_engine::dex::MultiHit;
    use pokemon_battle_engine::engine::{accuracy_roll, modified_move};
    let edited = |generation, ability: &str, moves: &[&str], evasion: i8| {
        let arena = singles(
            generation,
            [
                set("Mew", moves, Some(ability), None),
                set("Snorlax", &["Harden"], None, None),
                filler(),
                filler(),
            ],
            |pokemon| pokemon[1].stages.evasion = evasion,
        );
        let edited = modified_move(
            arena.engine.state(),
            &arena.rules,
            A,
            &arena.move_def(A, 0),
            false,
        )
        .0;
        let accuracy = accuracy_roll(arena.engine.state(), &arena.rules, A, B, &edited);
        (edited, accuracy)
    };
    // Keen Eye ignores the target's +6 evasion from Gen 6 (the gen5 mod drops its
    // `onModifyMove`): Body Slam never rolls; Gen 5 rolls at 100 / 3.
    assert_eq!(
        edited(Generation::Gen9, "Keen Eye", &["Body Slam"], 6).1,
        None
    );
    assert_eq!(
        edited(Generation::Gen5, "Keen Eye", &["Body Slam"], 6).1,
        Some((33, 100))
    );
    assert_eq!(
        edited(Generation::Gen9, "Mind's Eye", &["Body Slam"], 6).1,
        None
    );
    // Skill Link: a range strikes its maximum; Triple Axel checks accuracy on its first hit only.
    let (bullet_seed, _) = edited(Generation::Gen9, "Skill Link", &["Bullet Seed"], 0);
    assert_eq!(bullet_seed.multi_hit, MultiHit::Fixed(5));
    let (triple_axel, _) = edited(Generation::Gen9, "Skill Link", &["Triple Axel"], 0);
    assert!(!triple_axel.multi_accuracy);
    assert!(
        edited(Generation::Gen9, "Run Away", &["Triple Axel"], 0)
            .0
            .multi_accuracy
    );
    // Scrappy: only a Normal or Fighting move ignores the chart's 0×.
    assert!(
        edited(Generation::Gen9, "Scrappy", &["Body Slam"], 0)
            .0
            .flags
            .immunity_ignored
    );
    assert!(
        !edited(Generation::Gen9, "Scrappy", &["Psychic"], 0)
            .0
            .flags
            .immunity_ignored
    );
    // Long Reach: no contact; Unseen Fist: a contact move loses `protect`, another keeps it;
    // Propeller Tail tracks its target.
    assert!(
        !edited(Generation::Gen9, "Long Reach", &["Body Slam"], 0)
            .0
            .flags
            .contact
    );
    assert!(
        !edited(Generation::Gen9, "Unseen Fist", &["Body Slam"], 0)
            .0
            .flags
            .protectable
    );
    assert!(
        edited(Generation::Gen9, "Unseen Fist", &["Psychic"], 0)
            .0
            .flags
            .protectable
    );
    assert!(
        edited(Generation::Gen9, "Propeller Tail", &["Body Slam"], 0)
            .0
            .tracks_target
    );
    // Sheer Force: every secondary goes, target-directed ones included — a burn that never
    // lands looks the same to the replay whether it was rolled or not — and so does Diamond
    // Storm's own `self`; a move without secondaries keeps everything and gets no boost.
    let (flamethrower, _) = edited(Generation::Gen9, "Sheer Force", &["Flamethrower"], 0);
    assert!(flamethrower.secondaries.is_empty() && flamethrower.flags.sheer_force);
    let (diamond_storm, _) = edited(Generation::Gen9, "Sheer Force", &["Diamond Storm"], 0);
    assert!(diamond_storm.secondaries.is_empty() && diamond_storm.flags.sheer_force);
    let (close_combat, _) = edited(Generation::Gen9, "Sheer Force", &["Close Combat"], 0);
    assert!(!close_combat.secondaries.is_empty() && !close_combat.flags.sheer_force);
}

#[test]
fn parental_bond_prepares_single_target_damaging_moves() {
    // `prepared_move` (Showdown `onPrepareHit`): a status move never reaches the hit sequence,
    // so the replay cannot see it left alone; a spread hit neither.
    use pokemon_battle_engine::dex::MultiHit;
    use pokemon_battle_engine::engine::{prepared_move, secondary_withheld};
    let prepared_in = |generation, moves: &[&str], targets: usize| {
        let arena = singles(
            generation,
            [
                set("Mew", moves, Some("Parental Bond"), None),
                set("Snorlax", &["Harden"], None, None),
                filler(),
                filler(),
            ],
            |_| {},
        );
        prepared_move(
            arena.engine.state(),
            &arena.rules,
            A,
            arena.move_def(A, 0),
            targets,
            None,
        )
    };
    let prepared = |moves: &[&str], targets: usize| prepared_in(Generation::Gen9, moves, targets);
    let body_slam = prepared(&["Body Slam"], 1);
    assert_eq!(body_slam.multi_hit, MultiHit::Fixed(2));
    assert!(body_slam.flags.parental_bond);
    let swords_dance = prepared(&["Swords Dance"], 1);
    assert_eq!(swords_dance.multi_hit, MultiHit::Single);
    assert!(!swords_dance.flags.parental_bond);
    assert_eq!(prepared(&["Rock Slide"], 2).multi_hit, MultiHit::Single);
    // Secret Power (`onSourceModifySecondaries`): its own secondary waits for the second hit —
    // the replay reads the paralysis off the log either way.
    let secret_power = prepared_in(Generation::Gen7, &["Secret Power"], 1);
    let own = secret_power.secondaries[0];
    let gen7 = RulesResolver::resolve(&BattleSpec::new(
        Generation::Gen7,
        BattleMode::Singles,
        "custom",
    ))
    .unwrap();
    assert!(secondary_withheld(&gen7, &secret_power, 1, 2, &own));
    assert!(!secondary_withheld(&gen7, &secret_power, 2, 2, &own));
}

#[test]
fn accuracy_modifiers_follow_the_generation() {
    use pokemon_battle_engine::engine::accuracy_roll;
    let roll = |generation, user: PokemonSet, target: PokemonSet, stages: (i8, i8)| {
        let arena = singles(generation, [user, target, filler(), filler()], |pokemon| {
            pokemon[0].stages.accuracy = stages.0;
            pokemon[1].stages.evasion = stages.1;
        });
        accuracy_roll(
            arena.engine.state(),
            &arena.rules,
            A,
            B,
            &arena.move_def(A, 0),
        )
    };
    let mew = |moves: &[&str], ability: Option<&str>| set("Mew", moves, ability, None);
    let wall = |ability: Option<&str>| set("Snorlax", &["Harden"], ability, None);
    let (none, g4, g9) = ((0, 0), Generation::Gen4, Generation::Gen9);

    // Compound Eyes: Gen 5+ [5325, 4096] on Sing's 55 = 72; the Gen 4 cartridge's
    // 55 × 130 / 100, floored: 71. On Stone Edge's 80 it reaches 104: no roll.
    let eyes = Some("Compound Eyes");
    assert_eq!(
        roll(g9, mew(&["Sing"], eyes), wall(None), none),
        Some((72, 100))
    );
    assert_eq!(
        roll(g4, mew(&["Sing"], eyes), wall(None), none),
        Some((71, 100))
    );
    assert_eq!(roll(g9, mew(&["Stone Edge"], eyes), wall(None), none), None);
    // Victory Star: Rock Slide's 90 × 4506 / 4096 = 99.
    let star = Some("Victory Star");
    assert_eq!(
        roll(g9, mew(&["Rock Slide"], star), wall(None), none),
        Some((99, 100))
    );
    // Hustle: ×0.8 on physical moves. The Gen 4 cartridge (issues.md I5, DESIGN.md R100)
    // takes the net stage: Dynamic Punch 50 with the user's accuracy −1 and the target's
    // evasion +1 is −2, 50 × 60 / 100 = 30, then × 80 / 100 = 24 (Showdown: 28.125, its
    // stages as floats dropping Hustle).
    let hustle = Some("Hustle");
    assert_eq!(
        roll(g9, mew(&["Body Slam"], hustle), wall(None), none),
        Some((80, 100))
    );
    assert_eq!(
        roll(g4, mew(&["Dynamic Punch"], hustle), wall(None), (-1, 1)),
        Some((24, 100))
    );
    // The cartridge's net stage: Hypnosis 60 at +2 against +1 is +1, 60 × 133 / 100 = 79
    // (Showdown multiplies the stages apart: 60 × 5/3 × 3/4 = 75).
    assert_eq!(
        roll(g4, mew(&["Hypnosis"], None), wall(None), (2, 1)),
        Some((79, 100))
    );
    // Wonder Skin: a status move aimed at the holder has 50 %.
    assert_eq!(
        roll(
            g9,
            mew(&["Sand Attack"], None),
            wall(Some("Wonder Skin")),
            none
        ),
        Some((50, 100))
    );
    // No Guard, on either side: no roll.
    let punch = || mew(&["Dynamic Punch"], None);
    assert_eq!(
        roll(
            g9,
            mew(&["Dynamic Punch"], Some("No Guard")),
            wall(None),
            none
        ),
        None
    );
    assert_eq!(roll(g9, punch(), wall(Some("No Guard")), none), None);
    assert_eq!(roll(g9, punch(), wall(None), none), Some((50, 100)));
}
