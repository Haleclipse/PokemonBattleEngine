//! L1 rule tests for Future Sight and Doom Desire (DESIGN.md §15.22): where each generation
//! draws the random factor of a damage computed at use (Gen 4 at use, Gen 3 after the strike's
//! accuracy check, Gen 2 before it — issues.md I110), and the order of one residual's strikes
//! (Gen 3 by battler, issues.md I111; Gen 4 by speed). The replay plans each decision by kind
//! and reads each strike's outcome by its target, so neither shows in a frame.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, EffectSource, EngineStep, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, FutureAttack, RandomOverride, RandomRequestKind, SideState,
};

const SEER: PokemonId = PokemonId(1);
const WALL: PokemonId = PokemonId(2);

fn set(species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = level;
    set
}

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn arena(
    generation: Generation,
    mode: BattleMode,
    p1: Vec<PokemonSet>,
    p2: Vec<PokemonSet>,
    prepare: impl FnOnce(&mut BattleState),
) -> Arena {
    let rules =
        Arc::new(RulesResolver::resolve(&BattleSpec::new(generation, mode, "custom")).unwrap());
    let dex = Arc::new(DexResolver::resolve(generation));
    let slots = p1.len();
    let mut pokemon = Vec::new();
    let mut teams = [Vec::new(), Vec::new()];
    for (side, sets) in [(SideId::P1, p1), (SideId::P2, p2)] {
        for set in sets {
            let id = PokemonId(pokemon.len() as u8 + 1);
            pokemon.push(build_pokemon(&dex, id, side, &set).unwrap());
            teams[side.index()].push(id);
        }
    }
    let side = |team: &Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), slots);
        for (slot, id) in team.iter().enumerate() {
            side.active[slot] = Some(*id);
        }
        side
    };
    let mut state = BattleState::new(pokemon, [side(&teams[0]), side(&teams[1])], 1);
    prepare(&mut state);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    /// Every active Pokémon uses its move `slot` (at the foe across, if it needs a target),
    /// drawing exactly `tape`.
    fn turn(&mut self, moves: &[(PokemonId, usize)], tape: Vec<RandomOverride>) -> usize {
        let start = self.turn_drawing(moves, tape);
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
        start
    }

    /// `turn`, with `tape` offered but not required: how much of it was drawn is the caller's
    /// to check.
    fn turn_drawing(&mut self, moves: &[(PokemonId, usize)], tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [SideId::P1, SideId::P2].map(|side| SideChoice {
            side,
            actions: moves
                .iter()
                .filter(|(actor, _)| self.engine.state().pokemon(*actor).unwrap().side == side)
                .map(|&(actor, slot)| {
                    let pokemon = self.engine.state().pokemon(actor).unwrap();
                    let move_id = pokemon.moves[slot].move_id;
                    let position = self
                        .engine
                        .state()
                        .active_position(actor, self.rules.topology())
                        .unwrap();
                    let own = self.rules.topology().spec(position).unwrap().slot;
                    let target = (move_id != MoveId::HARDEN).then(|| {
                        DeclaredTarget(self.rules.topology().position(side.other(), own).unwrap())
                    });
                    SlotChoice {
                        actor,
                        action: ActionChoice::Move {
                            slot: MoveSlot::new(slot as u8).unwrap(),
                            target,
                            mechanic: None,
                        },
                    }
                })
                .collect(),
        });
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        start
    }

    fn struck(&self, from: usize) -> Vec<(PokemonId, u16)> {
        self.engine.trace().events()[from..]
            .iter()
            .filter_map(|event| match event {
                TraceEvent::DamageApplied {
                    target,
                    amount,
                    source: EffectSource::Move { move_id, .. },
                    ..
                } if *move_id == MoveId::FUTURE_SIGHT => Some((*target, *amount)),
                _ => None,
            })
            .collect()
    }

    fn stored(&self) -> Option<u16> {
        self.engine
            .state()
            .side(SideId::P2)
            .future_attacks
            .first()
            .and_then(|attack| attack.damage)
    }
}

fn singles(generation: Generation) -> Arena {
    arena(
        generation,
        BattleMode::Singles,
        vec![set("Slowbro", &["Future Sight", "Harden"], 50)],
        vec![set("Snorlax", &["Harden"], 40)],
        |_| {},
    )
}

const HARDEN: [(PokemonId, usize); 2] = [(SEER, 1), (WALL, 0)];

fn accuracy(hit: bool) -> RandomOverride {
    RandomOverride::bool(RandomRequestKind::Accuracy, hit)
}

fn roll(value: u16) -> RandomOverride {
    RandomOverride::range(RandomRequestKind::DamageRoll, value)
}

#[test]
fn gen4_future_sight_rolls_at_use_and_checks_accuracy_at_the_strike() {
    let mut arena = singles(Generation::Gen4);
    arena.turn(&[(SEER, 0), (WALL, 0)], vec![roll(85)]);
    let stored = arena.stored().expect("computed at use");
    arena.turn(&HARDEN, vec![]);
    let start = arena.turn(&HARDEN, vec![accuracy(true)]);
    assert_eq!(arena.struck(start), vec![(WALL, stored)]);
}

#[test]
fn gen3_future_sight_rolls_once_the_strike_hit() {
    let mut arena = singles(Generation::Gen3);
    // The use draws no random factor (one offered stays undrawn).
    arena.turn_drawing(&[(SEER, 0), (WALL, 0)], vec![roll(85)]);
    assert_eq!(arena.engine.state().rng.semantic_tape_cursor(), 0);
    let stored = arena.stored().expect("computed at use, without the factor");
    arena.turn(&HARDEN, vec![]);
    let start = arena.turn(&HARDEN, vec![accuracy(true), roll(85)]);
    assert_eq!(arena.struck(start), vec![(WALL, stored * 85 / 100)]);

    // A miss draws no factor.
    let mut arena = singles(Generation::Gen3);
    arena.turn(&[(SEER, 0), (WALL, 0)], vec![]);
    arena.turn(&HARDEN, vec![]);
    let start = arena.turn(&HARDEN, vec![accuracy(false)]);
    assert!(arena.struck(start).is_empty());
}

#[test]
fn gen2_future_sight_rolls_before_the_strikes_accuracy() {
    let mut arena = singles(Generation::Gen2);
    arena.turn_drawing(&[(SEER, 0), (WALL, 0)], vec![roll(217)]);
    assert_eq!(arena.engine.state().rng.semantic_tape_cursor(), 0);
    let stored = arena.stored().expect("computed at use, without the factor");
    arena.turn(&HARDEN, vec![]);
    // The factor is drawn first, so a miss has drawn it too.
    let start = arena.turn(&HARDEN, vec![roll(217), accuracy(false)]);
    assert!(arena.struck(start).is_empty());

    let mut arena = singles(Generation::Gen2);
    arena.turn(&[(SEER, 0), (WALL, 0)], vec![]);
    arena.turn(&HARDEN, vec![]);
    let start = arena.turn(&HARDEN, vec![roll(217), accuracy(true)]);
    assert_eq!(arena.struck(start), vec![(WALL, stored * 217 / 255)]);
}

#[test]
fn gen5_future_sight_strikes_as_the_move_itself() {
    let mut arena = singles(Generation::Gen5);
    arena.turn(&[(SEER, 0), (WALL, 0)], vec![]);
    assert_eq!(arena.stored(), None);
    arena.turn(&HARDEN, vec![]);
    let start = arena.turn(
        &HARDEN,
        vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            roll(100),
        ],
    );
    assert_eq!(arena.struck(start).len(), 1);
}

/// Two strikes due in one residual on the foes' slots, the right foe the faster.
fn doubles_due(generation: Generation) -> (Arena, PokemonId, PokemonId) {
    let arena = arena(
        generation,
        BattleMode::Doubles,
        vec![
            set("Slowbro", &["Harden"], 50),
            set("Alakazam", &["Harden"], 50),
        ],
        vec![
            set("Snorlax", &["Harden"], 60),
            set("Jolteon", &["Harden"], 50),
        ],
        |state| {
            for (slot, source) in [(0, PokemonId(1)), (1, PokemonId(2))] {
                state
                    .side_mut(SideId::P2)
                    .future_attacks
                    .push(FutureAttack {
                        slot,
                        source,
                        move_id: MoveId::FUTURE_SIGHT,
                        damage: Some(10),
                        turns_remaining: 1,
                    });
            }
        },
    );
    (arena, PokemonId(3), PokemonId(4))
}

#[test]
fn gen3_strikes_go_by_battler_and_gen4_by_speed() {
    let everyone = [
        (PokemonId(1), 0),
        (PokemonId(2), 0),
        (PokemonId(3), 0),
        (PokemonId(4), 0),
    ];
    // Gen 3: the left foe (battler 1) before the right one (battler 3), however fast.
    let (mut arena, left, right) = doubles_due(Generation::Gen3);
    let start = arena.turn(
        &everyone,
        vec![accuracy(true), roll(100), accuracy(true), roll(100)],
    );
    let order: Vec<PokemonId> = arena.struck(start).iter().map(|(id, _)| *id).collect();
    assert_eq!(order, vec![left, right]);

    // Gen 4: the faster right foe first.
    let (mut arena, left, right) = doubles_due(Generation::Gen4);
    let start = arena.turn(&everyone, vec![accuracy(true), accuracy(true)]);
    let order: Vec<PokemonId> = arena.struck(start).iter().map(|(id, _)| *id).collect();
    assert_eq!(order, vec![right, left]);
}

/// A strike due this turn on the foe, with `damage` computed at use.
fn singles_due(generation: Generation, damage: u16) -> Arena {
    arena(
        generation,
        BattleMode::Singles,
        vec![set("Slowbro", &["Future Sight", "Harden"], 50)],
        vec![set("Snorlax", &["Harden"], 40)],
        |state| {
            state
                .side_mut(SideId::P2)
                .future_attacks
                .push(FutureAttack {
                    slot: 0,
                    source: SEER,
                    move_id: MoveId::FUTURE_SIGHT,
                    damage: Some(damage),
                    turns_remaining: 1,
                });
        },
    )
}

#[test]
fn a_damage_of_one_and_its_random_factor() {
    // Gen 2: a damage of 1 draws no factor (pokecrystal `damagevariation`).
    let mut arena = singles_due(Generation::Gen2, 1);
    let start = arena.turn(&HARDEN, vec![accuracy(true)]);
    assert_eq!(arena.struck(start), vec![(WALL, 1)]);
    // Gen 3: it draws one, and stays 1 (`ApplyRandomDmgMultiplier`'s at least 1).
    let mut arena = singles_due(Generation::Gen3, 1);
    let start = arena.turn(&HARDEN, vec![accuracy(true), roll(85)]);
    assert_eq!(arena.struck(start), vec![(WALL, 1)]);
}

/// Gen 6+ (the condition's `onEnd`): a user on the field with Infiltrator as its Future Sight
/// strikes sends it past the target's Substitute (`moveData.infiltrates`, R-M10-81); without it
/// the Substitute takes the strike.
#[test]
fn an_infiltrators_strike_goes_past_a_substitute() {
    for (ability, past) in [("Infiltrator", true), ("Oblivious", false)] {
        let mut seer = set("Slowbro", &["Future Sight", "Harden"], 50);
        seer.ability = Some(ability.into());
        let mut arena = arena(
            Generation::Gen7,
            BattleMode::Singles,
            vec![seer],
            vec![set("Snorlax", &["Harden"], 40)],
            |state| {
                state
                    .pokemon_mut(WALL)
                    .unwrap()
                    .volatiles
                    .set_substitute(WALL, 200);
                state
                    .side_mut(SideId::P2)
                    .future_attacks
                    .push(FutureAttack {
                        slot: 0,
                        source: SEER,
                        move_id: MoveId::FUTURE_SIGHT,
                        damage: None,
                        turns_remaining: 1,
                    });
            },
        );
        let start = arena.turn(
            &HARDEN,
            vec![
                RandomOverride::bool(RandomRequestKind::Critical, false),
                roll(100),
            ],
        );
        assert_eq!(
            arena.struck(start).first().map(|(target, _)| *target),
            past.then_some(WALL),
            "{ability}"
        );
        let doll = arena
            .engine
            .state()
            .pokemon(WALL)
            .unwrap()
            .volatiles
            .substitute_hp();
        assert_eq!(doll == Some(200), past, "{ability}: the Substitute's HP");
    }
    // Off the field, its Infiltrator counts for nothing (`hasAbility`: an inactive Pokémon
    // ignores its ability from Gen 5): the Substitute takes the strike.
    let mut seer = set("Slowbro", &["Future Sight", "Harden"], 50);
    seer.ability = Some("Infiltrator".into());
    let (benched, target) = (PokemonId(2), PokemonId(3));
    let mut arena = arena(
        Generation::Gen7,
        BattleMode::Doubles,
        vec![set("Chansey", &["Harden"], 50), seer],
        vec![
            set("Snorlax", &["Harden"], 40),
            set("Blissey", &["Harden"], 45),
        ],
        |state| {
            state.side_mut(SideId::P1).active[1] = None;
            state
                .pokemon_mut(target)
                .unwrap()
                .volatiles
                .set_substitute(target, 200);
            state
                .side_mut(SideId::P2)
                .future_attacks
                .push(FutureAttack {
                    slot: 0,
                    source: benched,
                    move_id: MoveId::FUTURE_SIGHT,
                    damage: None,
                    turns_remaining: 1,
                });
        },
    );
    let start = arena.turn(
        &[(PokemonId(1), 0), (target, 0), (PokemonId(4), 0)],
        vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            roll(100),
        ],
    );
    assert!(arena.struck(start).is_empty(), "a benched Infiltrator");
    assert_ne!(
        arena
            .engine
            .state()
            .pokemon(target)
            .unwrap()
            .volatiles
            .substitute_hp(),
        Some(200)
    );
}
